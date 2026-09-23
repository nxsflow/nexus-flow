//! The flow side of the beads → nxs migration (nexus-flow-6ef.3): import beads tickets into the
//! flow store. Title, description, type, priority, status, the parent/dependency graph, closing
//! reasons, and timestamps are carried over.
//!
//! **Why the core store, not the facade `create` seam.** A migration's prime directive is fidelity
//! (the epic's "keine Datenvernichtung" + "Kanten bleiben erhalten"), and beads' hierarchy is more
//! permissive than the issue-tracker plugin's creation matrix (e.g. a sub-epic under an epic, which
//! the matrix would reject at `create`). Writing through the same core [`Store`] primitives the
//! facade itself composes preserves EVERY edge without the creation-policy gate — exactly the path
//! `nxm import` takes for memories (`store.remember`). The result is a valid op-log either way
//! (type/cardinality/depth are write-seam *policy*, not convergence invariants).
//!
//! **Targets issue-tracker.** The type mapping below is to the issue-tracker plugin's type set
//! (`epic/bug/feature/chore/decision`), the only bundled plugin whose vocabulary fits beads; the
//! orchestrator pins it when it sets flow up. Priorities map 1:1 (beads `0..=4` ↔ `P0..P4` ordinals).

use crate::error::Result;
use crate::workspace::{Workspace, WorkspaceExt};
use nexus_flow_core::id::ReplicaIds;
use nexus_flow_core::model::EdgeKind;
use nexus_flow_core::store::Store;
use nexus_flow_facade::validate;
use nxs_init::beads::BeadsIssue;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// The closing comment stamped on a beads-closed ticket that recorded no reason — flow requires a
/// reason at close, so the migration supplies this rather than refusing the ticket.
pub const DEFAULT_CLOSE_REASON: &str = "Migrated from beads (no closing reason was recorded).";

/// What the ticket import did, for the migration report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueImport {
    /// Items created in flow.
    pub created: usize,
    /// Items carried over already closed (with their reason, or the default).
    pub closed: usize,
    /// Of `closed`, how many had no beads reason and got [`DEFAULT_CLOSE_REASON`].
    pub default_close_reasons: usize,
    pub parent_edges: usize,
    pub dep_edges: usize,
    pub mention_edges: usize,
    /// User labels carried into the flow OR-set (h89s.4).
    pub labels: usize,
    /// Human-readable notes for edges that could not be carried (a cycle, or an endpoint that was
    /// not itself migrated) — surfaced in the report rather than failing the whole migration.
    pub skipped_edges: Vec<String>,
    /// Non-edge, non-fatal migration notes: a label that failed validation, or a malformed
    /// `defer_until`/`due` that was skipped rather than written (a bad date would misclassify the
    /// deferred/ready lane). Kept distinct from `skipped_edges` so the operator message is accurate.
    pub warnings: Vec<String>,
    /// beads id → freshly minted flow id (the translation table the edge pass rewires through).
    pub id_map: BTreeMap<String, String>,
}

/// Map a beads `issue_type` to an issue-tracker type. The five shared kinds map 1:1; beads' generic
/// `task` (which issue-tracker lacks) and any unknown kind fall to `feature` (issue-tracker's
/// generic unit of work). Pure.
pub fn map_type(beads_type: &str) -> &'static str {
    match beads_type {
        "epic" => "epic",
        "bug" => "bug",
        "feature" => "feature",
        "chore" => "chore",
        "decision" => "decision",
        // beads' generic `task` (no issue-tracker equivalent) + any unknown kind → the generic unit.
        _ => "feature",
    }
}

/// Map a beads numeric priority (`0..=4`, 0 = highest) to an issue-tracker priority ordinal string
/// (`P0..P4` ⇒ `"0".."4"`). A missing priority defaults to the middle band; an out-of-range value
/// is clamped. Pure.
pub fn map_priority(priority: Option<i64>) -> String {
    priority.unwrap_or(2).clamp(0, 4).to_string()
}

