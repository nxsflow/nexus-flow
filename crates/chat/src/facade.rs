//! nexus-chat's app-facade — the compute→render seam (be9y, the mirror of memory's `facade.rs` /
//! flow's `nexus-flow-facade`).
//!
//! The `nxc` verbs print records straight to stdout; this layer is the **compute** half of a
//! compute→render split: each read returns a canonical view value and each write returns a receipt
//! — no `println!`, no clap, no presentation. Each consumer renders over it: the `nxc` CLI serializes
//! under `--json` or draws the human line; the in-process [`Engine`](crate::engine::Engine) hands the
//! typed value to an embedding app (via app-foundations, downstream).
//!
//! There is NO new semantics: reads reuse the exact `messages`/membership views + ordering the CLI
//! uses, and writes reuse the exact rejection order the CLI uses ([`require_readable`] and its
//! public-channel sibling [`require_readable`], plus `check_no_sep`, live HERE now, shared by both
//! seams). Rejection kinds are part of the contract — pinned by the App↔CLI parity differential
//! (`tests/parity.rs`). Writes take **explicit** `now` + acting-handle
//! params (like flow/memory), never the ambient `NXC_NOW`/`caller_handle()` (that stays CLI-side).
//!
//! **One narrow, deliberate exception**, worth naming because the claim above is otherwise absolute:
//! this layer parses stored enum columns into typed values, so a row whose column is out of range is
//! an `io` error where the CLI's old raw-string reads shrugged and carried on. The state is
//! unreachable through the op log — the reducer refuses to fold an envelope carrying an unknown
//! variant — and self-repairing on refold; both halves are pinned by tests at the bottom of this
//! file. (It was `inbox`, and then `prime` beside it, that this was said of; both readings went
//! with the unread apparatus in nxf 6j6v.4d2z, and the rule is the layer's, not theirs.)
//!
//! The M2 quorum surface lives here too: [`ask`] opens a review board, [`threads`]/[`thread_board`]
//! render the derived [`ThreadQuorum`], and [`thread_quorums`]/[`opener_wake`] are the bulk /
//! requester-wake reads the in-process [`Engine`](crate::engine::Engine) exposes (§8). All reuse the
//! exact `nxc` derivations (seam invariant, `tests/parity.rs`). [`ThreadView`] (the M1 loose thread)
//! is deliberately left unchanged — the sparse-key wire keeps pre-M2 threads byte-compatible.

use std::collections::{BTreeMap, BTreeSet};

use crate::channel::{declared_policy, ChannelDecl, ChannelPolicy, ValidationError, Visibility};
use crate::declaration_quality::DeclarationWarning;
use crate::definitions::DeclarationSource;
use crate::error::{NxfError, Result};
use crate::model::{
    Disposition, MessageEnvelope, MessageKind, Priority, Refs, ThreadRoot, CHANNEL_KIND_PUBLIC, SEP,
};
use crate::persona::{Directory, PersonaBrief};
use crate::role::PrimeServices;
use crate::store::{ChatStore, MessageHit, MessageRow, OpenerWake, ThreadQuorum};
use crate::transcript::TranscriptPruneReport;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

// ---- DTOs (declared field order = the JSON contract) -----------------------

/// A full message as an app reads it (be9y). Typed enums (identical lowercase wire form to the
/// stored strings) + the parsed [`Refs`]. `thread_id`/`created` are sparse (skipped when absent) so
/// the wire stays additive; `refs` serializes as `{}` when empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageView {
    pub message_id: String,
    pub origin: String,
    pub channel_id: String,
    pub sender: String,
    pub kind: MessageKind,
    pub priority: Priority,
    pub disposition: Disposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub refs: Refs,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// **Nobody here can vouch for this message** (nxf 6j6v.pzkb): it carries no signature, one that
    /// does not check out, or one from a key this workspace does not trust. It is shown like any
    /// other, and no agent action follows it — it discharges no obligation, names no session to
    /// resume, is collected as nobody's answer. `nxs sync verify` and `nxs sync trust list` say who
    /// signed it.
    ///
    /// Omitted when false, so every message this replica or a trusted one wrote reads exactly as
    /// before.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unvouched: bool,
}

impl MessageView {
    /// The canonical JSON value (field order = declaration order).
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("MessageView serializes")
    }
}

/// Parse a stored enum column (the lowercase wire form) back into the typed enum. The reducer's
/// `is_foldable` guarantees only valid envelopes materialize, so this never fails in practice; an
/// impossible mismatch is a data-integrity `io` (keeps the seam total — never a panic).
fn parse_enum<T: serde::de::DeserializeOwned>(field: &str, s: &str) -> Result<T> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
        .map_err(|_| NxfError::io(format!("message row has an unparseable {field}: {s:?}")))
}

impl TryFrom<MessageRow> for MessageView {
    type Error = NxfError;

    fn try_from(r: MessageRow) -> Result<MessageView> {
        let refs: Refs = match &r.refs {
            Some(txt) => serde_json::from_str(txt).map_err(|_| {
                NxfError::io(format!("message {} has unparseable refs", r.message_id))
            })?,
            None => Refs::default(),
        };
        Ok(MessageView {
            kind: parse_enum("kind", &r.kind)?,
            priority: parse_enum("priority", &r.priority)?,
            disposition: parse_enum("disposition", &r.disposition)?,
            message_id: r.message_id,
            origin: r.origin,
            channel_id: r.channel_id,
            sender: r.sender,
            thread_id: r.thread_id,
            refs,
            body: r.body,
            created: r.created,
            unvouched: !r.acts,
        })
    }
}

/// A channel as an app lane reads it: the channel's own fields, its resolved member COUNT and the
/// degraded-DM flag (be9y). `name`/`kind` render as `null` when absent (matching the CLI's
/// `ChannelRow`).
///
/// **It carried two more fields until nxf 6j6v.4d2z** — `unread` and `unread_next_session`, the
/// caller's per-channel unread counts. They went with the apparatus that produced them: the counts
/// were a fold over the unread inbox, and there is no unread set any more. Nothing invents a
/// number in their place; what an app renders instead is the CONVERSATION (`thread`,
/// `thread_board`, `status`), which is the shape the surface has had since §3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelView {
    pub channel_id: String,
    pub name: Option<String>,
    pub kind: Option<String>,
    pub members: usize,
    pub degraded: bool,
}

impl ChannelView {
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("ChannelView serializes")
    }
}

/// A thread as an app reads it (M1: the set of messages sharing a `thread_id`). `opener`/`created`
/// are derived from the first message. Quorum/`expects_reply_from` are deliberately absent HERE — the
/// M2 quorum surface lives on [`ThreadBoardView`] instead, so this loose view stays byte-compatible
/// for pre-M2 consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThreadView {
    pub thread_id: String,
    pub channel_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opener: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    pub messages: Vec<MessageView>,
}

impl ThreadView {
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("ThreadView serializes")
    }
}

/// A thread's relation to the device-local working-tree lease (nxf epic 6j6v.bqe0, ticket 6j6v.qk5b)
/// — `nxc threads list`/`show`'s answer to "who holds it, who is waiting" for the reader who did
/// NOT happen to see a queued trigger's own receipt at the moment it queued
/// (`orchestration::TriggerReceipt::queue_position`, ticket 6j6v.303b). That receipt is the only
/// place this repo said so before this ticket; this is the view AFTERWARDS, read back off the board
/// instead of remembered from one message.
///
/// Deliberately not `#[non_exhaustive]`: like [`crate::working_tree::WorkScope`], this is decided
/// exhaustively from exactly two store facts (who holds, who is queued) by
/// [`working_tree_status_by_thread`] alone, and a third variant showing up unannounced there is a
/// bug that exhaustive matching is supposed to catch, not degrade past silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkingTreeStatus {
    /// The chain this thread belongs to currently holds the working-tree lease — which since nxf
    /// 6j6v.1xw1 means this thread is anywhere in the holder's claim area: the claim thread itself,
    /// or anything in the SUBTREE below it, or (for a `run:<id>` holder) the subtree below one of
    /// that run's channel-step boards. [`ChatStore::work_scope_threads`] is the one derivation.
    Holding,
    /// A queued trigger names this thread the same way; [`WorkingTreeStatus::Waiting`]'s position
    /// travels on a SIBLING field (`working_tree_queue_position` on both
    /// [`ThreadListEntry`]/[`ThreadBoardView`]), not on this enum, because the wire form the design
    /// fixes is a bare string (`"holding" | "waiting" | null`), not an object.
    Waiting,
}

/// Bulk-derive, for every thread the CURRENT holder or ANY queued trigger names, whether it is
/// [`WorkingTreeStatus::Holding`] or [`WorkingTreeStatus::Waiting`] (+ its 1-based queue position) —
/// reused verbatim by [`threads`] and [`thread_board`] so `nxc threads list` and `show` cannot
/// compute this two different ways (nxf 6j6v.qk5b; see [`ChatStore::work_scope_threads`]'s own doc
/// for why this epic is already four review findings deep on exactly that shape of drift). A thread
/// absent from the returned map needs the working tree not at all — the ordinary case, rendered as
/// `null` by both callers, never as a missing key.
///
/// **The membership test is [`ChatStore::work_scope_threads`], reused rather than re-derived**: the
/// SAME "which threads belong to this scope" question [`ChatStore::work_scope_has_outstanding`] (the
/// release rule, ticket 6j6v.fe0f) already answers for the holder's own scope. Since nxf 6j6v.1xw1
/// that is a SUBTREE, so the board now reports the state for everything a chain opened underneath
/// itself rather than for one level of it. On the QUEUE side that membership test is
/// joined by one thread it cannot see — the queued entry's own — see the comment at that loop for
/// which case that is and why it is not a second derivation of scope membership.
///
/// **Bulk by construction, never one lease query per thread on the board**: ONE
/// [`ChatStore::working_tree_holder`] read, ONE [`ChatStore::list_working_tree_queue`] read (the
/// WHOLE queue, not a peek), and one [`ChatStore::work_scope_threads`] call per DISTINCT scope the
/// holder or the queue actually names — bounded by how many chains are genuinely competing for the
/// tree, never by how many threads `threads list`'s board happens to enumerate.
///
/// **A thread named by more than one queued entry keeps the EARLIER position** — the entry that
/// will actually reach it first, since [`ChatStore::list_working_tree_queue`] is already in take
/// order and a `HashMap::entry().or_insert()` scan over it keeps the first write.
fn working_tree_status_by_thread(
    store: &ChatStore,
    now: &str,
) -> Result<HashMap<String, (WorkingTreeStatus, Option<i64>)>> {
    let mut out: HashMap<String, (WorkingTreeStatus, Option<i64>)> = HashMap::new();
    if let Some(holder_key) = store.working_tree_holder(now)? {
        if let Some(scope) = crate::working_tree::WorkScope::parse(&holder_key) {
            for thread_id in store.work_scope_threads(&scope)? {
                out.insert(thread_id, (WorkingTreeStatus::Holding, None));
            }
        }
    }
    // Cache threads-per-scope-key so a queue with several entries for the SAME scope (a chain that
    // queued more than one step behind itself) resolves that scope's membership once, not once per
    // entry — the other half of "bulk, not N+1" alongside the holder read above.
    let mut scope_threads: HashMap<String, Vec<String>> = HashMap::new();
    for (i, entry) in store.list_working_tree_queue()?.into_iter().enumerate() {
        let position = i as i64 + 1;
        let threads = match scope_threads.get(&entry.scope_key) {
            Some(cached) => cached.clone(),
            None => {
                let resolved = match crate::working_tree::WorkScope::parse(&entry.scope_key) {
                    Some(scope) => store.work_scope_threads(&scope)?,
                    None => Vec::new(),
                };
                scope_threads.insert(entry.scope_key.clone(), resolved.clone());
                resolved
            }
        };
        // PLUS the entry's OWN thread, which the scope membership above can still miss (PR review,
        // final pass). `resolve_scope` ranks run above thread, so a persona `send --to` issued by a
        // session that is mid-run parks under `run:<id>` while the thread it actually belongs to is
        // a fresh 1:1 DM. Since nxf 6j6v.1xw1 a `run:` area is the SUBTREE below each of the run's
        // boards, so such a DM is usually inside it after all — it is a child of the thread its
        // session was standing in. What is left is the case where it is not: a session whose
        // position was never recorded (nothing put it in motion on a thread on this device) opens a
        // ROOT, and a root hangs under nothing. Without this line, "order 2 is queued" would be
        // visible on the send RECEIPT and nowhere else for exactly that shape.
        //
        // No second store read — `thread` already travels on the queued entry — so the "bulk by
        // construction" property above is untouched. `or_insert` keeps this SUBORDINATE to
        // everything already decided: a thread the holder's scope claims stays `Holding`, and a
        // thread an earlier queue entry claims keeps that earlier position.
        //
        // **The HOLDER's side of this used to be an unfixable asymmetry and no longer is** (nxf
        // 6j6v.fm8k, resolved by 6j6v.1xw1). The lease row stores a `scope_key` and nothing else, so
        // a holder's own ad-hoc DM could not be reached from it while the claim area was a flat set
        // of board ids — a waiting DM showed on the board and a holding one did not. A SUBTREE
        // contains it, because a71h stamps every thread a session opens as a child of the thread
        // that session is standing in, so no schema question is left.
        for thread_id in threads.into_iter().chain(entry.thread) {
            out.entry(thread_id)
                .or_insert((WorkingTreeStatus::Waiting, Some(position)));
        }
    }
    Ok(out)
}

/// Look up one thread's working-tree state in the map [`working_tree_status_by_thread`] built —
/// `(None, None)` for a thread the map does not carry, which is the ordinary case, not a missing
/// answer.
fn working_tree_fields(
    status: &HashMap<String, (WorkingTreeStatus, Option<i64>)>,
    thread_id: &str,
) -> (Option<WorkingTreeStatus>, Option<i64>) {
    match status.get(thread_id) {
        Some((state, position)) => (Some(*state), *position),
        None => (None, None),
    }
}

/// One quorum board in full (M2, the `nxc threads show` shape): the derived [`ThreadQuorum`] fields
/// **flattened** in-line, this ticket's working-tree visibility, then the thread's reply `messages`
/// in id order. `#[serde(flatten)]` keeps the byte contract "the ThreadQuorum fields PLUS
/// working-tree state PLUS the reply messages" — the quorum fields serialize in their declaration
/// order (sparse `channel_id`/`opener`/`deadline` skipped when `None`), then `working_tree`/
/// `working_tree_queue_position`, then `messages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThreadBoardView {
    #[serde(flatten)]
    pub quorum: ThreadQuorum,
    /// Whether the chain this thread belongs to currently holds the working-tree lease, is queued
    /// behind it, or the thread has no relation to it at all (nxf 6j6v.qk5b) — `null` for the last,
    /// ORDINARY case. Always present, never `#[serde(skip_serializing_if)]`: a field that only
    /// appears in the interesting case forces every reader into a case distinction, and a reader who
    /// forgets to make it never sees the interesting case — the identical review finding on
    /// `orchestration::TriggerReceipt::warnings` (ticket 6j6v.hpv8), applied here on purpose rather
    /// than repeated as a fresh finding.
    ///
    /// **Tested by key PRESENCE, not by value equality, on the ordinary (`null`) case** (PR review
    /// of this ticket): `serde_json::Value`'s `Index` returns the identical static `Value::Null` for
    /// a MISSING key as for a key present holding JSON `null` (its own documented behaviour), so
    /// `assert_eq!(obj["working_tree"], Value::Null)` alone cannot tell "additive and always
    /// rendered" from "silently dropped by a future `skip_serializing_if`". The tests instead assert
    /// `obj.as_object().unwrap().contains_key("working_tree")` first — do not "simplify" that back
    /// to a bare equality check.
    pub working_tree: Option<WorkingTreeStatus>,
    /// This thread's 1-based position in the working-tree queue, when
    /// [`working_tree`](Self::working_tree) is [`WorkingTreeStatus::Waiting`] — `None` in every
    /// other case, including the ordinary one. Always present for the same reason `working_tree` is,
    /// and tested the identical way (key presence, not value equality — see that field's doc).
    pub working_tree_queue_position: Option<i64>,
    /// **The agent side handed this task back instead of answering it** (nxf 6j6v.7qfm) — the same
    /// fact, off the same one derivation ([`ChatStore::last_reply_escalated`]), that
    /// [`StatusThread::escalated`] carries on the operation view.
    ///
    /// It is here because this read is the one a REQUESTER is told to poll: the `await` block
    /// [`crate::awaiting::Await`] hands back names `escalated` as half of
    /// [`crate::awaiting::StoppedWhen`], and a predicate naming a field that this payload does not
    /// carry would be a contract the engine cannot keep. Before this, a caller looping on
    /// `threads show --json` could see `complete: false` forever on a round that had already
    /// stopped — which is exactly the hang `stopped_when` exists to prevent.
    ///
    /// Always present, never `#[serde(skip_serializing_if)]`, for
    /// [`working_tree`](Self::working_tree)'s reason: a field that appears only in the interesting
    /// case forces every reader into a case distinction, and a reader who forgets to make it never
    /// sees the interesting case.
    ///
    /// `false` for a thread nothing has answered this turn — nothing has come back, so nothing has
    /// been handed back.
    pub escalated: bool,
    pub messages: Vec<MessageView>,
}

impl ThreadBoardView {
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("ThreadBoardView serializes")
    }
}

/// One quorum board as `nxc threads list` renders it (M2 + working-tree visibility, nxf 6j6v.qk5b):
/// the derived [`ThreadQuorum`] fields **flattened**, followed by this ticket's two additions.
/// Field order = the JSON contract: the quorum fields are byte-identical to before this ticket, so
/// an existing `--json` reader keeps finding them at the same keys; `working_tree`/
/// `working_tree_queue_position` are new keys appended after them, never replacing or renaming one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThreadListEntry {
    #[serde(flatten)]
    pub quorum: ThreadQuorum,
    /// See [`ThreadBoardView::working_tree`] — the identical field, computed by the identical
    /// [`working_tree_status_by_thread`] call, so `list` and `show` cannot disagree about one thread.
    pub working_tree: Option<WorkingTreeStatus>,
    /// See [`ThreadBoardView::working_tree_queue_position`].
    pub working_tree_queue_position: Option<i64>,
}

impl ThreadListEntry {
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("ThreadListEntry serializes")
    }
}

// ---- the operation status (nxf 6j6v.a71h §3.2/§3.3) ---------------------------------------

/// Which operations a status read is about (nxf 6j6v.a71h §3.2). The four forms of `nxc status`.
///
/// **The channel is deliberately not the anchor.** A single operation spans several channels — the
/// item's own chain crosses three — so a channel-shaped view of it shows a third of the truth. The
/// anchor is the operation, and the operation IS its root thread; the channel form exists only as an
/// entry point into one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StatusScope<'a> {
    /// Every LIVE operation in the workspace (`nxc status`) — [`StatusOperation::live`] is what
    /// that word means here, and nxf 6j6v.1vxs is where it stopped meaning "and every finished one
    /// nobody has acknowledged", which was all of them.
    Workspace,
    /// The live operations whose ROOT thread sits in this channel (`nxc status --channel <name>`).
    /// Matched against a thread's `channel_id` both literally and as `decl:<name>`, so a declared
    /// channel can be named the way a user types it without this layer loading the catalogue.
    Channel(&'a str),
    /// **Every operation the workspace holds, finished ones included** — narrowed to one channel
    /// exactly as [`Channel`](StatusScope::Channel) narrows [`Workspace`](StatusScope::Workspace)
    /// (`nxc status --all`, `nxc status --all --channel <name>`).
    ///
    /// The RECOVERY half of nxf 6j6v.1vxs, and it is what makes that item's change a separation
    /// rather than a loss: the two listing forms above stopped showing a finished operation, so
    /// there had to be a way to see one again without already knowing its root id — which is all
    /// [`Threads`](StatusScope::Threads) can be asked with. The selection is the SAME as its
    /// unfiltered neighbour's; only the liveness filter is off.
    All(Option<&'a str>),
    /// The whole tree each of these threads belongs to, from its root down, live or finished
    /// (`nxc status --thread <id>` passes one).
    ///
    /// **This variant IS `thread_quorums`** (nxf 6j6v.yr59). That read was the second name for
    /// "the same question, for this explicit set" — the coordination UI's quorum bar reading all
    /// of a project's boards at once — and the item's ruling was that the anti-N+1 PROPERTY must
    /// survive and the second name need not. So the set became a parameter of the read that had
    /// already absorbed `threads`/`thread_board`, and the property is measured rather than claimed:
    /// `tests/bulk_quorum.rs` counts the SELECT statements a two-thread read and a forty-thread
    /// read prepare, and fails if the number moves.
    ///
    /// An id no thread in this workspace carries is `not_found` naming it, exactly as the
    /// single-id form always was: a caller asking about something that is not here is told so,
    /// rather than handed the subset that happened to resolve.
    Threads(&'a [&'a str]),
}

/// The derived state of ONE thread in an operation (nxf 6j6v.a71h, DoD "the three states"). Nothing
/// is stored: all three fall out of the thread's own quorum and its position in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ThreadState {
    /// Somebody still owes a reply here (`outstanding ≠ ∅`). This is where a chain that HANGS shows
    /// up: an open thread nobody is working on any more looks exactly like this, which is why the
    /// root's own [`StatusThread::awaiting_human`] has to be told apart from it (§3.4).
    Open,
    /// Discharged: nothing is owed on this thread any more.
    Answered,
    /// **A dead end that FAILED** — the derived place where a failed consequence surfaces without
    /// anyone reporting it (nxf 6j6v.a71h's third state, sharpened by 6j6v.93zd): the answer
    /// arrived, nothing was opened out of it, nothing upstream is waiting for it — **and the parent
    /// never moved on from it**.
    ///
    /// The full rule, in one place:
    ///
    /// 1. discharged (`outstanding = ∅`) — somebody answered;
    /// 2. no child — nothing was opened out of this thread;
    /// 3. the parent ASKED someone and was answered — a declared expectation, discharged. Not merely
    ///    "the parent owes nothing", which is equally true of a parent that never asked anyone
    ///    anything and would make every dead end below an ordinary conversation an alarm;
    /// 4. the parent did NOT move after this thread's discharge —
    ///    [`ChatStore::parent_moved_after_discharge`](crate::store::ChatStore::parent_moved_after_discharge),
    ///    which carries the derivation in full.
    ///
    /// Never the ROOT, whatever the rule says: a root thread has no parent to be open, and a root
    /// that has been answered is not a dead end but the normal end — its human has it now (§3.4).
    /// That carve-out is the ONLY exception, and it exists only at the top.
    ///
    /// **Why (3) and (4) had to be added, since the shape they remove is what a reader meets most
    /// often.** a71h implemented its own DoD formula verbatim — discharged, no child, no open parent
    /// — and reported the consequence rather than leaving it to be found: a FINISHED branch and a
    /// FAILED one satisfy it alike, so in a71h §2's chain the three reviewer threads read `orphaned`
    /// from the moment the `#review` supervisor consolidated their answers into T4, while the
    /// operation was perfectly healthy. Both are dead ends; only one is a failure, and a place for
    /// failed consequences that is full of healthy threads is a place nobody reads.
    ///
    /// **It errs towards missing a failure, never towards a false alarm** — see the store read for
    /// why Lamport order makes that the structural direction rather than a preference. That is the
    /// right direction for a state whose entire worth is being trustworthy when it fires.
    Orphaned,
}

/// One thread inside an operation's tree.
///
/// **`#[non_exhaustive]`, unlike its neighbours on this seam, and deliberately so.** The reason
/// [`WorkingTreeStatus`] records for NOT using it does not transfer: that is an ENUM produced in one
/// place, where exhaustive matching is the safety net that catches an unannounced variant. This is a
/// pure OUTPUT record — nothing outside this crate has a reason to build one — and the failure mode
/// of an exhaustive struct is the opposite one: every field added to it breaks every struct literal
/// in the repo and at every embedder that pins this crate by tag. That is nxf 6j6v.dr4h, filed
/// against `ChannelDecl`/`RoleDecl` for exactly this pain, and cited by the owner as the reason not
/// to add a field to `ChannelDecl` at all.
///
/// This type gained its SECOND field (`session`, nxf 6j6v.1q6d) before it was ever released, which
/// is the evidence that it will keep growing — an operation view is where the next question about a
/// thread naturally lands. It is unreleased on this branch, so this is the last moment the marker is
/// free. Reading is untouched: fields stay public, and `..` patterns still destructure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct StatusThread {
    pub thread_id: String,
    /// **What this conversation is CALLED** (nxf 6j6v.e76c) — absent for a thread nobody named.
    ///
    /// Beside `thread_id` and `channel_id`, which is exactly where the item asks for it: those two
    /// are the only handles this projection gave a renderer, and one of them is a `sha256` prefix
    /// over two handles that no reader can do anything with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// The thread this one was opened out of — `None` on the root, which is what MAKES it the root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Distance from the operation's root, so a renderer can indent without walking the tree again.
    pub depth: usize,
    pub state: ThreadState,
    /// **The root is answered and its human has not acted on it** (nxf 6j6v.a71h §3.4). The normal
    /// end of an operation, and NOT a standstill — which is exactly why it must be distinguishable
    /// from a thread that hangs, and why it is a flag beside the state rather than a fourth state:
    /// the three states are per-thread and this is a fact about one end of the chain.
    ///
    /// `true` only when the thread is a ROOT and its expectation was declared AND discharged. A root
    /// that asked nobody is not waiting for anything, and a root still waiting on its agent is
    /// [`ThreadState::Open`] like any other. **Only ever true at the top** — no human stands inside
    /// the flow (§3.4's correction of 2026-08-14).
    ///
    /// **It no longer decides whether the operation is listed** (nxf 6j6v.1vxs). It is derived from
    /// two permanent facts — the root asked, the root was answered — so it is a LABEL on a finished
    /// operation and not a debt anybody can discharge; reading it as a debt is what made the default
    /// view grow without bound. [`StatusOperation::live`] carries the measurement and the rule that
    /// replaced it.
    pub awaiting_human: bool,
    /// **The agent side handed the task back instead of answering it** (nxf 6j6v.mqad) — the newest
    /// reply of this thread's CURRENT turn carries the escalation marker
    /// ([`crate::model::MessageKind::Escalation`]).
    ///
    /// It is here because a caller could not otherwise tell a delivered answer from a CRASH. When a
    /// spawned persona session dies — an infrastructure error, a torn SDK stream — the sidecar's
    /// teardown settles the debt it owes so the thread does not go quiet, and until nxf 6j6v.mqad it
    /// settled it with an ORDINARY reply: the requester then read `state: answered`,
    /// `awaiting_human: true`, `warnings: []`, and the only thing separating that from a finished
    /// round was a message body that happened to begin with `sidecar:`. That is prose in the place
    /// `nxc guide limits-and-safety` tells an app to branch on a field.
    ///
    /// **It does not distinguish WHY the task came back**, and must not be read as "something
    /// crashed": a member that says "I need help, or a decision" with `nxc reply --escalate` sets
    /// the same marker, deliberately — for a caller the two mean one thing, which is that no answer to
    /// this round exists and a human or a supervisor has to decide what happens next. What the
    /// escalation MEANT is in the message body, which is where a reason belongs.
    ///
    /// `false` for a thread nothing has answered this turn: nothing has come back, so nothing has
    /// been handed back. [`state`](Self::state) is what says the round is still open.
    pub escalated: bool,
    /// **The RUNTIME wrote this thread's newest reply, standing in for the agent that owed it**
    /// (nxf 6j6v.kffm) — [`crate::model::Refs::substituted`] on that message.
    ///
    /// It is here for exactly the reason [`escalated`](Self::escalated) is, one step further on. A
    /// spawned session that ends still owing its thread an answer has one posted in its name by the
    /// sidecar's teardown, so the thread never goes quiet — and until this field the ONLY thing
    /// separating that from an answer the agent wrote was a message body beginning `sidecar:`. That
    /// is prose in the place `nxc guide limits-and-safety` tells an app to branch on a field, and it
    /// is what let nxf 6j6v.kffm's whole chain run green: a role that could not execute its own
    /// `nxc reply` produced a discharged expectation, a released working copy, and a round that
    /// looked finished.
    ///
    /// **Orthogonal to `escalated`, in both directions, and reading either as the other is wrong.**
    /// A substitution for a session that DIED escalates as well; one for a session that ended
    /// cleanly without answering deliberately does not (nothing went wrong, and marking it would
    /// tell a supervisor to stop); and a live agent's own `nxc reply --escalate` is an escalation
    /// that is no substitution at all. The pair is what tells "the agent answered", "the agent
    /// handed it back" and "nobody answered and the runtime said so" apart.
    ///
    /// `false` for a thread nothing has answered this turn, exactly as its neighbour.
    pub substituted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opener: Option<String>,
    /// **The internal session of whoever is working on this thread** — the edge that lets a consumer
    /// follow its assignee's transcript WITHOUT knowing that session id (nxf 6j6v.1q6d). Feed it to
    /// [`transcript`]/[`Engine::transcript_page`](crate::engine::Engine::transcript_page) and the
    /// existing path carries on from there.
    ///
    /// Nothing new is stored for it. It is read from what a trigger and a message already leave
    /// behind: the position the trigger recorded for that session (`session_map.thread`, nxf
    /// 6j6v.a71h §3.1) while the thread still owes an answer, and the return address
    /// (`refs.session_id`) of the newest message from an expected handle once nothing is owed. That
    /// first branch is the one the item exists for — an OPEN thread is precisely one nobody has
    /// answered yet — and [`ChatStore::thread_assignee_sessions`] carries both, the guard between
    /// them, and why the answer is the assignee's rather than the requester's.
    ///
    /// **It moves with the TURN, not with the thread's lifetime** (re-review N1): on turn two of a
    /// board this names the handle still owed, never the one that already answered — the same
    /// question, asked with the same comparison, as the [`outstanding`](Self::outstanding) printed
    /// beside it.
    ///
    /// **The same field for every caller.** There is no human/agent notion here and none may be
    /// introduced — 1q6d rejected both a declaration field and a derivation from the participants,
    /// on the 2026-08-05 design's rule that a droppable identity yields context, never a
    /// restriction. An agent does not read a transcript because its `prime` does not tell it to, not
    /// because anything stops it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// **What became of the session named in [`session`](Self::session)** (nxf 6j6v.qmy6) — the
    /// fact that separates *waiting for a human* from *hung*, on the read that already says who is
    /// working here.
    ///
    /// `nxc session state` has answered this since nxf 6j6v.h383 and still does; what it could not
    /// do was answer it for a whole board at once, so a tool that wanted to know which of forty
    /// threads had died under it made forty-one calls. That is the gap h383 named in its own text
    /// and left open on purpose, and this closes it: **one call, and the two facts stand beside
    /// each other** — `session` says whose transcript to follow, this says whether anyone is still
    /// writing it.
    ///
    /// **It is a fact about ONE session, not about the thread.** That was the choice this item had
    /// to make, and it is made in favour of the field it sits next to: "is anybody still writing in
    /// this thread" is a DIFFERENT question, the one the declared-channel advance gate asks
    /// (`sessions_still_running`), and answering it here under a name that reads like the neighbour
    /// would give two fields one meaning. A thread whose assignee session ended while a second
    /// session is being resumed onto it therefore reads `ended` — which is true of the session
    /// named, and the resume will move `session` along with it.
    ///
    /// `None` exactly when [`session`](Self::session) is `None`: nothing was ever put to work on
    /// this thread on this device and nobody expected has posted in it, so there is no session to
    /// have a state.
    ///
    /// **[`SessionState::Unknown`](crate::orchestration::SessionState::Unknown) carries one more
    /// case here than it does on `nxc session state`.** Two of them are that read's own — a session
    /// killed before it could announce anything, or a runtime nobody can ask — and
    /// [`StatusReport::worker_answers_liveness`] beside it is which. The third belongs to this read
    /// alone: a report whose distinct sessions outnumber
    /// [`MAX_LIVENESS_PROBES`](crate::facade::MAX_LIVENESS_PROBES) asks the process question about
    /// none of them, and so does an id that could not have been minted here (see
    /// [`is_mintable_session_id`]). Both were added in the review of PR #444 to bound a loop that
    /// runs under the handle's lock on ids that can arrive by sync; both fail towards *nobody
    /// looked*, which is what this word already means, rather than towards a claim.
    ///
    /// That flag is on the REPORT
    /// rather than repeated per thread because it is a property of the handle's worker and not of
    /// any row, and carrying it is what stops this field being the one a reader learns to ignore:
    /// a host whose worker does not answer the process question reads `unknown` on every unended
    /// session, and is told so in the same call rather than left to infer it from a column of
    /// identical values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_state: Option<crate::orchestration::SessionState>,
    /// Who this thread expects a reply from, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub expects: Vec<String>,
    /// The subset of `expects` that has not answered the CURRENT turn (nxf 6j6v.cg8g) — empty means
    /// discharged.
    ///
    /// **Who HAS answered is this subtracted from [`expects`](Self::expects)**, which is why there
    /// is no `replied` field beside it although [`ThreadQuorum`] carries one (nxf 6j6v.yr59, where
    /// `threads` folded in here). The two lists are computed from the same turn watermark, so a
    /// third one would be a second spelling of a difference the reader can take.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub outstanding: Vec<String>,
    /// The thread's deadline has passed while it still owes a reply.
    pub stale: bool,
    /// **When the answer is due** — the absolute instant a declared channel's `timeout:` resolved
    /// to when the board was opened (nxf 6j6v.yr59; the field is [`ThreadQuorum::deadline`]).
    ///
    /// [`stale`](Self::stale) is this compared against `now`, and until yr59 the comparison was
    /// all that travelled: an app could say "overdue" and not "due in four minutes", which is the
    /// difference between a warning and a countdown. It arrives here because `threads`/
    /// `thread_board` — the two reads that did carry it — folded into this one.
    ///
    /// `None` for a board with no window at all, which is every thread whose channel declares no
    /// `timeout:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    /// **Who holds the device-local working copy, and who is queued behind them** (nxf 6j6v.qk5b,
    /// folded in from `threads`/`thread_board` by 6j6v.yr59). `None` — the ordinary case — means
    /// this thread needs the working tree not at all.
    ///
    /// It is the other half of "why is nothing happening here": [`session`](Self::session) names
    /// whoever is working on the thread, and this says whether they are working or WAITING for a
    /// checkout somebody else is holding. Derived in bulk beside the quorums
    /// ([`working_tree_status_by_thread`]): one lease read, one queue read, and one scope read per
    /// chain genuinely competing for the tree — never one per thread on the board.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_tree: Option<WorkingTreeStatus>,
    /// The 1-based position in the working-tree queue for a [`WorkingTreeStatus::Waiting`] thread;
    /// `None` for every other thread. See [`ThreadBoardView::working_tree_queue_position`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_tree_queue_position: Option<i64>,
    /// **The party that owes this thread an answer is waiting on a round it commissioned ITSELF**
    /// (nxf 6j6v.hw2t) — the open sub-threads it opened, or empty for every other thread.
    ///
    /// It is the third way a session can fail to answer its thread, and until this field the view
    /// had no word for it. Measured on 2026-09-08: a `head-of-marketing` consulted four specialists
    /// ad hoc, could not say "I am waiting on them", and reached for `--escalate` — so the one view
    /// that exists to separate *hanging* from *running* reported `NEEDS DECISION` over four working
    /// sessions. The alarm is spent on the ordinary case exactly once before it stops meaning
    /// anything.
    ///
    /// **Nothing new is stored for it and no agent declares it**: it is
    /// [`crate::awaiting::own_open_sub_round`] over the children this report already walks, taken
    /// from the same bulk quorum read as everything else on the row — see that function for the
    /// predicate and for why a derived state beats a fourth reply verb.
    ///
    /// Non-empty implies [`state`](Self::state) is [`ThreadState::Open`] — a discharged thread is
    /// waiting for nobody, which is the predicate's own first term. It implies nothing about
    /// [`escalated`](Self::escalated), and deliberately: on a board that expects several handles,
    /// one of them can hand its turn back while another is still out consulting somebody. Both
    /// facts are then true and both are worth reading.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub waiting_on_sub_round: Vec<String>,
    /// **Where the working copy stood at this thread's most recent handover** (nxf 6j6v.2af2) —
    /// [`crate::model::Refs::working_copy`], read off the newest message in the thread that carries
    /// one.
    ///
    /// It is what makes the anchor data with a READER rather than a column nothing renders (the
    /// finding nxf 6j6v.s9ex names for the parked branch next door): a message says who spoke, and
    /// this says which state of the tree they were speaking about.
    ///
    /// **The NEWEST anchored message, and not the newest REPLY.** The two are different exactly
    /// where it matters most: on a thread nobody has answered yet, the anchored message is the
    /// COMMISSION, and the tree it names is the tree the session that is still working was started
    /// in — which is what a resume needs (nxf 6j6v.npy3). See
    /// [`ChatStore::working_copy_anchors`](crate::store::ChatStore::working_copy_anchors), which is
    /// bulk like every other fact on this row.
    ///
    /// **It is the newest HANDOVER, which is not the same as the newest MESSAGE** — and the
    /// difference is a correctness question, not a nicety (independent review of PR #474, Code
    /// Quality #1). While a handover existed that did NOT stamp an anchor, this field reported an
    /// older one as if it were the state just handed over: a channel member on its second turn kept
    /// showing turn one's tree. That hole is closed by stamping every handover
    /// ([`crate::orchestration`]'s `with_working_copy` carries the criterion that decides which
    /// messages those are — named in prose rather than linked, because it is private), so the
    /// newest anchored message and the newest message that hands anything over are now the same
    /// message. What can still sit in between is
    /// traffic that hands NOTHING over, and for that the older anchor is the right answer: it is
    /// the last time this conversation described the tree, which is what the field says.
    ///
    /// `None` for a thread whose handovers recorded nothing — every message written before this
    /// existed, and every workspace whose runtime names no working copy. When it is `None`
    /// everywhere, [`StatusReport::worker_names_a_working_copy`] is what says why.
    ///
    /// **What it does NOT say is whether the tree is still there.** This is the recorded past; the
    /// comparison against the present is [`crate::anchor::Anchor::drift_from`], and it needs a git
    /// read — which is why no read verb makes it. A resume does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_copy: Option<crate::anchor::Anchor>,
    /// **The session working here stopped because the model went away, and is coming back** (nxf
    /// 6j6v.npy3) — the hold standing over [`session`](Self::session), or `None`, which is the
    /// ordinary case.
    ///
    /// It is the fact that was, until this item, readable NOWHERE: measured on a run where a weekly
    /// window demonstrably closed mid-session, `nxc session state` reported a clean `ended`, this
    /// thread read `answered`, and `escalated`, `substituted` and `stale` were all false. The whole
    /// channel record of that workspace was searched for a trace of the interruption and there was
    /// none. The system could not SAY it had been interrupted, which is a shorter sentence than
    /// "cannot continue it" and a worse one.
    ///
    /// **A fact BESIDE [`session_state`](Self::session_state), never a value of it.** An interrupted
    /// session has genuinely ended — its process is gone and it announced so — so
    /// [`SessionState::Ended`](crate::orchestration::SessionState::Ended) stays true of it and every
    /// host matching on that enum keeps working. What this adds is why, and that something is
    /// coming; it rides orthogonally exactly as [`substituted`](Self::substituted) and
    /// [`waiting_on_sub_round`](Self::waiting_on_sub_round) do.
    ///
    /// It also CORRECTS the line beside it, and that is a requirement rather than a nicety: the
    /// terminal rendering may not say *"ITS SESSION HAS ENDED, nothing is coming"* over a session
    /// that is coming back. That is the same contradiction nxf 6j6v.hw2t had to remove from that
    /// line for a caller waiting on its own sub-round, reached from a second direction.
    ///
    /// `None` exactly when [`session`](Self::session) is `None`, or when that session is not on
    /// hold. Read in bulk beside every other fact on this row
    /// ([`ChatStore::interruptions_among`](crate::store::ChatStore::interruptions_among)) — one
    /// statement for the whole report, never one per thread.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupted: Option<crate::interruption::Interruption>,
}

/// One operation: a root thread and everything that was opened out of it, across channel borders.
///
/// There is no operation RECORD anywhere — this is assembled per read from the parent edges and the
/// quorum registers (§3.3), and nothing in the store stands beside it.
/// `#[non_exhaustive]` for [`StatusThread`]'s reason — see it; the two grow together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct StatusOperation {
    /// The root thread's id, which is the operation's identity.
    pub root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// **Somebody or something is still ENGAGED here** — the tree holds an open thread, or an
    /// unanswered hand-back, or this device's working copy. It is what the two plain listing forms
    /// filter on, and therefore what `nxc status` means by "still going on".
    ///
    /// **The root's [`StatusThread::awaiting_human`] used to be a fourth term of this sum, and nxf
    /// 6j6v.1vxs struck it out.** That flag is derived from facts that never stop being true — the
    /// root asked, and the root was answered — so an operation that entered the default view
    /// through it never left again, and there is no verb with which a human says "read, done"
    /// (inventing one is 6j6v.4d2z's question about who clears what, not this one's). Measured in
    /// the proving ground after a day: thirteen operations, 411 threads, 424 lines of output, every
    /// one of the thirteen standing on this flag alone and nine of them with nothing open at all. A
    /// view that answers "is anything still running here?" with thirteen yeses and four rights is
    /// not a longer view, it is a different sentence.
    ///
    /// The four terms that remain are the ones a reader can DO something about: an open thread waits
    /// for a reply, [`needs_decision`](Self::needs_decision) wants one into the escalated thread,
    /// [`holds_working_tree`](Self::holds_working_tree) wants its opener's `withdraw` if the chain
    /// holding it is still RUNNING and they want it back on purpose — a chain that simply died is the sweep's to
    /// notice on its own, unasked — and a
    /// [`ThreadState::Orphaned`] thread anywhere in the tree is an answer nothing was done with —
    /// *not* still running, and exactly the shape of hanging this view exists to catch, which is why
    /// it counts. It is rare by construction: that state errs towards missing a failure rather than
    /// raising a false alarm, and in the 411-thread workspace measured above there was not one.
    /// A finished operation is not lost — it is
    /// [`StatusScope::All`](crate::facade::StatusScope::All) and `--thread` away, both of which
    /// still carry this field so a reader there can tell the two apart.
    pub live: bool,
    /// How many of `threads` are [`ThreadState::Open`] — the "where do we stand" number.
    pub open: usize,
    /// **Somewhere under this root a task was handed back and nobody has taken it up** (nxf
    /// 6j6v.gk9j) — any thread in the tree carrying [`StatusThread::escalated`].
    ///
    /// It is read BESIDE [`live`](Self::live) and the root's own
    /// [`StatusThread::awaiting_human`], and that pairing is the whole reason it exists: both of
    /// those say "the human is up", and they say it about two situations of completely different
    /// urgency. A finished operation waiting to be read is the normal end. An operation whose root
    /// looks exactly the same while an unanswered escalation sits underneath it is a chain that has
    /// STOPPED — and an escalation holds the working copy, so it stops the machine as well as the
    /// round. Measured in the proving ground: two escalations went unanswered for over an hour
    /// because the recipient could not tell those apart.
    ///
    /// It goes false again on its own: [`StatusThread::escalated`] follows the CURRENT turn, so a
    /// requester that re-commissions the round clears it without anything having to be reset.
    pub needs_decision: bool,
    /// **Somewhere under this root the device-local working copy is held** (nxf 6j6v.fabb) — any
    /// thread in the tree at [`WorkingTreeStatus::Holding`].
    ///
    /// Until this, finding that out meant asking `nxc threads show` about every thread in the tree
    /// one at a time and reading the `working_tree:` line — which is how a chain came to sit at
    /// `working tree: waiting (#1)` for two hours with nothing anywhere saying who it was waiting
    /// for. The per-thread field is unchanged and still says WHICH thread; this says whether to go
    /// looking at all.
    pub holds_working_tree: bool,
    /// **Somewhere under this root a session is on hold at an availability boundary** (nxf
    /// 6j6v.npy3) — any thread in the tree carrying [`StatusThread::interrupted`].
    ///
    /// It is read beside [`needs_decision`](Self::needs_decision) and means the opposite of it,
    /// which is exactly why it needs a word of its own. Before this item an exhausted quota
    /// PRODUCED a `needs_decision`: the runtime posted "I cannot carry this out" in the session's
    /// name, the thread was discharged as an escalation, and a human was called to a chain that had
    /// nothing wrong with it. Now nothing is escalated, so that flag stays false — and without this
    /// one the operation would render as an ordinary open chain with no explanation for why nobody
    /// is working in it.
    ///
    /// It goes false again on its own, like `needs_decision`: taking the operation up stamps the
    /// hold, and a stamped hold is not standing.
    pub interrupted: bool,
    /// The tree in depth-first order, root first, siblings by thread id (which is a ULID, so
    /// chronological). Deterministic.
    pub threads: Vec<StatusThread>,
    /// **This operation's unresumed parked work** (nxf 6j6v.s9ex), newest first — every unresumed
    /// [`crate::park::ParkedWork`] row whose `scope_key` names a thread in this tree.
    ///
    /// [`crate::orchestration::park_the_stranded_holder`] and
    /// [`crate::orchestration::secure_the_interrupted_tree`] both record a park under the LEASE
    /// HOLDER key ([`crate::working_tree::WorkScope::key`]) that was standing at the moment they
    /// ran — `"thread:<id>"` for the one scope that can ever hold this workspace's working copy
    /// today ([`crate::working_tree::WorkScope::Session`] owns no thread, and so no operation, to
    /// attribute a row to). This field is every such row whose `<id>` is this operation's root OR
    /// any thread beneath it, regardless of which one happened to be holding the lease at the time —
    /// an operation that parked more than once over its life, under more than one holder key, still
    /// shows every open park here, newest first across all of them.
    ///
    /// Empty for the common case — an operation that has never parked — and
    /// `#[serde(skip_serializing_if = "Vec::is_empty")]` keeps `--json` silent about it, like every
    /// other optional fact on this record.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parked: Vec<crate::park::ParkedWork>,
    /// **This operation's park was refused, and the background service is retrying it** (nxf
    /// 6j6v.8bv9) — the `park_refusals` row whose `scope_key` names a thread in this tree, attributed
    /// exactly as [`parked`](Self::parked) is.
    ///
    /// Only a refusal that can pass is ever here — a tree mid-merge (`mid_sequence`), a git command
    /// that failed (`git`) — because only those HOLD the working copy; a permanent refusal hands it
    /// on and has nothing left to retry. `detail` names what to fix, `since` is when the park was
    /// first refused, `last_tried` the latest attempt. The row goes when the park goes through or
    /// the claim ends, so its presence means the copy is still held for it.
    ///
    /// `None` for every operation whose park was never refused, and omitted from `--json` then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub park_refused: Option<crate::park::ParkRefusalNote>,
    /// **This operation's running round was WITHDRAWN and its park is still owed** (nxf 6j6v.b9nf,
    /// Integrity #2 of this item's review) — the `withdrawn_holders` row whose `scope_key` names a
    /// thread in this tree, attributed exactly as [`park_refused`](Self::park_refused) is, plus
    /// every session under its claim area that is STILL a live process right now.
    ///
    /// Before this field a pinned withdrawn holder had no visible reason anywhere: `nxc status`
    /// showed only `holds working tree` with every thread already discharged, the tick re-armed a
    /// look every [`crate::orchestration::SESSION_LIVENESS_RECHECK_SECS`] forever, and "stop it by
    /// hand" printed only when the FIRST `stop_session` call had itself errored — never when the
    /// request was delivered and simply ignored, which is the ordinary shape of a wedged sidecar or
    /// a custom worker whose stop does not land. This is the mark on the existing register (see the
    /// human rendering beside `PARK REFUSED` and `holds working tree`) and the fact a caller can act
    /// on: which sessions to go stop by hand.
    ///
    /// The row goes with the copy going on ([`crate::working_tree::forget_withdrawn_holder`]), so
    /// its presence here means the copy is still held for it. `pinned_by` is empty in the one window
    /// between the last session leaving and the next tick noticing — the marker stands but nothing
    /// blocks it any more.
    ///
    /// `None` for every operation whose round was never withdrawn, and omitted from `--json` then —
    /// a new optional field on this already-`#[non_exhaustive]` struct, so reading a report is
    /// unaffected; a struct literal built outside the crate already could not name this type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub withdrawn: Option<WithdrawnHolderStatus>,
}

/// **The `withdrawn_holders` marker, as `nxc status` shows it** (nxf 6j6v.b9nf, Integrity #2 of this
/// item's review) — [`crate::working_tree::WithdrawnHolder`] plus the one fact the marker alone
/// cannot answer: which sessions are, right now, why the tick has not parked it.
/// `#[non_exhaustive]` like its neighbours on [`StatusOperation`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct WithdrawnHolderStatus {
    /// When the round was withdrawn, UTC-normalized.
    pub withdrawn_at: String,
    /// The qualified handle of whoever withdrew it.
    pub by: String,
    /// **The sessions still pinning the claim** — every session under the withdrawn holder's claim
    /// area the store has no end for AND the worker still answers `true` for
    /// ([`crate::worker::Worker::session_is_running`]), read at report time. Empty is not "nothing
    /// is wrong" here the way it is on [`park_refused`](StatusOperation::park_refused) — it is the
    /// moment right before the next tick parks the work and hands the copy on.
    pub pinned_by: Vec<String>,
}

/// What `nxc status` answers, in every one of its forms.
///
/// `#[non_exhaustive]` like the two records inside it: leaving the ENVELOPE exhaustive while its
/// leaves are not would half-fix the problem — the next field here would still break every literal,
/// which is the whole of what the marker is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct StatusReport {
    pub operations: Vec<StatusOperation>,
    /// **Whether the worker that answered was able to answer the process question at all** (nxf
    /// 6j6v.qmy6) — [`Worker::answers_liveness`](crate::worker::Worker::answers_liveness), carried
    /// here for the reason [`SessionStateReport::worker_answers_liveness`](
    /// crate::orchestration::SessionStateReport::worker_answers_liveness) carries the same bit: it
    /// is where [`StatusThread::session_state`]'s `unknown` gets its meaning.
    ///
    /// `false` says no process question was ever put, so every unended session on this report reads
    /// `unknown` and none of them is evidence about a session. It is deliberately the SAME fact,
    /// spelled the same way, as on the session read — two reads that answer one question must not
    /// qualify it differently.
    pub worker_answers_liveness: bool,
    /// **Whether this workspace's runtime names a working copy at all** (nxf 6j6v.2af2) —
    /// [`Worker::working_copy`](crate::worker::Worker::working_copy), carried here for exactly
    /// [`worker_answers_liveness`](Self::worker_answers_liveness)'s reason, one fact further along:
    /// it is where an EMPTY [`StatusThread::working_copy`] gets its meaning.
    ///
    /// `false` says no handover in this workspace could ever record where the tree stood, so every
    /// thread on this report reads `working_copy: null` and none of that absence is evidence about a
    /// handover. A reader is told once, here, rather than left to conclude from a column of nulls
    /// that nothing is being written down.
    ///
    /// It is the DEFAULT for every worker except the shipped sidecar — a remote runtime, an
    /// in-process stub, an embedding host that starts its sessions elsewhere — so it is a host
    /// configuration fact and not a finding. It costs nothing to ask: the trait requires the answer
    /// to come promptly from local state, and no git command is run for it.
    pub worker_names_a_working_copy: bool,
}

impl StatusReport {
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("StatusReport serializes")
    }
}

/// One search hit — the same shape as the `nxc search` `--json` row (byte-parity in `tests/parity`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageHitView {
    pub message_id: String,
    pub channel_id: String,
    pub sender: String,
    pub body: String,
}

impl From<MessageHit> for MessageHitView {
    fn from(h: MessageHit) -> MessageHitView {
        MessageHitView {
            message_id: h.message_id,
            channel_id: h.channel_id,
            sender: h.sender,
            body: h.body,
        }
    }
}

impl MessageHitView {
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("MessageHitView serializes")
    }
}

/// One entry of a role session's Claude Agent SDK transcript as an app reads it (nxf epic 6wt2,
/// ticket 5bym): a stored [`TranscriptRow`](crate::transcript::TranscriptRow) with its Task-spawned
/// sub-timeline **nested** rather than flat.
///
/// `parent_tool_use_id` is deliberately NOT a field: the tree IS the answer to "whose subagent is
/// this", and carrying the raw id alongside it would give a consumer two sources for one fact. The
/// one case where that id is genuinely unrecoverable — an ORPHAN, whose parent `tool_use` is not in
/// this transcript — keeps `subagent_type` and its own `seq` position instead (see [`transcript`]).
///
/// snake_case on this wire, unlike the camelCase the sidecar emits and the store round-trips: this
/// is the facade's own contract (the shape `MessageView` and every other view here use), not the
/// T1↔T2 wire. `data` passes through verbatim — opaque here exactly as it is in the store.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptEntryView {
    pub seq: i64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_type: Option<String>,
    pub data: serde_json::Value,
    /// The entries a Task-spawned subagent produced under this `tool_use`, in `seq` order — empty
    /// for every other entry. Always serialized (never skipped when empty) so a consumer can
    /// iterate it unconditionally.
    pub subagent: Vec<TranscriptEntryView>,
}

/// One role session's transcript in full (nxf epic 6wt2, ticket 5bym): the nested [`entries`] plus
/// the `session_map` facts that join it to the rest of the world — `real_sdk_id` (what
/// `messages.refs.session_id` and a `--resume` name) and `role`. Both are `None` when the session
/// was never minted through `create_pending_session`, which is not an error: a transcript is stored
/// without a foreign key to `session_map` precisely so evidence survives an unmapped session.
///
/// [`entries`]: TranscriptView::entries
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptView {
    pub session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub real_sdk_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub entries: Vec<TranscriptEntryView>,
}

impl TranscriptView {
    /// The canonical JSON value (field order = declaration order).
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("TranscriptView serializes")
    }
}

// ---- shared rejection helpers (moved from cli.rs — one source for both seams) ----

// `require_member` stood here — the plain membership gate (spec §5.1), unknown channel →
// `not_found` before non-member → `forbidden`. `require_readable` below is that gate with the
// `public` exception spliced into the middle and has always spelled both halves out rather than
// delegating; the last caller of the plain form was the `mark_read` ack, so it went with it (nxf
// 6j6v.4d2z). The RULE is unchanged and is stated once, below.

/// The READ gate (nxf 6j6v.bd6g) — the membership gate of spec §5.1 with one exception, and exactly
/// one: a channel whose `kind` is [`CHANNEL_KIND_PUBLIC`] is readable by ANYONE. Unknown channel →
/// `not_found` first, as before, so a typo'd front door still says so instead of reading as an
/// empty room; an existing channel a non-member asks after → `forbidden`.
///
/// **This is the only gate the `public` kind moves, and it moves it in one direction.** The three
/// reads that use it — [`messages`], [`thread`], [`thread_board`] — all answer the same question
/// ("what is in this channel"), so opening one and not its neighbours would be a distinction no
/// caller could explain. What deliberately keeps plain membership instead:
///
/// * [`channels`] and [`search`] — both are membership-SCOPED enumerations rather than gates.
///   Sweeping public channels into them would change the result set of a read every existing
///   consumer already makes; discovery is [`public_channels`], its own named read. (Since nxf
///   6j6v.cs03 [`search`] resolves that membership through [`declared_seat`] — the rule
///   [`is_channel_member`] is built out of, and so the rule [`require_thread_readable`] gates with.
///   The `public` exception is the half it does not take, and [`search`]'s own doc says why.)
pub(crate) fn require_readable(store: &ChatStore, channel: &str, handle: &str) -> Result<()> {
    if !store.channel_exists(channel)? {
        return Err(NxfError::not_found(format!("no such channel: {channel}")));
    }
    if store.channel_kind(channel)?.as_deref() == Some(CHANNEL_KIND_PUBLIC) {
        return Ok(());
    }
    if !store.is_member(channel, handle)? {
        return Err(NxfError::forbidden(format!(
            "{handle} is not a member of {channel}"
        )));
    }
    Ok(())
}

/// The READ gate a THREAD reader uses — [`require_readable`] plus the two doors nxf 6j6v.v39s
/// opened, in the order they are cheapest to answer.
///
/// 1. **The declared members, read from the DECLARATION** rather than from the substrate's
///    materialised snapshot of it — see [`ChannelPolicy`] for why the file has to be the membership
///    for a declared channel, and for the bare/qualified split that keeps a board's opener a member
///    of a channel no declaration ever named it in.
/// 2. **Whoever opened the OPERATION this thread belongs to** ([`opened_the_operation`]). `nxc
///    status` shows that reader the whole tree from its root down, across every channel border an
///    agent opened along the way, and before this it could read a WORD of only the first channel:
///    the pm opened `decl:coding`, the coder opened `decl:review`, and neither of those channels
///    knows the human whose question the whole chain is answering. The relation is the one
///    `awaiting_human` is already computed from.
///
/// What it does NOT do is widen what that reader sees INSIDE a thread past the channel's declared
/// `visibility` — [`requester_for`] carries that half, and says why the operation's opener is not a
/// peer of the round it is reading.
pub(crate) fn require_thread_readable(
    store: &ChatStore,
    channel: &str,
    thread_id: &str,
    as_handle: &str,
    policy: &ChannelPolicy,
) -> Result<()> {
    if !store.channel_exists(channel)? {
        return Err(NxfError::not_found(format!("no such channel: {channel}")));
    }
    if store.channel_kind(channel)?.as_deref() == Some(CHANNEL_KIND_PUBLIC) {
        return Ok(());
    }
    if is_channel_member(store, channel, as_handle, policy)? {
        return Ok(());
    }
    if opened_the_operation(store, thread_id, as_handle)? {
        return Ok(());
    }
    Err(NxfError::forbidden(format!(
        "{handle} is not a member of {channel} and did not open the operation it belongs to",
        handle = as_handle
    )))
}

/// Whether `as_handle` is a member of `channel`, asking the DECLARATION where there is one.
///
/// For a declared channel `members:` IS the membership — the file, in both directions, with no
/// `send` in between to materialise an edit. Undeclared channel ⇒ the store alone, exactly as
/// before.
///
/// # The match is on the BARE tail, and that is required rather than incidental
///
/// An earlier draft of this comment claimed the opposite — that a qualified `origin/handle` identity
/// "is not a declared seat and is answered by the store". The code never did that, and it must not:
/// **`members:` names personas BARE while a caller's own identity is always QUALIFIED**
/// (`cli.rs`'s `caller_handle` is `format!("{origin}/{actor}")`), so a bare-only comparison would
/// refuse `local/coder` from the very channel `members: [coder]` declares it into. It would also
/// miss the item's own second case: a human written into `members: [coder, ckoch]` arrives as
/// `4jgn/ckoch`, and granting exactly that is what nxf 6j6v.v39s asks for.
///
/// So [`same_identity`] does the comparison, and the comparison is **origin-blind**. Two
/// consequences, both named rather than left to be discovered:
///
/// * **Locally it grants nothing new.** A caller already chooses its own `NXC_ACTOR`, so anyone who
///   can run `nxc` here can present the bare handle outright and hold the seat. The qualified form
///   is the same claim spelled the way the surface spells it.
/// * **Across a SYNC boundary it admits a same-named identity of another origin** — `remote/coder`
///   holds a seat `members: [coder]` declares. That is a widening over the pre-6j6v.v39s gate, and
///   it is accepted here rather than closed, for two reasons. `nxc guide limits-and-safety` already
///   states that sync is full-log and unfiltered and that "channel membership is a convention about
///   *who is asked*, not a boundary about *who can read*" — so this gate was never the boundary
///   anyone reaching the log had to pass. And deciding what an identity from another origin MEANS
///   is the access-control design (nxf 6j6v.6aza), which is deliberately not built; inventing a
///   cross-origin rule inside a read gate would be that decision taken sideways. Every origin-aware
///   variant considered here fails in the other direction on some sync topology — gating on the
///   channel's own `origin` locks out the local member of a channel a remote replica opened first.
///
/// Pinned by `operation_opener_reads.rs::a_declared_member_reads_its_own_channel_under_its_qualified_identity`,
/// so the property is deliberate rather than accidental.
///
/// # TWO readers decide by this rule, not one (nxf 6j6v.cs03)
///
/// [`require_thread_readable`] above is one, through this function; [`searchable_channels`] — which
/// resolves the channel scope [`search`] reads — is the other, and it calls [`declared_seat`]
/// DIRECTLY rather than this. Precisely stated, because the difference is the one thing a reader
/// here could get wrong: what the two share is the rule about the FILE, and the store half is where
/// they legitimately differ, because `searchable_channels` already knows the store's answer for
/// every channel it is judging and re-asking would be an N+1 (its own doc carries that).
///
/// It is deliberate and it is the fix: the text search used to answer this same question out of the
/// substrate's materialised member set and gave a different answer — see [`search`] for the three
/// ways they disagreed. So a change to [`declared_seat`] changes what BOTH readers admit, which is
/// the property that keeps them from drifting apart again; what each reader adds on top of it (the
/// `public` door, the operation's opener) is its own and is written at its own site.
fn is_channel_member(
    store: &ChatStore,
    channel: &str,
    as_handle: &str,
    policy: &ChannelPolicy,
) -> Result<bool> {
    match declared_seat(policy, as_handle) {
        DeclaredSeat::Declared => Ok(true),
        DeclaredSeat::AskTheStore => store.is_member(channel, as_handle),
        DeclaredSeat::Refused => Ok(false),
    }
}

/// **What the DECLARATION alone settles about one reader's membership** — the pure half of
/// [`is_channel_member`], and the whole of the rule that reads a `channels.yaml`.
///
/// It is split out for the reason [`visible_under`] is: the rule has TWO callers with different
/// amounts of knowledge about the STORE. [`is_channel_member`] holds one channel and asks the store
/// when the file does not settle it; [`searchable_channels`] holds the caller's whole materialised
/// member set and therefore already KNOWS the store's answer for every channel in it — asking again,
/// once per channel, would be an N+1 over a fact it has just read. Both must mean the same thing by
/// "the file says so", so the SHAPES stay apart and the RULE is shared (nxf 6j6v.cs03).
enum DeclaredSeat {
    /// `members:` names this reader: a member, whatever the substrate holds. The direction where an
    /// edit to the file reaches a reader with no `send` in between.
    Declared,
    /// The file does not settle it, and the store answers exactly as it always did: either NO
    /// declaration names the channel (a plain channel, a DM, one that arrived by sync), or one does
    /// and the reader arrives QUALIFIED — which is not a declared seat, which is why no
    /// `channels.yaml` can revoke it (see [`is_channel_member`]'s own doc for that split).
    AskTheStore,
    /// A declaration names the channel, the reader is BARE, and the file does not name it: struck, or
    /// never seated. The grow-only substrate must not overrule the file.
    Refused,
}

fn declared_seat(policy: &ChannelPolicy, as_handle: &str) -> DeclaredSeat {
    let Some(declared) = policy.members.as_deref() else {
        return DeclaredSeat::AskTheStore;
    };
    if declared.iter().any(|m| same_identity(Some(as_handle), m)) {
        DeclaredSeat::Declared
    } else if as_handle.contains('/') {
        DeclaredSeat::AskTheStore
    } else {
        DeclaredSeat::Refused
    }
}

/// Whether `as_handle` opened the OPERATION `thread_id` belongs to — the opener of the root of its
/// tree ([`ChatStore::thread_root`]), which is the id `nxc status` groups that whole tree under.
///
/// **Fail-closed on an ancestry this workspace has not finished folding.** The walk ends at the
/// topmost thread that has a `threads` row; a parent that has not arrived yet has none, so it has no
/// opener either and this answers `false` — a reader is never handed a subtree because half its
/// chain is still in flight.
///
/// **A second reader decides by it since nxf 6j6v.ezbr**: [`crate::orchestration::withdraw`], for
/// which it is not a door but the whole bar — the owner's decision of 2026-09-20 is that only the
/// caller who ran the operation's first `send` may take it back. One rule for both, so "whoever
/// opened this" cannot come to mean two things.
pub(crate) fn opened_the_operation(
    store: &ChatStore,
    thread_id: &str,
    as_handle: &str,
) -> Result<bool> {
    let root = store.thread_root(thread_id)?;
    Ok(same_identity(
        store.thread_opener(&root)?.as_deref(),
        as_handle,
    ))
}

/// Reject a U+001F ([`SEP`]) byte in a user-supplied identifier BEFORE it reaches a composite
/// `channel{SEP}handle` / `consumer{SEP}channel` op id, where the store's `split2` invariant would
/// otherwise trip an internal `assert!` and crash the process with a raw panic. The contract is that
/// a caller never gets a raw panic — a bad identifier is a structured `validation` error.
pub(crate) fn check_no_sep(label: &str, value: &str) -> Result<()> {
    if value.contains(SEP) {
        return Err(NxfError::validation(format!(
            "{label} must not contain the U+001F (unit separator) byte"
        )));
    }
    Ok(())
}

// ---- reads -----------------------------------------------------------------

/// The caller's channels — the app's lane read (be9y). ONE SQLite round-trip regardless of channel
/// count: `list_member_channels` is a single query with a correlated member-count sub-select. It
/// was two until nxf 6j6v.4d2z, the second being the unread fold this joined against; the counts
/// are gone from [`ChannelView`] and so is the read that produced them.
pub fn channels(store: &ChatStore, handle: &str) -> Result<Vec<ChannelView>> {
    Ok(store
        .list_member_channels(handle)?
        .into_iter()
        .map(ChannelView::from)
        .collect())
}

/// Every `public` channel in the workspace, channel-id sorted — the discovery read (nxf 6j6v.bd6g).
/// **Needs no membership**: a front door nobody has joined is still a front door, and finding one is
/// the prerequisite for knocking on it.
///
/// Deliberately its OWN read rather than a widening of [`channels`], which keeps meaning "the
/// channels I am in". What travels is exactly what a [`ChannelView`] carries — id, name, kind, the
/// member COUNT and `degraded` — never the member handles and never a message body. Reading what is
/// behind the door is [`messages`], a separate named call that `public` also opens.
///
/// **It took a `handle` until nxf 6j6v.4d2z and does not any more.** The parameter existed for one
/// reason — decorating each row with that caller's per-channel unread — and it was ALWAYS `0` here,
/// because a non-member had no read state (which is what made the counts the wrong thing to carry
/// on a discovery read; [`crate::persona::FrontDoor`] said so and dropped them a fortnight
/// earlier). With the counts gone the argument was a parameter nothing read, which is the same
/// shape as the apparatus this removal is about, one size down.
pub fn public_channels(store: &ChatStore) -> Result<Vec<ChannelView>> {
    Ok(store
        .list_public_channels()?
        .into_iter()
        .map(ChannelView::from)
        .collect())
}

/// The workspace's PUBLIC channels as DIRECTORY entries — the discovery read as
/// [`crate::persona::Directory`] carries it since nxf 6j6v.yr59, with no acting handle and
/// therefore no `unread` (the argument is on [`crate::persona::FrontDoor`]).
///
/// The ONE place both surfaces get their doors from — [`crate::engine::Engine::directory`] and
/// `nxc list` — so the record they serve cannot drift, which is what lets the parity differential
/// compare them byte for byte. Same store read as [`public_channels`] above; what differs is the
/// projection.
pub fn front_doors(store: &ChatStore) -> Result<Vec<crate::persona::FrontDoor>> {
    Ok(store
        .list_public_channels()?
        .into_iter()
        .map(|c| crate::persona::FrontDoor {
            channel_id: c.channel_id,
            name: c.name,
            members: c.members,
        })
        .collect())
}

// `decorate_with_unread` stood here — the shared body of `channels`/`public_channels` that joined
// a channel-row list with the caller's per-channel unread counts. It went with the counts
// themselves (nxf 6j6v.4d2z); the projection it did the rest of is `ChannelView::from` below.
//
// `inbox(store, consumer, include_next_session)` stood here too — the unread derivation of spec
// §4, the one thing that ever read the cursor. Its last caller was `prime`, which no longer carries
// an unread half at all.

impl From<crate::store::ChannelRow> for ChannelView {
    fn from(c: crate::store::ChannelRow) -> ChannelView {
        ChannelView {
            channel_id: c.channel_id,
            name: c.name,
            kind: c.kind,
            members: c.members,
            degraded: c.degraded,
        }
    }
}

/// All messages in a channel, id-ordered — gated by [`require_readable`]: `not_found` for an
/// unknown channel, `forbidden` for a non-member of a `group`/`direct` one, and OPEN to anyone on a
/// `public` channel (nxf 6j6v.bd6g — this is the read a project's front door exists for).
pub fn messages(store: &ChatStore, channel: &str, as_handle: &str) -> Result<Vec<MessageView>> {
    require_readable(store, channel, as_handle)?;
    store
        .messages_in_channel(channel)?
        .into_iter()
        .map(MessageView::try_from)
        .collect()
}

/// A thread assembled from the messages sharing its id (M1). Resolves the thread's channel (via the
/// `threads` view, else via a stamped message), gates on that channel via [`require_readable`] (so a
/// thread in a `public` channel reads without membership), and returns the ordered messages with an
/// `opener`/`created` derived from the first. `not_found` if no such thread.
///
/// # It carries the channel's `visibility` policy since nxf 6j6v.yr59
///
/// `visibility` is the SAME explicit capability [`thread_board`] takes, applied through the same
/// [`filter_board_messages`] — read that function's doc for exactly what a `RequesterOnly` reader
/// does and does not see, and for the known multi-turn defect it carries.
///
/// It arrived here because yr59 cut the seam to ONE message reader and this is the one that
/// survived. `thread_board` was the reader that applied the policy and `thread` was the reader
/// beside it that did not — which was harmless only while both existed, because an app that wanted
/// the filter could reach for the other one. Leaving it off the survivor would have made a
/// CONSOLIDATION widen what a non-requester may read, which is the one thing a consolidation must
/// not do.
///
/// **The caller supplies the policy; it is not looked up here.** Resolving what a real `channel_id`
/// DECLARES means loading the declaration catalogue, which this layer deliberately does not do —
/// [`crate::engine::Engine::thread`] and `cli.rs`'s reads do it above, both through
/// [`crate::channel::declared_policy`], and both fall back to [`ChannelPolicy::undeclared`] for a
/// thread whose channel no declaration names (there is nothing to look up, and behaviour for it is
/// unchanged).
///
/// # The gate reads the declaration too, and the tree opens it — nxf 6j6v.v39s
///
/// [`ChannelPolicy`] carries the declared `members:` beside the visibility, because for a DECLARED
/// channel the file is the membership: an edit reaches this read directly instead of waiting for the
/// next `send` to materialise it. And whoever opened the OPERATION this thread belongs to may read
/// it whatever channel it sits in — see [`require_thread_readable`] for both doors, and
/// [`requester_for`] for why that reader is not a peer of the round it is reading.
pub fn thread(
    store: &ChatStore,
    thread_id: &str,
    as_handle: &str,
    policy: impl Into<ChannelPolicy>,
) -> Result<ThreadView> {
    let policy = policy.into();
    let channel = store
        .thread_channel(thread_id)
        .or_else(|| store.thread_channel_via_message(thread_id))
        .ok_or_else(|| NxfError::not_found(format!("no such thread: {thread_id}")))?;
    require_thread_readable(store, &channel, thread_id, as_handle, &policy)?;
    let messages: Vec<MessageView> = store
        .messages_in_thread(thread_id)?
        .into_iter()
        .map(MessageView::try_from)
        .collect::<Result<_>>()?;
    let opener = messages.first().map(|m| m.sender.clone());
    let created = messages.first().and_then(|m| m.created.clone());
    // The REQUESTER, resolved exactly as `thread_board` resolves it — usually the thread's own
    // opener, but a supervisor-opened member thread inherits its parent's, and the reader who opened
    // the whole operation is never a peer of the round it is reading (see `requester_for`).
    let requester = requester_for(store, thread_id, opener.as_deref(), as_handle)?;
    let messages =
        filter_board_messages(messages, as_handle, requester.as_deref(), policy.visibility);
    Ok(ThreadView {
        thread_id: thread_id.to_string(),
        channel_id: channel,
        opener,
        created,
        messages,
    })
}

/// The caller's quorum boards (M2 `nxc threads list`): every thread in a channel `consumer` is a
/// member of, with its bulk-derived [`ThreadQuorum`] state PLUS working-tree visibility (nxf
/// 6j6v.qk5b). Membership-scoped and bulk by construction — ONE enumeration query
/// ([`member_thread_ids`](ChatStore::member_thread_ids)), ONE
/// [`thread_quorums`](ChatStore::thread_quorums), and ONE [`working_tree_status_by_thread`] (itself
/// O(distinct scopes), never O(threads) — see that function's own doc). `now` drives only the
/// clock-dependent `stale` flag (§4.3); the deterministic `(channel_id, thread_id)` order is
/// `thread_quorums`'.
pub fn threads(
    store: &ChatStore,
    consumer: &str,
    now: &str,
    channel: Option<&str>,
) -> Result<Vec<ThreadListEntry>> {
    let ids = store.member_thread_ids(consumer, channel)?;
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let quorums = store.thread_quorums(&id_refs, now)?;
    let status = working_tree_status_by_thread(store, now)?;
    Ok(quorums
        .into_iter()
        .map(|quorum| {
            let (working_tree, working_tree_queue_position) =
                working_tree_fields(&status, &quorum.thread_id);
            ThreadListEntry {
                quorum,
                working_tree,
                working_tree_queue_position,
            }
        })
        .collect())
}

/// One quorum board in full (M2 `nxc threads show`): the derived [`ThreadQuorum`] PLUS the reply
/// messages in id order. Resolves the thread's channel the SAME way [`thread`] does (the `threads`
/// view, else a stamped message) — `not_found` if neither — then membership-gates on that channel
/// (`forbidden` a non-member). A thread that exists only through a stamped message (M1-style, no
/// `threads` row) has no quorum row, so its board is synthesized with empty expectations and the
/// opener taken from the first message. `now` drives only the clock-dependent `stale` flag (§4.3).
///
/// `visibility` (6j6v.xrk3) applies a channel's declared visibility policy to `messages` ONLY —
/// see [`filter_board_messages`] for exactly what that does and does not filter. This is a
/// CAPABILITY the caller supplies explicitly; resolving which `Visibility` a real channel_id
/// actually has (from a declared `channels.yaml`) is a later ticket's job (6j6v.cahk) — no such
/// name→channel_id resolution exists yet.
pub fn thread_board(
    store: &ChatStore,
    thread_id: &str,
    now: &str,
    as_handle: &str,
    policy: impl Into<ChannelPolicy>,
) -> Result<ThreadBoardView> {
    let policy = policy.into();
    let channel = store
        .thread_channel(thread_id)
        .or_else(|| store.thread_channel_via_message(thread_id))
        .ok_or_else(|| NxfError::not_found(format!("no such thread: {thread_id}")))?;
    require_thread_readable(store, &channel, thread_id, as_handle, &policy)?;
    let messages: Vec<MessageView> = store
        .messages_in_thread(thread_id)?
        .into_iter()
        .map(MessageView::try_from)
        .collect::<Result<_>>()?;
    let quorum = match store.thread_quorum(thread_id, now)? {
        Some(q) => q,
        // A message-only thread (no first-class `threads` row): synthesize an empty board so `show`
        // still renders its channel, opener, and messages.
        None => ThreadQuorum {
            thread_id: thread_id.to_string(),
            channel_id: Some(channel),
            opener: messages.first().map(|m| m.sender.clone()),
            expects: Vec::new(),
            replied: Vec::new(),
            outstanding: Vec::new(),
            complete: false,
            deadline: None,
            stale: false,
            // A message-only thread has no `threads` row, so there is no register to have named it
            // (nxf 6j6v.e76c) — the same answer a real row that was never named gives.
            name: None,
            // Nothing here to hold: without a row there is no obligation for an action to follow.
            held: false,
        },
    };
    let requester = requester_for(store, thread_id, quorum.opener.as_deref(), as_handle)?;
    let messages =
        filter_board_messages(messages, as_handle, requester.as_deref(), policy.visibility);
    let status = working_tree_status_by_thread(store, now)?;
    let (working_tree, working_tree_queue_position) = working_tree_fields(&status, thread_id);
    Ok(ThreadBoardView {
        quorum,
        working_tree,
        working_tree_queue_position,
        // `None` — nothing has answered this turn — reads `false`: nothing came back, so nothing
        // was handed back. The same collapse `StatusThread::escalated` makes from the same read.
        escalated: store.last_reply_escalated(thread_id)?.unwrap_or(false),
        messages,
    })
}

/// Who counts as the REQUESTER of `thread_id` for the `requester_only` visibility policy.
///
/// Normally the thread's own opener. The exception is a MEMBER thread of a two-level channel (nxf
/// 6j6v.pf6j), whose opener is the reserved supervisor identity — engine machinery, which can never
/// be a reader. Without this the requester that commissioned the channel would be locked out of the
/// very answers `requester_only` exists to reserve FOR it: before the two levels those answers sat
/// in the board it opened itself, and it saw them unfiltered. So a supervisor-opened thread inherits
/// its parent's opener, which is that same requester, and the policy keeps meaning exactly what it
/// meant.
///
/// Falls back to the thread's own opener whenever the parent is unknown to this workspace — the
/// closed direction, since an unresolvable requester must never widen a `requester_only` view.
fn requester_of(
    store: &ChatStore,
    thread_id: &str,
    opener: Option<&str>,
) -> Result<Option<String>> {
    let own = opener.map(str::to_string);
    let Some(handle) = opener else {
        return Ok(own);
    };
    if handle.rsplit_once('/').map(|(_, bare)| bare) != Some(crate::channel::SUPERVISOR_HANDLE) {
        return Ok(own);
    }
    let Some(parent) = store.thread_parent(thread_id)? else {
        return Ok(own);
    };
    // CLOCK-FREE, and that is the point of the read it uses (nxf 6j6v.yr59). This used to go
    // through `thread_quorums`, which derives `stale` and therefore takes a `now` this question has
    // no use for — so the call site handed it `""` and apologised for it in a comment. Worse, it
    // put the whole filter out of reach of a reader that has no clock, which is exactly what
    // [`thread`] is. `ChatStore::thread_opener` answers the same column with one point read.
    Ok(store.thread_opener(&parent)?.or(own))
}

/// The requester a channel's `visibility` policy is applied FOR, as THIS reader stands to the thread
/// (nxf 6j6v.v39s) — [`requester_of`] plus the one reader that round has no standing over.
///
/// **The reader who opened the OPERATION is not a peer of the round it is reading.** `requester_only`
/// is a rule BETWEEN the participants of one round: it reserves a member's answer for whoever asked
/// the round, so the other members of that same round do not read each other's work (nxf 6j6v.yr59
/// closed exactly that door, and this does not reopen it — a member is still not the requester of
/// its neighbour's answer). Whoever opened the operation is not in the round at all. It is the party
/// the entire chain answers to: every consolidation on the way up is delivered towards it, and
/// `awaiting_human` is computed from that same relation. Standing the round's peer rule between that
/// reader and the answers it commissioned is what left it, in the item's own words, seeing the shape
/// and not a word.
///
/// This is [`requester_of`]'s own argument at one more level. That function already lets a requester
/// INHERIT through a supervisor-opened member thread, because otherwise "a two-level channel would
/// lock the requester out of the answers `requester_only` exists to reserve for it". The tree is
/// deeper than two levels the moment an agent commissions a channel of its own, and the same
/// lock-out then falls on the human at the top.
///
/// The declared policy is otherwise untouched: for every reader that is neither the round's
/// requester nor the operation's opener, this returns exactly what [`requester_of`] returns.
fn requester_for(
    store: &ChatStore,
    thread_id: &str,
    opener: Option<&str>,
    as_handle: &str,
) -> Result<Option<String>> {
    let requester = requester_of(store, thread_id, opener)?;
    if same_identity(requester.as_deref(), as_handle) {
        return Ok(requester);
    }
    if opened_the_operation(store, thread_id, as_handle)? {
        return Ok(Some(as_handle.to_string()));
    }
    Ok(requester)
}

/// Apply a channel's declared `visibility` policy (6j6v.xrk3) to one reader's view of a thread's
/// `messages`. `AllMembers` is a no-op — every member sees every reply. `RequesterOnly`: the
/// REQUESTER — the third parameter, which is USUALLY the thread's own opener (the already-derived
/// [`ThreadQuorum::opener`] or its synthesized-fallback equivalent) but for a supervisor-opened
/// member thread is its PARENT's opener, resolved by [`requester_of`] because the supervisor is
/// machinery and can never be a reader — sees everything unfiltered; anyone else sees only the
/// opening message (the thread's first message, chronologically — `messages` arrives already
/// ordered lamport/site/message_id) plus their OWN messages, in their original relative order.
/// Every other member's reply is dropped from THIS reader's view only: the underlying store is
/// never touched, so a different reader's own call gets its own correctly-filtered view.
///
/// Only message CONTENT is filtered here. `ThreadQuorum`'s `replied`/`outstanding`/`expects` (the
/// who-has/hasn't-replied handle lists) are computed separately, upstream in [`thread_board`], and
/// are NEVER touched by this function — the epic spec (§4.2) scopes `requester_only` to hiding
/// reply CONTENT, not the quorum-completion bookkeeping that the mechanism itself depends on.
///
/// # The "opening message" is turn ONE's request — a known defect, parked
///
/// The opening message is kept for a non-opener because it stands for the REQUEST: a member has to
/// see what it was asked, or `requester_only` would hide the task along with the other answers. That
/// reasoning is a LIFETIME read of something that was said, and it holds only while the thread's
/// first message really is the current request.
///
/// **On a multi-turn thread it is not.** Since the two-level channel (nxf 6j6v.pf6j) a member's
/// thread gets a fresh task per turn — the supervisor re-declares and posts a new one — so from turn
/// two on, a member reading its own thread through `nxc threads show` under `requester_only` sees
/// turn ONE's task and not the one it currently owes an answer to. It still RECEIVES the right task
/// (it arrives in the resume text the trigger carries), so nothing stalls; what is wrong is the read.
///
/// Parked with the owner rather than fixed here, because the fix is a policy decision — either the
/// filter learns the turn watermark, or `requester_only` stops applying to the supervisor's own task
/// messages. It is written down at the site, and named in `orchestration.rs`'s scoping rule as the
/// one member of that category which FAILS the category's own admission test, so that a later read
/// of the same shape is judged a defect rather than excused by this one.
///
/// `as_handle` may be either a fully qualified `origin/handle` chat identity (a message `sender`/
/// `opener` always is) OR a bare declared-role handle (`ensure_declared_channel` records a declared
/// channel's `members` bare, the SAME bare/qualified split `SYNTHESIS_HANDLE`'s own doc warns about)
/// — [`same_identity`] treats either shape as naming the same participant, so wiring this up to the
/// REAL declared-channel read paths (independent review, Code Quality #1/Integrity #3, High) can't
/// regress a declared member's own visibility of their own reply merely because the caller passed
/// the bare form.
fn filter_board_messages(
    messages: Vec<MessageView>,
    as_handle: &str,
    requester: Option<&str>,
    visibility: Visibility,
) -> Vec<MessageView> {
    let reader_is_requester = same_identity(requester, as_handle);
    // `messages` is already ordered lamport,site,message_id, so index 0 IS the opening message.
    messages
        .into_iter()
        .enumerate()
        .filter(|(i, m)| {
            visible_under(
                visibility,
                reader_is_requester,
                same_identity(Some(&m.sender), as_handle),
                *i == 0,
            )
        })
        .map(|(_, m)| m)
        .collect()
}

/// **The declared-`visibility` rule itself, in one place** — the four ways a message reaches a
/// reader, and the only statement of them (nxf 6j6v.px98 review, Code Quality #1).
///
/// It exists because the rule now has TWO callers with different input shapes:
/// [`filter_board_messages`] holds a whole ordered thread and can see which message is the opening
/// one by position, while [`hit_is_visible`] judges one search hit at a time and has to look that
/// up. Writing the rule out at both sites is exactly the shape nxf 6j6v.px98 was: two readers, two
/// answers to "may I see this?" — and a rule stated twice is a rule that can be changed once. So
/// the SHAPES stay apart and the RULE is shared, which is the only half that must not drift.
///
/// * `AllMembers` — nothing is reserved, so nothing is filtered.
/// * The reader IS the round's requester — the party the answers are reserved FOR
///   ([`requester_for`] decides who that is, including the operation's opener and the parent's
///   opener on a supervisor-opened member thread).
/// * The message is the reader's OWN — nobody is kept from their own words.
/// * The message OPENS the thread — it stands for the REQUEST, and hiding it would hide the task
///   along with the other answers. See [`filter_board_messages`] for the known multi-turn defect
///   this last one carries.
fn visible_under(
    visibility: Visibility,
    reader_is_requester: bool,
    is_own_message: bool,
    is_opening_message: bool,
) -> bool {
    visibility == Visibility::AllMembers
        || reader_is_requester
        || is_own_message
        || is_opening_message
}

/// Whether `qualified` (a stored `origin/handle` chat identity — a message `sender` or a thread's
/// `opener`) names the SAME participant as `as_handle`: either the identical string, or `as_handle`
/// is that identity's BARE tail (the shape declared-channel membership is recorded in). See
/// [`filter_board_messages`]'s own doc for why both shapes must be accepted here.
fn same_identity(qualified: Option<&str>, as_handle: &str) -> bool {
    match qualified {
        Some(q) => q == as_handle || q.rsplit_once('/').map(|(_, bare)| bare) == Some(as_handle),
        None => false,
    }
}

/// Body-substring search over the caller's channels — the CLI's `search` derivation, verbatim.
///
/// # It applies the channel's declared `visibility` since nxf 6j6v.px98
///
/// The SAME rule [`thread`] applies, through the same [`filter_board_messages`] semantics, so the
/// two readers on this seam cannot answer "may I see this?" differently. They used to: `search`
/// joined the caller's live MEMBERSHIPS and nothing else, so on a `requester_only` board it handed
/// a member a body `thread` withheld from that same caller — the residue of nxf 6j6v.yr59, where
/// `thread` gained the filter and the reader beside it did not. The owner ruled on 2026-08-23 that
/// `visibility` is an ACCESS rule and not a display rule ("Versprechen für px98 und yxsa"), which
/// makes the wider answer a broken confidentiality promise rather than an inconsistency.
///
/// **Why the filter is here and not in the store, which is the shape the coordinator preferred.**
/// `visibility` is not IN the substrate: it is a field of `channels.yaml`, resolved through
/// [`declared_policy`](crate::channel::declared_policy) from a catalogue the store layer
/// deliberately does not load (see [`thread`]'s own note on that split). A store that enforced it
/// would have to grow a declaration reader, which is the dependency this layering exists to
/// refuse. So the catalogue TRAVELS, exactly as a single [`ChannelPolicy`] travels into [`thread`]
/// — `channels` here is the whole of it, because a search answers across channels and each hit is
/// judged under its own channel's declaration. `nxf 6j6v.yxsa` asks the same question for the
/// transcript reads and is NOT answered here.
///
/// **What it costs, since the anti-N+1 question is the one this reopened.** Nothing at all for a
/// channel that declares `all_members` or that no declaration names: the verdict comes from the
/// in-memory catalogue and the hit is kept without touching the database. A `requester_only`
/// channel costs the per-thread resolution [`thread`] pays once — [`requester_for`] plus one
/// opening-message read ([`ChatStore::thread_opening_message`]) — and it is paid **once per
/// distinct thread among the hits**, not once per hit, and never for a hit the caller wrote
/// itself. A search that finds fifty of its own lines in one board reads nothing extra. Resolving
/// WHICH channels are searched costs one more read on top of that — one, whatever the caller's
/// channel count, see [`searchable_channels`].
///
/// # It applies the channel's declared MEMBERSHIP since nxf 6j6v.cs03
///
/// The residual px98 named here rather than leaving it to be found a second time, and it became the
/// next finding exactly as predicted. The two readers agreed about `visibility` and still gave two
/// answers to the question underneath it — "am I in this channel at all?". [`thread`] gates through
/// [`require_thread_readable`], which for a DECLARED channel asks the DECLARATION: "the file is the
/// membership, in both directions, with no `send` in between" (nxf 6j6v.v39s). This read joined the
/// substrate's materialised member set instead, which `ensure_declared_channel` only ever ADDS to.
/// Three disagreements followed, and the third is the one a user met first:
///
/// * a handle STRUCK from a `channels.yaml` `members:` list kept finding bodies here after `thread`
///   had stopped serving them — the same confidentiality promise px98 was about, one rule over;
/// * a handle just WRITTEN INTO `members:` found nothing until the next `send` materialised it, so
///   v39s's "an edit reaches this read directly" held for one reader only;
/// * and on a declared channel this answered **"no matches" to every member that had not itself
///   opened a round there, including for that member's own words** — measured in a live `nxc` run
///   while px98 was being verified, not deduced. A caller arrives QUALIFIED (`cli.rs`'s
///   `resolve_consumer` falls back to `<origin>/<actor>`), `ensure_declared_channel` records a
///   declared channel's `members:` BARE and additionally the AUTHOR of the send, and the join
///   compared those two strings for equality. So the sender searched its own board and everybody who
///   had only REPLIED matched no membership row at all. The verb is in `prime`'s command reference,
///   so it is taught to every agent this workspace primes.
///
/// The scope is [`searchable_channels`] now, and it decides through [`declared_seat`] — the rule
/// [`is_channel_member`] is built out of, which is what [`require_thread_readable`] gates with. One
/// membership rule, not two implementations of one rule; `ChatStore::search_messages` takes the
/// resolved scope and no longer knows what a member is.
///
/// **What deliberately does NOT travel with it — two doors, both left shut on purpose.**
///
/// * **The `public` kind's open door.** [`require_thread_readable`] lets ANYONE read a `public`
///   channel; this read stays membership-scoped there, which is `public_channel.rs`'s
///   `the_membership_scoped_reads_do_not_change_their_result_set`: a body-substring read that swept
///   in every front door would change the hit list of every caller that already searches. Discovery
///   is [`public_channels`], its own named read.
/// * **The OPERATION's opener** ([`opened_the_operation`]), who reads any thread in its tree
///   whatever channel border it sits behind. An operation-wide text search is a CAPABILITY question
///   and not a leak — a widening of what `search` MEANS, which is the owner's to decide and not this
///   function's to take sideways. So `search` is channel-membership-scoped, deliberately, and that
///   sentence now stands where a user reads about the verb (`nxc guide commands`) and not only here.
///   Note the operation's opener is not shut out of a round it commissioned inside a channel it IS a
///   member of: [`requester_for`] still gives it the requester's unfiltered view there.
///
/// **The bare/qualified split survives untouched**, because it is [`is_channel_member`]'s and not
/// this read's: a QUALIFIED identity that opened a board in a declared channel keeps it although no
/// `members:` names it, and none may revoke it. That is why "the declaration decides" is stated of
/// the DECLARED SEAT — a bare handle is a member exactly when the file says so, in both directions.
pub fn search(
    store: &ChatStore,
    handle: &str,
    query: &str,
    channels: &[ChannelDecl],
) -> Result<Vec<MessageHitView>> {
    let hits = store.search_messages(&searchable_channels(store, handle, channels)?, query)?;
    let mut visibility: HashMap<String, Visibility> = HashMap::new();
    let mut boards: HashMap<String, SearchBoard> = HashMap::new();
    let mut out = Vec::with_capacity(hits.len());
    for hit in hits {
        if hit_is_visible(store, handle, &hit, channels, &mut visibility, &mut boards)? {
            out.push(MessageHitView::from(hit));
        }
    }
    Ok(out)
}

/// The channels [`search`] reads: every channel `handle` is a member of, membership decided by the
/// one rule the thread reader gates with (nxf 6j6v.cs03 — see [`search`] for what that closed).
///
/// **Two sources, one rule** — [`declared_seat`], which is [`is_channel_member`]'s own — and that is
/// what makes the item's two directions one fix rather than two:
///
/// * the substrate's live member set, through [`ChatStore::list_member_channels`] (the one remaining
///   expression of it, rather than the second copy of its `membership_adds` minus
///   `membership_removes` subquery that `search_messages` used to inline; the decoration that read
///   carries — name, kind, member count — is discarded, and one query that already exists beats a
///   second one saying the same thing). Every id it returns IS a live membership row for this
///   handle, so `AskTheStore` is answered before it is asked and only a `Refused` seat drops out:
///   that is the direction where a handle STRUCK from a declaration loses a channel the grow-only
///   substrate still lists it in;
/// * every DECLARED channel, of which only a `Declared` seat can add one — `AskTheStore` would find
///   nothing there by construction, the store having materialised no membership at all. That is the
///   other direction: `members:` names this caller in a channel nothing has sent to yet, and the
///   edit reaches this read with no `send` in between.
///
/// **Exactly one database read**, therefore, whatever the caller's channel count: the verdicts come
/// from the in-memory catalogue. Asking [`is_channel_member`] per candidate instead would re-ask the
/// store, once per channel, for the fact `list_member_channels` has just established — the N+1 this
/// read's own cost note exists to refuse.
///
/// A declared channel nobody has ever sent to survives into the scope and finds nothing, which is
/// correct rather than sloppy and costs one bound parameter: the substrate holds no message in it to
/// find. The scope is deterministic (a [`BTreeSet`], so channel-id ordered), and an empty one is
/// answered by `search_messages` itself without a query.
fn searchable_channels(
    store: &ChatStore,
    handle: &str,
    channels: &[ChannelDecl],
) -> Result<Vec<String>> {
    let mut scope: BTreeSet<String> = store
        .list_member_channels(handle)?
        .into_iter()
        .map(|c| c.channel_id)
        .filter(|id| {
            !matches!(
                declared_seat(&declared_policy(channels, Some(id)), handle),
                DeclaredSeat::Refused
            )
        })
        .collect();
    for decl in channels {
        let id = format!(
            "{}{}",
            crate::orchestration::DECLARED_CHANNEL_PREFIX,
            decl.name
        );
        // Through `declared_policy` rather than off `decl` directly, so a catalogue that declares one
        // name twice resolves here the way it resolves for every other reader: first match wins.
        if matches!(
            declared_seat(&declared_policy(channels, Some(&id)), handle),
            DeclaredSeat::Declared
        ) {
            scope.insert(id);
        }
    }
    Ok(scope.into_iter().collect())
}

/// What one board contributes to the [`search`] filter, resolved once per distinct thread.
struct SearchBoard {
    /// Whether THIS reader is the board's requester — [`requester_for`]'s answer, which sees the
    /// whole board unfiltered.
    reader_is_requester: bool,
    /// The board's opening message id: the REQUEST, which [`filter_board_messages`] keeps for every
    /// reader. `None` for a thread with no message (unreachable for a hit, kept closed anyway).
    opening_message_id: Option<String>,
}

/// Whether one search hit survives its channel's declared `visibility` for `handle` — the
/// single-message form of [`filter_board_messages`]. Both decide through [`visible_under`], which
/// is where the rule itself lives; what differs here is only the ORDER the four inputs are
/// established in, which is chosen to avoid the most work: the two that need no database come
/// first and carry the common search entirely.
fn hit_is_visible(
    store: &ChatStore,
    handle: &str,
    hit: &MessageHit,
    channels: &[ChannelDecl],
    visibility: &mut HashMap<String, Visibility>,
    boards: &mut HashMap<String, SearchBoard>,
) -> Result<bool> {
    let declared = *visibility
        .entry(hit.channel_id.clone())
        .or_insert_with(|| declared_policy(channels, Some(&hit.channel_id)).visibility);
    // Both of these are free — no database, no declaration lookup beyond the in-memory catalogue —
    // and between them they carry every hit in an undeclared or `all_members` channel and every hit
    // the caller wrote itself, which is what keeps the cost off the common search.
    if declared == Visibility::AllMembers || same_identity(Some(&hit.sender), handle) {
        return Ok(true);
    }
    // A hit stamped with NO thread has no board to be judged against, and `thread` — the reader this
    // one must agree with — cannot serve it at all, being keyed by thread. The closed direction, for
    // `requester_of`'s reason: an unresolvable requester must never widen a `requester_only` view.
    let Some(thread_id) = hit.thread_id.as_deref() else {
        return Ok(false);
    };
    if !boards.contains_key(thread_id) {
        let opening = store.thread_opening_message(thread_id)?;
        // The opener is the opening message's SENDER, which is what `thread` hands `requester_for`
        // (`messages.first().sender`) — not the `threads` row's `opener` column, so a message-only
        // thread with no row resolves the same way here as it does there.
        let requester = requester_for(
            store,
            thread_id,
            opening.as_ref().map(|(_, sender)| sender.as_str()),
            handle,
        )?;
        boards.insert(
            thread_id.to_string(),
            SearchBoard {
                reader_is_requester: same_identity(requester.as_deref(), handle),
                opening_message_id: opening.map(|(id, _)| id),
            },
        );
    }
    let board = &boards[thread_id];
    Ok(visible_under(
        declared,
        board.reader_is_requester,
        // Both were answered above — a hit in an `AllMembers` channel and the caller's own message
        // never reach here — but they are passed honestly rather than as `false`, so this call says
        // what it means and the shared rule is the only thing deciding.
        same_identity(Some(&hit.sender), handle),
        board.opening_message_id.as_deref() == Some(hit.message_id.as_str()),
    ))
}

/// The bulk quorum read for an EXPLICIT set of thread ids (T4/§8) — the coordination-UI quorum bar
/// reads all of a project's boards at once, no N+1. A pure passthrough to
/// [`thread_quorums`](ChatStore::thread_quorums): the seam adds no semantics — identical derivations
/// as `nxc` (unlike [`threads`], which additionally membership-scopes the id enumeration). `now`
/// drives only the clock-dependent `stale` flag (§4.3).
pub fn thread_quorums(
    store: &ChatStore,
    thread_ids: &[&str],
    now: &str,
) -> Result<Vec<ThreadQuorum>> {
    store.thread_quorums(thread_ids, now)
}

/// **What the caller's own commissions did while it was away** (nxf 6j6v.2hx9): the boards it
/// opened that finished or stopped inside the window `since` opens — its previous session's end. A
/// pure passthrough to [`opener_wake`](ChatStore::opener_wake), where the whole derivation and the
/// promise it makes are argued. `now` drives only the clock-dependent `stale` flag (§4.3).
///
/// **Nothing RENDERS this any more** (nxf 6j6v.1gm9). It was "the same derivation the `nxc
/// inbox`/`prime` 'Threads you opened' block renders (seam invariant)", and that sentence was the
/// case against the block: if the wake is the same derivation a session is pushed anyway, drawing
/// it at every session start is a duplicate — 46.487 bytes of one in the workspace that was
/// measured. Both the block and `nxc inbox` are gone. What the derivation still feeds is
/// [`PrimeReport::wake`] and, through it, `prime --json`'s `threads_you_opened` for an app.
pub fn opener_wake(
    store: &ChatStore,
    consumer: &str,
    now: &str,
    since: Option<&str>,
) -> Result<OpenerWake> {
    store.opener_wake(consumer, now, since)
}

/// **What is waiting for a caller that was working** (nxf 6j6v.gn8b) — the answers held for
/// `session`, in arrival order, and empty when nothing is.
///
/// A pure passthrough to [`held_wakes`](ChatStore::held_wakes). It exists because the seam would
/// otherwise RECORD a fact it could not give back: [`crate::engine::Engine::reply_thread`] fills
/// this queue whenever the caller it answers is mid-turn, and
/// [`crate::engine::Engine::deliver_held`] empties it — an app could do both and never ask what was
/// in it. That is the shape `crates/chat/tests/read_surface.rs` exists to refuse, and it refused
/// this one.
pub fn held_deliveries(
    store: &ChatStore,
    session: &str,
) -> Result<Vec<crate::collecting::HeldRow>> {
    store.held_wakes(session)
}

// ---- agent transcript (nxf epic 6wt2, ticket 5bym) -------------------------

/// One role session's agent transcript, nested (`nxc transcript show`'s value, and the seam surface
/// app-foundations mirrors). Reads the flat rows the sidecar appended and reassembles the tree:
///
/// - main-conversation entries (`parent_tool_use_id IS NULL`) are `entries`, in `seq` order;
/// - an entry tagged with `parent_tool_use_id = X` nests, in `seq` order, into the `subagent` vec of
///   the main-conversation `tool_use` entry whose `tool_use_id` is `X`. Only a `tool_use` can
///   parent: a Task's own `tool_result` comes back on the main conversation carrying the SAME id,
///   and would otherwise be a second, competing parent for the same sub-timeline;
/// - **an ORPHAN is emitted at top level, in its own `seq` position** — nothing is dropped. A
///   transcript flushed mid-Task (the parent `tool_use` landed in an earlier batch that never made
///   it, or the run died before the Task closed) must still show everything it has, and appending
///   orphans at the end instead would misdate them as later events;
/// - nesting is exactly ONE level: only main-conversation `tool_use` entries are registered as
///   parents, so a subagent's own `tool_use` never becomes one. Per the wire contract the SDK
///   reports the SPAWNING Task's id for a grandchild too, so a nested Task's entries surface under
///   the same top-level parent rather than disappearing.
///
/// An unknown session is NOT an error — it returns an empty view, the same shape a known session
/// whose sidecar has not flushed yet has (they are indistinguishable, and neither is a caller
/// mistake). Three point reads total: one index-backed range scan over `agent_transcript`, plus two
/// `session_map` primary-key lookups for `real_sdk_id`/`role`.
///
/// **UNGATED, unlike every sibling read — deliberate, and an embedder must not assume otherwise**
/// (PR #258 review, Integrity & Robustness #2). `messages`/`thread`/`thread_board` all call
/// [`require_readable`] first; this does not, and the reason is that the gate is not merely
/// unapplied here, it is UNDEFINED: that gate asks about a CHANNEL, and a transcript has none. It is
/// keyed by an internal session id, which `session_map` resolves to a ROLE, not to a channel.
/// The alternative gate — "only the session's own role may read it" — would break the one use case
/// this read exists for, an operator reading a role's transcript. And on the CLI it would buy
/// nothing regardless: `agent_transcript` is device-local and never synced, so anyone who can run
/// `nxc transcript show` can already read the table straight out of `.nxs/db.sqlite`.
///
/// What that leaves is a real hazard worth naming rather than a vulnerability to patch: an embedder
/// that has learned to trust `messages()` to enforce membership may assume the same of this call.
/// It does not. A transcript carries raw tool inputs and results — whatever the agent read, wrote,
/// or ran — so **an app serving more than one user must do its own authorization before calling
/// this.** If a domain-meaningful gate is ever wanted, it needs a session→channel link that does
/// not exist today; that is tracked, not forgotten.
///
/// **Memory (the read half of the no-cap decision argued in `ChatStore::append_transcript`):** this
/// materializes a whole session at once, `data` payloads included — and `tool_use`'s `data.input` is
/// the one unbounded field on the wire (a `Write` of a large file). That is ratified here rather
/// than papered over with a read-side cap: a truncating read would be a SILENT lie about stored
/// evidence. Both honest levers now exist (nxf 6j6v.t7pa) and neither serves less than what is
/// there: [`transcript_page`] windows the read at the CALLER's explicit request, so a short answer is
/// one they asked for rather than one they were handed, and [`ChatStore::prune_transcripts`] bounds
/// what is stored in the first place. The cost of THIS call is still bounded by one session, on one
/// device, per call.
pub fn transcript(store: &ChatStore, internal_session: &str) -> Result<TranscriptView> {
    transcript_page(store, internal_session, -1, None)
}

/// Retire the transcripts of sessions whose last recorded activity is older than `keep_days` before
/// `now`, and report what went (nxf 6j6v.t7pa) — the seam half of `nxc transcript prune`.
///
/// A pure passthrough to [`ChatStore::prune_transcripts`], which is where the policy is argued:
/// whole sessions only, aged on their LAST activity, strict cutoff. `dry_run` reports without
/// writing (and without taking a write lock).
///
/// **The automatic half needs no call from an app.** Retention also rides the first flush of every
/// new session, so an embedder that never calls this still gets a bounded table; this is the lever
/// for clearing out a workspace that has gone quiet — where no new session is coming to trigger that
/// pass — or for applying a tighter window than the configured one. It frees pages for reuse rather
/// than shrinking the file; see [`ChatStore::prune_transcripts`] for why no `VACUUM`.
pub fn prune_transcripts(
    store: &mut ChatStore,
    now: &str,
    keep_days: i64,
    dry_run: bool,
) -> Result<TranscriptPruneReport> {
    store.prune_transcripts(now, keep_days, dry_run)
}

/// [`transcript`] over a WINDOW of the session (nxf 6j6v.t7pa): entries with `seq` strictly greater
/// than `after_seq` (pass `-1` for "from the start"), at most `limit` of them when one is given.
/// This is the paging lever the no-cap decision names — a consumer walking a very long session takes
/// it in chunks instead of materializing every `tool_use` payload at once.
///
/// **`limit` counts stored ENTRIES, nested ones included**, because that — not the shape of the tree
/// — is what bounds the memory this call holds. The alternative (limit the top-level lane and let
/// each one drag its whole sub-timeline along) reads tidier and bounds nothing: a single `Task`
/// `tool_use` can carry hundreds of subagent entries, so one "page" could be the entire session.
///
/// The cursor is the largest `seq` in the returned view, which is the same
/// caller-tracks-its-own-cursor shape [`ChatStore::transcript_rows_after`] already serves `--stream`
/// with. A window that comes back shorter than `limit` (or empty) is the end of what is stored.
///
/// **Nesting is reconstructed from the rows IN the window**, so a window can cut a sub-timeline away
/// from the `tool_use` that spawned it — either because the parent sits below `after_seq`, or
/// because `limit` fell between them. Those entries surface at TOP LEVEL keeping `subagent_type` and
/// their own `seq` position: nothing is dropped, and the shape is the one
/// [`TranscriptEntryView`] already defines for an ORPHAN, not a new one invented by paging.
///
/// It is worth being blunt about the limit of that, because the view is deliberately built so the
/// tree is the only answer to "whose subagent is this": [`TranscriptEntryView`] carries no
/// `parent_tool_use_id`, so a consumer CANNOT re-attach a cut-off child to its parent from two
/// windows alone — it can only tell that the entry belongs to some earlier `tool_use`. Renderers
/// that walk forward are fine (track the last top-level `tool_use` you emitted); anything that needs
/// the exact tree should read the session unwindowed, which is what [`transcript`] still does. That
/// is the honest price of a bound that is real, and it is why the bound is on ROWS: the alternative
/// — never split a sub-timeline — buys back the tree at the cost of bounding nothing, since one
/// `Task` can carry most of a session.
pub fn transcript_page(
    store: &ChatStore,
    internal_session: &str,
    after_seq: i64,
    limit: Option<i64>,
) -> Result<TranscriptView> {
    let rows = store.transcript_rows_page(internal_session, after_seq, limit)?;
    // Which ids can parent a sub-timeline, resolved BEFORE the walk: a child may in principle be
    // read before its parent (the store preserves arrival order, and only the sidecar's own
    // ordering makes the parent come first), so "is this an orphan" cannot be decided by what has
    // been seen so far.
    let parents: HashSet<String> = rows
        .iter()
        .filter(|r| r.entry.parent_tool_use_id.is_none() && r.entry.kind == "tool_use")
        .filter_map(|r| r.entry.tool_use_id.clone())
        .collect();

    let mut entries: Vec<TranscriptEntryView> = Vec::new();
    let mut sub_timelines: HashMap<String, Vec<TranscriptEntryView>> = HashMap::new();
    for row in rows {
        let parent = row.entry.parent_tool_use_id;
        let view = TranscriptEntryView {
            seq: row.seq,
            kind: row.entry.kind,
            at: row.entry.at,
            tool_use_id: row.entry.tool_use_id,
            subagent_type: row.entry.subagent_type,
            data: row.entry.data,
            subagent: Vec::new(),
        };
        match parent {
            Some(p) if parents.contains(&p) => sub_timelines.entry(p).or_default().push(view),
            // Main conversation, or an orphan — the SAME top-level lane, so both keep their seq
            // position relative to everything else.
            _ => entries.push(view),
        }
    }
    for e in &mut entries {
        if e.kind != "tool_use" {
            continue; // only a tool_use was ever registered as a parent (see the doc comment)
        }
        if let Some(children) = e
            .tool_use_id
            .as_ref()
            .and_then(|id| sub_timelines.remove(id))
        {
            e.subagent = children;
        }
    }
    // "Nothing is dropped" made STRUCTURAL rather than argued. Every key of `sub_timelines` came
    // from `parents`, and every member of `parents` is the `tool_use_id` of a top-level `tool_use`
    // entry the loop above just drained — so this is unreachable today. It must not rest on the
    // `debug_assert!` alone, though: that compiles out of release, so a future drift between the
    // parent filter and the attach filter would delete entries from the served view in production
    // only. Anything still held here goes back into the top-level lane an orphan takes, re-sorted
    // into `seq` order (seqs are unique, so the order is total and deterministic). The assert stays
    // as the dev-time tripwire that the invariant drifted at all.
    if !sub_timelines.is_empty() {
        debug_assert!(
            false,
            "transcript {internal_session}: sub-timelines with no parent entry: {:?}",
            sub_timelines.keys().collect::<Vec<_>>()
        );
        entries.extend(sub_timelines.into_values().flatten());
        entries.sort_by_key(|e| e.seq);
    }
    Ok(TranscriptView {
        session: internal_session.to_string(),
        real_sdk_id: store.resolve_real(internal_session)?,
        role: store.session_role(internal_session)?,
        entries,
    })
}

/// Append a batch of normalized wire entries to one session's agent transcript, returning how many
/// landed (nxf 6j6v.c6e8) — the WRITE half of what [`transcript`] reads, and the half this seam did
/// not have.
///
/// **Who this is for.** The transcript is written by the local sidecar through
/// `nxc transcript append`, and that is the only path there was: a host that drives the Claude Agent
/// SDK itself — the case [`Engine::bind_runtime_session`](crate::engine::Engine::bind_runtime_session)
/// already exists for, "a local session can reach a CLI, a remote one cannot" — could fill a
/// transcript only by spawning a process or by reaching past this facade into the store. It showed
/// up as a bound session whose transcript stayed empty (`real=…  (0 entries)`); the binding had
/// worked, nobody could write.
///
/// **`seq` stays in the store**, and that is the load-bearing half of this signature: it is
/// deliberately not on the wire ([`TranscriptEntry`](crate::transcript::TranscriptEntry) has no such
/// field), so two callers flushing the same session — a `resume` that starts a second sidecar
/// process, a host that batches its own turns — continue one history instead of colliding over it.
/// A caller reports what came back; it never numbers its own entries.
///
/// **One batch is one transaction**, for the reason [`ChatStore::append_transcript`] argues at
/// length: a stored PREFIX of a flush that failed is indistinguishable on read from a flush that
/// succeeded, while the rest is gone with no marker. So a batch with one bad entry stores nothing,
/// and the error names the entry by its index in the batch — the coordinate THIS caller framed its
/// input in. `nxc transcript append` reports a stdin LINE number instead, off the same shared
/// [`TranscriptEntry::validate`](crate::transcript::TranscriptEntry), which is why the CLI keeps its
/// per-line check rather than this call growing a second one.
///
/// Retention rides this path: the first flush of a new session also retires the sessions older than
/// the configured window, so an app that never calls [`prune_transcripts`] still gets a bounded
/// table. Nothing in that pass can fail the flush — `ChatStore::append_transcript` says why.
///
/// **No membership gate**, symmetrically with [`transcript`]: a transcript is keyed by an internal
/// session id, which resolves to a ROLE and not to a channel, so the sibling gate is undefined
/// rather than skipped. An unknown session id is accepted and stored — the sidecar's own contract,
/// since a transcript may be flushed for a session `session_map` never minted — which means this
/// call cannot tell a host's typo from a legitimately unmapped session. An app serving more than
/// one user authorizes the caller itself, on the write as on the read.
///
/// **AND NO SIZE BOUND — the caller's, not this call's** (PR #407 review, Integrity #1). Every other
/// axis on this page is argued at length, and silence about this one would read as "guarded like the
/// CLI is". It is not. `nxc transcript append` caps STDIN at `MAX_TRANSCRIPT_BATCH_BYTES` (64 MiB,
/// `cli.rs`) — and that cap is about the READ: the command turns an unknown-length pipe into memory
/// and must not be OOM-killed by one pathological line. **That hazard does not reach here**, because
/// a caller of this function has already materialized the entries; the memory was spent before the
/// call. What DOES reach here is the rest of it: the batch goes in as ONE `BEGIN IMMEDIATE`
/// transaction, so it holds the workspace's single write lock — the same `db.sqlite` the op log
/// lives in — for as long as it takes, and `data` is stored whole by the deliberate no-cap rule
/// [`ChatStore::append_transcript`] argues. **A host ingesting from an untrusted or unbounded byte
/// stream must impose its own bound before calling this.**
///
/// It is deliberately not fixed by capping HERE, for two reasons that point the same way. The CLI
/// now routes through this call, so a cap here is a second cap on that path in different units
/// (serialized `data` bytes, not raw stdin bytes) — able to refuse a batch the stdin cap accepted,
/// which is the shipped sidecar contract breaking under a change that promised not to touch it. And
/// `cli.rs` says of its own number, in as many words, that it is "the backstop, NOT a policy: a cap
/// that ever bites belongs in `transcript.mjs`". Promoting a backstop to a seam policy would make
/// this call refuse the evidence the table exists to keep. Whether the seam should carry a bound of
/// its own — an opt-in one, or an append that streams — is tracked as nxf `6j6v.rydw`.
pub fn transcript_append(
    store: &mut ChatStore,
    internal_session: &str,
    entries: &[crate::transcript::TranscriptEntry],
) -> Result<u64> {
    store.append_transcript(internal_session, entries)
}

// ---- declared-team roster (prime-time referential-integrity partition) ----

/// The prime-time referential-integrity partition of a workspace's declared roles and channels
/// (nxf ticket 6j6v.v9k3): the CLEAN roster `nxc prime` offers, plus every referential error found
/// and — since nxf 6j6v.9w08 — every quality warning over the same declarations. `cli.rs`'s `prime`
/// decides (at the RENDERING layer only) whether to surface `errors`/`warnings` based on
/// interactive-vs-spawned context; this type carries no notion of that itself.
///
/// Partition granularity is deliberately WHOLE-FILE for channels (this ticket's own resolved design
/// decision, not deferred): `channels.yaml` can declare multiple channels in one file, and
/// `validate_channels`'s `ValidationError.what` embeds a channel's name as free text with no
/// structured "which channel" field to key a true per-channel decision on without fragile
/// text-parsing — so ANY error in `channels.yaml` excludes ALL of its channels, not just the
/// specific broken one. Roles are UNAFFECTED by this ticket: a malformed role file already fails at
/// LOAD time via `load_all_roles`'s own `?`-propagation (a hard error, not a partition) — this
/// ticket adds no role validator, per its own "Interfaces — Consumes" naming only
/// `validate_channels`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimeRoster {
    /// Clean role handles, sorted (mirrors `load_all_roles`'s own `handle`-sort).
    pub roles: Vec<String>,
    /// Clean channel names, in `channels.yaml`'s declaration order — empty when `channels.yaml`
    /// doesn't exist OR (whole-file scoping) it exists but `validate_channels` found any error.
    pub channels: Vec<String>,
    /// Every referential error found across the declared channels — possibly empty.
    pub errors: Vec<ValidationError>,
    /// Every QUALITY warning found across the declared personas and channels (nxf 6j6v.9w08) —
    /// possibly empty, and a different question from [`errors`](PrimeRoster::errors): a warning
    /// says a declaration is written in a way that will not work well, excludes nothing from the
    /// roster above, and leaves the declaration fully usable. See
    /// [`crate::declaration_quality`] for what is checked and what deliberately is not.
    pub warnings: Vec<DeclarationWarning>,
}

/// Build [`PrimeRoster`] for a workspace's declaration directory (nxf ticket 6j6v.v9k3): loads
/// roles (`role::load_all_roles`), loads + validates channels (`channel::load_all_channels` +
/// `channel::validate_channels`), then partitions per the whole-file rule documented on
/// [`PrimeRoster`] and returns the clean roster + the (possibly empty) error list. Pure aside from
/// the loaders' own file reads — no env, no interactive/spawned awareness at all, keeping this a
/// testable unit independent of `cli.rs`'s rendering decision.
///
/// It carried a third partition, the DECLARED WORKFLOWS, until 6j6v.dvyq §3 removed the run engine.
/// A `workflow.yaml` left in the folder is now simply not read by anything.
pub fn validate_declared_team(dir: &Path) -> Result<PrimeRoster> {
    let roles = crate::role::load_all_roles(dir)?;
    let channels = crate::channel::load_all_channels(dir)?;
    let channel_errors = crate::channel::validate_channels(&roles, &channels);
    // Whole-file scoping: any CHANNEL error excludes every channel this file declared — and only a
    // channel error does. The persona pass below is appended after this is decided, deliberately:
    // a persona's dangling reference says nothing about whether `channels.yaml` can be trusted, and
    // letting it blank the channel roster would be the partition answering a question it was not
    // asked (nxf 6j6v.st83).
    let clean_channels: Vec<String> = if channel_errors.is_empty() {
        channels.iter().map(|c| c.name.clone()).collect()
    } else {
        Vec::new()
    };
    // The PERSONA pass (nxf 6j6v.st83): the mirror of the channel one, reported beside it. Roles
    // stay in the roster whatever it finds — see [`PrimeRoster`], which has always said a role file
    // fails at LOAD time or not at all.
    let mut errors = channel_errors;
    errors.extend(crate::role::validate_roles(&roles, &channels));

    // Over the SAME two slices, and deliberately not bounded by the partition above: a quality
    // warning is about how a declaration is written, so a referential error elsewhere in
    // `channels.yaml` neither suppresses it nor is suppressed by it.
    let warnings = crate::declaration_quality::warn_declarations(&roles, &channels);

    Ok(PrimeRoster {
        roles: roles.into_iter().map(|r| r.handle).collect(),
        channels: clean_channels,
        errors,
        warnings,
    })
}

// ---- prime: the session bootstrap (nxf 6j6v.r5a2) --------------------------

/// The context-recovery hint the bootstrap opens with — the sentence, without the blockquote
/// framing the renderer adds (mirrors flow's and memory's `PRIME_CONTEXT_RECOVERY`).
///
/// It said "to reload this inbox" until nxf 6j6v.4mmk took the message bodies out of the rendered
/// block: there is no inbox here to reload any more, and a hint that names one is a false map of
/// the very text it heads.
///
/// **Superseded 2026-08-28 (nxf h4d3, task 3).** The human view no longer renders this hint at
/// all — see [`PrimeReport::render_markdown`]'s doc comment for the full account of what the
/// task-3 cut kept and dropped. This constant and the field it feeds are UNCHANGED: `to_value()`'s
/// `"context_recovery"` key still serializes the string below verbatim.
pub const PRIME_CONTEXT_RECOVERY: &str =
    "re-run `nxs prime` after a context compaction to reload this block.";

/// The coordination rule: agent-to-agent coordination goes through `nxc`, never through ad-hoc
/// notes or scratch files. One paragraph of Markdown, rendered verbatim — as with memory's rule, the
/// exact wording is the point.
///
/// **Superseded 2026-08-28 (nxf h4d3, task 3).** "Rendered verbatim" stopped being true of the
/// human view: it now folds the same prohibition into [`PRIME_INTRO`] below, in different, shorter
/// wording (the owner's own edited target draft, task-3 brief — not a rewording of THIS constant in
/// place). This constant and the `coordination_rule` field are UNCHANGED: `to_value()` still
/// serializes the string below verbatim, unaffected by the human view's cut.
pub const PRIME_COORDINATION_RULE: &str =
    "**Coordination rule:** agent-to-agent coordination in this workspace goes through `nxc` — \
     `send --to` to start something, `reply --thread` to answer it. **Do not** coordinate through \
     ad-hoc notes or scratch files: they are not delivered, not synced, and never replayed here. \
     `nxc` is the one durable channel between agents.";

/// The intro paragraph `nxc prime`'s human view opens with, right after the title (nxf h4d3, task
/// 3): what `nxc` is for, with the coordination rule folded in as a prohibition rather than a
/// labelled rule of its own — the one thing no `--help`/`nxc guide` page teaches, which is exactly
/// why the task-3 brief kept it and cut the rest. Rendered verbatim.
///
/// A deliberately DIFFERENT string from [`PRIME_COORDINATION_RULE`], not that constant reworded in
/// place: the owner's own edited target draft (`nxc-static-target.md`) is the specification for
/// this text, and the machine contract (`coordination_rule` in `--json`) is untouched — see that
/// constant's own doc comment.
pub const PRIME_INTRO: &str =
    "`nxc` is how the agents in this workspace coordinate: `send --to` starts something, `reply \
     --thread` answers it, and every message is durable and replayable. **Never coordinate through \
     scratch files or ad-hoc notes** — they are not delivered, not synced, and never replayed here. \
     `nxc --help` lists the commands, `nxc guide` the topics behind them.";

/// **What to do when a commission does not move** (nxf 6j6v.4mmk) — one short block, rendered
/// verbatim, and the one section here that was measured as MISSING rather than wrong.
///
/// It earns its place by the same two tests the command reference does: a session cannot get it
/// anywhere else (the working-tree lease has no verb of its own and no error message that explains
/// it), and it needs it BEFORE it acts — by the time a chain has hung, the session that could have
/// read this is the one waiting. In the 4jgn.g90w testbed a chain sat for over an hour and the
/// diagnosis went through `ps`, ULID decoding and reading the source; every fact below was in the
/// engine the whole time.
///
/// Deliberately four sentences and no more. This rides on every session that reads the block.
///
/// **Superseded 2026-08-28 (nxf h4d3, task 3).** "Rides on every session" stopped being true: the
/// task-3 brief pulled this paragraph from the human view because it already belongs to `nxc guide
/// limits-and-safety` (`docs/guide/en/limits-and-safety.md` — the working-copy lease's fuller
/// account) rather than a cost every session start pays unconditionally. This constant and the
/// `when_stuck` field are UNCHANGED: `to_value()` still serializes the string below verbatim.
///
/// **It stopped naming `withdraw` in nxf 6j6v.ezbr** (owner, 2026-09-20): the verb is a person's,
/// a session this workspace started is refused it, and a text read by agents must not teach it.
/// What an agent can do about a held copy is what this now says — answer the holding thread if the
/// answer is its to give, and otherwise escalate, which goes UP to whoever may decide, including
/// whether to take a round back.
pub const PRIME_WHEN_STUCK: &str =
    "**When something does not move:** a commission that has not started yet is parked behind the \
     WORKING COPY, and `nxc threads show <thread-id>` is the only place that state is visible \
     (`working tree: holding` / `waiting (#N)` / `not needed`). What holds it is the chain that \
     took it — most often an escalation nobody has answered, because an escalated round keeps the \
     working copy until a reply lands in that thread. So: answer the holding thread \
     (`nxc reply --thread <id> -`) if the answer is yours to give; if it is not, escalate on your \
     own thread (`nxc reply --thread <id> --escalate -`), which goes UP, towards whoever may \
     decide what becomes of the round holding the copy. An abandoned lease is reclaimable by the \
     next chain that asks after 2h.";

/// One entry of the bootstrap's command reference: the invocation(s) as an agent types them, and
/// what they do. Rendered `- `{invocation}` — {summary}`; the halves are separate so a consumer that
/// is not rendering Markdown (a palette, a tool list) can use them apart.
///
/// `invocations` is a slice because one entry MAY name a pair of closely related verbs that share a
/// summary — the renderer joins them with ` / `, each in its own code span. **No entry does
/// today**: the pair this was built for (`nxc channels` / `nxc agents`) is gone, `agents` with its
/// verbs and `channels` off the agent surface (nxf 6j6v.dvyq §3). The slice stays because the
/// alternative is worse, not because the case is live: carrying a pair as one pre-formatted string
/// would smuggle Markdown into a data field, which is the exact confusion this layer exists to end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrimeCommand {
    pub invocations: &'static [&'static str],
    pub summary: &'static str,
}

impl PrimeCommand {
    /// The Markdown bullet: every invocation in its own code span, joined by ` / `, then the summary.
    ///
    /// **Superseded 2026-08-28 (nxf h4d3, task 3).** `PrimeReport::render_markdown` no longer calls
    /// this — the human view's "## How a conversation moves" section is
    /// [`PRIME_HOW_A_CONVERSATION_MOVES`], a fixed block, not a loop over [`PRIME_COMMANDS`]. This
    /// method stays public for a caller that wants ONE command bullet rendered this way from the
    /// full list `--json`'s `commands` field still carries — the same reasoning that kept
    /// [`render_declared_team`] public after its own caller left `render_markdown`, and (review of
    /// task 3, Important 2) now covered the same way it is: direct unit tests in this file's own
    /// `#[cfg(test)] mod tests`, since the golden that used to exercise it indirectly through
    /// "## Chat Commands" is gone.
    pub fn render_markdown(&self) -> String {
        let invocations: Vec<String> = self.invocations.iter().map(|i| format!("`{i}`")).collect();
        format!("- {} — {}", invocations.join(" / "), self.summary)
    }
}

/// The "## How a conversation moves" cheatsheet `nxc prime`'s human view renders instead of looping
/// over [`PRIME_COMMANDS`] (nxf h4d3, task 3): the verbs the task-3 brief kept — `send --to` and
/// `reply --thread`, and until nxf 6j6v.ezbr `withdraw --thread` (see below) — as literal shell
/// examples with a trailing `#` comment each, mirroring `nxf prime`'s own "## How work moves" cheatsheet
/// (`crates/cli/src/commands/mod.rs`). Owner-edited target draft (`nxc-static-target.md`) is the
/// specification for the exact wording and column alignment; rendered verbatim, a raw string so
/// neither the embedded `"..."` bodies nor the newlines need escaping.
///
/// **The body is `-` since nxf 6j6v.s46h, and the block pays two lines to say what that means.**
/// This is the third of the three places that teach `nxc reply` (the other two are the forced
/// ending and a persona's own instructions, both in `role.rs`/`persona.rs`), and the defect
/// 6j6v.65zg records is precisely those three saying different things. Showing `"<body>"` here
/// while the engine's own obligation shows the STDIN form would be that defect, re-made on the day
/// the form changed. The two explaining lines are the price of a sentinel that means nothing on
/// its own — and against the block's measured budget, where 85% of a session start is memories in
/// full text (nxf 6j6v.waq9), two lines is the cheapest part of it.
///
/// **`--escalate` gets its own line, and the block says which way it travels** (nxf 6j6v.aqqa).
/// It rode here as a seven-word aside on the `reply` line — `` `--escalate` says "I cannot" `` —
/// and that is the HALF of its meaning that stops a session reaching for it. Measured in
/// `watch-bundestag` on 2026-09-12: a PM had decided everything technical and had two questions
/// left that belonged to the owner; it could carry its task out, so "I cannot" did not describe
/// it, and it ended its turn with a plain `reply`. The chain did what a chain does and handed the
/// answer DOWNWARD — three review rounds, 103 findings, 63 minutes judging a draft whose central
/// product decision was still open, and the questions reached the owner only when `max_passes`
/// ran out. The engine's own forced ending ([`crate::role`]'s `reply_obligation`) has always said
/// it correctly — "you cannot reach the result and need help or a decision" — but it is written at
/// the END of a turn, by which time the session has long since planned how to finish. This block
/// is what it read FIRST, so this block is where the full meaning has to stand.
///
/// Deliberately NOT derived from [`PRIME_COMMANDS`]: the wording here is shorter than the matching
/// entries' `summary` fields (no "say what it is about, or `--no-ref`", no "in that thread") and
/// the placeholder spelling differs (`<id>`, not
/// `<thread-id>`) — a budget-cut rewrite, not a subset render. `PRIME_COMMANDS` and the `commands`
/// field it feeds still carry their entries verbatim in `--json` — seven since nxf 6j6v.ezbr took
/// `withdraw` out of both.
///
/// The four lines were THREE VERBS — `send`, `reply`, `withdraw` — which is what task 3 of nxf
/// h4d3 cut the block to. **Since nxf 6j6v.ezbr they are three lines and TWO verbs**: `withdraw`
/// left the block on the owner's decision of 2026-09-20 — it is a person's verb, a session this
/// workspace started is refused it, and this block is read by every session that starts. What an
/// agent that wants a round stopped has instead is the `--escalate` line, which is also why that
/// line's paragraph below says which way it travels. The second `reply` line is the same verb
/// shown with the flag that changes how a turn ends, the way the forced ending shows it, not a
/// third entry creeping back in.
pub const PRIME_HOW_A_CONVERSATION_MOVES: &str = r#"    nxc send --to <handle> --ref nxf_ids=<id> -         # start one; a thread id comes back
    nxc reply --thread <id> -                           # answer — you are finished
    nxc reply --thread <id> --escalate -                # you need help or a DECISION first

`-` reads the message from STDIN — `… - <<'EOF'`, your text, then `EOF` on its own line — so the
shell evaluates nothing in it. A one-line message may be an argument instead.

`--escalate` is not only "I cannot": it is also how you ask for a DECISION that is not yours to
make — one you could act on either way, but may not settle. It goes UP, to whoever commissioned
you. Put that question in an ordinary reply instead and it travels DOWN to the next step, which
may not settle it either, and the work runs on around it."#;

/// The chat verbs the bootstrap names, in the order it lists them.
///
/// **This list IS the agent surface** (nxf 6j6v.dvyq §4): not "the verbs the binary has", but the
/// set a session is TAUGHT at its start, and nothing here is meant to be exhaustive.
///
/// That distinction used to be carried by an example — "a human at the keyboard still types `nxc
/// inbox`" — and **the example expired** (nxf 6j6v.1gm9): the verb is not in the binary at all any
/// more, for a human or anyone else, which is what the measurement below had been saying for two
/// tickets. The distinction itself stands: `nxc tick` is in the binary and is not taught, because
/// no human types it and the scheduled job invokes it.
///
/// **The order and the membership are MEASURED, not guessed** (nxf 6j6v.4mmk). Over 66 persona
/// sessions across seven roles in the 4jgn.g90w testbed, every transcript was searched for
/// `nxc <verb>`:
///
/// ```text
/// reply 61/66 · threads 28/66 · send 15/66 · status 13/66 · list 8/66
/// withdraw 3/66 · transcript 1/66 · search 0/66 · inbox 0/66 · read 0/66
/// ```
///
/// Two corrections came out of that. `threads show` is the SECOND most used verb in the whole
/// corpus and was not named here at all — and it is the only place the working copy's state is
/// visible (`working tree: holding` / `waiting (#N)` / `not needed`), which is what a hung chain is
/// diagnosed from. `withdraw` is the rescue verb for a commission parked behind that working copy,
/// and a primed persona could not know it existed. Both cover the BAD case; the six that were here
/// already cover the good one, and a session that only knows the good case is helpless exactly when
/// it matters.
///
/// **`withdraw` left this list again in nxf 6j6v.ezbr**, and the reason is the owner's rather than
/// the measurement's (2026-09-20): once the verb could stop another agent's running session, WHO
/// may call it became the question, and the answer is a person — or an assistant a person started —
/// who sent the operation's first commission. A session this workspace started is refused it by
/// the engine, so teaching it here would teach a refusal. The rescue an agent has for a held copy is
/// escalation, and [`PRIME_WHEN_STUCK`] says so.
///
/// **`read` and `inbox` are deliberately NOT here** (owner, 2026-08-23). Zero uses in 66 sessions is
/// not carelessness, it is the model: a spawned persona gets its task in the TRIGGER MESSAGE and a
/// resumed one gets the answer as a new turn, so there is no moment at which an agent would ask its
/// inbox. `read` was proposed for this list back when the block dumped the inbox and you had to be
/// able to acknowledge it; that dump is gone (see [`PrimeReport::render_markdown`]), and the reason
/// went with it.
///
/// **`send --to` is spelled WITH its `--ref` here, and that is load-bearing** (nxf 6j6v.ckeq §5).
/// The refs obligation cannot be enforced — a caller can always answer `--no-ref` where a ticket
/// belonged — so what makes it bite is that the persona meets it in the text it reads anyway. The
/// item says exactly that: "`prime_as` sagt es der Persona (es ist der Text, den sie ohnehin
/// liest)". A block that taught the bare form would be teaching the shape that warns.
///
/// **Superseded 2026-08-28 (nxf h4d3, task 3).** "This list IS the agent surface" stopped being
/// true of the RENDERED human view: task 3 cut it to three entries — `send --to`, `reply --thread`,
/// `withdraw --thread` — rendered as [`PRIME_HOW_A_CONVERSATION_MOVES`], not as a loop over this
/// list (see that constant's doc comment for why). It is still true of the DATA: every entry of
/// this array rides `to_value()`'s `commands` field in `--json` (seven since `withdraw` left it,
/// see above), and an
/// embedding app that wants the fuller reference reads it from there — `nxc --help`/`nxc guide` are
/// the human-typed equivalent the task-3 brief points to instead.
pub const PRIME_COMMANDS: &[PrimeCommand] = &[
    PrimeCommand {
        invocations: &["nxc list"],
        summary: "who can be addressed here, and what for",
    },
    PrimeCommand {
        invocations: &["nxc send --to <persona|channel> --ref nxf_ids=<id> -"],
        summary: "start a conversation; you get a thread id back — say what it is about, or \
                  `--no-ref`. `-` reads the body from STDIN",
    },
    PrimeCommand {
        invocations: &["nxc reply --thread <id> -"],
        summary: "answer in that thread — `--escalate` says you cannot reach the result and need \
                  help or a decision, and hands the round UP to whoever commissioned you. `-` \
                  reads the body from STDIN, so the shell evaluates nothing in it",
    },
    PrimeCommand {
        invocations: &["nxc status [--thread <id>] [--all]"],
        summary: "what is still going on here; `--thread` for one operation, `--all` for the \
                  finished ones too",
    },
    PrimeCommand {
        invocations: &["nxc threads show <thread-id>"],
        summary:
            "one board in full: who still owes a reply, and whether it holds the WORKING COPY \
                  (`holding` / `waiting (#N)` / `not needed`)",
    },
    PrimeCommand {
        invocations: &["nxc search \"<text>\""],
        summary: "find a message again, across your channels",
    },
    PrimeCommand {
        invocations: &["nxc transcript show <session>"],
        summary: "what a session actually did, step by step",
    },
];

// `InboxEntry` stood here — the five-field projection of a store row that `prime --json`'s
// `in_turn`/`next_session` carried, with `disposition_wire` beside it for its one lowercase field.
// Both went with the unread apparatus (nxf 6j6v.4d2z): there is no catch-up on this record any
// more, so there is nothing to project. `Disposition` itself is untouched — it is a field on every
// message, on the wire and in the model.

/// The full `nxc prime` session-bootstrap record (nxf 6j6v.r5a2, epic 6j6v.fjrc) — the mirror of
/// flow's and memory's `PrimeReport`. It carries the ENTIRE block as data: the context-recovery
/// hint, the coordination rule, the command reference, the requester wake, and the declared-team
/// roster with every referential error found. It carried the unread catch-up split by disposition
/// too, until nxf 6j6v.4d2z removed the unread set itself.
///
/// **Superseded 2026-08-28 (nxf h4d3, task 3):** every remaining field is still there, unchanged,
/// and `to_value()` still serializes all of them. It is no longer true of the human (Markdown)
/// rendering: `nxc prime`'s SessionStart hook has a hard host-side byte ceiling (q3fh
/// session-start-budget), so [`render_markdown`](PrimeReport::render_markdown) now renders a
/// CURATED SUBSET of this struct rather than a 1:1 walk of it — see that method's own doc comment
/// for what stayed and what a session now pulls on demand instead (`nxc --help`/`nxc guide`).
///
/// **The report carries the data; the CALLER decides what to show.** `nxc prime` surfaces
/// declaration errors — and, since nxf 6j6v.9w08, the quality WARNINGS beside them and the one line
/// pointing at `nxc guide writing-declarations` — only in an interactive (human-at-the-keyboard)
/// context. That is a RENDERING decision and stays one: both
/// [`render_markdown`](PrimeReport::render_markdown) and [`to_value`](PrimeReport::to_value) take
/// it as a parameter. An embedding host has no terminal notion of "interactive", and a facade that
/// made the call itself would silently serve an app less than it serves the CLI.
///
/// Deliberately NOT `Serialize`: two ways to serialize one record is exactly how the two views drift
/// apart (E5c found precisely that). [`to_value`](PrimeReport::to_value) is the single projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimeReport {
    /// The agent this record is for — the qualified handle the caller resolved.
    pub consumer: String,
    /// The context-recovery hint ([`PRIME_CONTEXT_RECOVERY`]), without its blockquote framing.
    pub context_recovery: &'static str,
    /// The coordination rule ([`PRIME_COORDINATION_RULE`]), one paragraph of Markdown.
    pub coordination_rule: &'static str,
    /// What to do when a commission does not move ([`PRIME_WHEN_STUCK`]), one paragraph of Markdown.
    pub when_stuck: &'static str,
    /// The command reference ([`PRIME_COMMANDS`]), in display order.
    pub commands: &'static [PrimeCommand],
    // `in_turn` and `next_session` stood here — the catch-up, split by disposition. nxf 6j6v.4mmk
    // stopped RENDERING them (98 % of a measured 199.491-byte `nxc prime` was message text nobody
    // had asked for, growing with the operating time of the workspace rather than with what the
    // session is about) and left the fields, because two live app surfaces read them and their fate
    // belonged to another ticket. That ticket is nxf 6j6v.4d2z and it has now been decided: they go
    // with the read cursor they were derived from. Nothing replaces them — all three delivery paths
    // PUSH (`send` into a fresh prompt, `reply` into the target's resumption, a completed quorum
    // into its opener's wake), which is the argument 4mmk made for dropping the display and is the
    // same argument one layer down.
    /// The boards this agent opened that completed (results in) or went stale (spec §4.4/§5.2).
    pub wake: OpenerWake,
    /// The declared roles/channels/workflows, clean ones offered and broken ones reported.
    pub roster: PrimeRoster,
    /// Who this session IS, when it is a declared persona (nxf 6j6v.p6m1) — its own identity and
    /// how it works with `nxc`. `None` for a human at the keyboard, who needs neither.
    pub persona: Option<PersonaBrief>,
    /// Whom this caller may address and what for — the persona's own address book, or the whole
    /// declared team for a human. The same record `nxc list` prints, so the two cannot disagree.
    pub directory: Directory,
    /// WHERE this workspace's declarations came from, and where they belong (nxf 6j6v.dvyq).
    ///
    /// Always present, not just when the roster is empty: the empty case is the one that needed it
    /// — an empty list is indistinguishable from "there is nothing here" — but an app rendering a
    /// non-empty team still wants to know it is reading a LEGACY `roles/` folder, which is the same
    /// question and therefore the same field. See [`DeclarationSource`].
    pub declarations: DeclarationSource,
}

impl PrimeReport {
    // `count()` stood here — the size of the catch-up set, the `## Unread` heading's number until
    // nxf 6j6v.4mmk removed the section and a `--json` field until nxf 6j6v.4d2z removed the set.

    /// The `prime` `--json` object: a 1:1 structured view of the human layout.
    /// `interactive` is the caller's rendering decision (see the type doc) — pass `false` from any
    /// context where nobody is there to act on the declaration errors and warnings.
    ///
    /// `threads_you_opened`/`roles`/`channels`/`workflows`/`declaration_errors`/
    /// `declaration_warnings` are additive and **omitted entirely when empty** (never an empty
    /// array), which is what keeps the shape byte-identical for a caller with no boards and no
    /// declarations.
    pub fn to_value(&self, interactive: bool) -> Value {
        let mut out = json!({
            "consumer": self.consumer,
            "context_recovery": self.context_recovery,
            "coordination_rule": self.coordination_rule,
            "when_stuck": self.when_stuck,
            "commands": self.commands
                .iter()
                .map(|c| json!({ "invocations": c.invocations, "summary": c.summary }))
                .collect::<Vec<_>>(),
            // Where the declarations came from and where they belong (nxf 6j6v.dvyq). ALWAYS
            // present, unlike the additive sections below it: its whole job is to make the EMPTY
            // case say something, and a field that disappears exactly when the workspace has
            // nothing to show would answer the question only when it was not being asked.
            "declarations": self.declarations.to_value(),
        });
        if !self.wake.is_empty() {
            out["threads_you_opened"] =
                serde_json::to_value(&self.wake).expect("opener wake serializes");
        }
        if !self.roster.roles.is_empty() {
            out["roles"] = json!(self.roster.roles);
        }
        if !self.roster.channels.is_empty() {
            out["channels"] = json!(self.roster.channels);
        }
        // Additive and omitted when empty, exactly like the three above: a workspace that declares
        // no team and a caller that is not a persona both keep the pre-p6m1 shape byte for byte.
        if let Some(persona) = &self.persona {
            out["persona"] = serde_json::to_value(persona).expect("persona brief serializes");
        }
        if !self.directory.is_empty() {
            out["address_book"] =
                serde_json::to_value(&self.directory).expect("directory serializes");
        }
        // Present ONLY when the caller asked for them AND there is something to report — absent (not
        // an empty array) in every other case, including a spawned context with real errors.
        if interactive && !self.roster.errors.is_empty() {
            out["declaration_errors"] = json!(self
                .roster
                .errors
                .iter()
                .map(|e| json!({ "file": e.file, "what": e.what }))
                .collect::<Vec<_>>());
        }
        // The same convention and the same gate, one question over (nxf 6j6v.9w08): a separate key
        // rather than a `severity` field on one list, because a consumer that acts on an error —
        // by refusing, by highlighting an excluded channel — must not act the same way on a
        // declaration that loads and runs.
        if interactive && !self.roster.warnings.is_empty() {
            out["declaration_warnings"] = json!(self
                .roster
                .warnings
                .iter()
                .map(|w| json!({ "file": w.file, "what": w.what }))
                .collect::<Vec<_>>());
        }
        out
    }

    /// The human session-start block as valid, structured Markdown — the SessionStart-hook output a
    /// host injects verbatim. No trailing newline (the caller's `println!` supplies it), so this
    /// composes cleanly into a larger document too. `declaration_errors` is the caller's rendering
    /// decision, exactly as in [`to_value`](PrimeReport::to_value).
    ///
    /// Each trailing section is omitted entirely when it has nothing to say.
    ///
    /// **NO MESSAGE TEXT REACHES THIS BLOCK** — not the unread (nxf 6j6v.4mmk) and, since nxf
    /// 6j6v.1gm9, not the requester wake either. What this block costs is paid by EVERY session, so
    /// what stands in it has to pass two tests: the session cannot get it any other way, and it
    /// needs it BEFORE it acts. Message bodies fail both, and the second ticket is where that
    /// stopped being an argument and became a measurement.
    ///
    /// `## Unread` went first, at 199.491 bytes and 98 % message text. `## Threads you opened`
    /// (`render_opener_wake`, removed with it) went second, and it was the larger half by the time
    /// it was measured: **46.487 of a 63.787-byte composed session start, 74 %** — 24 boards, six of
    /// them complete, each replaying its whole conversation, so a finished commission from yesterday
    /// was read out to every session today. Above the host's cut-off the yield of the whole block is
    /// not smaller but NIL (see `nxs::prime`), which is what made this a cliff rather than a price.
    ///
    /// **The owner's decision (2026-08-27): message text does not belong in a session start at
    /// all** — "Ich sehe keinen Mehrwert darin, dass bei einem Sitzungstart irgendwelche Nachrichten
    /// in den Kontext gespuelt werden." The code agrees with it: all THREE delivery paths PUSH.
    /// `send` opens a fresh session with the body in its prompt; `reply` resumes the target with
    /// this reply's body (`orchestration`'s `resume_return_address`); a completed quorum wakes the
    /// opener (`wake_role_requester`). [`Engine::prime_as`](crate::engine::Engine::prime_as) says of
    /// itself that the wake is "the same derivation this renders as 'Threads you opened'", so the
    /// section was the duplicate of what arrives anyway — held down by a read cursor that only
    /// `nxc read` moved, a verb used in 0 of 66 measured role sessions. The brake was never on.
    ///
    /// **Nothing replaced it, and the one candidate was checked rather than assumed.** "A thread
    /// awaiting a reply from THIS session" is a fact a session must have — but it was never in this
    /// section: the wake is the OPENER's view, and on every board in it `expects_reply_from` names
    /// OTHER handles, never the caller (`prime_golden.rs`'s
    /// `the_wake_never_carried_what_this_session_owes`). So the removal loses none of it. Should
    /// that line ever be wanted it is a NEW derivation over `expects_reply_from`, argued on its own
    /// merits, not a survivor of this one.
    ///
    /// **The one delivery path without a push does not change that**, and the code names it
    /// (`orchestration`'s `resume_return_address`): a target with no return address, or one that is
    /// no longer a registered role session, "triggers nothing — an ordinary case … not a skipped
    /// wake either: nothing was owed one". Both of its cases belong elsewhere. A post to a HUMAN is
    /// read at the terminal — `nxc threads show` / `nxc status`, the human surface this ticket
    /// leaves untouched, because a human at a terminal has no prompt to push into. And a session
    /// that no longer exists is not helped by a session-start block: a new session has a new id and
    /// would not be resumed regardless; it would only catch the text by accident, together with
    /// everyone else's.
    ///
    /// The size of this output is therefore independent of how much is unread AND of how many boards
    /// have completed. One of those two records lives on for the seam, [`wake`](PrimeReport::wake);
    /// the unread half went outright in nxf 6j6v.4d2z, fields and renderer together, so "unread" is
    /// no longer a thing this record can be independent OF.
    ///
    /// **Superseded 2026-08-28 (nxf h4d3, task 3, q3fh session-start-budget).** Everything above
    /// this paragraph is still true — no message text, size independent of unread/completed boards —
    /// but it stopped being the WHOLE story: the fixed prose, MEASURED (`nxc init && nxc prime` in a
    /// bare, empty workspace — the fixed prose is the whole output there, since nothing is
    /// declared), was **1.938 B**. The host drops any single hook output above 10.240 B entirely
    /// rather than delivering it smaller, so this fixed prose is a cost paid even before a workspace
    /// has anything to say — and in `watch-bundestag`, the real project workspace this branch is
    /// measured against, the SAME fixed prose sat inside a 3.394 B total (task-3 brief). The
    /// remaining 1.456 B is NOT all "## Who you can address" — that section is 1.308 B, unchanged by
    /// this task (2.028 B after, minus the 720 B fixed prose after, below); the other ~148 B was the
    /// "## Declared Team" section this task removes. **Corrected here (review of task 3):** an
    /// earlier draft of this paragraph called the whole 1.456 B remainder "the untouched address
    /// book", which the arithmetic does not support (3.394 − 1.938 = 1.456 ≠ 1.308). The
    /// owner's own edited target draft (`nxc-static-target.md`) cuts the fixed prose to **720 B**,
    /// MEASURED the same way: the title gains a subtitle, the coordination rule survives as a
    /// prohibition folded into one intro paragraph ([`PRIME_INTRO`]) instead of a separately
    /// labelled rule, and "## How a conversation moves" replaces "## Chat Commands" with three verbs
    /// instead of eight ([`PRIME_HOW_A_CONVERSATION_MOVES`], not a loop over [`PRIME_COMMANDS`]).
    /// `watch-bundestag`'s own composed `nxc prime` is now **2.028 B**, MEASURED against the live
    /// workspace — under the task-3 brief's 2.500 B ceiling, and byte-identical to
    /// `NXC_PRIME-full-example.md`.
    ///
    /// **Dropped outright, not shrunk:** the Context Recovery blockquote ([`PRIME_CONTEXT_RECOVERY`]
    /// — re-running `nxs prime` is not a fact a session needs before it acts); five of the eight
    /// verbs (`list`, `status`, `threads show`, `search`, `transcript show` — `nxc --help` teaches
    /// all five on demand, and the new intro paragraph says so); the when-stuck paragraph
    /// ([`PRIME_WHEN_STUCK`] — already the fuller account in `nxc guide limits-and-safety`,
    /// `docs/guide/en/limits-and-safety.md`); and the "## Declared Team" section
    /// ([`render_declared_team`] — a bare `roster.roles`/`roster.channels` name list that read as
    /// noise beside the richer section right beneath it, "## Who you can address", which says who
    /// each one is AND what for. **Not a strict subset**, corrected here after review (2026-08-28):
    /// a channel-only persona (`p.direct == false`) is not itself a line in the address book — it
    /// moves to its channel's entry, which is how it is actually reached
    /// ([`Directory::render_markdown`](crate::persona::Directory::render_markdown)'s own doc
    /// comment, nxf 6j6v.frek) — and a persona priming as itself is never in its own address book
    /// (see `prime_for_a_persona_puts_its_identity_and_nxc_rules_between_the_rule_and_the_commands`
    /// in `prime_golden.rs`, whose golden lost its trailing roster line for exactly this reason).
    /// Neither case loses a declared name outright; what moved is where it is said).
    ///
    /// **"## Who you can address" is unchanged** — [`self.directory`](PrimeReport::directory)'s own
    /// render is the one thing the task-3 brief said must stay byte-identical, and it does: this
    /// method still calls [`Directory::render_markdown`](crate::persona::Directory::render_markdown)
    /// exactly as before.
    ///
    /// **Every field this used to render stays exactly as it was — this is a rendering cut, not a
    /// data cut.** `to_value()` still serializes `context_recovery`, `coordination_rule`,
    /// `when_stuck`, and every one of the `commands` verbatim, and [`render_declared_team`] stays public for
    /// a caller that wants the roster rendered that way even though this method no longer calls it —
    /// see each constant's/function's own doc comment for the full account.
    ///
    /// **Two sections joined the interactive-only tail (nxf 6j6v.9w08):**
    /// [`PRIME_WRITING_DECLARATIONS`], one 92 B line pointing at the topic that would have
    /// prevented them, and [`render_declaration_warnings`] above the declaration errors that were
    /// already there. All three ride ONE flag, which is the whole of the interactive question: a
    /// human at a keyboard is the only reader who can act on any of them, and a spawned persona
    /// paying 92 B per session for a guide about its author's job is the cost this block exists to
    /// refuse. The spawned view is therefore BYTE-IDENTICAL to what it was before this item, which
    /// is what `contract.trycmd`'s `NXC_ACTOR=alice nxc prime` golden pins.
    pub fn render_markdown(&self, interactive: bool) -> String {
        // The persona's own identity sits between the intro paragraph and the command reference:
        // "who am I" should anchor how everything after it is read (the same argument
        // `compose_system_prompt` makes for putting the prime block ahead of even `CLAUDE.md`). A
        // human has no such block and the output is byte-identical to before this section existed.
        let identity = match &self.persona {
            Some(persona) => format!("{}\n\n", persona.render_markdown()),
            None => String::new(),
        };
        let mut md = format!(
            "# nexus-chat — the channel between agents\n\n\
             {}\n\n\
             {identity}\
             ## How a conversation moves\n\n\
             {}",
            PRIME_INTRO, PRIME_HOW_A_CONVERSATION_MOVES,
        );
        // The declarations area, in the order a reader works through it: who is there, where the
        // files live, the one line about writing them well, then what is wrong with the ones here —
        // warnings (it loads and will work badly) before errors (it does not resolve and something
        // above is excluded). The last three are INTERACTIVE-ONLY, on nxf 6j6v.v9k3's rule: a
        // spawned persona is not shown its author's mistakes, nor sent to read a guide about them.
        for section in [
            self.directory.render_markdown(),
            render_declaration_source(&self.declarations),
            match interactive {
                true => PRIME_WRITING_DECLARATIONS.to_string(),
                false => String::new(),
            },
            match interactive {
                true => render_declaration_warnings(&self.roster.warnings),
                false => String::new(),
            },
            match interactive {
                true => render_declaration_errors(&self.roster.errors),
                false => String::new(),
            },
        ] {
            if !section.is_empty() {
                md.push_str("\n\n");
                md.push_str(&section);
            }
        }
        md
    }
}

// `render_unread_blocks(in_turn, next_session)` stood here — the Markdown renderer for the
// catch-up, kept public after `prime`'s own block stopped calling it (nxf 6j6v.4mmk) so an
// embedding host would not have to re-derive the format its seam fields were shaped for. Those
// fields are gone (nxf 6j6v.4d2z) and so is the renderer: a formatter for a record that no longer
// exists is a false map, not a courtesy.

// `render_opener_wake` stood here, with `render_completed_thread` and `render_stale_thread` under
// it — the "## Threads you opened" block of spec §5.2. **REMOVED, ersatzlos, by nxf 6j6v.1gm9**;
// the argument is on `PrimeReport::render_markdown`, which was its only caller after `nxc inbox`
// went in the same change. In one line: it re-rendered, at every session start and in full text,
// results that all three delivery paths had already PUSHED — and the read cursor that was meant to
// bound it moved only under `nxc read`, used in 0 of 66 measured role sessions.
//
// **The derivation itself stays** (`opener_wake`, `store::OpenerWake`) and so does
// `PrimeReport::wake` and its `threads_you_opened` JSON: this ticket took a RENDERING off the
// agent surface, not a read off the app seam (that question is nxf 6j6v.4d2z). `nxc threads show`
// and `nxc status` answer the same question for a human, unchanged.

/// Render the "## Declared Team" section (nxf ticket 6j6v.v9k3): whichever of the clean
/// roles/channels are non-empty. Returns `""` when the roster is entirely empty (nothing declared
/// at all) so the caller can omit the section cleanly, the same empty-string convention every
/// section renderer here follows.
///
/// **`PrimeReport::render_markdown` no longer calls this** (nxf h4d3, task 3): the bare name list
/// it renders read as noise beside "## Who you can address" right beneath it, which says who each
/// one is AND what for — not a strict superset (a channel-only persona is a channel-entry line
/// there, not a roster line; a self-priming persona's own handle is absent from its own address
/// book), but close enough in the common case that a second, terser listing earned its byte cost
/// less and less as the session-start budget tightened. See
/// [`PrimeReport::render_markdown`]'s doc comment for the full account. It stays public — and
/// now covered by direct unit tests in this file's own `#[cfg(test)] mod tests`, since its one
/// caller here left (review of task 3, Important 2) — for the reason `render_unread_blocks` beside
/// it used to share before nxf 6j6v.4d2z took that one away with its record: an embedding host that
/// wants the roster drawn this way should not have to re-derive the format from
/// `PrimeReport::roster` (`roles`/`channels` in `--json`) itself.
pub fn render_declared_team(roster: &PrimeRoster) -> String {
    if roster.roles.is_empty() && roster.channels.is_empty() {
        return String::new();
    }
    let mut lines = vec!["## Declared Team".to_string()];
    if !roster.roles.is_empty() {
        lines.push(format!("- **Roles:** {}", roster.roles.join(", ")));
    }
    if !roster.channels.is_empty() {
        lines.push(format!("- **Channels:** {}", roster.channels.join(", ")));
    }
    lines.join("\n")
}

/// Render the "## Declarations" section (nxf 6j6v.dvyq): where this workspace's declarations came
/// from, and where they belong.
///
/// Deliberately SILENT for exactly one case, matched below as `(Personas, None)` — mirroring
/// [`DeclarationSource::explain`]'s own `(Personas, None)` arm (`definitions.rs:198–204`):
/// declarations read from the canonical `.nxs-personas/` folder, with no legacy `roles/` folder
/// anywhere in this workspace. `explain`'s text for that case is just "read from {path}
/// ({count} declared)." — no clause telling the reader to do anything, because there is nothing
/// to do: no migration is pending and no legacy leftover needs cleaning up. The other three arms
/// this function DOES render for are exactly `explain`'s other three (`definitions.rs:174–197`),
/// and each one is reporting something actionable that only `DeclarationSource` knows: nothing is
/// declared anywhere yet, so where to put the first file (`(None, _)`; see `nothing_declared` in
/// `tests/prime_golden.rs` for that message); declarations were read from the LEGACY folder, so a
/// move is overdue (`(LegacyRoles, _)`); or `.nxs-personas/` won the tie but a legacy folder is
/// still sitting there unread (`(Personas, Some(legacy))`).
///
/// **Corrected here (review of task 3, round 2):** the previous wording justified the silence by
/// saying the path this section would print is "not news to an agent that already read '## Who you
/// can address'". Checked directly against [`Directory::render_markdown`] (`persona.rs:411–436`,
/// via `render_persona_entry`/`render_channel_entry` at `persona.rs:449–492`): that section renders
/// handles, titles, seniority, members and `why`/description text, and never a path. There was
/// nothing for the personas path to be "not news" relative to — that reasoning was simply wrong,
/// not merely stale. **Before that, this same paragraph pointed at the "## Declared Team" section
/// instead** (superseded 2026-08-28, nxf h4d3, task 3 review): that section rendered here until
/// this same task's own change removed it a few lines above, and the justification was left
/// pointing at a section that no longer exists.
pub fn render_declaration_source(source: &DeclarationSource) -> String {
    match (source.kind, &source.legacy_path) {
        (crate::definitions::DeclarationSourceKind::Personas, None) => String::new(),
        _ => format!("## Declarations\n\n{}", source.explain()),
    }
}

/// Render the "## Declaration Warnings" section (nxf 6j6v.9w08) — one `{file}: {what}` line per
/// warning, in the same shape as [`render_declaration_errors`] below because the two are read in
/// one pass by one reader. Returns `""` when there is nothing to say; the interactive-vs-spawned
/// gating is the caller's, exactly as it is for the errors.
///
/// **A separate heading rather than one list with the errors**, because they ask the reader for
/// different things: an error means a declaration does not resolve and something is excluded from
/// the roster right above; a warning means a declaration resolves, loads and runs, and will work
/// badly. Folding them would make the warnings read as breakage — which they are not — or the
/// errors read as advice, which they are not either.
pub fn render_declaration_warnings(warnings: &[DeclarationWarning]) -> String {
    if warnings.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = warnings
        .iter()
        .map(|w| render_finding(&w.file, &w.what))
        .collect();
    format!("## Declaration Warnings\n\n{}", lines.join("\n"))
}

/// The one line `nxc prime` spends on the topic that would have prevented the warnings above (nxf
/// 6j6v.9w08), rendered at the end of the declarations area and only where somebody is there to act
/// on it.
///
/// **It earns its place against a measured budget, and the argument is that it reaches ONE reader.**
/// nxf 6j6v.waq9 measures 85 % of a session start as memories in full text and h4d3's task 3 cut
/// this block's fixed prose to 720 B on the way under the host's 10.240 B hook ceiling, so a line
/// here has to say what it costs. MEASURED: 90 B, 92 with the blank line that separates it from the
/// section above; it is paid only in an interactive context, and
/// the reader it is for is the one about to write the declaration — which is the one moment the
/// guide is worth anything at all.
///
/// **Rendered only, with no field on [`PrimeReport`] and no `--json` key**, following
/// [`PRIME_INTRO`] and [`PRIME_HOW_A_CONVERSATION_MOVES`] beside it rather than `context_recovery`
/// and `when_stuck`: it is a fixed sentence of the human view, not a record an embedding app would
/// render differently. An app that wants to say the same thing writes its own.
///
/// **Interactive only, the same cut as the errors and the warnings**, and the reason is the one
/// `personas.md` already gives for referential problems: a spawned persona is not shown its
/// author's mistakes. It is also not the persona's job — a role that WRITES declarations is told to
/// read this topic by its own `system_prompt`, which is one stable line pointing at a guide rather
/// than a copy of the guide, and therefore the one thing class 1 of that guide does not forbid.
pub const PRIME_WRITING_DECLARATIONS: &str =
    "> **Before you write or change a persona or a channel:** `nxc guide writing-declarations`.";

/// Render the "## Declaration Errors" section (nxf ticket 6j6v.v9k3) — one `{file}: {what}` line
/// per error. Returns `""` when `errors` is empty so the caller can omit the section cleanly;
/// callers are responsible for the interactive-vs-spawned gating (this function has no notion of
/// it, mirroring [`PrimeRoster`]'s own aggregation-layer purity).
pub fn render_declaration_errors(errors: &[ValidationError]) -> String {
    if errors.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = errors
        .iter()
        .map(|e| render_finding(&e.file, &e.what))
        .collect();
    format!("## Declaration Errors\n\n{}", lines.join("\n"))
}

/// One `{file, what}` finding as a Markdown bullet — shared by both renderers above so the two
/// sections cannot drift, and so the defence below is written once.
///
/// **`file` is not a trusted string** (review of PR #465, Integrity #1). For a channel it is the
/// constant `channels.yaml`, but for a persona it is derived from the declared `handle`, and
/// `definitions::validate_role_handle` — which does not run on this path anyway — rejects only an
/// empty handle, `/`, `\`, `..` and the reserved `__` prefix. A backtick in it closes the code span
/// this bullet puts it in; a newline in it forges a second bullet, or a heading, in a block a host
/// injects verbatim into a session. Every other author-written string these warnings echo goes
/// through `{:?}` already ([`crate::declaration_quality`]); this one could not, because it is meant
/// to read as a filename.
///
/// So the ordinary case is unchanged — `` - `coder.yaml`: … `` byte for byte — and a name that
/// CANNOT be a filename is rendered as an escaped literal outside the code span instead, where
/// `{:?}` turns its control characters into `\n`-style escapes and the backticks it is stripped of
/// can no longer open a span. The render is deliberately lossy there: a handle with a backtick in
/// it has no faithful filename to show, and making it inert matters more than reproducing it.
fn render_finding(file: &str, what: &str) -> String {
    let breaks_out = |c: char| c == '`' || c.is_control() || c == '\u{2028}' || c == '\u{2029}';
    match file.contains(breaks_out) {
        false => format!("- `{file}`: {what}"),
        true => format!("- {:?}: {what}", file.replace('`', "'")),
    }
}

/// Compute the `nxc prime` session-bootstrap record (spec §5.2): pin the coordination rule, name the
/// core verbs, and add the requester wake plus the declared-team roster. Wired via `nxs prime`'s
/// fan-out. (It replayed the consumer's entire unread set as well — both dispositions, because a new
/// session was the catch-up moment — until nxf 6j6v.4d2z removed the unread set.)
///
/// Every read goes through this layer's own seams ([`opener_wake`], [`validate_declared_team`]),
/// never the store directly: the CLI used to reach past the facade for exactly these, which is why
/// none of it was reachable from the library. `now` drives only
/// the wake's clock-dependent `stale` flag; `declarations` is the resolved declaration source (the
/// caller resolves it — the facade does not know the workspace layout).
pub fn prime(
    store: &ChatStore,
    consumer: &str,
    now: &str,
    declarations: &DeclarationSource,
) -> Result<PrimeReport> {
    prime_for(store, consumer, None, now, declarations)
}

/// [`prime`] for a caller that is a declared PERSONA (nxf 6j6v.p6m1) — the same record, plus the
/// two things a persona needs and a human does not: who it is, and whom it may address.
///
/// `persona` is the bare handle the CALLER resolved (see [`crate::persona::resolve_identity`]); this
/// layer does not guess at identity, exactly as it does not read the clock or the environment. A
/// handle that no longer resolves to a declaration degrades to the human record rather than failing:
/// a session whose role file was deleted mid-flight still deserves its inbox.
pub fn prime_for(
    store: &ChatStore,
    consumer: &str,
    persona: Option<&str>,
    now: &str,
    declarations: &DeclarationSource,
) -> Result<PrimeReport> {
    // The catch-up was pulled here — both dispositions off `inbox`, partitioned onto the report's
    // `in_turn`/`next_session`. It went with the unread apparatus (nxf 6j6v.4d2z), and with it the
    // one read on this path that could fail on data rather than on IO: a row whose `disposition`
    // column held neither value was an `io` error, deliberately fail-closed (independent review,
    // 6j6v.r5a2). That trade has no site left here; what still holds the same rule one layer down
    // is the reducer, which refuses to fold an envelope whose disposition it does not know
    // (`an_unknown_disposition_op_is_stored_but_never_folded`).
    //
    // The declarations are read a second time here (`validate_declared_team` loads its own copy to
    // partition them): a handful of small YAML files, once per prime, in exchange for leaving that
    // function's roster contract — and every caller of it — untouched.
    let decl_dir = declarations.read_dir.as_path();
    let roles = crate::role::load_all_roles(decl_dir)?;
    let channels = crate::channel::load_all_channels(decl_dir)?;
    let brief = persona
        .and_then(|handle| roles.iter().find(|r| r.handle == handle))
        .map(|decl| PersonaBrief::from_decl(decl, &channels));
    let directory = match persona {
        Some(handle) => Directory::for_persona(&roles, &channels, handle),
        None => Directory::full(&roles, &channels),
    };
    // **The caller's own previous session end** (nxf 6j6v.2hx9) — the watermark the notice below is
    // a window over.
    //
    // Resolved from the CONSUMER, with the host's `persona` as the override. The notice is about the
    // commissions this CONSUMER opened, so the watermark has to belong to the same identity: taking
    // it from `persona` alone would read one identity's threads against another identity's clock the
    // moment a host passes both, which is exactly what `--consumer` exists to allow.
    // `session_map.role` holds the BARE handle, which is the half a qualified chat identity carries
    // after the origin.
    //
    // A caller whose role has never had a session end on this device has no watermark — a human at a
    // terminal, or a persona's first start — and then there is no window at all rather than an
    // unbounded one.
    let watermark = store.previous_session_end(
        persona.unwrap_or_else(|| crate::orchestration::bare_handle_of(consumer)),
    )?;
    Ok(PrimeReport {
        consumer: consumer.to_string(),
        context_recovery: PRIME_CONTEXT_RECOVERY,
        coordination_rule: PRIME_COORDINATION_RULE,
        when_stuck: PRIME_WHEN_STUCK,
        commands: PRIME_COMMANDS,
        // **COMPUTED ON EVERY SESSION START, THOUGH THE MARKDOWN BLOCK NO LONGER DRAWS IT** (nxf
        // 6j6v.1gm9; named rather than optimized away, PR #385 review, Integrity #2). This does a
        // full-body `messages_in_thread` read per finished board inside the caller's window —
        // work the human `nxc prime` path then discards, since `render_markdown` stopped reading
        // `wake`.
        //
        // Measured before deciding: `nxc prime` is 40 ms in a DEBUG build in `watch-bundestag`, the
        // workspace with the heaviest wake in the fleet (6 complete boards, ~46 KB of bodies). The
        // ticket's subject was a CLIFF in rendered bytes — above the host's cut-off the whole block
        // yields nothing — and this is a local sqlite read that costs a fraction of a startup.
        //
        // Skipping it for `!json` was considered and refused: `to_value` and `render_markdown` take
        // ONE record, and a `prime` that filled it differently per view is the drift this type's own
        // doc warns about ("two ways to serialize one record is exactly how the two views drift
        // apart") — `parity.rs` compares `Engine::prime_as`'s report against BOTH CLI views for
        // exactly that reason. The same compute-stays/render-drops shape as 6j6v.4mmk's unread
        // fields, and it belongs to the same successor: whether this read survives at all is
        // **nxf 6j6v.4d2z**, which owns the read cursor that gates it.
        // **THE WINDOW, not a debt** (nxf 6j6v.2hx9): what this caller's own commissions did while
        // it was away, bounded by its previous session's end. Derived from the OPERATION — the
        // completing reply's own instant — rather than from a read cursor nothing ever advanced,
        // which is why the notice no longer accumulates and needs no clearing rule.
        //
        // `None` for a caller that is not a declared persona, and that is the item's own decision
        // rather than a gap: a human at a terminal has no session whose end could be a watermark,
        // and `nxc status` answers the same question on demand.
        wake: opener_wake(store, consumer, now, watermark.as_deref())?,
        roster: validate_declared_team(decl_dir)?,
        persona: brief,
        directory,
        declarations: declarations.clone(),
    })
}

// ---- the persona's composed session start (nxf 6j6v.k8zq) ------------------

/// One SIBLING module's rendered session-start block, as the composing adapter obtained it — the
/// text `nxf prime` / `nxm prime` writes to stdout, verbatim.
///
/// Opaque here on purpose. `crates/chat` is ONE module of the suite and knows nothing about the
/// others' stores, their reports or their binaries; what it can do is put their text in the right
/// place. Who obtains it is the adapter's business ([`ModulePrimes`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModulePrime {
    /// The module key, as the umbrella's registry spells it: `"flow"`, `"memory"`.
    pub module: String,
    /// That module's `prime` Markdown.
    pub text: String,
}

/// **Where the sibling modules' session-start blocks come from** (nxf 6j6v.k8zq) — the seam hole
/// that lets a persona be handed `nxs prime --persona <handle>` without `crates/chat` learning to
/// spawn `nxf` and `nxm`.
///
/// This is the layering decision the item asks to be made deliberately. Composing the whole block
/// needs three stores and the module registry; the registry lives in `crates/nxs`, which depends on
/// this crate and not the other way round. So the crate that OWNS the persona owns the composition
/// and the filter, and the modules' text arrives as data:
///
/// - `crates/nxs` implements it over its own `prime::fan_out` and hands it to
///   [`crate::run_from_with`], which is what the `nxc` binary calls — so a spawn from the CLI and
///   `nxs prime --persona` compose the same bytes from the same pieces;
/// - an embedding host implements it however it likes and reaches the result through
///   [`Engine::persona_prime`](crate::engine::Engine::persona_prime) — the item's own requirement,
///   because app-foundations starts role sessions of its own (`crates/agent-runtime`) and two
///   contracts for one prompt is exactly the split the verb-seam gate (6j6v.vtvs) exists against;
/// - nobody at all is a valid answer too: the persona then gets chat's own half, which is what
///   makes it able to answer, and the block simply carries no board and no memories.
///
/// `services` is passed in so a provider can skip the work for a module this persona filtered out —
/// the filter is a declaration about what reaches the session, and a module whose text is discarded
/// should not have been asked for it.
/// `Debug + Send + Sync` are supertraits rather than bounds at each use: a provider is held by a
/// long-lived [`Engine`](crate::engine::Engine) that is itself `Send + Sync` and by an
/// [`EngineConfig`](crate::engine::EngineConfig) that is `Debug`, so every consumer needs all three
/// and repeating them at the use sites only moves the requirement out of sight.
pub trait ModulePrimes: std::fmt::Debug + Send + Sync {
    /// The sibling blocks this persona's filter admits, in the umbrella's registry order, for the
    /// workspace whose store is at `db_path`.
    fn module_primes(&self, services: PrimeServices, db_path: &str) -> Result<Vec<ModulePrime>>;
}

/// The order the composed block puts the suite's modules in — the umbrella's own registry order, so
/// `nxs prime --persona <handle>` and the prompt a spawned persona is handed are assembled the same
/// way and can be compared byte for byte.
///
/// **memory comes LAST since nxf 6j6v.xbnh, and it displaced chat from that position.** The earlier
/// argument for chat-last was that the persona's identity should be the nearest thing above the job
/// it is about to be given. It is a good argument and it lost to a measured one: the host TRUNCATES
/// a large session start — 25.893 bytes pass, 32.000 are filed away behind a 2 KB preview — so the
/// last block is not the most emphatic one, it is the one a truncation eats. What may be eaten is
/// what a session can fetch afterwards, and that is memory (`nxm recall <key>`,
/// `nxm memories <text>`) — not the board, not the command vocabulary, and not who you are and who
/// is waiting on you, none of which is written down anywhere else. Identity is still last among the
/// blocks that must survive.
/// **Kept in step with `nxs_init::ModuleInit::order` by hand**, because chat does not depend on the
/// registry crate and so cannot read the ordinals (review of PR #381, Code Quality #1). `pub` for
/// exactly one reason: `crates/nxs` links both this crate and the registry, and
/// `the_two_module_order_tables_agree` there compares them so a half-done edit reddens instead of
/// leaving a persona reading its blocks in a different sequence from every other session.
pub const PRIME_MODULE_ORDER: &[&str] = &["flow", "chat", "memory"];

/// **Compose what a persona is handed at session start** (nxf 6j6v.k8zq): the sibling modules'
/// blocks its filter admits, then chat's own persona block, joined as one Markdown document.
///
/// `siblings` is what a [`ModulePrimes`] provider returned — already filtered, but filtered again
/// here, because a provider is a host's code and the DECLARATION is what decides. A module the
/// filter excludes contributes nothing, not an empty heading: `nxs prime --persona <a-reviewer>`
/// carries no `nxf prime` section at all, while another persona's carries one.
///
/// # The sections are joined, not fenced — and who reads them changed here
///
/// Sections are separated by a blank line and told apart by their own `# ` heading, and each
/// module's content is interpolated verbatim. A memory body or a ticket title that itself contains
/// `# nexus-chat` is, in the joined text, indistinguishable from a real section.
///
/// **The verbatim rendering is not new; the AUDIENCE is** (review of PR #379, Integrity &
/// Robustness #3). `nxm prime` and `nxf prime` have always rendered their content this way and a
/// human at a terminal has always read it. Before nxf 6j6v.k8zq a spawned persona got none of it;
/// now `prime: true` is the default for every service, so the same text goes into the system prompt
/// of an agent that then uses tools. That is a different trust level, even though it is not a
/// remote-attacker shape: this workspace's board and memories are written by its own people and its
/// own agents.
///
/// Deliberately NOT fenced or escaped here, and deliberately recorded rather than left implied:
/// escaping Markdown inside a prompt is not a solved problem, and a wall of delimiters costs
/// context in every session for a boundary a model only ever sees as more text. Which of the three
/// answers this gets — say nothing and mean it, fence each module, or name the provenance in one
/// sentence — is **nxf 6j6v.mb05**, taken as a decision rather than patched in as a reflex.
///
/// `declarations` is `None` for a catalogue a caller built in memory ([`crate::definitions::
/// Definitions::new`]), which has no folder — and chat's own half is then omitted rather than
/// composed against a folder that is not there. [`prime_for`] re-reads that folder to build the
/// roster and the address book (its own doc says why), so the alternative is a block that shows a
/// persona an empty team and an empty address book while its peers sit in the catalogue beside it:
/// a confident lie where an omission is merely a gap. Everything that reaches this through `nxc` or
/// through [`Engine`](crate::engine::Engine) was resolved from a workspace and passes `Some`.
pub fn compose_persona_prime(
    store: &ChatStore,
    consumer: &str,
    persona: &str,
    services: PrimeServices,
    siblings: &[ModulePrime],
    now: &str,
    declarations: Option<&DeclarationSource>,
) -> Result<String> {
    let mut parts: Vec<String> = Vec::new();
    for module in PRIME_MODULE_ORDER {
        let admitted = match *module {
            "flow" => services.flow,
            "memory" => services.memory,
            "chat" => services.chat,
            _ => true,
        };
        if !admitted {
            continue;
        }
        if *module == "chat" {
            // Rendered here rather than fetched: this is chat's own store, and a spawned session
            // has no terminal, so the declaration errors — and the quality warnings and the
            // writing-declarations pointer that ride the same `false` (nxf 6j6v.9w08) — stay out
            // exactly as they do for every other spawned context (see
            // `PrimeReport::render_markdown`). This call site is the one that makes "NOT in a
            // spawned persona's prompt" a property of the composer rather than of the CLI's env
            // sniffing: the persona's block is composed IN PROCESS here, never by shelling out.
            if let Some(declarations) = declarations {
                let report = prime_for(store, consumer, Some(persona), now, declarations)?;
                parts.push(report.render_markdown(false));
            }
        } else if let Some(sibling) = siblings.iter().find(|s| s.module == *module) {
            let text = sibling.text.trim();
            if !text.is_empty() {
                parts.push(text.to_string());
            }
        }
    }
    Ok(parts.join("\n\n"))
}

// ---- writes (now + acting-handle explicit) ---------------------------------

/// The receipt of a persisted `send`/`reply` — the minted message id + its thread (if any).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SendReceipt {
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

/// The receipt of a persisted `ask` (M2): the opened thread + its request message, the declared
/// expectations, and the resolved absolute `deadline` (skipped when none). Declared field order =
/// the `--json` contract `{ thread_id, message_id, persisted, expects, deadline? }` (spec §5.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AskReceipt {
    pub thread_id: String,
    pub message_id: String,
    pub persisted: bool,
    pub expects: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
}

/// A message to post via [`send`]. A named-field request struct rather than a positional argument
/// list: `now`/`origin`/`actor`/`channel`/`body` are all `&str`, so a positional API on this
/// long-lived public embedding surface would be a transposition hazard (be9y review CQ#2). The op
/// `sender`/author is `<origin>/<actor>` (identical to the CLI's `caller_handle()`); `now` stamps the
/// op wall_clock.
#[derive(Debug, Clone)]
pub struct SendRequest<'a> {
    pub now: &'a str,
    pub origin: &'a str,
    pub actor: &'a str,
    pub channel: &'a str,
    pub body: &'a str,
    pub kind: MessageKind,
    pub priority: Priority,
    pub disposition: Disposition,
    pub thread: Option<&'a str>,
    pub refs: Refs,
}

/// A reply to post via [`reply`] — like [`SendRequest`] but routed by `target` (a message or thread
/// id) instead of an explicit channel; the channel and thread are resolved from it.
#[derive(Debug, Clone)]
pub struct ReplyRequest<'a> {
    pub now: &'a str,
    pub origin: &'a str,
    pub actor: &'a str,
    pub target: &'a str,
    pub body: &'a str,
    pub kind: MessageKind,
    pub priority: Priority,
    pub disposition: Disposition,
    pub refs: Refs,
    /// Post ONLY if `<origin>/<actor>` still owes a reply on the resolved thread (nxf 6j6v.ww0a):
    /// it is named in the thread's `expects_reply_from` AND has not yet posted there. `false` (the
    /// default) is today's unconditional behaviour, unchanged. See [`reply`] for exactly what
    /// "still owes" means and why the check reuses [`ChatStore::thread_quorum`] rather than a
    /// second derivation.
    pub if_unanswered: bool,
}

/// A review board to open via [`ask`] (M2). Like [`SendRequest`] but ATOMIC: it also mints the
/// thread, declares `expect` (who must reply), and resolves an optional `--deadline`. `deadline` is
/// the RAW `--deadline` (an absolute RFC3339 instant OR a duration like `24h`); [`ask`] resolves it
/// to an absolute instant against `now` (TB-M2-6) before writing. The posted request `message`
/// carries the minted `thread_id` and defaults `kind` to `question` (CLI-side); disposition is
/// `in_turn` (the opener wants the board acted on now).
#[derive(Debug, Clone)]
pub struct AskRequest<'a> {
    pub now: &'a str,
    pub origin: &'a str,
    pub actor: &'a str,
    pub channel: &'a str,
    pub body: &'a str,
    pub expect: &'a [String],
    pub deadline: Option<&'a str>,
    pub kind: MessageKind,
    pub priority: Priority,
    pub refs: Refs,
}

/// Post a message to a channel. Rejection (the CLI contract, verbatim): unknown channel →
/// `not_found`. There is deliberately NO membership gate on `send` — any caller may post to an
/// existing channel (the CLI asymmetry).
pub fn send(store: &mut ChatStore, mut req: SendRequest) -> Result<SendReceipt> {
    req.actor = nxs_foundation::model::validate_author(req.actor)?;
    if !store.channel_exists(req.channel)? {
        return Err(NxfError::not_found(format!(
            "no such channel: {}",
            req.channel
        )));
    }
    store.set_wall_clock(req.now);
    let env = MessageEnvelope {
        origin: req.origin.to_string(),
        channel_id: req.channel.to_string(),
        sender: format!("{}/{}", req.origin, req.actor),
        kind: req.kind,
        priority: req.priority,
        disposition: req.disposition,
        thread_id: req.thread.map(str::to_string),
        refs: req.refs,
        body: req.body.to_string(),
    };
    let message_id = store.post_message(&env);
    Ok(SendReceipt {
        message_id,
        thread_id: env.thread_id,
    })
}

/// Reply to a message or thread `target`. Resolution is the CLI's, verbatim: a **message** id
/// inherits that message's thread; a **thread** id (via the `threads` view, else a message a
/// `send --thread` stamped) is carried; otherwise `not_found`.
///
/// **`req.if_unanswered` (nxf 6j6v.ww0a): posts only when the caller still owes a reply.** "Still
/// owes" is checked against the resolved thread's derived quorum — reusing
/// [`ChatStore::thread_quorum`] (the SAME derivation `threads list`/`threads show` and the app-facade
/// quorum bar read) rather than a second, independently-written SQL that could drift from it: the
/// qualified `<origin>/<actor>` must be named in the thread's `expects_reply_from` AND must not have
/// posted there yet — exactly [`ThreadQuorum::outstanding`] membership. A target with no thread at
/// all (a message-addressed reply outside any thread) trivially owes nothing.
///
/// When the check fails, this returns `Ok(None)` — a deliberate, successful no-op — rather than
/// writing anything or erring: this flag exists precisely so a caller that cannot tell whether it
/// already answered (the agent sidecar's unconditional teardown call) can ask unconditionally too.
/// A trigger into a thread that ALREADY carries somebody else's quorum falls out of the SAME rule
/// with no special case: it deliberately never registers the triggered role as owing a reply at all
/// (nxf 6j6v.enrs), so `expects_reply_from` never names it and this correctly no-ops there too —
/// not a carve-out, just what the same derivation already says. It was reachable as
/// `send --role --thread <id>` until 6j6v.dvyq §3 removed both flags; `orchestration`'s own callers
/// still reach it, and the derivation never depended on which door was used.
///
/// `Ok(Some(receipt))` — the ordinary shape — for every other call, `req.if_unanswered` true or not.
pub fn reply(store: &mut ChatStore, mut req: ReplyRequest) -> Result<Option<SendReceipt>> {
    req.actor = nxs_foundation::model::validate_author(req.actor)?;
    let (channel, thread): (String, Option<String>) =
        if let Some((ch, th)) = store.message_channel(req.target) {
            (ch, th) // target is a message id — inherit its thread (if any)
        } else if let Some(ch) = store
            .thread_channel(req.target)
            .or_else(|| store.thread_channel_via_message(req.target))
        {
            (ch, Some(req.target.to_string())) // target is a thread id — carry it
        } else {
            return Err(NxfError::not_found(format!(
                "no such thread or message: {}",
                req.target
            )));
        };
    if req.if_unanswered && !owes_reply(store, thread.as_deref(), req.origin, req.actor, req.now)? {
        return Ok(None);
    }
    store.set_wall_clock(req.now);
    let env = MessageEnvelope {
        origin: req.origin.to_string(),
        channel_id: channel,
        sender: format!("{}/{}", req.origin, req.actor),
        kind: req.kind,
        priority: req.priority,
        disposition: req.disposition,
        thread_id: thread.clone(),
        refs: req.refs,
        body: req.body.to_string(),
    };
    let message_id = store.post_message(&env);
    Ok(Some(SendReceipt {
        message_id,
        thread_id: thread,
    }))
}

/// Whether `<origin>/<actor>` still owes a reply on `thread` (nxf 6j6v.ww0a) — [`reply`]'s
/// `if_unanswered` predicate, factored out so it says one thing once: named in the thread's
/// `expects_reply_from` AND not yet posted, i.e. membership in
/// [`ThreadQuorum::outstanding`](crate::store::ThreadQuorum::outstanding). `thread: None` (a
/// message-addressed reply that inherited no thread) trivially owes nothing — there is no
/// `expects_reply_from` register to owe against. Reuses [`ChatStore::thread_quorum`] verbatim; see
/// that function's own doc for the query.
///
/// `pub(crate)` since nxf 6j6v.fe0f: the working-tree release re-derives a promoted trigger's
/// obligation by asking this exact question about the ROLE it is about to fire, so that what the
/// role is told it owes and what `reply --if-unanswered` will accept as a discharge are one
/// predicate rather than two that can disagree.
pub(crate) fn owes_reply(
    store: &ChatStore,
    thread: Option<&str>,
    origin: &str,
    actor: &str,
    now: &str,
) -> Result<bool> {
    let Some(thread_id) = thread else {
        return Ok(false);
    };
    let handle = format!("{origin}/{actor}");
    Ok(store
        .thread_quorum(thread_id, now)?
        .is_some_and(|q| q.outstanding.contains(&handle)))
}

// `mark_read(store, now, consumer, channel, through)` stood here — the ack that advanced the
// synced read cursor, and with it `ReadReceipt` above. REMOVED with the apparatus (nxf 6j6v.4d2z):
// the cursor it moved had exactly one consumer left, the unread inbox, and no product code ever
// called it in five layers of pipe. It was the last WRITE on this seam whose fact nothing could
// read back (`crates/chat/tests/read_surface.rs`'s recorded-and-unreadable list, nxf 6j6v.dh41),
// and that list is empty now.

/// **Where does this operation stand** (nxf 6j6v.a71h §3.2) — the whole thread TREE, across channel
/// borders, with each thread's state derived rather than looked up.
///
/// Four forms, one read; see [`StatusScope`] for which operations each selects and why the channel
/// is not the anchor. Two of them LIST and filter on [`StatusOperation::live`]; `--thread` and
/// `--all` hand back what they select whether it is live or not. What comes back is assembled from bulk queries over the parent edges
/// ([`ChatStore::thread_edges`]), the quorum registers, the assignee return addresses and the
/// working-tree lease — and nothing else: **there is no operation record in the store**, and this
/// verb creates none (§3.3).
/// That is the point of it, not an implementation detail — the thing it replaced (`workflow status`)
/// read a stored run record that a `workflow start` had to write first.
///
/// # What folded in here, and what it cost (nxf 6j6v.yr59)
///
/// This read absorbed the other three quorum reads on the seam, on the item's own condition —
/// *"`threads` und `thread_board` gehen in `status` auf, sobald `status` je Faden genug traegt"*:
///
/// - `Engine::threads` — the caller's boards with their quorum and their working-tree state. What
///   it had over this read was `deadline` and the working-tree fields, and both are on
///   [`StatusThread`] now, so the fold is a collapse rather than a loss. What it had that does NOT
///   come along is MEMBERSHIP SCOPING, deliberately: an operation crosses channels its reader is
///   not in, and gating on membership answers "where do we stand" with a third of the chain (the
///   paragraph below says so, and it is the older decision of the two).
/// - `Engine::thread_board` — one board plus its reply MESSAGES. The quorum half is here; the
///   messages are [`thread`], the one message reader yr59 left standing, which applies the
///   channel's declared `visibility` exactly as the board did.
/// - `Engine::thread_quorums` — the explicit set, now [`StatusScope::Threads`].
///
/// `facade::threads`/`thread_board` themselves are untouched: `nxc threads list`/`show` still run
/// them, and `tests/seam_disposition.rs` records what left the HANDLE against what stayed on the
/// compute layer.
///
/// **The shape is read before the state.** The forest comes out of the covering index on `threads`
/// alone; the scope picks its operations from that, and only THEN are the expectations of the
/// threads actually being returned paid for. So `--thread` and `--channel` cost their own operation,
/// not the workspace — which matters because this is the read an app calls on every refresh. Bare
/// `nxc status` still reads everything, because that is literally what it was asked for.
///
/// **Not membership-scoped**, unlike [`threads`]. An operation crosses channels its reader is not in
/// — in a71h §2's own chain the human who opened T1 is not a member of the DM between the pm and the
/// coder — so gating on membership would answer "where do we stand" with a third of the chain. What
/// this read hands back is the SHAPE of the operation (which threads exist, who still owes, what is
/// orphaned, and which session is working on it), never message bodies; reading those still goes
/// through [`thread_board`], which gates.
///
/// **The same answer for every caller** (nxf 6j6v.1q6d): there is no caller handle in this
/// signature, so there is nothing for a projection to key on — no human/agent distinction exists
/// here, and none may be introduced. Pinned by
/// `status_reads_the_same_for_every_caller_because_there_is_no_caller`.
///
/// **The three states and the root's carve-out** are [`ThreadState`]'s and
/// [`StatusThread::awaiting_human`]'s own docs. Two structural notes that belong here:
///
/// - A thread whose declared parent names no thread this workspace has folded is GROUPED as a root
///   of its own. Ops arrive in relay order, so a child can be folded before its parent; showing it
///   as its own operation until the parent lands is one operation too many, which is the harmless
///   direction — the alternative would be to hide it. It still reports its declared `parent`, and
///   `awaiting_human` is computed from THAT rather than from the grouping, so a provisional root
///   never tells a human it holds a mid-chain branch (that flag's own doc says why the asymmetry
///   matters there and nowhere else).
/// - A cycle in the edges cannot arise from any write here (a parent always exists before its
///   child), but a synced or hand-edited log could carry one, and a walk that trusts the data would
///   hang. [`root_of_thread`] stops on revisiting a thread and takes the smallest id **of the cycle
///   itself**, so every node on or below the cycle lands in one operation.
pub fn status(
    store: &ChatStore,
    worker: &dyn crate::worker::Worker,
    now: &str,
    scope: StatusScope<'_>,
) -> Result<StatusReport> {
    let edges = store.thread_edges()?;
    let forest = Forest::build(&edges);

    // Which operations this scope is about — decided from the SHAPE alone, before any state is read.
    let selected: Vec<&str> = match scope {
        StatusScope::Threads(thread_ids) => thread_ids
            .iter()
            .map(|thread_id| {
                forest
                    .root_of
                    .get(*thread_id)
                    .copied()
                    .ok_or_else(|| NxfError::not_found(format!("no such thread: {thread_id}")))
            })
            // Deduplicated, and the ORDER is the forest's own rather than the caller's: two ids of one
            // operation must not render that operation twice, and the report's determinism is a
            // property of this read, not of how a quorum bar happened to sort its board list.
            .collect::<Result<BTreeSet<&str>>>()?
            .into_iter()
            .collect(),
        StatusScope::Workspace | StatusScope::All(None) => forest.roots.iter().copied().collect(),
        StatusScope::Channel(name) | StatusScope::All(Some(name)) => {
            let declared = format!("{}{name}", crate::orchestration::DECLARED_CHANNEL_PREFIX);
            forest
                .roots
                .iter()
                .copied()
                .filter(|r| {
                    forest
                        .channel
                        .get(r)
                        .copied()
                        .flatten()
                        .is_some_and(|c| c == name || c == declared)
                })
                .collect()
        }
    };

    // The threads of exactly those operations, and then their state — one pass for the ids, one bulk
    // read for the quorums, one for the assignee sessions.
    let mut trees: Vec<(&str, Vec<&str>)> = Vec::new();
    let mut wanted_ids: BTreeSet<&str> = BTreeSet::new();
    for root in selected {
        let mut ids = Vec::new();
        let mut seen = BTreeSet::new();
        forest.collect_ids(root, &mut seen, &mut ids);
        wanted_ids.extend(ids.iter().copied());
        trees.push((root, ids));
    }
    let id_list: Vec<&str> = wanted_ids.iter().copied().collect();
    let quorums = quorums_for(store, now, &id_list, edges.len())?;
    let by_id: BTreeMap<&str, &ThreadQuorum> =
        quorums.iter().map(|q| (q.thread_id.as_str(), q)).collect();
    let sessions = assignee_sessions_for(store, &id_list, edges.len())?;
    // What became of each of those sessions (nxf 6j6v.qmy6), asked ONCE per distinct session rather
    // than once per thread that names it — see [`session_states_for`] for the bound and for why an
    // absence from the ended set goes to the worker instead of reading as finished.
    let session_states = session_states_for(store, worker, &sessions)?;
    let moved_on = moved_on_for(store, &id_list, edges.len())?;
    // The escalation marker per thread (nxf 6j6v.mqad), read in bulk beside the others so the field
    // costs ONE more statement for the whole report rather than one per thread — the property
    // `tests/bulk_quorum.rs` measures.
    let last_replies = last_replies_for(store, &id_list, edges.len())?;
    // The working-tree half `threads`/`thread_board` used to carry, on the read they folded into
    // (nxf 6j6v.yr59). It is the SAME derivation those two shared, called once for the whole
    // report: O(chains competing for the tree), never O(threads on it) — see
    // [`working_tree_status_by_thread`]'s own doc for why that bound holds.
    let working_tree = working_tree_status_by_thread(store, now)?;
    // Where the tree stood at each thread's newest handover (nxf 6j6v.2af2) — the seventh bulk read,
    // in its siblings' shape and under their ceiling, so the field costs ONE statement for the whole
    // report rather than one per thread (`tests/bulk_quorum.rs` is what holds that).
    let anchors = anchors_for(store, &id_list, edges.len())?;
    // The standing holds (nxf 6j6v.npy3) — the eighth bulk read, in its siblings' shape and under
    // the same ceiling, keyed by the SESSION ids this report already collected rather than by
    // thread. An empty table (every workspace where nothing has ever been interrupted) is one
    // indexed read that finds nothing.
    let interruptions = interruptions_for(store, &sessions)?;
    // Unresumed parked work (nxf 6j6v.s9ex) — the ninth bulk read, guarded by
    // [`crate::store::ChatStore::any_parked_work`] rather than folded into the bundle below: unlike
    // its eight siblings this is not read per THREAD at all, it is attributed once per OPERATION
    // (root), which is what [`parked_work_for`]'s own doc explains.
    let parked = parked_work_for(store, &id_list, &forest.root_of)?;
    // Refused parks being retried (nxf 6j6v.8bv9) — the tenth read, attributed by root exactly as
    // the ninth is; one statement over a table that holds at most one row per claim.
    let park_refused = park_refusals_for(store, &id_list, &forest.root_of)?;
    // A withdrawn round still pinning its claim (nxf 6j6v.b9nf, Integrity #2 of this item's
    // review) — the eleventh read, attributed by root exactly as the tenth is; one statement over a
    // table that holds at most one row per claim, plus a worker call per session of the ONE claim
    // that wins attribution.
    let withdrawn = withdrawn_holders_for(store, worker, &id_list, &forest.root_of)?;
    // **One bundle, because these seven ARE one thing**: the bulk reads this report is assembled
    // from, taken once for the whole selection and then read per thread. Passing them
    // individually made the per-thread derivation an eight-argument call, and the argument list
    // was the wrong place to learn that — the shape a reader needs is that nothing here is read
    // per thread, which is the property `tests/bulk_quorum.rs` measures.
    let facts = DerivedFacts {
        by_id: &by_id,
        sessions: &sessions,
        session_states: &session_states,
        moved_on: &moved_on,
        last_replies: &last_replies,
        working_tree: &working_tree,
        anchors: &anchors,
        interruptions: &interruptions,
    };

    let mut operations = Vec::new();
    for (root, ids) in trees {
        let threads: Vec<StatusThread> = ids
            .iter()
            .filter_map(|id| forest.status_thread(id, &facts))
            .collect();
        let open = threads
            .iter()
            .filter(|t| t.state == ThreadState::Open)
            .count();
        // Both derived from the SAME `threads` this operation already carries, so a reader can
        // always find the thread behind either flag and the two can never disagree with the rows
        // beneath them.
        let needs_decision = threads.iter().any(|t| t.escalated);
        let holds_working_tree = threads
            .iter()
            .any(|t| t.working_tree == Some(WorkingTreeStatus::Holding));
        // **A failed dead end keeps its operation listed too** — the fourth term, derived here like
        // the other two and NOT carried as a flag of its own: [`ThreadState::Orphaned`] is on the
        // row that has it, and unlike `needs_decision` there is no measured case of a reader having
        // to walk a tree to find one. If that changes, this is where the flag joins its neighbours.
        let dead_end = threads.iter().any(|t| t.state == ThreadState::Orphaned);
        // Derived from the same rows as its two neighbours, for their reason (nxf 6j6v.npy3).
        let interrupted = threads.iter().any(|t| t.interrupted.is_some());
        // The facts that mean somebody or something is still ENGAGED here, and nothing else —
        // see [`StatusOperation::live`] for why the root's `awaiting_human` was struck out of this
        // sum by nxf 6j6v.1vxs.
        //
        // **A hold is one of them** (nxf 6j6v.npy3), and it has to be its own term rather than
        // riding on `open > 0`: the thread an interrupted session owes an answer on is open, so the
        // sum is usually already true — but a session can be interrupted on a turn whose thread was
        // discharged a moment earlier, which is the measured 0.6-second case, and an operation that
        // still has a standing hold is not finished just because nothing is outstanding. Without
        // this term such an operation would drop out of the default listing with a hold nobody ever
        // sees again.
        //
        // **So are work sitting on a park branch and a park being retried** (nxf 6j6v.8bv9,
        // requirement 7). Both outlive every other signal on purpose: a withdrawn or swept
        // operation is discharged, holds no copy and has nothing open — and its work is on a branch
        // nobody has come back for, or its tree is waiting for a merge somebody has to finish. An
        // operation that dropped out of the default view at that moment would hide the one thing a
        // reader still has to act on.
        let parked_here = parked.get(root).cloned().unwrap_or_default();
        let refused_here = park_refused.get(root).cloned();
        let withdrawn_here = withdrawn.get(root).cloned();
        let live = open > 0
            || needs_decision
            || holds_working_tree
            || dead_end
            || interrupted
            || !parked_here.is_empty()
            || refused_here.is_some()
            || withdrawn_here.is_some();
        // `--thread` shows the tree it was asked about and `--all` shows every tree, finished or
        // not; the two plain LISTING forms show only what is still going on.
        if matches!(scope, StatusScope::Threads(_) | StatusScope::All(_)) || live {
            operations.push(StatusOperation {
                root: root.to_string(),
                channel_id: forest
                    .channel
                    .get(root)
                    .copied()
                    .flatten()
                    .map(str::to_string),
                live,
                open,
                needs_decision,
                holds_working_tree,
                interrupted,
                threads,
                parked: parked_here,
                park_refused: refused_here,
                withdrawn: withdrawn_here,
            });
        }
    }
    Ok(StatusReport {
        operations,
        worker_answers_liveness: worker.answers_liveness(),
        worker_names_a_working_copy: worker.working_copy().is_some(),
    })
}

/// **The bulk reads one status report is assembled from**, gathered once for the whole selection
/// and then read per thread.
///
/// It exists because these six travel together and always have: every one of them is a single
/// query (or a single worker pass) over the report's WHOLE id list, and the per-thread derivation
/// only ever looks things up in them. Handing them over one by one said the opposite of that by
/// accident — it made the derivation read like a function of six independent inputs, and by nxf
/// 6j6v.qmy6 it was an eight-argument call.
///
/// One lifetime for all of them: they are borrowed from locals of [`status`] that outlive the loop.
struct DerivedFacts<'a> {
    by_id: &'a BTreeMap<&'a str, &'a ThreadQuorum>,
    sessions: &'a BTreeMap<String, String>,
    session_states: &'a BTreeMap<String, crate::orchestration::SessionState>,
    moved_on: &'a BTreeSet<String>,
    last_replies: &'a LastReplyMarks,
    working_tree: &'a HashMap<String, (WorkingTreeStatus, Option<i64>)>,
    /// Where the working copy stood at each thread's most recent handover (nxf 6j6v.2af2) — one
    /// bulk read for the whole report, like its six neighbours.
    anchors: &'a BTreeMap<String, crate::anchor::Anchor>,
    /// The availability boundaries the report's sessions are standing at (nxf 6j6v.npy3) — one bulk
    /// read for the whole report, like its seven neighbours, and keyed by SESSION rather than by
    /// thread for [`DerivedFacts::session_states`]'s reason: it is a fact about the session named on
    /// the row, so the two cannot come to disagree about one id.
    interruptions: &'a BTreeMap<String, crate::interruption::Interruption>,
}

/// **How many process questions ONE status read will ask** (review of PR #444, Integrity #1).
///
/// [`session_states_for`] asks the worker once per distinct un-ended session the report names, and
/// before this that loop had no ceiling at all: the id set grows with the workspace, `--all` selects
/// every operation there has ever been, and the whole read runs inside the handle's process-wide
/// lock — so an unbounded loop there is a stall for every other reader and writer. The SQL side was
/// already bounded ([`MAX_BOUND_IDS`]); that constant bounds the STRATEGY of one query and says
/// nothing about what happens after it.
///
/// Deliberately far above any real report — the workspace this feature was measured in named 305
/// distinct sessions across 411 threads — so it is a ceiling, not a tuning knob. What a reader gets
/// past it is on [`StatusThread::session_state`]: the third case of `Unknown`.
pub(crate) const MAX_LIVENESS_PROBES: usize = 512;

/// **Is this an id this device could ever have minted?** (review of PR #444, Integrity #2) — the
/// guard [`session_states_for`] puts between a status read and
/// [`Worker::session_is_running`](crate::worker::Worker::session_is_running).
///
/// The ids fed to the worker are not all local. A thread's assignee session can come from
/// `refs.session_id`, which is a free-form string that travelled with a message and may have been
/// written by another device. The shipped worker turns that string into a file path
/// (`<root>/.nxs/agent-logs/<id>.pid`) and does not sanitise it — a pre-existing property of
/// `SidecarWorker`, but before this feature nothing reached it except an id a human had typed at
/// `nxc session state`. This read reaches it automatically, for every session on the board, on
/// every call. So the check belongs HERE, at the one place that made the traffic.
///
/// The shape is the store's own: a minted id is `m-` followed by a ULID's alphabet. Anything
/// carrying a path separator, a `..`, or any other character is not one, and reads
/// [`SessionState::Unknown`](crate::orchestration::SessionState::Unknown) without being asked
/// about — the truthful answer for a session this device never ran.
fn is_mintable_session_id(id: &str) -> bool {
    id.len() > 2
        && id.len() <= 64
        && id.starts_with("m-")
        && id[2..].bytes().all(|b| b.is_ascii_alphanumeric())
}

/// **What became of every session a status report names** (nxf 6j6v.qmy6) — the same two sources
/// [`crate::orchestration::session_state`] reads, in the same order, for a whole report at once.
///
/// The announcement first (`session_map.ended`, one statement for the whole set), process liveness
/// as the backstop for a session killed before it could speak. Reading them the other way round
/// would let this and `nxc session state` disagree about one session, which is the failure two
/// readers of one fact are for.
///
/// **The bound, since this is the one read that leaves SQL and asks an operating system.** The
/// store side is ONE statement whatever the report's size — that is what
/// [`ChatStore::ended_sessions_among`] is for, and it falls back to the whole-table read at the
/// same ceiling every other bulk read here uses. The worker side is one question per DISTINCT
/// session that has not announced an end, up to [`MAX_LIVENESS_PROBES`]: a session on four threads
/// is asked about once, one that said it was over is not asked at all, and past the ceiling nothing
/// is asked at all. For the shipped worker each question is a small file read and a `kill(pid, 0)`;
/// in the proving-ground workspace this was measured in, that is 247 of them for a 411-thread
/// report, and none of them touches the database.
///
/// **It does not take the `ids.len() == total` shortcut its four sibling bulk reads take**, and
/// that is a difference rather than an oversight (review of PR #444, Integrity #3). Those compare
/// the requested ids against a total the caller already holds for free — `edges.len()`, the
/// workspace's thread count, read once for the forest. There is no free total here: how many
/// sessions this workspace has ever minted is its own query, so the shortcut would cost a statement
/// to save one. The ceiling above is what the shortcut buys elsewhere.
///
/// A session named by a thread but held by no row here — a return address that arrived by sync from
/// a device this one never was — falls through to the worker, which has no pid file for it and says
/// no. That is `Unknown`, and it is the honest answer: this device cannot say what became of a
/// session it never ran.
/// **The holds standing over the sessions this report names** (nxf 6j6v.npy3) — one statement for
/// the whole report, in [`session_states_for`]'s shape and under the same ceiling.
///
/// Keyed by session, distinct-ed first: a session standing on four threads is asked about once, and
/// every one of those rows reads the same answer rather than four reads that could differ.
///
/// Past [`MAX_BOUND_IDS`] it falls back to the whole-table read and filters in memory, which is what
/// every other bulk read here does at the same ceiling — and here the fallback is cheap by
/// construction: the table holds only holds nobody has taken up, which is none at all in a workspace
/// where nothing has ever been interrupted.
fn interruptions_for(
    store: &ChatStore,
    sessions: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, crate::interruption::Interruption>> {
    let distinct: BTreeSet<&str> = sessions.values().map(String::as_str).collect();
    if distinct.is_empty() {
        return Ok(BTreeMap::new());
    }
    let ids: Vec<&str> = distinct.iter().copied().collect();
    if ids.len() > MAX_BOUND_IDS {
        return Ok(store
            .standing_interruptions()?
            .into_iter()
            .filter(|i| distinct.contains(i.session.as_str()))
            .map(|i| (i.session.clone(), i))
            .collect());
    }
    store.interruptions_among(&ids)
}

/// **The unresumed parks of this report's operations, attributed by ROOT** (nxf 6j6v.s9ex) —
/// [`crate::store::ChatStore::any_parked_work`] first, so a workspace that has never parked pays
/// only that one existence check, then [`crate::store::ChatStore::all_parked_work`] once for the
/// whole workspace rather than once per operation.
///
/// **Attribution, not a second lookup by scope.** A park is recorded under the lease HOLDER key —
/// see [`StatusOperation::parked`]'s own doc for the exact call chain — which for the one scope
/// that can ever hold this workspace's working copy today is always `"thread:<id>"`. So every
/// `id` this report already walked (`id_list`, the same set every other bulk read here is scoped
/// to) is checked for a group under `"thread:<id>"`, and whatever is found is attributed to
/// `root_of[id]` — the OPERATION that thread belongs to, not necessarily the thread the row itself
/// names, because the lease can be held under any thread in the tree, not only the root. An
/// operation that parked more than once, under more than one such key over its life, collects every
/// group this way and is re-sorted newest first across all of them, so the merge cannot leave an
/// older park from one holder key ahead of a newer one from another.
fn parked_work_for(
    store: &ChatStore,
    id_list: &[&str],
    root_of: &BTreeMap<&str, &str>,
) -> Result<BTreeMap<String, Vec<crate::park::ParkedWork>>> {
    if !store.any_parked_work()? {
        return Ok(BTreeMap::new());
    }
    let by_scope = store.all_parked_work()?;
    let mut by_root: BTreeMap<String, Vec<crate::park::ParkedWork>> = BTreeMap::new();
    for id in id_list {
        let Some(rows) = by_scope.get(&format!("thread:{id}")) else {
            continue;
        };
        let Some(root) = root_of.get(id) else {
            continue;
        };
        by_root
            .entry((*root).to_string())
            .or_default()
            .extend(rows.iter().cloned());
    }
    for rows in by_root.values_mut() {
        rows.sort_by_key(|r| std::cmp::Reverse(r.id));
    }
    Ok(by_root)
}

/// **The refused parks of this report's operations, attributed by ROOT** (nxf 6j6v.8bv9) —
/// [`parked_work_for`]'s rule, for [`crate::store::ChatStore::all_park_refusals`]'s rows: a
/// `"thread:<id>"` key belongs to the operation whose tree holds `<id>`.
///
/// A device has one claim at a time and the store forgets a refusal when its claim ends, so an
/// operation carries at most one of these in practice. Should two ever meet on one root — a row a
/// crash left between the acquire's swap and its clean-up — the most recently TRIED wins: it is the
/// one describing the claim that is still being retried.
fn park_refusals_for(
    store: &ChatStore,
    id_list: &[&str],
    root_of: &BTreeMap<&str, &str>,
) -> Result<BTreeMap<String, crate::park::ParkRefusalNote>> {
    let by_scope = store.all_park_refusals()?;
    let mut by_root: BTreeMap<String, crate::park::ParkRefusalNote> = BTreeMap::new();
    if by_scope.is_empty() {
        return Ok(by_root);
    }
    for id in id_list {
        let Some(note) = by_scope.get(&format!("thread:{id}")) else {
            continue;
        };
        let Some(root) = root_of.get(id) else {
            continue;
        };
        let newer = by_root
            .get(*root)
            .is_none_or(|kept| note.last_tried > kept.last_tried);
        if newer {
            by_root.insert((*root).to_string(), note.clone());
        }
    }
    Ok(by_root)
}

/// **This operation's `withdrawn_holders` row, if any** (nxf 6j6v.b9nf, Integrity #2 of this item's
/// review) — [`park_refusals_for`]'s shape: attributed to the root exactly as a refused park is,
/// because both tables key on the same claim scope and a device holds at most one live claim at a
/// time (the tie-break below is the same defensive "most recent wins" for the crash window that
/// leaves two). `pinned_by` is read only for the row that actually wins attribution to one of
/// `id_list`'s operations, so this pays a worker call per session of THAT claim and nothing at all
/// for every operation whose round was never withdrawn.
fn withdrawn_holders_for(
    store: &ChatStore,
    worker: &dyn crate::worker::Worker,
    id_list: &[&str],
    root_of: &BTreeMap<&str, &str>,
) -> Result<BTreeMap<String, WithdrawnHolderStatus>> {
    let by_scope = store.all_withdrawn_holders()?;
    let mut by_root: BTreeMap<String, WithdrawnHolderStatus> = BTreeMap::new();
    if by_scope.is_empty() {
        return Ok(by_root);
    }
    for id in id_list {
        let key = format!("thread:{id}");
        let Some(marker) = by_scope.get(&key) else {
            continue;
        };
        let Some(root) = root_of.get(id) else {
            continue;
        };
        let newer = by_root
            .get(*root)
            .is_none_or(|kept| marker.withdrawn_at > kept.withdrawn_at);
        if newer {
            let pinned_by = pinning_sessions(store, worker, &key)?;
            by_root.insert(
                (*root).to_string(),
                WithdrawnHolderStatus {
                    withdrawn_at: marker.withdrawn_at.clone(),
                    by: marker.by.clone(),
                    pinned_by,
                },
            );
        }
    }
    Ok(by_root)
}

/// **Every session under `scope_key`'s claim area that is STILL a live process, right now** — the
/// same two reads [`crate::orchestration::sessions_still_running`] makes (every session the store
/// has no end for, then the worker), duplicated rather than shared because that function is private
/// to the compute layer and this is a REPORT, not a decision: nothing here acts on the answer, so
/// asking it fresh at read time is what keeps `nxc status` honest about a fact that changes
/// underneath it every liveness cadence.
fn pinning_sessions(
    store: &ChatStore,
    worker: &dyn crate::worker::Worker,
    scope_key: &str,
) -> Result<Vec<String>> {
    let Some(scope) = crate::working_tree::WorkScope::parse(scope_key) else {
        return Ok(Vec::new());
    };
    let mut pinned = Vec::new();
    for thread in store.work_scope_threads(&scope)? {
        for session in store.unended_sessions_in_thread(&thread)? {
            if worker.session_is_running(&session) {
                pinned.push(session);
            }
        }
    }
    Ok(pinned)
}

fn session_states_for(
    store: &ChatStore,
    worker: &dyn crate::worker::Worker,
    sessions: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, crate::orchestration::SessionState>> {
    use crate::orchestration::SessionState;
    let distinct: BTreeSet<&str> = sessions.values().map(String::as_str).collect();
    if distinct.is_empty() {
        return Ok(BTreeMap::new());
    }
    let ids: Vec<&str> = distinct.iter().copied().collect();
    let ended = match ids.len() > MAX_BOUND_IDS {
        true => store.ended_sessions()?,
        false => store.ended_sessions_among(&ids)?,
    };
    // Exactly the ids the worker will be asked about: not already ended, and shaped like something
    // this device could have minted. Past the ceiling the set is EMPTY rather than truncated — a
    // partial answer whose membership depends on where the cut fell is worse than a whole one that
    // says nobody looked.
    let probe: BTreeSet<&str> = match ids.len() > MAX_LIVENESS_PROBES {
        true => BTreeSet::new(),
        false => ids
            .iter()
            .copied()
            .filter(|id| !ended.contains(*id) && is_mintable_session_id(id))
            .collect(),
    };
    Ok(ids
        .into_iter()
        .map(|id| {
            let state = if ended.contains(id) {
                SessionState::Ended
            } else if probe.contains(id) && worker.session_is_running(id) {
                SessionState::Running
            } else {
                SessionState::Unknown
            };
            (id.to_string(), state)
        })
        .collect())
}

/// How many thread ids a bulk quorum read will bind into one query before falling back to the
/// workspace-wide read and filtering in memory. SQLite's default `SQLITE_MAX_VARIABLE_NUMBER` is
/// 999, and an operation is normally a handful of threads — this is the guard for the pathological
/// one, not a tuning knob.
///
/// `pub(crate)` since the branch review of PR #337 (Integrity #5), so
/// [`ChatStore::work_scope_has_outstanding`](crate::store::ChatStore::work_scope_has_outstanding)
/// can honour the SAME ceiling rather than a second number three files away that drifts from this
/// one.
///
/// **Who else binds an id list into [`ChatStore::thread_quorums`](
/// crate::store::ChatStore::thread_quorums), derived rather than assumed** (`grep -rn
/// '\.thread_quorums('` over `crates/`, non-test call sites) — because an earlier draft of this
/// paragraph claimed "exactly two" and that was wrong, which is the failure class
/// `handover-lists-are-themselves-unverified` names:
///
/// - [`quorums_for`] and `work_scope_has_outstanding` — the two that apply this constant.
/// - [`threads`] — ids from `member_thread_ids`, i.e. every thread of every channel the caller is
///   in. **Unguarded, and it predates this branch.** Not widened here: it is a different path with
///   a different failure mode (a listing that errors, loudly, in front of whoever typed it) and
///   fixing it is a change to `nxc threads list`, not to a lease.
/// - `orchestration::supervised_member_threads` — one id per DECLARED channel member, so bounded by
///   a declaration rather than by a caller. **Unguarded**, and deliberately left so here: it is on
///   the synchronous reply write path, where swapping a scoped read for a workspace-wide one to
///   protect against a 500-member channel would be a certain cost for an unlikely shape. Worth its
///   own decision, not a silent widening of this one.
/// - [`thread_quorums`] (the facade passthrough) — explicitly the caller's responsibility
///   (`ChatStore::thread_quorums`' own doc says so). `requester_of` used to be named here as the
///   other one-id case and no longer binds any: it reads `ChatStore::thread_opener` on the primary
///   key (nxf 6j6v.yr59).
pub(crate) const MAX_BOUND_IDS: usize = 500;

/// Quorums for exactly `ids`, or for the whole workspace when there are too many to bind (see
/// [`MAX_BOUND_IDS`]). `total` is how many threads the workspace has, so the degenerate
/// "the scope IS the workspace" case skips the placeholder list entirely.
fn quorums_for(
    store: &ChatStore,
    now: &str,
    ids: &[&str],
    total: usize,
) -> Result<Vec<ThreadQuorum>> {
    if ids.len() > MAX_BOUND_IDS || ids.len() == total {
        return store.thread_quorums_all(now);
    }
    store.thread_quorums(ids, now)
}

/// [`quorums_for`]'s twin for the thread → assignee-session edge (nxf 6j6v.1q6d).
fn assignee_sessions_for(
    store: &ChatStore,
    ids: &[&str],
    total: usize,
) -> Result<BTreeMap<String, String>> {
    let rows = if ids.len() > MAX_BOUND_IDS || ids.len() == total {
        store.thread_assignee_sessions()?
    } else {
        store.thread_assignee_sessions_for(ids)?
    };
    Ok(rows.into_iter().collect())
}

/// [`quorums_for`]'s twin for "did this thread's parent move on after it was discharged?" (nxf
/// 6j6v.93zd) — the discriminator [`ThreadState::Orphaned`] applies. Same ceiling, same fallback.
fn moved_on_for(store: &ChatStore, ids: &[&str], total: usize) -> Result<BTreeSet<String>> {
    let rows = if ids.len() > MAX_BOUND_IDS || ids.len() == total {
        store.parent_moved_after_discharge()?
    } else {
        store.parent_moved_after_discharge_for(ids)?
    };
    Ok(rows.into_iter().collect())
}

/// [`quorums_for`]'s twin for the two facts a thread's newest reply of the current turn carries:
/// "did the agent side hand this task back?" (nxf 6j6v.mqad) and "did the RUNTIME write this reply
/// instead of the agent?" (nxf 6j6v.kffm). Same ceiling, same fallback as its three siblings above.
///
/// **Two sets out of ONE read**, because both are properties of the same message and the store
/// answers them on the same row — see
/// [`LastReply`](crate::store::LastReply) for why they are orthogonal and
/// [`ChatStore::last_reply_kind`](crate::store::ChatStore::last_reply_kind) for why "the last reply"
/// is derived exactly once in this crate.
///
/// The comparison against [`crate::model::KIND_ESCALATION`] happens HERE rather than in the store,
/// which answers with the raw kind LABEL for the same reason.
fn last_replies_for(store: &ChatStore, ids: &[&str], total: usize) -> Result<LastReplyMarks> {
    let rows = if ids.len() > MAX_BOUND_IDS || ids.len() == total {
        store.last_reply_kinds_all()?
    } else {
        store.last_reply_kinds(ids)?
    };
    let mut marks = LastReplyMarks::default();
    for row in rows {
        if row.kind == crate::model::KIND_ESCALATION {
            marks.escalated.insert(row.thread_id.clone());
        }
        if row.substituted {
            marks.substituted.insert(row.thread_id);
        }
    }
    Ok(marks)
}

/// [`quorums_for`]'s twin for "where did the working copy stand at this thread's newest handover?"
/// (nxf 6j6v.2af2). Same ceiling, same fallback as its three siblings above.
fn anchors_for(
    store: &ChatStore,
    ids: &[&str],
    total: usize,
) -> Result<BTreeMap<String, crate::anchor::Anchor>> {
    let rows = if ids.len() > MAX_BOUND_IDS || ids.len() == total {
        store.working_copy_anchors_all()?
    } else {
        store.working_copy_anchors(ids)?
    };
    Ok(rows.into_iter().map(|r| (r.thread_id, r.anchor)).collect())
}

/// **What this turn's newest reply SAID, per thread** — the two orthogonal marks [`last_replies_for`]
/// reads off one row, kept together because they are answered together and consumed together.
#[derive(Default)]
struct LastReplyMarks {
    /// Threads whose newest reply of the current turn carries the escalation marker.
    escalated: BTreeSet<String>,
    /// Threads whose newest reply of the current turn was written by the RUNTIME standing in for the
    /// agent that owed it — [`crate::model::Refs::substituted`].
    substituted: BTreeSet<String>,
}

/// The operation forest, built from the parent edges ALONE — no quorum, no message, no clock.
///
/// Two parent maps, deliberately: `declared` is what the `open` op actually says, `effective` is
/// what the GROUPING uses (`declared`, minus a self-edge and minus a parent this workspace has not
/// folded). Everything that answers "which operation is this in" reads `effective`; everything that
/// answers "what does this thread claim" reads `declared`. Collapsing them is what made a
/// provisionally-rooted thread render as a human's own (see [`StatusThread::awaiting_human`]).
struct Forest<'a> {
    declared: BTreeMap<&'a str, Option<&'a str>>,
    channel: BTreeMap<&'a str, Option<&'a str>>,
    children: BTreeMap<&'a str, Vec<&'a str>>,
    root_of: BTreeMap<&'a str, &'a str>,
    roots: BTreeSet<&'a str>,
}

impl<'a> Forest<'a> {
    fn build(edges: &'a [(String, Option<String>, Option<String>)]) -> Forest<'a> {
        let known: BTreeSet<&str> = edges.iter().map(|(id, _, _)| id.as_str()).collect();
        let declared: BTreeMap<&str, Option<&str>> = edges
            .iter()
            .map(|(id, parent, _)| (id.as_str(), parent.as_deref()))
            .collect();
        let channel: BTreeMap<&str, Option<&str>> = edges
            .iter()
            .map(|(id, _, ch)| (id.as_str(), ch.as_deref()))
            .collect();
        let effective: BTreeMap<&str, Option<&str>> = declared
            .iter()
            .map(|(id, parent)| (*id, parent.filter(|p| known.contains(p) && p != id)))
            .collect();
        let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (id, parent) in &effective {
            if let Some(p) = parent {
                children.entry(p).or_default().push(id);
            }
        }
        let mut roots: BTreeSet<&str> = BTreeSet::new();
        let mut root_of: BTreeMap<&str, &str> = BTreeMap::new();
        for id in effective.keys() {
            let root = root_of_thread(&effective, id);
            roots.insert(root);
            root_of.insert(id, root);
        }
        Forest {
            declared,
            channel,
            children,
            root_of,
            roots,
        }
    }

    /// `id` and everything under it, depth-first, siblings by thread id.
    ///
    /// `seen` is the cycle guard [`root_of_thread`] carries at the other end of the walk. A thread
    /// is yielded once.
    ///
    /// **Iterative, like both its siblings in this file** (branch review of PR #337, Integrity #6).
    /// It used to recurse, and the cycle guard alone does not bound the depth: it bounds the number
    /// of NODES, so a legitimately deep chain — this epic's threads hang under each other one level
    /// per hand-off — still descends once per level and can exhaust the stack, on a read path
    /// (`nxc status`) whose whole job is to be safe to run against a workspace in any state. An
    /// explicit stack costs one `Vec` and removes the bound question entirely, which is why it is
    /// preferred here to the loop-count guard [`Self::depth_of`] carries (that one is COUNTING, so
    /// it needs a number to stop at; this one only needs to finish, so it needs none).
    ///
    /// Children are pushed in reverse so they pop in thread-id order — the same pre-order the
    /// recursion produced, and `nxc status`'s rendered order.
    fn collect_ids(&self, id: &'a str, seen: &mut BTreeSet<&'a str>, out: &mut Vec<&'a str>) {
        let mut stack = vec![id];
        while let Some(next) = stack.pop() {
            if !seen.insert(next) {
                continue;
            }
            out.push(next);
            let kids = self.children.get(next).map(Vec::as_slice).unwrap_or(&[]);
            stack.extend(kids.iter().rev().copied());
        }
    }

    /// `id`'s depth below its operation's root, walking the EFFECTIVE edges (the grouping ones).
    ///
    /// **Cost, named rather than left to be discovered** (branch review of PR #337, Code #3):
    /// O(depth) per call, called once per thread from [`Self::status_thread`], so O(threads × depth)
    /// over a status read — quadratic in the pathological case of one chain N threads long. Left as
    /// it is, deliberately: [`Self::build`] already pays the same shape (`root_of_thread` per node),
    /// so memoizing here alone would move the number without changing the bound. At the documented
    /// scale it does not bite: an operation is a handful of threads deep, and `depth` is what a
    /// status read INDENTS by — a tree deep enough for this to cost anything is one no reader could
    /// use. If a depth map ever becomes worth building, it belongs in `build` beside `root_of`,
    /// computed for both in one pass, not bolted on here.
    fn depth_of(&self, id: &str) -> usize {
        let root = self.root_of.get(id).copied();
        let mut depth = 0;
        let mut cur = id;
        let mut guard = self.declared.len() + 1;
        while Some(cur) != root && guard > 0 {
            match self.declared.get(cur).copied().flatten() {
                Some(p) if self.declared.contains_key(p) && p != cur => {
                    cur = p;
                    depth += 1;
                }
                _ => break,
            }
            guard -= 1;
        }
        depth
    }

    /// One thread's derived row, or `None` for a thread with no quorum row at all — unreachable by
    /// construction (`thread_edges` and the quorum read both project every `threads` row, and the
    /// `LEFT JOIN json_each` keeps an expectation-less thread), and a skip rather than a panic
    /// because the alternative to being wrong about a thread is not being loud about the whole tree.
    fn status_thread(&self, id: &str, facts: &DerivedFacts<'_>) -> Option<StatusThread> {
        let DerivedFacts {
            by_id,
            sessions,
            session_states,
            moved_on,
            last_replies,
            working_tree,
            anchors,
            interruptions,
        } = facts;
        let q = by_id.get(id)?;
        let parent = self.declared.get(id).copied().flatten();
        let kids = self.children.get(id).map(Vec::as_slice).unwrap_or(&[]);
        // The three states (a71h's DoD), derived here and nowhere else. `parent` is the DECLARED
        // edge: a thread that names a parent is not a root, whether or not this workspace has folded
        // that parent yet.
        let discharged = q.outstanding.is_empty();
        let is_root = parent.is_none();
        // **A parent that ASKED and was answered**, not merely one that owes nothing (nxf
        // 6j6v.93zd). `outstanding.is_empty()` alone is equally true of a parent that never asked
        // anyone anything — a plain conversation, a substrate-channel post — and a dead end below
        // such a parent is not a coordination that broke down, it is a thread nobody was ever
        // waiting on. Requiring a DECLARED expectation is what tells the two apart, and it errs the
        // same way the rest of this rule does: it removes orphans.
        let parent_asked_and_was_answered = parent
            .and_then(|p| by_id.get(p))
            .is_some_and(|p| !p.expects.is_empty() && p.outstanding.is_empty());
        let state = if !discharged {
            ThreadState::Open
        } else if !is_root
            && kids.is_empty()
            && parent_asked_and_was_answered
            && !moved_on.contains(id)
        {
            ThreadState::Orphaned
        } else {
            ThreadState::Answered
        };
        let (working_tree, working_tree_queue_position) = working_tree_fields(working_tree, id);
        Some(StatusThread {
            thread_id: q.thread_id.clone(),
            name: q.name.clone(),
            channel_id: q.channel_id.clone(),
            parent: parent.map(str::to_string),
            depth: self.depth_of(id),
            state,
            awaiting_human: is_root && q.complete,
            escalated: last_replies.escalated.contains(id),
            substituted: last_replies.substituted.contains(id),
            opener: q.opener.clone(),
            session: sessions.get(id).cloned(),
            // Keyed by SESSION, not by thread, so the two fields cannot disagree: the state
            // reported is the state of the id printed beside it, and a thread with no session has
            // no state rather than a default one.
            session_state: sessions
                .get(id)
                .and_then(|sid| session_states.get(sid))
                .copied(),
            expects: q.expects.clone(),
            outstanding: q.outstanding.clone(),
            stale: q.stale,
            deadline: q.deadline.clone(),
            working_tree,
            working_tree_queue_position,
            // **Derived from the rows this report already holds** (nxf 6j6v.hw2t): the children are
            // the forest's own edges and their quorums are in the same bulk read as this thread's,
            // so the field costs no statement at all — the property `tests/bulk_quorum.rs`
            // measures. A child of another operation cannot appear here: `kids` are this forest's.
            waiting_on_sub_round: crate::awaiting::own_open_sub_round(
                q,
                kids.iter().filter_map(|kid| by_id.get(kid).copied()),
            ),
            working_copy: anchors.get(id).cloned(),
            // Keyed by SESSION for `session_state`'s reason, and beside it: the hold reported is a
            // hold on the id printed on this row, so the two facts about one session cannot
            // disagree.
            interrupted: sessions
                .get(id)
                .and_then(|sid| interruptions.get(sid))
                .cloned(),
        })
    }
}

/// Walk up to `id`'s root over the EFFECTIVE edges.
///
/// Terminates on a cycle by taking the smallest id **of the cycle**, not of everything visited: a
/// node with a tail into a cycle (`X → A → B → A`) must land in the same operation as the cycle's
/// own members, and `min(everything seen)` would answer `X` from `X` and `A` from `A`, emitting `X`
/// twice. The cycle is the suffix of the walked path from the revisited node onwards.
fn root_of_thread<'a>(effective: &BTreeMap<&'a str, Option<&'a str>>, id: &'a str) -> &'a str {
    let mut path: Vec<&str> = Vec::new();
    let mut cur = id;
    loop {
        if let Some(at) = path.iter().position(|p| *p == cur) {
            return path[at..].iter().copied().min().unwrap_or(cur);
        }
        path.push(cur);
        match effective.get(cur).copied().flatten() {
            Some(p) => cur = p,
            None => return cur,
        }
    }
}

/// **THE parent rule (nxf 6j6v.a71h §3.1), in one place.** Which thread a thread minted by this
/// caller right now hangs under: the one the caller's own session is currently working in.
///
/// > Der Elternteil ist der Faden, AUS DEM HERAUS gesendet wird — nicht der, in dessen Auftrag
/// > gehandelt wird.
///
/// "Currently working in" is [`ChatStore::record_session_thread`]'s value — the thread the session
/// was last put in motion on — and that is what makes the two halves of the rule come apart the way
/// the item's own chain requires: the `#coding` supervisor acts on BEHALF of its channel thread T2,
/// but it is reading the coder's answer in T3 when it sends to `#review`, so T4 hangs under T3.
///
/// `None` — no ambient session, or a session nothing has ever put in motion — is a ROOT. That is the
/// top end of a chain by construction: a human at a terminal has no session at all, which is exactly
/// §3.4's "the human stands at the ENDS of the chain, never in its middle".
///
/// A read failure resolves to `None` rather than propagating, deliberately: this rides along with
/// every thread that is opened, and the fail-safe direction is one operation too many in the tree,
/// never a chain spliced onto a stranger's.
pub fn parent_thread_of(store: &ChatStore, session: Option<&str>) -> Option<String> {
    session.and_then(|s| store.session_thread(s).ok().flatten())
}

/// Open a review board in one atomic step: emit, in order, `thread open` (root) → the request
/// `message` (thread_id set) → `thread set expects_reply_from` → (if a deadline)
/// `thread set deadline`. **The message goes before the obligation, and that order is load-bearing**
/// — see [`ask_under`], where it is argued (nxf 6j6v.pn0s). Rejection is the CLI contract,
/// verbatim: unknown channel → `not_found`; a
/// `--deadline` that is neither RFC3339 nor a `<n><unit>` duration → `validation`. Both are checked
/// BEFORE any op is emitted, so a rejected `ask` leaves no half-open thread. The `--deadline` is
/// resolved to an ABSOLUTE instant against `now` (TB-M2-6) so `stale` needs no per-reader arithmetic.
///
/// Opens a ROOT thread — no tree edge (nxf 6j6v.a71h §3.1). A caller that knows which thread it is
/// standing in uses [`ask_under`], which this delegates to; everything else about the verb is there.
pub fn ask(store: &mut ChatStore, req: AskRequest) -> Result<AskReceipt> {
    ask_under(store, req, None)
}

/// [`ask`] with the board thread hung under `parent` (nxf 6j6v.a71h §3.1) — **the whole verb lives
/// here**; `ask` is the no-edge wrapper. See it for what the verb does and how it rejects.
///
/// The tree edge is passed BESIDE the request rather than added to it.
///
/// A field on [`AskRequest`] was the obvious shape and is the wrong one, for the reason
/// `orchestration::channel_open_in` already states about `ChannelOpenRequest`: this struct is
/// app-facing and constructed by literal on both seams, so a new field breaks every existing caller
/// for a value most of them have no opinion about. `ask` keeps its signature and its meaning ("open
/// a board here"), and the callers that know where they are standing say so.
pub fn ask_under(
    store: &mut ChatStore,
    mut req: AskRequest,
    parent: Option<&str>,
) -> Result<AskReceipt> {
    req.actor = nxs_foundation::model::validate_author(req.actor)?;
    if !store.channel_exists(req.channel)? {
        return Err(NxfError::not_found(format!(
            "no such channel: {}",
            req.channel
        )));
    }
    // Resolve the deadline (may reject) BEFORE writing — atomic receipt, no half-open thread.
    let deadline = match req.deadline {
        Some(when) => Some(resolve_deadline(req.now, when)?),
        None => None,
    };
    let author = format!("{}/{}", req.origin, req.actor);
    store.set_wall_clock(req.now);
    let thread_id = store.mint_thread_id();
    store.open_thread(
        &thread_id,
        &ThreadRoot {
            origin: req.origin.to_string(),
            channel_id: req.channel.to_string(),
            opener: author.clone(),
            created: req.now.to_string(),
            parent: parent.map(str::to_string),
        },
        &author,
    );
    // **THE ASK IS POSTED BEFORE THE OBLIGATION IS DECLARED, AND THE ORDER IS THE WHOLE POINT**
    // (nxf 6j6v.pn0s).
    //
    // `replied` counts a message from an expected handle written AFTER the register that expects it
    // ([`after_the_declaration`](crate::store) — the turn watermark of nxf 6j6v.cg8g). Declaring
    // first therefore made this opening message count as its own answer whenever the OPENER is also
    // the expected party — and there is one such thread, opened on every run: a flow step whose
    // target is a declared CHANNEL. `orchestration::open_declared_channel_and_fan_out` opens it AS
    // the supervisor and declares the supervisor as the one who owes the answer, so it read
    // `complete: true` from the instant it was minted, with none of its members having said
    // anything.
    //
    // What that cost, measured in `watch-bundestag` on 2026-09-08: `member_set_is_settled` reads
    // `complete || stale`, so the outer flow's set counted that step as settled and a single
    // `nxc tick` released the step AFTER it — the finisher ran beside the review round it was
    // supposed to wait for, and shipped without the verdict. `nxc status` showed the review branch
    // as `answered` while its three members were still working, and a member that had answered read
    // `ORPHANED`, because that state asks whether the PARENT was asked and answered.
    //
    // Posting first fixes all of it at the source and says something true besides: **the opening
    // message is the question, and a question is never the answer to itself.** For every other
    // thread — where the opener is not among the expected — nothing observable moves: the predicate
    // only ever looked at messages FROM an expected handle.
    let message_id = store.post_message(&MessageEnvelope {
        origin: req.origin.to_string(),
        channel_id: req.channel.to_string(),
        sender: author.clone(),
        kind: req.kind,
        priority: req.priority,
        disposition: Disposition::InTurn,
        thread_id: Some(thread_id.clone()),
        refs: req.refs,
        body: req.body.to_string(),
    });
    let expects_json = serde_json::to_string(req.expect).expect("expects serialize");
    store.set_expects_reply_from(&thread_id, &expects_json, &author);
    if let Some(d) = &deadline {
        store.set_deadline(&thread_id, d, &author);
    }
    Ok(AskReceipt {
        thread_id,
        message_id,
        persisted: true,
        expects: req.expect.to_vec(),
        deadline,
    })
}

/// Re-declare a thread's `expects_reply_from` (§3.2, an LWW replace) — **the seam write that opens a
/// role's next TURN**, and the only entrance to it. OPENER-ONLY (mirrors M1's exactly-2-DM CLI
/// check): an unknown thread → `not_found`, a non-opener caller → `forbidden`; the store op stays
/// pure. The set is REPLACED whole. Returns the RE-DERIVED [`ThreadQuorum`] so the caller sees the
/// board as it now stands. `now` drives only `stale`.
///
/// **What this call means changed with nxf 6j6v.cg8g, and it is not a nuance.** Under the old
/// per-thread `replied` it was the in-core UNSTICK path: narrowing the set to the handles that had
/// already answered flipped `complete` false→true. The discharge is now measured from the moment the
/// obligation was declared, so this write is a QUESTION — everyone the new set names owes an answer
/// to *this* declaration, whether or not they answered an earlier one. That is what makes it the
/// channel supervisor's instrument (nxf 6j6v.hq71/6j6v.pf6j): the supervisor re-declares to hand a
/// role its next turn, and `reply --if-unanswered` then discharges exactly that turn. To close a
/// board nobody will answer, pass a set that names nobody outstanding rather than expecting a
/// narrowing to complete it.
///
/// **There is no CLI verb behind this any more.** `nxc threads expect` was removed by nxf 6j6v.dvyq
/// §3 (owner, 2026-08-13: *"sehe ich nicht, wofür das notwendig ist"*) — a human-typed re-declare is
/// not part of the agent-to-agent surface. The capability did not move: it is here, `pub`, on the
/// compute layer an embedding app links against, and it is what the engine's own supervisor calls.
pub fn set_expects(
    store: &mut ChatStore,
    now: &str,
    actor_handle: &str,
    thread_id: &str,
    expects: &[String],
) -> Result<ThreadQuorum> {
    let q = store
        .thread_quorum(thread_id, now)?
        .ok_or_else(|| NxfError::not_found(format!("no such thread: {thread_id}")))?;
    match q.opener.as_deref() {
        Some(opener) if opener == actor_handle => {}
        _ => {
            return Err(NxfError::forbidden(format!(
                "only the opener may re-declare who thread {thread_id} expects a reply from"
            )))
        }
    }
    // AFTER the opener check, deliberately: a caller with no standing gets the specific
    // `forbidden` ("only the opener may re-declare"), not a generic complaint about its handle —
    // and a thread whose stored opener is itself blank would otherwise MATCH a blank handle and
    // author the op unattributed. This is the last gate before the write (Code Quality #1, PR #311).
    let actor_handle = nxs_foundation::model::validate_author(actor_handle)?;
    let expects_json = serde_json::to_string(expects).expect("expects serialize");
    store.set_wall_clock(now);
    store.set_expects_reply_from(thread_id, &expects_json, actor_handle);
    store
        .thread_quorum(thread_id, now)?
        .ok_or_else(|| NxfError::io(format!("thread {thread_id} vanished after re-declare")))
}

/// **What came of an attempt to name a thread** (nxf 6j6v.e76c) — declared field order = the JSON
/// contract, as everywhere else an app reads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct NameReceipt {
    pub thread_id: String,
    /// The name the thread carries NOW — the one this call wrote, or the one it already had.
    /// `None` only when nothing could be derived and nothing was there before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// **Whether THIS call wrote it.** `false` for the two no-ops that are not failures: the thread
    /// was already named (the once-only rule), or nothing usable was offered.
    pub named: bool,
}

/// **Give a thread its display name, ONCE** (nxf 6j6v.e76c) — the one write behind the register,
/// and the place the once-only rule is enforced rather than remembered.
///
/// Three properties, and each one is the item's:
///
/// 1. **Once.** A thread that already carries a name is left alone and reported as `named: false`.
///    A no-op rather than an error, deliberately: the caller is a commissioned run that can be
///    retried (a second commission, a re-run by hand), and a retry that ERRORS turns an idempotent
///    convenience into something a caller has to reason about. The rule it protects is that a name
///    must not move under a reader.
/// 2. **Short.** Whatever is offered goes through [`crate::naming::sanitize`], so no caller — the
///    naming model, a host, a person at a terminal — can put a paragraph where a list expects a
///    label. Nothing usable is `named: false`, not a stored empty string.
/// 3. **Not gated on the opener**, unlike [`set_expects`] one function up, and the difference is
///    stated here because it looks like an oversight. `expects_reply_from` is an AUTHORIZATION fact
///    — it decides who owes work and what releases a working copy — so only the opener may move it.
///    A name decides nothing: [`crate::model::FIELD_NAME`] is a display string nothing joins,
///    routes or looks up on, and the write is once-only, so the whole of what a foreign caller
///    could do is name an unnamed conversation. The OPENER is a narrower seat than this write
///    needs, and it is not the seat the naming run sits in: the run is commissioned by whoever sent
///    the message, which on a channel is the supervisor and not the thread's opener.
///
///    What that leaves ungated is the WRITE, not the READ. The one caller that reads a thread's
///    content to derive a name — [`crate::surface::name_thread_from_its_opening_message`] — takes
///    [`require_thread_readable`] first, and the commissioned run carries the commissioning
///    caller's own identity so it can pass it ([`crate::naming::ModelNamer::commission`] stamps
///    `NXC_ORIGIN`/`NXC_ACTOR`, as `SidecarWorker::trigger` does for a spawned session).
///
/// An unknown thread is `not_found`: naming something that does not exist is a caller mistake, and
/// the alternative would silently create a register on a thread id nobody minted.
pub fn name_thread(
    store: &mut ChatStore,
    now: &str,
    actor_handle: &str,
    thread_id: &str,
    name: &str,
) -> Result<NameReceipt> {
    if store.thread_quorum(thread_id, now)?.is_none() {
        return Err(NxfError::not_found(format!("no such thread: {thread_id}")));
    }
    if let Some(existing) = store.thread_name(thread_id)? {
        return Ok(NameReceipt {
            thread_id: thread_id.to_string(),
            name: Some(existing),
            named: false,
        });
    }
    let Some(name) = crate::naming::sanitize(name) else {
        return Ok(NameReceipt {
            thread_id: thread_id.to_string(),
            name: None,
            named: false,
        });
    };
    let actor_handle = nxs_foundation::model::validate_author(actor_handle)?;
    store.set_wall_clock(now);
    store.set_thread_name(thread_id, &name, actor_handle);
    Ok(NameReceipt {
        thread_id: thread_id.to_string(),
        name: Some(name),
        named: true,
    })
}

/// Resolve a deadline argument to an absolute, **UTC-normalized** RFC3339 instant against `now`
/// (TB-M2-6). An argument that already parses as RFC3339 is taken as-is; otherwise it is parsed as a
/// `<n><unit>` duration and added to `now`. A value that is neither — or one that overflows the
/// representable date range — is a `validation` error (never a panic).
///
/// **The label it names in that error is `deadline` and no longer `--deadline`** (nxf 6j6v.ckeq
/// removed that flag). This is the anonymous entry point, used where a caller hands
/// [`AskRequest::deadline`] a value it resolved elsewhere; the two paths that read an AUTHOR's own
/// string — a channel's declared `timeout:` — call [`resolve_deadline_spec`] directly with
/// `"the channel's declared `timeout`"`, so the message a real typo produces names the field the
/// author actually wrote.
///
/// The result is normalized to UTC (`Z`) so the `stale` derivation's lexicographic string comparison
/// against the reader's (UTC) clock is SOUND: an offset-form instant stored verbatim mis-sorts (a
/// negative-zone FUTURE instant sorts as PAST, a false `stale`). Determinism is preserved — the
/// stored value is a single canonical absolute instant, no per-reader arithmetic.
fn resolve_deadline(now: &str, when: &str) -> Result<String> {
    resolve_instant(now, when, "deadline")
}

/// **What a deadline argument MEANS, decided in one place** (nxf 6j6v.nf38) — the two things every
/// caller of this grammar can need out of one string, resolved by one parse.
///
/// The distinction is the argument's own FORM, and it is the whole rule:
///
/// * a `<n><unit>` DURATION (`20m`) declares a WINDOW — "this long", measured from somewhere. Its
///   `at` is `now + window` and its `window` is that duration, which is what makes a member's clock
///   RESETTABLE: something that resets it has a length to reset it BY.
/// * an ABSOLUTE RFC3339 instant declares a point in time — "by 15:00". There is no window, nothing
///   to reset by, and nothing that should move it: a requester who named an instant meant that
///   instant. `window` is `None` and such a deadline stays exactly as stubborn as it has always been.
pub(crate) struct DeadlineSpec {
    /// The absolute, UTC-normalized RFC3339 instant this argument first falls due at.
    pub at: String,
    /// The idle window the argument declared, or `None` when it named an absolute instant.
    pub window: Option<time::Duration>,
}

/// [`resolve_deadline`] with the ARGUMENT NAME its error messages use made explicit, so a second
/// caller with the same grammar and a different flag reports its own (nxf 6j6v.nf38: a channel's
/// declared `timeout`). Factored out rather than copied — one grammar, one parser, one set of
/// overflow guards; only the label differs.
pub(crate) fn resolve_instant(now: &str, when: &str, arg: &str) -> Result<String> {
    Ok(resolve_deadline_spec(now, when, arg)?.at)
}

/// The ONE parse of a deadline argument — see [`DeadlineSpec`] for the rule it applies.
///
/// **It returns a `Result`, and that is load-bearing**: a value this grammar cannot read is a
/// `validation` error naming `arg` and the value, never a silent "no window". "No cap declared" and
/// "an unreadable cap" must not be the same answer, or a typo'd `timeout` quietly means "wait
/// forever" and the failure is one nobody ever notices.
pub(crate) fn resolve_deadline_spec(now: &str, when: &str, arg: &str) -> Result<DeadlineSpec> {
    match OffsetDateTime::parse(when, &Rfc3339) {
        Ok(dt) => Ok(DeadlineSpec {
            at: to_utc_rfc3339(dt)?,
            window: None,
        }),
        Err(_) => {
            let dur = parse_duration(when, arg)?;
            Ok(DeadlineSpec {
                at: instant_after(now, dur, &format!("{arg} `{when}`"))?,
                window: Some(dur),
            })
        }
    }
}

/// `now + add`, as an absolute UTC-normalized RFC3339 instant — the one place a duration becomes a
/// deadline, so every stored deadline sorts against every reader's clock the same way
/// ([`resolve_instant`]'s doc has the argument for the normalization; it is load-bearing, not
/// cosmetic). `what` names the thing being resolved for the overflow message, already rendered by
/// the caller, because the two callers describe it differently (a flag and its value; a channel's
/// declared window).
///
/// Checked add, so a huge duration is a `validation` error and never a panic on date overflow.
pub(crate) fn instant_after(now: &str, add: time::Duration, what: &str) -> Result<String> {
    let base = OffsetDateTime::parse(now, &Rfc3339)
        .map_err(|e| NxfError::validation(format!("cannot parse now `{now}` as RFC3339: {e}")))?;
    let at = base.checked_add(add).ok_or_else(|| {
        NxfError::validation(format!("{what} resolves past the representable date range"))
    })?;
    to_utc_rfc3339(at)
}

/// UTC-normalize and render one instant in the single canonical form every stored deadline uses.
fn to_utc_rfc3339(instant: OffsetDateTime) -> Result<String> {
    instant
        .to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .map_err(|e| NxfError::io(format!("formatting resolved deadline: {e}")))
}

/// Parse a `<n><unit>` duration (`s` seconds, `m` minutes, `h` hours, `d` days, `w` weeks) — e.g.
/// `24h`, `30m`, `7d`. A leading run of ASCII digits followed by exactly one known unit; anything
/// else (no digits, no unit, a trailing garbage, an unknown unit) is a `validation` error. This is
/// the fallback the RFC3339 parse falls through to, so a malformed absolute date lands here and is
/// reported as an unparseable deadline rather than silently accepted. A `<n>` so large that its
/// seconds overflow `i64` is a `validation` error too — `time::Duration::days(n)` etc. are UNCHECKED
/// and would otherwise panic ("overflow constructing time::Duration"), which release mode does NOT
/// strip (Integrity #1).
fn parse_duration(s: &str, arg: &str) -> Result<time::Duration> {
    let bad = || {
        NxfError::validation(format!(
            "{arg} `{s}` is neither an RFC3339 instant nor a <n><unit> duration \
             (units: s/m/h/d/w, e.g. 24h, 30m, 7d)"
        ))
    };
    let split = s.find(|c: char| !c.is_ascii_digit()).ok_or_else(bad)?;
    if split == 0 {
        return Err(bad()); // no leading digits
    }
    let (num, unit) = s.split_at(split);
    let n: i64 = num.parse().map_err(|_| bad())?;
    let per_unit: i64 = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "w" => 604_800,
        _ => return Err(bad()),
    };
    // Checked multiply: a huge `<n>` is a `validation` error, never an overflow panic.
    let secs = n.checked_mul(per_unit).ok_or_else(|| {
        NxfError::validation(format!("{arg} `{s}` is too large (duration overflow)"))
    })?;
    Ok(time::Duration::seconds(secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::model::MessageEnvelope;
    use serde_json::json;

    fn seed() -> ChatStore {
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "kind", "group", "o/a");
        s.set_channel_field("c-1", "name", "general", "o/a");
        s.add_member("c-1", "o/reader", "o/a");
        s.add_member("c-1", "o/a", "o/a");
        s
    }

    /// Assert `key` is a genuinely present key in `obj` — NOT the same thing as `obj[key] !=
    /// Value::Null`'s absence, because `serde_json::Value`'s `Index` returns the identical static
    /// `Value::Null` for a MISSING key as for a key present holding JSON `null`. This is the check
    /// an additive, never-`skip_serializing_if` field's ordinary (`null`) case needs to be proven by
    /// (nxf 6j6v.qk5b PR review) — a bare `assert_eq!(obj[key], Value::Null)` cannot tell "always
    /// renders" from "silently dropped".
    fn assert_key_present(obj: &Value, key: &str) {
        assert!(
            obj.as_object()
                .unwrap_or_else(|| panic!("{obj} is not a JSON object"))
                .contains_key(key),
            "expected key {key:?} to be present (even when its value is null) in {obj}"
        );
    }

    fn post(s: &mut ChatStore, ch: &str, body: &str, disp: Disposition) -> String {
        s.post_message(&MessageEnvelope {
            origin: "o".into(),
            channel_id: ch.into(),
            sender: "o/a".into(),
            kind: MessageKind::Task,
            priority: Priority::Urgent,
            disposition: disp,
            thread_id: None,
            refs: Refs::default(),
            body: body.into(),
        })
    }

    #[test]
    fn every_write_entry_point_rejects_a_blank_actor_instead_of_panicking() {
        // Review finding Code Quality #1 (PR #311). For `send`/`reply`/`ask` a blank actor would
        // not trip the substrate's assert — the op author is `<origin>/<actor>`, so it would land
        // as the degenerate `"o/"` — which is exactly why the check belongs at this seam and not
        // only at the store: chat is the surface where an op authorizes an agent ACTION, and a
        // half-empty identity is not one. `set_expects` passes its handle straight onto the op,
        // where a blank one WOULD panic.
        let mut s = seed();
        let sent = post(&mut s, "c-1", "seed", Disposition::InTurn);

        for blank in ["", "   ", "\t\n"] {
            let send_err = send(
                &mut s,
                SendRequest {
                    now: "2026-07-10T00:00:00Z",
                    origin: "o",
                    actor: blank,
                    channel: "c-1",
                    body: "b",
                    kind: MessageKind::Task,
                    priority: Priority::Normal,
                    disposition: Disposition::InTurn,
                    thread: None,
                    refs: Refs::default(),
                },
            )
            .unwrap_err();
            assert_eq!(send_err.kind, ErrorKind::Validation, "send {blank:?}");

            let reply_err = reply(
                &mut s,
                ReplyRequest {
                    now: "2026-07-10T00:00:00Z",
                    origin: "o",
                    actor: blank,
                    target: &sent,
                    body: "b",
                    kind: MessageKind::Task,
                    priority: Priority::Normal,
                    disposition: Disposition::InTurn,
                    refs: Refs::default(),
                    if_unanswered: false,
                },
            )
            .unwrap_err();
            assert_eq!(reply_err.kind, ErrorKind::Validation, "reply {blank:?}");

            let ask_err = ask(
                &mut s,
                AskRequest {
                    now: "2026-07-10T00:00:00Z",
                    origin: "o",
                    actor: blank,
                    channel: "c-1",
                    body: "b",
                    expect: &["o/reader".to_string()],
                    deadline: None,
                    kind: MessageKind::Question,
                    priority: Priority::Normal,
                    refs: Refs::default(),
                },
            )
            .unwrap_err();
            assert_eq!(ask_err.kind, ErrorKind::Validation, "ask {blank:?}");
        }

        // Nothing degenerate was authored, and nothing blank reached the log.
        let bad: i64 = s
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM ops WHERE author IS NULL OR trim(author) = '' OR author = 'o/'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bad, 0, "no op carries a blank or half-empty identity");
    }

    #[test]
    fn channels_carries_the_lane_fields_in_one_query() {
        // It was `channels_carries_unread_counts_in_two_queries` until nxf 6j6v.4d2z: the second
        // query was the unread fold, and both counts left `ChannelView` with the apparatus. What
        // an app reads off a channel is what is left — and the messages are still posted here, so
        // the row is derived over a channel with traffic rather than an empty one.
        let mut s = seed();
        post(&mut s, "c-1", "in turn", Disposition::InTurn);
        post(&mut s, "c-1", "next session", Disposition::NextSession);
        let rows = channels(&s, "o/reader").unwrap();
        assert_eq!(rows.len(), 1);
        let c = &rows[0];
        assert_eq!(c.channel_id, "c-1");
        assert_eq!(c.name.as_deref(), Some("general"));
        assert_eq!(c.kind.as_deref(), Some("group"));
        assert_eq!(c.members, 2);
        assert!(!c.degraded);
    }

    #[test]
    fn messages_requires_membership() {
        let s = seed();
        assert_eq!(
            messages(&s, "c-1", "o/outsider").unwrap_err().kind,
            ErrorKind::Forbidden
        );
        assert_eq!(
            messages(&s, "c-nope", "o/a").unwrap_err().kind,
            ErrorKind::NotFound
        );
        assert!(messages(&s, "c-1", "o/a").unwrap().is_empty());
    }

    #[test]
    fn messages_returns_typed_views_in_order() {
        let mut s = seed();
        let a = post(&mut s, "c-1", "one", Disposition::InTurn);
        let b = post(&mut s, "c-1", "two", Disposition::InTurn);
        let v = messages(&s, "c-1", "o/a").unwrap();
        // Concrete causal order [a, b] (0b1m): `messages` reads via `messages_in_channel`, now
        // ordered by `(lamport, site, message_id)`, so the first post reads back first —
        // deterministically, even for two same-millisecond ULIDs (was a sorted-id workaround).
        assert_eq!(
            v.iter().map(|m| m.message_id.clone()).collect::<Vec<_>>(),
            vec![a, b]
        );
        assert!(v
            .iter()
            .all(|m| m.kind == MessageKind::Task && m.priority == Priority::Urgent));
    }

    #[test]
    fn thread_assembles_messages_and_gates_membership() {
        let mut s = seed();
        let mut e = MessageEnvelope {
            origin: "o".into(),
            channel_id: "c-1".into(),
            sender: "o/a".into(),
            kind: MessageKind::Question,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("m-th".into()),
            refs: Refs::default(),
            body: "q?".into(),
        };
        let id1 = s.post_message(&e);
        e.body = "a!".into();
        let id2 = s.post_message(&e);
        let t = thread(&s, "m-th", "o/a", Visibility::AllMembers).unwrap();
        assert_eq!(t.channel_id, "c-1");
        assert_eq!(t.opener.as_deref(), Some("o/a"));
        // Concrete causal order [id1, id2] (0b1m): `thread` reads via `messages_in_thread`, now
        // ordered by `(lamport, site, message_id)`, so the question reads back before its same-ms
        // answer deterministically (was a sorted-id workaround for the non-deterministic id order).
        assert_eq!(
            t.messages
                .iter()
                .map(|m| m.message_id.clone())
                .collect::<Vec<_>>(),
            vec![id1, id2]
        );
        let bodies: std::collections::HashSet<&str> =
            t.messages.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(bodies, std::collections::HashSet::from(["q?", "a!"]));
        assert_eq!(
            thread(&s, "m-th", "o/outsider", Visibility::AllMembers)
                .unwrap_err()
                .kind,
            ErrorKind::Forbidden
        );
        assert_eq!(
            thread(&s, "m-missing", "o/a", Visibility::AllMembers)
                .unwrap_err()
                .kind,
            ErrorKind::NotFound
        );
    }

    #[test]
    fn search_returns_hits_in_the_callers_channels() {
        let mut s = seed();
        post(&mut s, "c-1", "ship the release now", Disposition::InTurn);
        // No declarations: every channel here is `undeclared` ⇒ `AllMembers`, so the visibility
        // filter (nxf 6j6v.px98) is a true no-op and this test still says what it always said.
        let hits = search(&s, "o/reader", "RELEASE", &[]).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].body, "ship the release now");
        assert!(
            search(&s, "o/outsider", "release", &[]).unwrap().is_empty(),
            "non-member sees nothing"
        );
    }

    #[test]
    fn send_rejects_unknown_channel_and_persists_to_known() {
        let mut s = seed();
        let req = |channel| SendRequest {
            now: "2026-07-10T00:00:00Z",
            origin: "o",
            actor: "a",
            channel,
            body: "hi",
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: None,
            refs: Refs::default(),
        };
        assert_eq!(
            send(&mut s, req("c-nope")).unwrap_err().kind,
            ErrorKind::NotFound
        );
        let r = send(&mut s, req("c-1")).unwrap();
        assert!(r.message_id.starts_with("m-"));
        let row = &s.messages_in_channel("c-1").unwrap()[0];
        assert_eq!(row.sender, "o/a", "sender is origin/actor");
        assert_eq!(
            row.created.as_deref(),
            Some("2026-07-10T00:00:00Z"),
            "now stamped"
        );
    }

    #[test]
    fn reply_inherits_thread_from_a_message_target() {
        let mut s = seed();
        let e = MessageEnvelope {
            origin: "o".into(),
            channel_id: "c-1".into(),
            sender: "o/a".into(),
            kind: MessageKind::Question,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("m-th".into()),
            refs: Refs::default(),
            body: "root".into(),
        };
        let root_id = s.post_message(&e);
        let r = reply(
            &mut s,
            ReplyRequest {
                now: "2026-07-10T00:00:00Z",
                origin: "o",
                actor: "a",
                target: &root_id,
                body: "answer",
                kind: MessageKind::Report,
                priority: Priority::Normal,
                disposition: Disposition::InTurn,
                refs: Refs::default(),
                if_unanswered: false,
            },
        )
        .unwrap()
        .expect("if_unanswered is false, so this always posts");
        assert_eq!(
            r.thread_id.as_deref(),
            Some("m-th"),
            "inherits the target message's thread"
        );
    }

    #[test]
    fn reply_unknown_target_is_not_found() {
        let mut s = seed();
        assert_eq!(
            reply(
                &mut s,
                ReplyRequest {
                    now: "2026-07-10T00:00:00Z",
                    origin: "o",
                    actor: "a",
                    target: "m-ghost",
                    body: "x",
                    kind: MessageKind::Info,
                    priority: Priority::Normal,
                    disposition: Disposition::InTurn,
                    refs: Refs::default(),
                    if_unanswered: false,
                }
            )
            .unwrap_err()
            .kind,
            ErrorKind::NotFound
        );
    }

    #[test]
    fn message_view_parses_refs_and_enums() {
        let mut s = seed();
        s.post_message(&MessageEnvelope {
            origin: "o".into(),
            channel_id: "c-1".into(),
            sender: "o/a".into(),
            kind: MessageKind::Decision,
            priority: Priority::Background,
            disposition: Disposition::NextSession,
            thread_id: None,
            refs: Refs {
                nxf_ids: vec!["6j6v.be9y".into()],
                branch: Some("feat/x".into()),
                ..Refs::default()
            },
            body: "decided".into(),
        });
        let v = messages(&s, "c-1", "o/a").unwrap();
        assert_eq!(v[0].kind, MessageKind::Decision);
        assert_eq!(v[0].priority, Priority::Background);
        assert_eq!(v[0].disposition, Disposition::NextSession);
        assert_eq!(v[0].refs.nxf_ids, vec!["6j6v.be9y".to_string()]);
        assert_eq!(v[0].refs.branch.as_deref(), Some("feat/x"));
    }

    // ---- M2: ask / threads list-show / expect ------------------------------

    const M2_NOW: &str = "2026-07-10T00:00:00Z";

    /// Open a board on `c-1` (opener `o/a`) via the real `ask` facade.
    fn ask_c1(s: &mut ChatStore, expect: &[&str], deadline: Option<&str>) -> AskReceipt {
        let expect: Vec<String> = expect.iter().map(|h| h.to_string()).collect();
        ask(
            s,
            AskRequest {
                now: M2_NOW,
                origin: "o",
                actor: "a",
                channel: "c-1",
                body: "please review",
                expect: &expect,
                deadline,
                kind: MessageKind::Question,
                priority: Priority::Normal,
                refs: Refs::default(),
            },
        )
        .unwrap()
    }

    /// Post a reply from `sender` into thread `thread_id` (channel `c-1`). Returns the minted id.
    fn reply_from(s: &mut ChatStore, thread_id: &str, sender: &str, body: &str) -> String {
        s.post_message(&MessageEnvelope {
            origin: "o".into(),
            channel_id: "c-1".into(),
            sender: sender.into(),
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some(thread_id.into()),
            refs: Refs::default(),
            body: body.into(),
        })
    }

    #[test]
    fn ask_opens_a_thread_declares_expectations_and_posts() {
        let mut s = seed();
        let r = ask_c1(&mut s, &["o/b", "o/c"], None);
        assert!(r.thread_id.starts_with("m-"));
        assert!(r.message_id.starts_with("m-"));
        assert!(r.persisted);
        assert_eq!(r.expects, vec!["o/b", "o/c"]);
        assert_eq!(r.deadline, None);
        // The board derives straight from the write: both expected handles outstanding, incomplete.
        let q = s.thread_quorum(&r.thread_id, M2_NOW).unwrap().unwrap();
        assert_eq!(q.opener.as_deref(), Some("o/a"));
        assert_eq!(q.channel_id.as_deref(), Some("c-1"));
        assert_eq!(q.expects, vec!["o/b", "o/c"]);
        assert_eq!(q.outstanding, vec!["o/b", "o/c"]);
        assert!(!q.complete);
        // The request message landed in the thread.
        let msgs = s.messages_in_thread(&r.thread_id).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].message_id, r.message_id);
        assert_eq!(msgs[0].body, "please review");
    }

    #[test]
    fn ask_rejects_an_unknown_channel() {
        let mut s = seed();
        let e = ask(
            &mut s,
            AskRequest {
                now: M2_NOW,
                origin: "o",
                actor: "a",
                channel: "c-nope",
                body: "x",
                expect: &["o/b".to_string()],
                deadline: None,
                kind: MessageKind::Question,
                priority: Priority::Normal,
                refs: Refs::default(),
            },
        )
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::NotFound);
    }

    #[test]
    fn ask_resolves_a_duration_deadline_to_absolute() {
        let mut s = seed();
        let r = ask_c1(&mut s, &["o/b"], Some("24h"));
        assert_eq!(r.deadline.as_deref(), Some("2026-07-11T00:00:00Z"));
        // The absolute instant is what the store folded (no per-reader duration arithmetic).
        let q = s.thread_quorum(&r.thread_id, M2_NOW).unwrap().unwrap();
        assert_eq!(q.deadline.as_deref(), Some("2026-07-11T00:00:00Z"));
    }

    #[test]
    fn ask_normalizes_an_absolute_rfc3339_deadline_to_utc() {
        let mut s = seed();
        // An already-UTC instant is unchanged…
        let r = ask_c1(&mut s, &["o/b"], Some("2026-07-20T00:00:00Z"));
        assert_eq!(r.deadline.as_deref(), Some("2026-07-20T00:00:00Z"));
        // …and an offset-form instant is normalized to UTC before it is stored (so `stale`'s string
        // comparison is sound — Code Quality #2 / Integrity #2).
        let r2 = ask_c1(&mut s, &["o/c"], Some("2026-07-20T05:00:00+05:00"));
        assert_eq!(r2.deadline.as_deref(), Some("2026-07-20T00:00:00Z"));
    }

    #[test]
    fn ask_rejects_a_malformed_deadline_and_writes_nothing() {
        let mut s = seed();
        let e = ask(
            &mut s,
            AskRequest {
                now: M2_NOW,
                origin: "o",
                actor: "a",
                channel: "c-1",
                body: "x",
                expect: &["o/b".to_string()],
                deadline: Some("whenever"),
                kind: MessageKind::Question,
                priority: Priority::Normal,
                refs: Refs::default(),
            },
        )
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Validation);
        // No half-open thread: the reject happened before any op was emitted.
        assert!(s.member_thread_ids("o/a", None).unwrap().is_empty());
    }

    #[test]
    fn set_expects_is_opener_only_and_re_derives_the_board() {
        let mut s = seed();
        let r = ask_c1(&mut s, &["o/b", "o/c"], None);
        reply_from(&mut s, &r.thread_id, "o/b", "lgtm");
        assert!(
            !s.thread_quorum(&r.thread_id, M2_NOW)
                .unwrap()
                .unwrap()
                .complete,
            "o/c still outstanding"
        );
        // A non-opener may not amend.
        assert_eq!(
            set_expects(&mut s, M2_NOW, "o/b", &r.thread_id, &["o/b".to_string()])
                .unwrap_err()
                .kind,
            ErrorKind::Forbidden
        );
        // The opener narrows to just the responder. Since 6j6v.cg8g that re-ASKS o/b as of now
        // rather than completing the board off its earlier answer — a re-declaration is a question
        // (see `ChatStore::thread_quorums`' own doc), and the receipt this verb hands back says so.
        let q = set_expects(&mut s, M2_NOW, "o/a", &r.thread_id, &["o/b".to_string()]).unwrap();
        assert_eq!(q.expects, vec!["o/b"]);
        assert_eq!(q.outstanding, vec!["o/b"]);
        assert!(!q.complete, "the narrowed set is asked again: {q:?}");
        // …and answering the re-declaration completes it.
        reply_from(&mut s, &r.thread_id, "o/b", "still lgtm");
        assert!(
            s.thread_quorum(&r.thread_id, M2_NOW)
                .unwrap()
                .unwrap()
                .complete,
            "the re-declared turn is answered"
        );
        // An unknown thread → not_found.
        assert_eq!(
            set_expects(&mut s, M2_NOW, "o/a", "t-missing", &["o/b".to_string()])
                .unwrap_err()
                .kind,
            ErrorKind::NotFound
        );
    }

    #[test]
    fn threads_lists_membership_scoped_boards_in_bulk() {
        let mut s = seed();
        let r = ask_c1(&mut s, &["o/b"], None);
        // A member of c-1 sees the board…
        let boards = threads(&s, "o/a", M2_NOW, None).unwrap();
        assert_eq!(boards.len(), 1);
        assert_eq!(boards[0].quorum.thread_id, r.thread_id);
        assert_eq!(boards[0].quorum.expects, vec!["o/b"]);
        // …restricting to the same channel is unchanged, a different channel is empty…
        assert_eq!(threads(&s, "o/a", M2_NOW, Some("c-1")).unwrap().len(), 1);
        assert!(threads(&s, "o/a", M2_NOW, Some("c-other"))
            .unwrap()
            .is_empty());
        // …and a non-member sees nothing.
        assert!(threads(&s, "o/outsider", M2_NOW, None).unwrap().is_empty());
    }

    #[test]
    fn threads_and_thread_board_report_working_tree_holding_waiting_and_the_ordinary_case() {
        // The DoD scenario (nxf 6j6v.qk5b) at the derivation's own level: three threads, one
        // holding the working-tree lease, one waiting behind it at queue position 1, one with no
        // relation to it — `working_tree`/`working_tree_queue_position` are ADDITIVE on every entry
        // (never an omitted key), `null` for the ordinary case. `threads` (list) and `thread_board`
        // (show) must agree, since both call the same `working_tree_status_by_thread`.
        use crate::working_tree::{QueuedTrigger, WorkScope};

        let mut s = seed();
        let holding = ask_c1(&mut s, &["o/b"], None).thread_id;
        let waiting = ask_c1(&mut s, &["o/b"], None).thread_id;
        let neither = ask_c1(&mut s, &["o/b"], None).thread_id;

        assert!(s
            .acquire_working_tree(
                &WorkScope::Thread(holding.clone()),
                M2_NOW,
                "2026-07-10T12:00:00Z",
            )
            .unwrap());
        s.enqueue_working_tree(
            &QueuedTrigger {
                id: 0,
                scope_key: WorkScope::Thread(waiting.clone()).key(),
                role: "b".into(),
                session: "s-1".into(),
                thread: Some(waiting.clone()),
                message: "go".into(),
                model: None,
                depth: 0,
                priority: Priority::Normal,
                enqueued_at: None,
            },
            M2_NOW,
        )
        .unwrap();

        let boards = threads(&s, "o/a", M2_NOW, None).unwrap();
        assert_eq!(boards.len(), 3);
        let board = |id: &str| boards.iter().find(|b| b.quorum.thread_id == id).unwrap();
        assert_eq!(
            board(&holding).working_tree,
            Some(WorkingTreeStatus::Holding)
        );
        assert_eq!(board(&holding).working_tree_queue_position, None);
        assert_eq!(
            board(&waiting).working_tree,
            Some(WorkingTreeStatus::Waiting)
        );
        assert_eq!(board(&waiting).working_tree_queue_position, Some(1));
        assert_eq!(board(&neither).working_tree, None, "the ordinary case");
        assert_eq!(board(&neither).working_tree_queue_position, None);

        // Rust-level `Option::None` cannot tell "additive, always renders as null" from "would be
        // silently dropped by a future `skip_serializing_if`" — both look identical at the struct
        // level. Prove the WIRE contract directly instead: serialize and assert the keys are
        // genuinely PRESENT, not merely that `obj["working_tree"]` reads as `Value::Null` (PR
        // review of this ticket — `serde_json::Value`'s `Index` returns that same static null for a
        // MISSING key too).
        let neither_json = board(&neither).to_value();
        assert_key_present(&neither_json, "working_tree");
        assert_key_present(&neither_json, "working_tree_queue_position");
        let holding_json = board(&holding).to_value();
        assert_key_present(&holding_json, "working_tree_queue_position");

        // `thread_board` (`threads show`) derives the identical answer.
        let show = thread_board(&s, &waiting, M2_NOW, "o/a", Visibility::AllMembers).unwrap();
        assert_eq!(show.working_tree, Some(WorkingTreeStatus::Waiting));
        assert_eq!(show.working_tree_queue_position, Some(1));
        let show = thread_board(&s, &neither, M2_NOW, "o/a", Visibility::AllMembers).unwrap();
        assert_eq!(show.working_tree, None);
        assert_eq!(show.working_tree_queue_position, None);
        let show_json = show.to_value();
        assert_key_present(&show_json, "working_tree");
        assert_key_present(&show_json, "working_tree_queue_position");
    }

    #[test]
    fn a_session_scoped_queued_trigger_shows_waiting_on_its_own_thread() {
        // PR review, final pass: "order 2 is queued" is the epic's headline case, and it used to be
        // visible on the send RECEIPT only. The fixture below is the shape a scope alone cannot
        // resolve: a queued entry whose thread hangs under nothing (its session was never put in
        // motion on a thread here), which resolving "waiting" by scope alone would leave reading
        // `working_tree: null` while a trigger for it sat on the queue. The entry's own `thread`
        // closes it.
        use crate::working_tree::{QueuedTrigger, WorkScope};

        let mut s = seed();
        let dm = ask_c1(&mut s, &["o/b"], None).thread_id;

        s.enqueue_working_tree(
            &QueuedTrigger {
                id: 0,
                scope_key: WorkScope::Session("s-1".to_string()).key(),
                role: "b".into(),
                session: "s-1".into(),
                thread: Some(dm.clone()),
                message: "order 2".into(),
                model: None,
                depth: 0,
                priority: Priority::Normal,
                enqueued_at: None,
            },
            M2_NOW,
        )
        .unwrap();

        let board = thread_board(&s, &dm, M2_NOW, "o/a", Visibility::AllMembers).unwrap();
        assert_eq!(
            board.working_tree,
            Some(WorkingTreeStatus::Waiting),
            "the queued trigger's own thread is waiting, even though the run claims no thread"
        );
        assert_eq!(board.working_tree_queue_position, Some(1));
        // `threads list` agrees, since both go through `working_tree_status_by_thread`.
        let boards = threads(&s, "o/a", M2_NOW, None).unwrap();
        let listed = boards.iter().find(|b| b.quorum.thread_id == dm).unwrap();
        assert_eq!(listed.working_tree, Some(WorkingTreeStatus::Waiting));
        assert_eq!(listed.working_tree_queue_position, Some(1));
    }

    #[test]
    fn a_holder_scope_still_outranks_a_queue_entry_naming_the_same_thread() {
        // The precedence the `or_insert` above relies on, pinned rather than assumed: a thread the
        // HOLDER's scope claims must read `holding`, even when a queued entry also names it by its
        // own `thread` field. Getting this backwards would report the chain that owns the working
        // copy as waiting for it.
        use crate::working_tree::{QueuedTrigger, WorkScope};

        let mut s = seed();
        let thread_id = ask_c1(&mut s, &["o/b"], None).thread_id;
        assert!(s
            .acquire_working_tree(
                &WorkScope::Thread(thread_id.clone()),
                M2_NOW,
                "2026-07-10T12:00:00Z",
            )
            .unwrap());
        s.enqueue_working_tree(
            &QueuedTrigger {
                id: 0,
                scope_key: WorkScope::Session("s-1".to_string()).key(),
                role: "b".into(),
                session: "s-1".into(),
                thread: Some(thread_id.clone()),
                message: "go".into(),
                model: None,
                depth: 0,
                priority: Priority::Normal,
                enqueued_at: None,
            },
            M2_NOW,
        )
        .unwrap();

        let board = thread_board(&s, &thread_id, M2_NOW, "o/a", Visibility::AllMembers).unwrap();
        assert_eq!(board.working_tree, Some(WorkingTreeStatus::Holding));
        assert_eq!(board.working_tree_queue_position, None);
    }

    #[test]
    fn a_thread_named_by_two_queued_entries_keeps_the_earlier_position() {
        // A promoted trigger that loses the post-release race re-queues behind whatever arrived
        // meanwhile (nxf 6j6v.fe0f) — so the SAME thread can legitimately appear more than once in
        // the queue. The position a reader sees must be the one that will actually reach it first.
        use crate::working_tree::{QueuedTrigger, WorkScope};

        let mut s = seed();
        let thread_id = ask_c1(&mut s, &["o/b"], None).thread_id;
        let mine = WorkScope::Thread(thread_id.clone()).key();
        // `scope_key` and `thread` move TOGETHER, as they do in production: `trigger_role` derives
        // the scope key from this very thread, so an entry naming somebody else's scope names
        // somebody else's thread too. (They used to be varied independently here, which mattered
        // once the queue loop started reading `thread` as well as the scope — see
        // `working_tree_status_by_thread`.)
        let entry = |scope: &WorkScope, thread: &str| QueuedTrigger {
            id: 0,
            scope_key: scope.key(),
            role: "b".into(),
            session: "s-1".into(),
            thread: Some(thread.to_string()),
            message: "go".into(),
            model: None,
            depth: 0,
            priority: Priority::Normal,
            enqueued_at: None,
        };
        // Somebody else's entry queues first (position 1)…
        s.enqueue_working_tree(
            &entry(&WorkScope::Thread("other".to_string()), "other"),
            M2_NOW,
        )
        .unwrap();
        // …then this thread's own entry queues twice (position 2, then 3).
        let mine_scope = WorkScope::parse(&mine).unwrap();
        s.enqueue_working_tree(&entry(&mine_scope, &thread_id), M2_NOW)
            .unwrap();
        s.enqueue_working_tree(&entry(&mine_scope, &thread_id), M2_NOW)
            .unwrap();

        let board = thread_board(&s, &thread_id, M2_NOW, "o/a", Visibility::AllMembers).unwrap();
        assert_eq!(board.working_tree, Some(WorkingTreeStatus::Waiting));
        assert_eq!(
            board.working_tree_queue_position,
            Some(2),
            "the FIRST entry naming this thread wins, not its later duplicate"
        );
    }

    #[test]
    fn thread_board_shows_quorum_with_messages_and_gates_membership() {
        let mut s = seed();
        let r = ask_c1(&mut s, &["o/b"], None);
        let reply_id = reply_from(&mut s, &r.thread_id, "o/b", "done");
        let board = thread_board(&s, &r.thread_id, M2_NOW, "o/a", Visibility::AllMembers).unwrap();
        assert!(board.quorum.complete);
        assert_eq!(board.quorum.replied, vec!["o/b"]);
        assert_eq!(board.messages.len(), 2, "request + reply");
        // Concrete causal order [request, reply] (0b1m): the board reads via `messages_in_thread`,
        // now ordered by `(lamport, site, message_id)`. The reply's op is emitted after the request
        // it answers, so its lamport is strictly higher — the request reads back first, deterministically
        // and reproducibly, even when both ids are minted in the same millisecond. (This reverts the
        // PR #200 sorted-id workaround, which only masked the non-deterministic `ORDER BY message_id`.)
        assert_eq!(
            board
                .messages
                .iter()
                .map(|m| m.message_id.clone())
                .collect::<Vec<_>>(),
            vec![r.message_id.clone(), reply_id]
        );
        let bodies: std::collections::HashSet<&str> =
            board.messages.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(
            bodies,
            std::collections::HashSet::from(["please review", "done"])
        );
        // A non-member is forbidden; an unknown thread is not_found.
        assert_eq!(
            thread_board(
                &s,
                &r.thread_id,
                M2_NOW,
                "o/outsider",
                Visibility::AllMembers
            )
            .unwrap_err()
            .kind,
            ErrorKind::Forbidden
        );
        assert_eq!(
            thread_board(&s, "t-missing", M2_NOW, "o/a", Visibility::AllMembers)
                .unwrap_err()
                .kind,
            ErrorKind::NotFound
        );
    }

    // ---- 6j6v.xrk3: channel visibility filter (the `filter_board_messages` helper) --------

    /// A minimal [`MessageView`] fixture for exercising `filter_board_messages` in isolation
    /// (whitebox — no store round-trip needed for a pure function over already-loaded messages).
    fn mv(id: &str, sender: &str, body: &str) -> MessageView {
        MessageView {
            message_id: id.to_string(),
            origin: "o".to_string(),
            channel_id: "c-1".to_string(),
            sender: sender.to_string(),
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("m-th".to_string()),
            refs: Refs::default(),
            body: body.to_string(),
            created: None,
            unvouched: false,
        }
    }

    #[test]
    fn filter_board_messages_all_members_is_a_no_op() {
        let opening = mv("m-1", "o/opener", "opening message");
        let reply_a = mv("m-2", "o/member-a", "a's reply");
        let reply_b = mv("m-3", "o/member-b", "b's reply");
        let messages = vec![opening.clone(), reply_a.clone(), reply_b.clone()];
        // Even the non-opener member-b, under AllMembers, sees everything unfiltered.
        let filtered = filter_board_messages(
            messages,
            "o/member-b",
            Some("o/opener"),
            Visibility::AllMembers,
        );
        assert_eq!(filtered, vec![opening, reply_a, reply_b]);
    }

    #[test]
    fn filter_board_messages_requester_only_never_filters_the_opener() {
        let opening = mv("m-1", "o/opener", "opening message");
        let reply_a = mv("m-2", "o/member-a", "a's reply");
        let reply_b = mv("m-3", "o/member-b", "b's reply");
        let messages = vec![opening.clone(), reply_a.clone(), reply_b.clone()];
        let filtered = filter_board_messages(
            messages,
            "o/opener",
            Some("o/opener"),
            Visibility::RequesterOnly,
        );
        assert_eq!(filtered, vec![opening, reply_a, reply_b]);
    }

    #[test]
    fn filter_board_messages_requester_only_keeps_opening_plus_readers_own_replies_in_order() {
        let opening = mv("m-1", "o/opener", "opening message");
        let b1 = mv("m-2", "o/member-b", "b1");
        let a1 = mv("m-3", "o/member-a", "a1");
        let b2 = mv("m-4", "o/member-b", "b2");
        let messages = vec![opening.clone(), b1.clone(), a1, b2.clone()];
        // member-b's own replies keep their original relative order; member-a's is dropped.
        let filtered = filter_board_messages(
            messages,
            "o/member-b",
            Some("o/opener"),
            Visibility::RequesterOnly,
        );
        assert_eq!(filtered, vec![opening, b1, b2]);
    }

    #[test]
    fn filter_board_messages_requester_only_with_no_own_replies_keeps_only_the_opening() {
        let opening = mv("m-1", "o/opener", "opening message");
        let reply_a = mv("m-2", "o/member-a", "a's reply");
        let messages = vec![opening.clone(), reply_a];
        let filtered = filter_board_messages(
            messages,
            "o/member-b",
            Some("o/opener"),
            Visibility::RequesterOnly,
        );
        assert_eq!(filtered, vec![opening]);
    }

    #[test]
    fn duration_deadlines_resolve_and_reject() {
        // The `--deadline` grammar: an absolute instant is normalized to UTC; a <n><unit> duration is
        // added to `now`; anything else is a validation error.
        assert_eq!(
            resolve_deadline(M2_NOW, "24h").unwrap(),
            "2026-07-11T00:00:00Z"
        );
        assert_eq!(
            resolve_deadline(M2_NOW, "30m").unwrap(),
            "2026-07-10T00:30:00Z"
        );
        assert_eq!(
            resolve_deadline(M2_NOW, "7d").unwrap(),
            "2026-07-17T00:00:00Z"
        );
        assert_eq!(
            resolve_deadline(M2_NOW, "1w").unwrap(),
            "2026-07-17T00:00:00Z"
        );
        // An already-UTC absolute instant is unchanged.
        assert_eq!(
            resolve_deadline(M2_NOW, "2026-07-20T00:00:00Z").unwrap(),
            "2026-07-20T00:00:00Z"
        );
        for bad in ["soon", "24", "h", "24x", ""] {
            assert_eq!(
                resolve_deadline(M2_NOW, bad).unwrap_err().kind,
                ErrorKind::Validation,
                "`{bad}` is not a valid deadline"
            );
        }
    }

    #[test]
    fn resolve_deadline_rejects_a_duration_that_overflows_instead_of_panicking() {
        // Integrity #1: a huge `<n><unit>` must be a `validation` error, NOT an
        // "overflow constructing time::Duration" panic (unchecked `time::Duration::days` etc.),
        // which release mode does not strip. Both the seconds multiply and the date add are checked.
        for huge in [
            "999999999999999d",
            "99999999999999999999w",
            "9223372036854775807d",
        ] {
            assert_eq!(
                resolve_deadline(M2_NOW, huge).unwrap_err().kind,
                ErrorKind::Validation,
                "`{huge}` overflows → validation, never a panic"
            );
        }
    }

    #[test]
    fn resolve_deadline_normalizes_a_non_utc_offset_to_utc() {
        // Code Quality #2 / Integrity #2: an offset-form absolute instant is normalized to UTC (`Z`)
        // so `stale`'s lexicographic string comparison against the reader's UTC clock is sound. Both
        // of these name the SAME instant as `2026-07-20T12:00:00Z`.
        assert_eq!(
            resolve_deadline(M2_NOW, "2026-07-20T17:00:00+05:00").unwrap(),
            "2026-07-20T12:00:00Z"
        );
        assert_eq!(
            resolve_deadline(M2_NOW, "2026-07-20T07:00:00-05:00").unwrap(),
            "2026-07-20T12:00:00Z"
        );
    }

    #[test]
    fn a_future_deadline_in_a_negative_offset_zone_is_not_falsely_stale() {
        // Integrity #2 reproduced then fixed: a deadline 2 hours in the FUTURE, expressed in a
        // negative-offset zone, once stored verbatim string-sorted as PAST → false `stale:true`.
        // With UTC normalization at `ask` time the board reads NOT stale. `now` = 12:00Z; the
        // deadline names 14:00Z via a `-05:00` zone (09:00-05:00).
        let mut s = seed();
        let now = "2026-07-20T12:00:00Z";
        let r = ask(
            &mut s,
            AskRequest {
                now,
                origin: "o",
                actor: "a",
                channel: "c-1",
                body: "review please",
                expect: &["o/b".to_string()],
                deadline: Some("2026-07-20T09:00:00-05:00"), // == 14:00Z, two hours ahead
                kind: MessageKind::Question,
                priority: Priority::Normal,
                refs: Refs::default(),
            },
        )
        .unwrap();
        assert_eq!(
            r.deadline.as_deref(),
            Some("2026-07-20T14:00:00Z"),
            "stored UTC-normalized"
        );
        let q = s.thread_quorum(&r.thread_id, now).unwrap().unwrap();
        assert!(
            !q.stale,
            "a future deadline must not read as stale, regardless of its source zone"
        );
    }

    // ---- agent transcript (nxf epic 6wt2, ticket 5bym) ---------------------

    /// A seq-less wire entry, the shape `append_transcript` takes. `parent`/`tool_use_id` are what
    /// the nesting is reconstructed from, so every transcript test states both explicitly.
    fn te(
        kind: &str,
        tool_use_id: Option<&str>,
        parent: Option<&str>,
        data: serde_json::Value,
    ) -> crate::transcript::TranscriptEntry {
        crate::transcript::TranscriptEntry {
            kind: kind.to_string(),
            at: Some("2026-07-26T07:42:39.123Z".to_string()),
            tool_use_id: tool_use_id.map(str::to_string),
            parent_tool_use_id: parent.map(str::to_string),
            subagent_type: parent.map(|_| "code-reviewer".to_string()),
            data,
        }
    }

    #[test]
    fn transcript_nests_a_subagents_entries_under_the_tool_use_that_spawned_them() {
        // The beads gap, closed: its persist path dropped subagent entries, so a Task's own
        // sub-timeline vanished on reload. Here the flat rows carry the spawning Task's id and the
        // read reassembles the tree — main conversation at top level, the subagent's four entries
        // nested under the `tool_use` whose `tool_use_id` they name, both in `seq` order.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript(
            "s-1",
            &[
                te(
                    "session_init",
                    None,
                    None,
                    json!({ "sdkSessionId": "real-abc" }),
                ),
                te(
                    "tool_use",
                    Some("toolu_task_1"),
                    None,
                    json!({ "name": "Task" }),
                ),
                te(
                    "thinking",
                    None,
                    Some("toolu_task_1"),
                    json!({ "text": "…" }),
                ),
                te(
                    "tool_use",
                    Some("toolu_read_9"),
                    Some("toolu_task_1"),
                    json!({ "name": "Read" }),
                ),
                te(
                    "tool_result",
                    Some("toolu_read_9"),
                    Some("toolu_task_1"),
                    json!({ "content": "fn main() {}" }),
                ),
                te(
                    "assistant",
                    None,
                    Some("toolu_task_1"),
                    json!({ "text": "looks fine" }),
                ),
                te(
                    "tool_result",
                    Some("toolu_task_1"),
                    None,
                    json!({ "content": "looks fine" }),
                ),
                te("result", None, None, json!({ "subtype": "success" })),
            ],
        )
        .unwrap();

        let v = transcript(&s, "s-1").unwrap();
        assert_eq!(v.session, "s-1");
        assert_eq!(
            v.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [0, 1, 6, 7],
            "top level is the main conversation only, in seq order"
        );
        let task = &v.entries[1];
        assert_eq!(
            task.subagent.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [2, 3, 4, 5],
            "the sub-timeline hangs off the spawning tool_use, in seq order"
        );
        assert_eq!(
            task.subagent[0].subagent_type.as_deref(),
            Some("code-reviewer")
        );
        // Nesting is ONE level: a subagent's own tool_use keeps an empty `subagent` vec (the SDK
        // reports the spawning Task's id for grandchildren too, so they surface under this parent).
        assert!(task.subagent.iter().all(|e| e.subagent.is_empty()));
        // The Task's own tool_result rides the SAME tool_use_id (seq 6) but is main-conversation, so
        // it is a sibling of the tool_use, never a second parent that steals the sub-timeline.
        assert_eq!(v.entries[2].kind, "tool_result");
        assert!(v.entries[2].subagent.is_empty());
    }

    #[test]
    fn transcript_emits_an_orphan_subagent_entry_at_top_level_in_its_own_seq_position() {
        // LOAD-BEARING (the read drops nothing): a transcript flushed mid-Task — or one whose
        // spawning `tool_use` entry was lost — still shows every entry it has. The orphan keeps its
        // `subagent_type` (the only remaining evidence of where it came from) and sits at its own
        // `seq` position, not appended at the end where it would misread as a later event.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript(
            "s-1",
            &[
                te("assistant", None, None, json!({ "text": "before" })),
                te(
                    "assistant",
                    None,
                    Some("toolu_gone"),
                    json!({ "text": "orphan" }),
                ),
                te("assistant", None, None, json!({ "text": "after" })),
            ],
        )
        .unwrap();

        let v = transcript(&s, "s-1").unwrap();
        assert_eq!(
            v.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        let orphan = &v.entries[1];
        assert_eq!(orphan.data["text"], "orphan");
        assert_eq!(orphan.subagent_type.as_deref(), Some("code-reviewer"));
    }

    #[test]
    fn transcript_nests_a_child_that_is_stored_before_its_parent() {
        // The regression guard for the TWO-PASS design (`parents` is resolved before the walk). A
        // one-pass "have I seen this tool_use yet" decision would read this transcript as an orphan
        // + a childless Task and stay green on every other test in this epic — softening the orphan
        // rule into "anything out of order is an orphan", which the rule explicitly forbids. Rows in
        // this order are legal: the store preserves ARRIVAL order, and only the sidecar's own
        // emission order makes a parent come first.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript(
            "s-1",
            &[
                te(
                    "thinking",
                    None,
                    Some("toolu_task_1"),
                    json!({ "text": "…" }),
                ),
                te(
                    "tool_use",
                    Some("toolu_task_1"),
                    None,
                    json!({ "name": "Task" }),
                ),
            ],
        )
        .unwrap();
        let v = transcript(&s, "s-1").unwrap();
        assert_eq!(
            v.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [1],
            "the child nested even though it was stored first"
        );
        assert_eq!(
            v.entries[0]
                .subagent
                .iter()
                .map(|e| e.seq)
                .collect::<Vec<_>>(),
            [0]
        );
    }

    #[test]
    fn transcript_gives_a_duplicated_tool_use_id_to_the_first_parent_and_drops_nothing() {
        // Two main-conversation `tool_use` entries sharing an id is not a shape the SDK produces,
        // but the read must be total: the sub-timeline attaches to the FIRST of them (the walk
        // drains the map) and the second stays childless — never duplicated into both, and never
        // dropped because two candidates existed.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript(
            "s-1",
            &[
                te(
                    "tool_use",
                    Some("toolu_dup"),
                    None,
                    json!({ "name": "Task" }),
                ),
                te(
                    "tool_use",
                    Some("toolu_dup"),
                    None,
                    json!({ "name": "Task" }),
                ),
                te("assistant", None, Some("toolu_dup"), json!({ "text": "x" })),
            ],
        )
        .unwrap();
        let v = transcript(&s, "s-1").unwrap();
        assert_eq!(v.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(
            v.entries[0]
                .subagent
                .iter()
                .map(|e| e.seq)
                .collect::<Vec<_>>(),
            [2]
        );
        assert!(v.entries[1].subagent.is_empty());
    }

    #[test]
    fn transcript_keeps_two_subagents_under_one_parent_separate_and_seq_ordered() {
        // One Task's `tool_use` can front more than one subagent identity in practice (a
        // grandchild's entries carry the SPAWNING Task's id, so they land under the same parent).
        // They interleave in the stream and must come back interleaved in `seq` order, each entry
        // keeping its OWN `subagent_type` — the sub-timeline is not homogenized to one type.
        let mut s = ChatStore::open_in_memory(1);
        let tagged = |kind: &str, ty: &str, text: &str| crate::transcript::TranscriptEntry {
            kind: kind.to_string(),
            at: Some("2026-07-26T07:42:39.123Z".to_string()),
            tool_use_id: None,
            parent_tool_use_id: Some("toolu_task_1".to_string()),
            subagent_type: Some(ty.to_string()),
            data: json!({ "text": text }),
        };
        s.append_transcript(
            "s-1",
            &[
                te(
                    "tool_use",
                    Some("toolu_task_1"),
                    None,
                    json!({ "name": "Task" }),
                ),
                tagged("assistant", "code-reviewer", "reviewer speaking"),
                tagged("assistant", "doc-writer", "writer speaking"),
                tagged("assistant", "code-reviewer", "reviewer again"),
            ],
        )
        .unwrap();
        let v = transcript(&s, "s-1").unwrap();
        let sub = &v.entries[0].subagent;
        assert_eq!(sub.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(
            sub.iter()
                .map(|e| e.subagent_type.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["code-reviewer", "doc-writer", "code-reviewer"]
        );
    }

    #[test]
    fn a_non_tool_use_entry_carrying_the_id_never_parents_the_sub_timeline() {
        // Only a `tool_use` may parent. The Task's own `tool_result` comes back on the main
        // conversation carrying the SAME `tool_use_id`, so keying on the id alone would make it a
        // second, competing parent — and here, where the `tool_use` itself is missing entirely
        // (a mid-Task flush), a `tool_result`-as-parent would swallow the sub-timeline into an
        // entry that is not the spawning call. Instead the children stay visible as top-level
        // orphans in their own seq positions.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript(
            "s-1",
            &[
                te(
                    "tool_result",
                    Some("toolu_task_1"),
                    None,
                    json!({ "content": "looks fine" }),
                ),
                te(
                    "assistant",
                    None,
                    Some("toolu_task_1"),
                    json!({ "text": "orphaned child" }),
                ),
            ],
        )
        .unwrap();
        let v = transcript(&s, "s-1").unwrap();
        assert_eq!(v.entries.iter().map(|e| e.seq).collect::<Vec<_>>(), [0, 1]);
        assert!(
            v.entries[0].subagent.is_empty(),
            "a tool_result never adopts a sub-timeline"
        );
        assert_eq!(v.entries[1].subagent_type.as_deref(), Some("code-reviewer"));
    }

    #[test]
    fn transcript_resolves_role_and_real_sdk_id_from_the_session_map() {
        let mut s = ChatStore::open_in_memory(1);
        s.create_pending_session("s-1", "coder").unwrap();
        s.bind_session("s-1", "real-abc").unwrap();
        s.append_transcript("s-1", &[te("assistant", None, None, json!({}))])
            .unwrap();
        let v = transcript(&s, "s-1").unwrap();
        assert_eq!(v.role.as_deref(), Some("coder"));
        assert_eq!(v.real_sdk_id.as_deref(), Some("real-abc"));
    }

    // ---- the write half (nxf 6j6v.c6e8) ------------------------------------

    #[test]
    fn transcript_append_stores_a_batch_the_read_serves_back_and_the_store_assigns_seq() {
        // The gap this closes: the READ was on the seam and the WRITE was not, so a host running
        // its own role runtime could only fill a transcript by spawning `nxc` or reaching past the
        // facade into the store. Both halves are here now, and the batch that goes in is the view
        // that comes out.
        let mut s = ChatStore::open_in_memory(1);
        let appended = transcript_append(
            &mut s,
            "s-1",
            &[
                te("tool_use", Some("toolu_1"), None, json!({ "name": "Read" })),
                te("assistant", None, None, json!({ "text": "done" })),
            ],
        )
        .unwrap();
        assert_eq!(appended, 2, "the count is what the caller reports");
        let v = transcript(&s, "s-1").unwrap();
        assert_eq!(
            v.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [0, 1],
            "`seq` is the STORE's to assign — it is not on the wire and not the caller's to pass"
        );
        assert_eq!(v.entries[1].data["text"], "done");
    }

    #[test]
    fn a_second_transcript_append_continues_the_seq_rather_than_restarting_it() {
        // The property a host depends on when it flushes in batches, and the reason `seq` stays in
        // the store: two calls from the same caller must read back as one history.
        //
        // MULTI-ENTRY batches on purpose (PR #407 review, Test Quality #1). With one entry per call
        // this test could not tell a faithful passthrough from one that forwarded only the first
        // entry of each batch — measured, not supposed: under that mutation the single-entry version
        // stayed green while `transcript_append_stores_a_batch_…` went red. What this pins is the
        // seam's own job, which the store tests structurally cannot see: that the WHOLE slice
        // reaches the store, in order, on every call.
        let mut s = ChatStore::open_in_memory(1);
        let batch = |from: i64| {
            (from..from + 3)
                .map(|i| te("assistant", None, None, json!({ "i": i })))
                .collect::<Vec<_>>()
        };
        assert_eq!(transcript_append(&mut s, "s-1", &batch(0)).unwrap(), 3);
        assert_eq!(transcript_append(&mut s, "s-1", &batch(3)).unwrap(), 3);
        let v = transcript(&s, "s-1").unwrap();
        assert_eq!(
            v.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4, 5],
            "one continuous history across two calls, nothing dropped inside either batch"
        );
        assert_eq!(
            v.entries
                .iter()
                .map(|e| e.data["i"].as_i64())
                .collect::<Vec<_>>(),
            (0..6).map(Some).collect::<Vec<_>>(),
            "and in the order the caller passed them"
        );
    }

    #[test]
    fn an_empty_kind_is_refused_at_the_seam_with_the_batch_index_that_carries_it() {
        // The store's invariant, reported in the coordinate THIS caller framed its input in: a
        // batch index. The CLI reports a stdin LINE number instead, from the same shared
        // `TranscriptEntry::validate` — one predicate, two coordinates.
        let mut s = ChatStore::open_in_memory(1);
        let err = transcript_append(
            &mut s,
            "s-1",
            &[
                te("assistant", None, None, json!({})),
                te("", None, None, json!({})),
            ],
        )
        .expect_err("an empty `kind` makes the row uninterpretable forever");
        assert!(
            err.to_string().contains("transcript entry 1"),
            "the index names WHICH entry: {err}"
        );
        assert!(
            transcript(&s, "s-1").unwrap().entries.is_empty(),
            "the batch is atomic — a refused entry takes the whole flush with it, so a stored \
             prefix can never pass for a complete one"
        );
    }

    #[test]
    fn an_empty_transcript_append_batch_is_a_clean_zero() {
        let mut s = ChatStore::open_in_memory(1);
        assert_eq!(transcript_append(&mut s, "s-1", &[]).unwrap(), 0);
        assert!(transcript(&s, "s-1").unwrap().entries.is_empty());
    }

    #[test]
    fn transcript_of_an_unknown_session_is_empty_not_an_error() {
        // Same shape as a known session that has not flushed yet — a sidecar that died before its
        // first flush is indistinguishable from one that had nothing to say, and neither is a
        // caller error.
        let s = ChatStore::open_in_memory(1);
        let v = transcript(&s, "never-existed").unwrap();
        assert_eq!(v.session, "never-existed");
        assert_eq!(v.role, None);
        assert_eq!(v.real_sdk_id, None);
        assert!(v.entries.is_empty());
    }

    // ---- paging the read (nxf 6j6v.t7pa) -----------------------------------

    /// Ten plain main-conversation entries, `seq` 0..9, each distinguishable by its `data.i`.
    fn seeded_long_session() -> ChatStore {
        let mut s = ChatStore::open_in_memory(1);
        let batch: Vec<_> = (0..10)
            .map(|i| te("assistant", None, None, json!({ "i": i })))
            .collect();
        s.append_transcript("s-1", &batch).unwrap();
        s
    }

    #[test]
    fn a_window_starts_after_the_cursor_and_stops_at_the_limit() {
        let s = seeded_long_session();
        let v = transcript_page(&s, "s-1", 2, Some(3)).unwrap();
        assert_eq!(
            v.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [3, 4, 5],
            "strictly after the cursor, `limit` long"
        );
        // The cursor for the next window is the largest seq seen — the same
        // caller-tracks-its-own-cursor shape `--stream` uses.
        let next = transcript_page(&s, "s-1", 5, Some(3)).unwrap();
        assert_eq!(
            next.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [6, 7, 8]
        );
    }

    #[test]
    fn walking_the_windows_yields_every_entry_exactly_once_and_ends_short() {
        // The property that makes paging usable at all: no gap, no repeat, and a terminating walk.
        let s = seeded_long_session();
        let mut seen: Vec<i64> = Vec::new();
        let mut cursor = -1;
        loop {
            let page = transcript_page(&s, "s-1", cursor, Some(4)).unwrap();
            if page.entries.is_empty() {
                break;
            }
            cursor = page.entries.iter().map(|e| e.seq).max().unwrap();
            seen.extend(page.entries.iter().map(|e| e.seq));
            if page.entries.len() < 4 {
                break; // short window = end of session, no extra round-trip needed
            }
        }
        assert_eq!(seen, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn an_unwindowed_read_is_exactly_the_whole_session() {
        // `transcript` delegates here, so this is the "additive, nothing changed for the old
        // caller" assertion the ticket's design promised.
        let s = seeded_long_session();
        assert_eq!(
            transcript(&s, "s-1").unwrap().entries.len(),
            transcript_page(&s, "s-1", -1, None).unwrap().entries.len()
        );
        assert_eq!(transcript(&s, "s-1").unwrap().entries.len(), 10);
    }

    #[test]
    fn a_limit_counts_nested_entries_too_because_that_is_what_bounds_the_memory() {
        // A `Task` with three subagent entries under it: a limit of 2 must stop INSIDE the
        // sub-timeline rather than pull the whole tree in behind its parent. Limiting the top-level
        // lane instead would bound nothing — one Task can carry hundreds of nested entries, which is
        // exactly the session this read exists for.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript(
            "s-1",
            &[
                te(
                    "tool_use",
                    Some("toolu_task"),
                    None,
                    json!({ "name": "Task" }),
                ),
                te("assistant", None, Some("toolu_task"), json!({ "i": 1 })),
                te("assistant", None, Some("toolu_task"), json!({ "i": 2 })),
                te("assistant", None, Some("toolu_task"), json!({ "i": 3 })),
            ],
        )
        .unwrap();
        let v = transcript_page(&s, "s-1", -1, Some(2)).unwrap();
        assert_eq!(v.entries.len(), 1, "one top-level tool_use");
        assert_eq!(
            v.entries[0].subagent.len(),
            1,
            "and one of its three children"
        );
    }

    #[test]
    fn a_child_whose_parent_fell_below_the_cursor_surfaces_at_top_level_not_dropped() {
        // The one behaviour a window changes, stated as a test rather than only in prose: nesting is
        // reconstructed from the rows IN the window, so a subagent entry cut off from its spawning
        // `tool_use` takes the orphan lane. What it keeps there is asserted at the bottom.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript(
            "s-1",
            &[
                te(
                    "tool_use",
                    Some("toolu_task"),
                    None,
                    json!({ "name": "Task" }),
                ),
                te("assistant", None, Some("toolu_task"), json!({ "i": 1 })),
                te("assistant", None, Some("toolu_task"), json!({ "i": 2 })),
            ],
        )
        .unwrap();
        let v = transcript_page(&s, "s-1", 0, None).unwrap();
        assert_eq!(
            v.entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [1, 2],
            "both present, at top level"
        );
        assert!(v.entries.iter().all(|e| e.subagent.is_empty()));
        // What survives the cut is `subagent_type` and the seq position — the view carries no
        // parent id at all (by design, see `TranscriptEntryView`), which is exactly why the doc
        // says a window cannot be re-nested from two windows alone.
        assert_eq!(v.to_value()["entries"][0]["subagent_type"], "code-reviewer");
    }

    #[test]
    fn a_cursor_past_the_end_is_an_empty_window_not_an_error() {
        let s = seeded_long_session();
        let v = transcript_page(&s, "s-1", 99, Some(5)).unwrap();
        assert!(v.entries.is_empty());
        assert_eq!(v.session, "s-1");
    }

    #[test]
    fn a_nonsensical_limit_reads_as_no_limit_rather_than_as_an_empty_page() {
        // Zero or negative has no useful reading, and an empty vec would be indistinguishable from
        // "end of session" to the cursor walk — which would silently truncate a consumer's read.
        let s = seeded_long_session();
        for limit in [Some(0), Some(-3)] {
            assert_eq!(
                transcript_page(&s, "s-1", -1, limit).unwrap().entries.len(),
                10,
                "limit {limit:?}"
            );
        }
    }

    #[test]
    fn the_transcript_view_serializes_with_absent_tags_omitted_and_subagent_always_present() {
        // The `--json` contract `nxc transcript show` prints verbatim: declaration field order,
        // sparse optionals omitted (the view idiom), and `subagent` ALWAYS an array — a consumer
        // iterates it without a presence check, on every entry.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript(
            "s-1",
            &[
                te(
                    "tool_use",
                    Some("toolu_task_1"),
                    None,
                    json!({ "name": "Task" }),
                ),
                te(
                    "assistant",
                    None,
                    Some("toolu_task_1"),
                    json!({ "text": "x" }),
                ),
            ],
        )
        .unwrap();
        let v = transcript(&s, "s-1").unwrap().to_value();
        assert_eq!(v["session"], "s-1");
        assert!(v.get("role").is_none(), "absent optional omitted: {v}");
        assert!(
            v.get("real_sdk_id").is_none(),
            "absent optional omitted: {v}"
        );
        let task = &v["entries"][0];
        assert_eq!(task["seq"], 0);
        assert_eq!(task["kind"], "tool_use");
        assert_eq!(task["tool_use_id"], "toolu_task_1");
        assert!(
            task.get("subagent_type").is_none(),
            "a main-conversation entry has no subagent_type: {task}"
        );
        assert_eq!(task["subagent"][0]["subagent_type"], "code-reviewer");
        assert_eq!(task["subagent"][0]["subagent"], json!([]));
    }

    // ---- the disposition-parsing boundary (independent review, 6j6v.r5a2) ---------------------
    //
    // This layer parses the stored `disposition` column into the typed enum, which makes every read
    // that produces a `MessageView` fail-closed on a value it does not know; the CLI's old raw-row
    // version compared the string and swept anything unrecognized into `in_turn`. These two tests
    // pin BOTH halves of that trade-off — that the strict path really is a clean error rather than
    // a panic, and that nothing short of hand-editing the materialized view can reach it. Until
    // 6j6v.r5a2 the claim lived only in a doc comment.
    //
    // The pair was written about `inbox` and `prime`, the two readings 6j6v.4d2z has since removed.
    // The BOUNDARY is unchanged and belongs to the layer rather than to those two callers, so what
    // moved is only which read demonstrates it: `messages`, the channel history an app renders.

    /// An op whose envelope carries a disposition this binary does not know is **not folded** — so
    /// no row with an out-of-range `disposition` can ever materialize from the op log.
    ///
    /// This is the reachability boundary the fail-closed partition rests on, and it is what rules
    /// out the version-skew story: a future replica that invents a third disposition syncs its op
    /// here, `is_foldable` rejects the envelope (serde refuses the unknown variant), and the message
    /// simply never appears — the same forward-compat "store, don't fold" rule every unknown op
    /// follows. `fold_message` then writes the column by re-serializing the typed enum, so a folded
    /// row's `disposition` is `in_turn` or `next_session` by construction.
    #[test]
    fn an_unknown_disposition_op_is_stored_but_never_folded() {
        use nxs_foundation::model::Op;
        use nxs_foundation::reducer::Reducer;

        let envelope = serde_json::json!({
            "origin": "o",
            "channel_id": "c-1",
            "sender": "o/a",
            "kind": "task",
            "priority": "normal",
            "disposition": "some_future_disposition",
            "body": "from a newer replica",
        });
        let op = Op {
            op_id: "op-future".into(),
            lamport: 1,
            site: 2,
            domain: "message".into(),
            target_kind: "message".into(),
            target_id: "m-future".into(),
            field: "envelope".into(),
            op_type: "post".into(),
            value: Some(envelope.to_string()),
            author: "o/a".into(),
            wall_clock: "2026-07-10T10:00:00Z".into(),
            key_id: None,
            sig: None,
        };
        assert!(
            !crate::message_reducer::MessageReducer.is_foldable(&op),
            "an unknown disposition must not fold — otherwise the strict read below IS reachable \
             by sync, and fail-closed would be the wrong trade"
        );
    }

    /// A row hand-corrupted straight into the materialized view — the only way to reach the strict
    /// path at all — surfaces as a clean `io` error, never a panic.
    ///
    /// Fail-closed is the deliberate choice here, not an oversight: the state is unreachable
    /// through the op log (test above) and self-repairing (a refold rebuilds the views from
    /// foldable ops only), so a skip-and-report path would add public surface for a state the
    /// system cannot produce.
    #[test]
    fn a_hand_corrupted_disposition_row_is_a_clean_io_error_not_a_panic() {
        let mut s = seed();
        post(&mut s, "c-1", "hello", Disposition::InTurn);
        s.connection()
            .execute_batch("UPDATE messages SET disposition = 'gibberish';")
            .expect("corrupt the materialized view by hand");

        let err = messages(&s, "c-1", "o/reader").expect_err("the read rejects the row");
        assert_eq!(err.kind, ErrorKind::Io, "a clean io error: {err}");
        assert!(
            err.msg.contains("disposition"),
            "the message names the offending field: {err}"
        );
    }

    // ---- prime rendering: the lifted section renderers (nxf 6j6v.r5a2) -------------------------
    //
    // Four `render_unread_blocks` tests and their `ib` fixture stood here — head layout, the `---`
    // rule between entries, a multi-paragraph body kept intact, an empty body as head only. They
    // went with the renderer (nxf 6j6v.4d2z).

    // ---- render_declared_team / PrimeCommand::render_markdown: callerless since render_markdown
    // stopped using them (nxf h4d3, task 3), tested directly here for the reason
    // `render_unread_blocks` above used to share — public, embedding-host-facing renderers keep
    // direct coverage even after their one-time caller in `render_markdown` leaves, so a change to
    // either still reddens here rather than only in a golden that no longer exercises them (review
    // of task 3, Important 2).

    /// **A declared handle cannot forge a line in a block a host injects verbatim** (review of PR
    /// #465, Integrity #1). `validate_role_handle` rejects `/`, `\\`, `..` and the `__` prefix and
    /// nothing else — and it does not run on the prime path at all — so a backtick or a newline in
    /// a handle reaches this renderer. The ordinary case must stay byte-identical; the hostile one
    /// must not be able to close the code span or start a second bullet.
    #[test]
    fn a_finding_whose_file_could_break_the_markdown_is_rendered_inert() {
        assert_eq!(
            render_finding("coder.yaml", "something"),
            "- `coder.yaml`: something",
            "a well-formed handle renders exactly as it always did"
        );
        for hostile in [
            "co`der.yaml",
            "coder.yaml`\n- `forged.yaml`: not a real finding",
            "coder.yaml\n## Forged Heading",
            "coder\u{2028}.yaml",
        ] {
            let rendered = render_finding(hostile, "something");
            assert!(
                !rendered.contains('`'),
                "no backtick survives to close a span: {rendered:?}"
            );
            assert!(
                !rendered.contains('\n') && !rendered.contains('\u{2028}'),
                "and no line break survives to forge a bullet or a heading: {rendered:?}"
            );
            assert!(
                rendered.starts_with("- ") && rendered.ends_with(": something"),
                "…while still being the one bullet it was: {rendered:?}"
            );
        }
    }

    #[test]
    fn render_declared_team_lists_roles_then_channels() {
        let roster = PrimeRoster {
            roles: vec!["coder".to_string(), "pm".to_string()],
            channels: vec!["standup".to_string()],
            errors: Vec::new(),
            warnings: Vec::new(),
        };
        assert_eq!(
            render_declared_team(&roster),
            "## Declared Team
- **Roles:** coder, pm
- **Channels:** standup"
        );
    }

    #[test]
    fn render_declared_team_omits_a_line_for_an_empty_half() {
        let roles_only = PrimeRoster {
            roles: vec!["pm".to_string()],
            channels: Vec::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
        };
        assert_eq!(
            render_declared_team(&roles_only),
            "## Declared Team
- **Roles:** pm"
        );
        let channels_only = PrimeRoster {
            roles: Vec::new(),
            channels: vec!["standup".to_string()],
            errors: Vec::new(),
            warnings: Vec::new(),
        };
        assert_eq!(
            render_declared_team(&channels_only),
            "## Declared Team
- **Channels:** standup"
        );
    }

    #[test]
    fn render_declared_team_of_an_empty_roster_is_empty() {
        let empty = PrimeRoster {
            roles: Vec::new(),
            channels: Vec::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
        };
        assert_eq!(render_declared_team(&empty), "");
    }

    #[test]
    fn prime_command_render_markdown_joins_invocations_and_appends_the_summary() {
        let single = PrimeCommand {
            invocations: &["nxc list"],
            summary: "who can be addressed here, and what for",
        };
        assert_eq!(
            single.render_markdown(),
            "- `nxc list` — who can be addressed here, and what for"
        );
        // No shipped entry pairs two invocations today (see PRIME_COMMANDS's own doc comment), but
        // the renderer still has to join them correctly if one ever does.
        let paired = PrimeCommand {
            invocations: &["nxc channels", "nxc agents"],
            summary: "retired pair, kept only to exercise the join",
        };
        assert_eq!(
            paired.render_markdown(),
            "- `nxc channels` / `nxc agents` — retired pair, kept only to exercise the join"
        );
    }

    // `render_opener_wake_of_nothing_is_empty` and
    // `render_opener_wake_lays_out_complete_and_stale_sections` stood here, with their `wake_msg`
    // helper. They went with the renderer they pinned (nxf 6j6v.1gm9). What replaced them is not a
    // unit test but a golden: `tests/prime_golden.rs` now asserts that a COMPLETED board adds not
    // one byte to the block, and that the wake never carried the one fact worth keeping — see
    // `PrimeReport::render_markdown` for the whole argument.

    #[test]
    fn owes_reply_and_the_working_tree_re_derivation_stay_one_predicate_across_a_turn() {
        // [`owes_reply`]'s own doc: it is `pub(crate)` so that "what the role is told it owes and
        // what `reply --if-unanswered` will accept as a discharge are one predicate rather than two
        // that can disagree" — the working-tree release re-derives the obligation of a promoted
        // trigger through this exact function, and the release rule asks the same question in bulk
        // through `ChatStore::work_scope_has_outstanding`. 6j6v.cg8g moves that predicate from
        // per-thread to per-turn, and the two halves must move TOGETHER: they share
        // `ChatStore::thread_quorums`, and this is the test that keeps that shared.
        //
        // Both are read at all four points of one turn boundary — asked, answered, asked again,
        // answered again — because a divergence in either direction is a real defect: a lease that
        // never releases, or a working copy handed to a rival while the role is still writing its
        // final report.
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "kind", "group", "o/pm");
        let root = ThreadRoot {
            origin: "o".into(),
            channel_id: "c-1".into(),
            opener: "o/pm".into(),
            created: "2026-07-10T00:00:00Z".into(),
            parent: None,
        };
        s.open_thread("t-1", &root, "o/pm");
        let declare = |s: &mut ChatStore| {
            s.set_expects_reply_from("t-1", "[\"o/coder\"]", "o/pm");
        };
        let answer = |s: &mut ChatStore| {
            s.post_message(&MessageEnvelope {
                origin: "o".into(),
                channel_id: "c-1".into(),
                sender: "o/coder".into(),
                kind: MessageKind::Report,
                priority: Priority::Normal,
                disposition: Disposition::InTurn,
                thread_id: Some("t-1".into()),
                refs: Refs::default(),
                body: "done".into(),
            });
        };
        let both = |s: &ChatStore| -> (bool, bool) {
            (
                owes_reply(s, Some("t-1"), "o", "coder", "2026-07-11T00:00:00Z").unwrap(),
                s.work_scope_has_outstanding(
                    &crate::working_tree::WorkScope::Thread("t-1".to_string()),
                    "2026-07-11T00:00:00Z",
                )
                .unwrap(),
            )
        };

        declare(&mut s);
        assert_eq!(both(&s), (true, true), "turn 1, asked");
        answer(&mut s);
        assert_eq!(both(&s), (false, false), "turn 1, answered");
        declare(&mut s);
        assert_eq!(both(&s), (true, true), "turn 2, asked again");
        answer(&mut s);
        assert_eq!(both(&s), (false, false), "turn 2, answered");
    }

    // ---- the operation status (nxf 6j6v.a71h) --------------------------------------------------

    /// The reader's wall clock — drives only the clock-dependent `stale` flag, which none of these pin.
    const NOW: &str = "2026-08-14T10:00:00Z";

    /// [`super::status`] with a worker that answers nothing, SHADOWING the free function so the
    /// choice is made once instead of on thirty call sites (nxf 6j6v.qmy6).
    ///
    /// `DryWorker`'s `session_is_running` and `answers_liveness` are both the trait's `false`
    /// default, so every unended session below reads `unknown` — which is what these tests want:
    /// none of them is about the PROCESS half, and a worker that answered would make the ones about
    /// the tree depend on a fact they do not set. The process half is driven from the SEAM, where a
    /// host supplies its own worker, in `tests/a_session_you_can_ask_about.rs`.
    fn status(store: &ChatStore, now: &str, scope: StatusScope<'_>) -> Result<StatusReport> {
        super::status(store, &crate::worker::DryWorker { log: None }, now, scope)
    }

    /// A thread `open` op arriving from a PEER, carrying whatever `parent` it claims — the only way
    /// a malformed or not-yet-resolvable edge can reach this store, since every local write goes
    /// through `orchestration::open_child_thread` and can only name a thread that already exists.
    fn foreign_thread(s: &mut ChatStore, thread_id: &str, parent: Option<&str>) {
        // Its own coordinate, taken off the log. `(lamport, site)` identifies at most one op
        // (6j6v.fc5p), so stamping every forged thread `(7, 9)` — as this helper used to — describes
        // a peer that minted the same number for each of them, which no correct replica can do; the
        // log now refuses all but the first. Ascending numbers also match the arrival order these
        // tests are about (a child before its parent, a cycle's two halves).
        let lamport: i64 = s
            .connection()
            .query_row("SELECT COALESCE(MAX(lamport), 0) + 1 FROM ops", [], |r| {
                r.get(0)
            })
            .unwrap();
        s.apply(&[nxs_foundation::model::Op {
            op_id: format!("01KZTHREADTREE{thread_id:0>10}"),
            lamport,
            site: 9,
            domain: crate::model::DOMAIN_MESSAGE.into(),
            target_kind: crate::model::KIND_THREAD.into(),
            target_id: thread_id.into(),
            field: crate::model::FIELD_ROOT.into(),
            op_type: crate::model::OP_OPEN.into(),
            value: Some(
                serde_json::to_string(&ThreadRoot {
                    origin: "o".into(),
                    channel_id: "c-1".into(),
                    opener: "o/a".into(),
                    created: "2026-08-14T10:00:00Z".into(),
                    parent: parent.map(str::to_string),
                })
                .unwrap(),
            ),
            author: "o/a".into(),
            wall_clock: "2026-08-14T10:00:00Z".into(),
            key_id: None,
            sig: None,
        }]);
    }

    #[test]
    fn the_seam_reports_the_sub_round_a_waiting_party_commissioned_itself() {
        // **AT THE SEAM, not only through `nxc`** (nxf 6j6v.hw2t, review of PR #460 · Test Quality
        // #2). This field and `ChatStore::thread_children` under it are public surface: an embedding
        // host reads `Engine::status` and never runs the binary, so black-box coverage alone leaves
        // the contract it compiles against untested. This is that test — and it is also where the
        // ORDER is pinned, which the CLI cannot show: the bulk quorum read comes back channel-first,
        // so the ids must be sorted here or the two surfaces name a different "first" round.
        let mut s = seed();
        s.set_wall_clock(NOW);
        s.open_thread(
            "t-root",
            &ThreadRoot {
                origin: "o".into(),
                channel_id: "c-1".into(),
                opener: "o/human".into(),
                created: NOW.into(),
                parent: None,
            },
            "o/human",
        );
        s.set_expects_reply_from("t-root", r#"["o/marketing"]"#, "o/human");
        // Two consultations the marketing role opened out of its own thread, and one a bystander
        // opened in the same tree. Minted out of id order on purpose.
        for (id, opener) in [
            ("t-zz", "o/marketing"),
            ("t-aa", "o/marketing"),
            ("t-by", "o/bystander"),
        ] {
            s.open_thread(
                id,
                &ThreadRoot {
                    origin: "o".into(),
                    channel_id: "c-1".into(),
                    opener: opener.into(),
                    created: NOW.into(),
                    parent: Some("t-root".into()),
                },
                opener,
            );
            s.set_expects_reply_from(id, r#"["o/specialist"]"#, opener);
        }

        assert_eq!(
            s.thread_children("t-root").unwrap(),
            vec!["t-aa".to_string(), "t-by".to_string(), "t-zz".to_string()],
            "the store read returns every child, thread-id ordered"
        );

        let report = status(&s, NOW, StatusScope::Threads(&["t-root"])).unwrap();
        let root = report.operations[0]
            .threads
            .iter()
            .find(|t| t.thread_id == "t-root")
            .expect("the root is on the report");
        assert_eq!(
            root.waiting_on_sub_round,
            vec!["t-aa".to_string(), "t-zz".to_string()],
            "…and the field carries the two the WAITED-FOR party opened, oldest first — never the \
             bystander's, which hangs in the same tree and says nothing about this answer: {root:?}"
        );
        assert_eq!(
            root.state,
            ThreadState::Open,
            "non-empty implies open, which is the predicate's own first term"
        );
        assert!(
            !report.operations[0].needs_decision,
            "and waiting is not a stopped chain: {:?}",
            report.operations[0]
        );

        // The bystander's own thread commissioned nothing, so it carries nothing — the field's
        // absence is as load-bearing as its presence.
        let bystander = report.operations[0]
            .threads
            .iter()
            .find(|t| t.thread_id == "t-by")
            .expect("the bystander's thread is on the report");
        assert!(bystander.waiting_on_sub_round.is_empty(), "{bystander:?}");
    }

    #[test]
    fn the_bounded_and_the_workspace_wide_quorum_reads_agree_about_the_same_threads() {
        // The property the new `reply` write point's ceiling rests on (nxf 6j6v.hw2t, review of
        // PR #460 · Integrity #3): past `MAX_BOUND_IDS` it asks `thread_quorums_all` and filters to
        // the ids it wanted, because binding one SQL variable per child would start failing at
        // SQLite's 999. That fallback is only correct if the two reads say the SAME thing about a
        // thread — and the filter is not optional, which is the half worth pinning: the
        // workspace-wide read answers for threads nobody asked about, and the predicate downstream
        // weighs an `opener`, not a parent.
        let mut s = seed();
        s.set_wall_clock(NOW);
        for (id, opener) in [("t-1", "o/a"), ("t-2", "o/b"), ("t-elsewhere", "o/c")] {
            s.open_thread(
                id,
                &ThreadRoot {
                    origin: "o".into(),
                    channel_id: "c-1".into(),
                    opener: opener.into(),
                    created: NOW.into(),
                    parent: None,
                },
                opener,
            );
            s.set_expects_reply_from(id, r#"["o/reader"]"#, opener);
        }

        let wanted = ["t-1", "t-2"];
        let bound = s.thread_quorums(&wanted, NOW).unwrap();
        let whole: Vec<_> = s
            .thread_quorums_all(NOW)
            .unwrap()
            .into_iter()
            .filter(|q| wanted.contains(&q.thread_id.as_str()))
            .collect();
        assert_eq!(
            bound, whole,
            "the two branches of the ceiling answer identically"
        );
        assert!(
            s.thread_quorums_all(NOW).unwrap().len() > bound.len(),
            "…and the unfiltered read really does carry threads nobody asked about, which is what \
             makes the filter load-bearing rather than tidy"
        );
    }

    #[test]
    fn a_thread_whose_parent_has_not_folded_yet_reads_as_a_root_of_its_own() {
        // Ops arrive in RELAY order, so a child's `open` can land before its parent's. Showing the
        // child as its own operation until the parent turns up is one operation too many — the
        // harmless direction; the alternative would be to drop it from every view until then.
        let mut s = seed();
        foreign_thread(&mut s, "t-child", Some("t-parent-not-here-yet"));
        let report = status(&s, NOW, StatusScope::Workspace).unwrap();
        // It owes nobody anything, so it is not LIVE and the workspace listing stays quiet…
        assert!(report.operations.is_empty(), "{report:?}");
        // …but it is reachable, whole, by name, and it says what it claims its parent is. The
        // GROUPING roots it here; the reported edge is the one the op actually carries, so a reader
        // can tell "top of a chain" from "its parent has not arrived yet".
        let report = status(&s, NOW, StatusScope::Threads(&["t-child"])).unwrap();
        assert_eq!(report.operations.len(), 1);
        assert_eq!(report.operations[0].root, "t-child");
        assert_eq!(
            report.operations[0].threads[0].parent.as_deref(),
            Some("t-parent-not-here-yet"),
            "grouped as a root, but it does not CLAIM to be one"
        );

        // And the parent arriving afterwards joins them into ONE operation, with no refold and no
        // repair step: the edge was on the child all along.
        foreign_thread(&mut s, "t-parent-not-here-yet", None);
        let report = status(&s, NOW, StatusScope::Threads(&["t-child"])).unwrap();
        assert_eq!(report.operations[0].root, "t-parent-not-here-yet");
        assert_eq!(
            report.operations[0]
                .threads
                .iter()
                .map(|t| t.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["t-parent-not-here-yet", "t-child"]
        );
    }

    #[test]
    fn a_cycle_in_the_parent_edges_terminates_instead_of_hanging_the_read() {
        // No write here can produce this — a parent always exists before its child — but a peer's op
        // log is not something this store gets to vouch for, and a walk that trusted it would spin
        // (upwards) or recurse until the stack ran out (downwards). Both directions are guarded, and
        // the answer is the same whichever member of the cycle is asked about.
        let mut s = seed();
        foreign_thread(&mut s, "t-a", Some("t-b"));
        foreign_thread(&mut s, "t-b", Some("t-a"));
        let from_a = status(&s, NOW, StatusScope::Threads(&["t-a"])).unwrap();
        let from_b = status(&s, NOW, StatusScope::Threads(&["t-b"])).unwrap();
        assert_eq!(from_a, from_b, "one cycle, one answer");
        assert_eq!(from_a.operations.len(), 1);
        assert_eq!(
            from_a.operations[0]
                .threads
                .iter()
                .map(|t| t.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["t-a", "t-b"],
            "each thread once: {from_a:?}"
        );
    }

    #[test]
    fn a_thread_that_names_itself_as_its_parent_groups_as_a_root_but_never_claims_to_be_one() {
        // The degenerate one-node cycle. The GROUPING drops it where the edge map is built, so the
        // thread is its own operation rather than an infinite walk — but the record still reports the
        // edge the op actually carries, and the root's carve-out is refused on the strength of it
        // (re-review N5: this test's old name and comment said "a self-edge is no edge", which is
        // true of the grouping and false of the record it produces).
        let mut s = seed();
        foreign_thread(&mut s, "t-self", Some("t-self"));
        let report = status(&s, NOW, StatusScope::Threads(&["t-self"])).unwrap();
        assert_eq!(report.operations[0].root, "t-self", "its own operation");
        assert_eq!(
            report.operations[0].threads[0].parent.as_deref(),
            Some("t-self"),
            "…and the nonsense edge is reported verbatim, not quietly erased"
        );
        assert!(
            !report.operations[0].threads[0].awaiting_human,
            "so a self-edge never earns the root's carve-out either"
        );
    }

    // ---- the sharpened dead end (nxf 6j6v.93zd) ------------------------------------------------

    /// Open `id` (optionally under `parent`) and declare `expects` on it — the two writes every
    /// thread in this section is built from.
    fn thread_expecting(s: &mut ChatStore, id: &str, parent: Option<&str>, expects: &str) {
        s.set_wall_clock(NOW);
        s.open_thread(
            id,
            &ThreadRoot {
                origin: "o".into(),
                channel_id: "c-1".into(),
                opener: "o/a".into(),
                created: NOW.into(),
                parent: parent.map(str::to_string),
            },
            "o/a",
        );
        if !expects.is_empty() {
            s.set_expects_reply_from(id, expects, "o/a");
        }
    }

    /// `sender` says something in `thread`. Its `(lamport, site)` is what the discriminator compares.
    fn say_in(s: &mut ChatStore, thread: &str, sender: &str) {
        s.set_wall_clock(NOW);
        s.post_message(&MessageEnvelope {
            origin: "o".into(),
            channel_id: "c-1".into(),
            sender: sender.into(),
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some(thread.into()),
            refs: Refs::default(),
            body: "…".into(),
        });
    }

    fn state_of(s: &ChatStore, root: &str, id: &str) -> ThreadState {
        status(s, NOW, StatusScope::Threads(&[root]))
            .unwrap()
            .operations
            .remove(0)
            .threads
            .into_iter()
            .find(|t| t.thread_id == id)
            .unwrap_or_else(|| panic!("no {id} in the tree of {root}"))
            .state
    }

    #[test]
    fn a_dead_end_reads_orphaned_only_while_its_parent_has_not_moved_on() {
        // The false positive a71h reported and left to this item: a FINISHED branch and a FAILED one
        // both satisfy "discharged, no child, parent owes nothing". What separates them is whether
        // the parent moved AFTER the child was discharged — in a71h §2's chain literally the
        // supervisor's consolidating reply.
        let mut s = seed();
        thread_expecting(&mut s, "t-p", None, r#"["o/mid"]"#);
        thread_expecting(&mut s, "t-c", Some("t-p"), r#"["o/leaf"]"#);

        // The parent is discharged FIRST, then the child answers. Nothing upstream ever looked at
        // that answer: a genuine dead end.
        say_in(&mut s, "t-p", "o/mid");
        say_in(&mut s, "t-c", "o/leaf");
        assert_eq!(
            state_of(&s, "t-p", "t-c"),
            ThreadState::Orphaned,
            "discharged, childless, and the parent settled BEFORE this answer arrived"
        );

        // …and the same tree the moment the parent says something after it. Nothing else changed.
        say_in(&mut s, "t-p", "o/mid");
        assert_eq!(
            state_of(&s, "t-p", "t-c"),
            ThreadState::Answered,
            "the parent moved on from it, so this is a branch that FINISHED"
        );
    }

    #[test]
    fn a_sibling_opened_after_the_discharge_counts_as_the_parent_moving_on() {
        // The second half of the discriminator: a parent can consume an answer by COMMISSIONING the
        // next step instead of replying, and then it says nothing itself. "Opened after" is read as
        // "that sibling has messages and every one of them is later", since a thread's `open` op
        // carries no causal position of its own.
        let mut s = seed();
        thread_expecting(&mut s, "t-p", None, r#"["o/mid"]"#);
        thread_expecting(&mut s, "t-c", Some("t-p"), r#"["o/leaf"]"#);
        say_in(&mut s, "t-p", "o/mid");
        say_in(&mut s, "t-c", "o/leaf");
        assert_eq!(state_of(&s, "t-p", "t-c"), ThreadState::Orphaned);

        thread_expecting(&mut s, "t-next", Some("t-p"), r#"["o/other"]"#);
        say_in(&mut s, "t-next", "o/mid");
        assert_eq!(
            state_of(&s, "t-p", "t-c"),
            ThreadState::Answered,
            "the next step was commissioned out of the parent after this branch answered"
        );
    }

    #[test]
    fn a_parent_that_never_asked_anyone_anything_orphans_nothing_below_it() {
        // The second false-positive class (a71h's review, unreported by its implementer):
        // "no open parent" was `parent.outstanding.is_empty()`, which is just as true of a parent
        // that never asked ANYONE — an ordinary conversation, a plain substrate post. A dead end
        // below one of those is not a coordination that broke down.
        let mut s = seed();
        thread_expecting(&mut s, "t-p", None, "");
        thread_expecting(&mut s, "t-c", Some("t-p"), r#"["o/leaf"]"#);
        say_in(&mut s, "t-c", "o/leaf");
        assert_eq!(
            state_of(&s, "t-p", "t-c"),
            ThreadState::Answered,
            "nobody upstream was ever waiting for this, so nothing failed"
        );
    }

    #[test]
    fn the_assignee_session_prefers_the_silent_party_and_falls_back_to_the_one_that_answered() {
        // nxf 6j6v.1q6d, the two branches and the guard between them, at the layer that decides.
        // Branch 1 is the item's own case: an OPEN thread has, by definition, no message from its
        // assignee to read a return address off, so the edge comes from the position the TRIGGER
        // recorded (`session_map.thread` — an existing column with an existing write path, added by
        // a71h §3.1 for the parent rule; this is a read of it, not a second write).
        let mut s = seed();
        s.set_wall_clock(NOW);
        let root = ThreadRoot {
            origin: "o".into(),
            channel_id: "c-1".into(),
            opener: "o/a".into(),
            created: NOW.into(),
            parent: None,
        };
        s.open_thread("t-1", &root, "o/a");
        s.set_expects_reply_from("t-1", r#"["o/reader"]"#, "o/a");
        s.create_pending_session("s-reader", "reader").unwrap();
        s.record_session_thread("s-reader", Some("t-1")).unwrap();

        let edge = |s: &ChatStore| -> Option<String> {
            s.thread_assignee_sessions()
                .unwrap()
                .into_iter()
                .find(|(t, _)| t == "t-1")
                .map(|(_, sess)| sess)
        };
        assert_eq!(
            edge(&s).as_deref(),
            Some("s-reader"),
            "silent, owing, and reachable — the whole point of the item"
        );
        // The scoped read answers identically; only the filter differs.
        assert_eq!(
            s.thread_assignee_sessions_for(&["t-1"]).unwrap(),
            vec![("t-1".to_string(), "s-reader".to_string())]
        );

        // Branch 2: once every expected handle HAS spoken, the guard shuts branch 1 off and the
        // return address of the newest answer takes over. Here that answer came from a DIFFERENT
        // session than the one standing on the thread, so the two branches are distinguishable.
        s.post_message(&MessageEnvelope {
            origin: "o".into(),
            channel_id: "c-1".into(),
            sender: "o/reader".into(),
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("t-1".into()),
            refs: Refs {
                session_id: Some("s-that-answered".into()),
                ..Refs::default()
            },
            body: "done".into(),
        });
        assert_eq!(
            edge(&s).as_deref(),
            Some("s-that-answered"),
            "a settled thread names who ANSWERED, not whoever is standing on it now"
        );
    }

    /// Post `body` into `t-1` as `sender`, stamped with `session` as its return address.
    fn post_in(s: &mut ChatStore, sender: &str, session: &str) {
        s.set_wall_clock(NOW);
        s.post_message(&MessageEnvelope {
            origin: "o".into(),
            channel_id: "c-1".into(),
            sender: sender.into(),
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("t-1".into()),
            refs: Refs {
                session_id: Some(session.into()),
                ..Refs::default()
            },
            body: "done".into(),
        });
    }

    /// A two-handle board on `t-1`, with a session standing on it per member.
    fn seed_board(s: &mut ChatStore) {
        s.set_wall_clock(NOW);
        s.open_thread(
            "t-1",
            &ThreadRoot {
                origin: "o".into(),
                channel_id: "c-1".into(),
                opener: "o/a".into(),
                created: NOW.into(),
                parent: None,
            },
            "o/a",
        );
        s.set_expects_reply_from("t-1", r#"["o/one","o/two"]"#, "o/a");
        // `s-2` is the YOUNGER session, so the `ORDER BY sm.created DESC, sm.internal_id DESC`
        // tie-break prefers it — which is what makes the exclusion below provable rather than
        // accidentally satisfied by the ordering (re-review N4).
        for (id, role) in [("s-1", "one"), ("s-2", "two")] {
            s.create_pending_session(id, role).unwrap();
            s.record_session_thread(id, Some("t-1")).unwrap();
        }
    }

    fn edge_of(s: &ChatStore, thread: &str) -> Option<String> {
        s.thread_assignee_sessions()
            .unwrap()
            .into_iter()
            .find(|(t, _)| t == thread)
            .map(|(_, sess)| sess)
    }

    #[test]
    fn a_session_that_has_already_answered_this_turn_is_never_named_as_the_one_still_working() {
        // Re-review N4: the "has itself posted nothing" clause was deletable with every test still
        // green, because in the fixtures the silent member happened to be the YOUNGER session and so
        // won the tie-break anyway. Here the member that ANSWERED is the younger one, so the ordering
        // actively pulls the wrong way and only the clause can produce the right answer. Delete
        // `AND NOT EXISTS(… json_extract(m3.refs …) …)` from `assignee_sql` and this goes red.
        let mut s = seed();
        seed_board(&mut s);
        post_in(&mut s, "o/two", "s-2");
        assert_eq!(
            edge_of(&s, "t-1").as_deref(),
            Some("s-1"),
            "the one still working, not the younger session that already answered"
        );
    }

    #[test]
    fn a_session_resumed_into_a_settled_thread_does_not_displace_the_one_that_answered() {
        // Review Test Quality #1. `assignee_sql` has TWO guards between its branches — the outer
        // `CASE WHEN EXISTS(… m2 …)` ("is anybody here still owed an answer?") and branch 1's inner
        // `NOT EXISTS(… m3 …)` ("has this session itself spoken?") — and until now every fixture
        // that settled a thread had exactly ONE session standing on it: the one that answered. There
        // the inner guard excludes it too, so a BROKEN outer guard produced byte-identical output
        // and nothing could tell the two apart. `thread_tree.rs`'s member-thread test is that shape,
        // by construction: since 6j6v.pf6j a member thread has exactly one assignee.
        //
        // This is the residual `assignee_sql`'s own doc names — "a supervisor resumed into a settled
        // thread must not displace it" — and the one scene where the two guards DISAGREE: a second
        // session stands on the settled thread and has never spoken, so the inner guard lets it
        // through and only the outer guard keeps it out.
        //
        // MUTATION: drop the outer `CASE WHEN … THEN`/`END` from `assignee_sql`, leaving its
        // `THEN` arm as the first `COALESCE` arm, and this goes red naming `s-sup`.
        let mut s = seed();
        s.set_wall_clock(NOW);
        s.open_thread(
            "t-1",
            &ThreadRoot {
                origin: "o".into(),
                channel_id: "c-1".into(),
                opener: "o/a".into(),
                created: NOW.into(),
                parent: None,
            },
            "o/a",
        );
        s.set_expects_reply_from("t-1", r#"["o/one"]"#, "o/a");
        // The session put to work here, plus a supervisor resumed into the SAME thread that posts
        // nothing — the second session the older fixtures never had. `s-sup` sorts ahead of `s-1` on
        // branch 1's own `ORDER BY sm.created DESC, sm.internal_id DESC` (same `created`, later id),
        // so the tie-break actively pulls toward the wrong answer and only the guard can produce the
        // right one.
        for (id, role) in [("s-1", "one"), ("s-sup", "supervisor")] {
            s.create_pending_session(id, role).unwrap();
            s.record_session_thread(id, Some("t-1")).unwrap();
        }
        post_in(&mut s, "o/one", "s-1");

        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert!(q.complete, "every expected handle has answered: {q:?}");
        assert_eq!(
            edge_of(&s, "t-1").as_deref(),
            Some("s-1"),
            "the session that ANSWERED, not the silent one that happens to be standing here"
        );
    }

    #[test]
    fn on_the_second_turn_the_session_and_the_outstanding_handle_name_the_same_side() {
        // Re-review N1, the defect this round exists for. The guard used to ask "has this handle ever
        // spoken HERE", a thread-LIFETIME question, while everything printed beside it has been
        // turn-scoped since 6j6v.cg8g. On turn two of a board — both answered turn one, the
        // expectation re-declared, one answers again, the other silent — the lifetime guard read
        // "everyone has spoken at some point", shut the silent branch off, and let the field name the
        // handle that had ALREADY answered while `outstanding` named the other one, in adjacent
        // fields of the same object.
        let mut s = seed();
        seed_board(&mut s);
        // Turn one: both answer.
        post_in(&mut s, "o/one", "s-1");
        post_in(&mut s, "o/two", "s-2");
        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert!(q.complete, "turn one is settled: {q:?}");
        assert_eq!(
            edge_of(&s, "t-1").as_deref(),
            Some("s-2"),
            "a settled thread names who answered LAST"
        );

        // Turn two: the same expectation re-declared (what a supervisor does to open the next turn),
        // then `o/two` answers again and `o/one` stays silent.
        s.set_expects_reply_from("t-1", r#"["o/one","o/two"]"#, "o/a");
        post_in(&mut s, "o/two", "s-2");

        let q = s.thread_quorum("t-1", NOW).unwrap().unwrap();
        assert_eq!(q.outstanding, vec!["o/one".to_string()], "{q:?}");
        assert_eq!(
            edge_of(&s, "t-1").as_deref(),
            Some("s-1"),
            "the session and the outstanding handle name the SAME side"
        );

        // The two fields agree in the record a consumer actually reads, which is where the defect
        // was visible: `session` and `outstanding` sat next to each other and disagreed.
        let t = &status(&s, NOW, StatusScope::Threads(&["t-1"]))
            .unwrap()
            .operations[0]
            .threads[0];
        assert_eq!(t.outstanding, vec!["o/one".to_string()]);
        assert_eq!(t.session.as_deref(), Some("s-1"), "{t:?}");
        assert_eq!(t.state, ThreadState::Open);
    }

    #[test]
    fn a_thread_nobody_was_put_to_work_on_and_nobody_answered_has_no_session_edge() {
        // The only `None` left: no trigger recorded a position here on THIS device (the table is
        // device-local, like the transcript the edge leads to) and nobody expected has posted.
        let mut s = seed();
        s.set_wall_clock(NOW);
        s.open_thread(
            "t-quiet",
            &ThreadRoot {
                origin: "o".into(),
                channel_id: "c-1".into(),
                opener: "o/a".into(),
                created: NOW.into(),
                parent: None,
            },
            "o/a",
        );
        s.set_expects_reply_from("t-quiet", r#"["o/reader"]"#, "o/a");
        assert!(s
            .thread_assignee_sessions()
            .unwrap()
            .iter()
            .all(|(t, _)| t != "t-quiet"));
    }

    #[test]
    fn a_provisionally_rooted_thread_never_tells_a_human_the_chain_is_theirs() {
        // Review finding F7. The GROUPING treats a thread whose parent has not folded yet as a root
        // of its own — one operation too many, the harmless direction. `awaiting_human` is the one
        // field where that direction is NOT harmless: §3.4's entire purpose is telling "the human
        // has it" apart from "nothing is happening", and rendering `awaiting you` for a mid-chain
        // sub-branch on a peer says the opposite of the truth. The flag therefore reads the DECLARED
        // edge, not the normalized one.
        let mut s = seed();
        foreign_thread(&mut s, "t-mid", Some("t-parent-still-on-a-peer"));
        // …with its expectation declared AND discharged, which is exactly the shape that earns a
        // real root its `awaiting_human`.
        s.set_expects_reply_from("t-mid", r#"["o/reader"]"#, "o/a");
        s.set_wall_clock(NOW);
        s.post_message(&MessageEnvelope {
            origin: "o".into(),
            channel_id: "c-1".into(),
            sender: "o/reader".into(),
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("t-mid".into()),
            refs: Refs::default(),
            body: "done".into(),
        });
        let t = &status(&s, NOW, StatusScope::Threads(&["t-mid"]))
            .unwrap()
            .operations[0]
            .threads[0];
        assert!(
            t.outstanding.is_empty(),
            "the expectation is discharged: {t:?}"
        );
        assert!(
            !t.awaiting_human,
            "it names a parent, so it is not the top of anything: {t:?}"
        );
        assert_eq!(t.state, ThreadState::Answered);
    }

    #[test]
    fn a_tail_into_a_cycle_lands_in_the_same_operation_as_the_cycle() {
        // Review finding F3. `min(everything visited)` answered `t-aaa` from `t-aaa` and `t-mmm`
        // from `t-mmm`, so the tail was emitted BOTH as its own operation and as a child inside the
        // cycle's tree. The min is over the cycle itself, so every node on or below it agrees.
        let mut s = seed();
        foreign_thread(&mut s, "t-mmm", Some("t-nnn")); // the cycle
        foreign_thread(&mut s, "t-nnn", Some("t-mmm"));
        foreign_thread(&mut s, "t-aaa", Some("t-mmm")); // the tail, with the SMALLEST id
        let from_tail = status(&s, NOW, StatusScope::Threads(&["t-aaa"])).unwrap();
        let from_cycle = status(&s, NOW, StatusScope::Threads(&["t-nnn"])).unwrap();
        assert_eq!(from_tail, from_cycle, "one operation, one answer");
        let ids: Vec<&str> = from_tail.operations[0]
            .threads
            .iter()
            .map(|t| t.thread_id.as_str())
            .collect();
        assert_eq!(ids, vec!["t-mmm", "t-aaa", "t-nnn"], "each exactly once");
    }

    #[test]
    fn the_status_of_an_unknown_thread_is_not_found_rather_than_an_empty_answer() {
        let s = seed();
        let err = status(&s, NOW, StatusScope::Threads(&["t-nope"])).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound, "{err:?}");
    }
}
