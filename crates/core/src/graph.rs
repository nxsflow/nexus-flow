//! The board's graph logic as plain Rust (6j6v.vvw6 point 4, 6j6v.jr42): which tickets are ready,
//! deferred, blocked or suppressed, and the finish-first tier signals `next` orders by — computed
//! over simple structures, so the SAME code runs where the tickets come from SQL (this repository's
//! [`select`]) and where they come from DynamoDB (a server's query of its "active" index).
//!
//! # What the library is given
//!
//! Only the ACTIVE tickets — live, not archived, `open` or `in_progress` — and the present edges
//! at least one of whose ends is active. **A counterpart that is not among the active tickets
//! counts as closed** (owner decision 2026-10-06): a dependency on it is satisfied, a parent that
//! is not active lets its child rest (the closed-mask), a child that is not active does not hold
//! its parent back, and a cycle needs active tickets all the way round. That is what lets a server
//! answer from a sparse index of active tickets instead of the whole board.
//!
//! The SQL path in [`crate::derive`] answers by the same rule, and a differential test holds the two
//! to the same output on generated boards.

use crate::model::EdgeKind;
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};

/// An active ticket: what the derivation reads of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub id: String,
    /// `open` or `in_progress` — an active ticket has no other status.
    pub in_progress: bool,
    /// The defer instant (ISO-8601), compared as text against `now` as the SQL path does.
    pub defer_until: Option<String>,
}

/// A present edge, `from` → `to` (a `dep` points at its blocker, a `parent` edge at the parent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

/// One `next` candidate with its tier signals (see [`crate::derive::NextCandidate`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub in_progress: bool,
    /// Tier 1: claimed work with no active child — closeable now.
    pub finishable: bool,
    /// Tier 2: the smallest actionable in-progress parent of this open child.
    pub promoter: Option<String>,
}

/// Everything the lanes need, derived once. Every list is id-sorted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lanes {
    /// Open, actionable.
    pub ready: Vec<String>,
    /// In progress, actionable.
    pub in_progress: Vec<String>,
    /// Open, unblocked, acyclic, deferred into the future — its OWN lane, parents aside.
    pub deferred: Vec<String>,
    /// Open, and in a cycle or held by an active dependency — its OWN lane, parents aside.
    pub blocked: Vec<String>,
    /// Active tickets suppressed by a blocked ancestor.
    pub suppressed_by_blocked: Vec<String>,
    /// Active tickets suppressed only by a deferred ancestor.
    pub suppressed_by_deferred_only: Vec<String>,
    /// `ready ∪ in_progress` with the tier signals.
    pub candidates: Vec<Candidate>,
}

/// The active tickets and their edges.
#[derive(Debug, Clone, Default)]
pub struct Board {
    tickets: BTreeMap<String, Ticket>,
    /// `dep` targets per ticket.
    deps: BTreeMap<String, BTreeSet<String>>,
    /// `parent` targets per child, and children per parent.
    parents: BTreeMap<String, BTreeSet<String>>,
    children: BTreeMap<String, BTreeSet<String>>,
}

impl Board {
    /// A board from active tickets and edges. Edges of other kinds are ignored; an edge neither of
    /// whose ends is active cannot change anything and is ignored too.
    pub fn new(
        tickets: impl IntoIterator<Item = Ticket>,
        edges: impl IntoIterator<Item = Edge>,
    ) -> Board {
        let mut board = Board {
            tickets: tickets.into_iter().map(|t| (t.id.clone(), t)).collect(),
            ..Board::default()
        };
        for e in edges {
            match e.kind {
                EdgeKind::Dep => {
                    board.deps.entry(e.from).or_default().insert(e.to);
                }
                EdgeKind::Parent => {
                    board
                        .children
                        .entry(e.to.clone())
                        .or_default()
                        .insert(e.from.clone());
                    board.parents.entry(e.from).or_default().insert(e.to);
                }
                EdgeKind::ContributesTo | EdgeKind::Mentions => {}
            }
        }
        board
    }