/// Import beads `issues` into the flow `store` under replica `prefix`, attributing ops to `actor`
/// and stamping `now` on records that carry no timestamp of their own. Two passes: create every
/// item first (filling the id-translation table), then rewire the parent/dependency/reference edges
/// through it — so an edge to a ticket created later in the batch still resolves. The store mutates
/// in place; the returned [`IssueImport`] is the report.
pub fn import_issues(
    store: &mut Store,
    prefix: &str,
    actor: &str,
    now: &str,
    issues: &[BeadsIssue],
) -> IssueImport {
    let minter = ReplicaIds::new(prefix);
    let mut rep = IssueImport::default();

    // Pass 1 — create every item (filling the id-translation table) + its scalar fields + status,
    // each backdated to the ticket's own timestamps.
    for iss in issues {
        let Some(id) =
            minter.mint_unique(|c| store.get_item(c).map(|o| o.is_some()).unwrap_or(false))
        else {
            rep.skipped_edges.push(format!(
                "could not mint a flow id for beads ticket {}",
                iss.id
            ));
            continue;
        };
        rep.id_map.insert(iss.id.clone(), id.clone());

        let created = iss.created_at.as_deref().unwrap_or(now);
        store.set_wall_clock(created);
        store.create_item(&id, map_type(&iss.issue_type), &iss.title, actor);
        if !iss.description.is_empty() {
            store.set_field(&id, "description", Some(iss.description.clone()), actor);
        }
        store.set_field(&id, "priority", Some(map_priority(iss.priority)), actor);
        set_if_present(store, &id, "design", iss.design.as_deref(), actor);
        set_if_present(
            store,
            &id,
            "completion_criterion",
            iss.acceptance_criteria.as_deref(),
            actor,
        );
        set_if_present(store, &id, "assignee", iss.assignee.as_deref(), actor);
        // a5w: the defer/due dates (dropped before this fix), backdated to `created` like every other
        // Pass-1 field. Validated through the SAME `iso_date` seam every other write path uses — a
        // malformed value is SKIPPED + warned, never silently written, since the derivation string-
        // compares these fields and a bad date would permanently misclassify the ready/deferred lane.
        set_date_if_valid(
            store,
            &id,
            "defer_until",
            iss.defer_until.as_deref(),
            actor,
            &iss.id,
            &mut rep,
        );
        set_date_if_valid(
            store,
            &id,
            "due",
            iss.due.as_deref(),
            actor,
            &iss.id,
            &mut rep,
        );
        // h89s.4: carry beads labels into flow's user-label OR-set through the SHARED `validate::label`
        // (trim / non-empty / separator-free) — the same normalization the facade write seam applies.
        // An invalid label is skipped + warned rather than failing the migration or panicking the core.
        for raw in &iss.labels {
            match validate::label(raw) {
                Ok(label) => {
                    store.add_label(&id, &label, actor);
                    rep.labels += 1;
                }
                Err(e) => rep
                    .warnings
                    .push(format!("{}: label {raw:?} skipped ({e})", iss.id)),
            }
        }
        if let Some(notes) = iss.notes.as_deref().filter(|s| !s.trim().is_empty()) {
            store.add_note(&id, notes, actor);
        }
        rep.created += 1;

        match iss.status.as_str() {
            "closed" => {
                let ts = iss
                    .closed_at
                    .as_deref()
                    .or(iss.updated_at.as_deref())
                    .unwrap_or(now);
                store.set_wall_clock(ts);
                let reason = match iss.close_reason.as_deref().filter(|s| !s.trim().is_empty()) {
                    Some(r) => r.to_string(),
                    None => {
                        rep.default_close_reasons += 1;
                        DEFAULT_CLOSE_REASON.to_string()
                    }
                };
                store.set_field(&id, "status", Some("closed".to_string()), actor);
                store.set_field(&id, "closing_comment", Some(reason), actor);
                store.set_field(&id, "closed_at", Some(ts.to_string()), actor);
                rep.closed += 1;
            }
            "in_progress" => {
                store.set_wall_clock(iss.updated_at.as_deref().unwrap_or(now));
                store.set_field(&id, "status", Some("in_progress".to_string()), actor);
            }
            // open (or any other value) — `create_item` already left it open.
            _ => {}
        }
    }

    // Pass 2 — rewire the edges through the translation table now every endpoint exists.
    for iss in issues {
        let Some(from) = rep.id_map.get(&iss.id).cloned() else {
            continue;
        };
        let edge_clock = iss.created_at.as_deref().unwrap_or(now);
        for d in &iss.dependencies {
            let Some(to) = rep.id_map.get(&d.depends_on_id).cloned() else {
                rep.skipped_edges.push(format!(
                    "{} -[{}]-> {}: endpoint not migrated",
                    iss.id, d.dep_type, d.depends_on_id
                ));
                continue;
            };
            store.set_wall_clock(edge_clock);
            match d.dep_type.as_str() {
                // child (from) belongs to parent (to). Multi-parent-safe (add, not replace) so a
                // ticket under several parents keeps them all.
                "parent-child" => {
                    if from == to || reaches(store, &to, &from, EdgeWalk::Parent) {
                        rep.skipped_edges.push(format!(
                            "parent {} -> {}: would create a parent cycle",
                            iss.id, d.depends_on_id
                        ));
                    } else {
                        store.add_parent(&from, &to, actor);
                        rep.parent_edges += 1;
                    }
                }
                // `from` depends on `to` (to blocks from).
                "blocks" => {
                    if from == to || reaches(store, &to, &from, EdgeWalk::Dep) {
                        rep.skipped_edges.push(format!(
                            "dep {} -> {}: would create a dependency cycle",
                            iss.id, d.depends_on_id
                        ));
                    } else {
                        store.add_edge(&from, &to, EdgeKind::Dep, actor);
                        rep.dep_edges += 1;
                    }
                }
                // related / relates-to / supersedes / discovered-from / anything else: a pure
                // non-blocking reference (mention) so the relationship survives without affecting
                // derivation.
                _ => {
                    store.add_edge(&from, &to, EdgeKind::Mentions, actor);
                    rep.mention_edges += 1;
                }
            }
        }
    }

    rep
}

