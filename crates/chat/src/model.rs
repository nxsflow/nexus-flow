//! nexus-chat's `message` vocabulary + the serde envelope (spec §2.1). The substrate op shape is
//! domain-agnostic and owned by the foundation; this module owns only the constants that tag a
//! message op and the payload types a fold parses.

use serde::{Deserialize, Serialize};

/// The reducer domain — the axis the registry dispatches on (spec §4.1).
pub const DOMAIN_MESSAGE: &str = "message";

// Intra-domain kinds (target_kind), dispatched after the domain (spec §2).
pub const KIND_MESSAGE: &str = "message";
pub const KIND_CHANNEL: &str = "channel";
pub const KIND_PROFILE: &str = "profile";
pub const KIND_MEMBERSHIP: &str = "membership";
pub const KIND_THREAD: &str = "thread";
// `KIND_READ_CURSOR` ("read_cursor") stood here until nxf 6j6v.4d2z removed the unread
// apparatus. The kind is not re-used: a `read_cursor` op in an older log stays STORED and
// simply stops being foldable, which is the substrate's ordinary treatment of a kind no
// reducer claims (`MessageReducer::is_foldable` answers `false` and `fold` is never reached).

// Op types.
pub const OP_POST: &str = "post"; // message
pub const OP_SET: &str = "set"; // channel/profile field, thread expects_reply_from
pub const OP_ADD: &str = "add"; // membership
pub const OP_REMOVE: &str = "remove"; // membership
pub const OP_OPEN: &str = "open"; // thread root

// Fields.
pub const FIELD_ENVELOPE: &str = "envelope";
pub const FIELD_ROOT: &str = "root";
pub const FIELD_EXPECTS_REPLY_FROM: &str = "expects_reply_from";
/// M2 (spec §2.1/§3.1): the opt-in, sparse wall-clock after which an incomplete thread reads as
/// `stale`. Stored absolute RFC3339; folds on the SAME keep-if-beats LWW path as
/// `expects_reply_from`.
///
/// **"Advisory" is no longer the whole truth, and saying so is load-bearing.** In M2 nothing acted
/// on `stale`: it drove the "waiting past deadline" line of a requester's wake and nothing else.
/// Since nxf 6j6v.pf6j a channel member's `stale` is what lets the supervisor settle its SET without
/// that member — the deadline is one of the two things that end a channel — so anything written here
/// decides whether an answer is waited for or given up on. What a reader of a MEMBER thread's
/// deadline sees is not this register but the resettable clock of nxf 6j6v.nf38
/// (`member_deadline.rs`), which shadows it; this register stays what it always was for every other
/// thread, and is what a sync peer with no transcript of its own still sees.
pub const FIELD_DEADLINE: &str = "deadline";
/// **What a conversation is CALLED** (nxf 6j6v.e76c): a short display name derived from the message
/// that opened the thread, at most [`crate::naming::NAME_MAX_WORDS`] words. Sparse — a thread
/// nobody named carries none — and folded on the SAME keep-if-beats LWW path as the two registers
/// above it.
///
/// **A DISPLAY name and never a key.** The thread id is the identity and stays it; nothing joins,
/// looks up or routes on this value. It exists because a `dm:` channel id is a hash over the two
/// sorted chat identities and therefore has, by construction, no name a surface could show — which
/// is what put `dm:d9ab4f601a08…` in front of a user in a live app.
///
/// # "Once" is enforced at the WRITE, and that leaves a residue — stated rather than implied
///
/// [`crate::facade::name_thread`] reads this register and declines when it is set, which is what
/// makes the rule unforgettable for a call site. What it is not is a FOLD rule: two writers that are
/// genuinely concurrent — the commissioned run racing a hand-typed `nxc threads name`, or two
/// devices naming offline before they sync — can each read `NULL`, each emit a SET, and LWW then
/// picks one. A reader on the losing device sees the name change once (review of PR #452, Code
/// Quality · Medium).
///
/// It is accepted, and the alternative is why. A fold rule of "only if currently NULL" is not
/// write-once, it is DIVERGENT: replicas that apply the two ops in different orders keep different
/// names forever, which is strictly worse than the transient this has. A convergent write-once — the
/// lowest op id wins, say — would mint a second register kind in the substrate, with its own
/// migration and its own fold path, for a value that decides nothing (see the paragraph above).
/// So: a name is not guaranteed immutable under a concurrent writer; it is guaranteed to converge,
/// and every surface renders a thread with no name at all.
pub const FIELD_NAME: &str = "name";
/// **Which machine a persona chat runs on** (nxf 6j6v.1c6k): the id of one machine
/// (`nxs_service::Machine::id`, a service home's `machine.toml`). Written when the chat starts —
/// from the chat's own `--machine`, else the persona's `machine:`, else the machine that started it —
/// and written again to hand the chat to another machine. Sparse: a thread no machine was named for
/// (every channel thread, and every thread a host without a machine opened) carries none, and runs
/// where its messages are written, as before.
///
/// Folded on the same keep-if-beats LWW path as the three registers above, so every replica reads
/// the same machine and the last hand-over wins. **What lets a machine act on it is the op that
/// wrote it, never the value**: only a register whose winning op acts (`acting_ops`) designates
/// anybody — see `docs/specs/E4-executing-machine.md`.
pub const FIELD_MACHINE: &str = "machine";

