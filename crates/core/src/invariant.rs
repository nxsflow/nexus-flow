//! Validity checks over the converged views — "Convergence != Validity". Pure, derived,
//! NON-DESTRUCTIVE: violations are surfaced and the affected items are isolated from
//! `ready` (see derive.rs); no edge is ever auto-deleted.

use rusqlite::Connection;

/// Shared cycle-detection CTE body (the `edges` + bounded `walk` definitions only —
/// no leading `WITH RECURSIVE`, no trailing select). Consolidated here because the
/// identical prefix is convergence-critical and used by `derive::ready`,
/// `derive::blocked`, and `cyclic_nodes`; a single source prevents drift. A node is in
/// a cycle iff a walk starting at it reaches itself (`node = start`). The depth bound
/// (<= edge count) guarantees termination on cyclic graphs (method from E0.5).
pub(crate) const CYCLE_CTE: &str = "\
    edges(f, t) AS (SELECT from_id, to_id FROM present_edges WHERE kind='dep'),
    walk(start, node, depth) AS (
        SELECT f, t, 1 FROM edges
        UNION ALL
        SELECT w.start, e.t, w.depth+1
        FROM walk w JOIN edges e ON e.f = w.node
        WHERE w.depth <= (SELECT count(*) FROM edges)
    )";

/// Item ids that participate in a `dep` cycle (both levels — items are type-agnostic).
/// Bounded recursive walk → terminates even on cyclic graphs (method from E0.5).
pub fn cyclic_nodes(conn: &Connection) -> Vec<String> {
    let sql = format!(
        "WITH RECURSIVE {CYCLE_CTE}
         SELECT DISTINCT start FROM walk WHERE node = start ORDER BY start"
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// A reference-integrity violation (surfaced, never auto-repaired).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub item_id: String,
    pub kind: String, // e.g. "belongs_to_missing", "belongs_to_deleted", "cycle"
}

/// The child's current parent — the `present_parent` projection of the `parent` OR-set edge
/// (sp6.3) — must point at an existing, non-deleted item. These are the **structural** parent
/// invariants (`belongs_to_missing`/`belongs_to_deleted`); the former `belongs_to_not_project`
/// type rule moved out of the meaning-free core in sp6.6 — *which* parent types are allowed is now
/// plugin policy (the relationship matrix), enforced at the facade write seam, not here.
pub fn reference_violations(conn: &Connection) -> Vec<Violation> {
    let mut stmt = conn
        .prepare(
            "SELECT i.id, p.id, COALESCE(p.deleted,'0')
             FROM items i
             JOIN present_parent pp ON pp.child_id = i.id
             LEFT JOIN items p ON p.id = pp.parent_id
             WHERE COALESCE(i.deleted,'0')<>'1'
             ORDER BY i.id",
        )
        .unwrap();
    let rows: Vec<(String, Option<String>, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut out = Vec::new();
    for (id, parent_id, parent_deleted) in rows {
        match parent_id {
            None => out.push(Violation {
                item_id: id,
                kind: "belongs_to_missing".into(),
            }),
            Some(_) if parent_deleted == "1" => out.push(Violation {
                item_id: id,
                kind: "belongs_to_deleted".into(),
            }),
            Some(_) => {}
        }
    }
    out.extend(dangling_edge_violations(conn));
    out
}

/// Spec §6: "edges point to existing items". Detect (never repair) present-edge
/// endpoints whose `from_id`/`to_id` does not resolve to an existing, non-deleted item.
/// `parent` edges are excluded: a missing/deleted parent is already reported as
/// `belongs_to_missing`/`belongs_to_deleted` by [`reference_violations`], so counting the parent
/// edge here too would double-flag it and break behaviour-preservation (sp6.3).
fn dangling_edge_violations(conn: &Connection) -> Vec<Violation> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT endpoint FROM (
                 SELECT from_id AS endpoint FROM present_edges WHERE kind<>'parent'
                 UNION SELECT to_id FROM present_edges WHERE kind<>'parent'
             ) e WHERE NOT EXISTS (
                 SELECT 1 FROM items i
                 WHERE i.id = e.endpoint AND COALESCE(i.deleted,'0') <> '1')
             ORDER BY endpoint",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok(Violation {
            item_id: r.get::<_, String>(0)?,
            kind: "edge_dangling".into(),
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EdgeKind;
    use crate::store::Store;

    #[test]
    fn detects_a_cycle_formed_by_merge() {
        // a: A->B ; b: B->A ; after merge the graph has a cycle A<->B.
        let mut a = Store::open_in_memory(1);
        a.create_item("c1.A", "task", "A", "x");
        a.create_item("c1.B", "task", "B", "x");
        let mut b = Store::open_in_memory(2);
        b.apply(&a.export());

        a.add_edge("c1.A", "c1.B", EdgeKind::Dep, "x");
        b.add_edge("c1.B", "c1.A", EdgeKind::Dep, "x");
        a.apply(&b.export());

        let mut cyc = cyclic_nodes(a.connection());
        cyc.sort();
        assert_eq!(cyc, vec!["c1.A".to_string(), "c1.B".to_string()]);
    }

    #[test]
    fn core_does_not_flag_a_parents_type_that_is_plugin_policy() {
        // sp6.6: the former `belongs_to_not_project` rule left the meaning-free core — *which* parent
        // types are allowed is plugin policy (the relationship matrix), enforced at the facade write
        // seam. A live parent of any type is therefore STRUCTURALLY valid here (it exists, not
        // deleted), so the core surfaces NO violation; the facade rejects a disallowed type pair.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.P", "task", "a non-container parent", "x"); // a task parent
        s.create_item("c1.T", "task", "child", "x");
        s.set_parent("c1.T", "c1.P", "x").unwrap();
        assert!(
            reference_violations(s.connection()).is_empty(),
            "the core no longer flags a parent's type — that is plugin policy now"
        );
    }

    #[test]
    fn flags_edge_pointing_at_a_missing_item() {
        // A exists; B was never created. The A->B dep edge dangles at B (spec §6).
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "x");
        s.add_edge("c1.A", "c1.B", EdgeKind::Dep, "x");
        let v = reference_violations(s.connection());
        assert!(
            v.contains(&Violation {
                item_id: "c1.B".into(),
                kind: "edge_dangling".into()
            }),
            "expected edge_dangling for c1.B, got {v:?}"
        );
    }

    #[test]
    fn flags_belongs_to_missing_parent() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.T", "task", "child", "x");
        s.set_parent("c1.T", "c1.NOPE", "x").unwrap();
        let v = reference_violations(s.connection());
        assert!(v.contains(&Violation {
            item_id: "c1.T".into(),
            kind: "belongs_to_missing".into()
        }));
    }

    #[test]
    fn flags_belongs_to_deleted_parent() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.P", "project", "proj", "x");
        s.create_item("c1.T", "task", "child", "x");
        s.set_parent("c1.T", "c1.P", "x").unwrap();
        s.delete_item("c1.P", "x");
        let v = reference_violations(s.connection());
        assert!(v.contains(&Violation {
            item_id: "c1.T".into(),
            kind: "belongs_to_deleted".into()
        }));
    }

    #[test]
    fn valid_belongs_to_yields_no_violation() {
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.P", "project", "proj", "x");
        s.create_item("c1.T", "task", "child", "x");
        s.set_parent("c1.T", "c1.P", "x").unwrap();
        let v = reference_violations(s.connection());
        assert!(
            v.is_empty(),
            "valid project parent must produce no violation, got {v:?}"
        );
    }
}