/// Open the flow store for the workspace at `root` and import `issues` into it (the seam the `nxs`
/// orchestrator calls, so it stays free of flow's workspace/store internals). The workspace must
/// already exist — the orchestrator sets flow up first. Writes persist as ops are emitted.
pub fn import_at(
    root: &Path,
    actor: &str,
    now: &str,
    issues: &[BeadsIssue],
) -> Result<IssueImport> {
    let ws = Workspace::resolve(None, root)?;
    let mut store = ws.open_store()?;
    Ok(import_issues(
        &mut store,
        &ws.replica.prefix,
        actor,
        now,
        issues,
    ))
}

/// Set a date field (`defer_until`/`due`) only when present AND a valid ISO-8601 date / RFC3339
/// timestamp — the same [`validate::iso_date`] gate every facade write path enforces (a5w review).
/// A malformed value is SKIPPED and recorded in `warnings`, never written: the derivation
/// string-compares these fields, so a bad date would silently and permanently misclassify the
/// item's ready/deferred lane (the exact silent-loss class a5w set out to close).
fn set_date_if_valid(
    store: &mut Store,
    id: &str,
    field: &str,
    value: Option<&str>,
    actor: &str,
    beads_id: &str,
    rep: &mut IssueImport,
) {
    let Some(v) = value.filter(|s| !s.is_empty()) else {
        return;
    };
    match validate::iso_date(v) {
        Ok(v) => store.set_field(id, field, Some(v), actor),
        Err(e) => rep
            .warnings
            .push(format!("{beads_id}: {field} {v:?} skipped ({e})")),
    }
}