// Channel kinds — the values the `channel.kind` LWW register carries. Deliberately an OPEN string
// on the wire (spec §3.2; `chat-types.ts` mirrors it as one), so a reader that does not know a kind
// still round-trips it; these constants are the values THIS engine writes and dispatches on, and
// exist so the same word is never spelled twice in two places.
/// A DM between exactly two handles, under a deterministic `dm:` id (spec §3.2). The read side
/// treats one whose resolved membership is not exactly 2 as degraded (§3.3).
pub const CHANNEL_KIND_DIRECT: &str = "direct";
/// A named, member-only channel — the default a declaration produces (and, until 6j6v.dvyq §3,
/// what `nxc channels create` produced too).
pub const CHANNEL_KIND_GROUP: &str = "group";
/// A project's FRONT DOOR (nxf 6j6v.bd6g, app-foundations spec §8 D6.2): readable and addressable
/// without membership, and discoverable by anyone in the workspace.
///
/// It is a READ opening and nothing else. `send`/`reply`/`ask` were membership-free for every kind
/// before it and still are. It used to be said here that `mark_read` was membership-gated for every
/// kind including this one; that ack and the read state behind it went with nxf 6j6v.4d2z, so the
/// only gate this kind moves is the one it always moved. See [`crate::facade::require_readable`].
pub const CHANNEL_KIND_PUBLIC: &str = "public";

/// Composite-`target_id` separator (unit separator), matching flow's edge convention
/// (`crates/core/src/store.rs`). Used for `channel{SEP}handle`.
pub const SEP: char = '\u{1f}';

/// The LWW-register field whitelists — injection-safe column names for the `format!`-built upsert
/// (a field not on the list is not foldable, so it can never reach the SQL).
pub const CHANNEL_FIELDS: &[&str] = &["name", "kind", "origin"];
pub const PROFILE_FIELDS: &[&str] = &[
    "job_title",
    "job_description",
    "capability_tags",
    "runtime_binding",
    "reports_to",
    "origin",
];
/// The `thread` LWW-register fields (spec §3.1/§3.2, plus [`FIELD_NAME`] since nxf 6j6v.e76c). All
/// three fold on the same keep-if-beats path; the whitelist keeps the `format!`-built column name in
/// [`crate::message_reducer`] injection-safe (a field not on the list is not foldable, so it can
/// never reach the SQL).
pub const THREAD_FIELDS: &[&str] = &[
    FIELD_EXPECTS_REPLY_FROM,
    FIELD_DEADLINE,
    FIELD_NAME,
    FIELD_MACHINE,
];

/// The label [`MessageKind::Escalation`] is stored as in `messages.kind` and carried on the wire.
///
/// Written out as a constant because SQL has to compare against it (`ChatStore::last_reply_escalated`
/// asks the column, not a Rust value) and a string literal in a query is exactly the kind of thing
/// that drifts away from the serde attribute above it. `model::tests::the_escalation_marker_is_the
/// _label_the_column_stores` pins the two together, so the derivation and the wire form cannot
/// disagree.
pub const KIND_ESCALATION: &str = "escalation";

