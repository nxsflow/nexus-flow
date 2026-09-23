//! Pure, deterministic derivation over the materialized views — never stored state.
//! Time-dependent queries take `now` (ISO-8601) as a parameter to stay deterministic.
//!
//! ## Three distinct derived states
//!
//! Every open, non-deleted task falls into exactly one of three categories:
//!
//! - **ready** — no open blocker, not in a dep-cycle, not deferred into the future.
//! - **blocked** — has at least one open blocker dep, OR is part of a dep-cycle.
//! - **deferred** — `defer_until` is set to a future date, has NO open blocker, and is
//!   not in a cycle. Such a task appears in **neither** `ready` nor `blocked`.
//!
//! "Deferred" is therefore a third, distinct derived state, not a sub-case of blocked.
//! A task that is *both* deferred and has an open blocker shows as **blocked** (the
//! blocker takes precedence — the task can't start regardless of the defer date).
//!
//! ### `ready` is this module's word, not a verb anyone can type (nxf 6j6v.4dvc)
//!
//! There is no `nxf ready`, and there is not meant to be one: the lanes a reader can ask for are
//! `next` and `blocked`, and **ready** is the state `next` ranks. The published guides used to
//! print it in code font in a list beside those two, which taught a reader a lane they could not
//! query — the docs contradicting the surface, with both sides green on their own because no gate
//! spans them.
//!
//! The decision (2026-09-02) keeps the word and constrains where it may appear: in user-facing
//! prose *ready* is a state an item is **in**, never code font, never a third item in a list of
//! lanes. **Backticks mean "you can type this"** — that is the whole rule. The name stays HERE
//! unchanged, because it is accurate here; renaming an engine's own vocabulary to protect a reader
//! from a formatting habit would cost the engine its word and fix nothing.
//!
//! This is why `blocked()` takes **no `now` parameter**: deferral does not change
//! whether something is blocked. The E2 CLI should surface deferred tasks with a
//! separate query (e.g. `WHERE defer_until > now AND status='open'`) rather than
//! expecting them to appear in `ready` or `blocked`.
//!
//! ## Parent is gating containment; `contributes_to` is the non-gating edge (07a, inverts sp6.7 ②c)
//!
//! The agreed target model (`docs/specs/07a-parent-child-status-coupling.md`) makes the `parent`
//! edge a **gating containment** edge: a child's *effective* (derived, displayed) lane is coupled to
//! its parents' lifecycle — a child under a `blocked`/`deferred` ancestor is suppressed, a child of a
//! `ready` parent shows as `ready`, all-parents-`closed` masks the child as `closed` — while
//! `contributes_to` (#vf4, n:m) stays the **non-gating** escape hatch. This **inverts** the original
//! sp6.7 ②c invariant ("parenthood is *containment*, not a blocking relationship").
//!
//! **Status of the rollout in THIS module.** `ready`/`next` (the [`actionable`] engine) now **gate on
//! ancestors**: a child whose transitive `parent` chain holds a `blocked`/`deferred` ancestor is
//! **suppressed** (07a.2, the shared `blocked_gating`/`deferred_gating` CTEs feeding `suppressed`),
//! and a still-open child whose every live parent is `closed` is **closed-masked** out of
//! `ready`/`next` (07a.3). Claiming a child also propagates `in_progress` up the chain (07a.1, write
//! layer). The effective-lane *display* lives in the read layer: the facade surfaces
//! `parent_closed_reason` on `show`/`list`/`next` and groups `search` over **effective** lanes (07a.3
//! §5/§8), using [`suppressed_by_blocked`]/[`suppressed_by_deferred_only`] to place a suppressed
//! child under its gating ancestor's lane; the ready-mask is a read-layer projection too. A child's
//! own block/defer is still computed from its own `dep`s/defer date; the parent chain *additionally*
//! gates it, and `blocked`/`deferred` here still report each item's OWN lane (the effective lane is a
//! read-layer concern — see `docs/specs/07a-parent-child-status-coupling.md`).

use crate::invariant::CYCLE_CTE;
use rusqlite::{params, Connection};

// ---- shared gating CTEs (07a.2/07a.3) --------------------------------------
//
// The gating set — open ancestors whose lane suppresses their descendants from ready/next — splits
// into a BLOCKED half and a DEFERRED half (§2 most-restrictive-wins ranks blocked over deferred).
// These two CTE fragments are the SINGLE source of that predicate: `actionable` unions them for its
// `suppressed` walk (behaviour unchanged from the old combined `gating`), and the effective-lane
// search grouping (07a.3 §8) walks each half separately to group a suppressed child under its
// gating ancestor's lane. Both require `cyclic(id)` already in scope (from `CYCLE_CTE`).

/// `blocked_gating(id)`: open, live, non-archived items that are BLOCKED — in a dep-cycle, or with a
/// present `dep` on an item still open/undeleted. Carries no `now` parameter.
const BLOCKED_GATING_CTE: &str = "blocked_gating(id) AS (
    SELECT i.id FROM items i
    WHERE COALESCE(i.deleted,'0')<>'1' AND i.archived IS NULL AND i.status='open'
      AND ( i.id IN (SELECT id FROM cyclic)
         OR EXISTS (SELECT 1 FROM present_edges d JOIN items b ON b.id=d.to_id
                    WHERE d.from_id=i.id AND d.kind='dep'
                      AND COALESCE(b.deleted,'0')<>'1' AND b.status<>'closed') ) )";

/// `deferred_gating(id)`: open, live, non-archived items deferred into the future (`defer_until >
/// ?1`) that are NOT blocked (no cycle, no open dep) — disjoint from `blocked_gating` by the blocker
/// precedence (§2). Binds `now` as `?1`.
const DEFERRED_GATING_CTE: &str = "deferred_gating(id) AS (
    SELECT i.id FROM items i
    WHERE COALESCE(i.deleted,'0')<>'1' AND i.archived IS NULL AND i.status='open'
      AND i.defer_until > ?1
      AND i.id NOT IN (SELECT id FROM cyclic)
      AND NOT EXISTS (SELECT 1 FROM present_edges d JOIN items b ON b.id=d.to_id
                      WHERE d.from_id=i.id AND d.kind='dep'
                        AND COALESCE(b.deleted,'0')<>'1' AND b.status<>'closed') )";

/// `actionable_items(id, status)`: THE actionability filter — live, non-archived, not in a dep-cycle,
/// not suppressed by a gating ancestor (07a.2), not deferred into the future, not closed-masked
/// (07a.3), and with no present `dep` on an item still open/undeleted. It carries each item's `status`
/// so the caller picks which lifecycle status to admit — that is the ONLY difference between `ready`
/// (open) and `in_progress` (claimed). The finish-first tier signals (§3) select from this same CTE,
/// so a tier can never widen the actionable set behind `ready`/`in_progress`' back.
///
/// Requires `cyclic(id)`, `blocked_gating(id)` and `deferred_gating(id)` already in scope (see
/// [`actionable_prelude`]); binds `now` as `?1`.
const ACTIONABLE_CTE: &str = "
    -- the transitive descendants of any gating item, walked DOWN the parent edges
    -- (from_id=child, to_id=parent). UNION (not UNION ALL) dedups, so a transient
    -- convergence-delivered parent loop still terminates (cycles are write-seam rejected).
    suppressed(id) AS (
        SELECT e.from_id FROM present_edges e
          WHERE e.kind='parent'
            AND ( e.to_id IN (SELECT id FROM blocked_gating)
               OR e.to_id IN (SELECT id FROM deferred_gating) )
        UNION
        SELECT e.from_id FROM present_edges e JOIN suppressed s ON e.to_id = s.id
          WHERE e.kind='parent'
    ),
    actionable_items(id, status) AS (
        SELECT i.id, i.status FROM items i
        WHERE COALESCE(i.deleted,'0')<>'1' AND i.archived IS NULL
          AND i.id NOT IN (SELECT id FROM cyclic)
          AND i.id NOT IN (SELECT id FROM suppressed)
          AND (i.defer_until IS NULL OR i.defer_until <= ?1)
          -- 07a.3 closed-mask: a still-OPEN child whose every LIVE parent is closed is
          -- effectively closed — excluded from ready/next (it groups under the closed lane and
          -- carries parent_closed_reason; §4). Only bites for open items; an in_progress child
          -- shows its own status. A deleted parent is not a live parent.
          AND ( i.status<>'open'
             OR NOT ( EXISTS (SELECT 1 FROM present_edges pe JOIN items p ON p.id=pe.to_id
                              WHERE pe.from_id=i.id AND pe.kind='parent'
                                AND COALESCE(p.deleted,'0')<>'1')
                  AND NOT EXISTS (SELECT 1 FROM present_edges pe JOIN items p ON p.id=pe.to_id
                              WHERE pe.from_id=i.id AND pe.kind='parent'
                                AND COALESCE(p.deleted,'0')<>'1' AND p.status<>'closed') ) )
          AND NOT EXISTS (
              SELECT 1 FROM present_edges d JOIN items b ON b.id=d.to_id
              WHERE d.from_id=i.id AND d.kind='dep'
                AND COALESCE(b.deleted,'0')<>'1' AND b.status<>'closed') )";