    fn active(&self, id: &str) -> bool {
        self.tickets.contains_key(id)
    }

    /// The active tickets, id-sorted — the ids whose records a reader of the lanes needs.
    pub fn tickets(&self) -> impl Iterator<Item = &Ticket> + '_ {
        self.tickets.values()
    }

    /// The blockers of `id`: the active tickets it depends on, id-sorted (6j6v.t1ym). A dependency
    /// on a ticket that is not active is satisfied and not listed — the rule [`blocked`] decides by,
    /// so a ticket in the blocked lane lists exactly what holds it (a member of a cycle lists its
    /// successors on the cycle, a ticket depending on itself lists itself).
    ///
    /// [`blocked`]: Self::blocked
    pub fn blockers(&self, id: &str) -> Vec<&Ticket> {
        self.active_deps(id).map(|t| &self.tickets[t]).collect()
    }

    /// Active tickets on a `dep` cycle of active tickets: the members of every strongly connected
    /// component of more than one ticket, and every ticket that depends on itself.
    ///
    /// Tarjan's algorithm, iterative so a long dependency chain cannot overflow the stack, and
    /// linear in tickets plus edges — a server derives the lanes on every read (review of PR #23).
    fn cyclic(&self) -> BTreeSet<String> {
        let ids: Vec<&str> = self.tickets.keys().map(String::as_str).collect();
        let at: BTreeMap<&str, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        let succ: Vec<Vec<usize>> = ids
            .iter()
            .map(|id| self.active_deps(id).map(|t| at[t]).collect())
            .collect();
        let n = ids.len();
        let (mut index, mut low) = (vec![usize::MAX; n], vec![0; n]);
        let mut on_stack = vec![false; n];
        let (mut stack, mut next, mut cyclic) = (Vec::new(), 0, BTreeSet::new());
        for root in 0..n {
            if index[root] != usize::MAX {
                continue;
            }
            // Each frame is a ticket and how many of its successors it has walked.
            let mut frames = vec![(root, 0)];
            index[root] = next;
            low[root] = next;
            next += 1;
            stack.push(root);
            on_stack[root] = true;
            while let Some(&(v, walked)) = frames.last() {
                if let Some(&w) = succ[v].get(walked) {
                    frames.last_mut().expect("a frame is open").1 += 1;
                    if index[w] == usize::MAX {
                        index[w] = next;
                        low[w] = next;
                        next += 1;
                        stack.push(w);
                        on_stack[w] = true;
                        frames.push((w, 0));
                    } else if on_stack[w] {
                        low[v] = low[v].min(index[w]);
                    }
                    continue;
                }
                frames.pop();
                if let Some(&(parent, _)) = frames.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut component = Vec::new();
                    loop {
                        let w = stack.pop().expect("v is on the stack");
                        on_stack[w] = false;
                        component.push(w);
                        if w == v {
                            break;
                        }
                    }
                    if component.len() > 1 || succ[v].contains(&v) {
                        cyclic.extend(component.into_iter().map(|w| ids[w].to_string()));
                    }
                }
            }
        }
        cyclic
    }

    fn active_deps<'a>(&'a self, id: &str) -> impl Iterator<Item = &'a str> + 'a {
        self.deps
            .get(id)
            .into_iter()
            .flatten()
            .filter(|t| self.active(t))
            .map(String::as_str)
    }

    /// Every active descendant (down the parent edges, through active tickets) of `roots`.
    fn descendants(&self, roots: &BTreeSet<String>) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut stack: Vec<&str> = roots.iter().map(String::as_str).collect();
        while let Some(p) = stack.pop() {
            for c in self.children.get(p).into_iter().flatten() {
                if self.active(c) && out.insert(c.clone()) {
                    stack.push(c);
                }
            }
        }
        out
    }

    /// The blocked lane alone: open tickets in an active cycle or held by an active dependency. It
    /// depends on no instant, which is why it takes none; [`lanes`](Self::lanes) yields the same.
    pub fn blocked(&self) -> Vec<String> {
        let cyclic = self.cyclic();
        self.tickets
            .values()
            .filter(|t| !t.in_progress)
            .filter(|t| cyclic.contains(&t.id) || self.active_deps(&t.id).next().is_some())
            .map(|t| t.id.clone())
            .collect()
    }

    /// Derive every lane at `now`.
    pub fn lanes(&self, now: &str) -> Lanes {
        let cyclic = self.cyclic();
        let held = |id: &str| self.active_deps(id).next().is_some();
        let deferred_at = |t: &Ticket| t.defer_until.as_deref().is_some_and(|d| d > now);
        let open = |t: &&Ticket| !t.in_progress;

        let blocked: BTreeSet<String> = self
            .tickets
            .values()
            .filter(open)
            .filter(|t| cyclic.contains(&t.id) || held(&t.id))
            .map(|t| t.id.clone())
            .collect();
        let deferred: BTreeSet<String> = self
            .tickets
            .values()
            .filter(open)
            .filter(|t| !blocked.contains(&t.id) && deferred_at(t))
            .map(|t| t.id.clone())
            .collect();
        let under_blocked = self.descendants(&blocked);
        let under_deferred = self.descendants(&deferred);

        let actionable: BTreeMap<&str, &Ticket> = self
            .tickets
            .values()
            .filter(|t| {
                !cyclic.contains(&t.id)
                    && !held(&t.id)
                    && !deferred_at(t)
                    && !under_blocked.contains(&t.id)
                    && !under_deferred.contains(&t.id)
                    // closed-mask: an OPEN ticket all of whose parents are not active rests.
                    && !(open(t)
                        && self.parents.get(&t.id).is_some_and(|ps| {
                            !ps.is_empty() && ps.iter().all(|p| !self.active(p))
                        }))
            })
            .map(|t| (t.id.as_str(), t))
            .collect();

        let candidates = actionable
            .values()
            .map(|t| Candidate {
                id: t.id.clone(),
                in_progress: t.in_progress,
                finishable: !self
                    .children
                    .get(&t.id)
                    .into_iter()
                    .flatten()
                    .any(|c| self.active(c)),
                promoter: if t.in_progress {
                    None
                } else {
                    self.parents
                        .get(&t.id)
                        .into_iter()
                        .flatten()
                        .find(|p| actionable.get(p.as_str()).is_some_and(|pt| pt.in_progress))
                        .cloned()
                },
            })
            .collect::<Vec<_>>();

        Lanes {
            ready: candidates
                .iter()
                .filter(|c| !c.in_progress)
                .map(|c| c.id.clone())
                .collect(),
            in_progress: candidates
                .iter()
                .filter(|c| c.in_progress)
                .map(|c| c.id.clone())
                .collect(),
            deferred: deferred.into_iter().collect(),
            blocked: blocked.into_iter().collect(),
            suppressed_by_blocked: under_blocked.iter().cloned().collect(),
            suppressed_by_deferred_only: under_deferred
                .difference(&under_blocked)
                .cloned()
                .collect(),
            candidates,
        }
    }
}