/// The label [`MessageKind::NeedsRework`] is stored as in `messages.kind` — [`KIND_ESCALATION`]'s
/// twin, written out for the same reason and pinned by the same test: SQL compares against it
/// (`ChatStore::last_reply_needs_rework`), and a literal inside a query is exactly what drifts away
/// from the serde attribute above it.
pub const KIND_NEEDS_REWORK: &str = "needs_rework";

/// The label [`MessageKind::Accepted`] is stored as in `messages.kind` — the third of
/// [`KIND_ESCALATION`]'s family, written out for the same reason and pinned by the same test.
///
/// **This one is the reason the override is FINDABLE** (nxf 6j6v.am8j). An acceptance is a verdict
/// being set aside by the party that commissioned the round, and the whole point of recording it as
/// its own kind rather than as a plain reply is that somebody looking for it later — "which rounds
/// went on over a reviewer's objection?" — has one label to ask for instead of a prose search
/// through bodies.
pub const KIND_ACCEPTED: &str = "accepted";

/// What a message IS (spec §2.1). The wire form is the lowercase LABEL below, and `messages.kind`
/// stores that label — **not** an ordinal.
///
/// **Which rule applies to this enum, spelled out because the one below it has the opposite one**
/// (nxf 6j6v.wt37): [`Priority`]'s discriminants are an ON-DISK FORMAT (`working_tree_queue.priority`
/// stores `Priority as i64` and orders by it), so reordering ITS variants silently re-labels rows.
/// Nothing stores a `MessageKind` as a number: the reducer writes
/// `serde_json::to_value(env.kind).as_str()` into a `TEXT` column, so adding a variant here is purely
/// ADDITIVE on disk — no migration, no refold, and an older reader simply fails to parse an envelope
/// carrying a kind it does not know, which is the store-don't-fold path §3.1 already defines.
///
/// **Deliberately NOT `#[non_exhaustive]`, decided when `Escalation` was added** (nxf 6j6v.wt37).
/// Adding a variant to an exhaustive `pub` enum is a semver break (`cargo-semver-checks` reports
/// `enum_variant_added`, and the fragment for that change carries `facade: breaking`), so the marker
/// looks like the obvious remedy. It is not, for three reasons:
///
/// 1. **It does not avoid this break** — applying it is itself breaking, and it only buys FUTURE
///    additions.
/// 2. **This repo's rule for enums points the other way.** [`crate::facade::WorkingTreeStatus`] is
///    kept exhaustive on purpose, so that a variant added without announcement is a COMPILE ERROR at
///    every reader rather than a silently-taken `_` arm. The structs that took the marker earlier in
///    this work order are pure OUTPUT records with an "unreleased, last free moment" argument; a
///    vocabulary enum has neither half of it.
/// 3. **`MessageKind` is an INPUT enum** — consumers CONSTRUCT it to send a message, and
///    `#[non_exhaustive]` does not restrict construction of an enum's known variants at all. It would
///    forbid exhaustive matching while leaving the thing consumers actually do untouched: cost
///    without the protection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    Task,
    /// The real FOLLOW-UP question — "what do you mean by X?". Deliberately NOT
    /// [`MessageKind::Escalation`]: the sender can still do the work and wants one thing cleared up.
    ///
    /// **It holds the working-tree claim exactly as an escalation does** (nxf 6j6v.1xw1, owner's
    /// ruling of 2026-08-16), and that is not a contradiction of the line above — it is the one
    /// place where the difference between the two is no difference. The distinction is about what a
    /// reply MEANS, and it survives everywhere it decides something: `ChatStore::last_reply_escalated`
    /// answers `Some(false)` here, and a channel's consolidator does not pass a question upward the
    /// way it passes an escalation. The lease asks a different question — is anybody working? — and
    /// the answer for both is no, while the task is mid-flight. Releasing would let a rival start
    /// into a working copy the questioner is going to resume into. See
    /// `working_tree::hands_the_task_back`, which is the one place the two are named together.
    Question,
    Report,
    Decision,
    Info,
    /// **"I cannot reach the result on my own — I need help, or a decision"** — one of the three
    /// things an agent may say with `reply` (nxf 6j6v.wt37, widened to three by 6j6v.553s (a); the
    /// other two are "I am finished" and [`NeedsRework`](MessageKind::NeedsRework)). Written by `nxc reply --escalate` and by
    /// [`crate::engine::Engine::reply_thread`] with `escalate: true`, which resolves to this kind.
    ///
    /// It is ONE DECLARED bit (hq71 §4), and that is exactly what makes it a signal at all: the
    /// channel supervisor branches on something whose MEANING IS WRITTEN IN THE CHANNEL, not on a
    /// token an agent invents on the spot and no reader can enumerate.
    ///
    /// It is committed to that one meaning and must stay committed to it, or it becomes the
    /// wastebasket for everything that is not a result — which is what [`MessageKind::Question`]
    /// above is NOT, and [`NeedsRework`](MessageKind::NeedsRework) below is not either, and the
    /// reason all three are separate variants rather than one "not a result" flag. This one is
    /// about the SENDER's own work; the verdict is about somebody else's.
    ///
    /// **Two effects that must not be conflated** (they hold at the same time):
    ///
    /// - **The TURN** (6j6v.cg8g): an escalating reply discharges the sender's turn exactly like a
    ///   finished one. The session ends either way, so the discharge predicate gives this kind NO
    ///   special case — see `ChatStore::thread_quorums`, which deliberately never looks at
    ///   `messages.kind` at all.
    /// - **The CLAIM** (6j6v.1xw1): an escalating reply does NOT close the thread it was written on.
    ///   The question travels upward, possibly as far as the human, while the task is still mid-flight
    ///   — so the working copy stays protected. The readable derivation for that is
    ///   `ChatStore::last_reply_escalated`; the rule that consumes it is
    ///   `ChatStore::work_scope_handed_back`, which asks the wider question ("was the task handed
    ///   back?") and therefore also holds for an open [`MessageKind::Question`]. That is not this
    ///   variant losing its meaning — see that variant's own doc for why the two facts stay apart
    ///   everywhere else.
    Escalation,
    /// **"What you handed me does not meet the standard"** — the THIRD and last thing an agent may
    /// say with `reply` (nxf 6j6v.553s (a), owner 2026-08-29). Written by `nxc reply --needs-rework`
    /// and by [`crate::engine::Engine::reply_thread`] with `needs_rework: true`.
    ///
    /// **It is on the OTHER AXIS from [`Escalation`](MessageKind::Escalation), and keeping the two
    /// apart is the whole reason it is a variant of its own.** An escalation is a statement about
    /// the sender's OWN work ("I need help, or a decision") and travels UP, holding the working
    /// copy. A verdict is a statement about SOMEBODY ELSE's work ("this is not deliverable") and
    /// travels BACK, to the party that produced it, with the run still going. Overloading
    /// `--escalate` with it would be the double meaning this house builds gates against — and it would make "I have
    /// findings" and "I am stuck" the same fact, which they demonstrably are not.
    ///
    /// Together the three are a trichotomy over {done, again, cannot}, and there is no fourth case:
    /// *"Gut, aber unvollstaendig"* is rework, *"ich brauche erst X"* is escalation, *"passt"* is the
    /// normal way on.
    ///
    /// **Its MEANING is engine-defined and its EFFECT is declared** (owner, same note): the bit is
    /// always spelled `--needs-rework`, and where it goes is [`crate::channel::FlowStep::on_needs_rework`]
    /// — literally written in the channel, which is what `--escalate`'s own documentation has always
    /// said makes a bit a signal at all. A step that declares no back edge does not OFFER the bit
    /// ([`crate::role::ReplyObligation`]), so "set but inert" is prevented rather than merely logged.
    ///
    /// **The turn and the claim, exactly as for the other two**: it discharges the sender's turn
    /// like any answer (`ChatStore::thread_quorums` never looks at `kind`), and it does NOT hand the
    /// task back for the working copy — the run continues, so the very next step's obligation is
    /// what holds the lease. See `working_tree::hands_the_task_back`, which says why the opposite
    /// answer would hold a working copy long past a run that finished cleanly.
    ///
    /// **The explicit `rename`**: this enum's `rename_all = "lowercase"` has no word separator, so
    /// the derived label would be `needsrework` — readable by nothing and spelled by nobody. The
    /// wire form is named here and pinned against [`KIND_NEEDS_REWORK`] by the test below, exactly
    /// as the escalation label is.
    #[serde(rename = "needs_rework")]
    NeedsRework,
    /// **"I know what the review said, and this goes on anyway"** — the FOURTH thing that can be
    /// said with `reply`, and the only one that is not an agent's (nxf 6j6v.am8j, owner 2026-09-12).
    /// Written by `nxc reply --accept` and by [`crate::engine::Engine::reply_thread`] with
    /// `accept: true`.
    ///
    /// **It is not a fourth member of the trichotomy — it is on a level above it.** The other three
    /// are what a party SERVING a step may say about the work in front of it: done, again, cannot.
    /// This one is said by the party that COMMISSIONED the round, about a judgement that round
    /// produced, and it is the only way the state "the review is not satisfied, and the work is good
    /// enough" has ever had a spelling. Until this variant a round whose reviewer never became
    /// satisfied could not produce a result at all: the only transition to
    /// [`crate::channel::FlowStep::next`] was the reviewer's own, and the human above it could
    /// re-commission the round (three more passes) or throw it away, and nothing else. Measured in
    /// `watch-bundestag` on 2026-09-12: six draft/review passes, six verdicts, no ticket.
    ///
    /// **Who may write it is a RULE and not a convention** (the same note, answering the owner's
    /// second question with "no"): the party that produced the work may not declare its own reviewer
    /// satisfied — that would dissolve the separation the channel exists to create. So the
    /// acceptance is honoured on the CHANNEL thread and only from the party that opened it, and a
    /// producer reaching for it on its own slot is refused by name and pointed at
    /// [`Escalation`](MessageKind::Escalation), which is how an agent asks for a decision it may not
    /// take.
    ///
    /// **The turn and the claim**: it is not an answer to an obligation at all — it settles nobody's
    /// turn (`ChatStore::thread_quorums` never looks at `kind`, and the channel thread expects the
    /// supervisor, not the owner) and it holds no working copy. What it does is start the next step,
    /// and that step's own obligation is what takes the lease, exactly as on every other edge.
    Accepted,
}

