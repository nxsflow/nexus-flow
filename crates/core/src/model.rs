//! flow's task vocabulary. Enums round-trip to the strings stored in SQLite. The substrate
//! [`Op`](nxs_foundation::model::Op) is re-exported here so existing `nexus_flow_core::model::Op`
//! paths keep resolving — the op shape is domain-agnostic and owned by the foundation.

pub use nxs_foundation::model::Op;

// The item `type` is an opaque string on `items`, NOT a fixed core enum (sp6.2, Option B of
// #sp6.1). The core is type-agnostic: it folds and stores whatever type string an op carries,
// and the *valid set* is declared by the active plugin and enforced at the facade write seam —
// never here. (Until sp6.2 this was a hardcoded `enum ItemType { Project, Task }`, which baked a
// two-type hierarchy into the meaning-free core; dissolving it is the foundation for the
// plugin-declared type system.)

/// Canonical core lifecycle. Plugins map display labels, never the lifecycle itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Open,
    InProgress,
    Closed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::InProgress => "in_progress",
            Status::Closed => "closed",
        }
    }
    pub fn parse(s: &str) -> Option<Status> {
        match s {
            "open" => Some(Status::Open),
            "in_progress" => Some(Status::InProgress),
            "closed" => Some(Status::Closed),
            _ => None,
        }
    }
}

/// Typed n:m edges. `Dep` works on both levels (task<->task and project<->project).
/// `Mentions` is a pure **reference** edge — it records that an item's free text cites
/// another item's short-id, and carries NO blocking semantics (derivation ignores it,
/// see `derive`/`invariant`, which walk `dep` alone). It exists so a cited short-id stays
/// resolvable after a prefix remap (spec §8): the remap pass rewrites the edge's endpoints
/// structurally even when the free text cannot be (e.g. an immutable note).
///
/// `Parent` is hierarchy parenthood, `from = child → to = parent` (sp6.3, Option B of #sp6.1):
/// the OR-set replacement for the former single-parent `belongs_to` LWW register. The substrate is
/// n:m-capable like every other edge; single-parent is enforced at the write seam during ②a and
/// relaxed in ②c. The single current parent is projected by the `present_parent` view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Dep,
    ContributesTo,
    Mentions,
    Parent,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Dep => "dep",
            EdgeKind::ContributesTo => "contributes_to",
            EdgeKind::Mentions => "mentions",
            EdgeKind::Parent => "parent",
        }
    }
    pub fn parse(s: &str) -> Option<EdgeKind> {
        match s {
            "dep" => Some(EdgeKind::Dep),
            "contributes_to" => Some(EdgeKind::ContributesTo),
            "mentions" => Some(EdgeKind::Mentions),
            "parent" => Some(EdgeKind::Parent),
            _ => None,
        }
    }
}

/// What a chat thread's link to a board item consists IN (nxf 6j6v.8dbe) — the *kind* half of a
/// thread link, independent of its [`LinkWeight`].
///
/// The distinction settles a case that is otherwise a judgement call with no right answer: *"let's
/// solve xyz the way we solved abc"* produces a [`LinkRelation::WorkedOn`] link to xyz and a
/// [`LinkRelation::Cited`] link to abc. Both are wanted, and they mean different things — one thread
/// is doing work on xyz; it is merely reasoning from abc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkRelation {
    /// This thread is working on this item — the deterministic case, derivable from a run's
    /// commissioned ticket set with no model call (see nxf 6j6v.jd37).
    WorkedOn,
    /// This thread refers to this item without working on it.
    Cited,
}

impl LinkRelation {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkRelation::WorkedOn => "worked_on",
            LinkRelation::Cited => "cited",
        }
    }
    pub fn parse(s: &str) -> Option<LinkRelation> {
        match s {
            "worked_on" => Some(LinkRelation::WorkedOn),
            "cited" => Some(LinkRelation::Cited),
            _ => None,
        }
    }
}

