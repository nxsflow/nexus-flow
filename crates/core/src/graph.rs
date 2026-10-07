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

    /// Active tickets on a `dep` cycle of active tickets.
    fn cyclic(&self) -> BTreeSet<String> {
        let mut cyclic = BTreeSet::new();
        for start in self.tickets.keys() {
            // Depth-first from `start` over active dep targets; `start` is cyclic iff it comes back.
            let mut seen = BTreeSet::new();
            let mut stack: Vec<&str> = self.active_deps(start).collect();
            while let Some(n) = stack.pop() {
                if n == start {
                    cyclic.insert(start.clone());
                    break;
                }
                if seen.insert(n) {
                    stack.extend(self.active_deps(n));
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
    let mut stmt = conn.prepare(
        "SELECT id, status, defer_until FROM items
          WHERE COALESCE(deleted,'0')<>'1' AND archived IS NULL
            AND status IN ('open','in_progress')",
    )?;
    let tickets = stmt
        .query_map([], |r| {
            Ok(Ticket {
                id: r.get(0)?,
                in_progress: r.get::<_, String>(1)? == "in_progress",
                defer_until: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn
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