/// **The discriminants are written out because they are an ON-DISK FORMAT, not because a reader
/// needs the numbers** (PR review of nxf epic 6j6v.bqe0). `working_tree_queue.priority` stores
/// `Priority as i64` and ORDERS BY it — smaller is more urgent — so every queued trigger already
/// persisted on a device carries these exact ordinals. Reordering the variants would silently
/// re-label those rows (a parked `Urgent` reading back as `Normal`) with no error and no failing
/// test: `working_tree::priority_from_ordinal` is written against the same enum, so a round-trip
/// test moves with the reordering and keeps passing. Spelling the numbers here makes the invariant
/// the compiler's rather than a reviewer's — a reorder now has to overwrite a literal, which is a
/// deliberate act. Serde is unaffected either way: the wire form is the LABEL below, and
/// `messages.priority` (this crate's only other persisted `Priority`) stores that label, not an
/// ordinal, because that column is user-facing history rather than an ordering key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Urgent = 0,
    Normal = 1,
    Background = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    InTurn,
    NextSession,
}

/// Pointers, not content (spec §2.1). Unknown extra fields are ignored (serde default) →
/// forward-compat; every field is optional so an empty `refs` is valid.
///
/// **What "pointer" covers, said once because the list is no longer only about the SUBJECT**:
/// `session_id` has always been a pointer at the message's own PROVENANCE — the return address of
/// whoever wrote it — [`substituted`](Refs::substituted) is the second of that kind, and
/// [`working_copy`](Refs::working_copy) (nxf 6j6v.2af2) is the third: where the tree stood when this
/// turn changed hands. All three answer "where did this come from", which is what makes them
/// pointers and not content; the body is the content.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nxf_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    /// **The RUNTIME wrote this reply, standing in for the agent that owed it** (nxf 6j6v.kffm).
    ///
    /// Set by exactly one door — [`crate::surface::settle_if_unanswered`], which is the sidecar
    /// teardown's and nobody else's — and only on a call that actually POSTED, i.e. one where the
    /// session ended still owing its thread an answer. An agent's own reply never carries it.
    ///
    /// It exists because that substitution was distinguishable only by PROSE: the body begins
    /// `sidecar:`, and a reader who did not read the body saw a discharged expectation, a released
    /// working copy and a round that looked answered. That is what made nxf 6j6v.kffm's chain
    /// invisible — the role could not run its own `nxc reply`, the teardown answered for it, and the
    /// first smoke of the acceptance run came back GREEN having never reached the model. `escalated`
    /// does not cover it either: a session that ends cleanly without answering is deliberately not
    /// an escalation, and an escalation is equally what a live agent says when it hands the task
    /// back. The two facts are orthogonal and both travel.
    ///
    /// **On `refs` and not in a column of its own**: `messages.refs` is a JSON text column, so this
    /// costs no schema change and no refold — an older reader folding a newer op ignores the key,
    /// and `json_extract` answers NULL for every message written before it existed, which is exactly
    /// "not a substitution".
    ///
    /// The reading side is [`crate::facade::StatusThread::substituted`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub substituted: bool,
    /// **Which RUN of a declared flow this message opened a step for** (nxf 6j6v.553s (d)) — the id
    /// of the message on the channel thread that started the run, stamped by the supervisor onto the
    /// commission it writes into each slot thread.
    ///
    /// It is the one thing about a stepped flow that is RECORDED rather than derived, and the reason
    /// is a cycle. Without steps, a pass could be read off the slots themselves: an ordered channel
    /// opens one slot per step, so "the newest slot serving the first step" anchored a pass
    /// (`orchestration::current_pass`). A back edge breaks that — the first step's newest slot is
    /// the REWORK, and everything the run has already done falls outside its own pass, taking the
    /// consolidation's answers with it. Every derivation that avoids recording has to guess where a
    /// run began; this says it.
    ///
    /// **Why the requester's message id and not a minted one**: every step of one run derives it the
    /// same way, from the newest non-engine message on the channel thread, which is also the message
    /// the flow forwards as the task. A follow-up from the requester is a NEW message and therefore a
    /// new run, with the counters that hang off a run starting over — which is exactly the owner's
    /// *"Bekommt er aber die Erlaubnis von oben, wird max_passes wieder auf Null gesetzt"*, arrived
    /// at without a second mechanism.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_run: Option<String>,
    /// **Which declared step this message opened** — [`crate::channel::FlowStep::id`], stamped
    /// beside [`flow_run`](Refs::flow_run) onto the same commission.
    ///
    /// This is the job `channel::validate_channels`' check 6 used to protect by refusing a repeated
    /// target ("the engine decides which step a slot thread serves by matching its target against
    /// this list"). With `steps:` the id carries it, so two steps may address one role and the same
    /// step may run twice in a run without either occurrence being mistaken for the other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_step: Option<String>,
    /// **Where the working copy stood when this turn changed hands** (nxf 6j6v.2af2) — the third
    /// pointer at the message's own PROVENANCE, beside [`session_id`](Refs::session_id) and
    /// [`substituted`](Refs::substituted).
    ///
    /// Stamped by the engine on every `send` and every `reply` — never by a caller, and no request
    /// field carries it, which is what keeps it a fact rather than a claim ([`substituted`]'s rule,
    /// applied to the one other thing on this struct the engine decides for itself). `None` where
    /// no anchor could be taken: a host that runs its sessions in no directory this process can see,
    /// a directory git does not know, or a git read that failed. See
    /// [`crate::anchor::AnchorRefusal`] for which of those is a failure and which is simply a
    /// workspace that does not do this.
    ///
    /// **A nested object and not four flat keys.** It is one fact with four parts — a commit is
    /// meaningless without knowing whether the tree was dirty at it — and `refs` is JSON, so nesting
    /// costs nothing on the wire. An older reader ignores the key entirely (unknown fields are
    /// dropped by serde default, which is what makes this additive with no migration and no refold),
    /// and `json_extract(refs, '$.working_copy')` answers `NULL` for every message written before it
    /// existed — exactly "this handover recorded nothing".
    ///
    /// The reading side is [`crate::facade::StatusThread::working_copy`] and, per message,
    /// [`crate::facade::MessageView::refs`] — which is what `nxc threads show` renders.
    ///
    /// [`substituted`]: Refs::substituted
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_copy: Option<crate::anchor::Anchor>,
}

