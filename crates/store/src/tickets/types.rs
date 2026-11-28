//! What a ticket is written from and read as (RFC 0002, "The ticket", "Commands, JSON and the
//! seam"): [`NewTicket`] holds what the writer of `nxs ticket create` gives, [`Ticket`] a ticket
//! as `nxs ticket show` prints it, its fields the record's JSON keys.

/// A ticket to create, as the writer gives it. The key, `createdBy`, `updatedBy` and the
/// timestamps are not here: `create` writes them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewTicket {
    /// One of `epic`, `bug`, `feature`, `chore`, `decision` and `question`.
    pub kind: String,
    /// One line.
    pub title: String,
    pub description: String,
    /// `P0` to `P4`, or the ordinal `0` to `4`; not set when `None`.
    pub priority: Option<String>,
    pub design: Option<String>,
    /// The acceptance criteria, `--dod`.
    pub completion_criterion: Option<String>,
    /// RFC 3339.
    pub due: Option<String>,
    /// RFC 3339.
    pub defer_until: Option<String>,
    /// The epic it belongs to, by key or object id.
    pub parent: Option<String>,
    /// The tickets that block it, each by key or object id.
    pub depends_on: Vec<String>,
    pub labels: Vec<String>,
    /// `member:<member id>` or `account:<account fingerprint>`.
    pub assignee: Option<String>,
}

/// A ticket as `nxs ticket show` prints it (RFC 0002, "Commands, JSON and the seam"). Derived
/// state is `None` or empty until its computation lands; so are the reverse edges, the notes,
/// work links and references, which later tickets write and read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    /// The object id, 32 lowercase hex digits.
    pub id: String,
    /// The key as `nxs` shows it: with `~` and the first digits of the object id when another
    /// ticket holds the same key.
    pub key: String,
    pub kind: String,
    pub title: String,
    pub status: String,
    pub resolution: Option<String>,
    /// The ordinal 0 to 4.
    pub priority: Option<i64>,
    pub assignee: Option<Holder>,
    /// The parent epic's key.
    pub parent: Option<String>,
    /// In byte order.
    pub labels: Vec<String>,
    pub due: Option<String>,
    pub defer_until: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub closed_at: Option<String>,
    pub archived_at: Option<String>,
    /// The request it serves or waits for (RFC 0009); `None` until requests exist.
    pub request: Option<TicketRequest>,
    pub derived: Derived,
    pub description: String,
    pub design: String,
    pub completion_criterion: String,
    pub closing_comment: Option<String>,
    /// The key of the ticket that replaced it.
    pub replaced_by: Option<String>,
    /// The keys of the tickets it replaced.
    pub replaces: Vec<String>,
    /// The tickets it depends on, in key order.
    pub deps: Vec<Edge>,
    /// The tickets that depend on it.
    pub dependents: Vec<Edge>,
    /// The keys of its children, in key order.
    pub children: Vec<String>,
    pub contributes_to: Vec<String>,
    pub mentions: Vec<String>,
    pub mentioned_by: Vec<String>,
    pub notes: Vec<Note>,
    pub work: Vec<WorkLink>,
    pub references: Vec<Reference>,
}

/// Who holds a ticket (RFC 0002, "Who holds a ticket").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    /// `member`, `person` or `text`.
    pub kind: String,
    /// The member id, the account fingerprint, or the text as written.
    pub id: String,
    /// The member's role from its `spaceMember` record, `owner` for the owner's account, `None`
    /// for text and for anyone else.
    pub role: Option<String>,
}

/// The request a ticket serves or waits for (RFC 0009).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketRequest {
    pub id: String,
    pub key: String,
    pub relation: String,
    pub status: Option<String>,
}

/// A ticket's derived state (RFC 0002, "Derived state"), computed on read; `None` while it is
/// not known.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Derived {
    pub ready: Option<bool>,
    pub blocked: Option<bool>,
    pub deferred: Option<bool>,
    pub done: Option<bool>,
    pub on_main: Option<bool>,
    /// `direct`, `inherited` or `none`.
    pub focus: Option<String>,
    pub focus_goal: Option<FocusGoal>,
    pub flags: Vec<String>,
}

/// The goal in focus a ticket ranks by (RFC 0005, "Ranking").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusGoal {
    pub id: String,
    pub key: String,
    pub title: String,
    pub position: i64,
}

/// Another ticket, as `deps` and `dependents` list it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub key: String,
    pub status: String,
    pub done: Option<bool>,
    pub on_main: Option<bool>,
}

/// A note on a ticket (RFC 0002, "Notes").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: String,
    pub body: String,
    pub reply_to: Option<String>,
    pub created_at: Option<String>,
    pub created_by: Option<String>,
}

/// A PR, branch or artifact linked to a ticket (RFC 0002, "Work links").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkLink {
    pub kind: String,
    pub url: Option<String>,
    pub branch: Option<String>,
    pub state: Option<String>,
    pub commit: Option<String>,
    pub on_main: Option<bool>,
    pub created_at: Option<String>,
}

/// A reference from a ticket out of its space (RFC 0002, "References out of the space").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub kind: String,
    pub space: String,
    pub id: String,
    pub label: Option<String>,
    pub state: String,
}
