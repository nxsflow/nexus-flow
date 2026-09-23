//! nexus-chat's store: the shared [`nxs_foundation::store::Store`] op-log plus chat's `message`
//! vocabulary folded over it (mirrors `MemoryStore`). Wraps the substrate, registers the message
//! reducer at open, and adds typed write/read helpers. Depends ONLY on the foundation (spec §4.4).

use crate::error::NxfError;
use crate::message_reducer::MessageReducer;
use crate::model::*;
use crate::schema;
use nxs_foundation::model::Op;
use nxs_foundation::store::Store as Substrate;
use rusqlite::{params, Connection, OptionalExtension};

/// chat's `view_watermarks.store_id` — distinct from flow's/memory's, so all three fold the SAME
/// log into their own views and advance independently (aye.36).
const CHAT_VIEWS: &str = "chat";

/// A full `messages`-view row — the app-facade's message read shape (be9y). Declared field order =
/// the view column order. `refs` is the stored JSON text (the facade parses it to [`Refs`]); the
/// `nxc` inbox/prime verbs project this down to their five-field `InboxOut`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRow {
    pub message_id: String,
    pub origin: String,
    pub channel_id: String,
    pub sender: String,
    pub kind: String,
    pub priority: String,
    pub disposition: String,
    pub thread_id: Option<String>,
    pub refs: Option<String>,
    pub body: String,
    pub created: Option<String>,
    /// Whether an agent action may follow the op behind this message here (nxf 6j6v.pzkb): this
    /// replica wrote it, or it was verified against a key on the trust list. Read with the row, at
    /// read time — see [`acts`].
    pub acts: bool,
}

/// **Which run and which step of a declared flow a slot thread stands for** (nxf 6j6v.553s (d)) —
/// what [`ChatStore::flow_mark`] answers with.
///
/// A struct rather than a tuple for [`LastReply`]'s reason one paragraph down: two strings whose
/// order a reader has to remember is two strings a transposition survives, and these two are both
/// ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowMark {
    /// The id of the message on the channel thread that started the run this slot belongs to.
    /// Empty for a mark written before this field existed, which is nothing this build ever wrote —
    /// the two are stamped together.
    pub run: String,
    /// [`crate::channel::FlowStep::id`].
    pub step: String,
}

/// **The newest reply of one thread's current turn, and what it IS** (nxf 6j6v.mqad, widened by nxf
/// 6j6v.kffm) — the row [`ChatStore::last_reply_kinds`] answers with.
///
/// A struct rather than a tuple because it now carries TWO orthogonal facts about the same message
/// and a `(String, String, bool)` gives a reader no way to tell which bool is which. Both are read
/// off ONE row of ONE query, for the reason [`ChatStore::last_reply_kind`] spells out: "the last
/// reply" has exactly one derivation in this crate, and a second one drifts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastReply {
    /// The thread this is the newest reply of.
    pub thread_id: String,
    /// Its `messages.kind` LABEL, verbatim — compared against
    /// [`crate::model::KIND_ESCALATION`] by whoever wants the narrower question. See
    /// [`ChatStore::last_reply_kind`] for why the raw label rather than a parsed variant.
    pub kind: String,
    /// **The RUNTIME wrote it, standing in for the agent that owed it** —
    /// [`crate::model::Refs::substituted`], read back out of the `refs` JSON column.
    ///
    /// Orthogonal to [`kind`](LastReply::kind) in both directions, which is why it is a second
    /// field and not another label: a substitution for a session that DIED escalates, one for a
    /// session that merely ended quietly does not, and a live agent's own `--escalate` is an
    /// escalation that is not a substitution.
    pub substituted: bool,
}

/// **One thread and the anchor of its most recent handover** ([`ChatStore::working_copy_anchors`],
/// nxf 6j6v.2af2) — [`LastReply`]'s shape for the other fact a message's `refs` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadAnchor {
    /// The thread this anchor was read off.
    pub thread_id: String,
    /// Where the working copy stood when that handover happened.
    pub anchor: crate::anchor::Anchor,
}

/// The `messages`-view columns in declared order, qualified with the `m.` alias every message read
/// uses. (An unqualified twin stood beside it, with a test pinning the two equal; it went when the
/// reads gained the [`acts`] column, which needs the alias — one list cannot drift from itself.)
const MESSAGE_COLS_M: &str = "m.message_id, m.origin, m.channel_id, m.sender, m.kind, m.priority, \
     m.disposition, m.thread_id, m.refs, m.body, m.created";

/// What every message read selects, in [`message_row`]'s order: [`MESSAGE_COLS_M`], then whether
/// an agent action may follow the message (nxf 6j6v.pzkb).
fn message_select() -> String {
    format!("{MESSAGE_COLS_M}, {}", acts("m"))
}

/// Map a `messages`-view row (selected as [`message_select`]) into a [`MessageRow`].
fn message_row(r: &rusqlite::Row) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        acts: r.get(11)?,
        message_id: r.get(0)?,
        origin: r.get(1)?,
        channel_id: r.get(2)?,
        sender: r.get(3)?,
        kind: r.get(4)?,
        priority: r.get(5)?,
        disposition: r.get(6)?,
        thread_id: r.get(7)?,
        refs: r.get(8)?,
        body: r.get(9)?,
        created: r.get(10)?,
    })
}

/// A channel as [`facade::channels`](crate::facade::channels) renders it (spec §5.1; `nxc channels
/// list` rendered it too until 6j6v.dvyq §3, and `Engine::channels` until 6j6v.yr59 — the row is
/// what `facade::public_channels`/`front_doors` still project from). Declared field order = the
/// JSON contract.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChannelRow {
    pub channel_id: String,
    pub name: Option<String>,
    pub kind: Option<String>,
    pub members: usize,
    pub degraded: bool,
}

/// One row of the runtime PROFILE table (spec §3.2). Declared field order = the JSON contract.
///
/// **Nothing on the surface renders this any more.** `nxc agents list`/`register`/`search` were the
/// three verbs that did, and 6j6v.dvyq §3 removed them: a team is DECLARED — a persona file in
/// `.nxs-personas/`, read through `Engine::directory`/`nxc list` — where a profile row lived in one
/// workspace's database and nothing reviewed it. The type and its reads stay because the ops that
/// wrote them are in the log's history and a replica may still fold one a peer wrote before the
/// removal; what is gone is the door.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProfileRow {
    pub handle: String,
    pub job_title: Option<String>,
    pub job_description: Option<String>,
    pub capability_tags: Option<String>,
    pub runtime_binding: Option<String>,
    pub reports_to: Option<String>,
    pub origin: Option<String>,
}

/// One `nxc search` hit (spec §5.1).
///
/// **The JSON contract is [`MessageHitView`](crate::facade::MessageHitView)'s, not this type's**
/// (nxf 6j6v.px98). Both surfaces — `Engine::search` and `nxc search --json` — now serialize the
/// VIEW, because the declared-`visibility` filter that both must apply lives at the facade and this
/// row is its input. The four projected fields and their order are unchanged; `thread_id` is what
/// the filter needs and deliberately does not travel outward.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MessageHit {
    pub message_id: String,
    pub channel_id: String,
    pub sender: String,
    pub body: String,
    /// The thread this hit sits in, or `None` for a message stamped with no thread. Read for the
    /// visibility filter: `requester_only` is a rule about a BOARD, so a hit can only be judged
    /// once the board it belongs to is known.
    pub thread_id: Option<String>,
}

/// The derived quorum state of one thread (spec §4.1–§4.3) — the heart of M2, a pure fold of the
/// log. `complete` is **clock-free**; `stale` is the ONE clock-dependent flag. Declared field order =
/// the JSON contract the `nxc` board + the app-facade both render (seam invariant).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ThreadQuorum {
    pub thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opener: Option<String>,
    /// The expected handles, in `expects_reply_from` declaration order (deterministic).
    pub expects: Vec<String>,
    /// The subset of `expects` that has posted ≥1 message in the thread (§4.1).
    pub replied: Vec<String>,
    /// `expects \ replied`, in declaration order (§4.1).
    pub outstanding: Vec<String>,
    /// `E ≠ ∅ ∧ outstanding = ∅` — clock-free, deterministic (§4.2). The wake trigger.
    pub complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    /// `deadline ≠ NULL ∧ now > deadline ∧ outstanding ≠ ∅` (§4.3) — the DERIVATION is exactly the
    /// one M2 specified and has not moved.
    ///
    /// **What HAS moved is what reads it, so do not carry M2's word "advisory" forward.** In M2
    /// nothing acted on this flag: it drove the "waiting past deadline" line of a requester's wake
    /// and stopped there. Since nxf 6j6v.pf6j it is load-bearing — `member_set_is_settled` treats a
    /// `stale` channel member as settled, so this flag is one of the two things that END a channel
    /// and release its consolidation to the requester. TB-M2-1 still holds and is a different
    /// question: a deadline never makes a thread `complete`; the consolidator's own reply does.
    ///
    /// On a channel MEMBER thread the deadline behind it is the resettable one of nxf 6j6v.nf38
    /// (`member_deadline.rs`), so `stale` there means "silent for the whole declared window", not
    /// "opened a long time ago". Everywhere else it is the thread's own op-folded register.
    pub stale: bool,
    /// **What this conversation is CALLED** (nxf 6j6v.e76c) — the thread's display name, absent for
    /// one nobody ever named.
    ///
    /// It rides on the quorum read because that is the read every thread surface is built on
    /// (`nxc threads list`/`show`, `nxc status`, the app-facade board), and the whole point of the
    /// name is that a surface which today can only print `dm:<24 hex>` has something readable to
    /// show instead.
    ///
    /// **Omitted rather than `null`**, exactly as [`deadline`](Self::deadline) and
    /// [`opener`](Self::opener) beside it: a thread with no name is not a degraded thread, it is the
    /// ordinary state every thread was in before this field existed, and a reader that finds no key
    /// falls back to what it rendered before — which is precisely the behaviour the item asks for.
    ///
    /// **Never a key.** The id is the identity; nothing here or above it joins on this string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// **Nothing here may act on this thread** (nxf 6j6v.pzkb): an op that shaped its obligation —
    /// its `open`, or the current value of its `expects_reply_from` or `deadline` register — came
    /// from a key this replica does not trust, carried no signature, or failed its check. A held
    /// thread is shown like any other, and is never [`complete`](Self::complete) nor
    /// [`stale`](Self::stale), so no consolidation, next step or wake follows from it.
    ///
    /// Omitted when false, so every thread this replica's own writers shaped reads exactly as before.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub held: bool,
}

/// One message in a completed board's wake render (spec §4.4/§5.2) — a lean projection of the
/// thread's messages. `completes` flags the single completing reply (the latest reply from an
/// expected handle), annotated `✓ completes <thread> (N/N)`. Declared field order = JSON contract.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WakeMessage {
    pub message_id: String,
    pub sender: String,
    pub body: String,
    /// `true` for the completing reply (the latest reply from an expected handle); `false` otherwise.
    pub completes: bool,
}

/// **A commission of the caller's that FINISHED while it was away** (nxf 6j6v.2hx9) — one entry of
/// the session-start notice. Declared field order = JSON contract.
///
/// It was "complete and NOT yet acked, cleared when the opener's channel read cursor passes
/// `completing_message_id`", and that clearing rule was the defect: nothing ever called the ack
/// that moved the cursor, so the notice never cleared and a session start accumulated finished work
/// without bound — the same shape nxf 6j6v.1vxs took out of the operation view, one surface
/// further. The list is a WINDOW now: what finished since this caller's previous session ended.
/// Nothing has to clear it, because "finished" does not become unfinished, and the window moves on
/// its own with every session that ends. (The cursor it was gated on is itself gone since nxf
/// 6j6v.4d2z, which is what that replacement made possible.)
///
/// **Nothing on the CLI renders it** (nxf 6j6v.1gm9): the `prime` block that replayed these boards
/// in full text and the two verbs that went with it (`nxc inbox`, `nxc read`) are all removed. What
/// is left is an APP-seam record — `prime --json`'s `threads_you_opened`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CompletedThread {
    pub thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// The expected handles (N = `expects.len()`); every one has replied (complete).
    pub expects: Vec<String>,
    /// The thread's messages, id-ordered — the completed board and its replies; exactly one carries
    /// `completes = true`.
    pub messages: Vec<WakeMessage>,
    /// The latest reply message in the thread (from an expected handle) — the completing reply.
    ///
    /// It WAS also "the cursor watermark that clears this wake", and nxf 6j6v.2hx9 struck that half
    /// out: nothing clears this list any more, because it is a WINDOW rather than a debt. See
    /// [`finished_at`](Self::finished_at) and [`ChatStore::opener_wake`].
    pub completing_message_id: String,
    /// **When it finished** — the completing reply's own instant (nxf 6j6v.2hx9), which is what
    /// puts this board inside the caller's window.
    ///
    /// Additive and always present: a reader that wants to sort a session start's notices, or to
    /// tell yesterday's finish from one that landed a minute ago, had to open every board to do it.
    pub finished_at: String,
}

/// A thread the caller opened that is **stale** — past its deadline with replies still outstanding
/// (spec §4.3/§4.4) — the requester-wake's advisory "waiting past deadline" list. It self-clears
/// when the board completes, the deadline lifts, or all reply. Declared field order = JSON
/// contract.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StaleThread {
    pub thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    /// The expected handles that still owe a reply (§4.1).
    pub outstanding: Vec<String>,
}

/// The requester-wake surface for a caller `C` (spec §4.4/§5.2): the threads `C` opened that are
/// finished (`complete`) or stopped (`stale`) inside the caller's window. Both lists are deterministically ordered
/// (channel + thread ULID); an empty wake omits its `inbox`/`prime` block entirely. Declared field
/// order = JSON contract.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OpenerWake {
    pub complete: Vec<CompletedThread>,
    pub stale: Vec<StaleThread>,
}

impl OpenerWake {
    /// The wake is empty when nothing the caller opened reached a terminal state inside its own
    /// window — and for a caller with no window at all (nxf 6j6v.2hx9). It
    /// governed whether the `prime` block rendered a "Threads you opened" section (spec §5.2);
    /// since nxf 6j6v.1gm9 removed that section it governs only whether `prime --json` carries the
    /// additive `threads_you_opened` key.
    pub fn is_empty(&self) -> bool {
        self.complete.is_empty() && self.stale.is_empty()
    }
}

/// One raw `(thread, expected-handle)` row from the bulk quorum join, before per-thread assembly.
struct QuorumRow {
    thread_id: String,
    channel_id: Option<String>,
    opener: Option<String>,
    deadline: Option<String>,
    /// The thread's display name register (nxf 6j6v.e76c); `None` for a thread nobody named.
    name: Option<String>,
    /// The expected handle for this row; `None` for a thread with empty/NULL `expects_reply_from`
    /// (the `LEFT JOIN json_each` still yields one row so the thread is not dropped).
    handle: Option<String>,
    replied: bool,
    /// See [`ThreadQuorum::held`].
    held: bool,
}

/// **Whether an agent action may follow the op behind a row** (nxf 6j6v.pzkb) — a message, or the
/// winning op of a register, named by its `(lamport, site)` coordinate: `lamport`/`site` are SQL
/// expressions for it. The coordinate identifies at most one op (6j6v.fc5p), and every chat row
/// carries the coordinate of the op that folded it, so this needs no view column of its own.
///
/// The definition itself is the substrate's, not a chat copy: the view `acting_ops` — this
/// replica's own ops, and those verified by a key on its trust list. It is asked at READ time,
/// because trust changes: take a key off the list and every row it signed stops carrying actions at
/// once.
fn acts_at(lamport: &str, site: &str) -> String {
    format!("EXISTS(SELECT 1 FROM acting_ops a WHERE a.lamport = {lamport} AND a.site = {site})")
}

/// [`acts_at`] for the message aliased `msg`.
pub(crate) fn acts(msg: &str) -> String {
    acts_at(&format!("{msg}.lamport"), &format!("{msg}.site"))
}

/// **What discharges a turn** (nxf 6j6v.cg8g + 6j6v.pzkb): a message written after the thread's
/// current declaration ([`after_the_declaration`]) whose op may carry an action ([`acts`]).
///
/// Every reader that asks "did somebody answer?" — the quorum's `replied`, the assignee edge, the
/// discharge position, the collected answers, the completing reply, the last reply's kind — splices
/// THIS, so a message nobody here can vouch for is visible in the thread and discharges nothing:
/// otherwise anybody who can reach the relay could answer for a member by writing a message that
/// claims the member's name, and the round would move on.
fn answers(msg: &str) -> String {
    format!("({} AND {})", after_the_declaration(msg), acts(msg))
}

/// **The turn watermark (nxf 6j6v.cg8g), written once.** Whether a message from alias `msg` was
/// written AFTER the obligation the thread `t` currently declares — the row-value comparison
/// `message_reducer::fold_lww` resolves that register with, so the predicate and the register agree
/// by construction rather than by coincidence.
///
/// A function taking the alias rather than a `const` because it is spliced next to three different
/// message aliases in two queries, and cg8g's own acceptance is that this comparison exists ONCE: a
/// second copy is how "discharge belongs to the turn" starts meaning two different things in two
/// fields of the same JSON object, which is exactly the defect this parameterization closes
/// ([`assignee_sql`]'s own doc says which one).
fn after_the_declaration(msg: &str) -> String {
    format!("({msg}.lamport, {msg}.site) > (t.expects_reply_from_v, t.expects_reply_from_site)")
}

/// The ONE quorum projection, with `filter` (a whole `WHERE` clause, or empty) spliced in — shared
/// by [`ChatStore::thread_quorums`] and [`ChatStore::thread_quorums_all`] so the `replied` predicate
/// exists exactly once. `filter` is never caller data: both call sites pass a literal, one of them
/// with a `?`-placeholder list generated from a slice LENGTH.
///
/// **The deadline is the member's clock when there is one** (nxf 6j6v.nf38). A channel member's
/// deadline is restarted by every write to the ONE session's transcript behind its thread, and that
/// live value lives in the device-local `channel_member_deadline` table (see `member_deadline.rs`
/// for why it cannot ride the op log). Joining it HERE, into the one
/// projection `stale` is derived from, is what keeps a single staleness rule in the crate: nothing
/// downstream — `member_set_is_settled`, `nxc tick`, `opener_wake`'s "waiting past deadline"
/// list, `nxc status` — learned a second way to ask. A thread with no armed clock (every thread
/// outside a declared channel, and every member whose deadline was given as an absolute instant)
/// falls through to its own op-folded register exactly as before.
fn quorum_sql(filter: &str) -> String {
    let replied_since = answers("m");
    let held = held_sql();
    format!(
        "SELECT t.thread_id, t.channel_id, t.opener,
                COALESCE(md.deadline, t.deadline) AS deadline, t.name AS name, je.value AS handle,
                CASE WHEN je.value IS NULL THEN 0
                     ELSE EXISTS(SELECT 1 FROM messages m
                                 WHERE m.thread_id = t.thread_id AND m.sender = je.value
                                   AND {replied_since})
                END AS replied,
                {held} AS held
           FROM threads t
           LEFT JOIN channel_member_deadline md ON md.thread_id = t.thread_id
           LEFT JOIN json_each(t.expects_reply_from) je
           {filter}
          ORDER BY t.channel_id, t.thread_id, je.key"
    )
}

/// **Whether the thread `t` is held** (nxf 6j6v.pzkb) — see [`ThreadQuorum::held`]. True when an
/// op that shaped its obligation may not carry an action here: any of its `open` ops, or the
/// winning op of its `expects_reply_from` or `deadline` register (`_v = 0` is a register nobody
/// set). Fail-closed on the root: a second, unvouched `open` for a thread's id holds it too.
fn held_sql() -> String {
    let unvouched_open = format!(
        "EXISTS(SELECT 1 FROM ops o
                 WHERE o.target_id = t.thread_id AND o.domain = '{domain}'
                   AND o.target_kind = '{kind}' AND o.field = '{root}' AND o.op_type = '{open}'
                   AND NOT {acts})",
        domain = crate::model::DOMAIN_MESSAGE,
        kind = crate::model::KIND_THREAD,
        root = crate::model::FIELD_ROOT,
        open = crate::model::OP_OPEN,
        acts = acts("o"),
    );
    let expects = acts_at("t.expects_reply_from_v", "t.expects_reply_from_site");
    let deadline = acts_at("t.deadline_v", "t.deadline_site");
    format!(
        "({unvouched_open}
          OR (COALESCE(t.expects_reply_from_v, 0) <> 0 AND NOT {expects})
          OR (COALESCE(t.deadline_v, 0) <> 0 AND NOT {deadline}))"
    )
}

/// The ONE assignee-session projection (nxf 6j6v.1q6d), with `filter` (a whole `WHERE` clause, or
/// empty) spliced in — shared by [`ChatStore::thread_assignee_sessions`] and its scoped twin, the
/// same shape and the same reason as [`quorum_sql`] above. `filter` is never caller data.
///
/// **Two sources, in this order, because the evidence lives in two places** (nxf 6j6v.1q6d):
///
/// 1. **The session put to work here that has not spoken THIS TURN** — `session_map.thread`, the
///    position every trigger records (nxf 6j6v.a71h §3.1). This is the branch that matters: an OPEN
///    thread is by definition one whose assignee has not answered, so on the very case the item
///    exists for there is no message to read a return address off. Guarded on "some expected handle
///    still owes an answer", so it only ever fires while somebody genuinely has not shown up; a
///    thread everyone has answered falls through to (2), where the interesting session is the one
///    that ANSWERED, not whoever happens to be standing there now (a supervisor resumed into a
///    settled thread must not displace it).
/// 2. **The return address of the newest message from an expected handle** — `refs.session_id`, what
///    a thread that HAS been answered carries.
///
/// **Both of (1)'s predicates are TURN-scoped, and that is not a detail** (re-review N1). They were
/// thread-lifetime once — "has never spoken HERE" — while everything printed beside them has been
/// turn-scoped since 6j6v.cg8g. On turn two of a multi-handle board (a answered, b silent) the
/// lifetime guard read "everyone has spoken at some point", closed this branch, and let (2) name
/// **a** while `outstanding` named **b**, in adjacent fields of the same JSON object. It could not
/// name a session that never worked on the thread, but it could name the wrong one. Both predicates
/// now carry [`after_the_declaration`] — cg8g's own comparison, spliced from the one place it is
/// written — so the guard is now exactly the question `outstanding` answers, and the exclusion is
/// exactly the question `replied` answers. They cannot drift again without the shared function
/// changing under all three.
///
/// "Newest" in (2) is the `(lamport, site, message_id)` causal order every other thread read uses —
/// NOT the raw `message_id`, which is arbitrary within a millisecond (0b1m). "Newest" in (1) is the
/// session's own `(created, internal_id)` — `created` is stamped at mint by
/// [`ChatStore::create_pending_session`] and never touched again, so this is session AGE, not
/// trigger recency: the youngest session standing on the thread wins. The candidate pool is any
/// silent session whose position is this thread, which is wider than "the members of a fan-out" — a
/// supervisor resumed into the thread without posting is one too. That ambiguity is real and
/// bounded: every candidate is a session genuinely working on this thread, and 6j6v.pf6j gives each
/// member a thread of its own, after which a thread has exactly one assignee.
///
/// **The one residual, named rather than left to be found.** When the thread is open but NO silent
/// session stands on it (the owed party's session was resumed elsewhere, or never existed on this
/// device), (1) yields nothing and (2) answers with the newest answer — on a multi-handle board that
/// can again be a different member than `outstanding` names. It is the honest fallback for "no
/// better evidence here", and on a 1:1 thread it is the RIGHT answer, because the only expected
/// handle is the owed one.
///
/// **Locality, worth naming.** Branch (1) reads a DEVICE-LOCAL table, so it answers only on the
/// machine that started the session — which is exactly where `agent_transcript` lives too, so the
/// edge and the thing it leads to have the same reach. Branch (2) rides the synced envelope and can
/// therefore name a session whose transcript this device does not hold; that is pre-existing and
/// unchanged.
fn assignee_sql(filter: &str) -> String {
    // The SAME comparison `replied` is computed with, spliced from the one place it is written.
    let (owed_since, spoke_since) = (answers("m2"), answers("m3"));
    // Branch (2) reads a RETURN ADDRESS — a session on this machine that a reply resumes — so the
    // message it is read off must be one an action may follow (nxf 6j6v.pzkb): a message claiming an
    // expected handle's name would otherwise pick the local session a reply wakes.
    let vouched = acts("m");
    format!(
        "SELECT t.thread_id,
                COALESCE(
                  CASE WHEN EXISTS(SELECT 1 FROM json_each(t.expects_reply_from) je
                                    WHERE NOT EXISTS(SELECT 1 FROM messages m2
                                                      WHERE m2.thread_id = t.thread_id
                                                        AND m2.sender = je.value
                                                        AND {owed_since}))
                       THEN (SELECT sm.internal_id FROM session_map sm
                              WHERE sm.thread = t.thread_id
                                AND NOT EXISTS(SELECT 1 FROM messages m3
                                                WHERE m3.thread_id = t.thread_id
                                                  AND json_extract(m3.refs, '$.session_id')
                                                      = sm.internal_id
                                                  AND {spoke_since})
                              ORDER BY sm.created DESC, sm.internal_id DESC
                              LIMIT 1)
                  END,
                  json_extract(
                    (SELECT m.refs FROM messages m
                      WHERE m.thread_id = t.thread_id AND {vouched}
                        AND EXISTS(SELECT 1 FROM json_each(t.expects_reply_from) je
                                    WHERE je.value = m.sender)
                      ORDER BY m.lamport DESC, m.site DESC, m.message_id DESC
                      LIMIT 1),
                    '$.session_id')
                )
           FROM threads t
           {filter}
          ORDER BY t.thread_id"
    )
}

