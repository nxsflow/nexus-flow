//! The Rust graph library and the SQL derivation answer alike (6j6v.vvw6 point 4, 6j6v.jr42).
//!
//! Generated boards — open, in-progress, closed, archived and deleted tickets, future and past
//! defer dates, several parents, dependencies, cycles, dangling ends — are derived both ways: by
//! `derive::*` over the whole database, and by `graph::Board` over only the ACTIVE tickets and the
//! edges that touch one, which is all a server's index gives it. Every lane and every tier signal
//! must come out the same.

use nexus_flow_core::derive;
use nexus_flow_core::graph;
use nexus_flow_core::model::EdgeKind;
use nexus_flow_core::store::Store;

const NOW: &str = "2026-10-07T12:00:00Z";

/// A tiny deterministic generator, so a failing board can be rebuilt from its seed.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

fn board(seed: u64) -> Store {
    let mut r = Lcg(seed);
    let mut s = Store::open_in_memory(1);
    let n = 4 + r.next(9) as usize;
    let id = |i: usize| format!("ab12.{i:04}");
    for i in 0..n {
        s.create_item(&id(i), "task", "T", "u");
        match r.next(7) {
            0 | 1 => s.set_field(&id(i), "status", Some("in_progress".into()), "u"),
            2 => s.set_field(&id(i), "status", Some("closed".into()), "u"),
            // An archived ticket, closed or — as only a merge leaves it — still open.
            3 => {
                if r.next(2) == 0 {
                    s.set_field(&id(i), "status", Some("closed".into()), "u");
                }
                s.set_field(&id(i), "archived", Some("2026-10-01T00:00:00Z".into()), "u");
            }
            4 if r.next(3) == 0 => s.delete_item(&id(i), "u"),
            _ => {}
        }
        match r.next(5) {
            0 => s.set_field(
                &id(i),
                "defer_until",
                Some("2027-01-01T00:00:00Z".into()),
                "u",
            ),
            1 => s.set_field(
                &id(i),
                "defer_until",
                Some("2026-01-01T00:00:00Z".into()),
                "u",
            ),
            // The boundary: an instant equal to `now` has passed (`defer_until > now` is strict).
            2 => s.set_field(&id(i), "defer_until", Some(NOW.into()), "u"),
            _ => {}
        }
    }
    let edges = r.next(2 * n as u64) as usize;
    for _ in 0..edges {
        // `n` is a dangling end, at either side: an id no ticket has. `a == b` is a self edge — a
        // dependency on itself is a cycle of one.
        let (a, b) = (r.next(n as u64 + 1) as usize, r.next(n as u64 + 1) as usize);
        let kind = if r.next(2) == 0 {
            EdgeKind::Dep
        } else {
            EdgeKind::Parent
        };
        s.add_edge(&id(a), &id(b), kind, "u");
    }
    s
}

/// Which of the decided cases a board contains — so the generated set can be shown to reach each
/// of them rather than only by chance.
#[derive(Default, Clone, Copy)]
struct Seen {
    archived_open_blocker: bool,
    inactive_only_parent: bool,
    dangling_parent: bool,
    dep_through_closed: bool,
}

fn seen(s: &Store) -> Seen {
    let count = |sql: &str| -> bool {
        s.connection()
            .query_row(sql, [], |r| r.get::<_, i64>(0))
            .unwrap()
            > 0
    };
    Seen {
        archived_open_blocker: count(
            "SELECT COUNT(*) FROM present_edges d JOIN items b ON b.id=d.to_id
             WHERE d.kind='dep' AND b.archived IS NOT NULL AND b.status<>'closed'",
        ),
        inactive_only_parent: count(
            "SELECT COUNT(*) FROM present_edges e JOIN items p ON p.id=e.to_id
             WHERE e.kind='parent' AND COALESCE(p.deleted,'0')='1'",
        ),
        dangling_parent: count(
            "SELECT COUNT(*) FROM present_edges e
             WHERE e.kind='parent' AND e.to_id NOT IN (SELECT id FROM items)",
        ),
        dep_through_closed: count(
            "SELECT COUNT(*) FROM present_edges a JOIN present_edges b
                 ON a.to_id=b.from_id AND b.to_id=a.from_id
             JOIN items c ON c.id=a.to_id
             WHERE a.kind='dep' AND b.kind='dep' AND c.status='closed'",
        ),
    }
}

fn both(seed: u64) -> Seen {
    let s = board(seed);
    let conn = s.connection();
    let lanes = graph::select(conn).unwrap().lanes(NOW);
    let sql_candidates: Vec<graph::Candidate> = derive::next_candidates(conn, NOW)
        .unwrap()
        .into_iter()
        .map(|c| graph::Candidate {
            in_progress: c.status == "in_progress",
            id: c.id,
            finishable: c.finishable,
            promoter: c.promoter,
        })
        .collect();
    let ctx = format!("seed {seed}");
    assert_eq!(
        lanes.ready,
        derive::ready(conn, NOW).unwrap(),
        "ready, {ctx}"
    );
    assert_eq!(
        lanes.in_progress,
        derive::in_progress(conn, NOW).unwrap(),
        "in_progress, {ctx}"
    );
    assert_eq!(
        lanes.deferred,
        derive::deferred(conn, NOW).unwrap(),
        "deferred, {ctx}"
    );
    assert_eq!(
        lanes.blocked,
        derive::blocked(conn).unwrap(),
        "blocked, {ctx}"
    );
    assert_eq!(
        lanes.suppressed_by_blocked,
        derive::suppressed_by_blocked(conn).unwrap(),
        "suppressed_by_blocked, {ctx}"
    );
    assert_eq!(
        lanes.suppressed_by_deferred_only,
        derive::suppressed_by_deferred_only(conn, NOW).unwrap(),
        "suppressed_by_deferred_only, {ctx}"
    );
    assert_eq!(lanes.candidates, sql_candidates, "candidates, {ctx}");
    seen(&s)
}

