//! Full change history per item, read straight from the op log. Includes concurrent,
//! later-overwritten edits once their ops have propagated — the motivating requirement.

use rusqlite::Connection;

/// One recorded change.
///
/// Changes are ordered by `(lamport, site)` — a **causal/deterministic order,
/// NOT wall-clock chronology**. Lamport clocks guarantee that if op *a* causally
/// precedes op *b* then `lamport(a) < lamport(b)`; concurrent ops (neither causes
/// the other) are ordered deterministically by `site`, not by real time.
/// `wall_clock` on each op is **display-only** (a human-readable hint) and may be
/// empty in E1 — never use it to compare or sort ops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub field: String,
    pub value: Option<String>,
    pub author: String,
    pub lamport: i64,
    pub site: i64,
}

/// All `set` changes to an item's fields, oldest first. Edge/note ops are excluded
/// (those have their own streams).
pub fn item_history(conn: &Connection, item_id: &str) -> Vec<Change> {
    let mut stmt = conn
        .prepare(
            "SELECT field, value, author, lamport, site FROM ops
             WHERE target_id=?1 AND target_kind='item' AND op_type='set'
             ORDER BY lamport, site",
        )
        .unwrap();
    stmt.query_map([item_id], |r| {
        Ok(Change {
            field: r.get(0)?,
            value: r.get(1)?,
            author: r.get(2)?,
            lamport: r.get(3)?,
            site: r.get(4)?,
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn history_keeps_concurrent_overwritten_title_edits() {
        // c1 sets title; c3 concurrently sets title; c2 sets status. After everyone
        // syncs, all three changes are visible in history (even the losing title).
        let mut c1 = Store::open_in_memory(1);
        c1.create_item("g.X", "task", "orig", "u1");

        let mut c3 = Store::open_in_memory(3);
        c3.apply(&c1.export());
        let mut c2 = Store::open_in_memory(2);
        c2.apply(&c1.export());

        c1.set_field("g.X", "title", Some("title-from-c1".into()), "u1");
        c3.set_field("g.X", "title", Some("title-from-c3".into()), "u2");
        c2.set_field("g.X", "status", Some("in_progress".into()), "u1");

        // Everyone exchanges ops.
        let all = [c1.export(), c2.export(), c3.export()].concat();
        c1.apply(&all);

        let titles: Vec<Option<String>> = item_history(c1.connection(), "g.X")
            .into_iter()
            .filter(|c| c.field == "title")
            .map(|c| c.value)
            .collect();
        // "orig" (create) + both concurrent edits = 3 title entries.
        assert_eq!(titles.len(), 3);
        assert!(titles.contains(&Some("title-from-c1".into())));
        assert!(titles.contains(&Some("title-from-c3".into())));

        let statuses: Vec<Option<String>> = item_history(c1.connection(), "g.X")
            .into_iter()
            .filter(|c| c.field == "status")
            .map(|c| c.value)
            .collect();
        // "open" (create) + "in_progress" = 2 status entries.
        assert_eq!(statuses.len(), 2);
    }
}