/// How much a thread is ABOUT the item it links to (nxf 6j6v.8dbe) — the *weight* half, fully
/// independent of [`LinkRelation`]: all four combinations are meaningful. A thread can
/// load-bearingly CITE an item (a direction debate working through an earlier decision), or
/// in-passing WORK ON one (an aside that fixes a small thing along the way).
///
/// Without it a long, wandering conversation accumulates links to everything it touched, and each
/// of those items would read "there are conversations about this" — noise in exactly the place a
/// coding agent looks. So the read paths gate on it: [`LinkWeight::Bearing`] links surface in
/// `next`, [`LinkWeight::Passing`] ones only on the item itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkWeight {
    /// The item is what the thread is about.
    Bearing,
    /// It came up. No more than that.
    Passing,
}

impl LinkWeight {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkWeight::Bearing => "bearing",
            LinkWeight::Passing => "passing",
        }
    }
    pub fn parse(s: &str) -> Option<LinkWeight> {
        match s {
            "bearing" => Some(LinkWeight::Bearing),
            "passing" => Some(LinkWeight::Passing),
            _ => None,
        }
    }
}

/// One present thread↔item link, as the read paths report it (nxf 6j6v.8dbe).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadLink {
    /// The chat thread's own id, in the CHAT id space — opaque to flow, which never resolves it.
    pub thread_id: String,
    pub item_id: String,
    pub relation: LinkRelation,
    pub weight: LinkWeight,
}

/// Per-field merge strategy the item fold dispatches on (w213 — the seam of the note-body
/// decision, 13uo). *Selecting* the strategy is plugin vocabulary (the facade `[merge]` table);
/// the *behaviour* lives here in the core reducer. The strategy rides the op's `op_type`, so the
/// fold is a pure function of the op — a replica folding a synced op needs no sender plugin config
/// (convergence).
///
/// `Lww` (the default) keeps the bare `"set"` op_type, so every historical item op is unchanged.
/// `CrdtText` is reserved for the collaborative note body: it folds LWW until the text-CRDT reducer
/// lands (4b39), so declaring it today is safe and lossless. `or-set` is a plugin-declarable
/// strategy but NOT an item-field fold strategy — it is realized by its own observed-remove view
/// (labels/h89s), so it is deliberately absent from this item-fold enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    Lww,
    CrdtText,
}

impl MergeStrategy {
    /// The strategy's canonical name (matches the facade `[merge]` vocabulary).
    pub fn as_str(self) -> &'static str {
        match self {
            MergeStrategy::Lww => "lww",
            MergeStrategy::CrdtText => "crdt-text",
        }
    }

    /// Parse a strategy name; `None` for anything this item fold does not realize (e.g. `or-set`).
    pub fn parse(s: &str) -> Option<MergeStrategy> {
        match s {
            "lww" => Some(MergeStrategy::Lww),
            "crdt-text" => Some(MergeStrategy::CrdtText),
            _ => None,
        }
    }

    /// The `op_type` an item field `set` carries for this strategy. `Lww` is the bare `"set"`
    /// (backward compatible — every historical item op stays foldable); a non-default strategy is
    /// `"set:<name>"`, self-describing so the reducer dispatches on the op alone.
    pub fn item_set_op_type(self) -> &'static str {
        match self {
            MergeStrategy::Lww => "set",
            MergeStrategy::CrdtText => "set:crdt-text",
        }
    }

    /// The strategy an item `set` op_type encodes, or `None` when `op_type` is not an item field
    /// set at all (an edge/note op, or a shape this version does not understand).
    pub fn from_item_set_op_type(op_type: &str) -> Option<MergeStrategy> {
        match op_type {
            "set" => Some(MergeStrategy::Lww),
            other => other.strip_prefix("set:").and_then(MergeStrategy::parse),
        }
    }
}