#[test]
fn the_library_over_the_active_tickets_answers_as_the_sql_over_the_whole_board() {
    let mut reached = [0usize; 4];
    for seed in 0..400 {
        let s = both(seed);
        for (i, hit) in [
            s.archived_open_blocker,
            s.inactive_only_parent,
            s.dangling_parent,
            s.dep_through_closed,
        ]
        .into_iter()
        .enumerate()
        {
            reached[i] += usize::from(hit);
        }
    }
    // Not left to chance: every decided case occurs in the generated set, and more than once.
    assert!(
        reached.iter().all(|&n| n >= 3),
        "cases reached: {reached:?}"
    );
}

/// Every lane, from the library and from the SQL, for one store — what the named cases compare.
fn lanes_both_ways(s: &Store) -> (graph::Lanes, Vec<Vec<String>>) {
    let conn = s.connection();
    let lanes = graph::select(conn).unwrap().lanes(NOW);
    let sql = vec![
        derive::ready(conn, NOW).unwrap(),
        derive::in_progress(conn, NOW).unwrap(),
        derive::deferred(conn, NOW).unwrap(),
        derive::blocked(conn).unwrap(),
        derive::suppressed_by_blocked(conn).unwrap(),
        derive::suppressed_by_deferred_only(conn, NOW).unwrap(),
    ];
    let lib = vec![
        lanes.ready.clone(),
        lanes.in_progress.clone(),
        lanes.deferred.clone(),
        lanes.blocked.clone(),
        lanes.suppressed_by_blocked.clone(),
        lanes.suppressed_by_deferred_only.clone(),
    ];
    assert_eq!(lib, sql, "both paths, every lane");
    (lanes, sql)
}

/// A child that RESTS is in no lane at all — not dropped into blocked, deferred or a suppression.
fn rests(s: &Store, id: &str) {
    let (lanes, _) = lanes_both_ways(s);
    for lane in [
        &lanes.ready,
        &lanes.in_progress,
        &lanes.deferred,
        &lanes.blocked,
        &lanes.suppressed_by_blocked,
        &lanes.suppressed_by_deferred_only,
    ] {
        assert!(!lane.iter().any(|x| x == id), "{id} rests: {lanes:?}");
    }
}

/// The cases the owner's rule of 2026-10-06 decides, each pinned by name rather than left to the
/// generator: in each the old SQL answered differently, and both paths — every lane — now follow
/// the rule that a counterpart outside the active tickets counts as closed.
#[test]
fn an_inactive_counterpart_counts_as_closed_on_both_paths() {
    let lanes = |s: &Store| graph::select(s.connection()).unwrap().lanes(NOW);

    // 1. An archived ticket that is open again (a merge) no longer blocks.
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "task", "Work", "u");
    s.create_item("ab12.0002", "task", "Archived blocker", "u");
    s.set_field(
        "ab12.0002",
        "archived",
        Some("2026-10-01T00:00:00Z".into()),
        "u",
    );
    s.add_edge("ab12.0001", "ab12.0002", EdgeKind::Dep, "u");
    assert_eq!(
        derive::ready(s.connection(), NOW).unwrap(),
        vec!["ab12.0001"]
    );
    assert_eq!(lanes(&s).ready, vec!["ab12.0001"]);

    // 2. A child whose only parent is deleted rests, as under a closed parent.
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "epic", "Gone", "u");
    s.create_item("ab12.0002", "task", "Child", "u");
    s.add_parent("ab12.0002", "ab12.0001", "u");
    s.delete_item("ab12.0001", "u");
    rests(&s, "ab12.0002");
    // …and the same under an archived parent that is open again.
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "epic", "Archived", "u");
    s.create_item("ab12.0002", "task", "Child", "u");
    s.add_parent("ab12.0002", "ab12.0001", "u");
    s.set_field(
        "ab12.0001",
        "archived",
        Some("2026-10-01T00:00:00Z".into()),
        "u",
    );
    rests(&s, "ab12.0002");

    // 3. A dependency cycle that runs through a closed ticket is no cycle.
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0001", "task", "A", "u");
    s.create_item("ab12.0002", "task", "B", "u");
    s.set_field("ab12.0002", "status", Some("closed".into()), "u");
    s.add_edge("ab12.0001", "ab12.0002", EdgeKind::Dep, "u");
    s.add_edge("ab12.0002", "ab12.0001", EdgeKind::Dep, "u");
    assert!(derive::blocked(s.connection()).unwrap().is_empty());
    assert_eq!(
        derive::ready(s.connection(), NOW).unwrap(),
        vec!["ab12.0001"]
    );
    assert_eq!(lanes(&s).ready, vec!["ab12.0001"]);
    lanes_both_ways(&s);

    // 4. A parent id no ticket has — an edge that arrived before its parent, or a parent never
    //    delivered — counts as closed too: the child rests. A server's index cannot tell a ticket
    //    it never saw from one that is not active, so the rule cannot either (decided on the review
    //    of PR #23 by the rule of 2026-10-06).
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0002", "task", "Child", "u");
    s.add_parent("ab12.0002", "ab12.9999", "u");
    rests(&s, "ab12.0002");
}