/// The ONE "did the parent move on after this thread was discharged?" projection (nxf 6j6v.93zd),
/// with `filter` (an extra `AND …` clause, or empty) spliced in — the same shape and the same reason
/// as [`quorum_sql`] above: the predicate exists exactly once and only the scope differs. `filter` is
/// never caller data; the scoped call site passes a `?`-placeholder list generated from a slice
/// LENGTH.
///
/// # What it answers, and why the question is this one
///
/// A thread that is discharged, has no child and whose parent owes nothing is a **dead end**. The
/// derivation of nxf 6j6v.a71h calls every such thread `orphaned`, and that is too many: a FINISHED
/// branch and a FAILED one satisfy those three conditions alike — in a71h §2's chain the three
/// reviewer threads matched them the moment the `#review` supervisor consolidated their answers,
/// while the operation was perfectly healthy. A place for failed consequences that is full of
/// healthy threads is a place nobody reads, so 93zd owns the distinction.
///
/// This is the discriminator: take the thread's **discharge position `D`** — the greatest
/// `(lamport, site)` among the messages from a handle it EXPECTS that were written after the
/// obligation it currently declares, i.e. the newest message that [`quorum_sql`]'s own `replied`
/// counts — and ask whether the parent MOVED after it, in either of the two ways a parent can act on
/// an answer:
///
/// 1. **the parent said something after `D`** — in the §2 chain literally the supervisor's
///    consolidating discharge of the channel thread, which alone kills the reported false positive;
/// 2. **a SIBLING was opened out of the parent after `D`** — because a parent can consume an answer
///    by commissioning the next step instead of replying. "Opened after `D`" is read as "that
///    sibling has messages and every one of them is after `D`", since a thread's `open` op carries
///    no causal position of its own and its first message is what a thread starting looks like in
///    the record.
///
/// # The error direction, which is the whole point
///
/// Lamport order is a causal **upper** bound: anything genuinely caused by `D` is ordered after it,
/// but something ordered after it need not have been caused by it. So this over-reports "the parent
/// moved on", which REMOVES orphans — it errs towards missing a failure, never towards raising a
/// false alarm. That is the right direction for a state whose entire worth is being trustworthy when
/// it fires, and any change here that flips it makes the state worse rather than sharper.
///
/// Rows are thread ids, thread-id ordered; a thread absent from the result is one whose parent has
/// NOT moved on (including every thread that was never discharged at all).
///
/// # Cost, named rather than left to be discovered
///
/// This is correlated: per candidate thread it asks four `EXISTS` questions over `messages`, all of
/// them on `(thread_id, …)` and so served by the `messages_thread` index. It is one more query per
/// `nxc status`, on a read path whose scoped forms (`--thread`, `--channel`) already bind only the
/// threads they return. It is deliberately NOT folded into [`quorum_sql`]: that projection yields
/// one row per (thread, expected handle) and this yields one per thread, so joining them would make
/// the shared predicate harder to read for a saving of one prepared statement.
fn moved_on_sql(filter: &str) -> String {
    // The SAME comparison `replied` is computed with, spliced from the one place it is written.
    let (discharge, later_discharge) = (answers("d"), answers("d2"));
    format!(
        "SELECT t.thread_id
           FROM threads t
           JOIN messages d ON d.thread_id = t.thread_id
          WHERE t.parent IS NOT NULL {filter}
            AND EXISTS(SELECT 1 FROM json_each(COALESCE(t.expects_reply_from, '[]')) je
                        WHERE je.value = d.sender)
            AND {discharge}
            AND NOT EXISTS(SELECT 1 FROM messages d2
                            WHERE d2.thread_id = t.thread_id
                              AND EXISTS(SELECT 1
                                           FROM json_each(COALESCE(t.expects_reply_from, '[]')) je2
                                          WHERE je2.value = d2.sender)
                              AND {later_discharge}
                              AND (d2.lamport, d2.site) > (d.lamport, d.site))
            AND (EXISTS(SELECT 1 FROM messages pm
                         WHERE pm.thread_id = t.parent
                           AND (pm.lamport, pm.site) > (d.lamport, d.site))
              OR EXISTS(SELECT 1 FROM threads s
                         WHERE s.parent = t.parent
                           AND s.thread_id <> t.thread_id
                           AND EXISTS(SELECT 1 FROM messages sm
                                       WHERE sm.thread_id = s.thread_id
                                         AND (sm.lamport, sm.site) > (d.lamport, d.site))
                           AND NOT EXISTS(SELECT 1 FROM messages sm0
                                           WHERE sm0.thread_id = s.thread_id
                                             AND (sm0.lamport, sm0.site) <= (d.lamport, d.site))))
          ORDER BY t.thread_id"
    )
}

/// The empty bind list for the unfiltered reads — a typed `&[&str]` so `params_from_iter` has
/// something to infer from.
const EMPTY_IDS: &[&str] = &[];

/// One row of [`quorum_sql`]'s projection.
fn quorum_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<QuorumRow> {
    Ok(QuorumRow {
        held: r.get::<_, i64>(7)? != 0,
        thread_id: r.get(0)?,
        channel_id: r.get(1)?,
        opener: r.get(2)?,
        deadline: r.get(3)?,
        name: r.get(4)?,
        handle: r.get(5)?,
        replied: r.get::<_, i64>(6)? != 0,
    })
}

/// Assemble ordered `(thread, expected-handle)` rows (from the bulk quorum query) into per-thread
/// [`ThreadQuorum`] values. Rows are pre-ordered `(channel_id, thread_id, handle-index)`, so a
/// thread's rows are contiguous — a new group starts when `thread_id` changes. `now` drives ONLY
/// the clock-dependent `stale` flag (§4.3); `complete` is clock-free (§4.2).
fn assemble_quorums(rows: Vec<QuorumRow>, now: &str) -> Vec<ThreadQuorum> {
    let mut out: Vec<ThreadQuorum> = Vec::new();
    for row in rows {
        if out.last().map(|q| q.thread_id.as_str()) != Some(row.thread_id.as_str()) {
            out.push(ThreadQuorum {
                thread_id: row.thread_id,
                channel_id: row.channel_id,
                opener: row.opener,
                expects: Vec::new(),
                replied: Vec::new(),
                outstanding: Vec::new(),
                complete: false,
                deadline: row.deadline,
                stale: false,
                name: row.name,
                held: row.held,
            });
        }
        if let Some(h) = row.handle {
            let q = out.last_mut().unwrap();
            q.expects.push(h.clone());
            if row.replied {
                q.replied.push(h);
            } else {
                q.outstanding.push(h);
            }
        }
    }
    for q in &mut out {
        // complete(T) = E ≠ ∅ ∧ outstanding = ∅ (§4.2); stale(T,now) = deadline≠NULL ∧ now>deadline
        // ∧ outstanding≠∅ (§4.3, lexicographic on absolute RFC3339-Z instants). A complete thread is
        // never stale (outstanding = ∅).
        //
        // A HELD thread is neither (nxf 6j6v.pzkb): both flags are what an action follows — a
        // completion is consolidated and routed, a lapsed member ends its channel — and an op this
        // replica cannot vouch for shaped the obligation they are computed from.
        q.complete = !q.held && !q.expects.is_empty() && q.outstanding.is_empty();
        q.stale = !q.held
            && matches!(q.deadline.as_deref(), Some(d) if now > d)
            && !q.outstanding.is_empty();
    }
    out
}

