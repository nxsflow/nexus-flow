//! What a decision is written from and read as (RFC 0003, "Reading decisions", "The library
//! seam"): `NewDecision` holds what the writer gives, `Decision` a decision in full, its fields the
//! JSON fields `nxs decision show` prints.

/// A decision to write, as the writer gives it (RFC 0003, "Writing a decision"). `author` is not
/// here: it is always the writing member's id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewDecision {
    /// The decision, stated in one line of at most 200 characters.
    pub title: String,
    pub why: String,
    /// The options not taken.
    pub instead_of: String,
    /// At least one lowercase tag.
    pub topics: Vec<String>,
    /// Ticket labels, each a ticket id or a range of them; none for `no ticket`. The first names
    /// the triggering ticket.
    pub tickets: Vec<String>,
    /// The decisions this one supersedes, each by handle or object id.
    pub supersedes: Vec<String>,
    /// The `https` URL of the pull request whose code the decision describes.
    pub pr: Option<String>,
    /// Who decided; the writer's role from its role record when `None`.
    pub decided_by: Option<String>,
    /// The day the decision was made, `YYYY-MM-DD`; today's in UTC when `None`.
    pub date: Option<String>,
    /// The handle's slug; the title's when `None`.
    pub slug: Option<String>,
    /// Everything below the fields, as Markdown.
    pub body: Option<String>,
}

/// A decision in full (RFC 0003, "Reading decisions").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// Its object id, 32 lowercase hex digits.
    pub id: String,
    pub handle: String,
    pub date: String,
    /// The ticket part as written: `no ticket`, a list or a range.
    pub tickets: String,
    /// One per ticket, goal or request it is about, in the order of `tickets`.
    pub subjects: Vec<Subject>,
    pub title: String,
    pub decided_by: String,
    pub why: String,
    pub instead_of: String,
    pub topics: Vec<String>,
    /// Empty when there is none.
    pub body: String,
    /// The decisions it supersedes, in date order.
    pub supersedes: Vec<Linked>,
    /// The decisions that supersede it, in date order.
    pub superseded_by: Vec<Linked>,
    pub in_force: bool,
    pub pr: Option<String>,
    /// `None` when it is not known, such as for a caller without a checkout.
    pub on_main: Option<bool>,
    /// The member id of the member that wrote it.
    pub author: String,
    pub edited: bool,
    pub deleted: bool,
}

/// A ticket, goal or request a decision is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject {
    /// Its human-readable id as written, such as `NEX-22`.
    pub label: String,
    pub kind: SubjectKind,
    /// The other space's id; `None` for a subject in the decision's own space.
    pub space: Option<String>,
    /// The object it resolves to; `None` for a subject known only by its label.
    pub object: Option<String>,
    /// `ok`, `notAccessible` or `missing`, as NoCentralDB reads it; `None` with no `object`.
    pub status: Option<String>,
}

/// What a subject is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubjectKind {
    Ticket,
    Goal,
    Request,
}

impl SubjectKind {
    /// The kind as the JSON prints it.
    pub fn as_str(self) -> &'static str {
        match self {
            SubjectKind::Ticket => "ticket",
            SubjectKind::Goal => "goal",
            SubjectKind::Request => "request",
        }
    }
}

/// Another decision, as `supersedes` and `superseded_by` list it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Linked {
    pub id: String,
    pub handle: String,
}