/// `set_field` only for a present, non-empty value (so an absent beads field stays `None` in flow
/// rather than landing an empty cell).
fn set_if_present(store: &mut Store, id: &str, field: &str, value: Option<&str>, actor: &str) {
    if let Some(v) = value.filter(|s| !s.is_empty()) {
        store.set_field(id, field, Some(v.to_string()), actor);
    }
}

/// Which edge kind a reachability walk follows (for the write-time cycle guards).
#[derive(Clone, Copy)]
enum EdgeWalk {
    Dep,
    Parent,
}

/// Does `from` reach `target` by following edges of `walk`? Mirrors the facade's write-time cycle
/// guards so the importer never lands a cyclic dep/parent edge (beads is acyclic, so this is
/// defensive — a detected cycle is skipped + reported, never force-written).
fn reaches(store: &Store, from: &str, target: &str, walk: EdgeWalk) -> bool {
    let mut stack = vec![from.to_string()];
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if node == target {
            return true;
        }
        if !seen.insert(node.clone()) {
            continue;
        }
        match walk {
            // `deps_of` is fallible (#76u.13); this is a one-shot CLI import (not a long-lived read
            // seam), so a genuine db error here is surfaced as a panic — the pre-existing,
            // acceptable behavior — rather than silently swallowed (which would miss a cycle).
            EdgeWalk::Dep => {
                stack.extend(store.deps_of(&node).expect("deps_of read during import"))
            }
            EdgeWalk::Parent => stack.extend(store.parents_of(&node)),
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_type_maps_shared_kinds_one_to_one_and_task_to_feature() {
        for k in ["epic", "bug", "feature", "chore", "decision"] {
            assert_eq!(map_type(k), k, "{k} maps to itself");
        }
        assert_eq!(
            map_type("task"),
            "feature",
            "beads task → issue-tracker feature"
        );
        assert_eq!(
            map_type("mystery"),
            "feature",
            "unknown kind → generic feature"
        );
    }

    #[test]
    fn map_priority_preserves_the_band_and_defaults_when_absent() {
        assert_eq!(map_priority(Some(0)), "0", "P0 highest");
        assert_eq!(map_priority(Some(4)), "4", "P4 lowest");
        assert_eq!(map_priority(Some(2)), "2");
        assert_eq!(map_priority(None), "2", "absent → middle band");
        assert_eq!(map_priority(Some(9)), "4", "out of range clamps to lowest");
    }

    // ---- import_issues integration (over an in-memory flow store) ------------

    fn issue(id: &str, ty: &str, status: &str) -> BeadsIssue {
        BeadsIssue {
            id: id.to_string(),
            title: format!("title of {id}"),
            description: format!("desc of {id}"),
            status: status.to_string(),
            priority: Some(2),
            issue_type: ty.to_string(),
            created_at: Some("2026-01-02T03:04:05Z".to_string()),
            ..Default::default()
        }
    }

    fn dep(issue_id: &str, depends_on: &str, ty: &str) -> nxs_init::beads::BeadsDep {
        nxs_init::beads::BeadsDep {
            issue_id: issue_id.to_string(),
            depends_on_id: depends_on.to_string(),
            dep_type: ty.to_string(),
        }
    }

    #[test]
    fn a_dependency_cycle_is_skipped_and_reported_not_written() {
        // A↔B mutual `blocks` (beads is acyclic, but a corrupt/hand-edited export could carry one):
        // the first edge lands, the second would close a cycle → the guard skips it and records the
        // skip, rather than landing an invalid edge the derivation can't cope with.
        let mut store = Store::open_in_memory(1);
        let mut a = issue("b-a", "feature", "open");
        let mut b = issue("b-b", "feature", "open");
        a.dependencies = vec![dep("b-a", "b-b", "blocks")];
        b.dependencies = vec![dep("b-b", "b-a", "blocks")];
        let rep = import_issues(&mut store, "px", "m", "2026-06-26T00:00:00Z", &[a, b]);
        assert_eq!(rep.created, 2);
        assert_eq!(rep.dep_edges, 1, "only the acyclic edge is written");
        assert_eq!(
            rep.skipped_edges.len(),
            1,
            "the cycle-closing edge is skipped"
        );
        assert!(
            rep.skipped_edges[0].contains("cycle"),
            "the skip names the reason: {:?}",
            rep.skipped_edges[0]
        );
    }

    #[test]
    fn creates_each_ticket_with_title_description_type_and_priority() {
        let mut store = Store::open_in_memory(1);
        let issues = vec![issue("b-1", "task", "open")];
        let rep = import_issues(
            &mut store,
            "px",
            "migrator",
            "2026-06-26T00:00:00Z",
            &issues,
        );
        assert_eq!(rep.created, 1);
        let flow_id = rep.id_map.get("b-1").expect("b-1 mapped to a flow id");
        let item = store
            .get_item(flow_id)
            .unwrap()
            .expect("created item readable");
        assert_eq!(item.title.as_deref(), Some("title of b-1"));
        assert_eq!(item.description.as_deref(), Some("desc of b-1"));
        assert_eq!(item.item_type.as_deref(), Some("feature"), "task → feature");
        assert_eq!(item.priority.as_deref(), Some("2"));
        assert_eq!(
            item.status.as_deref(),
            Some("open"),
            "open ticket stays open"
        );
    }

    #[test]
    fn preserves_parent_and_dependency_edges_through_id_remapping() {
        // epic ← child (parent-child); child2 depends on child1 (blocks). All ids must be remapped.
        let mut store = Store::open_in_memory(1);
        let mut epic = issue("b-epic", "epic", "open");
        let mut c1 = issue("b-c1", "feature", "open");
        let mut c2 = issue("b-c2", "feature", "open");
        c1.dependencies = vec![dep("b-c1", "b-epic", "parent-child")];
        c2.dependencies = vec![
            dep("b-c2", "b-epic", "parent-child"),
            dep("b-c2", "b-c1", "blocks"),
        ];
        epic.dependencies = vec![];
        let rep = import_issues(
            &mut store,
            "px",
            "m",
            "2026-06-26T00:00:00Z",
            &[epic, c1, c2],
        );
        let f = |b: &str| rep.id_map.get(b).cloned().unwrap();
        // parent edges: c1 and c2 both belong to the epic (remapped ids, not the beads ids).
        assert_eq!(store.parents_of(&f("b-c1")), vec![f("b-epic")]);
        assert_eq!(store.parents_of(&f("b-c2")), vec![f("b-epic")]);
        // dep edge: c2 depends on c1.
        assert_eq!(store.deps_of(&f("b-c2")).unwrap(), vec![f("b-c1")]);
        assert_eq!(rep.parent_edges, 2);
        assert_eq!(rep.dep_edges, 1);
    }

    #[test]
    fn defer_until_and_due_migrate_backdated_to_created() {
        // a5w: a deferred beads ticket carries its `defer_until` (and `due`) into flow — the fields
        // the struct used to silently drop, landing the item as plain `open` with no defer boundary.
        let mut store = Store::open_in_memory(1);
        let mut iss = issue("b-1", "task", "open");
        iss.defer_until = Some("2026-08-01T00:00:00Z".to_string());
        iss.due = Some("2026-09-01T00:00:00Z".to_string());
        let rep = import_issues(&mut store, "px", "m", "2026-06-26T00:00:00Z", &[iss]);
        let item = store
            .get_item(rep.id_map.get("b-1").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            item.defer_until.as_deref(),
            Some("2026-08-01T00:00:00Z"),
            "defer_until is carried over"
        );
        assert_eq!(
            item.due.as_deref(),
            Some("2026-09-01T00:00:00Z"),
            "due is carried over"
        );
        assert_eq!(
            item.status.as_deref(),
            Some("open"),
            "a deferred ticket stays open"
        );
    }

    #[test]
    fn labels_migrate_into_the_user_label_or_set() {
        // h89s.4: beads labels become the flow user-label OR-set (sorted on read); the count is
        // reported so the migration summary can surface it.
        let mut store = Store::open_in_memory(1);
        let mut iss = issue("b-1", "task", "open");
        iss.labels = vec!["urgent".to_string(), "backend".to_string()];
        let rep = import_issues(&mut store, "px", "m", "2026-06-26T00:00:00Z", &[iss]);
        let flow_id = rep.id_map.get("b-1").unwrap();
        assert_eq!(
            store.labels_of(flow_id).unwrap(),
            vec!["backend".to_string(), "urgent".to_string()],
            "both beads labels land in the OR-set"
        );
        assert_eq!(rep.labels, 2, "both labels are counted");
    }

    #[test]
    fn a_malformed_defer_or_due_date_is_skipped_and_warned_not_written() {
        // a5w review: derivation string-compares defer_until/due, so a bad date would silently and
        // permanently misclassify the ready/deferred lane. The migration validates via the shared
        // iso_date gate and SKIPS + warns rather than writing garbage.
        let mut store = Store::open_in_memory(1);
        let mut iss = issue("b-1", "task", "open");
        iss.defer_until = Some("not-a-date".to_string());
        iss.due = Some("2026-13-99".to_string()); // syntactically-shaped but impossible
        let rep = import_issues(&mut store, "px", "m", "2026-06-26T00:00:00Z", &[iss]);
        let item = store
            .get_item(rep.id_map.get("b-1").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            item.defer_until, None,
            "a malformed defer_until is not written"
        );
        assert_eq!(item.due, None, "a malformed due is not written");
        assert_eq!(rep.warnings.len(), 2, "both malformed dates are warned");
        assert!(rep.warnings.iter().any(|w| w.contains("defer_until")));
        assert!(rep.warnings.iter().any(|w| w.contains("due")));
        assert!(
            rep.skipped_edges.is_empty(),
            "a skipped date is not an edge"
        );
    }

    #[test]
    fn an_invalid_label_is_warned_not_filed_under_skipped_edges() {
        // The shared validate::label trims + rejects blank/separator labels; the migration routes a
        // rejection into `warnings` (not `skipped_edges`, which is edges), so the operator message is
        // accurate. A valid label still lands.
        let mut store = Store::open_in_memory(1);
        let mut iss = issue("b-1", "task", "open");
        iss.labels = vec!["   ".into(), "ok".into(), "a\u{1f}b".into()];
        let rep = import_issues(&mut store, "px", "m", "2026-06-26T00:00:00Z", &[iss]);
        assert_eq!(
            store.labels_of(rep.id_map.get("b-1").unwrap()).unwrap(),
            vec!["ok".to_string()],
            "only the valid label is carried"
        );
        assert_eq!(rep.labels, 1);
        assert_eq!(
            rep.warnings.len(),
            2,
            "the blank + separator labels are warned"
        );
        assert!(
            rep.skipped_edges.is_empty(),
            "label skips are not filed as edges"
        );
    }

    #[test]
    fn a_ticket_without_labels_or_dates_is_unchanged() {
        // The migration-without-labels/dates path stays byte-for-byte as before (no empty cells).
        let mut store = Store::open_in_memory(1);
        let rep = import_issues(
            &mut store,
            "px",
            "m",
            "2026-06-26T00:00:00Z",
            &[issue("b-1", "task", "open")],
        );
        let flow_id = rep.id_map.get("b-1").unwrap();
        let item = store.get_item(flow_id).unwrap().unwrap();
        assert!(store.labels_of(flow_id).unwrap().is_empty());
        assert_eq!(rep.labels, 0);
        assert_eq!(item.defer_until, None);
        assert_eq!(item.due, None);
    }

    #[test]
    fn closed_tickets_keep_their_reason_and_default_when_missing() {
        let mut store = Store::open_in_memory(1);
        let mut with_reason = issue("b-1", "bug", "closed");
        with_reason.close_reason = Some("fixed in #42".to_string());
        with_reason.closed_at = Some("2026-03-04T05:06:07Z".to_string());
        let mut no_reason = issue("b-2", "bug", "closed");
        no_reason.close_reason = None;
        no_reason.closed_at = Some("2026-03-05T00:00:00Z".to_string());
        let rep = import_issues(
            &mut store,
            "px",
            "m",
            "2026-06-26T00:00:00Z",
            &[with_reason, no_reason],
        );
        assert_eq!(rep.closed, 2);
        assert_eq!(rep.default_close_reasons, 1, "one ticket lacked a reason");
        let i1 = store
            .get_item(rep.id_map.get("b-1").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(i1.status.as_deref(), Some("closed"));
        assert_eq!(i1.closing_comment.as_deref(), Some("fixed in #42"));
        let i2 = store
            .get_item(rep.id_map.get("b-2").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(i2.closing_comment.as_deref(), Some(DEFAULT_CLOSE_REASON));
    }

    #[test]
    fn carries_over_beads_timestamps() {
        let mut store = Store::open_in_memory(1);
        let mut closed = issue("b-1", "bug", "closed");
        closed.created_at = Some("2026-01-01T00:00:00Z".to_string());
        closed.close_reason = Some("done".to_string());
        closed.closed_at = Some("2026-02-02T00:00:00Z".to_string());
        let rep = import_issues(&mut store, "px", "m", "2026-06-26T00:00:00Z", &[closed]);
        let flow_id = rep.id_map.get("b-1").unwrap();
        let item = store.get_item(flow_id).unwrap().unwrap();
        // closed_at is surfaced on the row → must equal the beads close instant, not migration-now.
        assert_eq!(item.closed_at.as_deref(), Some("2026-02-02T00:00:00Z"));
        // the create op carries the beads created_at as its wall clock (backdated, not now).
        let create_op_clock = store
            .export()
            .into_iter()
            .find(|op| &op.target_id == flow_id && op.field == "title")
            .map(|op| op.wall_clock)
            .expect("a title op for the created item");
        assert_eq!(create_op_clock, "2026-01-01T00:00:00Z", "created backdated");
    }

    #[test]
    fn an_edge_to_an_unmigrated_endpoint_is_skipped_not_fatal() {
        let mut store = Store::open_in_memory(1);
        let mut c = issue("b-c", "feature", "open");
        c.dependencies = vec![dep("b-c", "b-ghost", "blocks")]; // b-ghost is not in the batch
        let rep = import_issues(&mut store, "px", "m", "2026-06-26T00:00:00Z", &[c]);
        assert_eq!(rep.created, 1, "the ticket still imports");
        assert_eq!(rep.dep_edges, 0, "the dangling edge is not wired");
        assert_eq!(rep.skipped_edges.len(), 1, "and is reported as skipped");
    }

    #[test]
    fn import_at_opens_the_workspace_store_and_persists_to_disk() {
        let tmp = tempfile::TempDir::new().unwrap();
        crate::workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
        let rep = import_at(
            tmp.path(),
            "m",
            "2026-06-26T00:00:00Z",
            &[issue("b-1", "epic", "open")],
        )
        .unwrap();
        assert_eq!(rep.created, 1);
        let flow_id = rep.id_map.get("b-1").unwrap();
        // Re-resolve + re-open the workspace: the item must have persisted to the on-disk store.
        let ws = Workspace::resolve(None, tmp.path()).unwrap();
        let store = ws.open_store().unwrap();
        assert!(
            store.get_item(flow_id).unwrap().is_some(),
            "imported item persisted across reopen"
        );
    }

    #[test]
    fn soft_relationship_types_become_mention_edges() {
        let mut store = Store::open_in_memory(1);
        let a = issue("b-a", "feature", "open");
        let mut b = issue("b-b", "feature", "open");
        b.dependencies = vec![dep("b-b", "b-a", "related")];
        let rep = import_issues(&mut store, "px", "m", "2026-06-26T00:00:00Z", &[a, b]);
        assert_eq!(rep.mention_edges, 1, "related → mention");
        assert_eq!(rep.dep_edges, 0);
        assert_eq!(rep.parent_edges, 0);
    }
}