/// The selection for a local board (6j6v.jr42): the active tickets and the present edges that
/// touch one, read with SQL. A server makes the same selection from its "active" index.
pub fn select(conn: &Connection) -> rusqlite::Result<Board> {
    // One read transaction for both statements, so a commit in between cannot hand the library an
    // edge whose ticket the first read did not see (review of PR #23, Integrity #4). Dropped, not
    // committed: it wrote nothing.
    let tx = conn.unchecked_transaction()?;
    let mut stmt = tx.prepare(&format!(
        "SELECT id, status, defer_until FROM items WHERE {}",
        crate::model::ACTIVE_SQL
    ))?;
    let tickets = stmt
        .query_map([], |r| {
            Ok(Ticket {
                id: r.get(0)?,
                in_progress: r.get::<_, String>(1)? == "in_progress",
                defer_until: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = tx
        .prepare("SELECT from_id, to_id, kind FROM present_edges WHERE kind IN ('dep','parent')")?;
    let edges = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let active: BTreeSet<&str> = tickets.iter().map(|t| t.id.as_str()).collect();
    let edges: Vec<Edge> = edges
        .into_iter()
        .filter(|(f, t, _)| active.contains(f.as_str()) || active.contains(t.as_str()))
        .filter_map(|(from, to, kind)| EdgeKind::parse(&kind).map(|kind| Edge { from, to, kind }))
        .collect();
    Ok(Board::new(tickets, edges))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-10-07T12:00:00Z";

    fn t(id: &str) -> Ticket {
        Ticket {
            id: id.into(),
            in_progress: false,
            defer_until: None,
        }
    }

    fn started(id: &str) -> Ticket {
        Ticket {
            in_progress: true,
            ..t(id)
        }
    }

    fn deferred(id: &str) -> Ticket {
        Ticket {
            defer_until: Some("2027-01-01T00:00:00Z".into()),
            ..t(id)
        }
    }

    fn e(from: &str, to: &str, kind: EdgeKind) -> Edge {
        Edge {
            from: from.into(),
            to: to.into(),
            kind,
        }
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_promoter_is_the_smallest_actionable_started_parent_and_finishable_ignores_inactive_children(
    ) {
        // c has two started parents (p1 < p2) and one parent outside the active set (gone): it is
        // promoted by p1. p2 has c as an active child, so it is not finishable; p3's only child is
        // not active (closed or archived — the server never sends it), so p3 is.
        let board = Board::new(
            [started("p1"), started("p2"), started("p3"), t("c")],
            [
                e("c", "p2", EdgeKind::Parent),
                e("c", "p1", EdgeKind::Parent),
                e("c", "gone", EdgeKind::Parent),
                e("done-child", "p3", EdgeKind::Parent),
            ],
        );
        let lanes = board.lanes(NOW);
        let c = lanes.candidates.iter().find(|c| c.id == "c").unwrap();
        assert_eq!(c.promoter.as_deref(), Some("p1"));
        let finishable: Vec<&str> = lanes
            .candidates
            .iter()
            .filter(|c| c.finishable && c.in_progress)
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(finishable, vec!["p3"]);
    }

    #[test]
    fn a_child_with_one_active_parent_among_inactive_ones_does_not_rest() {
        let board = Board::new(
            [t("p"), t("c")],
            [
                e("c", "p", EdgeKind::Parent),
                e("c", "closed", EdgeKind::Parent),
            ],
        );
        assert_eq!(board.lanes(NOW).ready, ids(&["c", "p"]));
        // …and with none active it rests: in no lane at all, not dropped into blocked.
        let board = Board::new([t("c")], [e("c", "closed", EdgeKind::Parent)]);
        let lanes = board.lanes(NOW);
        assert!(lanes.ready.is_empty() && lanes.blocked.is_empty() && lanes.deferred.is_empty());
    }

    #[test]
    fn suppressed_by_deferred_only_leaves_out_what_a_blocked_ancestor_already_suppresses() {
        // b is blocked (held by x), d deferred; c hangs under both, e under d alone.
        let board = Board::new(
            [t("b"), t("x"), deferred("d"), t("c"), t("e")],
            [
                e("b", "x", EdgeKind::Dep),
                e("c", "b", EdgeKind::Parent),
                e("c", "d", EdgeKind::Parent),
                e("e", "d", EdgeKind::Parent),
            ],
        );
        let lanes = board.lanes(NOW);
        assert_eq!(lanes.blocked, ids(&["b"]));
        assert_eq!(lanes.deferred, ids(&["d"]));
        assert_eq!(lanes.suppressed_by_blocked, ids(&["c"]));
        assert_eq!(lanes.suppressed_by_deferred_only, ids(&["e"]));
        assert_eq!(lanes.ready, ids(&["x"]));
    }

    #[test]
    fn blockers_are_the_active_dependencies_in_id_order() {
        // b depends on z, a and on a ticket that is not active (gone): it lists a and z, in id
        // order. s depends on itself and lists itself; free depends on nothing.
        let board = Board::new(
            [t("b"), t("z"), started("a"), t("s"), t("free")],
            [
                e("b", "z", EdgeKind::Dep),
                e("b", "gone", EdgeKind::Dep),
                e("b", "a", EdgeKind::Dep),
                e("b", "free", EdgeKind::Parent),
                e("s", "s", EdgeKind::Dep),
            ],
        );
        let ids = |v: Vec<&Ticket>| v.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(board.blockers("b")), ["a", "z"]);
        assert!(board.blockers("b")[0].in_progress);
        assert_eq!(ids(board.blockers("s")), ["s"]);
        assert!(board.blockers("free").is_empty());
        assert!(board.blockers("not-a-ticket").is_empty());
        assert_eq!(
            board.tickets().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["a", "b", "free", "s", "z"]
        );
    }

    #[test]
    fn edges_of_other_kinds_change_nothing() {
        let board = Board::new(
            [t("a"), t("b")],
            [
                e("a", "b", EdgeKind::ContributesTo),
                e("a", "b", EdgeKind::Mentions),
            ],
        );
        assert_eq!(board.lanes(NOW).ready, ids(&["a", "b"]));
    }

    #[test]
    fn blocked_depends_on_no_instant() {
        let board = Board::new(
            [t("a"), deferred("b"), t("c"), t("d")],
            [
                e("a", "c", EdgeKind::Dep),
                e("b", "c", EdgeKind::Dep),
                e("c", "d", EdgeKind::Dep),
                e("d", "c", EdgeKind::Dep),
            ],
        );
        let expected = ids(&["a", "b", "c", "d"]);
        assert_eq!(board.blocked(), expected);
        for now in ["", NOW, "9999-12-31T23:59:59Z"] {
            assert_eq!(board.lanes(now).blocked, expected, "now = {now:?}");
        }
    }

    #[test]
    fn a_cycle_holds_its_members_and_not_what_merely_leads_into_it() {
        // c → d → e → c is a cycle; t leads into it and s depends on itself; u hangs off s. Only the
        // members and the self edge are cyclic — `t` and `u` are blocked because they are HELD.
        let board = Board::new(
            [t("c"), t("d"), t("e"), t("t"), t("s"), t("u"), t("free")],
            [
                e("c", "d", EdgeKind::Dep),
                e("d", "e", EdgeKind::Dep),
                e("e", "c", EdgeKind::Dep),
                e("t", "c", EdgeKind::Dep),
                e("s", "s", EdgeKind::Dep),
                e("u", "s", EdgeKind::Dep),
            ],
        );
        assert_eq!(
            board.cyclic(),
            ids(&["c", "d", "e", "s"]).into_iter().collect()
        );
        assert_eq!(board.blocked(), ids(&["c", "d", "e", "s", "t", "u"]));
        assert_eq!(board.lanes(NOW).ready, ids(&["free"]));
    }

    #[test]
    fn a_long_dependency_chain_is_walked_without_recursion() {
        // 100 000 tickets in one chain, closed into a ring by the last edge: a recursive walk would
        // overflow the test thread's stack long before the end.
        let n = 100_000;
        let id = |i: usize| format!("t{i:06}");
        let board = Board::new(
            (0..n).map(|i| t(&id(i))),
            (0..n).map(|i| e(&id(i), &id((i + 1) % n), EdgeKind::Dep)),
        );
        assert_eq!(board.cyclic().len(), n);
    }

    #[test]
    fn a_defer_date_equal_to_now_has_passed() {
        // The SQL compares `defer_until > now`; an instant equal to now is not in the future.
        let board = Board::new(
            [Ticket {
                defer_until: Some(NOW.into()),
                ..t("a")
            }],
            [],
        );
        let lanes = board.lanes(NOW);
        assert_eq!(lanes.ready, ids(&["a"]));
        assert!(lanes.deferred.is_empty());
    }
}