/// The immutable message payload carried in one op's `value` (spec §2.1). `message_id`/`created`
/// come from the op (`target_id`/`wall_clock`), so they are NOT duplicated here. Required fields are
/// non-`Option`: a missing one fails `serde` parse → the op is store-don't-fold (spec §3.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageEnvelope {
    pub origin: String,
    pub channel_id: String,
    pub sender: String,
    pub kind: MessageKind,
    pub priority: Priority,
    pub disposition: Disposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub refs: Refs,
    pub body: String,
}

/// The immutable thread root carried in a `thread`/`open` op's `value` (spec §3.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRoot {
    pub origin: String,
    pub channel_id: String,
    pub opener: String,
    #[serde(default)]
    pub created: String,
    /// The thread this one was opened OUT OF — the parent edge that makes an operation a thread
    /// TREE (nxf 6j6v.a71h §3.1). `None` is the tree's ROOT: nothing was open around the caller
    /// when this thread was minted, which by construction is the top end of a chain (a human at a
    /// terminal, or any caller acting outside a triggered session). The root is therefore DERIVED,
    /// never stored as a flag of its own (§3.1's closing line).
    ///
    /// **`Option` with `#[serde(default)]`, and it must stay that way.**
    /// [`crate::message_reducer::MessageReducer`]'s `is_foldable` decides a `thread`/`open` op's
    /// foldability by whether its `value` parses as a `ThreadRoot`, so a required field here would
    /// turn every thread op ever written — including ones already synced from another device — into
    /// store-don't-fold. `skip_serializing_if` keeps a ROOT thread's op bytes identical to what this
    /// crate wrote before the field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// Split a composite `target_id` on [`SEP`] into exactly two non-separator parts (`None` otherwise),