/// The full CTE prelude behind every actionability query: the cycle walk, the two gating halves
/// (07a.2 — open, live, non-archived items that are NOT ready, whose descendants are suppressed;
/// their union is the old combined gating set), then [`ACTIONABLE_CTE`]. Follows `WITH RECURSIVE`.
/// Binds `now` as `?1`.
fn actionable_prelude() -> String {
    format!("{CYCLE_CTE},\n cyclic(id) AS (SELECT DISTINCT start FROM walk WHERE node = start),\n {BLOCKED_GATING_CTE},\n {DEFERRED_GATING_CTE},{ACTIONABLE_CTE}")
}

/// Items of a given `status` that are actionable ([`ACTIONABLE_CTE`]). The shared engine behind
/// `ready` (status `open`) and `in_progress` (claimed work).
///
/// Fallible (#76u.14): the long-lived MCP/embed read seams call this through `read::next`/`ready`,
/// so a db error during derivation (e.g. `SQLITE_BUSY`) is surfaced as an `Err` to be mapped to the
/// structured `io` kind — never an `.unwrap()` that unwinds the handler.
fn actionable(conn: &Connection, now: &str, status: &str) -> rusqlite::Result<Vec<String>> {
    let sql = format!(
        "WITH RECURSIVE {prelude}
             SELECT id FROM actionable_items WHERE status=?2
             ORDER BY id",
        prelude = actionable_prelude()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![now, status], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// ready = open, not deleted, not in a dep-cycle, no present `dep` on an item still
/// open/undeleted, and not deferred into the future. Startable, not-yet-begun work — it never
/// includes already-claimed (`in_progress`) items, so two agents never both treat one as free.
pub fn ready(conn: &Connection, now: &str) -> rusqlite::Result<Vec<String>> {
    actionable(conn, now, "open")
}

/// in_progress = the *same* actionability as [`ready`] (unblocked, acyclic, not deferred) but
/// for already-claimed work (`status='in_progress'`). `next` surfaces these alongside `ready`
/// and ranks them first so an agent never loses sight of work it has started; `ready` and
/// `blocked` are unchanged and ignore this set (the three-state model above is about `open`).
pub fn in_progress(conn: &Connection, now: &str) -> rusqlite::Result<Vec<String>> {
    actionable(conn, now, "in_progress")
}

/// deferred = open, not deleted, not archived, not in a dep-cycle, with NO present `dep` on an
/// item still open/undeleted, AND `defer_until` set to a future instant (`> now`). The third
/// derived state (see the module docs): an item here appears in neither `ready` (its defer date
/// has not passed) nor `blocked` (it has no open blocker) — the blocker takes precedence, so a
/// deferred-AND-blocked item is `blocked`, never here. Disjoint from `ready`/`blocked` by
/// construction, which is what lets the lane verbs partition the open space cleanly (C3 #916.4).
pub fn deferred(conn: &Connection, now: &str) -> rusqlite::Result<Vec<String>> {
    let sql = format!(
        "WITH RECURSIVE {CYCLE_CTE},
               cyclic(id) AS (SELECT DISTINCT start FROM walk WHERE node = start)
             SELECT i.id FROM items i
             WHERE COALESCE(i.deleted,'0')<>'1' AND i.archived IS NULL AND i.status='open'
               AND i.id NOT IN (SELECT id FROM cyclic)
               AND i.defer_until IS NOT NULL AND i.defer_until > ?1
               AND NOT EXISTS (
                   SELECT 1 FROM present_edges d JOIN items b ON b.id=d.to_id
                   WHERE d.from_id=i.id AND d.kind='dep'
                     AND COALESCE(b.deleted,'0')<>'1' AND b.status<>'closed')
             ORDER BY i.id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![now], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// blocked = open, not deleted, not archived, and either has an open blocker dep or is in a cycle.
pub fn blocked(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let sql = format!(
        "WITH RECURSIVE {CYCLE_CTE},
               cyclic(id) AS (SELECT DISTINCT start FROM walk WHERE node = start)
             SELECT i.id FROM items i
             WHERE COALESCE(i.deleted,'0')<>'1' AND i.archived IS NULL AND i.status='open'
               AND ( i.id IN (SELECT id FROM cyclic)
                  OR EXISTS (
                       SELECT 1 FROM present_edges d JOIN items b ON b.id=d.to_id
                       WHERE d.from_id=i.id AND d.kind='dep'
                         AND COALESCE(b.deleted,'0')<>'1' AND b.status<>'closed') )
             ORDER BY i.id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// The live items SUPPRESSED by a BLOCKED ancestor (07a.3 §8): every open/in_progress, live,
/// non-archived transitive descendant — walked DOWN the `parent` edges — of an item in
/// [`BLOCKED_GATING_CTE`]. The effective-lane search grouping puts these under the **blocked** lane
/// (a suppressed child groups by its gating ancestor's lane), since `ready`/`next` already exclude
/// them. Id-sorted, deterministic; the `UNION` walk terminates on a transient parent loop (cycles
/// are write-seam rejected). Status-filtered to open/in_progress because closed/archived descendants
/// belong to higher-precedence lanes the read layer classifies first.
pub fn suppressed_by_blocked(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let sql = format!(
        "WITH RECURSIVE {CYCLE_CTE},
               cyclic(id) AS (SELECT DISTINCT start FROM walk WHERE node = start),
               {BLOCKED_GATING_CTE},
               sb(id) AS (
                   SELECT e.from_id FROM present_edges e
                     WHERE e.kind='parent' AND e.to_id IN (SELECT id FROM blocked_gating)
                   UNION
                   SELECT e.from_id FROM present_edges e JOIN sb ON e.to_id = sb.id
                     WHERE e.kind='parent'
               )
             SELECT i.id FROM items i
             WHERE i.id IN (SELECT id FROM sb)
               AND COALESCE(i.deleted,'0')<>'1' AND i.archived IS NULL
               AND i.status IN ('open','in_progress')
             ORDER BY i.id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// The live items suppressed ONLY by a DEFERRED ancestor (07a.3 §8): open/in_progress, live,
/// non-archived descendants of a [`DEFERRED_GATING_CTE`] item that are NOT also descendants of any
/// [`BLOCKED_GATING_CTE`] item — most-restrictive-wins (§2) ranks blocked above deferred, so a child
/// under both gates groups under blocked, not here. The effective-lane search grouping puts these
/// under the **deferred** lane. Disjoint from [`suppressed_by_blocked`] by construction; together
/// they are exactly the suppression `ready`/`next` enforce. Id-sorted; binds `now` as `?1`.
pub fn suppressed_by_deferred_only(conn: &Connection, now: &str) -> rusqlite::Result<Vec<String>> {
    let sql = format!(
        "WITH RECURSIVE {CYCLE_CTE},
               cyclic(id) AS (SELECT DISTINCT start FROM walk WHERE node = start),
               {BLOCKED_GATING_CTE},
               {DEFERRED_GATING_CTE},
               sb(id) AS (
                   SELECT e.from_id FROM present_edges e
                     WHERE e.kind='parent' AND e.to_id IN (SELECT id FROM blocked_gating)
                   UNION
                   SELECT e.from_id FROM present_edges e JOIN sb ON e.to_id = sb.id
                     WHERE e.kind='parent'
               ),
               sd(id) AS (
                   SELECT e.from_id FROM present_edges e
                     WHERE e.kind='parent' AND e.to_id IN (SELECT id FROM deferred_gating)
                   UNION
                   SELECT e.from_id FROM present_edges e JOIN sd ON e.to_id = sd.id
                     WHERE e.kind='parent'
               )
             SELECT i.id FROM items i
             WHERE i.id IN (SELECT id FROM sd)
               AND i.id NOT IN (SELECT id FROM sb)
               AND COALESCE(i.deleted,'0')<>'1' AND i.archived IS NULL
               AND i.status IN ('open','in_progress')
             ORDER BY i.id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![now], |r| r.get::<_, String>(0))?;
    rows.collect()
}

// ---- finish-first tier signals (docs/specs/next-finish-first-tiers.md §3) ---
//
// Tier *assignment* only: a pure graph fact ("closeable now" / "child of a started container"),
// the same convergent SQL over the materialized views as ready/blocked/suppressed. RANKING stays
// plugin policy (see the note at the foot of this module) — these two say nothing about order
// beyond a deterministic id sort, and read only `status` + the `parent` edge, never plugin
// vocabulary ("epic", priority). The facade composes the final order as (tier, cluster, rank).

/// One `next` candidate with its tier signals — the row [`next_candidates`] returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextCandidate {
    /// The item id.
    pub id: String,
    /// Its lifecycle status: `open` (a [`ready`] item) or `in_progress` (claimed work).
    pub status: String,
    /// Tier 1: claimed work with no live non-closed child — closeable right now.
    pub finishable: bool,
    /// Tier 2: the actionable in-progress `parent` promoting this ready child into its cluster
    /// (smallest id on multi-parent). `None` for everything else.
    pub promoter: Option<String>,
}

/// **THE tier-assignment derivation** — every actionable candidate (`ready ∪ in_progress`, the set
/// `next` recommends over) with its tier signals, in ONE pass over the substrate (§3).
///
/// This is the single query behind the finish-first tiers and the single SQL definition of both
/// signals: [`finishable_in_progress`] and [`promoted_children`] are thin projections of it, so a
/// standalone signal can never drift from what `next` actually orders by. The one pass is the point
/// — every derivation re-walks the recursive cycle + suppression CTEs (~5ms on a 150-item board),
/// so deriving the signals separately quadrupled `next`'s cost for no gain (the `xn8s` ratio guard
/// in the facade caught exactly that regression).
///
/// Pure graph facts over `status` + the `parent` edge — no plugin vocabulary, no ranking. Drawn from
/// the shared [`ACTIONABLE_CTE`], so 07a suppression and the closed-mask apply unchanged and the
/// candidate set is exactly `ready ∪ in_progress` **by construction**, not by a second definition
/// that could drift. Id-sorted, deterministic, fallible.
pub fn next_candidates(conn: &Connection, now: &str) -> rusqlite::Result<Vec<NextCandidate>> {
    let sql = format!(
        "WITH RECURSIVE {prelude}
             SELECT a.id, a.status,
                    -- Tier 1: no live (undeleted) child still open or in_progress. A closed child
                    -- does not hold its parent back; covers the childless leaf too.
                    NOT EXISTS (
                        SELECT 1 FROM present_edges c JOIN items ch ON ch.id=c.from_id
                        WHERE c.to_id=a.id AND c.kind='parent'
                          AND COALESCE(ch.deleted,'0')<>'1'
                          AND ch.status IN ('open','in_progress')) AS finishable,
                    -- Tier 2: the smallest actionable in-progress parent of this OPEN child. Both
                    -- sides come from actionable_items, so a promoted child's parent is always a
                    -- header (§3.3) and a suppressed child is never promoted.
                    CASE WHEN a.status='open' THEN (
                        SELECT MIN(e.to_id) FROM present_edges e
                        WHERE e.kind='parent' AND e.from_id=a.id
                          AND e.to_id IN (SELECT id FROM actionable_items WHERE status='in_progress')
                    ) END AS promoter
             FROM actionable_items a
             WHERE a.status IN ('open','in_progress')
             ORDER BY a.id",
        prelude = actionable_prelude()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![now], |r| {
        Ok(NextCandidate {
            id: r.get(0)?,
            status: r.get(1)?,
            finishable: r.get::<_, i64>(2)? != 0,
            promoter: r.get(3)?,
        })
    })?;
    rows.collect()
}

/// The **Tier-1 set** — actionable claimed work that can be CLOSED NOW: [`in_progress`] items with
/// no live, non-closed child. Covers the all-done/empty epic **and** the in-progress leaf (a claimed
/// bug or chore), which is what makes the rule type-agnostic. A child that is closed does not hold
/// its parent back; one still `open`/`in_progress` does. "Live" = not deleted.
///
/// A projection of [`next_candidates`], hence a subset of [`in_progress`] by construction: it
/// inherits the 07a suppression + closed-mask unchanged, so a claimed item under a blocked ancestor
/// stays hidden here too. Id-sorted, deterministic, fallible. `next` reads [`next_candidates`]
/// directly; this is the standalone view.
pub fn finishable_in_progress(conn: &Connection, now: &str) -> rusqlite::Result<Vec<String>> {
    Ok(next_candidates(conn, now)?
        .into_iter()
        .filter(|c| c.status == "in_progress" && c.finishable)
        .map(|c| c.id)
        .collect())
}

/// The **Tier-2 child→cluster map** — `(child_id, promoting_parent_id)` for every [`ready`] child
/// whose `parent` is an actionable in-progress item (∈ [`in_progress`], i.e. a genuine Tier-2
/// header). Both sides are drawn from the same [`ACTIONABLE_CTE`], which gives two guarantees the
/// facade relies on (§3.3): the parent is always in `in_progress \ finishable_in_progress`, so **no
/// cluster is ever headerless**; and a child under a blocked ancestor is not in `ready`, so 07a
/// suppression is untouched.
///
/// One row per child: with several in-progress parents (multi-parent — impossible in
/// `issue-tracker`, but the core is generic) the lexicographically smallest parent id wins, so
/// clustering is unambiguous. Sorted by child id, deterministic, fallible.
pub fn promoted_children(conn: &Connection, now: &str) -> rusqlite::Result<Vec<(String, String)>> {
    Ok(next_candidates(conn, now)?
        .into_iter()
        .filter_map(|c| c.promoter.map(|p| (c.id, p)))
        .collect())
}

/// Direct fan-out leverage: how many still-open (open *or* in_progress), undeleted items
/// depend directly on `id` via a present `dep` edge — i.e. how many items closing `id` would
/// unblock. Deterministic, over the materialized views only (nexus-flow-bcj). A closed or
/// deleted dependent is not counted (it is no longer waiting). Fallible like the other derivations
/// (#76u.14): `prime` reads it on the long-lived seam, so a db error surfaces as `io`, not a panic.
pub fn dependents_count(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    let sql = "SELECT COUNT(*) FROM present_edges d JOIN items i ON i.id=d.from_id
               WHERE d.to_id=?1 AND d.kind='dep'
                 AND COALESCE(i.deleted,'0')<>'1' AND i.status<>'closed'";
    Ok(conn.query_row(sql, params![id], |r| r.get::<_, i64>(0))? as usize)
}

// Ranking ("what to work on next") is deliberately NOT in the meaning-free core: priority
// semantics and tiebreaks are plugin policy. The CLI ranks `ready` ∪ `in_progress` through the
// active `PluginConfig` (`rank(cfg)`), so the core exposes the actionable *sets* and nothing
// about their order. (A former `derive::next` hardcoded a 0–4 priority range — removed in
// nexus-flow-1q6 as it baked a plugin assumption into the core.)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EdgeKind;
    use crate::store::Store;

    const NOW: &str = "2026-06-08T00:00:00Z";

    fn task(s: &mut Store, id: &str) {
        s.create_item(id, "task", id, "x");
    }

    #[test]
    fn open_task_with_no_deps_is_ready() {
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.A");
        assert_eq!(
            ready(s.connection(), NOW).unwrap(),
            vec!["c1.A".to_string()]
        );
    }

    #[test]
    fn task_blocked_by_open_dependency_is_not_ready() {
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.A");
        task(&mut s, "c1.B");
        s.add_edge("c1.A", "c1.B", EdgeKind::Dep, "x"); // A depends on B
        assert_eq!(
            ready(s.connection(), NOW).unwrap(),
            vec!["c1.B".to_string()]
        );
        assert_eq!(blocked(s.connection()).unwrap(), vec!["c1.A".to_string()]);
    }

    #[test]
    fn closing_the_blocker_makes_the_dependent_ready() {
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.A");
        task(&mut s, "c1.B");
        s.add_edge("c1.A", "c1.B", EdgeKind::Dep, "x");
        s.set_field("c1.B", "status", Some("closed".into()), "x");
        assert_eq!(
            ready(s.connection(), NOW).unwrap(),
            vec!["c1.A".to_string()]
        );
    }

    #[test]
    fn deferred_task_is_not_ready_until_the_date_passes() {
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.A");
        s.set_field(
            "c1.A",
            "defer_until",
            Some("2026-12-01T00:00:00Z".into()),
            "x",
        );
        assert!(ready(s.connection(), NOW).unwrap().is_empty());
        assert_eq!(
            ready(s.connection(), "2027-01-01T00:00:00Z").unwrap(),
            vec!["c1.A".to_string()]
        );
    }

    #[test]
    fn in_progress_returns_actionable_claimed_work_only() {
        // `in_progress` is the same actionability filter as `ready` (unblocked, acyclic,
        // not deferred) but for already-claimed work. `ready` must still exclude it.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.A"); // claimed, unblocked  → in_progress
        task(&mut s, "c1.B"); // claimed but blocked by open C → excluded
        task(&mut s, "c1.C"); // open blocker
        task(&mut s, "c1.D"); // claimed but deferred into the future → excluded
        s.add_edge("c1.B", "c1.C", EdgeKind::Dep, "x");
        s.set_field("c1.A", "status", Some("in_progress".into()), "x");
        s.set_field("c1.B", "status", Some("in_progress".into()), "x");
        s.set_field("c1.D", "status", Some("in_progress".into()), "x");
        s.set_field(
            "c1.D",
            "defer_until",
            Some("2026-12-01T00:00:00Z".into()),
            "x",
        );

        assert_eq!(
            in_progress(s.connection(), NOW).unwrap(),
            vec!["c1.A".to_string()],
            "only unblocked, non-deferred claimed work is actionable"
        );
        // ready is unchanged: it never reports in_progress items (no double-claim risk).
        let r = ready(s.connection(), NOW).unwrap();
        assert!(
            !r.contains(&"c1.A".to_string()) && !r.contains(&"c1.B".to_string()),
            "ready excludes in_progress entirely: {r:?}"
        );
        assert_eq!(
            r,
            vec!["c1.C".to_string()],
            "only the open blocker is ready"
        );
    }

    #[test]
    fn deferred_is_open_unblocked_acyclic_work_with_a_future_defer_date() {
        // C3 (#916.4): the deferred lane is the third derived state — open, NO open blocker, not
        // cyclic, but `defer_until` is still in the future. It is disjoint from both `ready` and
        // `blocked` (the three-state model), so the same item never appears in two lanes.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.A"); // open, deferred into the future → deferred
        task(&mut s, "c1.B"); // open, no defer → ready (NOT deferred)
        task(&mut s, "c1.C"); // open, deferred BUT blocked by D → blocked (blocker precedence)
        task(&mut s, "c1.D"); // open blocker of C
        s.add_edge("c1.C", "c1.D", EdgeKind::Dep, "x");
        let future = "2026-12-01T00:00:00Z";
        s.set_field("c1.A", "defer_until", Some(future.into()), "x");
        s.set_field("c1.C", "defer_until", Some(future.into()), "x");

        assert_eq!(
            deferred(s.connection(), NOW).unwrap(),
            vec!["c1.A".to_string()],
            "only the open, unblocked, future-deferred item is deferred"
        );
        // C is blocked (blocker takes precedence over deferral), never deferred.
        assert_eq!(blocked(s.connection()).unwrap(), vec!["c1.C".to_string()]);
        // Once the defer date passes, A leaves the deferred lane and becomes ready.
        let past = "2027-01-01T00:00:00Z";
        assert!(
            deferred(s.connection(), past).unwrap().is_empty(),
            "no longer deferred once the date passes"
        );
        assert!(ready(s.connection(), past)
            .unwrap()
            .contains(&"c1.A".to_string()));
    }

    #[test]
    fn deferred_excludes_archived_in_progress_and_closed() {
        // The deferred lane is open work only, and (like every enumerating derivation) excludes
        // archived items — so the partition stays clean.
        let mut s = Store::open_in_memory(1);
        let future = "2026-12-01T00:00:00Z";
        for id in ["c1.A", "c1.B", "c1.C"] {
            task(&mut s, id);
            s.set_field(id, "defer_until", Some(future.into()), "x");
        }
        s.set_field("c1.A", "status", Some("in_progress".into()), "x"); // claimed → not deferred
        s.set_field("c1.B", "status", Some("closed".into()), "x"); // closed → not deferred
        s.set_field("c1.C", "archived", Some("2026-06-23T00:00:00Z".into()), "x"); // archived → not deferred
        assert!(
            deferred(s.connection(), NOW).unwrap().is_empty(),
            "deferred is open, non-archived work only"
        );
    }

    #[test]
    fn a_blocked_parent_suppresses_its_children_from_ready() {
        // 07a.2: a child of a blocked (or deferred) ancestor is suppressed from ready/next — the
        // gating-parent inversion of sp6.7 ②c. P is blocked by its own open dep B; child C is ready
        // on its own terms but is suppressed because its parent P is blocked.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.P");
        task(&mut s, "c1.B"); // P's blocker
        task(&mut s, "c1.C"); // child of P, open + no deps of its own
        s.add_edge("c1.P", "c1.B", EdgeKind::Dep, "x"); // P depends on open B → P blocked
        s.add_parent("c1.C", "c1.P", "x");

        let r = ready(s.connection(), NOW).unwrap();
        assert!(
            !r.contains(&"c1.C".to_string()),
            "C is suppressed: its parent P is blocked"
        );
        assert!(
            r.contains(&"c1.B".to_string()),
            "B (the unparented blocker) is itself ready"
        );

        // Closing B unblocks P → C is no longer gated and becomes ready.
        s.set_field("c1.B", "status", Some("closed".into()), "x");
        let r2 = ready(s.connection(), NOW).unwrap();
        assert!(
            r2.contains(&"c1.C".to_string()),
            "P now ready → C no longer suppressed"
        );
    }

    #[test]
    fn a_deferred_parent_suppresses_children_but_ready_in_progress_parents_do_not() {
        // 07a.2 suppression: the gating ancestors are exactly blocked ∪ deferred. A DEFERRED parent
        // suppresses; a ready / in_progress parent does NOT. (The CLOSED-parent case is the separate
        // closed-MASK mechanism — see `an_open_child_of_all_closed_parents_is_closed_masked…`.)
        let future = "2026-12-01T00:00:00Z";
        let mut s = Store::open_in_memory(1);
        for p in ["c1.PR", "c1.PI", "c1.PD"] {
            task(&mut s, p);
        }
        // PR ready (open, no deps) · PI in_progress · PD deferred (future defer date).
        s.set_field("c1.PI", "status", Some("in_progress".into()), "x");
        s.set_field("c1.PD", "defer_until", Some(future.into()), "x");
        // One open, no-dep child under each parent.
        for (c, p) in [("c1.CR", "c1.PR"), ("c1.CI", "c1.PI"), ("c1.CD", "c1.PD")] {
            task(&mut s, c);
            s.add_parent(c, p, "x");
        }

        let r = ready(s.connection(), NOW).unwrap();
        assert!(
            r.contains(&"c1.CR".to_string()),
            "ready parent does not gate"
        );
        assert!(
            r.contains(&"c1.CI".to_string()),
            "in_progress parent does not gate"
        );
        assert!(
            !r.contains(&"c1.CD".to_string()),
            "a deferred parent suppresses its child"
        );
    }

    #[test]
    fn suppression_cascades_to_a_grandchild_and_covers_the_in_progress_lane() {
        // Suppression is transitive (a blocked grandparent gates a grandchild) and applies to the
        // in_progress lane too (so `next` hides a claimed child of a blocked
        // ancestor, not just an open one).
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.G"); // grandparent
        task(&mut s, "c1.B"); // grandparent's open blocker
        task(&mut s, "c1.P"); // parent (child of G)
        task(&mut s, "c1.C"); // grandchild (child of P), claimed
        s.add_edge("c1.G", "c1.B", EdgeKind::Dep, "x"); // G blocked
        s.add_parent("c1.P", "c1.G", "x");
        s.add_parent("c1.C", "c1.P", "x");
        s.set_field("c1.C", "status", Some("in_progress".into()), "x");

        assert!(
            !ready(s.connection(), NOW)
                .unwrap()
                .contains(&"c1.P".to_string()),
            "P (open child of blocked G) is suppressed from ready"
        );
        assert!(
            !in_progress(s.connection(), NOW)
                .unwrap()
                .contains(&"c1.C".to_string()),
            "the claimed grandchild C is suppressed from the in_progress lane too"
        );
    }

    #[test]
    fn an_open_child_of_all_closed_parents_is_closed_masked_out_of_ready() {
        // 07a.3 closed-mask: when ALL of a still-open child's (live) parents are closed, the child is
        // effectively closed — excluded from ready/next. With any open parent left it is NOT masked
        // (it is instead gated by that open parent's lane, §1/§2).
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.P1");
        task(&mut s, "c1.P2");
        task(&mut s, "c1.C"); // open child, no deps of its own
        s.add_parent("c1.C", "c1.P1", "x");
        s.add_parent("c1.C", "c1.P2", "x");
        assert!(
            ready(s.connection(), NOW)
                .unwrap()
                .contains(&"c1.C".to_string()),
            "both parents open → C is ready, not masked"
        );
        s.set_field("c1.P1", "status", Some("closed".into()), "x");
        assert!(
            ready(s.connection(), NOW)
                .unwrap()
                .contains(&"c1.C".to_string()),
            "one open parent remains → C is still ready"
        );
        s.set_field("c1.P2", "status", Some("closed".into()), "x");
        assert!(
            !ready(s.connection(), NOW)
                .unwrap()
                .contains(&"c1.C".to_string()),
            "ALL parents closed → C is closed-masked out of ready"
        );
    }

    #[test]
    fn suppressed_split_groups_children_by_blocked_vs_deferred_only_ancestor() {
        // 07a.3 §8 (effective-lane search grouping): a child suppressed by a BLOCKED ancestor groups
        // under the blocked lane; one suppressed ONLY by a deferred ancestor groups under deferred.
        // Most-restrictive-wins (§2): a child under BOTH a blocked and a deferred ancestor is
        // blocked, never deferred-only. The walk is transitive (a grandchild counts).
        let mut s = Store::open_in_memory(1);
        let future = "2026-12-01T00:00:00Z";
        // PB is blocked by its own open dep XB.
        task(&mut s, "c1.PB");
        task(&mut s, "c1.XB");
        s.add_edge("c1.PB", "c1.XB", EdgeKind::Dep, "x");
        // PD is deferred into the future (and otherwise unblocked).
        task(&mut s, "c1.PD");
        s.set_field("c1.PD", "defer_until", Some(future.into()), "x");
        // CB under the blocked PB; CD under the deferred PD; CBD under both.
        task(&mut s, "c1.CB");
        s.add_parent("c1.CB", "c1.PB", "x");
        task(&mut s, "c1.CD");
        s.add_parent("c1.CD", "c1.PD", "x");
        task(&mut s, "c1.CBD");
        s.add_parent("c1.CBD", "c1.PB", "x");
        s.add_parent("c1.CBD", "c1.PD", "x");
        // GB is a grandchild of the blocked PB (child of CB) → transitively suppressed by blocked.
        task(&mut s, "c1.GB");
        s.add_parent("c1.GB", "c1.CB", "x");

        let sb = suppressed_by_blocked(s.connection()).unwrap();
        assert_eq!(
            sb,
            vec![
                "c1.CB".to_string(),
                "c1.CBD".to_string(),
                "c1.GB".to_string()
            ],
            "every transitive descendant of a blocked ancestor (incl. the multi-parent CBD)"
        );
        let sd = suppressed_by_deferred_only(s.connection(), NOW).unwrap();
        assert_eq!(
            sd,
            vec!["c1.CD".to_string()],
            "CBD is under a blocked ancestor too → blocked-gated, not deferred-only"
        );
    }

    #[test]
    fn suppressed_split_is_consistent_with_ready_exclusion() {
        // The two split sets together are exactly the suppression `ready` already enforces: an OPEN,
        // otherwise-actionable child that is suppressed (by either gate) must be absent from ready,
        // and the split must place it in exactly one of the two sets (they are disjoint).
        let mut s = Store::open_in_memory(1);
        let future = "2026-12-01T00:00:00Z";
        task(&mut s, "c1.PB");
        task(&mut s, "c1.XB");
        s.add_edge("c1.PB", "c1.XB", EdgeKind::Dep, "x"); // PB blocked
        task(&mut s, "c1.PD");
        s.set_field("c1.PD", "defer_until", Some(future.into()), "x"); // PD deferred
        task(&mut s, "c1.CB");
        s.add_parent("c1.CB", "c1.PB", "x");
        task(&mut s, "c1.CD");
        s.add_parent("c1.CD", "c1.PD", "x");

        let r = ready(s.connection(), NOW).unwrap();
        let sb = suppressed_by_blocked(s.connection()).unwrap();
        let sd = suppressed_by_deferred_only(s.connection(), NOW).unwrap();
        for c in ["c1.CB", "c1.CD"] {
            assert!(!r.contains(&c.to_string()), "{c} is suppressed from ready");
        }
        // Disjoint and each lands in exactly one set.
        assert!(sb.contains(&"c1.CB".to_string()) && !sd.contains(&"c1.CB".to_string()));
        assert!(sd.contains(&"c1.CD".to_string()) && !sb.contains(&"c1.CD".to_string()));
    }

    #[test]
    fn suppression_terminates_on_a_transient_parent_cycle() {
        // 07a.2 / Test #1: `Store::add_parent` has NO acyclicity check, so a convergence-delivered
        // transient A↔B parent cycle is constructible. The recursive `suppressed` CTE uses `UNION`
        // (set semantics), so the walk must TERMINATE — the test returning at all is the proof —
        // even though A is B's parent and B is A's parent. With A blocked (its own open dep), both A
        // and B are suppressed out of `ready`.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.A");
        task(&mut s, "c1.B");
        task(&mut s, "c1.X"); // A's open blocker
        s.add_edge("c1.A", "c1.X", EdgeKind::Dep, "x"); // A is blocked
        s.add_parent("c1.A", "c1.B", "x");
        s.add_parent("c1.B", "c1.A", "x"); // A↔B parent cycle

        let r = ready(s.connection(), NOW).unwrap(); // terminates (no infinite walk)
        assert!(
            r.contains(&"c1.X".to_string()),
            "the unparented blocker is ready"
        );
        assert!(
            !r.contains(&"c1.A".to_string()) && !r.contains(&"c1.B".to_string()),
            "the cycle members are excluded (A own-blocked, B suppressed by blocked A): {r:?}"
        );
        // The split walk terminates too, placing B under the blocked lane.
        assert!(suppressed_by_blocked(s.connection())
            .unwrap()
            .contains(&"c1.B".to_string()));
    }

    // ---- finish-first tier signals (next-finish-first-tiers §3) -------------

    #[test]
    fn finishable_includes_an_in_progress_epic_whose_children_are_all_closed() {
        // Tier 1 (§3.1): the started epic with nothing left open is exactly the item an agent can
        // close right now. It is `in_progress` and has no live non-closed child.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.E");
        s.set_field("c1.E", "status", Some("in_progress".into()), "x");
        for c in ["c1.C1", "c1.C2"] {
            task(&mut s, c);
            s.add_parent(c, "c1.E", "x");
            s.set_field(c, "status", Some("closed".into()), "x");
        }
        assert_eq!(
            finishable_in_progress(s.connection(), NOW).unwrap(),
            vec!["c1.E".to_string()],
            "all children closed → the epic is finishable"
        );
    }

    #[test]
    fn finishable_includes_an_in_progress_leaf() {
        // Type-agnostic (§7): a claimed item with no children at all is finishable — "finish what
        // you started" is not an epic-only rule.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.A");
        s.set_field("c1.A", "status", Some("in_progress".into()), "x");
        assert_eq!(
            finishable_in_progress(s.connection(), NOW).unwrap(),
            vec!["c1.A".to_string()],
            "a childless in_progress item is finishable"
        );
    }

    #[test]
    fn finishable_excludes_an_in_progress_epic_with_an_open_child() {
        // §3.1: a live open child disqualifies the parent — it is not yet closeable.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.E");
        s.set_field("c1.E", "status", Some("in_progress".into()), "x");
        task(&mut s, "c1.C");
        s.add_parent("c1.C", "c1.E", "x");
        assert!(
            finishable_in_progress(s.connection(), NOW)
                .unwrap()
                .is_empty(),
            "an open child means the epic cannot be finished yet"
        );
    }

    #[test]
    fn finishable_excludes_an_in_progress_epic_with_an_in_progress_child() {
        // §3.1 / §7: "live non-closed" covers in_progress children too, not just open ones.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.E");
        s.set_field("c1.E", "status", Some("in_progress".into()), "x");
        task(&mut s, "c1.C");
        s.add_parent("c1.C", "c1.E", "x");
        s.set_field("c1.C", "status", Some("in_progress".into()), "x");
        assert_eq!(
            finishable_in_progress(s.connection(), NOW).unwrap(),
            vec!["c1.C".to_string()],
            "the in_progress child is itself finishable; its parent is not"
        );
    }

    #[test]
    fn finishable_excludes_a_deleted_or_closed_child_bearing_epic_correctly() {
        // "Live" = not deleted (§3.1): a DELETED open child does not hold its parent back, while a
        // live open child does. Guards the COALESCE(deleted) predicate.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.E");
        s.set_field("c1.E", "status", Some("in_progress".into()), "x");
        task(&mut s, "c1.C");
        s.add_parent("c1.C", "c1.E", "x");
        assert!(
            finishable_in_progress(s.connection(), NOW)
                .unwrap()
                .is_empty(),
            "live open child holds the epic back"
        );
        s.delete_item("c1.C", "x");
        assert_eq!(
            finishable_in_progress(s.connection(), NOW).unwrap(),
            vec!["c1.E".to_string()],
            "a deleted child is not a live child → the epic is finishable"
        );
    }

    #[test]
    fn finishable_respects_suppression_by_a_blocked_ancestor() {
        // 07a.2 tracking (§0/§3.1): tiering never widens the actionable set. A claimed, childless
        // item under a BLOCKED ancestor is suppressed from `in_progress`, so it is not finishable.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.P");
        task(&mut s, "c1.B"); // P's open blocker → P blocked → suppresses its subtree
        s.add_edge("c1.P", "c1.B", EdgeKind::Dep, "x");
        task(&mut s, "c1.C");
        s.add_parent("c1.C", "c1.P", "x");
        s.set_field("c1.C", "status", Some("in_progress".into()), "x");
        assert!(
            !in_progress(s.connection(), NOW)
                .unwrap()
                .contains(&"c1.C".to_string()),
            "precondition: C is suppressed out of the in_progress set"
        );
        assert!(
            finishable_in_progress(s.connection(), NOW)
                .unwrap()
                .is_empty(),
            "a suppressed claimed item is not finishable either"
        );
    }

    #[test]
    fn finishable_excludes_own_blocked_deferred_archived_and_non_claimed_work() {
        // The Tier-1 set is a SUBSET of `in_progress` (§3.1) — it inherits every actionability rule
        // and admits claimed work only.
        let mut s = Store::open_in_memory(1);
        let future = "2026-12-01T00:00:00Z";
        task(&mut s, "c1.OPEN"); // open, ready → not claimed → not finishable
        task(&mut s, "c1.BLK"); // in_progress but own open dep
        task(&mut s, "c1.X"); // BLK's open blocker
        s.add_edge("c1.BLK", "c1.X", EdgeKind::Dep, "x");
        task(&mut s, "c1.DEF"); // in_progress but deferred into the future
        s.set_field("c1.DEF", "defer_until", Some(future.into()), "x");
        task(&mut s, "c1.ARC"); // in_progress but archived
        s.set_field(
            "c1.ARC",
            "archived",
            Some("2026-06-23T00:00:00Z".into()),
            "x",
        );
        task(&mut s, "c1.DEL"); // in_progress but deleted
        for id in ["c1.BLK", "c1.DEF", "c1.ARC", "c1.DEL"] {
            s.set_field(id, "status", Some("in_progress".into()), "x");
        }
        s.delete_item("c1.DEL", "x");

        assert!(
            finishable_in_progress(s.connection(), NOW)
                .unwrap()
                .is_empty(),
            "only actionable, claimed work is finishable"
        );
    }

    #[test]
    fn finishable_is_deterministic_and_id_sorted() {
        // Like every enumerating derivation: a total, id-sorted order.
        let mut s = Store::open_in_memory(1);
        for id in ["c1.C", "c1.A", "c1.B"] {
            task(&mut s, id);
            s.set_field(id, "status", Some("in_progress".into()), "x");
        }
        let got = finishable_in_progress(s.connection(), NOW).unwrap();
        assert_eq!(
            got,
            vec!["c1.A".to_string(), "c1.B".to_string(), "c1.C".to_string()]
        );
        assert_eq!(
            got,
            finishable_in_progress(s.connection(), NOW).unwrap(),
            "repeated derivation is stable"
        );
    }

    #[test]
    fn promoted_maps_a_ready_child_to_its_in_progress_parent() {
        // Tier 2 (§3.2): the open child of a started epic is promoted into the epic's cluster.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.E");
        s.set_field("c1.E", "status", Some("in_progress".into()), "x");
        task(&mut s, "c1.C");
        s.add_parent("c1.C", "c1.E", "x");
        assert_eq!(
            promoted_children(s.connection(), NOW).unwrap(),
            vec![("c1.C".to_string(), "c1.E".to_string())]
        );
    }

    #[test]
    fn promoted_excludes_a_child_of_an_open_parent() {
        // Only a STARTED (in_progress) parent forms a cluster; an open parent's child stays Tier 3.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.E"); // open, not claimed
        task(&mut s, "c1.C");
        s.add_parent("c1.C", "c1.E", "x");
        assert!(
            promoted_children(s.connection(), NOW).unwrap().is_empty(),
            "an open parent is not a Tier-2 header"
        );
    }

    #[test]
    fn promoted_excludes_a_non_ready_child() {
        // §3.2: only children in the `ready` set are promoted — a child blocked by its own open dep
        // is not actionable, so it is no "do this now" row.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.E");
        s.set_field("c1.E", "status", Some("in_progress".into()), "x");
        task(&mut s, "c1.C");
        s.add_parent("c1.C", "c1.E", "x");
        task(&mut s, "c1.X"); // C's own open blocker
        s.add_edge("c1.C", "c1.X", EdgeKind::Dep, "x");
        assert!(
            promoted_children(s.connection(), NOW).unwrap().is_empty(),
            "a blocked child is not promoted"
        );
    }

    #[test]
    fn promoted_excludes_the_child_of_an_own_blocked_in_progress_parent() {
        // §3.3: an in_progress parent blocked by its OWN open dep is not in the `in_progress` set,
        // so it is not a header — its child falls to Tier 3 rather than into a headerless cluster.
        // (An in_progress parent does not suppress its child, so the child IS still ready.)
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.E");
        s.set_field("c1.E", "status", Some("in_progress".into()), "x");
        task(&mut s, "c1.X"); // E's own open blocker
        s.add_edge("c1.E", "c1.X", EdgeKind::Dep, "x");
        task(&mut s, "c1.C");
        s.add_parent("c1.C", "c1.E", "x");
        assert!(
            ready(s.connection(), NOW)
                .unwrap()
                .contains(&"c1.C".to_string()),
            "precondition: an in_progress parent does not gate, so C is ready"
        );
        assert!(
            !in_progress(s.connection(), NOW)
                .unwrap()
                .contains(&"c1.E".to_string()),
            "precondition: the own-blocked parent is not actionable"
        );
        assert!(
            promoted_children(s.connection(), NOW).unwrap().is_empty(),
            "no actionable in_progress parent → no promotion (no headerless cluster)"
        );
    }

    #[test]
    fn promoted_picks_the_smallest_parent_id_for_a_multi_parent_child() {
        // §3.2: the generic core allows several parents; clustering must be unambiguous and
        // deterministic → the lexicographically smallest in_progress parent wins.
        let mut s = Store::open_in_memory(1);
        for p in ["c1.P2", "c1.P1"] {
            task(&mut s, p);
            s.set_field(p, "status", Some("in_progress".into()), "x");
        }
        task(&mut s, "c1.C");
        s.add_parent("c1.C", "c1.P2", "x");
        s.add_parent("c1.C", "c1.P1", "x");
        assert_eq!(
            promoted_children(s.connection(), NOW).unwrap(),
            vec![("c1.C".to_string(), "c1.P1".to_string())],
            "smallest parent id wins, one row per child"
        );
    }

    #[test]
    fn promoted_is_deterministic_and_sorted_by_child_id() {
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.E");
        s.set_field("c1.E", "status", Some("in_progress".into()), "x");
        for c in ["c1.C3", "c1.C1", "c1.C2"] {
            task(&mut s, c);
            s.add_parent(c, "c1.E", "x");
        }
        let got = promoted_children(s.connection(), NOW).unwrap();
        assert_eq!(
            got.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(),
            vec!["c1.C1", "c1.C2", "c1.C3"],
            "child-id sorted"
        );
        assert_eq!(got, promoted_children(s.connection(), NOW).unwrap());
    }

    #[test]
    fn promoted_ignores_a_contributes_to_edge() {
        // Tiering reads the GATING `parent` edge only — the non-gating `contributes_to` association
        // must never pull an item into someone's cluster (CLAUDE.md's deliberate edge split).
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.E");
        s.set_field("c1.E", "status", Some("in_progress".into()), "x");
        task(&mut s, "c1.C");
        s.add_edge("c1.C", "c1.E", EdgeKind::ContributesTo, "x");
        assert!(
            promoted_children(s.connection(), NOW).unwrap().is_empty(),
            "contributes_to is not a containment edge → no promotion"
        );
    }

    #[test]
    fn headers_are_in_progress_minus_finishable_with_every_cluster_covered() {
        // §3.3: the facade derives the Tier-2 headers as `in_progress \ finishable_in_progress`,
        // and every promoted child's parent must land in that set — otherwise a cluster would be
        // headerless. This is the contract between the two functions.
        let mut s = Store::open_in_memory(1);
        // E1: started epic with an open child → header + promoted child.
        task(&mut s, "c1.E1");
        s.set_field("c1.E1", "status", Some("in_progress".into()), "x");
        task(&mut s, "c1.C1");
        s.add_parent("c1.C1", "c1.E1", "x");
        // E2: started epic, all children closed → finishable, not a header.
        task(&mut s, "c1.E2");
        s.set_field("c1.E2", "status", Some("in_progress".into()), "x");
        task(&mut s, "c1.C2");
        s.add_parent("c1.C2", "c1.E2", "x");
        s.set_field("c1.C2", "status", Some("closed".into()), "x");
        // E3: started epic whose only open child is blocked → LONE header (no promoted child).
        task(&mut s, "c1.E3");
        s.set_field("c1.E3", "status", Some("in_progress".into()), "x");
        task(&mut s, "c1.C3");
        s.add_parent("c1.C3", "c1.E3", "x");
        task(&mut s, "c1.X3");
        s.add_edge("c1.C3", "c1.X3", EdgeKind::Dep, "x");

        let ip = in_progress(s.connection(), NOW).unwrap();
        let fin = finishable_in_progress(s.connection(), NOW).unwrap();
        let headers: Vec<String> = ip.iter().filter(|i| !fin.contains(i)).cloned().collect();
        assert_eq!(fin, vec!["c1.E2".to_string()], "only E2 is closeable now");
        assert_eq!(
            headers,
            vec!["c1.E1".to_string(), "c1.E3".to_string()],
            "E1 (open child) and E3 (lone header) are the started epics"
        );
        for (_, parent) in promoted_children(s.connection(), NOW).unwrap() {
            assert!(
                headers.contains(&parent),
                "every promoted child's parent is a header — no headerless cluster: {parent}"
            );
        }
    }

    #[test]
    fn dependents_count_is_direct_fanout_of_still_open_items() {
        // B blocks A, C, D, E. Closing B would unblock the still-open dependents only:
        // A (open) and C (in_progress) count; D (closed) and E (deleted) do not.
        let mut s = Store::open_in_memory(1);
        for id in ["c1.A", "c1.B", "c1.C", "c1.D", "c1.E"] {
            task(&mut s, id);
        }
        for dep in ["c1.A", "c1.C", "c1.D", "c1.E"] {
            s.add_edge(dep, "c1.B", EdgeKind::Dep, "x");
        }
        s.set_field("c1.C", "status", Some("in_progress".into()), "x");
        s.set_field("c1.D", "status", Some("closed".into()), "x");
        s.delete_item("c1.E", "x");

        assert_eq!(
            dependents_count(s.connection(), "c1.B").unwrap(),
            2,
            "only still-open (open or in_progress) direct dependents count"
        );
        // An item nobody depends on has zero leverage.
        assert_eq!(dependents_count(s.connection(), "c1.A").unwrap(), 0);
    }

    #[test]
    fn cyclic_tasks_are_excluded_from_ready() {
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.A");
        task(&mut s, "c1.B");
        s.add_edge("c1.A", "c1.B", EdgeKind::Dep, "x");
        s.add_edge("c1.B", "c1.A", EdgeKind::Dep, "x"); // cycle
        assert!(ready(s.connection(), NOW).unwrap().is_empty());
        let mut b = blocked(s.connection()).unwrap();
        b.sort();
        assert_eq!(
            b,
            vec!["c1.A".to_string(), "c1.B".to_string()],
            "cyclic nodes are blocked"
        );
    }

    #[test]
    fn archived_item_is_excluded_from_ready_blocked_and_in_progress() {
        // C1 (#916.1): the enumerating derivations exclude archived items by default, exactly as
        // they exclude deleted ones — one item per lane is archived and must drop out.
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.A"); // ready
        task(&mut s, "c1.B"); // blocked by C
        task(&mut s, "c1.C"); // ready (the blocker)
        task(&mut s, "c1.D"); // in_progress
        s.add_edge("c1.B", "c1.C", EdgeKind::Dep, "x");
        s.set_field("c1.D", "status", Some("in_progress".into()), "x");

        // Baseline: A & C ready, B blocked, D in_progress.
        assert_eq!(
            ready(s.connection(), NOW).unwrap(),
            vec!["c1.A".to_string(), "c1.C".to_string()]
        );
        assert_eq!(blocked(s.connection()).unwrap(), vec!["c1.B".to_string()]);
        assert_eq!(
            in_progress(s.connection(), NOW).unwrap(),
            vec!["c1.D".to_string()]
        );

        let ts = "2026-06-23T00:00:00Z";
        s.set_field("c1.A", "archived", Some(ts.into()), "x");
        s.set_field("c1.B", "archived", Some(ts.into()), "x");
        s.set_field("c1.D", "archived", Some(ts.into()), "x");

        assert_eq!(
            ready(s.connection(), NOW).unwrap(),
            vec!["c1.C".to_string()],
            "archived A leaves ready"
        );
        assert!(
            blocked(s.connection()).unwrap().is_empty(),
            "archived B leaves blocked"
        );
        assert!(
            in_progress(s.connection(), NOW).unwrap().is_empty(),
            "archived D leaves in_progress"
        );
    }

    #[test]
    fn ready_parents_do_not_suppress_a_child_regardless_of_count() {
        // 07a.2 residual truth (the narrowed sp6.7 ②c invariant): a child is gated only by a
        // BLOCKED/DEFERRED ancestor. Here every parent is `ready` (open, no deps), so it is
        // non-gating, and a child with 0, 1, or n ready parents is ready on the same terms — parent
        // *count* never gates. The gating case (a blocked/deferred parent suppresses) is pinned by
        // `a_blocked_parent_suppresses_its_children_from_ready` and the cases below.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.P1", "project", "P1", "x");
        s.create_item("c1.P2", "project", "P2", "x");
        s.create_item("c1.O", "task", "orphan", "x"); // 0 parents
        s.create_item("c1.S", "task", "single", "x"); // 1 parent
        s.create_item("c1.M", "task", "multi", "x"); // n parents
        s.add_parent("c1.S", "c1.P1", "x");
        s.add_parent("c1.M", "c1.P1", "x");
        s.add_parent("c1.M", "c1.P2", "x");
        assert_eq!(
            s.parents_of("c1.M").len(),
            2,
            "the multi-parent item really has n parents"
        );

        let r = ready(s.connection(), NOW).unwrap();
        for id in ["c1.O", "c1.S", "c1.M"] {
            assert!(
                r.contains(&id.to_string()),
                "{id} is open + unblocked → ready regardless of parent count"
            );
        }
    }

    #[test]
    fn a_multi_parent_item_blocks_and_defers_on_its_own_state_not_its_parents() {
        // The item's OWN block/defer semantics still aggregate over its own `dep`s and defer date,
        // independent of parent count. Here the two parents are open `project`s with no deps (READY,
        // hence non-gating, post-07a.2), so they never suppress — leaving the item's own state to
        // decide. (The complementary gating case — a blocked/deferred parent DOES suppress — is
        // pinned by `a_blocked_parent_suppresses_its_children_from_ready`.)
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.P1", "project", "P1", "x");
        s.create_item("c1.P2", "project", "P2", "x");
        s.create_item("c1.M", "task", "multi", "x");
        s.create_item("c1.B", "task", "blocker", "x");
        s.add_parent("c1.M", "c1.P1", "x");
        s.add_parent("c1.M", "c1.P2", "x");
        s.add_edge("c1.M", "c1.B", EdgeKind::Dep, "x"); // M depends on the open B

        assert!(
            blocked(s.connection())
                .unwrap()
                .contains(&"c1.M".to_string()),
            "an open dep blocks a multi-parent item"
        );
        assert!(!ready(s.connection(), NOW)
            .unwrap()
            .contains(&"c1.M".to_string()));
        // Close the blocker → ready, still with n parents.
        s.set_field("c1.B", "status", Some("closed".into()), "x");
        assert!(ready(s.connection(), NOW)
            .unwrap()
            .contains(&"c1.M".to_string()));
        // A future defer hides it from ready (deferred lane), parents notwithstanding.
        s.set_field(
            "c1.M",
            "defer_until",
            Some("2026-12-01T00:00:00Z".into()),
            "x",
        );
        assert!(!ready(s.connection(), NOW)
            .unwrap()
            .contains(&"c1.M".to_string()));
        assert!(deferred(s.connection(), NOW)
            .unwrap()
            .contains(&"c1.M".to_string()));
    }

    #[test]
    fn derivation_converges_under_concurrent_multi_parent_edits() {
        // ②c convergence: even concurrent, conflicting parent edits (different parents on each
        // replica) never perturb the derivation, because it is parent-agnostic — both replicas
        // converge on the same parent set AND the same `ready`.
        let mut a = Store::open_in_memory(1);
        a.create_item("c1.P1", "project", "P1", "x");
        a.create_item("c1.P2", "project", "P2", "x");
        a.create_item("c1.T", "task", "T", "x");
        let mut b = Store::open_in_memory(2);
        b.apply(&a.export());

        a.add_parent("c1.T", "c1.P1", "alice"); // concurrent, different parents
        b.add_parent("c1.T", "c1.P2", "bob");
        let ea = a.export();
        let eb = b.export();
        a.apply(&eb);
        b.apply(&ea);

        assert_eq!(
            a.parents_of("c1.T"),
            b.parents_of("c1.T"),
            "the parent OR-set converges"
        );
        assert_eq!(
            ready(a.connection(), NOW).unwrap(),
            ready(b.connection(), NOW).unwrap(),
            "and the derivation converges with it"
        );
        assert!(ready(a.connection(), NOW)
            .unwrap()
            .contains(&"c1.T".to_string()));
    }

    #[test]
    fn deleted_item_is_not_ready_or_in_progress() {
        let mut s = Store::open_in_memory(1);
        task(&mut s, "c1.A");
        assert_eq!(
            ready(s.connection(), NOW).unwrap(),
            vec!["c1.A".to_string()]
        );
        s.set_field("c1.A", "status", Some("in_progress".into()), "x");
        assert_eq!(
            in_progress(s.connection(), NOW).unwrap(),
            vec!["c1.A".to_string()]
        );
        s.delete_item("c1.A", "x");
        assert!(
            ready(s.connection(), NOW).unwrap().is_empty(),
            "deleted item leaves ready"
        );
        assert!(
            in_progress(s.connection(), NOW).unwrap().is_empty(),
            "deleted item leaves in_progress"
        );
    }
}