/// flow's product/reducer domain (the spec's "kind" axis, §4.1) — the domain every op flow
/// originates rides, and the key its task reducer is registered under. `task` is flow's name; the
/// substrate is domain-agnostic, so the constant lives here, in the product crate.
pub const DOMAIN_TASK: &str = "task";

/// A materialized item row, as read back from the `items` view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    pub id: String,
    pub item_type: Option<String>,
    pub title: Option<String>,
    pub completion_criterion: Option<String>,
    pub description: Option<String>,
    pub design: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    pub due: Option<String>,
    pub defer_until: Option<String>,
    pub assignee: Option<String>,
    pub belongs_to: Option<String>,
    pub closing_comment: Option<String>,
    pub deleted: Option<String>,
    /// The archive tombstone (C1, #916.1): `Some(<ISO-8601 instant>)` once archived, `None`
    /// otherwise. A timestamp, not a bool, so the `archived` lane can sort by archive-date.
    pub archived: Option<String>,
    /// The close instant (C3, #916.4): `Some(<ISO-8601 instant>)` once closed, `None` otherwise.
    /// Stamped by `close` from the injected `now`, so the `closed` lane can sort by close-date
    /// descending (recency). A pure timestamp — `status='closed'` remains the lifecycle truth;
    /// this is the recency key, modelled exactly like `archived`.
    pub closed_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_round_trip_through_strings() {
        // `type` is no longer an enum (sp6.2) — only the lifecycle (`Status`) and edge kinds round
        // trip through fixed strings; the item type is an opaque string the core never parses.
        for s in [Status::Open, Status::InProgress, Status::Closed] {
            assert_eq!(Status::parse(s.as_str()), Some(s));
        }
        for k in [
            EdgeKind::Dep,
            EdgeKind::ContributesTo,
            EdgeKind::Mentions,
            EdgeKind::Parent,
        ] {
            assert_eq!(EdgeKind::parse(k.as_str()), Some(k));
        }
        // sp6.3: the parent edge's wire string is the stable `"parent"`.
        assert_eq!(EdgeKind::Parent.as_str(), "parent");
        assert_eq!(EdgeKind::parse("parent"), Some(EdgeKind::Parent));
    }

    #[test]
    fn unknown_strings_do_not_parse() {
        assert_eq!(Status::parse("frozen"), None);
        assert_eq!(EdgeKind::parse("supersedes"), None);
    }

    #[test]
    fn merge_strategy_encodes_and_parses_the_item_set_op_type() {
        // w213: Lww keeps the bare "set" op_type — backward compatible, every historical item op
        // stays foldable. crdt-text is a distinct, self-describing op_type so the fold dispatches on
        // the op ALONE (a replica folding a synced op needs no sender plugin config → convergence).
        assert_eq!(MergeStrategy::Lww.item_set_op_type(), "set");
        assert_eq!(MergeStrategy::CrdtText.item_set_op_type(), "set:crdt-text");
        assert_eq!(
            MergeStrategy::from_item_set_op_type("set"),
            Some(MergeStrategy::Lww)
        );
        assert_eq!(
            MergeStrategy::from_item_set_op_type("set:crdt-text"),
            Some(MergeStrategy::CrdtText)
        );
        assert_eq!(MergeStrategy::from_item_set_op_type("add"), None);
        assert_eq!(MergeStrategy::from_item_set_op_type("note_add"), None);
        // The strategy name round-trips (used by the op-type encoding + diagnostics). `or-set` is a
        // plugin-declarable strategy but NOT an item-field fold strategy here (its own view, h89s).
        assert_eq!(MergeStrategy::Lww.as_str(), "lww");
        assert_eq!(MergeStrategy::CrdtText.as_str(), "crdt-text");
        assert_eq!(
            MergeStrategy::parse("crdt-text"),
            Some(MergeStrategy::CrdtText)
        );
        assert_eq!(MergeStrategy::parse("or-set"), None);
    }
}