/// so no fold path can index-panic on a malformed id (mirrors flow's `parse_edge`).
pub fn split2(id: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = id.split(SEP).collect();
    match parts.as_slice() {
        [a, b] => Some((a.to_string(), b.to_string())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_round_trip_via_serde() {
        // The lowercase wire form is the contract (spec §2.1 enum values).
        assert_eq!(
            serde_json::to_string(&MessageKind::Decision).unwrap(),
            "\"decision\""
        );
        assert_eq!(
            serde_json::to_string(&Disposition::NextSession).unwrap(),
            "\"next_session\""
        );
        assert_eq!(
            serde_json::to_string(&Priority::Background).unwrap(),
            "\"background\""
        );
    }

    #[test]
    fn the_rework_marker_is_the_label_the_column_stores() {
        // nxf 6j6v.553s (a), and the trap is sharper here than for the escalation twin: this enum
        // renames to LOWERCASE with no separator, so a variant added without its own `rename` is
        // stored as `needsrework` while `KIND_NEEDS_REWORK` says `needs_rework` — a verdict bit that
        // is written, read back as nothing, and silently dropped, with every routing test still
        // green because nothing ever branches.
        assert_eq!(
            serde_json::to_value(MessageKind::NeedsRework)
                .unwrap()
                .as_str(),
            Some(KIND_NEEDS_REWORK)
        );
    }

    #[test]
    fn the_acceptance_marker_is_the_label_the_column_stores() {
        // nxf 6j6v.am8j. The whole reason this kind exists rather than a plain reply is that the
        // override is FINDABLE afterwards, and findable means one label to ask the column for. A
        // rename that drifted from `KIND_ACCEPTED` would leave the routing working and the record
        // unsearchable, which is the half nobody would notice.
        assert_eq!(
            serde_json::to_value(MessageKind::Accepted)
                .unwrap()
                .as_str(),
            Some(KIND_ACCEPTED)
        );
        assert_eq!(
            serde_json::from_str::<MessageKind>("\"accepted\"").unwrap(),
            MessageKind::Accepted
        );
    }

    #[test]
    fn the_escalation_marker_is_the_label_the_column_stores() {
        // nxf 6j6v.wt37. `messages.kind` stores exactly `serde_json::to_value(kind).as_str()` (see
        // `message_reducer`), and `ChatStore::last_reply_escalated` compares that COLUMN against
        // [`KIND_ESCALATION`]. Without this pin, renaming the serde label would silently turn the
        // derivation into "no answer has ever escalated" — a lease that releases while the question
        // is still travelling upward, with every test still green.
        assert_eq!(
            serde_json::to_value(MessageKind::Escalation)
                .unwrap()
                .as_str()
                .unwrap(),
            KIND_ESCALATION
        );
        // And it round-trips: an older reader that does not know the variant fails to PARSE the
        // envelope, which is the store-don't-fold path (§3.1), not a silent mis-read.
        assert_eq!(
            serde_json::from_str::<MessageKind>("\"escalation\"").unwrap(),
            MessageKind::Escalation
        );
    }

    #[test]
    fn envelope_round_trips_and_rejects_bad_enum() {
        let env = MessageEnvelope {
            origin: "nxsflow/nexus-flow".into(),
            channel_id: "c-1".into(),
            sender: "nxsflow/nexus-flow/PmAgent".into(),
            kind: MessageKind::Task,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: None,
            refs: Refs::default(),
            body: "ship it".into(),
        };
        let json = serde_json::to_string(&env).unwrap();
        assert_eq!(serde_json::from_str::<MessageEnvelope>(&json).unwrap(), env);
        // An out-of-range enum fails to parse — this is exactly what is_foldable relies on (§3.1).
        let bad = json.replace("\"normal\"", "\"screaming\"");
        assert!(serde_json::from_str::<MessageEnvelope>(&bad).is_err());
        // A missing required field also fails (defers store-don't-fold).
        let missing = json.replace(",\"body\":\"ship it\"", "");
        assert!(serde_json::from_str::<MessageEnvelope>(&missing).is_err());
    }

    #[test]
    fn a_thread_root_written_before_the_parent_edge_existed_still_parses() {
        // nxf 6j6v.a71h §3.1. The parent is OPTIONAL for exactly one reason: `is_foldable` decides a
        // `thread`/`open` op by whether its value parses as a `ThreadRoot`, and the ops already in
        // the log (and already synced from other devices) carry no parent at all. A required field
        // would make every one of them store-don't-fold.
        let old = r#"{"origin":"local","channel_id":"c-1","opener":"local/a","created":"2026-08-14T10:00:00Z"}"#;
        let parsed: ThreadRoot = serde_json::from_str(old).expect("an old root still parses");
        assert_eq!(parsed.parent, None, "no parent recorded → a ROOT thread");

        // And a root with no parent serializes back to those exact bytes: the wire form of every
        // thread this crate opened before the edge existed is unchanged.
        assert_eq!(serde_json::to_string(&parsed).unwrap(), old);

        // A child round-trips with the edge on it.
        let child = ThreadRoot {
            parent: Some("t-parent".into()),
            ..parsed
        };
        let json = serde_json::to_string(&child).unwrap();
        assert!(json.contains(r#""parent":"t-parent""#), "{json}");
        assert_eq!(serde_json::from_str::<ThreadRoot>(&json).unwrap(), child);
    }

    #[test]
    fn split2_needs_exactly_two_parts() {
        assert_eq!(split2(&format!("a{SEP}b")), Some(("a".into(), "b".into())));
        assert_eq!(split2("noseparator"), None);
        assert_eq!(split2(&format!("a{SEP}b{SEP}c")), None);
    }
}