/// Internal discriminator for the shared profile query (list / by-handle / substring-search).
enum ProfileFilter<'a> {
    Handle(&'a str),
    Search(&'a str),
}

/// Escape LIKE metacharacters so a user query matches them literally: a bound parameter stops SQL
/// *injection*, but `%`/`_` are still LIKE wildcards. Pair with `LIKE ?n ESCAPE '\'` (spec §5.1 /
/// review 3cz4: bound, literal substring — no interpolation, no accidental wildcard).
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The read-side degraded-DM contract (spec §3.3): a `direct` channel whose resolved membership is
/// not exactly 2 is degraded. ONE definition, shared by [`ChatStore::is_degraded_dm`] and
/// [`ChatStore::list_member_channels`], so the invariant can never drift between them.
fn is_degraded(kind: Option<&str>, member_count: usize) -> bool {
    kind == Some(crate::model::CHANNEL_KIND_DIRECT) && member_count != 2
}

pub struct ChatStore {
    inner: Substrate,
}

impl ChatStore {
    pub fn open_in_memory(site: i64) -> ChatStore {
        let mut inner = Substrate::open_in_memory(site);
        let upgraded = schema::try_apply_chat_views(inner.connection()).expect("chat views");
        inner.register_reducer(Box::new(MessageReducer));
        Self::materialize_views(&mut inner, upgraded);
        ChatStore { inner }
    }

    pub fn open(path: &str, site: i64) -> rusqlite::Result<ChatStore> {
        let mut inner = Substrate::open(path, site)?;
        let upgraded = schema::try_apply_chat_views(inner.connection())?;
        inner.register_reducer(Box::new(MessageReducer));
        Self::materialize_views(&mut inner, upgraded);
        Ok(ChatStore { inner })
    }

    /// Bind chat's views to the shared log after opening. Steady state is the O(1)
    /// [`refold_if_behind`](Substrate::refold_if_behind) (fold only what a sync pull moved). When the
    /// open UPGRADED a pre-M2 `threads` table (the sparse-key deadline minor, spec §7),
    /// [`force_refold`](Substrate::force_refold) instead — the deferred `set deadline` ops sit BELOW
    /// the advanced watermark, so only an unconditional refold resurfaces them.
    fn materialize_views(inner: &mut Substrate, upgraded: bool) {
        if upgraded {
            inner.force_refold(CHAT_VIEWS);
        } else {
            inner.refold_if_behind(CHAT_VIEWS);
        }
    }

    // ---- substrate delegation -------------------------------------------------
    pub fn set_wall_clock(&mut self, now: &str) {
        self.inner.set_wall_clock(now);
    }
    pub fn connection(&self) -> &Connection {
        self.inner.connection()
    }
    pub fn data_version(&self) -> rusqlite::Result<i64> {
        self.inner.data_version()
    }
    pub fn export(&self) -> Vec<Op> {
        self.inner.export()
    }
    pub fn apply(&mut self, ops: &[Op]) -> Vec<Op> {
        self.inner.apply(ops)
    }
    pub fn refold(&mut self) {
        self.inner.refold();
    }

    // ---- who wrote an op, and whom this replica believes (nxf 6j6v.pzkb) ---------------------
    // Delegations to the substrate, which holds the key, the verdicts and the trust list for the
    // whole workspace (see `nxs_foundation::trust`). The decision reads above do not call these —
    // they join `acting_ops` in SQL (see `acts`) — these are for callers that hold one op id.

    /// This replica's key id — what another replica trusts to believe it.
    pub fn key_id(&self) -> &str {
        self.inner.key_id()
    }

    /// Put a key on the trust list; `true` when it was not on it before.
    pub fn trust_key(&mut self, key_id: &str, name: &str, now: &str) -> crate::error::Result<bool> {
        self.inner.trust_key(key_id, name, now)
    }

    /// Take a key off the trust list; `true` when it was on it.
    pub fn distrust_key(&mut self, key_id: &str) -> crate::error::Result<bool> {
        self.inner.distrust_key(key_id)
    }

    /// Where one op came from, and whether an action may follow it.
    pub fn op_provenance(&self, op_id: &str) -> Option<nxs_foundation::trust::OpProvenance> {
        self.inner.op_provenance(op_id)
    }

    /// Where the op behind one MESSAGE came from — the message row carries the coordinate of the op
    /// that folded it, and that coordinate names exactly one op (6j6v.fc5p). `None` for a message
    /// this replica does not hold. What an app shows beside a message it cannot vouch for.
    pub fn message_provenance(
        &self,
        message_id: &str,
    ) -> crate::error::Result<Option<nxs_foundation::trust::OpProvenance>> {
        let op_id: Option<String> = self
            .inner
            .connection()
            .query_row(
                "SELECT o.op_id FROM messages m
                   JOIN ops o ON o.lamport = m.lamport AND o.site = m.site
                  WHERE m.message_id = ?1
                  LIMIT 1",
                [message_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(op_id.and_then(|op_id| self.inner.op_provenance(&op_id)))
    }

    /// Mint a chat id (spec §2.2). Production: `<prefix>` + a fresh ULID (global, collision-free).
    /// Under the golden-docs determinism switch ([`deterministic_ids_enabled`]): `<prefix>` + a
    /// 26-digit zero-padded, lexicographically-sortable sequence (one past the current row count of
    /// `count_table`), so send/channel/thread goldens are byte-stable and still time-sortable. A
    /// test-only seam — production never sets the flag.
    ///
    /// [`deterministic_ids_enabled`]: nxs_foundation::workspace::deterministic_ids_enabled
    fn mint_id(&self, prefix: &str, count_table: &str) -> String {
        if nxs_foundation::workspace::deterministic_ids_enabled() {
            let n: i64 = self
                .inner
                .connection()
                .query_row(&format!("SELECT COUNT(*) FROM {count_table}"), [], |r| {
                    r.get(0)
                })
                .unwrap_or(0);
            return format!("{prefix}{:026}", n + 1);
        }
        format!("{prefix}{}", ulid::Ulid::new())
    }

    /// Mint a group-channel id (`m-`+ULID; deterministic under the golden switch). DM ids are NOT
    /// minted here — they are derived deterministically from the two handles (spec §3.2), computed by
    /// the CLI.
    pub fn mint_channel_id(&self) -> String {
        self.mint_id("m-", "channels")
    }

    /// Mint a thread id (`m-`+ULID; deterministic under the golden switch).
    pub fn mint_thread_id(&self) -> String {
        self.mint_id("m-", "threads")
    }

    /// Mint an internal (nxc-owned) role-runtime session id (`m-`+ULID; deterministic under the
    /// golden switch, count anchored on `session_map`'s own row count) — the seam every persona
    /// summon crosses into `create_pending_session` (nxf epic 6j6v.zenf, T6; typed `send --role`
    /// then, `send --to <persona>` since 6j6v.dvyq §3). The real Claude Agent SDK session id is
    /// never exposed here; it is bound later via `bind_session`.
    pub fn mint_session_id(&self) -> String {
        self.mint_id("m-", "session_map")
    }

    // ---- write helpers --------------------------------------------------------
    /// Append a message and return its minted global id (`m-`+ULID, spec §2.2; deterministic under
    /// the golden switch). The envelope rides in the op's `value`; the reducer folds it into
    /// `messages`. The op author IS the message sender (the acting agent's qualified handle).
    pub fn post_message(&mut self, env: &MessageEnvelope) -> String {
        let message_id = self.mint_id("m-", "messages");
        let value = serde_json::to_string(env).expect("envelope serializes");
        self.inner.emit(
            DOMAIN_MESSAGE,
            KIND_MESSAGE,
            &message_id,
            FIELD_ENVELOPE,
            OP_POST,
            Some(value),
            &env.sender,
        );
        message_id
    }

    pub fn set_channel_field(&mut self, channel_id: &str, field: &str, value: &str, author: &str) {
        // The reducer's is_foldable whitelists CHANNEL_FIELDS for (KIND_CHANNEL, OP_SET); a field
        // not on that list can never fold (fold_lww's format!-built column name relies on the same
        // whitelist for injection-safety). Fail loudly here so a typo'd field is a caller-visible
        // bug, not a silently-never-folding op sitting in the log (review finding).
        assert!(
            CHANNEL_FIELDS.contains(&field),
            "unknown channel field: {field}"
        );
        self.inner.emit(
            DOMAIN_MESSAGE,
            KIND_CHANNEL,
            channel_id,
            field,
            OP_SET,
            Some(value.to_string()),
            author,
        );
    }

    pub fn set_profile_field(&mut self, handle: &str, field: &str, value: &str, author: &str) {
        // Same guard as set_channel_field, mirrored against PROFILE_FIELDS (spec §4.1's other
        // per-field LWW kind).
        assert!(
            PROFILE_FIELDS.contains(&field),
            "unknown profile field: {field}"
        );
        self.inner.emit(
            DOMAIN_MESSAGE,
            KIND_PROFILE,
            handle,
            field,
            OP_SET,
            Some(value.to_string()),
            author,
        );
    }

    pub fn add_member(&mut self, channel_id: &str, handle: &str, author: &str) {
        // A SEP byte inside either component would make the reducer's `channel{SEP}handle` split2
        // parse ambiguously (or, for a handle/channel_id that itself contains further SEPs, produce
        // >2 parts and fail entirely) — the composite id would silently never fold, corrupting the
        // OR-set tag (Task 5 review finding). Locally-minted ids never contain SEP; a SEP reaching
        // here is a programming error, so fail loudly in all profiles — mirrors flow's `edge_target`
        // assert (`crates/core/src/store.rs`). No pre-validation facade exists yet, so this is the
        // ONLY defense; `debug_assert!` would be stripped in release builds and leave none.
        assert!(
            !channel_id.contains(SEP) && !handle.contains(SEP),
            "membership id must not contain the U+001F separator: {channel_id:?} / {handle:?}"
        );
        let tid = format!("{channel_id}{SEP}{handle}");
        self.inner.emit(
            DOMAIN_MESSAGE,
            KIND_MEMBERSHIP,
            &tid,
            "member",
            OP_ADD,
            None,
            author,
        );
    }

    /// Remove `handle` from `channel_id`, tombstoning every add-tag currently observed for it
    /// (observed-remove: only adds we can see are removed; a concurrent unseen add survives).
    pub fn remove_member(&mut self, channel_id: &str, handle: &str, author: &str) {
        // Same SEP guard as `add_member` — same composite id, same corruption risk (Task 5 review).
        assert!(
            !channel_id.contains(SEP) && !handle.contains(SEP),
            "membership id must not contain the U+001F separator: {channel_id:?} / {handle:?}"
        );
        let tags: Vec<String> = {
            let mut st = self
                .inner
                .connection()
                .prepare(
                    "SELECT tag FROM membership_adds WHERE channel_id=?1 AND handle=?2
                   AND tag NOT IN (SELECT tag FROM membership_removes)",
                )
                .unwrap();
            st.query_map(params![channel_id, handle], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        if tags.is_empty() {
            return;
        }
        let value = tags.join(&SEP.to_string());
        let tid = format!("{channel_id}{SEP}{handle}");
        self.inner.emit(
            DOMAIN_MESSAGE,
            KIND_MEMBERSHIP,
            &tid,
            "member",
            OP_REMOVE,
            Some(value),
            author,
        );
    }

    /// Open a thread: the immutable root (spec §3.5). Grow-only in the reducer, so re-opening the
    /// same `thread_id` with an identical root converges; a distinct root under the same id is a
    /// caller bug (the store does not check — the reducer's `ON CONFLICT DO UPDATE` just re-applies
    /// the latest one seen, order-independent only because roots are expected identical).
    pub fn open_thread(&mut self, thread_id: &str, root: &ThreadRoot, author: &str) {
        let value = serde_json::to_string(root).expect("thread root serializes");
        self.inner.emit(
            DOMAIN_MESSAGE,
            KIND_THREAD,
            thread_id,
            FIELD_ROOT,
            OP_OPEN,
            Some(value),
            author,
        );
    }

    /// Set/clear who a thread expects a reply from (a keep-if-beats LWW register, spec §3.5).
    pub fn set_expects_reply_from(&mut self, thread_id: &str, value: &str, author: &str) {
        self.inner.emit(
            DOMAIN_MESSAGE,
            KIND_THREAD,
            thread_id,
            FIELD_EXPECTS_REPLY_FROM,
            OP_SET,
            Some(value.to_string()),
            author,
        );
    }

    /// Set a thread's `deadline` (absolute RFC3339, spec §2.1/§3.1) — a sparse keep-if-beats LWW
    /// register that folds on the SAME path as `expects_reply_from`. Callers resolve a `--deadline`
    /// duration to an absolute instant BEFORE this (determinism, TB-M2-6); the store only stores it.
    ///
    /// Deliberately NOT called "advisory" any more — see [`ThreadQuorum::stale`] for what reads what
    /// this writes, and `member_deadline.rs` for the per-member clock that shadows it on a channel
    /// member's thread.
    pub fn set_deadline(&mut self, thread_id: &str, deadline: &str, author: &str) {
        self.inner.emit(
            DOMAIN_MESSAGE,
            KIND_THREAD,
            thread_id,
            FIELD_DEADLINE,
            OP_SET,
            Some(deadline.to_string()),
            author,
        );
    }

    /// Set a thread's display `name` (nxf 6j6v.e76c) — the third keep-if-beats LWW register, on the
    /// same fold path as the two above.
    ///
    /// **The store only stores it.** The SEVEN-word bound, the sanitising and the once-only rule are
    /// [`crate::naming`]'s and [`crate::facade::name_thread`]'s respectively: this emit is the
    /// primitive both stand on, exactly as [`set_deadline`](Self::set_deadline) is the primitive
    /// under a resolved `--deadline`.
    pub fn set_thread_name(&mut self, thread_id: &str, name: &str, author: &str) {
        self.inner.emit(
            DOMAIN_MESSAGE,
            KIND_THREAD,
            thread_id,
            FIELD_NAME,
            OP_SET,
            Some(name.to_string()),
            author,
        );
    }

    /// Name the machine a persona chat runs on (nxf 6j6v.1c6k) — the fourth keep-if-beats LWW
    /// register, on the same fold path as the three above. The store only stores it: which machine,
    /// and whether it may be named at all, is [`crate::machine`]'s.
    pub fn set_thread_machine(&mut self, thread_id: &str, machine_id: &str, author: &str) {
        self.inner.emit(
            DOMAIN_MESSAGE,
            KIND_THREAD,
            thread_id,
            FIELD_MACHINE,
            OP_SET,
            Some(machine_id.to_string()),
            author,
        );
    }

    /// The machine a chat runs on, and whether the op that named it may carry an action here
    /// (nxf 6j6v.1c6k + 6j6v.pzkb). `None` for a thread no machine was named for, and for a thread
    /// id this workspace does not know.
    ///
    /// `acts` is asked at read time, like every other trust question: a register written by a key
    /// nobody here trusts names a machine that nobody here acts for — it is shown, and it moves
    /// nothing.
    pub fn thread_machine(
        &self,
        thread_id: &str,
    ) -> crate::error::Result<Option<crate::machine::ThreadMachine>> {
        let acts = acts_at("t.machine_v", "t.machine_site");
        let row: Option<(Option<String>, bool)> = self
            .inner
            .connection()
            .query_row(
                &format!("SELECT t.machine, {acts} FROM threads t WHERE t.thread_id=?1"),
                [thread_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            Some((Some(machine_id), acts)) => {
                Some(crate::machine::ThreadMachine { machine_id, acts })
            }
            _ => None,
        })
    }

    /// The chats whose `machine` register names `machine_id` AND whose winning op may carry an
    /// action here (nxf 6j6v.1c6k) — what a pickup walks. Ordered by thread id, which is a ULID and
    /// therefore oldest first.
    pub fn threads_designated_to(&self, machine_id: &str) -> crate::error::Result<Vec<String>> {
        let conn = self.inner.connection();
        let mut st = conn.prepare(&format!(
            "SELECT t.thread_id FROM threads t WHERE t.machine = ?1 AND {} ORDER BY t.thread_id",
            acts_at("t.machine_v", "t.machine_site")
        ))?;
        let rows = st
            .query_map([machine_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
    }

    /// **The messages a persona on `machine_id` still owes a turn for** (nxf 6j6v.1c6k) — in every
    /// chat designated to that machine (acting register), the acting messages after the persona's
    /// own last acting one, oldest first. The service's cheap question after a pass: whether any
    /// of these is not yet claimed on the machine decides if a pickup is started at all.
    ///
    /// The persona is the one party the chat was opened to expect; a chat with any other shape, and
    /// a held one, owes nothing here.
    pub fn orders_owed(&self, machine_id: &str) -> crate::error::Result<Vec<String>> {
        let mut owed = Vec::new();
        for thread in self.threads_designated_to(machine_id)? {
            let Some(q) = self.thread_quorum(&thread, "")? else {
                continue;
            };
            let [persona] = q.expects.as_slice() else {
                continue;
            };
            if q.held {
                continue;
            }
            let msgs = self.acting_messages_in_thread(&thread)?;
            let since = match msgs.iter().rposition(|m| &m.sender == persona) {
                Some(last) => &msgs[last + 1..],
                None => &msgs[..],
            };
            owed.extend(since.iter().map(|m| m.message_id.clone()));
        }
        Ok(owed)
    }

    /// A thread's display `name`, or `None` for one nobody ever named (nxf 6j6v.e76c) — and for a
    /// thread id this workspace does not know, which is deliberately the same answer: both mean
    /// "there is no name to show here", and a surface does the same thing with either.
    ///
    /// The point read behind [`crate::facade::name_thread`]'s once-only rule and behind every
    /// composer that wants to say what a conversation is called. A db error → `io`.
    pub fn thread_name(&self, thread_id: &str) -> crate::error::Result<Option<String>> {
        let row: Option<Option<String>> = self
            .inner
            .connection()
            .query_row(
                "SELECT name FROM threads WHERE thread_id=?1",
                [thread_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row.flatten())
    }

    // ---- reads ----------------------------------------------------------------
    /// The resolved OR-set membership of a channel (present handles, sorted). A db error surfaces as
    /// `io` rather than panicking — this read is on the long-lived `Engine`'s hot path (be9y review).
    pub fn resolve_membership(&self, channel_id: &str) -> crate::error::Result<Vec<String>> {
        let conn = self.inner.connection();
        let mut st = conn.prepare(
            "SELECT handle FROM membership_adds WHERE channel_id=?1
               AND tag NOT IN (SELECT tag FROM membership_removes) ORDER BY handle",
        )?;
        let rows = st
            .query_map([channel_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The channel's declared `kind` register (`group` | `direct` | `public`, an OPEN string on the
    /// wire), or `None` when the channel has no row or no `kind` set yet — a channel that exists
    /// only through its membership OR-set is exactly that case, and it is a legitimate one, not a
    /// fault. A db error → `io`.
    pub fn channel_kind(&self, channel_id: &str) -> crate::error::Result<Option<String>> {
        let row: Option<Option<String>> = self
            .inner
            .connection()
            .query_row(
                "SELECT kind FROM channels WHERE channel_id=?1",
                [channel_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row.flatten())
    }

    /// A `direct` channel whose resolved membership is not exactly 2 is degraded — the read-side
    /// defensive contract (spec §3.3); callers must not assume "exactly 2". A db error → `io`.
    ///
    /// **Changed with nxf 6j6v.bd6g** (independent review, Code Quality #1): a channel row whose
    /// `kind` is still NULL used to come back as an `io` error, because the column was read as a
    /// non-nullable `String`. It is nullable in the schema and a half-folded row (name folded, kind
    /// not yet) is a legitimate state ops pass through, so that was a fault reported for a healthy
    /// database. It now answers the question asked: no kind ⇒ not a degraded DM ⇒ `false`.
    pub fn is_degraded_dm(&self, channel_id: &str) -> crate::error::Result<bool> {
        Ok(is_degraded(
            self.channel_kind(channel_id)?.as_deref(),
            self.resolve_membership(channel_id)?.len(),
        ))
    }

    // `inbox(consumer, include_next_session)` stood here — the unread derivation of spec §4, the
    // one read that consumed the `read_cursors` watermark. REMOVED with the whole unread apparatus
    // (nxf 6j6v.4d2z); what a caller wants from a channel is its CONVERSATION, which is
    // [`messages_in_channel`](Self::messages_in_channel) and [`messages_in_thread`](Self::
    // messages_in_thread).

    /// All messages in `channel_id`, in deterministic causal order — the app-facade's channel
    /// history read (be9y). Ordered by the CRDT clock `(lamport, site, message_id)`, NOT the raw
    /// `message_id`: two `m-`+ULID ids minted in the same millisecond sort by ULID randomness, so
    /// `message_id` order is arbitrary within a millisecond (0b1m); lamport is the substrate's
    /// logical clock (a later op has a higher lamport), so this is a total, reproducible post order
    /// that also holds across sites/merges. Membership gating is the facade's, not this pure read's.
    /// A db error surfaces as `io`.
    pub fn messages_in_channel(&self, channel_id: &str) -> crate::error::Result<Vec<MessageRow>> {
        let conn = self.inner.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {} FROM messages m WHERE m.channel_id=?1 \
             ORDER BY m.lamport, m.site, m.message_id",
            message_select()
        ))?;
        let rows = st
            .query_map([channel_id], message_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// All messages carrying `thread_id`, in deterministic causal order (`lamport, site,
    /// message_id`, not raw `message_id` — see [`messages_in_channel`](Self::messages_in_channel)) —
    /// the app-facade's thread assembly read (be9y; M1 threads are the set of messages sharing a
    /// `thread_id`). This is what makes an `ask` opener read back before its same-millisecond first
    /// reply. A db error → `io`.
    pub fn messages_in_thread(&self, thread_id: &str) -> crate::error::Result<Vec<MessageRow>> {
        let conn = self.inner.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {} FROM messages m WHERE m.thread_id=?1 \
             ORDER BY m.lamport, m.site, m.message_id",
            message_select()
        ))?;
        let rows = st
            .query_map([thread_id], message_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// [`messages_in_thread`](Self::messages_in_thread), narrowed to the messages an agent action may
    /// follow (nxf 6j6v.pzkb) — what a DECISION reads a thread through: whose request opened it,
    /// what the members answered, what the synthesizer said. The display reads keep every message;
    /// a message this replica cannot vouch for is shown, it just never steers anything.
    pub fn acting_messages_in_thread(
        &self,
        thread_id: &str,
    ) -> crate::error::Result<Vec<MessageRow>> {
        let conn = self.inner.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {} FROM messages m WHERE m.thread_id=?1 AND {} \
             ORDER BY m.lamport, m.site, m.message_id",
            message_select(),
            acts("m")
        ))?;
        let rows = st
            .query_map([thread_id], message_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The derived quorum state of a single thread (spec §4.1–§4.3), or `None` if no such thread.
    /// `now` is the reader's wall-clock, used ONLY for the clock-dependent `stale` flag (§4.3).
    pub fn thread_quorum(
        &self,
        thread_id: &str,
        now: &str,
    ) -> crate::error::Result<Option<ThreadQuorum>> {
        Ok(self.thread_quorums(&[thread_id], now)?.into_iter().next())
    }

    /// The derived quorum state for a SET of threads in ONE query (spec §4.1, "bulk-friendly … no
    /// N+1") — the read the `threads list` board and the app-facade quorum bar are built on. The
    /// join is `threads ⋈ json_each(expects_reply_from)` with a correlated `replied` EXISTS per
    /// expected handle; a `LEFT JOIN` keeps a thread whose `expects_reply_from` is empty/NULL (it
    /// yields one handle-`NULL` row so the thread still appears, with `expects = []`). Results are
    /// ordered `(channel_id, thread_id, handle-index)` — deterministic; handles keep declaration
    /// order. A db error surfaces as `io`. Unknown ids simply do not appear (no error).
    ///
    /// **`replied` is the discharge of a TURN, not of the thread** (nxf 6j6v.cg8g). It used to be a
    /// bare `EXISTS(… m.thread_id = t.thread_id AND m.sender = je.value)` — *any* message from that
    /// sender in this thread, EVER — which is inherited M2 semantics and correct for what M2 had: a
    /// one-shot review board, where "has this reviewer shown up" is the whole question. It is wrong
    /// the moment a thread carries several turns OF THE SAME ROLE, which since 6j6v.hq71's
    /// supervisor flow (and already in 6j6v.a71h §2's chain) is the normal case: from the role's
    /// FIRST answer the thread reads as answered forever, so `reply --if-unanswered` at session
    /// teardown becomes a permanent no-op and the working-tree lease reads the claim area as finished
    /// while the role is still writing its final report.
    ///
    /// **The watermark is the register's own LWW position**, `(expects_reply_from_v,
    /// expects_reply_from_site)`, compared against each message's `(lamport, site)` — the same clock,
    /// the same total order `messages_thread_causal` indexes, and the same row-value comparison
    /// `message_reducer::fold_lww` resolves the register itself with. So NO new column and no fold
    /// path was needed: the declaration already carries the instant it was made, because every
    /// re-declaration by the opener folds as a later op with a higher Lamport. A message written
    /// BEFORE the current declaration therefore does not settle it, which is the whole point —
    /// including the degenerate re-declaration of an unchanged handle set, which is exactly how a
    /// supervisor opens the next turn.
    ///
    /// Two consequences worth naming rather than leaving to be discovered:
    ///
    /// - **Nothing here looks at `messages.kind`**, deliberately (6j6v.cg8g's own acceptance): an
    ///   escalating answer discharges the turn like a finished one, because the session ends either
    ///   way. What BECOMES of an escalation is the channel supervisor's decision, and whether it
    ///   releases the working copy is a separate claim question — neither belongs in this predicate,
    ///   and 6j6v.cg8g's rejected alternatives say why putting the truth about "finished" in a field
    ///   the GUARDED party sets is the wrong shape.
    /// - **What the Lamport comparison does and does not guarantee** (corrected in this item's own
    ///   review — an earlier version of this comment claimed a safety it does not have). Lamport
    ///   order implies causal order in ONE direction only: if the reply saw the declaration, it sorts
    ///   after it, always. The converse does not hold. So for two writes that are genuinely
    ///   CONCURRENT — neither saw the other — `(lamport, site)` still yields a total order, but an
    ///   arbitrary one, and a reply that never saw the declaration can land on either side of it.
    ///   Landing after means discharging an obligation it never received, and that is the UNSAFE
    ///   direction (the working copy freed while the role is still working), not the conservative one.
    ///
    ///   **How wide the window is.** It was once wider than a relay, and wider than one device
    ///   made it look. The Lamport clock used to be per OPEN STORE HANDLE:
    ///   [`nxs_foundation::store::Store::open`] recovered it once from `MAX(lamport)` and then
    ///   advanced it purely in memory, so ops another PROCESS wrote into the same file were never
    ///   observed by a handle already open over it — and `site_id` is per replica, so two
    ///   concurrent handles even shared a site. A long-lived handle's clock drifted arbitrarily far
    ///   below the file it was writing into. This repo ships exactly that shape: a long-lived
    ///   [`crate::engine::Engine`] holding the store for an app's lifetime, beside the short-lived
    ///   `nxc reply` subprocess a triggered session answers from. Engine declared turn 1 at 51, the
    ///   subprocess replied at 501, Engine re-declared turn 2 at 52 — and `501 > 52` discharged
    ///   turn 2 the instant it was declared, on one device, with no relay in sight.
    ///
    ///   **That axis is closed** (nxf 6j6v.fc5p): every local write now re-reads the log's
    ///   high-water mark under the write lock before it mints a coordinate
    ///   ([`nxs_foundation::store::Store::emit`]), so a re-declaration always sorts above every op
    ///   already in the file, whichever process wrote it. What REMAINS is only the genuinely
    ///   cross-replica axis: two DEVICES writing between syncs, where no local read can see the
    ///   other's op because it is not in the file yet.
    ///
    ///   **The watermark itself is correct.** What was wrong was the assumption under which it was
    ///   called safe: this comparison says "written after the declaration" in the substrate's own
    ///   order, and the substrate's order had a hole in it. Every consumer of this predicate
    ///   inherited that hole and none of them could close it locally.
    ///
    ///   **What would close the remainder**: a causal reference on the reply naming the register
    ///   version it answers, or vector clocks in place of the scalar Lamport. Either does move the
    ///   synced payload, which is why neither is in scope for 6j6v.cg8g's fourth acceptance point —
    ///   whereas the same-replica half cost nothing at the wire format at all.
    pub fn thread_quorums(
        &self,
        thread_ids: &[&str],
        now: &str,
    ) -> crate::error::Result<Vec<ThreadQuorum>> {
        if thread_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; thread_ids.len()].join(",");
        let sql = quorum_sql(&format!("WHERE t.thread_id IN ({placeholders})"));
        let conn = self.inner.connection();
        let mut st = conn.prepare(&sql)?;
        let rows = st
            .query_map(rusqlite::params_from_iter(thread_ids.iter()), quorum_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(assemble_quorums(rows, now))
    }

    /// [`thread_quorums`](Self::thread_quorums) over EVERY thread in the workspace, in ONE query —
    /// what `nxc status` derives an operation's state from (nxf 6j6v.a71h §3.3).
    ///
    /// A separate entry point rather than a `thread_ids` list, for a reason that is not cosmetic:
    /// the bulk read binds one SQL variable per id, and SQLite's default `SQLITE_MAX_VARIABLE_NUMBER`
    /// is 999 — a workspace-wide status would start failing at the thousandth thread. Both share
    /// [`quorum_sql`], so there is still exactly ONE copy of the `replied` predicate (the property
    /// nxf 6j6v.cg8g's own acceptance pins); only the filter differs.
    ///
    /// Deliberately NOT membership-scoped, unlike [`member_thread_ids`](Self::member_thread_ids):
    /// the anchor of a status read is the OPERATION, and an operation crosses channels its reader is
    /// not a member of (a71h §3.2 — a chain over three channels seen through one of them is a third
    /// of the truth). A db error → `io`.
    pub fn thread_quorums_all(&self, now: &str) -> crate::error::Result<Vec<ThreadQuorum>> {
        let conn = self.inner.connection();
        let mut st = conn.prepare(&quorum_sql(""))?;
        let rows = st
            .query_map([], quorum_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(assemble_quorums(rows, now))
    }

    /// The whole forest in ONE query (nxf 6j6v.a71h §3.1): every thread's parent edge and the
    /// channel it lives in, `(thread_id, parent, channel_id)`, thread-id sorted.
    ///
    /// This is the SHAPE of the operation graph, and it is deliberately separate from — and read
    /// before — the quorum registers: `nxc status --thread`/`--channel` select which operations they
    /// are about from this alone, and only then pay for the expectations of the threads they
    /// actually return (`facade::status`). A `parent` of `None` is a ROOT: the top end of a chain,
    /// DERIVED from the absence of an edge rather than stored as a flag of its own (§3.1's closing
    /// line).
    ///
    /// Served whole out of the `threads_parent` covering index — all three columns are in it for
    /// exactly that reason (gated by
    /// `schema::tests::the_tree_walk_is_index_backed_not_a_table_scan`), so walking the forest never
    /// pages through a root's opener/expects payload. A db error → `io`.
    #[allow(clippy::type_complexity)]
    pub fn thread_edges(
        &self,
    ) -> crate::error::Result<Vec<(String, Option<String>, Option<String>)>> {
        let conn = self.inner.connection();
        let mut st =
            conn.prepare("SELECT thread_id, parent, channel_id FROM threads ORDER BY thread_id")?;
        let rows = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The internal session of each thread's ASSIGNEE — the return address of the newest message in
    /// that thread from a handle the thread EXPECTS (nxf 6j6v.1q6d), for every thread at once.
    ///
    /// This is the edge `thread → session` that lets a consumer follow the transcript of whoever is
    /// working on a thread **without knowing that session id**, which is the whole of 1q6d: the
    /// transcript is addressed by session (`facade::transcript`), and the opener of a thread had no
    /// way to get from one to the other. Nothing new is stored for it — `refs.session_id` is the
    /// return address every message already carries
    /// ([`message_return_address`](Self::message_return_address)), and the "from an expected handle"
    /// filter is [`latest_reply_message_id`](Self::latest_reply_message_id)'s, in bulk.
    ///
    /// **The assignee's, not the requester's.** The obvious reading — "the newest message carrying a
    /// return address" — would hand back the session of whoever OPENED the thread, since that is
    /// usually the only message in it, and a consumer following that would be watching the requester
    /// think instead of the party it is waiting for.
    ///
    /// **And it answers while the assignee is still SILENT**, which is the whole point rather than a
    /// refinement: an open thread is by definition one nobody has answered yet, and the incident that
    /// produced 1q6d was a human watching a silent conversation for ten minutes while a finished
    /// answer sat uncollected. A triggered session is told about its thread and does not write to it,
    /// so there is no message to read a return address off — but the trigger DID record where it put
    /// that session (`session_map.thread`, nxf 6j6v.a71h §3.1), and that is a read, not a new write.
    /// See [`assignee_sql`] for the two branches and the guard between them.
    ///
    /// `None` only when neither exists: nothing has been put to work on this thread on THIS device
    /// and nobody expected has posted in it.
    ///
    /// Ordered by `thread_id`, one row per thread that has such a session. A db error → `io`.
    pub fn thread_assignee_sessions(&self) -> crate::error::Result<Vec<(String, String)>> {
        self.assignee_sessions(&assignee_sql(""), rusqlite::params_from_iter(EMPTY_IDS))
    }

    /// [`thread_assignee_sessions`](Self::thread_assignee_sessions) restricted to `thread_ids` — the
    /// scoped form `nxc status --thread`/`--channel` use, so asking about one operation does not
    /// re-derive the whole workspace. Both share [`assignee_sql`], so there is exactly one copy of
    /// the projection; only the filter differs. The caller is responsible for keeping `thread_ids`
    /// under SQLite's variable ceiling (`facade::MAX_BOUND_IDS`), exactly as with
    /// [`thread_quorums`](Self::thread_quorums).
    pub fn thread_assignee_sessions_for(
        &self,
        thread_ids: &[&str],
    ) -> crate::error::Result<Vec<(String, String)>> {
        if thread_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; thread_ids.len()].join(",");
        let sql = assignee_sql(&format!("WHERE t.thread_id IN ({placeholders})"));
        self.assignee_sessions(&sql, rusqlite::params_from_iter(thread_ids.iter()))
    }

    /// The threads whose PARENT moved on after they were discharged (nxf 6j6v.93zd) — the
    /// discriminator that tells a dead end that FINISHED from one that FAILED. [`moved_on_sql`]
    /// carries the derivation, the two ways a parent can act on an answer, and the error direction;
    /// this is the workspace-wide form.
    ///
    /// Thread-id ordered. A db error → `io`.
    pub fn parent_moved_after_discharge(&self) -> crate::error::Result<Vec<String>> {
        self.moved_on(&moved_on_sql(""), rusqlite::params_from_iter(EMPTY_IDS))
    }

    /// [`parent_moved_after_discharge`](Self::parent_moved_after_discharge) restricted to
    /// `thread_ids` — the scoped form `nxc status --thread`/`--channel` use, so asking about one
    /// operation does not re-derive the whole workspace. Both share [`moved_on_sql`], so the
    /// predicate exists exactly once; only the filter differs. The caller keeps `thread_ids` under
    /// SQLite's variable ceiling (`facade::MAX_BOUND_IDS`), exactly as with
    /// [`thread_quorums`](Self::thread_quorums).
    pub fn parent_moved_after_discharge_for(
        &self,
        thread_ids: &[&str],
    ) -> crate::error::Result<Vec<String>> {
        if thread_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; thread_ids.len()].join(",");
        let sql = moved_on_sql(&format!("AND t.thread_id IN ({placeholders})"));
        self.moved_on(&sql, rusqlite::params_from_iter(thread_ids.iter()))
    }

    /// [`moved_on_sql`]'s one execution path, shared by both forms above.
    fn moved_on<P: rusqlite::Params>(
        &self,
        sql: &str,
        params: P,
    ) -> crate::error::Result<Vec<String>> {
        let conn = self.inner.connection();
        let mut st = conn.prepare(sql)?;
        let rows = st
            .query_map(params, |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// One thread's parent edge (nxf 6j6v.a71h §3.1), by id — `None` for a ROOT **and** for a thread
    /// this workspace has never folded, which are indistinguishable here on purpose (a child can fold
    /// before its parent, so "no row" and "no edge" mean the same thing to a reader: this thread is
    /// the top of what we can see).
    ///
    /// The whole-forest read [`thread_edges`](Self::thread_edges) is what `nxc status` walks; this is
    /// the single-thread question the channel supervisor asks on a hot path (nxf 6j6v.pf6j — "is the
    /// thread this reply landed in a MEMBER thread, and of which channel thread?"), where paging the
    /// whole workspace in to answer it for one id would be the wrong shape. Served out of the
    /// `threads_tree` covering index. A db error → `io`.
    pub fn thread_parent(&self, thread_id: &str) -> crate::error::Result<Option<String>> {
        let conn = self.inner.connection();
        let mut st = conn.prepare("SELECT parent FROM threads WHERE thread_id = ?1")?;
        let mut rows = st.query([thread_id])?;
        match rows.next()? {
            Some(row) => Ok(row.get(0)?),
            None => Ok(None),
        }
    }

    /// The ROOT of the operation `thread_id` belongs to — the top of its chain of parent edges, and
    /// the id `nxc status` groups the whole tree under (nxf 6j6v.v39s).
    ///
    /// **THIS IS WHERE THE TERM "OPERATION" IS DEFINED, and it is defined in one place on purpose**
    /// (nxf 6j6v.8y6t, owner's note of 2026-08-23). An OPERATION is the thread tree hanging off a
    /// root; its IDENTITY is what this function returns, and its EXTENT is
    /// [`thread_subtree`](Self::thread_subtree) of that root. Nothing else is needed to say what one
    /// is, and nothing should say it a second way.
    ///
    /// Three consumers already read it that way and a fourth is coming, which is why it is written
    /// down rather than left to each of them:
    ///
    /// * `nxc status` groups by it — a chain over three channels seen through one of them is a third
    ///   of the truth (nxf 6j6v.a71h §3.2).
    /// * [`facade::opened_the_operation`](crate::facade) reads it as a permission: whoever opened the
    ///   operation may read any thread in it, whatever channel that thread sits in.
    /// * the WORKING-TREE CLAIM is scoped to it since 6j6v.8y6t — the checkout is held for an
    ///   operation and given back when the operation is finished, not when the branch its exclusive
    ///   step started in is.
    /// * nxf 6j6v.nby8 will freeze a declaration for the length of one, and is asked to take the
    ///   boundary FROM HERE rather than invent a second one.
    ///
    /// [`thread_parent`](Self::thread_parent)'s upward twin, walked with point reads rather than
    /// through the whole-forest read [`facade::status`](crate::facade::status) pays for: this is the
    /// single-thread question a READ GATE asks, once, about the thread in front of it, and paging
    /// every parent edge in the workspace to answer it would be the wrong shape for the same reason
    /// `thread_parent` itself exists.
    ///
    /// **It resolves to the same root `status` does**, including the cycle rule: a walk that
    /// revisits a thread takes the SMALLEST id of the cycle itself, so every node on or below the
    /// cycle names one root (`facade`'s `root_of_thread` carries that argument in full). A cycle
    /// cannot arise from any write here, but a synced or hand-edited log could carry one and a walk
    /// that trusts the data would hang.
    ///
    /// **A parent this workspace has never folded ENDS the walk at that parent**, which is a thread
    /// with no `threads` row and therefore no [`thread_opener`](Self::thread_opener) — the
    /// fail-closed direction for the gate above, and the one that matters: ops arrive in relay
    /// order, so an ancestry that is only half here must not be read as "this reader owns the tree".
    ///
    /// `root` is `thread_id` itself when it has no parent edge at all. A db error → `io`.
    pub fn thread_root(&self, thread_id: &str) -> crate::error::Result<String> {
        let mut path: Vec<String> = Vec::new();
        let mut cur = thread_id.to_string();
        loop {
            if let Some(at) = path.iter().position(|p| *p == cur) {
                return Ok(path[at..].iter().min().cloned().unwrap_or(cur));
            }
            match self.thread_parent(&cur)? {
                Some(parent) => {
                    path.push(cur);
                    cur = parent;
                }
                None => return Ok(cur),
            }
        }
    }

    /// The thread's OPENING message — its id and its sender — in the same causal
    /// `(lamport, site, message_id)` order [`messages_in_thread`](Self::messages_in_thread) reads,
    /// so "first" here means byte for byte what "first" means to the reader that filters a board.
    /// `None` for a thread with no message at all.
    ///
    /// It exists for the declared-`visibility` filter on the SEARCH path (nxf 6j6v.px98).
    /// `facade::filter_board_messages` gets both facts for free — it holds the whole ordered thread
    /// — but a search hit arrives alone, and judging it needs exactly these two: whether it IS the
    /// opening message (which stands for the REQUEST and is never filtered), and who opened the
    /// thread (which is where `facade::requester_for` starts). One point read on the causal index
    /// rather than assembling the thread the hit happens to sit in. A db error → `io`.
    pub fn thread_opening_message(
        &self,
        thread_id: &str,
    ) -> crate::error::Result<Option<(String, String)>> {
        Ok(self
            .inner
            .connection()
            .query_row(
                "SELECT message_id, sender FROM messages WHERE thread_id = ?1 \
                 ORDER BY lamport, site, message_id LIMIT 1",
                [thread_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// One thread's OPENER, read straight off the `threads` row — the same column
    /// [`ThreadQuorum::opener`] carries, without the quorum assembly around it.
    ///
    /// **It exists because the question is CLOCK-FREE and the bulk read is not** (nxf 6j6v.yr59).
    /// `facade::requester_of` needs a parent thread's opener to resolve who a `requester_only`
    /// board is reserved FOR, and it used to get it from [`thread_quorums`](Self::thread_quorums) —
    /// which derives `stale` and therefore takes a `now` that this question has no use for. That
    /// left the call site apologising for handing a clock-taking read a value that was not a time,
    /// and it put the visibility filter out of reach of a reader with no clock at all
    /// ([`facade::thread`](crate::facade::thread), the one message reader). One point read on the
    /// primary key answers it instead.
    ///
    /// `None` for a thread with no `threads` row — a message-only thread — exactly as
    /// [`thread_parent`](Self::thread_parent) beside it. A db error → `io`.
    pub fn thread_opener(&self, thread_id: &str) -> crate::error::Result<Option<String>> {
        let conn = self.inner.connection();
        let mut st = conn.prepare("SELECT opener FROM threads WHERE thread_id = ?1")?;
        let mut rows = st.query([thread_id])?;
        match rows.next()? {
            Some(row) => Ok(row.get(0)?),
            None => Ok(None),
        }
    }

    /// The SUBTREE under `root`, `root` itself included — every thread that hangs beneath it through
    /// any chain of parent edges, however deep and whatever channel it lives in (nxf 6j6v.1xw1).
    ///
    /// This is the shape of the working-tree lease's claim area: `#coding`'s channel thread with the
    /// member threads below it, the `#review` round one of those members opened, that round's own
    /// members, and the ad-hoc DM a session opened out of its own conversation. All of them are
    /// somebody's work on the same task, and none of them is reachable from a set of ids the way one
    /// level of children is. It walks DOWNWARD only, which is what keeps the conversation ABOVE the
    /// root — a71h §2's T1, the human's own thread — out of it.
    ///
    /// **Termination is by construction, not by a depth limit.** The walk is a BFS over an in-memory
    /// child index with a `seen` set, so each thread is expanded at most ONCE: a cycle in the parent
    /// edges (which `facade::status` already has to survive — the edge is a claim on a synced op, not
    /// a validated foreign key) revisits a node that is already in `seen` and is dropped there, and a
    /// thread naming itself as its own parent is the same case.
    ///
    /// **One query for the whole forest, not one per level.** `threads.parent` has no index of its
    /// own — the covering index is `threads_tree(thread_id, parent, channel_id)` — so a
    /// level-by-level `WHERE parent IN (…)` would scan the table once per level, while
    /// [`thread_edges`](Self::thread_edges) scans it once, out of that index, and is the same read
    /// `nxc status` already pays on every invocation.
    ///
    /// `root` comes back even when this workspace has no such thread: "the area is at least the
    /// thread it is named after" is the answer the lease needs, and it is what the release path's own
    /// in-scope gate is written against. A db error → `io`.
    pub fn thread_subtree(&self, root: &str) -> crate::error::Result<Vec<String>> {
        self.thread_subtrees(&[root])
    }

    /// [`thread_subtree`](Self::thread_subtree) from several roots at once, over ONE forest read —
    /// what a `run:` claim area needs, since it is anchored on a SET of channel threads rather than
    /// on one. The union, each thread once, roots first in the order given and then breadth-first.
    pub fn thread_subtrees(&self, roots: &[&str]) -> crate::error::Result<Vec<String>> {
        if roots.is_empty() {
            // Before the forest read, not after it (branch review of nxf 6j6v.1xw1, Minor 8): a
            // `run:` area with no boards of its own — a run whose steps have all been role-target
            // ones, or one this device has never folded — would otherwise pay a full scan of
            // `threads` to be told what its empty root set already says.
            return Ok(Vec::new());
        }
        let mut children: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (thread_id, parent, _) in self.thread_edges()? {
            if let Some(parent) = parent {
                children.entry(parent).or_default().push(thread_id);
            }
        }
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut out: Vec<String> = Vec::new();
        let mut queue: std::collections::VecDeque<String> =
            roots.iter().map(|r| r.to_string()).collect();
        while let Some(thread_id) = queue.pop_front() {
            if !seen.insert(thread_id.clone()) {
                // Already expanded — a cycle, a diamond, or a root that is another root's
                // descendant. This is the whole of the termination argument.
                continue;
            }
            if let Some(kids) = children.get(&thread_id) {
                queue.extend(kids.iter().cloned());
            }
            out.push(thread_id);
        }
        Ok(out)
    }

    /// The MEMBER threads directly under `parent` — the children a channel SUPERVISOR opened (nxf
    /// 6j6v.pf6j), as opposed to whatever else happens to hang there.
    ///
    /// **The marker is the `opener`, and it is sound because the handle is reserved.** A member
    /// thread's opener is `<origin>/__channel__`, and no declared role can claim that identity
    /// ([`crate::channel::SUPERVISOR_HANDLE`] says why), so "the supervisor opened this" is decidable
    /// from the record with no second bookkeeping table. Matched on the BARE tail rather than on a
    /// qualified string: the store does not know which origin is minting here, and the reserved
    /// handle is unambiguous without it. A thread a member itself opened out of its own conversation
    /// is somebody else's work in the same tree and is correctly not in this set.
    ///
    /// Thread-id ordered, served out of the `threads_tree` index. A db error → `io`.
    pub fn supervised_children(&self, parent: &str) -> crate::error::Result<Vec<String>> {
        let suffix = format!("/{}", crate::channel::SUPERVISOR_HANDLE);
        let conn = self.inner.connection();
        let mut st = conn.prepare(
            "SELECT thread_id FROM threads
              WHERE parent = ?1 AND substr(opener, -length(?2)) = ?2
              ORDER BY thread_id",
        )?;
        let rows = st
            .query_map(rusqlite::params![parent, suffix], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// **Every thread directly under `parent`** — [`supervised_children`](Self::supervised_children)
    /// without the supervisor filter (nxf 6j6v.hw2t).
    ///
    /// What reads it is [`crate::awaiting::own_open_sub_round`], at the `reply` write point, and the
    /// question it answers there is whether the caller is waiting on a round it commissioned itself.
    /// That predicate weighs the `opener` of each child, so the filtering the neighbour does in SQL
    /// has to happen after the quorum read here — the child's OPENNESS is not a column.
    ///
    /// **MEASURED rather than assumed, because the item asked** (its design's "zu pruefen statt
    /// anzunehmen"): SQLite plans this as `SCAN threads USING COVERING INDEX threads_tree` — one
    /// pass over the narrow `(thread_id, parent, channel_id)` index, no table access and no walk up
    /// or down the tree. It is the same shape and the same cost as the neighbour above, which
    /// already runs on the `tick` path. It is nevertheless placed BEHIND the cheap pre-checks at
    /// its caller (the reply is a plain one, and the caller is named in the thread's `outstanding`)
    /// — a scan is cheap, and not running it at all is cheaper.
    ///
    /// Thread-id ordered, i.e. chronological. A db error → `io`.
    pub fn thread_children(&self, parent: &str) -> crate::error::Result<Vec<String>> {
        let conn = self.inner.connection();
        let mut st =
            conn.prepare("SELECT thread_id FROM threads WHERE parent = ?1 ORDER BY thread_id")?;
        let rows = st
            .query_map([parent], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The messages written in `thread_id` SINCE the obligation it currently declares — the turn
    /// watermark of nxf 6j6v.cg8g applied to a whole THREAD rather than to one handle.
    ///
    /// [`messages_in_thread`](Self::messages_in_thread)'s turn-scoped twin, and it exists because
    /// reading a thread's whole life where the surrounding machinery is scoped to a turn has now been
    /// the same defect three times in this work order (`replied`, the assignee guard, and the channel
    /// consolidation this was added for). Same projection, same order, one extra clause — spliced from
    /// [`after_the_declaration`], the single definition of that comparison.
    ///
    /// EMPTY for a thread whose expectation was never declared: the watermark is then NULL and no
    /// message can be after it. That is the correct answer for the question this asks ("what has been
    /// said in answer to the current obligation") and the reason a caller wanting a thread's whole
    /// history must keep asking `messages_in_thread`. A db error → `io`.
    ///
    /// # It answers "since the declaration", NOT "this turn" — and the two come apart
    ///
    /// The watermark is the `set_expects` op, so what this returns depends on **where that op sits
    /// relative to the messages you care about**, and the engine writes it in both orders:
    ///
    /// - on a MEMBER thread the supervisor re-declares BEFORE it posts the forwarded task, so the
    ///   task and the member's answer are both after the watermark and this function is exactly "the
    ///   current turn";
    /// - on a CHANNEL thread the requester's message is durable BEFORE the supervisor re-declares
    ///   (`facade::reply` writes, then step (4b) runs), so the message that OPENED the turn sorts
    ///   *before* the watermark and this function would drop it.
    ///
    /// `orchestration::collected_replies` therefore uses this for the member threads and a different
    /// rule — the newest message from a party that is not the engine — for the channel thread. That
    /// is not a shortcut; it is forced by the two write orders. A future caller reaching for this on
    /// a thread whose turn is opened by an inbound message wants the same pair, not this alone.
    pub fn messages_since_the_declaration(
        &self,
        thread_id: &str,
    ) -> crate::error::Result<Vec<MessageRow>> {
        let since = answers("m");
        let select = message_select();
        let conn = self.inner.connection();
        let mut st = conn.prepare(&format!(
            "SELECT {select} FROM messages m
               JOIN threads t ON t.thread_id = m.thread_id
              WHERE m.thread_id = ?1 AND {since}
              ORDER BY m.lamport, m.site, m.message_id"
        ))?;
        let rows = st
            .query_map([thread_id], message_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// How many messages `sender` has written in `thread_id` SINCE the obligation the thread
    /// currently declares — the turn watermark of nxf 6j6v.cg8g, asked about one handle.
    ///
    /// **The fourth reader of [`after_the_declaration`], not a fourth copy of it.** The question
    /// "did THIS post flip the board from incomplete to complete" used to be answered in Rust as "is
    /// it the sender's first message in the thread" (`orchestration::reply`'s `is_completing_reply`),
    /// which is the pre-cg8g reading of a discharge: from turn two on, a handle that answered turn
    /// one is never the completing reply again, so a multi-turn board's second completion routes
    /// nowhere. Asking it here — with the same row-value comparison `replied`/`outstanding` and the
    /// assignee edge are computed from — makes "who completed it" and "who owes it" one rule.
    ///
    /// `0` for an unknown thread, and for a thread whose expectation was never declared (the
    /// watermark is then NULL and no message can be after it), which is the correct answer in both
    /// cases: nothing has been written against an obligation that does not exist. A db error → `io`.
    pub fn replies_since_the_declaration(
        &self,
        thread_id: &str,
        sender: &str,
    ) -> crate::error::Result<i64> {
        let since = answers("m");
        let sql = format!(
            "SELECT COUNT(*) FROM threads t
               JOIN messages m ON m.thread_id = t.thread_id
              WHERE t.thread_id = ?1 AND m.sender = ?2 AND {since}"
        );
        let conn = self.inner.connection();
        Ok(conn.query_row(&sql, rusqlite::params![thread_id, sender], |r| r.get(0))?)
    }

    /// **Was the last reply on this thread an ESCALATION?** — the readable derivation of nxf
    /// 6j6v.wt37, and the one 6j6v.1xw1 (the working-tree lease's claim scope) is built to call.
    ///
    /// "The last reply" is the NEWEST message written SINCE the thread's current declaration by a
    /// handle the thread expects — §2.2's definition of a reply (a message from an expected handle;
    /// the opener's own request and a bystander's drive-by never count), narrowed to this TURN.
    ///
    /// - `Some(true)` — that answer said "I cannot do this".
    /// - `Some(false)` — that answer was a result, or a follow-up QUESTION, which is a different
    ///   thing and stays one ([`MessageKind::Question`](crate::model::MessageKind::Question)).
    /// - `None` — nothing has answered this turn: an unknown thread, a thread whose expectation was
    ///   never declared, or one that has simply not been answered yet. **`None` is not `Some(false)`**
    ///   and a caller must not collapse the two: "nobody has spoken" and "somebody said it is
    ///   finished" are the two states a release rule has to tell apart.
    ///
    /// **This predicate is about WHAT WAS SAID, so it is TURN-SCOPED** — the distinction this work
    /// order learned three times the hard way (a predicate about what a thread IS — who opened it,
    /// what it hangs under — must not be). It therefore splices [`after_the_declaration`], the single
    /// definition of that comparison — the FIFTH reader of it, not a fifth copy: turn one's "I am
    /// finished" must not still be the answer to turn two, and turn one's escalation must not hold a
    /// working copy that turn two has already released.
    ///
    /// **What this does NOT decide**: whether the claim is released. That rule — the subtree from the
    /// claim thread, and the exception that a hand-back holds it — is 6j6v.1xw1's, built there and
    /// not here: [`ChatStore::work_scope_threads`] and [`ChatStore::work_scope_handed_back`]. This
    /// answers one question and answers it about one thread. Note that the release rule does NOT call
    /// this one, and the reason is worth knowing: an open QUESTION holds the claim exactly as an
    /// escalation does — nobody is working either way — and this predicate correctly answers
    /// `Some(false)` for it, because the two are different FACTS. The lease reads
    /// [`ChatStore::last_reply_kind`], the same one derivation, one step wider.
    ///
    /// # ON A CHANNEL THREAD it now answers the same way whatever the channel declares
    ///
    /// **It answers about ONE thread and nothing below it.** On a supervised channel (nxf 6j6v.pf6j)
    /// the members answer on their own threads, one level down, so what this reports for the CHANNEL
    /// thread is whatever the CONSOLIDATOR's own discharge of it carried.
    ///
    /// Until nxf 6j6v.e9qj that depended on the declared `on_complete`, and the asymmetry was a real
    /// hole: `pass_through`'s discharge is the engine's own `__delivered__` marker, which the engine
    /// can mark, while a `summarize` channel's is the SYNTHESIZER's reply, whose kind the engine does
    /// not author — so a member's escalation died one level down there. **That is closed**: the
    /// declared consolidator never folds an escalating set (`ChannelDecl::consolidation`), so such a
    /// set always passes through and the answer of record is always the engine's own marker. A
    /// member's "I cannot" therefore reaches the channel thread on BOTH output forms, and a caller
    /// may read this on a claim thread without first asking what that channel declares.
    ///
    /// Pinned end to end by
    /// `tests/reply_escalate.rs::a_summarize_channel_hands_a_members_escalation_up_instead_of_folding_it_away`,
    /// which also asserts that no synthesizer ran — the escalation changed which output form ran, it
    /// was not marked on afterwards.
    ///
    /// **What still holds**: this reads ONE thread. A caller that has to see what a channel's MEMBERS
    /// said walks them with the `pub` [`ChatStore::supervised_children`] — which is what
    /// `orchestration::set_escalated` does for the supervisor's own branch. The working-tree claim is
    /// NOT that walk: [`ChatStore::work_scope_handed_back`] asks the whole claim area
    /// ([`ChatStore::work_scope_threads`]) and asks the wider question, because a hand-back can sit
    /// any number of levels down and the level above it can consolidate past one (its own doc carries
    /// the counter-shape). And a SYNTHESIZER's own `--escalate` is still reported here, which is a
    /// different fact ("the consolidation could not be produced") and correctly its own.
    ///
    /// "Newest" is the `(lamport, site, message_id)` causal order every other thread read uses, not
    /// the raw `message_id`. A db error → `io`.
    pub fn last_reply_escalated(&self, thread_id: &str) -> crate::error::Result<Option<bool>> {
        Ok(self
            .last_reply_kind(thread_id)?
            .map(|k| k == crate::model::KIND_ESCALATION))
    }

    /// **Did this thread's current turn come back with a VERDICT?** (nxf 6j6v.553s (a)) —
    /// [`last_reply_escalated`](Self::last_reply_escalated)'s twin on the other axis, off the same
    /// one derivation ([`last_reply_kind`](Self::last_reply_kind)) so the two comparisons cannot
    /// drift into disagreeing about which message "the last reply" is.
    ///
    /// `None` is "nothing has answered this turn", exactly as there — deliberately not `Some(false)`,
    /// because a step whose window lapsed with nothing in it must not read as "this was approved".
    ///
    /// **Read by the flow supervisor and by nothing that decides a lease.** A verdict does not hand
    /// the task back (`working_tree::hands_the_task_back` has the argument), so no working-copy rule
    /// asks this.
    ///
    /// A db error → `io`.
    pub fn last_reply_needs_rework(&self, thread_id: &str) -> crate::error::Result<Option<bool>> {
        Ok(self
            .last_reply_kind(thread_id)?
            .map(|k| k == crate::model::KIND_NEEDS_REWORK))
    }

    /// **Which RUN and which STEP of a declared flow this thread was opened for** (nxf 6j6v.553s
    /// (d)) — read back off the commission the supervisor wrote into it
    /// ([`crate::model::Refs::flow_run`]/[`crate::model::Refs::flow_step`]).
    ///
    /// `None` for every thread that is not a step of a stepped channel — a member of a `parallel`
    /// fan-out, a step of a `flow: sequential` channel (whose pass is DERIVED, and stays derived),
    /// an ordinary 1:1 thread. Those readers are untouched by this and must stay so.
    ///
    /// **The OLDEST message carrying the mark, not the newest.** A slot's mark is stamped once, by
    /// the commission that opened it, and is a property of the thread rather than of a turn: reading
    /// the newest would let a later message on the same thread — a resume, a wake — change which
    /// step the engine believes a slot serves. `(lamport, site, message_id)` is the causal order
    /// every other thread read in this file uses.
    ///
    /// A db error → `io`.
    pub fn flow_mark(&self, thread_id: &str) -> crate::error::Result<Option<FlowMark>> {
        let conn = self.inner.connection();
        // Only a commission an action may follow (nxf 6j6v.pzkb): the mark decides which step a
        // slot serves, and a planted message can pick its own Lamport number — "oldest" included.
        let mut stmt = conn.prepare(&format!(
            "SELECT json_extract(m.refs, '$.flow_run'), json_extract(m.refs, '$.flow_step')
               FROM messages m
              WHERE m.thread_id = ?1 AND {vouched}
                AND json_extract(m.refs, '$.flow_step') IS NOT NULL
              ORDER BY m.lamport, m.site, m.message_id
              LIMIT 1",
            vouched = acts("m"),
        ))?;
        let mut rows = stmt.query(rusqlite::params![thread_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(FlowMark {
                run: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                step: row.get(1)?,
            })),
            None => Ok(None),
        }
    }

    /// [`last_reply_kind`](Self::last_reply_kind) for a whole SET of threads, in ONE statement — what
    /// `nxc status` needs to say `escalated` per thread without an N+1 (nxf 6j6v.mqad).
    ///
    /// Same derivation, same words: for each thread, the KIND of the newest message written since
    /// that thread's current declaration by a handle the thread expects. A thread that nothing has
    /// answered this turn is simply absent from the result, which is the map form of that function's
    /// `None`.
    ///
    /// One prepared statement regardless of how many ids are bound — the ordering does the work and
    /// the last row per thread wins, rather than a correlated `LIMIT 1` per id. `tests/bulk_quorum.rs`
    /// is what holds that property for the read this feeds. A db error → `io`.
    pub fn last_reply_kinds(&self, thread_ids: &[&str]) -> crate::error::Result<Vec<LastReply>> {
        if thread_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; thread_ids.len()].join(",");
        self.last_reply_kinds_sql(
            &format!("AND m.thread_id IN ({placeholders})"),
            rusqlite::params_from_iter(thread_ids.iter()),
        )
    }

    /// [`last_reply_kinds`](Self::last_reply_kinds) for the WHOLE workspace — the fallback for a
    /// selection too large to bind, exactly as [`thread_quorums_all`](Self::thread_quorums_all) is.
    pub fn last_reply_kinds_all(&self) -> crate::error::Result<Vec<LastReply>> {
        self.last_reply_kinds_sql("", [])
    }

    /// The ONE execution path behind both forms above, so the two can never answer differently.
    /// `filter` is never caller data: both call sites pass a literal, one of them with a
    /// `?`-placeholder list generated from a slice LENGTH.
    fn last_reply_kinds_sql<P: rusqlite::Params>(
        &self,
        filter: &str,
        params: P,
    ) -> crate::error::Result<Vec<LastReply>> {
        let since = answers("m");
        let conn = self.inner.connection();
        // **The substitution marker rides the SAME row** (nxf 6j6v.kffm), read out of the `refs`
        // JSON text column with `json_extract`. A second query for it would be a second derivation
        // of "the last reply" — the drift this function's own siblings warn about — and it would
        // answer for a different moment the instant the two ran either side of a write.
        //
        // `json_extract` is NULL both for a message written before the key existed and for one whose
        // `refs` is not an object at all, and `IS 1` is false for every NULL — so "not a
        // substitution" is what every old row says, with no migration and no refold.
        let mut st = conn.prepare(&format!(
            "SELECT m.thread_id, m.kind, json_extract(m.refs, '$.substituted') IS 1 FROM messages m
               JOIN threads t ON t.thread_id = m.thread_id
               JOIN json_each(t.expects_reply_from) je ON je.value = m.sender
              WHERE {since} {filter}
              ORDER BY m.thread_id, m.lamport, m.site, m.message_id"
        ))?;
        let rows = st
            .query_map(params, |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // Ordered by (thread, lamport, site, id), so the LAST row of each thread's run is its newest
        // reply — the same "newest wins" the single-thread read gets from `ORDER BY … DESC LIMIT 1`.
        let mut out: Vec<LastReply> = Vec::new();
        for (thread_id, kind, substituted) in rows {
            let last = LastReply {
                thread_id,
                kind,
                substituted,
            };
            match out.last_mut() {
                Some(prev) if prev.thread_id == last.thread_id => *prev = last,
                _ => out.push(last),
            }
        }
        Ok(out)
    }

    /// **Where the working copy stood at each of these threads' most recent handover** (nxf
    /// 6j6v.2af2) — [`last_reply_kinds`](Self::last_reply_kinds)' shape for the anchor, and what
    /// `nxc status` needs to say it per thread without an N+1.
    ///
    /// "Most recent handover" is the NEWEST message in the thread that carries an anchor, in the
    /// `(lamport, site, message_id)` causal order every other thread read in this file uses. A thread
    /// with none is simply absent from the result.
    ///
    /// **Deliberately NOT restricted to replies from an expected handle**, which is the one way this
    /// differs from its sibling next door. That restriction is what makes "the last reply of the
    /// current turn" the right answer for a VERDICT; here it would throw away the anchor that matters
    /// most — the commission that OPENED the thread, written by the requester, which is where the
    /// tree stood when the session that has not answered yet was started. That is precisely the
    /// message a resume needs (nxf 6j6v.npy3).
    ///
    /// One prepared statement regardless of how many ids are bound; the ordering does the work and
    /// the last row per thread wins. A db error → `io`.
    pub fn working_copy_anchors(
        &self,
        thread_ids: &[&str],
    ) -> crate::error::Result<Vec<ThreadAnchor>> {
        if thread_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; thread_ids.len()].join(",");
        self.working_copy_anchors_sql(
            &format!("AND m.thread_id IN ({placeholders})"),
            rusqlite::params_from_iter(thread_ids.iter()),
        )
    }

    /// [`working_copy_anchors`](Self::working_copy_anchors) for the WHOLE workspace — the fallback
    /// for a selection too large to bind, exactly as its three siblings have.
    pub fn working_copy_anchors_all(&self) -> crate::error::Result<Vec<ThreadAnchor>> {
        self.working_copy_anchors_sql("", [])
    }

    /// The ONE execution path behind both forms above, so the two can never answer differently.
    /// `filter` is never caller data: both call sites pass a literal, one of them with a
    /// `?`-placeholder list generated from a slice LENGTH.
    fn working_copy_anchors_sql<P: rusqlite::Params>(
        &self,
        filter: &str,
        params: P,
    ) -> crate::error::Result<Vec<ThreadAnchor>> {
        let conn = self.inner.connection();
        // `json_extract` of an OBJECT hands back its JSON text, which is what the anchor is
        // (`refs.working_copy` is nested — see [`crate::model::Refs::working_copy`] for why). NULL
        // both for a message written before the key existed and for one whose `refs` is not an
        // object at all, so `IS NOT NULL` is exactly "this handover recorded where the tree stood",
        // with no migration and no refold.
        let mut st = conn.prepare(&format!(
            "SELECT m.thread_id, json_extract(m.refs, '$.working_copy') FROM messages m
              WHERE m.thread_id IS NOT NULL
                AND json_extract(m.refs, '$.working_copy') IS NOT NULL {filter}
              ORDER BY m.thread_id, m.lamport, m.site, m.message_id"
        ))?;
        let rows = st
            .query_map(params, |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out: Vec<ThreadAnchor> = Vec::new();
        for (thread_id, json) in rows {
            // **An anchor this build cannot parse is SKIPPED, not an error** — the substrate's own
            // store-don't-fold treatment (spec §3.1) applied one layer up. A newer peer may write a
            // shape this version does not know, and a read that failed the whole report over one
            // unfamiliar row would make `nxc status` unusable on a workspace that syncs with a newer
            // device. The thread then reads as "no anchor", which is what it is FOR THIS READER.
            let Ok(anchor) = serde_json::from_str::<crate::anchor::Anchor>(&json) else {
                continue;
            };
            let row = ThreadAnchor { thread_id, anchor };
            match out.last_mut() {
                Some(prev) if prev.thread_id == row.thread_id => *prev = row,
                _ => out.push(row),
            }
        }
        Ok(out)
    }

    /// **What KIND was the last reply on this thread?** — the same one derivation
    /// [`last_reply_escalated`](Self::last_reply_escalated) answers its narrower question from, and
    /// the reason there is still exactly ONE query behind both (nxf 6j6v.1xw1).
    ///
    /// "The last reply" means precisely what it means there: the newest message written SINCE the
    /// thread's current declaration by a handle the thread expects. `None` for "nothing has answered
    /// this turn", which is not the same as "somebody said it is finished".
    ///
    /// **Why the wider read exists.** `--escalate` is not the only thing an agent can say that leaves
    /// the task mid-flight: a `MessageKind::Question` discharges the turn just as an escalation does
    /// (6j6v.cg8g), and `last_reply_escalated` correctly answers `Some(false)` for it — the two are
    /// deliberately different FACTS (6j6v.wt37: "what do you mean by X?" is not "I cannot"). For the
    /// working-tree claim the difference is no difference, because in both shapes nobody is working,
    /// so 6j6v.1xw1 needs the value rather than the one comparison. Deriving that a second time is
    /// the drift this crate has been bitten by repeatedly; this is the one derivation, read twice.
    ///
    /// The raw column LABEL rather than a parsed [`MessageKind`](crate::model::MessageKind), because
    /// that is what the column holds and what [`crate::model::KIND_ESCALATION`] is already compared
    /// against. A caller that wants the variant parses it and decides for itself what an
    /// unrecognised one means — see `working_tree::hands_the_task_back`, which treats it
    /// conservatively.
    ///
    /// A db error → `io`.
    pub fn last_reply_kind(&self, thread_id: &str) -> crate::error::Result<Option<String>> {
        let since = answers("m");
        Ok(self
            .inner
            .connection()
            .query_row(
                &format!(
                    "SELECT m.kind FROM messages m
                       JOIN threads t ON t.thread_id = m.thread_id
                       JOIN json_each(t.expects_reply_from) je ON je.value = m.sender
                      WHERE m.thread_id = ?1 AND {since}
                      ORDER BY m.lamport DESC, m.site DESC, m.message_id DESC
                      LIMIT 1"
                ),
                [thread_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The shared body of the two reads above.
    fn assignee_sessions<P: rusqlite::Params>(
        &self,
        sql: &str,
        params: P,
    ) -> crate::error::Result<Vec<(String, String)>> {
        let conn = self.inner.connection();
        let mut st = conn.prepare(sql)?;
        let rows = st
            .query_map(params, |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(t, s)| s.map(|s| (t, s)))
            .collect())
    }

    /// The thread ids of every first-class thread (in the `threads` view) whose `channel_id`
    /// `consumer` is a resolved member of, optionally restricted to one `channel` — the enumeration
    /// the `threads list` board feeds to [`thread_quorums`](Self::thread_quorums) (ONE query, then
    /// ONE bulk quorum call; no N+1). Ordered `(channel_id, thread_id)` — the SAME deterministic
    /// order `thread_quorums` re-imposes, so the board list is stable. A db error → `io`.
    pub fn member_thread_ids(
        &self,
        consumer: &str,
        channel: Option<&str>,
    ) -> crate::error::Result<Vec<String>> {
        let conn = self.inner.connection();
        let mut sql = String::from(
            "SELECT t.thread_id FROM threads t
              WHERE t.channel_id IN (SELECT DISTINCT channel_id FROM membership_adds
                       WHERE handle=?1 AND tag NOT IN (SELECT tag FROM membership_removes))",
        );
        let mut binds: Vec<&str> = vec![consumer];
        if let Some(ch) = channel {
            sql.push_str(" AND t.channel_id=?2");
            binds.push(ch);
        }
        sql.push_str(" ORDER BY t.channel_id, t.thread_id");
        let mut st = conn.prepare(&sql)?;
        let rows = st
            .query_map(rusqlite::params_from_iter(binds.iter()), |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Thread ids the `opener` opened (the `threads.opener` column) — the opener-wake candidate set
    /// (spec §4.4). Order is irrelevant here; [`opener_wake`](Self::opener_wake) re-sorts via the
    /// bulk quorum read (channel + thread ULID). A db error surfaces as `io`.
    pub fn threads_opened_by(&self, opener: &str) -> crate::error::Result<Vec<String>> {
        let conn = self.inner.connection();
        let mut st = conn.prepare("SELECT thread_id FROM threads WHERE opener=?1")?;
        let rows = st
            .query_map([opener], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The latest reply message in a thread: the NEWEST message whose `sender` is one of the thread's
    /// expected handles (§2.2 — a "reply" is any message from an expected handle; the opener's own
    /// request and drive-by posts from non-expected handles never count). `None` when no expected
    /// handle has posted. This is BOTH the completing-reply annotation target and the value the
    /// message whose own instant decides whether a finished board falls inside a session's window
    /// (nxf 6j6v.2hx9; it was also the cursor watermark until then). A db error
    /// → `io`.
    ///
    /// "Newest" is the `(lamport, site, message_id)` causal order every other thread read uses —
    /// [`messages_in_thread`](Self::messages_in_thread), [`last_reply_kind`](Self::last_reply_kind)
    /// and [`assignee_sql`]'s branch (2) all read it that way. **This site used to be the exception**
    /// (nxf 6j6v.fcv4): it took `MAX(message_id)`, and two replies minted in the same millisecond
    /// differ only in the ULID's random tail, so it picked between them by coin toss. 0b1m fixed
    /// exactly this class and this site survived its sweep because the sweep looked for
    /// `ORDER BY … message_id` and this was an aggregate.
    ///
    /// **The one lexicographic comparison that stood beside this is gone, and with it the last of
    /// nxf 6j6v.1zew.** [`opener_wake`](Self::opener_wake) used to gate on `seen >= latest`, and
    /// that was deliberately left lexicographic here (fcv4): it was not an ordering claim of its
    /// own but the system's READ-SET predicate, byte for byte the one the unread inbox used
    /// (`m.message_id > seen`), so making a single site causal would have had the wake and the
    /// inbox disagree about which messages are read. nxf 6j6v.2hx9 took the cursor out of the wake
    /// (it is derived from the operation now) and nxf 6j6v.4d2z removed the read set outright, so
    /// no lexicographic comparison over `message_id` is left anywhere: every ordering claim in this
    /// store is the causal key.
    pub fn latest_reply_message_id(&self, thread_id: &str) -> crate::error::Result<Option<String>> {
        // `ORDER BY … LIMIT 1` returns NO row when the joins match nothing (no expected reply,
        // empty/NULL expects_reply_from), unlike the aggregate this replaced, which always returned
        // one row holding NULL — hence `.optional()`.
        Ok(self
            .inner
            .connection()
            .query_row(
                // A reply this replica can vouch for (nxf 6j6v.pzkb) — it is the completing reply.
                &format!(
                    "SELECT m.message_id
                       FROM messages m
                       JOIN threads t ON t.thread_id = m.thread_id
                       JOIN json_each(t.expects_reply_from) je ON je.value = m.sender
                      WHERE m.thread_id = ?1 AND {vouched}
                      ORDER BY m.lamport DESC, m.site DESC, m.message_id DESC
                      LIMIT 1",
                    vouched = acts("m"),
                ),
                [thread_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    // `read_cursor_seen(consumer, channel)` stood here — the getter over `read_cursors.seen`.
    // REMOVED with the apparatus (nxf 6j6v.4d2z). It had two readers in its life and outlived
    // both: [`opener_wake`](Self::opener_wake), which is derived from the OPERATION since nxf
    // 6j6v.2hx9, and `inbox` above.

    /// **What the caller's own commissions did while it was away** (nxf 6j6v.2hx9) — the threads `C`
    /// opened that reached a terminal state INSIDE the window `since` opens: **complete** (every
    /// expected handle answered, and the completing reply landed after `since`) or **stale** (the
    /// declared window ran out, and it ran out after `since`). Both lists inherit the bulk quorum
    /// read's deterministic `(channel, thread)` order; `now` drives ONLY the clock-dependent
    /// `stale` flag (§4.3). A db error surfaces as `io`.
    ///
    /// # What `since` is, and why the read has one at all
    ///
    /// The caller's PREVIOUS SESSION END ([`previous_session_end`](Self::previous_session_end)) — a
    /// watermark the system already records, and it needs no acknowledgement because "finished" does
    /// not become unfinished.
    ///
    /// Until this it was the READ CURSOR: a completed board dropped off exactly when the opener's
    /// channel cursor passed the completing reply. Nothing ever acked, so in practice the notice
    /// never cleared and every session start replayed every commission the caller had ever
    /// finished. Three consequences follow from the change and they are the point: no clearing rule
    /// has to be invented, the read cursor lost its last consumer here (and was removed outright by
    /// nxf 6j6v.4d2z), and there is ONE truth about where an operation stands instead of two
    /// derivations that can disagree.
    ///
    /// **The cost, stated rather than discovered**: a caller that starts twice in a row sees the
    /// notice once. That is a different promise from "until you acknowledge it", and it is the
    /// promise this read makes.
    ///
    /// `None` for `since` means the caller has no window at all — see the branch below.
    pub fn opener_wake(
        &self,
        consumer: &str,
        now: &str,
        since: Option<&str>,
    ) -> crate::error::Result<OpenerWake> {
        let empty = || OpenerWake {
            complete: Vec::new(),
            stale: Vec::new(),
        };
        // **No watermark, no window** (nxf 6j6v.2hx9). A caller with no previous session — a human
        // at a terminal, or a persona's very first start — is not served by this read at all, and
        // saying so with an empty answer is the honest shape: the alternative would be to fall back
        // on "everything, ever", which is precisely the unbounded notice this item removes. That
        // caller asks `nxc status`, which answers the same question on demand.
        let Some(since) = since else {
            return Ok(empty());
        };
        let opened = self.threads_opened_by(consumer)?;
        if opened.is_empty() {
            return Ok(empty());
        }
        let refs: Vec<&str> = opened.iter().map(String::as_str).collect();
        let mut complete = Vec::new();
        let mut stale = Vec::new();
        for q in self.thread_quorums(&refs, now)? {
            // complete and stale are disjoint (complete ⇒ outstanding=∅ ⇒ not stale), so a thread
            // lands in at most one list.
            if q.complete {
                // complete ⇒ every expected handle replied ⇒ a latest reply exists; skip defensively
                // if somehow absent rather than surfacing a board with no completing reply.
                let Some(latest) = self.latest_reply_message_id(&q.thread_id)? else {
                    continue;
                };
                let rows = self.messages_in_thread(&q.thread_id)?;
                // **WHEN it finished, off the completing reply's own instant** (nxf 6j6v.2hx9) —
                // read from the rows this branch was already fetching, so the window costs no
                // second query.
                //
                // A completing reply with NO recorded instant is skipped rather than included. Such
                // a row predates this column being written on every message, so it is by
                // construction older than any watermark this derivation can be given — and the
                // failure direction matters: including it would put a board from a year ago on
                // every session start, which is the exact defect being removed.
                let Some(finished_at) = rows
                    .iter()
                    .find(|m| m.message_id == latest)
                    .and_then(|m| m.created.clone())
                else {
                    continue;
                };
                // The WINDOW, and it replaces the read cursor entirely: what a session start names
                // is what finished while it was away, not what nobody has acknowledged. Nothing
                // clears this list because nothing has to — "finished" does not become unfinished,
                // and the window moves on its own with every session that ends.
                if finished_at.as_str() <= since {
                    continue;
                }
                let messages = rows
                    .into_iter()
                    .map(|m| WakeMessage {
                        completes: m.message_id == latest,
                        message_id: m.message_id,
                        sender: m.sender,
                        body: m.body,
                    })
                    .collect();
                complete.push(CompletedThread {
                    thread_id: q.thread_id,
                    channel_id: q.channel_id,
                    expects: q.expects,
                    messages,
                    completing_message_id: latest,
                    finished_at,
                });
            } else if q.stale {
                // The same window, asked of the other terminal state: a round STOPPED when its
                // declared deadline passed, so a board whose window ran out before this caller's
                // previous session ended was already reported to it — or was never its business.
                // `stale` implies a deadline exists; the `else` is defensive.
                let Some(deadline) = q.deadline.clone() else {
                    continue;
                };
                if deadline.as_str() <= since {
                    continue;
                }
                stale.push(StaleThread {
                    thread_id: q.thread_id,
                    channel_id: q.channel_id,
                    deadline: q.deadline,
                    outstanding: q.outstanding,
                });
            }
        }
        Ok(OpenerWake { complete, stale })
    }

    /// **When this role's previous session ended** (nxf 6j6v.2hx9) — the watermark a session start
    /// derives "what finished while I was away" from.
    ///
    /// The most recent end anywhere in this role's session history — `ended` for a session that
    /// stopped and stayed stopped, and `ended_before` for one that has since been RESUMED, which is
    /// where [`reopen_session`](Self::reopen_session) moves the instant it clears. A session being
    /// resumed is by far the commonest caller of this read: it is priming right now, its own `ended`
    /// was cleared a moment ago by the trigger that started it, and its previous end is exactly the
    /// window it wants.
    ///
    /// **Scoped to the ROLE, not to one session**, and the widening is deliberate: a persona has one
    /// live session at a time by construction (the single-process guard), so "the last time this
    /// caller stopped" and "the last time any session of this role stopped" are the same instant in
    /// every ordinary workspace — and asking by role means a FRESH session of a role that has run
    /// before still gets a window, which asking by session id could not.
    ///
    /// `None` means this role has never had a session end on this device — its first start, or a
    /// device that only ever killed its sessions — and the caller then has no window at all rather
    /// than an unbounded one.
    ///
    /// **Device-local, like the table it reads.** `session_map` names processes on THIS machine, so
    /// a persona that ran yesterday on another laptop leaves no watermark here. That is the honest
    /// answer for a notice whose whole subject is "what happened while I was not running".
    pub fn previous_session_end(&self, role: &str) -> crate::error::Result<Option<String>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT MAX(COALESCE(ended, ended_before)) FROM session_map WHERE role = ?1",
                [role],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// A channel is "known" once it has a `channels` row OR any resolved member — either a
    /// materialisation that sets fields, or a bare membership add, makes it addressable. Unknown →
    /// the CLI's `not_found` (spec §5.1).
    pub fn channel_exists(&self, channel_id: &str) -> crate::error::Result<bool> {
        let has_row: bool = self.inner.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM channels WHERE channel_id=?1)",
            [channel_id],
            |r| r.get(0),
        )?;
        Ok(has_row || !self.resolve_membership(channel_id)?.is_empty())
    }

    /// Whether `handle` is a resolved member of `channel_id` (OR-set present). A non-member of an
    /// EXISTING channel is the CLI's `forbidden` (spec §5.1). A db error → `io`.
    pub fn is_member(&self, channel_id: &str, handle: &str) -> crate::error::Result<bool> {
        Ok(self
            .resolve_membership(channel_id)?
            .iter()
            .any(|h| h == handle))
    }

    /// `(channel_id, thread_id)` of a message, for `reply` target resolution. `None` if unknown.
    pub fn message_channel(&self, message_id: &str) -> Option<(String, Option<String>)> {
        self.inner
            .connection()
            .query_row(
                "SELECT channel_id, thread_id FROM messages WHERE message_id=?1",
                [message_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
    }

    /// The `refs.session_id` on a message — its "return address" (nxf epic 6j6v.zenf, T7): the
    /// internal session id `reply` routes a resume to. Mirrors `message_channel`'s direct column
    /// read/`.optional()` shape, then parses the stored `refs` JSON the same way
    /// `MessageView::try_from` does. `None` — never an error — when the message is unknown, carries
    /// no `refs` at all, or its `refs.session_id` is unset: a reply to a plain human/ad-hoc message
    /// (no return address) is an ordinary, unremarkable case, not a failure.
    ///
    /// **Only off a message an action may follow** (nxf 6j6v.pzkb): the address names a session on
    /// THIS machine that a reply resumes, and a message nobody here can vouch for would otherwise
    /// choose it. Such a message has no return address, which is the ordinary, unremarkable case.
    pub fn message_return_address(&self, message_id: &str) -> crate::error::Result<Option<String>> {
        let refs: Option<String> = self
            .inner
            .connection()
            .query_row(
                &format!(
                    "SELECT m.refs FROM messages m WHERE m.message_id=?1 AND {}",
                    acts("m")
                ),
                [message_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let Some(txt) = refs else {
            return Ok(None);
        };
        let parsed: Refs = serde_json::from_str(&txt)
            .map_err(|_| NxfError::io(format!("message {message_id} has unparseable refs")))?;
        Ok(parsed.session_id)
    }

    /// The channel a thread was OPENED in (the `threads` view), for `reply` target resolution.
    /// `None` if unknown. In M1 threads are not first-class "opened" (open/quorum is M2, §9 TB-C5),
    /// so this is empty today; [`thread_channel_via_message`](Self::thread_channel_via_message) is
    /// the M1 path.
    pub fn thread_channel(&self, thread_id: &str) -> Option<String> {
        self.inner
            .connection()
            .query_row(
                "SELECT channel_id FROM threads WHERE thread_id=?1",
                [thread_id],
                |r| r.get(0),
            )
            .ok()
    }

    /// The channel of a thread as seen from its MESSAGES — any message carrying `thread_id` (the
    /// deterministic lowest id). This is how `reply <thread_id>` resolves in M1: threads are not
    /// opened as first-class objects yet (§9 TB-C5), so a thread is addressable only through the
    /// messages that a `send --thread` stamped with its id. `None` if no message bears it.
    pub fn thread_channel_via_message(&self, thread_id: &str) -> Option<String> {
        self.inner
            .connection()
            .query_row(
                "SELECT channel_id FROM messages WHERE thread_id=?1 ORDER BY message_id LIMIT 1",
                [thread_id],
                |r| r.get(0),
            )
            .ok()
    }

    /// Channels `handle` is a member of, with name/kind/member-count/degraded flag, channel-id sorted.
    /// A `direct` channel whose resolved membership is not exactly 2 is flagged `degraded` (spec §3.3).
    ///
    /// ONE SQLite round-trip regardless of channel count (be9y review): the member count is a
    /// correlated sub-select and the name/kind a `LEFT JOIN`, so there is no per-channel query. The
    /// member count matches [`resolve_membership`](Self::resolve_membership)`.len()` (live add-tags,
    /// not distinct handles) and `degraded` reuses the shared [`is_degraded`], so the row bytes are
    /// identical to the old per-channel loop. A db error → `io`.
    pub fn list_member_channels(&self, handle: &str) -> crate::error::Result<Vec<ChannelRow>> {
        let conn = self.inner.connection();
        let mut st = conn.prepare(
            "SELECT ma.channel_id, c.name, c.kind,
                    (SELECT COUNT(*) FROM membership_adds a
                       WHERE a.channel_id = ma.channel_id
                         AND a.tag NOT IN (SELECT tag FROM membership_removes)) AS members
               FROM (SELECT DISTINCT channel_id FROM membership_adds
                       WHERE handle=?1 AND tag NOT IN (SELECT tag FROM membership_removes)) ma
               LEFT JOIN channels c ON c.channel_id = ma.channel_id
              ORDER BY ma.channel_id",
        )?;
        let rows = st
            .query_map([handle], |r| {
                let channel_id: String = r.get(0)?;
                let name: Option<String> = r.get(1)?;
                let kind: Option<String> = r.get(2)?;
                let members = r.get::<_, i64>(3)? as usize;
                let degraded = is_degraded(kind.as_deref(), members);
                Ok(ChannelRow {
                    channel_id,
                    name,
                    kind,
                    members,
                    degraded,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Every `public` channel in the workspace — the discovery read (nxf 6j6v.bd6g), channel-id
    /// sorted, with the SAME `ChannelRow` shape and member-count derivation
    /// [`list_member_channels`](Self::list_member_channels) uses.
    ///
    /// **Membership plays no part**, which is the whole point: a front door nobody has joined is
    /// still a front door. `members` is therefore the count and may legitimately be `0`;
    /// `degraded` reuses the shared [`is_degraded`], which only ever flags a `direct` channel, so a
    /// public channel is never degraded by having none.
    ///
    /// ONE SQLite round-trip regardless of channel count (the same correlated sub-select), and no
    /// per-channel decoration query. A db error → `io`.
    pub fn list_public_channels(&self) -> crate::error::Result<Vec<ChannelRow>> {
        let conn = self.inner.connection();
        let mut st = conn.prepare(
            "SELECT c.channel_id, c.name, c.kind,
                    (SELECT COUNT(*) FROM membership_adds a
                       WHERE a.channel_id = c.channel_id
                         AND a.tag NOT IN (SELECT tag FROM membership_removes)) AS members
               FROM channels c
              WHERE c.kind = ?1
              ORDER BY c.channel_id",
        )?;
        let rows = st
            .query_map([crate::model::CHANNEL_KIND_PUBLIC], |r| {
                let channel_id: String = r.get(0)?;
                let name: Option<String> = r.get(1)?;
                let kind: Option<String> = r.get(2)?;
                let members = r.get::<_, i64>(3)? as usize;
                let degraded = is_degraded(kind.as_deref(), members);
                Ok(ChannelRow {
                    channel_id,
                    name,
                    kind,
                    members,
                    degraded,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// All profiles, handle-sorted (deterministic).
    pub fn profiles(&self) -> Vec<ProfileRow> {
        self.query_profiles(None)
    }

    /// One profile by handle, or `None`.
    pub fn profile(&self, handle: &str) -> Option<ProfileRow> {
        self.query_profiles(Some(ProfileFilter::Handle(handle)))
            .into_iter()
            .next()
    }

    /// Profiles whose `job_title` OR `job_description` contains `q` (case-insensitive, bound
    /// `LIKE ? ESCAPE '\'` — no interpolation; spec §5.1 / review 3cz4), handle-sorted.
    pub fn search_profiles(&self, q: &str) -> Vec<ProfileRow> {
        self.query_profiles(Some(ProfileFilter::Search(q)))
    }

    fn query_profiles(&self, filter: Option<ProfileFilter>) -> Vec<ProfileRow> {
        let base = "SELECT handle, job_title, job_description, capability_tags, runtime_binding,
                           reports_to, origin FROM profiles";
        let map = |r: &rusqlite::Row| {
            Ok(ProfileRow {
                handle: r.get(0)?,
                job_title: r.get(1)?,
                job_description: r.get(2)?,
                capability_tags: r.get(3)?,
                runtime_binding: r.get(4)?,
                reports_to: r.get(5)?,
                origin: r.get(6)?,
            })
        };
        let conn = self.inner.connection();
        match filter {
            None => {
                let mut st = conn.prepare(&format!("{base} ORDER BY handle")).unwrap();
                st.query_map([], map).unwrap().map(Result::unwrap).collect()
            }
            Some(ProfileFilter::Handle(h)) => {
                let mut st = conn.prepare(&format!("{base} WHERE handle=?1")).unwrap();
                st.query_map([h], map)
                    .unwrap()
                    .map(Result::unwrap)
                    .collect()
            }
            Some(ProfileFilter::Search(q)) => {
                // Bound LIKE with the wildcards added to the PARAMETER, never the SQL; ESCAPE '\' so
                // a literal %/_ in the query is not a wildcard. Case-fold BOTH sides with SQLite's
                // `lower()` (ASCII) so the query and the stored value never disagree — a Rust
                // `to_lowercase()` (Unicode) on one side would silently mismatch non-ASCII (CQ #4).
                let like = format!("%{}%", escape_like(q));
                let mut st = conn
                    .prepare(&format!(
                        "{base} WHERE lower(COALESCE(job_title,'')) LIKE lower(?1) ESCAPE '\\'
                                  OR lower(COALESCE(job_description,'')) LIKE lower(?1) ESCAPE '\\'
                                ORDER BY handle"
                    ))
                    .unwrap();
                st.query_map([&like], map)
                    .unwrap()
                    .map(Result::unwrap)
                    .collect()
            }
        }
    }

    /// Messages whose `body` contains `q` (case-insensitive, bound `LIKE ? ESCAPE '\'`), in the
    /// GIVEN `channels`, ordered by `(channel_id, lamport, site, message_id)` — deterministic (spec
    /// §5.1). An empty `channels` is an empty result and no query at all — never "then all of them".
    ///
    /// **WHO may search WHAT is not decided here any more, and that is the whole of nxf 6j6v.cs03.**
    /// This read used to take a `handle` and join the substrate's materialised member set
    /// (`membership_adds` minus `membership_removes`), which made it a SECOND answer to a question
    /// [`facade::require_thread_readable`](crate::facade::require_thread_readable) already answered
    /// for the reader beside it — and a different one, because that set is grow-only, is only ever
    /// added to when somebody SENDS, and records a declared channel's `members:` BARE while a caller
    /// arrives QUALIFIED (`origin/handle`). The membership rule is ONE rule one layer up now:
    /// [`facade::search`](crate::facade::search) resolves the scope through `facade::declared_seat`,
    /// which is what the thread reader's own gate is built out of — and this read applies a scope
    /// instead of inventing one.
    ///
    /// A channel's declared `visibility` is not in the store either — it is a field of
    /// `channels.yaml`, which this layer deliberately does not load (see
    /// [`facade::thread`](crate::facade::thread)) — so the rule that decides whether one hit may be
    /// SHOWN is applied in the same place, out of the [`thread_id`](MessageHit::thread_id) this read
    /// carries. **Neither half of the answer is here: reach for the facade, never for this, wherever
    /// a caller is being answered.**
    ///
    /// **The scope binds one SQL variable per channel, so it is bounded by SQLite's
    /// `SQLITE_LIMIT_VARIABLE_NUMBER`** — 32766 in the bundled build, i.e. a caller would have to be
    /// a member of that many channels to reach it (independent review, Integrity #1). Named rather
    /// than capped, because the failure mode is already the safe one: `prepare` returns an `Err`
    /// that travels as an `io` error, so the read refuses rather than truncating its scope — and a
    /// truncated scope is the one outcome that would be a silent confidentiality change.
    pub fn search_messages(
        &self,
        channels: &[String],
        q: &str,
    ) -> crate::error::Result<Vec<MessageHit>> {
        if channels.is_empty() {
            return Ok(Vec::new());
        }
        // Case-fold BOTH sides with SQLite's `lower()` (ASCII) — see the CQ #4 note in
        // `query_profiles`; a Rust `to_lowercase()` on the query alone would mismatch non-ASCII.
        let like = format!("%{}%", escape_like(q));
        let placeholders = vec!["?"; channels.len()].join(",");
        let conn = self.inner.connection();
        let mut st = conn.prepare(&format!(
            "SELECT m.message_id, m.channel_id, m.sender, m.body, m.thread_id
                   FROM messages m
                  WHERE m.channel_id IN ({placeholders})
                    AND lower(m.body) LIKE lower(?) ESCAPE '\\'
                  ORDER BY m.channel_id, m.lamport, m.site, m.message_id"
        ))?;
        // The scope binds first and the pattern last — anonymous `?` in that order, the shape
        // `thread_quorums` and the assignee reads already bind an IN-list with.
        let mut binds: Vec<&str> = channels.iter().map(String::as_str).collect();
        binds.push(&like);
        let rows = st
            .query_map(rusqlite::params_from_iter(binds.iter()), |r| {
                Ok(MessageHit {
                    message_id: r.get(0)?,
                    channel_id: r.get(1)?,
                    sender: r.get(2)?,
                    body: r.get(3)?,
                    thread_id: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // Every read over `workflow_runs` stood here — the folded run, the filtered list, the event
    // stream and its cursor, the step visit count, a run's declared workflow name, its ticket set,
    // its session binding, its status and its channel threads. ALL REMOVED with the run record
    // itself (6j6v.dvyq §3): with `workflow start` gone from both surfaces nothing can write one
    // again, so these were reads over a table that could never be filled.
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Every table a chat store keeps is either one of `MessageReducer`'s views — which a snapshot
    /// carries and `clear_views` empties — or declared here as this machine's own, which must never
    /// reach another replica (6j6v.mxt2, review of PR #487, Code Quality #4). A table added to the
    /// schema is red here until somebody decides which it is.
    #[test]
    fn every_table_in_a_chat_store_is_a_view_or_declared_machine_local() {
        use nxs_foundation::reducer::Reducer as _;
        const MACHINE_LOCAL: &[&str] = &[
            "session_map",
            "pending_wakes",
            "agent_transcript",
            "channel_member_deadline",
            "synthesis_claims",
            "working_tree_lease",
            "working_tree_queue",
            "declaration_snapshot",
            "declaration_freeze",
            "role_declaration_version",
            "operation_base",
            "parked_work",
            "park_refusals",
            "withdrawn_holders",
            "withdrawn_sessions",
            "session_interruption",
        ];
        let store = ChatStore::open_in_memory(1);
        let tables: Vec<String> = store
            .connection()
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let views = MessageReducer.view_tables();
        for t in &tables {
            assert!(
                nxs_foundation::schema::SUBSTRATE_TABLES.contains(&t.as_str())
                    || views.contains(&t.as_str())
                    || MACHINE_LOCAL.contains(&t.as_str()),
                "{t} is neither a chat view nor declared machine-local — decide which it is"
            );
        }
        for listed in views.iter().chain(MACHINE_LOCAL) {
            assert!(
                tables.iter().any(|t| t == listed),
                "{listed} is classified but no longer exists"
            );
        }
    }

    /// The rows a test places DIRECTLY into the views, vouched for: an op of this replica's own at
    /// each `(lamport, site)`, standing in for the op a real row is folded from. The check before an
    /// action (nxf 6j6v.pzkb) joins the log on exactly that coordinate, so without it a hand-placed
    /// row is one nobody can vouch for — and discharges nothing, which is not what these tests pin.
    fn vouch_for(s: &ChatStore, coordinates: &[(i64, i64)]) {
        for (lamport, site) in coordinates {
            s.connection()
                .execute(
                    "INSERT INTO ops(op_id, lamport, site, domain, target_kind, target_id, field,
                                     op_type, value, author, wall_clock, provenance)
                     VALUES(?1, ?2, ?3, 'message', 'message', ?1, 'body', 'post', NULL, 'o/a', '',
                            'own')",
                    params![format!("vouch-{lamport}-{site}"), lamport, site],
                )
                .unwrap();
        }
    }

    /// Serializes the tests that care about `NXF_DETERMINISTIC_IDS`, which is process-global.
    ///
    /// Found by a flake, not by review: under the switch a minted id is a per-store COUNT, so two
    /// stores independently mint the same `m-…0001`. A test that merges two stores then loses a
    /// message to the primary key — while the switch was flipped by a *different* test running on
    /// another thread of the same binary, for reasons having nothing to do with the code under test.
    /// Reproduced at 1-in-30 by running just the two implicated tests on two threads.
    ///
    /// So: any test that flips the switch, AND any test that mints ids in two stores and merges
    /// them, takes this lock. (Mirrors `timer.rs`'s and `channel_complete.rs`'s own env locks; a
    /// poisoned mutex is recovered so one failure does not cascade.)
    static ID_SWITCH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Hold [`ID_SWITCH_LOCK`] and guarantee the switch is off again on the way out — including
    /// through a panicking assertion, which would otherwise leak it into every later test.
    struct IdSwitchGuard(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

    impl IdSwitchGuard {
        fn acquire() -> IdSwitchGuard {
            IdSwitchGuard(ID_SWITCH_LOCK.lock().unwrap_or_else(|e| e.into_inner()))
        }
    }

    impl Drop for IdSwitchGuard {
        fn drop(&mut self) {
            std::env::remove_var("NXF_DETERMINISTIC_IDS");
        }
    }

    fn env(channel: &str, body: &str, disp: Disposition) -> MessageEnvelope {
        MessageEnvelope {
            origin: "o".into(),
            channel_id: channel.into(),
            sender: "o/a".into(),
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: disp,
            thread_id: None,
            refs: Refs::default(),
            body: body.into(),
        }
    }

    #[test]
    #[should_panic(expected = "unknown channel field")]
    fn set_channel_field_rejects_a_non_whitelisted_field() {
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "kynd", "group", "o/a"); // typo — not in CHANNEL_FIELDS
    }

    #[test]
    #[should_panic(expected = "unknown profile field")]
    fn set_profile_field_rejects_a_non_whitelisted_field() {
        let mut s = ChatStore::open_in_memory(1);
        s.set_profile_field("o/a", "salary", "1", "o/a"); // not in PROFILE_FIELDS
    }

    #[test]
    #[should_panic(expected = "membership id must not contain the U+001F separator")]
    fn add_member_rejects_a_sep_byte_in_the_composite_id() {
        let mut s = ChatStore::open_in_memory(1);
        s.add_member(&format!("c{SEP}1"), "alice", "o/a");
    }

    #[test]
    #[should_panic(expected = "membership id must not contain the U+001F separator")]
    fn remove_member_rejects_a_sep_byte_in_the_composite_id() {
        let mut s = ChatStore::open_in_memory(1);
        s.remove_member("c-1", &format!("ali{SEP}ce"), "o/a");
    }

    #[test]
    fn messages_in_channel_returns_full_rows_in_causal_order() {
        // Causal order is now minting-independent (0b1m): `messages_in_channel` orders by
        // `(lamport, site, message_id)`, so `first` (posted first → lower lamport) reads back before
        // `second` deterministically, regardless of the ULID-vs-sequence minting mode. The old
        // `expected.sort()` workaround (needed because `ORDER BY message_id` raced on same-ms ULIDs)
        // is gone — the concrete post order is asserted directly.
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "kind", "group", "o/a");
        let m1 = s.post_message(&env("c-1", "first", Disposition::InTurn));
        let m2 = s.post_message(&env("c-1", "second", Disposition::NextSession));
        s.set_channel_field("c-2", "kind", "group", "o/a");
        s.post_message(&env("c-2", "other channel", Disposition::InTurn));
        let rows = s.messages_in_channel("c-1").unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r.message_id.clone())
                .collect::<Vec<_>>(),
            vec![m1, m2],
            "only c-1's messages, in post (causal) order"
        );
        // Content keyed by body (position-independent, so it can't flake on minting order).
        let first = rows.iter().find(|r| r.body == "first").unwrap();
        assert_eq!(first.kind, "info");
        let second = rows.iter().find(|r| r.body == "second").unwrap();
        assert_eq!(second.disposition, "next_session");
    }

    #[test]
    fn same_millisecond_posts_read_back_in_causal_post_order() {
        // 0b1m: a production `message_id` is `m-`+ULID = a 48-bit millisecond + 80 random bits, so
        // posts within ONE millisecond (an `ask` opener + its first reply, a rapid send/reply burst)
        // sort by ULID randomness under `ORDER BY message_id` — an arbitrary, per-db, ~38%-flaky read
        // order. Ordering by the CRDT clock `(lamport, site, message_id)` makes read-back
        // deterministic == post order: each op's lamport is strictly higher than the ops it follows.
        // This posts a tight same-ms burst (real ULIDs — no NXF_DETERMINISTIC_IDS) and asserts causal
        // order. It stays green across repeated debug AND release runs because lamport ordering is
        // minting-independent; the burst is large enough that the only way `message_id` order could
        // coincide with post order — 24 random ULID tails already ascending — is ~1/24! ≈ never.
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "kind", "group", "o/a");
        let posted: Vec<String> = (0..24)
            .map(|i| {
                let body = format!("m{i:02}");
                s.post_message(&env("c-1", &body, Disposition::InTurn));
                body
            })
            .collect();
        let read_back: Vec<String> = s
            .messages_in_channel("c-1")
            .unwrap()
            .into_iter()
            .map(|r| r.body)
            .collect();
        assert_eq!(
            read_back, posted,
            "a same-millisecond burst reads back in post (causal) order, not ULID-tail order"
        );
    }

    #[test]
    fn same_lamport_cross_site_messages_order_by_site_deterministically() {
        // 0b1m (PR-review Test Quality, Low): the `site` component of `(lamport, site, message_id)`
        // breaks ties between CONCURRENT same-lamport ops from different replicas, so a merged read
        // is identical on every site. Two stores each post their first message before seeing the
        // other — both land at the same lamport (the channel-field op then the post → lamport 2),
        // sites 1 and 2 — and after a symmetric merge both must read site 1 before site 2,
        // regardless of the ULID tails.
        //
        // Takes the id-switch lock (see `IdSwitchGuard`): this test mints in TWO stores and merges
        // them, so if another test flips `NXF_DETERMINISTIC_IDS` mid-run both messages become
        // `m-…0001` and the merge silently drops one — a red that says nothing about this code.
        let _guard = IdSwitchGuard::acquire();
        let mut a = ChatStore::open_in_memory(1);
        a.set_channel_field("c-1", "kind", "group", "o/a");
        let ma = a.post_message(&env("c-1", "from-a", Disposition::InTurn));

        let mut b = ChatStore::open_in_memory(2);
        b.set_channel_field("c-1", "kind", "group", "o/a");
        let mb = b.post_message(&env("c-1", "from-b", Disposition::InTurn));

        let (ea, eb) = (a.export(), b.export());
        a.apply(&eb);
        b.apply(&ea);

        let ids = |s: &ChatStore| {
            s.messages_in_channel("c-1")
                .unwrap()
                .into_iter()
                .map(|r| r.message_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&a),
            ids(&b),
            "the merged read order converges on both sites"
        );
        assert_eq!(
            ids(&a),
            vec![ma, mb],
            "site 1 (from-a) orders before site 2 (from-b) at equal lamport"
        );
    }

    #[test]
    fn messages_in_thread_filters_by_thread_id() {
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "kind", "group", "o/a");
        let mut e = env("c-1", "in thread", Disposition::InTurn);
        e.thread_id = Some("m-thread".into());
        let mid = s.post_message(&e);
        s.post_message(&env("c-1", "loose", Disposition::InTurn)); // no thread
        let rows = s.messages_in_thread("m-thread").unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r.message_id.as_str())
                .collect::<Vec<_>>(),
            vec![mid.as_str()]
        );
        assert_eq!(rows[0].thread_id.as_deref(), Some("m-thread"));
    }

    #[test]
    fn full_row_reads_surface_a_db_error_as_io() {
        let s = ChatStore::open_in_memory(1);
        s.connection()
            .execute_batch("DROP TABLE messages;")
            .unwrap();
        assert_eq!(
            s.messages_in_channel("c-1").unwrap_err().kind,
            crate::error::ErrorKind::Io
        );
        assert_eq!(
            s.messages_in_thread("t-1").unwrap_err().kind,
            crate::error::ErrorKind::Io
        );
    }

    #[test]
    fn dm_with_wrong_member_count_reads_as_degraded() {
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("dm-1", "kind", "direct", "o/a");
        s.add_member("dm-1", "o/a", "o/a");
        s.add_member("dm-1", "o/b", "o/a");
        assert!(!s.is_degraded_dm("dm-1").unwrap(), "exactly 2 → healthy");
        s.add_member("dm-1", "o/intruder", "o/a"); // a raw 3rd add (non-nxc client)
        assert!(
            s.is_degraded_dm("dm-1").unwrap(),
            "≠2 direct-channel members → degraded (spec §3.3)"
        );
    }

    /// A foreign `message` op as it would ride the sync wire — built by hand so the test can land it
    /// in a shared log via a raw substrate (no message reducer), mirroring a foundation-only pull
    /// (aye.36's `refold_on_open_materializes_fact_ops_from_a_foundation_only_pull`,
    /// `crates/memory/src/store.rs`).
    fn message_op(op_id: &str, message_id: &str, envelope: &MessageEnvelope) -> Op {
        Op {
            op_id: op_id.into(),
            lamport: 7,
            site: 2,
            domain: DOMAIN_MESSAGE.into(),
            target_kind: KIND_MESSAGE.into(),
            target_id: message_id.into(),
            field: FIELD_ENVELOPE.into(),
            op_type: OP_POST.into(),
            value: Some(serde_json::to_string(envelope).unwrap()),
            author: "device-A".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        }
    }

    #[test]
    fn refold_on_open_materializes_message_ops_from_a_foundation_only_pull() {
        // aye.36: nxs sync moves the shared op-log but, foundation-only, folds no product views. A
        // message op that lands out-of-band must materialize into `messages` the next time the chat
        // store opens — refold-when-behind, driven by the folded-through watermark. Mirrors memory's
        // `refold_on_open_materializes_fact_ops_from_a_foundation_only_pull`.
        use nxs_foundation::store::Store as Substrate;

        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nxc-aye36.db");
        let p = path.to_str().unwrap();

        // Device-local chat, folded inline by the chat store.
        {
            let mut c = ChatStore::open(p, 1).unwrap();
            c.post_message(&env("c-1", "from device 1", Disposition::InTurn));
        }
        // A foundation-only pull: a RAW substrate store (no message reducer) appends a foreign
        // message op. It lands in the shared log unfolded and does not touch the chat store's
        // watermark.
        {
            let mut sub = Substrate::open(p, 2).unwrap();
            let deferred = sub.apply(std::slice::from_ref(&message_op(
                "m.foreign",
                "m-foreign",
                &env("c-1", "from device 2", Disposition::InTurn),
            )));
            assert_eq!(
                deferred.len(),
                1,
                "the raw substrate defers the message op (no reducer)"
            );
        }
        // The next chat open is behind the log → refold-on-open folds the foreign message in,
        // keeping the device-local one.
        {
            let c = ChatStore::open(p, 1).unwrap();
            let body: String = c
                .connection()
                .query_row(
                    "SELECT body FROM messages WHERE message_id='m-foreign'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                body, "from device 2",
                "the out-of-band message op is materialized on the next chat open"
            );
            let n: i64 = c
                .connection()
                .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 2, "device-local message survives alongside it");
        }
        // Idempotent: a further open with no new ops leaves the view unchanged.
        {
            let c = ChatStore::open(p, 1).unwrap();
            let n: i64 = c
                .connection()
                .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 2, "stable across a needless reopen");
        }
    }

    #[test]
    fn opening_an_m1_workspace_force_refolds_a_deferred_deadline_op() {
        // The sparse-key minor (spec §7): an M1 workspace has a `threads` table WITHOUT the deadline
        // columns and a synced `set deadline` op that M1 store-don't-folded — yet M1 advanced its
        // chat watermark PAST that op (it refolded everything else). A plain refold_if_behind on the
        // M2 open would be a no-op (caught up), so the deadline would never fold. The view-schema
        // bump (column add + force_refold) must resurface it.
        use nxs_foundation::store::Store as Substrate;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("m1.db");
        let p = path.to_str().unwrap();

        {
            // Build the M1 state directly in the shared db.
            let sub = Substrate::open(p, 1).unwrap();
            let conn = sub.connection();
            // The M1-shaped threads table: no deadline / deadline_v / deadline_site columns.
            conn.execute_batch(
                "CREATE TABLE threads(
                     thread_id TEXT PRIMARY KEY,
                     origin TEXT, channel_id TEXT, opener TEXT, created TEXT,
                     expects_reply_from TEXT,
                     expects_reply_from_v INTEGER DEFAULT 0, expects_reply_from_site INTEGER DEFAULT 0
                 );",
            )
            .unwrap();
            // A `thread set deadline` op sitting UNFOLDED in the log (M1 could not fold it).
            conn.execute(
                "INSERT INTO ops(op_id, lamport, site, domain, target_kind, target_id, field,
                                 op_type, value, author, wall_clock)
                 VALUES('op-dl', 3, 1, 'message', 'thread', 't-1', 'deadline', 'set',
                        '2026-07-20T00:00:00Z', 'local/a', '')",
                [],
            )
            .unwrap();
            // M1 advanced its chat watermark to the log boundary (it folded everything foldable).
            conn.execute(
                "INSERT INTO view_watermarks(store_id, folded_through)
                 VALUES('chat', (SELECT MAX(rowid) FROM ops))",
                [],
            )
            .unwrap();
        }

        // The M2 open: the migration adds the deadline columns and force-refolds, folding the op.
        let cs = ChatStore::open(p, 1).unwrap();
        let deadline: Option<String> = cs
            .connection()
            .query_row(
                "SELECT deadline FROM threads WHERE thread_id='t-1'",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap()
            .flatten();
        assert_eq!(
            deadline.as_deref(),
            Some("2026-07-20T00:00:00Z"),
            "the deferred deadline op is resurfaced by the view-schema bump"
        );
    }

    #[test]
    fn open_thread_and_set_expects_reply_from_populate_threads_view() {
        let mut s = ChatStore::open_in_memory(1);
        let root = ThreadRoot {
            origin: "o".into(),
            channel_id: "c-1".into(),
            opener: "o/a".into(),
            created: "2026-07-08T00:00:00Z".into(),
            parent: None,
        };
        s.open_thread("t-1", &root, "o/a");
        let erf = serde_json::to_string(&vec!["o/a", "o/b"]).expect("handles serialize");
        s.set_expects_reply_from("t-1", &erf, "o/a");

        let (channel_id, opener, expects_reply_from): (String, String, String) = s
            .connection()
            .query_row(
                "SELECT channel_id, opener, expects_reply_from FROM threads WHERE thread_id='t-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(channel_id, "c-1", "root's channel_id folds into the view");
        assert_eq!(opener, "o/a", "root's opener folds into the view");
        assert_eq!(
            expects_reply_from, erf,
            "expects_reply_from folds in as the serialized handle list"
        );
    }

    /// Seed a thread `tid` in channel `chan` opened by `opener` expecting `expects`, and post the
    /// opener's request message into it. Returns the store for chaining replies.
    /// Open `tid` under `parent` — [`seed_thread`]'s twin for the tree tests below.
    fn seed_child(s: &mut ChatStore, tid: &str, parent: Option<&str>, opener: &str) {
        let root = ThreadRoot {
            origin: "o".into(),
            channel_id: "c-1".into(),
            opener: opener.into(),
            created: "2026-07-10T00:00:00Z".into(),
            parent: parent.map(str::to_string),
        };
        s.open_thread(tid, &root, opener);
    }

    #[test]
    fn thread_root_walks_up_to_the_top_of_the_chain() {
        let mut s = ChatStore::open_in_memory(1);
        seed_child(&mut s, "t-root", None, "o/human");
        seed_child(&mut s, "t-mid", Some("t-root"), "o/pm");
        seed_child(&mut s, "t-leaf", Some("t-mid"), "o/coder");

        for id in ["t-root", "t-mid", "t-leaf"] {
            assert_eq!(
                s.thread_root(id).unwrap(),
                "t-root",
                "every thread of one operation names the same root ({id})"
            );
        }
    }

    #[test]
    fn thread_root_of_a_thread_with_no_parent_edge_is_itself() {
        let mut s = ChatStore::open_in_memory(1);
        seed_child(&mut s, "t-only", None, "o/a");
        assert_eq!(s.thread_root("t-only").unwrap(), "t-only");
        // And a thread this workspace has never folded at all is the top of what we can see, which
        // is the same answer for the same reason `thread_parent` gives it.
        assert_eq!(s.thread_root("t-unknown").unwrap(), "t-unknown");
    }

    #[test]
    fn thread_root_ends_at_a_parent_this_workspace_has_not_folded() {
        // Ops arrive in relay order, so a child can be here before its parent. The walk stops at the
        // parent that is not here — which has no `threads` row and therefore no opener, which is
        // what makes the read gate above fail CLOSED on a half-arrived chain.
        let mut s = ChatStore::open_in_memory(1);
        seed_child(&mut s, "t-child", Some("t-not-here-yet"), "o/pm");
        assert_eq!(s.thread_root("t-child").unwrap(), "t-not-here-yet");
        assert_eq!(s.thread_opener("t-not-here-yet").unwrap(), None);
    }

    #[test]
    fn thread_root_takes_the_smallest_id_of_a_cycle_exactly_as_status_does() {
        // A cycle cannot arise from any write here, but a synced or hand-edited log could carry one
        // and a walk that trusts the data would hang. `facade`'s `root_of_thread` resolves it by
        // taking the smallest id OF THE CYCLE, and this has to agree with it or one operation would
        // have two names.
        let mut s = ChatStore::open_in_memory(1);
        seed_child(&mut s, "t-bbb", Some("t-ccc"), "o/a");
        seed_child(&mut s, "t-ccc", Some("t-bbb"), "o/a");
        seed_child(&mut s, "t-below", Some("t-ccc"), "o/a");

        assert_eq!(s.thread_root("t-below").unwrap(), "t-bbb");
        assert_eq!(s.thread_root("t-ccc").unwrap(), "t-bbb");
        assert_eq!(s.thread_root("t-bbb").unwrap(), "t-bbb");
    }

    fn seed_thread(s: &mut ChatStore, tid: &str, chan: &str, opener: &str, expects: &[&str]) {
        let root = ThreadRoot {
            origin: "o".into(),
            channel_id: chan.into(),
            opener: opener.into(),
            created: "2026-07-10T00:00:00Z".into(),
            parent: None,
        };
        s.open_thread(tid, &root, opener);
        let erf = serde_json::to_string(expects).unwrap();
        s.set_expects_reply_from(tid, &erf, opener);
    }

    /// Post a message from `sender` into thread `tid` (channel `chan`).
    fn post_in_thread(
        s: &mut ChatStore,
        chan: &str,
        tid: &str,
        sender: &str,
        body: &str,
    ) -> String {
        post_in_thread_of_kind(s, chan, tid, sender, body, MessageKind::Report)
    }

    /// Every [`MessageKind`], walked through an EXHAUSTIVE `match` with no `_` arm.
    ///
    /// **What this guarantees, exactly** (narrowed in fix round 2 — the previous wording claimed
    /// more than the construction delivers): adding a variant to [`MessageKind`] is a COMPILE ERROR
    /// here, so its author cannot avoid coming to this function and deciding what the kind means for
    /// the discharge. That is the property acceptance point 5 needs — an array literal has not even
    /// that, and would have let 6j6v.wt37's escalation marker land silently unpinned.
    ///
    /// **What it does NOT guarantee**: membership. An author who satisfies the compiler with a dead
    /// arm (`Escalation => break`, leaving `Info => break` in place) compiles fine and never puts the
    /// new kind in the list. Rust cannot enforce "every variant appears in a runtime collection"
    /// without a derive macro (`strum`) or the unstable `variant_count`, and neither is worth a
    /// dependency for one test — so the honest statement is "the author is FORCED to look", not "the
    /// author cannot get it wrong". Hence the instruction on the terminator arm below, which is where
    /// such an author is standing when they read it.
    ///
    /// **`pub(crate)` since nxf 6j6v.am8j** (independent review of PR #470, Test Quality #2). The
    /// forcing above reaches the MATCH and stops there: `working_tree.rs` pins every kind's
    /// hand-back classification BY VALUE in a table of its own, and an author forced to come here
    /// is not forced to go there — twice over, `needs_rework` and `accepted` were both classified
    /// at the match and both missing from that table. It now sizes itself against this walk, so the
    /// one derivation the compiler does enforce is the one both tests count from.
    pub(crate) fn every_message_kind() -> Vec<MessageKind> {
        let mut all = Vec::new();
        let mut kind = MessageKind::Task;
        loop {
            all.push(kind);
            kind = match kind {
                MessageKind::Task => MessageKind::Question,
                MessageKind::Question => MessageKind::Report,
                MessageKind::Report => MessageKind::Decision,
                MessageKind::Decision => MessageKind::Info,
                MessageKind::Info => MessageKind::Escalation,
                MessageKind::Escalation => MessageKind::NeedsRework,
                // THE END OF THE CHAIN. Adding a variant? Point this arm at yours and let YOURS
                // break — do not give the new kind its own `break` arm, or it never enters the list
                // and the `--escalate` acceptance stops covering it.
                //
                // 6j6v.wt37 arrived here the way this construction intends: adding
                // `MessageKind::Escalation` was a COMPILE ERROR at this `match`, and threading it in
                // is what put the real marker in the list. 6j6v.553s (a) arrived the same way with
                // `NeedsRework`, and moved the terminator on rather than adding a second `break`.
                // 6j6v.am8j arrived the same way again with `Accepted`, and moved it on again.
                MessageKind::NeedsRework => MessageKind::Accepted,
                MessageKind::Accepted => break,
            };
        }
        all
    }

    /// [`post_in_thread`] with the message KIND spelled out — the parameter the discharge predicate
    /// must go on ignoring (6j6v.cg8g's `--escalate` acceptance).
    fn post_in_thread_of_kind(
        s: &mut ChatStore,
        chan: &str,
        tid: &str,
        sender: &str,
        body: &str,
        kind: MessageKind,
    ) -> String {
        // **Stamped like production** (nxf 6j6v.2hx9). `facade::send`/`reply` set the wall clock
        // from the caller's `now` before every post, so `messages.created` is a real instant in
        // every workspace; a test helper that left it empty would make the session-start window
        // untestable here and would test a shape the product never writes.
        s.set_wall_clock(POSTED_AT);
        s.post_message(&MessageEnvelope {
            origin: "o".into(),
            channel_id: chan.into(),
            sender: sender.into(),
            kind,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some(tid.into()),
            refs: Refs::default(),
            body: body.into(),
        })
    }

    const NOW: &str = "2026-07-11T00:00:00Z";
    /// When [`post_in_thread_of_kind`] stamps its messages — the same instant as [`NOW`], so a
    /// message is "just posted" from the reader's point of view.
    const POSTED_AT: &str = NOW;
    /// A session-start watermark BEFORE anything these tests post: "since my previous session
    /// ended". The window nxf 6j6v.2hx9 replaced the read cursor with.
    const SINCE: &str = "2026-07-10T00:00:00Z";

    #[test]
    fn thread_quorum_tracks_replied_outstanding_and_completion() {
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b", "o/c"]);
        // The opener's OWN request post does not count toward quorum (TB-M2-8: opener not in E).
        post_in_thread(&mut s, "c-1", "t-1", "o/a", "please review");
        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert_eq!(q.expects, vec!["o/b", "o/c"]);
        assert!(q.replied.is_empty(), "no expected handle has replied yet");
        assert_eq!(q.outstanding, vec!["o/b", "o/c"]);
        assert!(!q.complete, "nothing replied → not complete");

        // An expected handle replies → it moves to replied; the board is still incomplete.
        post_in_thread(&mut s, "c-1", "t-1", "o/b", "lgtm");
        // A NON-expected handle posting in the thread never advances quorum (TB-M2-8).
        post_in_thread(&mut s, "c-1", "t-1", "o/x", "drive-by comment");
        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert_eq!(q.replied, vec!["o/b"]);
        assert_eq!(q.outstanding, vec!["o/c"]);
        assert!(!q.complete);

        // The last expected handle replies → complete (E≠∅ ∧ outstanding=∅).
        post_in_thread(&mut s, "c-1", "t-1", "o/c", "approved");
        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert_eq!(q.replied, vec!["o/b", "o/c"]);
        assert!(q.outstanding.is_empty());
        assert!(q.complete, "all expected replied → complete");
    }

    #[test]
    fn a_thread_with_empty_expectations_is_never_complete() {
        // TB-M2-7: E=∅ is not a quorum thread — it never completes (nothing was expected).
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-1", "c-1", "o/a", &[]);
        post_in_thread(&mut s, "c-1", "t-1", "o/a", "just talking");
        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert!(q.expects.is_empty());
        assert!(q.replied.is_empty());
        assert!(q.outstanding.is_empty());
        assert!(!q.complete, "empty E is never complete");
    }

    #[test]
    fn thread_quorum_is_none_for_an_unknown_thread() {
        let s = ChatStore::open_in_memory(1);
        assert!(s.thread_quorum("t-missing", NOW).unwrap().is_none());
    }

    #[test]
    fn quorum_result_is_reorder_independent() {
        // The quorum reads the CONVERGED views (LWW expects + grow-only messages), so the op
        // application order cannot change the result. Apply the same ops naturally vs. scrambled
        // (the thread root folds LAST, after both the register and the reply that references it) and
        // assert the derived quorum is byte-equal.
        //
        // WHAT THE SCRAMBLE MAY MOVE, sharpened by 6j6v.cg8g. It used to sequence the reply BEFORE
        // the declaration, which reads as "fold order" only while `replied` ignores WHEN a message
        // was written. Now that the discharge is measured from the declaration, emitting the reply
        // first is not a reordering of one history at all: `set_expects_reply_from` and
        // `post_message` mint ops with ascending Lamports, so it produces a genuinely DIFFERENT
        // history — one where the answer predates the question, which is a case with its own test
        // (`what_was_written_before_the_obligation_existed_does_not_discharge_it`). What this test is
        // for is the fold's order-independence, so it scrambles what a relay can genuinely deliver
        // out of order: the thread ROOT (whose columns are NULLable for exactly this reason) arrives
        // after the register and the message that name it.
        let natural = {
            let mut s = ChatStore::open_in_memory(1);
            seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b", "o/c"]);
            post_in_thread(&mut s, "c-1", "t-1", "o/b", "reply b");
            s.thread_quorum("t-1", NOW).unwrap().unwrap()
        };
        let scrambled = {
            let mut s = ChatStore::open_in_memory(2);
            s.set_expects_reply_from(
                "t-1",
                &serde_json::to_string(&["o/b", "o/c"]).unwrap(),
                "o/a",
            );
            post_in_thread(&mut s, "c-1", "t-1", "o/b", "reply b");
            // The thread root folds last — after the register and the reply that both name it.
            let root = ThreadRoot {
                origin: "o".into(),
                channel_id: "c-1".into(),
                opener: "o/a".into(),
                created: "2026-07-10T00:00:00Z".into(),
                parent: None,
            };
            s.open_thread("t-1", &root, "o/a");
            s.thread_quorum("t-1", NOW).unwrap().unwrap()
        };
        assert_eq!(
            natural, scrambled,
            "quorum is a pure fold — order cannot matter"
        );
        assert_eq!(scrambled.replied, vec!["o/b"]);
        assert_eq!(scrambled.outstanding, vec!["o/c"]);
    }

    #[test]
    fn re_declaring_expectations_re_asks_everyone_the_new_declaration_names() {
        // §3.2/§4.2: quorum is derived against the CURRENT expects_reply_from (an LWW register), so
        // amending the register re-derives the board.
        //
        // WHAT AMENDING MEANS CHANGED with 6j6v.cg8g, and this test is where it is written down.
        // Under the per-THREAD reading, narrowing to the handles that had already answered flipped
        // `complete` false→true — the "in-core unstick path" for a board one reviewer never came
        // back to. Under the per-TURN reading, a narrowing is a RE-DECLARATION like any other: it
        // asks the handles it names, again, as of now. That is the acceptance point ("a
        // re-declaration resets it, and only what was written afterwards counts") and not a side
        // effect of it — the same register write is what opens a supervisor's second turn, and one
        // register cannot mean "ask again" in one caller's hands and "consider yourself answered" in
        // another's. Closing a board that nobody will answer is therefore no longer a narrowing; it
        // is dropping the expectation (`--set` with the handle removed, leaving nobody outstanding).
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b", "o/c"]);
        post_in_thread(&mut s, "c-1", "t-1", "o/b", "lgtm");
        assert!(
            !s.thread_quorum("t-1", NOW).unwrap().unwrap().complete,
            "o/c outstanding"
        );

        // Narrow to just the responder → the board asks o/b again, and is NOT complete.
        s.set_expects_reply_from("t-1", &serde_json::to_string(&["o/b"]).unwrap(), "o/a");
        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert_eq!(q.expects, vec!["o/b"]);
        assert_eq!(q.outstanding, vec!["o/b"], "re-asked as of now: {q:?}");
        assert!(!q.complete);

        // o/b answers the new declaration → complete.
        post_in_thread(&mut s, "c-1", "t-1", "o/b", "lgtm again");
        assert!(s.thread_quorum("t-1", NOW).unwrap().unwrap().complete);

        // Widen to add a fresh handle → the board re-opens for BOTH: the widening is itself a new
        // declaration, so o/b's answer to the previous one does not carry over to this one.
        s.set_expects_reply_from(
            "t-1",
            &serde_json::to_string(&["o/b", "o/d"]).unwrap(),
            "o/a",
        );
        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert_eq!(q.outstanding, vec!["o/b", "o/d"]);
        assert!(!q.complete, "widening re-opened the board");

        // Dropping the expectation entirely is what closes a board nobody will answer: E=∅ is never
        // `complete` (TB-M2-7), and nothing is outstanding — the reading every other consumer of
        // this derivation (the working-tree release, `--if-unanswered`, `stale`) acts on.
        s.set_expects_reply_from("t-1", "[]", "o/a");
        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert!(q.outstanding.is_empty(), "nobody owes anything: {q:?}");
        assert!(!q.complete);
    }

    #[test]
    fn the_discharge_belongs_to_the_turn_so_a_re_declaration_re_opens_the_debt() {
        // 6j6v.cg8g. `replied` used to be a bare "has this sender EVER written in this thread",
        // which is the right reading for a one-shot review board and the wrong one for a thread
        // that carries SEVERAL TURNS of the same role (the normal case in 6j6v.a71h §2's chain).
        // The discharge is measured from the moment the obligation was declared: the declaration is
        // an LWW register whose `(expects_reply_from_v, expects_reply_from_site)` IS that moment, so
        // re-declaring it — the same set, deliberately — re-opens the debt, and only what is written
        // afterwards settles it.
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b"]);
        post_in_thread(&mut s, "c-1", "t-1", "o/a", "please implement");
        post_in_thread(&mut s, "c-1", "t-1", "o/b", "done");
        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert_eq!(q.replied, vec!["o/b"], "turn 1 is answered");
        assert!(q.outstanding.is_empty());
        assert!(q.complete);

        // Turn 2: the SAME expected set, declared again — nothing about the set changed, only the
        // moment it was asked for. The answer to turn 1 does not answer turn 2.
        s.set_expects_reply_from("t-1", &serde_json::to_string(&["o/b"]).unwrap(), "o/a");
        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert_eq!(q.expects, vec!["o/b"], "the set is unchanged");
        assert!(
            q.replied.is_empty(),
            "the re-declaration re-opened the debt: {q:?}"
        );
        assert_eq!(q.outstanding, vec!["o/b"]);
        assert!(!q.complete, "turn 2 is unanswered: {q:?}");

        // And answering the new turn discharges it, exactly as the first one was discharged.
        post_in_thread(&mut s, "c-1", "t-1", "o/b", "done again");
        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert_eq!(q.replied, vec!["o/b"]);
        assert!(q.outstanding.is_empty());
        assert!(q.complete, "turn 2 is answered: {q:?}");
    }

    #[test]
    fn what_was_written_before_the_obligation_existed_does_not_discharge_it() {
        // The other half of 6j6v.cg8g's first acceptance point, and the sharper one: "only what was
        // written AFTERWARDS counts". A handle that was already talking in the thread when it was
        // asked for something owes that something — its earlier words cannot answer a question that
        // had not been put yet.
        let mut s = ChatStore::open_in_memory(1);
        let root = ThreadRoot {
            origin: "o".into(),
            channel_id: "c-1".into(),
            opener: "o/a".into(),
            created: "2026-07-10T00:00:00Z".into(),
            parent: None,
        };
        s.open_thread("t-1", &root, "o/a");
        post_in_thread(&mut s, "c-1", "t-1", "o/b", "just chatting");
        s.set_expects_reply_from("t-1", &serde_json::to_string(&["o/b"]).unwrap(), "o/a");

        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert!(q.replied.is_empty(), "the chat was not an answer: {q:?}");
        assert_eq!(q.outstanding, vec!["o/b"]);
        assert!(!q.complete);
    }

    #[test]
    fn the_watermark_comparison_is_strictly_after_the_declaration_on_both_axes() {
        // The BOUNDARY of `(m.lamport, m.site) > (expects_reply_from_v, expects_reply_from_site)`,
        // pinned so that `>` silently becoming `>=` — or the site half being dropped — reds here
        // (fix round 1 of 6j6v.cg8g; nothing failed on either mutation before).
        //
        // The three cases are placed by DIRECT INSERT rather than through `post_message`, and that is
        // the only honest way to reach them: a replica stamps strictly increasing lamports, so the
        // exact tie — same lamport AND same site as the declaration — cannot be produced by writing
        // through the store at all. It is the one input that separates `>` from `>=`, so it has to be
        // constructed. The equal-lamport-different-site pair pins the second component of the
        // comparison, which a `m.lamport > v` mutation would silently drop.
        let s = ChatStore::open_in_memory(1);
        s.connection()
            .execute_batch(
                // The declaration sits at (lamport 10, site 5).
                "INSERT INTO threads(thread_id, origin, channel_id, opener, created,
                                     expects_reply_from, expects_reply_from_v, expects_reply_from_site)
                 VALUES('t-1','o','c-1','o/a','2026-07-10T00:00:00Z',
                        '[\"o/tie\",\"o/lower-site\",\"o/higher-site\"]', 10, 5);
                 INSERT INTO messages(message_id, origin, channel_id, sender, kind, priority,
                                      disposition, thread_id, refs, body, created, lamport, site)
                 VALUES
                   -- exactly the declaration's own position: NOT after it (this is the `>=` tripwire)
                   ('m-tie','o','c-1','o/tie','report','normal','in_turn','t-1','{}','tie','',10,5),
                   -- same lamport, lower site: before it
                   ('m-lo','o','c-1','o/lower-site','report','normal','in_turn','t-1','{}','lo','',10,4),
                   -- same lamport, higher site: after it
                   ('m-hi','o','c-1','o/higher-site','report','normal','in_turn','t-1','{}','hi','',10,6);",
            )
            .unwrap();
        vouch_for(&s, &[(10, 4), (10, 5), (10, 6)]);

        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert_eq!(
            q.replied,
            vec!["o/higher-site"],
            "only a message strictly after the declaration discharges it: {q:?}"
        );
        assert_eq!(
            q.outstanding,
            vec!["o/tie", "o/lower-site"],
            "the tie is NOT a discharge (`>`, not `>=`), and neither is a lower site: {q:?}"
        );
    }

    #[test]
    fn the_discharge_never_looks_at_the_message_kind_so_escalation_needs_no_special_case() {
        // 6j6v.cg8g's fifth acceptance point, held as a test so nobody later "fixes" it: an
        // escalating answer ("I cannot do this") discharges the TURN exactly as a finished one does.
        // The session ends either way; what BECOMES of the escalation is the channel supervisor's
        // decision (6j6v.hq71) and the claim on the working copy is a separate question (6j6v.a71h's
        // successor item), neither of which this predicate may anticipate.
        //
        // RETARGETED BY 6j6v.wt37 onto the REAL marker. This test was written by 6j6v.cg8g before
        // `reply --escalate` existed, against `MessageKind::Decision` as a stand-in ("the nearest
        // thing this vocabulary has to a terminal answer that is not a plain report") and over the
        // WHOLE enum, precisely so the marker wt37 would eventually pick could not land unpinned.
        // wt37's marker is now `MessageKind::Escalation` — the carrier is still `messages.kind`, so
        // there is still no envelope field — and it is in the set below by construction rather than
        // by anyone remembering to add it (see `every_message_kind`). The ASSERTION IS UNCHANGED:
        // every kind, escalation included, discharges the turn identically.
        //
        // The set comes from [`every_message_kind`], which is EXHAUSTIVENESS-CHECKED (fix round 1):
        // a variant added later cannot land here unpinned, because adding it is a compile error
        // until its author has said where it belongs.
        for kind in every_message_kind() {
            let mut s = ChatStore::open_in_memory(1);
            seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b"]);
            post_in_thread_of_kind(&mut s, "c-1", "t-1", "o/b", "I cannot do this", kind);
            let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
            assert_eq!(
                q.replied,
                vec!["o/b"],
                "kind {kind:?} discharges the turn like any other: {q:?}"
            );
            assert!(q.outstanding.is_empty(), "kind {kind:?}: {q:?}");
            assert!(q.complete, "kind {kind:?}: {q:?}");
        }
    }

    #[test]
    fn the_answer_to_this_turn_is_escalating_or_not_and_the_question_is_neither() {
        // 6j6v.wt37 acceptance point 5, at the derivation 6j6v.1xw1 (Task 8) will call: THE CLAIM.
        // Deliberately a different question from the one above it — that one proves the discharge
        // IGNORES the kind, this one proves the kind is READABLE. Both hold at once; conflating them
        // builds either a lease that never releases or one that gives up at every question.
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b"]);

        // Nobody has answered this turn: NOT "not escalating".
        assert_eq!(
            s.last_reply_escalated("t-1").unwrap(),
            None,
            "no answer yet is not the same as a non-escalating answer"
        );

        // A plain result closes it.
        post_in_thread_of_kind(&mut s, "c-1", "t-1", "o/b", "done", MessageKind::Report);
        assert_eq!(s.last_reply_escalated("t-1").unwrap(), Some(false));

        // Turn two: asked again, and answered with "I cannot".
        let erf = serde_json::to_string(&["o/b"]).unwrap();
        s.set_expects_reply_from("t-1", &erf, "o/a");
        assert_eq!(
            s.last_reply_escalated("t-1").unwrap(),
            None,
            "the re-declaration re-opens the question: turn one's answer is not turn two's"
        );
        post_in_thread_of_kind(
            &mut s,
            "c-1",
            "t-1",
            "o/b",
            "I cannot do this",
            MessageKind::Escalation,
        );
        assert_eq!(s.last_reply_escalated("t-1").unwrap(), Some(true));

        // Turn three: a real FOLLOW-UP QUESTION is NOT an escalation (acceptance point 3). The two
        // must not merge — `question` is "what do you mean by X?" from somebody who can still do the
        // work, `escalation` is "I cannot do this".
        s.set_expects_reply_from("t-1", &erf, "o/a");
        post_in_thread_of_kind(
            &mut s,
            "c-1",
            "t-1",
            "o/b",
            "what do you mean by X?",
            MessageKind::Question,
        );
        assert_eq!(
            s.last_reply_escalated("t-1").unwrap(),
            Some(false),
            "a follow-up question is not an escalation"
        );
    }

    #[test]
    fn the_escalation_read_ignores_a_drive_by_and_answers_none_for_an_unknown_thread() {
        // The same §2.2 rule `latest_reply_message_id` applies: a "reply" is a message from a handle
        // the thread EXPECTS. A bystander posting an escalation into somebody else's thread does not
        // hold their claim open, or any passer-by could.
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b"]);
        post_in_thread_of_kind(
            &mut s,
            "c-1",
            "t-1",
            "o/nosy",
            "I cannot do this",
            MessageKind::Escalation,
        );
        assert_eq!(
            s.last_reply_escalated("t-1").unwrap(),
            None,
            "a non-expected sender is not an answer to this turn"
        );

        // The opener's own request is not an answer either, even in the same thread.
        post_in_thread_of_kind(
            &mut s,
            "c-1",
            "t-1",
            "o/a",
            "still waiting",
            MessageKind::Escalation,
        );
        assert_eq!(s.last_reply_escalated("t-1").unwrap(), None);

        assert_eq!(s.last_reply_escalated("t-nope").unwrap(), None);
    }

    #[test]
    fn stale_is_clock_gated_and_never_set_on_a_complete_thread() {
        // §4.3: stale = deadline≠NULL ∧ now>deadline ∧ outstanding≠∅. It flips only after the
        // deadline passes, only while something is outstanding, and NEVER on a complete thread.
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b"]);
        s.set_deadline("t-1", "2026-07-15T00:00:00Z", "o/a");

        // Before the deadline: not stale even though o/b is outstanding.
        let before = s
            .thread_quorum("t-1", "2026-07-11T00:00:00Z")
            .unwrap()
            .unwrap();
        assert!(!before.stale, "now < deadline → not stale");
        assert_eq!(before.deadline.as_deref(), Some("2026-07-15T00:00:00Z"));

        // After the deadline, still outstanding: stale, but NOT complete.
        let after = s
            .thread_quorum("t-1", "2026-07-20T00:00:00Z")
            .unwrap()
            .unwrap();
        assert!(after.stale, "now > deadline ∧ outstanding≠∅ → stale");
        assert!(!after.complete, "stale never completes a thread");

        // o/b replies → complete, and a complete thread is never stale even past the deadline.
        post_in_thread(&mut s, "c-1", "t-1", "o/b", "done");
        let done = s
            .thread_quorum("t-1", "2026-07-20T00:00:00Z")
            .unwrap()
            .unwrap();
        assert!(done.complete);
        assert!(
            !done.stale,
            "outstanding=∅ → never stale (complete ⇒ not stale)"
        );
    }

    #[test]
    fn a_thread_without_a_deadline_is_never_stale() {
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b"]);
        // Far-future "now": still not stale, because deadline is NULL (wait indefinitely).
        let q = s
            .thread_quorum("t-1", "2099-01-01T00:00:00Z")
            .unwrap()
            .unwrap();
        assert_eq!(q.deadline, None);
        assert!(!q.stale, "no deadline → never stale");
    }

    #[test]
    fn thread_quorums_bulk_returns_a_deterministic_channel_then_id_order() {
        // The bulk read (no N+1) returns threads ordered by (channel_id, thread_id) regardless of
        // the input id order — the deterministic board order the CLI/facade render.
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-2", "c-1", "o/a", &["o/b"]);
        seed_thread(&mut s, "t-1", "c-2", "o/a", &["o/b", "o/c"]);
        seed_thread(&mut s, "t-3", "c-1", "o/a", &[]);
        post_in_thread(&mut s, "c-1", "t-2", "o/b", "done");
        // Pass ids scrambled; expect (channel_id, thread_id) order: c-1/t-2, c-1/t-3, c-2/t-1.
        let qs = s.thread_quorums(&["t-1", "t-3", "t-2"], NOW).unwrap();
        assert_eq!(
            qs.iter().map(|q| q.thread_id.as_str()).collect::<Vec<_>>(),
            vec!["t-2", "t-3", "t-1"],
            "ordered by channel then thread id, not by input order"
        );
        assert!(qs[0].complete, "t-2: its single expected handle replied");
        assert!(!qs[1].complete, "t-3: empty E is never complete");
        assert_eq!(
            qs[2].outstanding,
            vec!["o/b", "o/c"],
            "t-1: both still outstanding"
        );
    }

    #[test]
    fn member_thread_ids_enumerates_only_the_consumers_channels_channel_then_id_sorted() {
        // The `threads list` enumeration: threads in channels the consumer is a member of, in
        // (channel_id, thread_id) order — feeding the bulk quorum read (no N+1). A thread in a
        // channel the consumer is NOT a member of never appears; `--channel` narrows to one channel.
        let mut s = ChatStore::open_in_memory(1);
        s.add_member("c-1", "o/reader", "o/opener");
        s.add_member("c-2", "o/reader", "o/opener");
        // c-3: reader is NOT a member — its thread must be excluded.
        seed_thread(&mut s, "t-2", "c-1", "o/opener", &["o/b"]);
        seed_thread(&mut s, "t-1", "c-2", "o/opener", &["o/b"]);
        seed_thread(&mut s, "t-3", "c-1", "o/opener", &[]);
        seed_thread(&mut s, "t-9", "c-3", "o/opener", &["o/b"]);
        // All member channels: (c-1,t-2), (c-1,t-3), (c-2,t-1) — c-3/t-9 excluded (non-member).
        assert_eq!(
            s.member_thread_ids("o/reader", None).unwrap(),
            vec!["t-2", "t-3", "t-1"]
        );
        // Restricted to one channel.
        assert_eq!(
            s.member_thread_ids("o/reader", Some("c-1")).unwrap(),
            vec!["t-2", "t-3"]
        );
        // A non-member sees nothing.
        assert!(s.member_thread_ids("o/outsider", None).unwrap().is_empty());
    }

    #[test]
    fn opener_wake_surfaces_a_completed_thread_until_the_window_moves_past_it() {
        // §4.4: a thread the opener C opened that is complete inside C's window surfaces in C's
        // wake (with the board + replies). The gate was C's channel read cursor until nxf
        // 6j6v.2hx9 and is the caller's previous session end since (nxf 6j6v.4d2z then removed the
        // cursor outright).
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b", "o/c"]);
        post_in_thread(&mut s, "c-1", "t-1", "o/a", "please review"); // the opener's own request
        let bref = post_in_thread(&mut s, "c-1", "t-1", "o/b", "lgtm");
        let cref = post_in_thread(&mut s, "c-1", "t-1", "o/c", "approved");
        // The completing reply is the LAST-POSTED expected reply, full stop — and saying it that
        // plainly is what 6j6v.fcv4 bought. This line used to read `std::cmp::max(bref, cref)`,
        // because `latest_reply_message_id` took `MAX(message_id)` and two ULIDs minted in the same
        // millisecond sort by their random tail: the test could not name its own expected value and
        // had to re-derive the implementation's coin toss. Now that the read orders by
        // `(lamport, site, message_id)` like every other thread read, the causally last post wins
        // deterministically, and the expectation is a constant instead of a computation.
        let completing = cref;

        let wake = s.opener_wake("o/a", NOW, Some(SINCE)).unwrap();
        assert!(wake.stale.is_empty());
        assert_eq!(
            wake.complete.len(),
            1,
            "the completed board surfaces to its opener"
        );
        let ct = &wake.complete[0];
        assert_eq!(ct.thread_id, "t-1");
        assert_eq!(ct.channel_id.as_deref(), Some("c-1"));
        assert_eq!(ct.expects, vec!["o/b", "o/c"]);
        assert_eq!(
            ct.completing_message_id, completing,
            "the causally last expected reply is the completing one"
        );
        // The board carries the request + both replies (in causal order, 0b1m); exactly the
        // completing reply — the one whose instant puts the board in the window — is flagged.
        assert_eq!(ct.messages.len(), 3);
        let completers: Vec<&str> = ct
            .messages
            .iter()
            .filter(|m| m.completes)
            .map(|m| m.message_id.as_str())
            .collect();
        assert_eq!(
            completers,
            vec![completing.as_str()],
            "only the completing reply is flagged"
        );
        // And the OTHER expected reply is on the board unflagged WHATEVER its id happens to be —
        // the half that reds when the flag follows the id instead of the clock, which is the whole
        // of 6j6v.fcv4 and is invisible on a run where the two ULID tails happen to agree with the
        // post order.
        assert!(
            ct.messages
                .iter()
                .any(|m| m.message_id == bref && !m.completes),
            "the earlier expected reply is on the board and not flagged: {:?}",
            ct.messages
        );

        // A NON-opener is never woken by someone else's board.
        assert!(
            s.opener_wake("o/b", NOW, Some(SINCE))
                .unwrap()
                .complete
                .is_empty(),
            "only the opener is woken"
        );

        // **What takes a board off the notice is the window moving past it** (nxf 6j6v.2hx9), and
        // nothing else. An assertion stood here that acked past the completing reply and asserted
        // the board STAYED — the last trace of the rule this replaced, where a board dropped off
        // exactly when the opener's channel cursor passed it. That was the defect (nothing ever
        // acked, so the notice never cleared and a session start accumulated finished work without
        // bound), and the ack it drove is itself gone since nxf 6j6v.4d2z, so there is no longer a
        // way to even attempt it. The positive half is what remains and it is the whole promise:
        // a caller whose previous session ended AFTER this board completed has already been told,
        // and is not told again.
        assert!(
            s.opener_wake("o/a", NOW, Some("2026-07-12T00:00:00Z"))
                .unwrap()
                .complete
                .is_empty(),
            "a board that finished before the previous session ended is behind the window"
        );
        assert_eq!(
            s.opener_wake("o/a", NOW, Some(SINCE)).unwrap().complete[0].finished_at,
            POSTED_AT,
            "and the instant it finished at rides the notice"
        );

        // A caller with NO previous session end — a human at a terminal, a persona's first start —
        // is not served by this read at all, rather than served everything ever.
        assert!(
            s.opener_wake("o/a", NOW, None).unwrap().is_empty(),
            "no watermark, no window"
        );
    }

    #[test]
    fn a_completing_reply_with_no_recorded_instant_is_left_out_rather_than_let_in() {
        // The legacy branch of the window (nxf 6j6v.2hx9; review of that branch, Test Quality). A
        // message row written before `created` was stamped on every post has no instant to compare,
        // and the direction of the skip is the point: letting it IN would put a board from before
        // the column existed on every session start, which is the exact unbounded notice this item
        // removes. Such a row is by construction older than any watermark, so leaving it out is not
        // merely safe — it is right.
        let s = ChatStore::open_in_memory(1);
        s.connection()
            .execute_batch(
                "INSERT INTO threads(thread_id, origin, channel_id, opener, created,
                                     expects_reply_from)
                 VALUES('t-1','o','c-1','o/a','2026-07-10T00:00:00Z','[\"o/b\"]');
                 INSERT INTO messages(message_id, origin, channel_id, sender, kind, priority,
                                      disposition, thread_id, refs, body, created, lamport, site)
                 VALUES
                   ('m-req','o','c-1','o/a','question','normal','in_turn','t-1','{}','please review',
                    NULL,4,1),
                   ('m-ans','o','c-1','o/b','report','normal','in_turn','t-1','{}','lgtm',
                    NULL,5,1);",
            )
            .unwrap();
        vouch_for(&s, &[(4, 1), (5, 1)]);
        assert!(
            s.thread_quorum("t-1", NOW).unwrap().unwrap().complete,
            "the board IS complete — what is missing is only when it became so"
        );
        assert!(
            s.opener_wake("o/a", NOW, Some(SINCE))
                .unwrap()
                .complete
                .is_empty(),
            "…and with no instant on the completing reply it stays out of the window"
        );
    }

    #[test]
    fn opener_wake_stale_appears_only_after_the_deadline_and_is_advisory() {
        // §4.3/§4.4: a stale board opened by C surfaces ONLY after now>deadline, as an advisory
        // line with the outstanding handles; it self-clears when the board completes.
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b"]);
        s.set_deadline("t-1", "2026-07-15T00:00:00Z", "o/a");
        post_in_thread(&mut s, "c-1", "t-1", "o/a", "please review");

        // Before the deadline: nothing (not complete, not yet stale).
        let before = s
            .opener_wake("o/a", "2026-07-11T00:00:00Z", Some(SINCE))
            .unwrap();
        assert!(before.complete.is_empty() && before.stale.is_empty());

        // After the deadline, still outstanding: a stale advisory with the outstanding handle.
        let after = s
            .opener_wake("o/a", "2026-07-20T00:00:00Z", Some(SINCE))
            .unwrap();
        assert!(after.complete.is_empty());
        assert_eq!(after.stale.len(), 1);
        assert_eq!(after.stale[0].thread_id, "t-1");
        assert_eq!(after.stale[0].outstanding, vec!["o/b"]);
        assert_eq!(
            after.stale[0].deadline.as_deref(),
            Some("2026-07-15T00:00:00Z")
        );

        // o/b replies → complete; stale clears even past the deadline (complete ⇒ not stale) and the
        // board now surfaces as finished-inside-the-window instead.
        post_in_thread(&mut s, "c-1", "t-1", "o/b", "done");
        let done = s
            .opener_wake("o/a", "2026-07-20T00:00:00Z", Some(SINCE))
            .unwrap();
        assert!(done.stale.is_empty(), "a completed board is never stale");
        assert_eq!(done.complete.len(), 1);
    }

    #[test]
    fn opener_wake_surfaces_a_narrowed_board_that_completes() {
        // §3.2/§4.4: narrowing `expects_reply_from` re-derives a stuck board, and once the narrowed
        // set answers, the wake surfaces it — the in-core unstick path is visible in the wake.
        //
        // Since 6j6v.cg8g the narrowing does not complete the board BY ITSELF (it re-asks the handle
        // it names; see `re_declaring_expectations_re_asks_everyone_the_new_declaration_names` for
        // why one register cannot mean two things), so the unstick is now two beats rather than one:
        // narrow, then be answered. The wake path under test — a completed board surfacing with the
        // right completing message — is unchanged, and the completing message is now the answer to
        // the CURRENT declaration rather than one from before it.
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b", "o/c"]);
        post_in_thread(&mut s, "c-1", "t-1", "o/a", "please review");
        post_in_thread(&mut s, "c-1", "t-1", "o/b", "lgtm"); // o/c never replies
        assert!(
            s.opener_wake("o/a", NOW, Some(SINCE))
                .unwrap()
                .complete
                .is_empty(),
            "o/c outstanding → no wake"
        );

        // Drop the non-responder: the board now asks o/b alone, as of now.
        s.set_expects_reply_from("t-1", &serde_json::to_string(&["o/b"]).unwrap(), "o/a");
        assert!(
            s.opener_wake("o/a", NOW, Some(SINCE))
                .unwrap()
                .complete
                .is_empty(),
            "the re-declaration is a question, not an answer"
        );

        // o/b answers it → the board completes and wakes the opener.
        let bref = post_in_thread(&mut s, "c-1", "t-1", "o/b", "still lgtm");
        let wake = s.opener_wake("o/a", NOW, Some(SINCE)).unwrap();
        assert_eq!(
            wake.complete.len(),
            1,
            "the narrowed board completes and surfaces"
        );
        assert_eq!(wake.complete[0].completing_message_id, bref);
        assert_eq!(wake.complete[0].expects, vec!["o/b"]);
    }

    #[test]
    fn opener_wake_orders_completed_boards_by_channel_then_thread() {
        // Deterministic order (channel + thread ULID), inherited from the bulk quorum read.
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-2", "c-1", "o/a", &["o/b"]);
        seed_thread(&mut s, "t-1", "c-2", "o/a", &["o/b"]);
        post_in_thread(&mut s, "c-1", "t-2", "o/b", "done c1");
        post_in_thread(&mut s, "c-2", "t-1", "o/b", "done c2");
        let wake = s.opener_wake("o/a", NOW, Some(SINCE)).unwrap();
        assert_eq!(
            wake.complete
                .iter()
                .map(|c| c.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["t-2", "t-1"],
            "c-1/t-2 before c-2/t-1"
        );
    }

    #[test]
    fn latest_reply_message_id_takes_the_causally_newest_reply_not_the_greatest_id() {
        // 6j6v.fcv4 — the one message-ordering read site 0b1m's sweep missed, because it is a
        // `MAX()` and not an `ORDER BY`. Two replies from the same expected handle minted in the
        // same millisecond differ only in the ULID's random tail, so `MAX(message_id)` picks
        // between them by coin toss; on `main` that reddened
        // `opener_wake_surfaces_a_narrowed_board_that_completes` in 8 of 60 single runs.
        //
        // DIRECT INSERT, for the same reason `the_watermark_comparison_...` uses one: the input
        // that separates the two orders — causally LATER but lexicographically SMALLER — cannot be
        // produced by writing through the store, because a ULID's tail is random and a run that
        // happens to invert it is exactly the 13 % this test exists to make deterministic.
        let s = ChatStore::open_in_memory(1);
        s.connection()
            .execute_batch(
                "INSERT INTO threads(thread_id, origin, channel_id, opener, created,
                                     expects_reply_from)
                 VALUES('t-1','o','c-1','o/a','2026-07-10T00:00:00Z','[\"o/b\"]');
                 INSERT INTO messages(message_id, origin, channel_id, sender, kind, priority,
                                      disposition, thread_id, refs, body, created, lamport, site)
                 VALUES
                   -- posted FIRST (lamport 5), and the GREATEST id: what `MAX()` used to answer
                   ('m-zzz','o','c-1','o/b','report','normal','in_turn','t-1','{}','lgtm','',5,1),
                   -- posted SECOND (lamport 6), and the SMALLEST id: the actual latest reply
                   ('m-aaa','o','c-1','o/b','report','normal','in_turn','t-1','{}','and again','',6,1);",
            )
            .unwrap();
        vouch_for(&s, &[(5, 1), (6, 1)]);

        assert_eq!(
            s.latest_reply_message_id("t-1").unwrap().as_deref(),
            Some("m-aaa"),
            "the causally newest reply, not the lexicographically greatest id"
        );
    }

    #[test]
    fn a_board_that_expects_its_own_opener_is_the_one_shape_the_wake_would_carry_as_a_debt() {
        // **The boundary of the claim nxf 6j6v.1gm9 rests on** (PR #385 review, Integrity #3).
        //
        // That ticket removed the session-start block on the finding that the wake is the OPENER's
        // view — so "a thread awaiting a reply from THIS session" was never in it, and nothing of
        // that fact was lost. The finding is correct, and this pins the ONE shape that would make it
        // false: a board whose opener is also one of its own `expects_reply_from`. Then the debt and
        // the wake key are the same handle, and past its deadline the caller's own wake says
        // "waiting on: yourself".
        //
        // **It is unreachable through every production write, and that is a WRITE-side invariant,
        // not a property of this derivation** — which is exactly why it is worth a test rather than
        // a sentence. The two doors are `orchestration::ensure_dm_channel`, which refuses `a == b`
        // outright, and the channel supervisor, whose re-declares always name a
        // `RESERVED_HANDLE_PREFIX` identity (`__channel__`, `__delivered__`) that
        // `definitions::validate_role_handle` forbids any declared persona from colliding with.
        // Both guards were written for unrelated reasons (DM sanity, reserved-identity collision),
        // so the invariant is EMERGENT, and a future commissioning path that goes through neither
        // door would reopen the gap silently.
        //
        // So the shape is built here by hand, through the raw store, which is the only way to build
        // it at all. What this asserts is not that the derivation is wrong — it does exactly what
        // its own contract says — but that the claim above depends on nobody ever minting this, and
        // that the day someone does, the fix belongs at the write and not here.
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "kind", "group", "o/a");
        s.add_member("c-1", "o/a", "o/a");
        let root = ThreadRoot {
            origin: "o".into(),
            channel_id: "c-1".into(),
            opener: "o/a".into(),
            created: "2026-07-10T00:00:00Z".into(),
            parent: None,
        };
        s.open_thread("t-self", &root, "o/a");
        s.set_expects_reply_from("t-self", "[\"o/a\"]", "o/a");
        s.set_deadline("t-self", "2026-07-11T00:00:00Z", "o/a");

        let wake = s
            .opener_wake("o/a", "2026-07-12T00:00:00Z", Some(SINCE))
            .unwrap();
        assert_eq!(
            wake.stale.len(),
            1,
            "the derivation surfaces it — the opener key and the debt are the same handle here"
        );
        assert_eq!(wake.stale[0].outstanding, vec!["o/a".to_string()]);
        assert!(
            wake.complete.is_empty(),
            "nobody answered, so it is a debt and not a result"
        );

        // …and the shape a real board has — opener and expected are different handles — puts the
        // debt where every production write puts it: on somebody else.
        let mut real = ChatStore::open_in_memory(1);
        real.set_channel_field("c-1", "kind", "group", "o/a");
        real.add_member("c-1", "o/a", "o/a");
        real.open_thread("t-ok", &root, "o/a");
        real.set_expects_reply_from("t-ok", "[\"o/b\"]", "o/a");
        real.set_deadline("t-ok", "2026-07-11T00:00:00Z", "o/a");
        let wake = real
            .opener_wake("o/a", "2026-07-12T00:00:00Z", Some(SINCE))
            .unwrap();
        assert_eq!(wake.stale[0].outstanding, vec!["o/b".to_string()]);
        assert!(
            real.opener_wake("o/b", "2026-07-12T00:00:00Z", Some(SINCE))
                .unwrap()
                .is_empty(),
            "and the one who OWES the reply is told nothing by their own wake — the whole finding"
        );
    }

    #[test]
    fn opener_wake_annotates_the_causally_newest_reply_as_completing() {
        // The same inversion one level up, where it is visible to a supervisor: the wake's
        // `completing_message_id` and the `completes` flag on the board must both land on the reply
        // that actually finished the turn. This is the shape CI saw fail (run 32204232669) —
        // expected `m-01M0BSHSPJKPE5NBYW034YD7JM`, got `m-01M0BSHSPJP9J0XEKKQJBAMF1Q`: same
        // millisecond prefix, different random tail.
        let s = ChatStore::open_in_memory(1);
        s.connection()
            .execute_batch(
                "INSERT INTO threads(thread_id, origin, channel_id, opener, created,
                                     expects_reply_from)
                 VALUES('t-1','o','c-1','o/a','2026-07-10T00:00:00Z','[\"o/b\"]');
                 INSERT INTO messages(message_id, origin, channel_id, sender, kind, priority,
                                      disposition, thread_id, refs, body, created, lamport, site)
                 VALUES
                   -- A real `created` on both, because this test reads through `opener_wake`, whose
                   -- window is derived from exactly that column (nxf 6j6v.2hx9).
                   ('m-zzz','o','c-1','o/b','report','normal','in_turn','t-1','{}','lgtm',
                    '2026-07-11T00:00:00Z',5,1),
                   ('m-aaa','o','c-1','o/b','report','normal','in_turn','t-1','{}','and again',
                    '2026-07-11T00:00:00Z',6,1);",
            )
            .unwrap();
        vouch_for(&s, &[(5, 1), (6, 1)]);

        let wake = s.opener_wake("o/a", NOW, Some(SINCE)).unwrap();
        assert_eq!(wake.complete.len(), 1, "the board completed: {wake:?}");
        assert_eq!(
            wake.complete[0].completing_message_id, "m-aaa",
            "the completing reply is the causally last one"
        );
        assert_eq!(
            wake.complete[0]
                .messages
                .iter()
                .filter(|m| m.completes)
                .map(|m| m.message_id.as_str())
                .collect::<Vec<_>>(),
            vec!["m-aaa"],
            "and the flag rides the same message, not the greatest id"
        );
    }

    #[test]
    fn thread_opening_message_is_the_causally_first_one_not_the_lexicographically_smallest() {
        // nxf 6j6v.px98 review, Test Quality #4. `thread_opening_message`'s whole contract is that
        // "first" means what it means to `messages_in_thread` — the reader whose filter it feeds —
        // and the one input that can tell the two apart is a same-millisecond pair whose ULID tails
        // disagree with the post order. Constructed rather than raced, exactly as
        // `opener_wake_annotates_the_causally_newest_reply_as_completing` above constructs it.
        let s = ChatStore::open_in_memory(1);
        s.connection()
            .execute_batch(
                "INSERT INTO messages(message_id, origin, channel_id, sender, kind, priority,
                                      disposition, thread_id, refs, body, created, lamport, site)
                 VALUES
                   ('m-zzz','o','c-1','o/opener','question','normal','in_turn','t-1','{}','ask','',5,1),
                   ('m-aaa','o','c-1','o/member','report','normal','in_turn','t-1','{}','answer','',6,1);",
            )
            .unwrap();

        assert_eq!(
            s.thread_opening_message("t-1").unwrap(),
            Some(("m-zzz".to_string(), "o/opener".to_string())),
            "the causally FIRST message opens the thread, though `m-aaa` sorts before it"
        );
        // …and it is the same message the reader that filters a board calls first, which is the
        // agreement the visibility filter rests on: `facade::search` judges a hit to BE the opening
        // message by this id, while `facade::filter_board_messages` judges by position in this list.
        let assembled = s.messages_in_thread("t-1").unwrap();
        assert_eq!(
            (assembled[0].message_id.clone(), assembled[0].sender.clone()),
            s.thread_opening_message("t-1").unwrap().unwrap(),
            "the point read and the assembled thread name the same opening message"
        );
    }

    #[test]
    fn thread_opening_message_is_none_for_a_thread_with_no_message() {
        // The `None` arm, which is the one the search filter treats as the closed direction: a hit
        // whose board has no opening message can match nobody's "is this the request?".
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-empty", "c-1", "o/a", &["o/b"]);
        assert_eq!(
            s.thread_opening_message("t-empty").unwrap(),
            None,
            "a thread row with no message opens nothing"
        );
        assert_eq!(
            s.thread_opening_message("t-never-existed").unwrap(),
            None,
            "and so does a thread id this workspace has never carried"
        );
    }

    #[test]
    fn latest_reply_message_id_counts_only_expected_handles() {
        // §2.2: a "reply" is a message from an EXPECTED handle; the opener's own request and a
        // non-expected drive-by never count as the latest reply — even with a higher id.
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b"]);
        assert!(
            s.latest_reply_message_id("t-1").unwrap().is_none(),
            "no expected reply yet"
        );
        post_in_thread(&mut s, "c-1", "t-1", "o/a", "please review"); // opener, not expected
        let bref = post_in_thread(&mut s, "c-1", "t-1", "o/b", "lgtm"); // the one expected reply
        post_in_thread(&mut s, "c-1", "t-1", "o/x", "drive-by"); // non-expected, higher id
        assert_eq!(
            s.latest_reply_message_id("t-1").unwrap().as_deref(),
            Some(bref.as_str()),
            "the drive-by (higher id) does not count; only o/b's reply does"
        );
    }

    #[test]
    fn threads_opened_by_lists_only_the_openers_threads() {
        let mut s = ChatStore::open_in_memory(1);
        seed_thread(&mut s, "t-1", "c-1", "o/a", &["o/b"]);
        seed_thread(&mut s, "t-2", "c-1", "o/z", &["o/b"]);
        let mut mine = s.threads_opened_by("o/a").unwrap();
        mine.sort();
        assert_eq!(mine, vec!["t-1"]);
        assert!(s.threads_opened_by("o/nobody").unwrap().is_empty());
    }

    #[test]
    fn write_helpers_stamp_the_explicit_actor_not_system() {
        // Every mutating helper must stamp the CALLER as op author, never the removed "system"
        // placeholder (review T1 #179). Exercise all of them and assert no op is authored "system".
        //
        // SIX since nxf 6j6v.4d2z, seven before it: `advance_read_cursor` was the seventh and went
        // with the read cursor. The COUNT is asserted rather than the set, so adding a helper
        // without exercising it here reds — and so does removing one, which is what caught this
        // line.
        let mut s = ChatStore::open_in_memory(1);
        let a = "local/alice";
        s.set_channel_field("c-1", "name", "review", a);
        s.set_profile_field("local/qa", "job_title", "QA", a);
        s.add_member("c-1", "local/bob", a);
        s.remove_member("c-1", "local/bob", a);
        let root = ThreadRoot {
            origin: "local".into(),
            channel_id: "c-1".into(),
            opener: a.into(),
            created: String::new(),
            parent: None,
        };
        s.open_thread("t-1", &root, a);
        s.set_expects_reply_from("t-1", "[\"local/bob\"]", a);
        // post_message authors as the envelope sender (the acting agent's qualified handle).
        s.post_message(&env("c-1", "hi", Disposition::InTurn)); // sender "o/a" (from the test `env`)

        // No op is authored by the placeholder…
        let system_ops: i64 = s
            .connection()
            .query_row("SELECT COUNT(*) FROM ops WHERE author='system'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            system_ops, 0,
            "no op carries the removed 'system' placeholder"
        );
        // …and every non-message op carries the explicit caller.
        let caller_ops: i64 = s
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM ops WHERE author=?1 AND target_kind != 'message'",
                [a],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(caller_ops, 6, "all six mutating helpers stamped the caller");
    }

    #[test]
    fn membership_and_message_authorship_are_readable_off_the_op_log_alone() {
        // Forward-compat invariant 5 (6j6v.xsf3), the chat half. Two properties the E4 auth slice
        // (6j6v.6aza) builds "who may read this" and "verify before the agent acts" on, and both
        // have to be true of the OP LOG itself — not of a view, not of a payload a reader has to
        // parse first, because that log is what sync carries and what a signature covers.
        //
        // 1. Channel membership is op-level first-class: joining a channel is an op in the shared
        //    log (`membership` add/remove, an observed-remove OR-set), so "who is in this channel"
        //    is derivable from history on any replica and can become an ACL key.
        // 2. Every message op carries its author on the op.
        let mut s = ChatStore::open_in_memory(1);
        s.add_member("c-1", "o/bob", "o/alice");
        s.post_message(&env("c-1", "hi", Disposition::InTurn)); // sender "o/a"

        let rows: Vec<(String, String, String)> = s
            .connection()
            .prepare("SELECT target_kind, op_type, author FROM ops ORDER BY lamport")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(
            rows,
            vec![
                (
                    "membership".to_string(),
                    "add".to_string(),
                    "o/alice".into()
                ),
                ("message".to_string(), "post".to_string(), "o/a".into()),
            ],
            "membership is its own op kind in the shared log, and both ops name their author"
        );
        assert!(
            rows.iter().all(|(_, _, author)| !author.trim().is_empty()),
            "no chat op reaches the log unattributed"
        );
    }

    #[test]
    fn deterministic_ids_are_sortable_and_stable_under_the_switch() {
        // Golden determinism (spec §2.2): under NXF_DETERMINISTIC_IDS, a minted message id is a
        // zero-padded, lexicographically-sortable m-<seq> so send/read/inbox goldens are byte-stable.
        // The switch is process-global — see `IdSwitchGuard` for what that cost before it was locked.
        let _guard = IdSwitchGuard::acquire();
        std::env::set_var("NXF_DETERMINISTIC_IDS", "1");
        let mut s = ChatStore::open_in_memory(1);
        let m1 = s.post_message(&env("c-1", "one", Disposition::InTurn));
        let m2 = s.post_message(&env("c-1", "two", Disposition::InTurn));
        assert_eq!(m1, "m-00000000000000000000000001");
        assert_eq!(m2, "m-00000000000000000000000002");
        assert!(
            m1 < m2,
            "insertion order == lexicographic order == time order"
        );
    }

    #[test]
    fn channel_and_membership_reads_back_the_cli_predicates() {
        let mut s = ChatStore::open_in_memory(1);
        assert!(!s.channel_exists("c-1").unwrap(), "unknown channel");
        s.set_channel_field("c-1", "kind", "group", "local/alice");
        assert!(s.channel_exists("c-1").unwrap(), "created channel exists");
        assert!(!s.is_member("c-1", "local/bob").unwrap(), "not joined yet");
        s.add_member("c-1", "local/bob", "local/alice");
        assert!(s.is_member("c-1", "local/bob").unwrap(), "joined");
    }

    // ---- the check before an action (nxf 6j6v.pzkb) -------------------------------------------
    //
    // A peer is a second replica: its ops arrive signed with ITS key, which this replica trusts
    // only once told to. `peer_after` catches the peer up first, so what it writes is causally after
    // everything the local replica has declared — the only way an answer can discharge a turn.

    fn peer_after(local: &ChatStore) -> ChatStore {
        let mut peer = ChatStore::open_in_memory(2);
        peer.apply(&local.export());
        peer
    }

    /// The ops the peer itself wrote (its site), in log order.
    fn written_by(peer: &ChatStore) -> Vec<Op> {
        peer.export().into_iter().filter(|o| o.site == 2).collect()
    }

    #[test]
    fn a_reply_an_action_may_not_follow_is_shown_but_discharges_nothing() {
        let mut local = ChatStore::open_in_memory(1);
        seed_thread(&mut local, "t-1", "c-1", "o/a", &["o/b"]);
        let mut peer = peer_after(&local);
        post_in_thread(&mut peer, "c-1", "t-1", "o/b", "done");
        let answer = written_by(&peer);
        local.apply(&answer);

        let q = local.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert!(q.replied.is_empty() && !q.complete, "untrusted: {q:?}");
        assert_eq!(
            local.messages_in_thread("t-1").unwrap().len(),
            1,
            "but shown"
        );
        assert!(local.acting_messages_in_thread("t-1").unwrap().is_empty());

        local.trust_key(peer.key_id(), "peer", "").unwrap();
        let q = local.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert!(q.complete, "trusted, the same reply discharges: {q:?}");
        assert_eq!(
            local.latest_reply_message_id("t-1").unwrap(),
            Some(answer[0].target_id.clone())
        );

        local.distrust_key(peer.key_id()).unwrap();
        let q = local.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert!(!q.complete, "revoked, it stops at once: {q:?}");
        assert_eq!(local.latest_reply_message_id("t-1").unwrap(), None);
    }

    /// A trusted key's reply that arrives without its signature — an old relay, or one that strips
    /// it — or altered on the way, discharges nothing either (spec §2.6).
    #[test]
    fn a_trusted_keys_reply_stripped_or_altered_on_the_way_discharges_nothing() {
        for damage in ["stripped", "altered"] {
            let mut local = ChatStore::open_in_memory(1);
            seed_thread(&mut local, "t-1", "c-1", "o/a", &["o/b"]);
            let mut peer = peer_after(&local);
            local.trust_key(peer.key_id(), "peer", "").unwrap();
            post_in_thread(&mut peer, "c-1", "t-1", "o/b", "done");
            let mut answer = written_by(&peer);
            if damage == "stripped" {
                answer[0].key_id = None;
                answer[0].sig = None;
            } else {
                answer[0].author = "o/c".into();
            }
            local.apply(&answer);
            let q = local.thread_quorum("t-1", NOW).unwrap().unwrap();
            assert!(!q.complete, "{damage}: {q:?}");
        }
    }

    /// An op that changes a local thread's OBLIGATION holds the thread when nobody here can vouch for
    /// it: who owes an answer, and until when, is what every completion and every lapse is computed
    /// from — a deadline planted in the past would end a round, a re-declaration would move its turn.
    #[test]
    fn a_thread_whose_obligation_an_unvouched_op_shaped_is_held_never_complete_nor_stale() {
        let mut local = ChatStore::open_in_memory(1);
        seed_thread(&mut local, "t-1", "c-1", "o/a", &["o/b", "o/c"]);
        let mut peer = peer_after(&local);
        peer.set_expects_reply_from("t-1", &serde_json::to_string(&["o/b"]).unwrap(), "o/a");
        local.apply(&written_by(&peer));
        post_in_thread(&mut local, "c-1", "t-1", "o/b", "done");
        let q = local.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert_eq!(q.replied, ["o/b"], "the answer is counted as ever");
        assert!(
            q.held && !q.complete,
            "but the obligation it answers is unvouched: {q:?}"
        );
        let json = serde_json::to_value(&q).unwrap();
        assert_eq!(json["held"], true, "a held thread says so");

        local.trust_key(peer.key_id(), "peer", "").unwrap();
        let q = local.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert!(
            !q.held && q.complete,
            "vouched for, the round completes: {q:?}"
        );
        let json = serde_json::to_value(&q).unwrap();
        assert!(
            json.get("held").is_none(),
            "and an ordinary thread reads as before"
        );

        let mut local = ChatStore::open_in_memory(1);
        seed_thread(&mut local, "t-1", "c-1", "o/a", &["o/b"]);
        let mut peer = peer_after(&local);
        peer.set_deadline("t-1", "2026-01-01T00:00:00Z", "o/a");
        local.apply(&written_by(&peer));
        let q = local.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert!(
            q.held && !q.stale,
            "a planted past deadline does not end it: {q:?}"
        );
    }

    /// A second `open` for a local thread's id, from nobody this replica can vouch for, holds the
    /// thread — fail-closed, whichever of the two the root columns happen to show.
    #[test]
    fn an_unvouched_open_for_a_threads_id_holds_it() {
        let mut local = ChatStore::open_in_memory(1);
        seed_thread(&mut local, "t-1", "c-1", "o/a", &["o/b"]);
        post_in_thread(&mut local, "c-1", "t-1", "o/b", "done");
        assert!(local.thread_quorum("t-1", NOW).unwrap().unwrap().complete);
        let mut peer = ChatStore::open_in_memory(2);
        seed_thread(&mut peer, "t-1", "c-9", "o/x", &[]);
        let open: Vec<Op> = written_by(&peer)
            .into_iter()
            .filter(|o| o.op_type == OP_OPEN)
            .collect();
        local.apply(&open);
        let q = local.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert!(q.held && !q.complete, "{q:?}");
    }

    /// A chat's machine reads back with whether its op may carry an action (nxf 6j6v.1c6k): this
    /// replica's own write acts, a peer's acts only once its key is trusted, and the peer's later
    /// hand-over wins the register either way — the value converges, the authority does not follow it.
    #[test]
    fn a_chats_machine_reads_back_with_whether_its_op_acts() {
        let mut local = ChatStore::open_in_memory(1);
        assert_eq!(local.thread_machine("t-1").unwrap(), None);
        seed_thread(&mut local, "t-1", "c-1", "o/a", &["o/b"]);
        assert_eq!(local.thread_machine("t-1").unwrap(), None);
        local.set_thread_machine("t-1", "01jlaptop", "o/a");
        assert_eq!(
            local.thread_machine("t-1").unwrap(),
            Some(crate::machine::ThreadMachine {
                machine_id: "01jlaptop".into(),
                acts: true
            })
        );

        let mut peer = peer_after(&local);
        peer.set_thread_machine("t-1", "01jstudio", "o/a");
        local.apply(&written_by(&peer));
        assert_eq!(
            local.thread_machine("t-1").unwrap(),
            Some(crate::machine::ThreadMachine {
                machine_id: "01jstudio".into(),
                acts: false
            })
        );
        local.trust_key(peer.key_id(), "peer", "").unwrap();
        assert!(local.thread_machine("t-1").unwrap().unwrap().acts);
    }

    /// The return address names a session on THIS machine that a reply resumes — never one a
    /// message nobody here can vouch for chose.
    #[test]
    fn a_message_an_action_may_not_follow_has_no_return_address() {
        let local = ChatStore::open_in_memory(1);
        let mut peer = peer_after(&local);
        peer.set_channel_field("c-1", "kind", "group", "local/a");
        let planted = peer.post_message(&MessageEnvelope {
            origin: "local".into(),
            channel_id: "c-1".into(),
            sender: "local/pm".into(),
            kind: MessageKind::Task,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: None,
            refs: Refs {
                session_id: Some("s-local-coder".into()),
                ..Refs::default()
            },
            body: "order".into(),
        });
        let mut local = local;
        local.apply(&written_by(&peer));
        assert_eq!(local.message_return_address(&planted).unwrap(), None);
        let seen = local.message_provenance(&planted).unwrap().unwrap();
        assert_eq!(
            seen.provenance,
            nxs_foundation::signing::Provenance::Verified
        );
        assert!(!seen.trusted && !seen.acts);

        local.trust_key(peer.key_id(), "peer", "").unwrap();
        assert_eq!(
            local.message_return_address(&planted).unwrap().as_deref(),
            Some("s-local-coder")
        );
        assert!(local.message_provenance(&planted).unwrap().unwrap().acts);
    }

    #[test]
    fn message_return_address_reads_back_refs_session_id_or_none() {
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "kind", "group", "local/a");
        let with_addr = s.post_message(&MessageEnvelope {
            origin: "local".into(),
            channel_id: "c-1".into(),
            sender: "local/pm".into(),
            kind: MessageKind::Task,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: None,
            refs: Refs {
                session_id: Some("s-pm".into()),
                ..Refs::default()
            },
            body: "order".into(),
        });
        let no_addr = s.post_message(&MessageEnvelope {
            origin: "local".into(),
            channel_id: "c-1".into(),
            sender: "local/human".into(),
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: None,
            refs: Refs::default(),
            body: "just chatting".into(),
        });
        assert_eq!(
            s.message_return_address(&with_addr).unwrap().as_deref(),
            Some("s-pm")
        );
        assert_eq!(s.message_return_address(&no_addr).unwrap(), None);
        assert_eq!(s.message_return_address("m-nope").unwrap(), None);
    }

    #[test]
    fn message_and_thread_channel_lookups_resolve_reply_targets() {
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "kind", "group", "local/a");
        let mid = s.post_message(&MessageEnvelope {
            origin: "local".into(),
            channel_id: "c-1".into(),
            sender: "local/a".into(),
            kind: MessageKind::Question,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("m-thread".into()),
            refs: Refs::default(),
            body: "q?".into(),
        });
        assert_eq!(
            s.message_channel(&mid),
            Some(("c-1".into(), Some("m-thread".into())))
        );
        assert_eq!(s.message_channel("m-nope"), None);
        let root = ThreadRoot {
            origin: "local".into(),
            channel_id: "c-9".into(),
            opener: "local/a".into(),
            created: String::new(),
            parent: None,
        };
        s.open_thread("m-thread", &root, "local/a");
        assert_eq!(s.thread_channel("m-thread"), Some("c-9".into()));
    }

    #[test]
    fn profile_and_message_search_use_substring_over_the_right_columns() {
        let mut s = ChatStore::open_in_memory(1);
        s.set_profile_field("local/qa", "job_title", "QA Reviewer", "local/qa");
        s.set_profile_field(
            "local/qa",
            "job_description",
            "runs the race-flag suite",
            "local/qa",
        );
        s.set_profile_field("local/dev", "job_title", "Backend Dev", "local/dev");
        // case-insensitive substring over job_title AND job_description
        let hits = s.search_profiles("reviewer");
        assert_eq!(
            hits.iter().map(|p| p.handle.as_str()).collect::<Vec<_>>(),
            vec!["local/qa"]
        );
        assert_eq!(
            s.search_profiles("RACE").len(),
            1,
            "matches job_description too"
        );
        // a LIKE metacharacter is literal (bound param + ESCAPE)
        assert_eq!(s.search_profiles("%").len(), 0, "literal %, not a wildcard");

        s.set_channel_field("c-1", "kind", "group", "local/qa");
        s.add_member("c-1", "local/qa", "local/qa");
        s.post_message(&env("c-1", "ship the release now", Disposition::InTurn));
        let mhits = s.search_messages(&["c-1".to_string()], "RELEASE").unwrap();
        assert_eq!(
            mhits.len(),
            1,
            "substring over body, in the scope it was given"
        );
        assert_eq!(
            s.search_messages(&["c-2".to_string()], "release")
                .unwrap()
                .len(),
            0,
            "outside the given scope → no hits"
        );
        // WHO may search WHAT left this layer with nxf 6j6v.cs03 — `facade::search`'s own
        // `search_returns_hits_in_the_callers_channels` holds the non-member case now. What belongs
        // here is that an EMPTY scope is an empty result and never "then every channel": this read
        // fails closed, so a caller that resolved no membership cannot accidentally sweep the
        // workspace.
        assert!(
            s.search_messages(&[], "release").unwrap().is_empty(),
            "an empty scope is an empty result, not an unscoped search"
        );
    }

    #[test]
    fn search_messages_returns_same_millisecond_hits_in_causal_post_order() {
        // 0b1m (PR-review Test Quality, Low): search results order by `(channel_id, lamport, site,
        // message_id)`, so same-millisecond hits within one channel read back in post order,
        // deterministically — the same causal guarantee as the channel/thread reads.
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "kind", "group", "o/a");
        s.add_member("c-1", "o/a", "o/a");
        let posted: Vec<String> = (0..24)
            .map(|i| {
                let body = format!("findme {i:02}");
                s.post_message(&env("c-1", &body, Disposition::InTurn));
                body
            })
            .collect();
        let hits: Vec<String> = s
            .search_messages(&["c-1".to_string()], "findme")
            .unwrap()
            .into_iter()
            .map(|h| h.body)
            .collect();
        assert_eq!(hits, posted, "same-ms search hits in post (causal) order");
    }
}
