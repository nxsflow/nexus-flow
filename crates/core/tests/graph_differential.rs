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
            _ => {}
        }
    }
    let edges = r.next(2 * n as u64) as usize;
    for _ in 0..edges {
        let (a, b) = (r.next(n as u64) as usize, r.next(n as u64 + 1) as usize);
        // `b == n` is a dangling end: an id no ticket has.
        let kind = if r.next(2) == 0 {
            EdgeKind::Dep
        } else {
            EdgeKind::Parent
        };
        if a != b {
            s.add_edge(&id(a), &id(b), kind, "u");
        }
    }
    s
}

fn both(seed: u64) {
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
}

#[test]
fn the_library_over_the_active_tickets_answers_as_the_sql_over_the_whole_board() {
    for seed in 0..400 {
        both(seed);
    }
}

/// The three cases the owner decided on 2026-10-06, each pinned by name rather than left to the
/// generator: in each the old SQL answered differently, and both paths now follow the rule that a
/// counterpart outside the active tickets counts as closed.
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
    assert!(derive::ready(s.connection(), NOW).unwrap().is_empty());
    assert!(lanes(&s).ready.is_empty());

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
}
