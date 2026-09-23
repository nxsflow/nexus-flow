//! The working-tree lease (nxf epic 6j6v.bqe0): device-local coordination so only one CHAIN of
//! triggered role sessions — not one session — holds exclusive use of the repo's working copy and
//! build directory at a time.
//!
//! **Ticket 2 (6j6v.3wsm) added the module skeleton and [`WorkScope`]/[`resolve_scope`]** — the
//! pure function that decides which claim area a trigger belongs to. **Ticket 3 (6j6v.wdjg) added
//! the lease table** and the `acquire`/`holder` methods (it added a standalone `release` too;
//! ticket 10 folded that away — see `release_lease_row` for why the only release left is the one
//! that also advances the queue). **Ticket 4 (6j6v.m3d5) added the
//! queue** — [`QueuedTrigger`] and the `enqueue`/`peek`/`take`/`position`/`len` methods — for
//! triggers that did NOT win the lease: they wait rather than vanish. **Ticket 9 (6j6v.303b) wired
//! the acquire** into `orchestration::trigger_role`, and **ticket 10 (6j6v.fe0f) the release**:
//! [`ChatStore::work_scope_has_outstanding`] — "does this claim area still owe anybody a reply?",
//! the deterministic done signal the whole epic is built on — and
//! [`ChatStore::release_working_tree_and_take_next`], which gives the lease back and hands the
//! queue's next CLAIM AREA — the head plus every entry sharing its scope, see [`take_queue_scope`] —
//! to the caller in one write transaction. **Ticket 6j6v.yd4w made that hand-off complete in both
//! directions**: the same transaction now also GRANTS the lease to the area it promotes (see
//! [`hand_the_working_tree_on`]), so the working copy is never free while somebody is waiting for
//! it; and [`ChatStore::reclaim_expired_working_tree_and_take_next`] lets the queue take an EXPIRED
//! lease away from a chain that is not coming back, so a newcomer's own acquire no longer overtakes
//! a chain that has been standing in line. Who CALLS them stays in
//! `orchestration.rs`, on the reply path; this module owns the mechanics, following the pattern
//! [`crate::consolidation_claim`] and [`crate::member_deadline`] already set: a dedicated module
//! per device-local table rather than another `impl ChatStore` block folded into `store.rs`.
//!
//! **Why the lease is held by a scope, not a session.** A session-scoped lock releases the moment
//! the session that took it exits — and exactly that gap is where the next triggered session slips
//! in, before review, mitigation and merge of the first have even happened. The lease instead has to
//! be held by the CHAIN a trigger belongs to (a run, or a thread of replies), so it stays held
//! across every session in that chain until the chain itself produces a deterministic done signal —
//! not until any one session happens to go quiet.
//!
//! **Ticket 6j6v.1xw1 settled what that chain IS and what the done signal SAYS**, and both answers
//! moved. The chain is a SUBTREE (see [`ChatStore::work_scope_threads`]), so the protection stands
//! while `#coding` waits on `#review`. The signal is "the claim thread is
//! CLOSED" rather than "`expects_reply_from` has cleared": an escalating reply, and an open
//! question, clear the register exactly like a finished one while the task is still mid-flight
//! ([`ChatStore::work_scope_handed_back`]). WHETHER a chain is protected at all moved too, one step
//! up: a channel counts as `exclusive` when a declared MEMBER of it does
//! (`Definitions::channel_needs_working_tree`), one hop and not transitive.
//!
//! **Ticket 6j6v.8y6t re-anchored that subtree at the OPERATION** (owner decision (a), 2026-08-23).
//! The claim still arises only at the first `exclusive` step and the one-hop derivation above is
//! untouched — what changed is where the area starts: the root of the thread tree the work belongs
//! to ([`ChatStore::thread_root`], where "operation" is defined), so one operation competes under
//! one key and gives the checkout back when the OPERATION is finished rather than when the branch
//! its exclusive step started in is. With it, the lease's flat two-hour bound stopped being every
//! lease's bound: a lease now runs to the last window its own area declared, and reaches
//! [`WORKING_TREE_LEASE_BOUND`] only where something in the area declared none.
//!
//! **Ticket 6j6v.de9s gave the hold a second way to end, and it is the first one that is not a
//! release.** Until it, a claim area left the working copy in exactly two ways: it finished (the
//! deterministic done signal above), or its bound ran out and the sweep reclaimed it. An escalation
//! is neither — question (3) of `orchestration::release_working_tree_if_scope_is_done` refuses to
//! let a handed-back area go, correctly, because the task is mid-flight and travelling upward to a
//! human — so a question nobody answered held this device's checkout with no session, no process and
//! no bound (6j6v.fabb, measured). The third way is PARKING: when another operation is waiting and
//! the escalation has gone unanswered for [`crate::park::PARK_AFTER`], the stranded operation's work
//! is committed to a branch, the tree goes back to its base, and the copy is handed on by the same
//! `release_and_fire` every other hand-off uses. Nothing about the LEASE's own rules changed — what
//! changed is that a hold nobody can end is no longer one of the shapes this module can be left in.
//! [`crate::park`] carries the mechanics and every decision behind them; here it shows up as
//! `working_tree_lease.contended_since` and [`ChatStore::threads_await_an_unanswered_hand_back`].
//!
//! **Why `WorkScope` is an opaque identifier, not a hardcoded lifecycle.** [`resolve_scope`] just
//! picks the most specific handle a trigger already carries — its run id, else its thread id, else
//! its session id — and wraps it in a key. It does not know or care what a "run" or a "thread" IS;
//! when a future trigger path resolves to `run:` instead of `thread:` (nxf 6j6v.hq71, `send --to
//! <channel>` coming to initiate a run), the lease and queue mechanics that consume the key do not
//! change at all. That is the entire reason this is a function instead of a match sprinkled across
//! every call site that currently triggers a role.
//!
//! **Why the tables this module will grow are device-local, NOT folded into the op-log** — the same
//! reasoning [`crate::consolidation_claim`] and [`crate::member_deadline`] give for their own
//! tables, restated for this one. A lease is a claim on files sitting on ONE machine's disk: the working copy itself,
//! and whatever build artifacts live under it. A synced replica has no such files — its own working
//! copy, if it even has one, is a physically different checkout on physically different disk.
//! Folding "chain X holds the lease" into the CRDT op log would let a remote device believe it is
//! coordinating access to a directory it has never written to and could not release even if it
//! wanted to — a claim that means something only on the device that took it. The op log stays the
//! record of what happened in the shared history (messages, run status); which chain currently has
//! exclusive hands on THIS disk's working copy is a fact about this disk alone. The acquire itself
//! also has to be a single-statement compare-and-swap for the reason [`crate::consolidation_claim`]
//! spells out in full: two `nxc` processes on the same machine must not be able to interleave
//! between reading "is it free" and writing "I have it".

use std::collections::BTreeSet;

use rusqlite::{params, Connection, OptionalExtension, Row};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::error::{NxfError, Result};
use crate::model::Priority;
use crate::role::Model;
use crate::store::ChatStore;

/// How long a working-tree lease stays live before ANOTHER chain is allowed to reclaim it on its
/// own next acquire attempt (nxf 6j6v.303b), **for a claim area that declared no window of its own**
/// (nxf 6j6v.8y6t).
///
/// **It is no longer every lease's bound, and that is this constant's biggest change** (owner,
/// 2026-08-23: *"Die 2-Stunden-Grenze ergibt keinen Sinn. Sie muss an den Timeouts der einzelnen
/// Aktionen hängen."*). A lease now runs to the last window its own area declared —
/// [`crate::orchestration::operation_lease_expiry`] over
/// [`ChatStore::declared_window_end`] — and reaches this value only where there is nothing to derive
/// from: an area in which some outstanding obligation carries no deadline at any level (no channel
/// `timeout:`, no thread `--deadline`). Everything below is therefore the reasoning for THAT case,
/// which is the case it was always really about.
///
/// **What that fixes, in the two directions this doc has always weighed.** A declared `timeout: 6h`
/// round now holds a 6h lease instead of losing the checkout under itself at two hours — the
/// residual named at the bottom of this doc, closed. And an operation whose steps declare minutes is
/// reclaimed in minutes instead of wedging this device for two hours after a hard death. The second
/// direction is only safe because the acquire path now asks the sweep's own question before taking
/// an expired lease over — [`crate::orchestration::expired_but_still_writing`]: a chain whose
/// process is still alive keeps the checkout however long its clock says.
///
/// **It is no longer the only backstop either** (nxf 6j6v.de9s). A lease held by an operation that
/// escalated and is waiting on a human now ends at [`crate::park::PARK_AFTER`] from the moment
/// somebody else starts waiting — usually far sooner than this value, and by SAVING the work rather
/// than reclaiming a tree with it still in it. This bound stays exactly what the paragraph below
/// says it is: what happens when nothing else ran at all.
///
/// **A backstop for a hard death, not a budget** — the same distinction
/// [`crate::worker::CUSTOM_WORKER_BOUND`] draws at the worker seam and `timer.rs`'s
/// `AT_SUBPROCESS_BOUND` draws at the `at` seam, restated for a bound that is wall-clock rather than
/// a wait. Nothing in the normal life of a chain consults this value: the lease is meant to be given
/// up by the deterministic done signal this epic exists to build — the last `expects_reply_from` on
/// the chain clearing, which `agent-sidecar`'s teardown discharges unconditionally (nxf 6j6v.7e9d),
/// even for a session that failed. This bound is only what happens when even THAT never runs: the
/// machine loses power, the process is `kill -9`ed, the sidecar dies before its teardown block.
/// Without it, one hard death would wedge this device's working copy for every future chain,
/// permanently, with no command to clear it — and this epic deliberately adds no such command
/// ("KEIN NEUES MUTATIONSVERB").
///
/// **Why two hours.** The number is chosen from the shape of the failure, not from how long work
/// "should" take, because the two directions of being wrong are not symmetric:
///
/// - **Too short** expires a lease out from under a chain that is still running, and hands the
///   working copy to a second chain while the first still has hands on the git index and
///   `target/` — precisely the corruption observed in PR #252 that this whole epic exists to
///   prevent. This failure is silent, concurrent, and produces garbage in a real repository.
/// - **Too long** leaves one device-local row claimed after a crash, so the next chain waits in a
///   VISIBLE queue (the receipt says so, `nxc threads` will show it) instead of colliding. This
///   failure is loud and local.
///
///   **It is self-healing since nxf 6j6v.fabb, and it was not before** — this doc claimed it was,
///   then claimed the opposite, and both are recorded here because the difference is the whole of
///   6j6v.yd4w. Expiry alone only stopped the abandoned lease from refusing every FUTURE acquire:
///   nothing swept the lease table and nothing polled the queue, so a trigger ALREADY parked kept
///   waiting past the bound indefinitely, until some later chain happened to acquire and then
///   release. What polls now is `orchestration::sweep_expired_working_tree`, at the front of every
///   `nxc tick` — and a commission that parks arms one for this bound
///   (`orchestration::arm_the_lease_bound`), so the drain happens with nobody present. The sweep
///   asks one question expiry does not, and declines on it: whether any session in the holding
///   chain still has a live process. See that function for why declining costs nothing this was
///   about.
///
///   **And since nxf 6j6v.yd4w the waiter is not merely reached but FIRST.** Healing the wedge left
///   the other half of that ticket open: whoever came back for the copy at the bound got it, and a
///   NEWCOMER's own acquire was as good a way back as the sweep's — so a rival that had stood at
///   position 1 the whole time was overtaken by a chain that had waited nothing, and waiting was
///   punished by waiting. An acquire that meets an expired lease now hands the queue's turn out
///   first (`orchestration::the_queue_goes_before_a_newcomer`) and competes for what is left.
///
///   `working_tree_two_process_e2e.rs` pins the three halves side by side:
///   `a_hard_killed_holder_loses_the_lease_past_the_bound_but_the_queue_does_not_self_drain` (a
///   clock that merely MOVES hands the queue's turn to nobody),
///   `a_tick_past_the_bound_drains_the_queue_the_expiry_alone_did_not` (the one call the first
///   leaves out) and
///   `at_expiry_the_chain_already_waiting_goes_before_the_newcomer_that_reclaims` (the other event
///   it leaves out, and who it belongs to).
///
/// So the bound is set well past the longest chain that could still be alive, not near it. A
/// healthy code → review → mitigate → merge chain on this repo runs in minutes to tens of minutes;
/// its individual steps are bounded far tighter already (a cloud runtime reaps an idle session at
/// 15 minutes and any session at 8 hours; a step's own `liveness` window nudges long before that).
/// Two hours is several times the worst plausible healthy chain and still an order of magnitude
/// below "somebody notices tomorrow", which is what no bound at all amounts to. It is not a
/// deadline anyone is expected to meet, and no code path treats it as one: a chain that is still
/// working simply RE-ACQUIRES on its next trigger and rides the bound forward, because
/// [`ChatStore::acquire_working_tree`] renews `expires` on an inherit.
///
/// **The residual case that renewal does NOT cover is CLOSED, and it is worth keeping the shape of
/// it** (PR review of nxf 6j6v.303b; closed by nxf 6j6v.8y6t). It read: a chain rides the bound
/// forward only when it TRIGGERS something, so a single step that works — not idles — for more than
/// two hours without commissioning anything loses the lease under itself, and the next chain can
/// take the working copy while it is still building. That is the "too short" failure this doc says
/// must never happen, and two hours did not make it impossible, only unlikely. The paragraph then
/// said closing it properly needs a heartbeat — a renewal driven by evidence of work rather than by
/// the next trigger.
///
/// It was closed with LESS than a heartbeat, by asking a question instead of taking a measurement:
/// [`crate::orchestration::expired_but_still_writing`] refuses to let an expired lease be taken over
/// while any session in the holding area still has a live process. A step that works for six hours
/// is a live process, so the clock running out no longer hands its checkout to anybody. The
/// transcript-progress heartbeat is still the better instrument for a DIFFERENT question — how far
/// along is this step — and is not needed for this one.
///
/// A `<n><unit>` string rather than a [`std::time::Duration`], unlike its two siblings above,
/// because what the lease stores is an INSTANT: it is resolved through
/// [`crate::facade::resolve_instant`], the one grammar (and the one set of overflow guards) this
/// crate already parses every `--deadline` and every step `liveness` window with. A `Duration` here
/// would mean a second, independent piece of date arithmetic beside it.
pub const WORKING_TREE_LEASE_BOUND: &str = "2h";

/// The claim area a trigger's session competes for the working-tree lease under (nxf 6j6v.bqe0) —
/// resolved once, at trigger time, by [`resolve_scope`]. Wraps whichever id is the CHAIN's own
/// identity: a `Thread` for a role trigger (a `send`/`reply` DM, a channel member's board), or a
/// `Session` for the few triggers that carry neither — the ephemeral synthesizer, a bare resume.
///
/// It had a third variant, `Run(String)`, until 6j6v.dvyq §3 removed the run record. It is not
/// missed: a `run:` claim area did NOT contain its own threads, which is what forced the two
/// bridges 6j6v.1xw1 removed, and a thread subtree does.
///
/// Deliberately **not** `#[non_exhaustive]`, unlike sibling declaration enums such as
/// [`crate::role::WorkingTree`]: this is an internal coordination type, never a facade type, and
/// later tickets in this same epic match on it exhaustively inside this crate — a fourth variant
/// showing up unannounced there is exactly the bug an exhaustive match is supposed to catch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkScope {
    Thread(String),
    Session(String),
}

/// The two [`WorkScope::key`] prefixes, named once because [`WorkScope::key`] and
/// [`WorkScope::parse`] are each other's inverse and a literal spelled twice is how an inverse pair
/// stops being one.
const THREAD_PREFIX: &str = "thread:";
const SESSION_PREFIX: &str = "session:";

impl WorkScope {
    /// The stable string key each variant resolves to — `"thread:<id>"` / `"session:<id>"`. This
    /// is not cosmetic: the lease/queue rows are stored under this exact string as their primary
    /// key, so it is a database contract, and a test in this module pins both prefixes exactly for
    /// that reason.
    pub fn key(&self) -> String {
        match self {
            WorkScope::Thread(id) => format!("{THREAD_PREFIX}{id}"),
            WorkScope::Session(id) => format!("{SESSION_PREFIX}{id}"),
        }
    }

    /// [`key`](WorkScope::key)'s inverse: read a stored `scope_key` back into the scope it names
    /// (nxf 6j6v.fe0f).
    ///
    /// The release path is where this is needed and why it exists: what the lease table hands back
    /// is a KEY ([`ChatStore::working_tree_holder`]), and deciding whether that holder is finished
    /// means asking about its threads — which is a question about the scope, not about the string.
    ///
    /// `None` for a key carrying none of the three prefixes. That is not defensive padding: the
    /// column is device-local text that a future variant (or a hand-edited row) can put an unknown
    /// prefix into, and the caller's right answer there is to leave the lease alone rather than to
    /// guess a scope and release somebody else's claim.
    pub fn parse(key: &str) -> Option<WorkScope> {
        if let Some(id) = key.strip_prefix(THREAD_PREFIX) {
            Some(WorkScope::Thread(id.to_string()))
        } else {
            key.strip_prefix(SESSION_PREFIX)
                .map(|id| WorkScope::Session(id.to_string()))
        }
    }
}

/// Resolve which [`WorkScope`] a trigger belongs to: `thread` beats `session`.
///
/// The order matches the CHAIN the trigger is actually part of, not the shape of the call site. It
/// took a `run` first, ahead of both, until 6j6v.dvyq §3 removed the run record.
///
/// **Which arm a COMMISSION takes, and why the answer changed twice.** This paragraph is the third
/// version of itself, so it is worth reading as a sequence rather than as a claim:
///
///  1. It first said the persona trigger "opens or reuses the DM channel, and its thread, BEFORE
///     triggering the role, so a thread id already exists by the time this function runs" — for
///     EVERY direct persona trigger.
///  2. **nxf 6j6v.jepk (PR review 2026-08-12) disproved that**, and narrowed it: the verb minted no
///     thread of its own, so only `nxc send --to <persona>` resolved to [`WorkScope::Thread`],
///     because `surface::send_to`'s persona branch pre-minted one and passed it in. A THREAD-LESS
///     trigger posted no thread at all and fell through to [`WorkScope::Session`] — genuinely
///     thread-less, not merely under-documented.
///  3. **nxf 6j6v.ntp9 made (1) true after all, by building it.** The thread is the persona
///     coordinator's now: `orchestration::coordinator_commission` opens one when its caller hands
///     it none, precisely so the property stops depending on which surface called. So EVERY
///     commission
///     resolves to [`WorkScope::Thread`] — from the CLI, from `Engine::send_to`, from a channel's
///     flow step, and from any future caller of the verb.
///
/// The correction in (2) is therefore SUPERSEDED, and it is spelled out rather than deleted because
/// its argument is the reason (3) was built: a property that holds only because one surface
/// remembers to establish it is a property an embedding host does not get (nxf 41j0.vhsk).
///
/// **The [`WorkScope::Session`] arm below is still live, and by exactly two callers**, neither of
/// which any surface reaches: `orchestration::role_resume`, which takes its caller's `thread` as it
/// is and deliberately keeps having none when the caller names none, and the ephemeral
/// `summarize` synthesizer, which is not a declared role. `orchestration_trigger.rs`'s
/// `an_exclusive_trigger_that_resolves_to_session_scope_takes_no_lease_at_all` is what pins it.
pub fn resolve_scope(thread: Option<&str>, session: &str) -> WorkScope {
    if let Some(thread) = thread {
        WorkScope::Thread(thread.to_string())
    } else {
        WorkScope::Session(session.to_string())
    }
}

/// Normalize an already-RFC3339 instant to its canonical UTC (`Z`) form — the same normalization
/// `resolve_deadline`/`resolve_instant` (facade.rs) apply before storing a deadline, and for the
/// same reason, restated here because [`ChatStore::acquire_working_tree`] and
/// [`ChatStore::working_tree_holder`] compare `now`/`expires` as raw SQL byte strings (`expires <=
/// ?`, `expires > ?`): that comparison is sound only when both sides are anchored to the same
/// offset. A caller's `now`/`expires` can arrive in ANY valid RFC3339 offset — `nxc`'s own clock
/// (`cli.rs::resolve_now`) either passes `NXC_NOW` through verbatim or emits `OffsetDateTime::
/// now_utc()`, and a future caller is not obliged to pre-normalize before calling this module.
/// Stored (or compared) verbatim, an offset-form instant can mis-sort — a negative-zone FUTURE
/// instant sorting as PAST — which here means a live lease read as already expired and handed to a
/// second chain: the exact corruption this epic exists to prevent. So both methods normalize before
/// they touch SQL, on every read and every write, rather than trusting the caller to have done it.
pub(crate) fn to_utc_rfc3339(instant: &str) -> Result<String> {
    let parsed = OffsetDateTime::parse(instant, &Rfc3339).map_err(|e| {
        NxfError::validation(format!("not a valid RFC3339 instant: {instant} ({e})"))
    })?;
    parsed
        .to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .map_err(|e| NxfError::io(format!("formatting UTC instant: {e}")))
}

/// One trigger that lost the working-tree lease race and is waiting its turn (nxf 6j6v.m3d5).
///
/// Deliberately carries the trigger's raw INPUTS, never a composed prompt. The engine re-reads and
/// re-composes the role's prompt from the declaration folder on every trigger (role.rs), so a
/// prompt frozen at enqueue time would go silently stale the moment someone edits the role file
/// while the entry sits in the queue — the field list here is exactly what a later `trigger_role`
/// call needs to re-run that composition against the catalogue in force AT RELEASE TIME, not what
/// it said when this entry was queued.
///
/// **Since nxf 6j6v.n92p "in force" is the entry's OWN OPERATION's version**, not whatever the
/// folder says at the moment the lease comes free — see `orchestration::fire_queued_trigger`, which
/// re-composes against it. The argument for storing inputs rather than a prompt is unchanged by
/// that; what changed is which catalogue the re-composition reads, and without the change waiting
/// in this queue would have been the one way to slip an edited declaration into a chain already
/// under way.
///
/// `id` is meaningful only on the way OUT: [`ChatStore::peek_working_tree_queue`] and
/// [`ChatStore::take_working_tree_queue`] fill it in from the row; [`ChatStore::enqueue_working_tree`]
/// ignores whatever a caller puts here on the way in — the table's own `AUTOINCREMENT` assigns it,
/// and the real id comes back as that call's `Ok(i64)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedTrigger {
    pub id: i64,
    pub scope_key: String,
    pub role: String,
    pub session: String,
    pub thread: Option<String>,
    pub message: String,
    pub model: Option<Model>,
    pub depth: u32,
    /// How urgent the trigger was — the ordering key `(priority, enqueued_at, id)` sorts on, stored
    /// as the ORDINAL [`Priority`]'s own declaration order defines (`Urgent` = 0, `Normal` = 1,
    /// `Background` = 2) rather than as its serde label, because this column is an ordering key and
    /// not user-facing history (see [`ChatStore::enqueue_working_tree`]).
    ///
    /// A FIELD since nxf 6j6v.fe0f rather than a separate argument to
    /// [`ChatStore::enqueue_working_tree`]: the release path re-fires this entry through
    /// `trigger_role`, which competes for the lease exactly as the original call did and can
    /// therefore be re-queued — and an entry that came back out as `Urgent` and went back in as
    /// something else would silently lose the one judgement the ordering exists to honour. It is
    /// part of the trigger's raw inputs like every other field here, so it round-trips with them.
    pub priority: Priority,
    /// When this entry FIRST joined the queue — the second component of the `(priority,
    /// enqueued_at, id)` order, and like [`id`](QueuedTrigger::id) it is asymmetric by design.
    ///
    /// On the way IN, `None` is the ordinary case and means "this entry is new": [`ChatStore::
    /// enqueue_working_tree`] stamps its own `now` argument. `Some` is a promotion that lost the
    /// race for the lease and is going back into the queue, and it keeps the place in line it has
    /// already waited for rather than starting over behind everything that arrived meanwhile (PR
    /// review of nxf 6j6v.fe0f, Finding 2 — the other half of the argument that made `priority` a
    /// field). On the way OUT it is always `Some`, read back from the row.
    pub enqueued_at: Option<String>,
}

/// The columns, in the one order [`queued_trigger_from_row`] reads them back in — written once so
/// the three statements that hand it a row (peek, take, and the release path's own take inside its
/// transaction) cannot drift into three different column orders.
const QUEUE_COLS: &str =
    "id, scope_key, role, session, thread, message, model, depth, priority, enqueued_at";

/// Parse one `working_tree_queue` row (the [`QUEUE_COLS`] order) into a [`QueuedTrigger`]. `model`
/// is stored as [`Model`]'s lowercase serde wire form or SQL `NULL` — the same convention
/// `message_reducer.rs` uses for `messages.kind`/`priority`/`disposition` — so it round-trips
/// through `serde_json`'s bare-string form rather than inventing a second spelling.
fn queued_trigger_from_row(r: &Row) -> rusqlite::Result<QueuedTrigger> {
    let model: Option<String> = r.get(6)?;
    let model = model
        .map(|s| serde_json::from_value(serde_json::Value::String(s)))
        .transpose()
        .map_err(|e: serde_json::Error| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
        })?;
    Ok(QueuedTrigger {
        id: r.get(0)?,
        scope_key: r.get(1)?,
        role: r.get(2)?,
        session: r.get(3)?,
        thread: r.get(4)?,
        message: r.get(5)?,
        model,
        depth: r.get(7)?,
        priority: priority_from_ordinal(r.get(8)?),
        enqueued_at: Some(r.get(9)?),
    })
}

/// The inverse of the `Priority as i64` ordinal [`ChatStore::enqueue_working_tree`] stores — the
/// ONLY place the mapping is read backwards, so the pair cannot drift apart unnoticed (a round-trip
/// test in this module pins all three).
///
/// An ordinal outside the three is [`Priority::Normal`], not an error: this column is an ordering
/// key on a device-local table, and a row a future variant (or a hand-edited file) put an unknown
/// number into must still be able to RUN. Refusing to parse it would strand the trigger in the
/// queue with nothing able to take it out.
fn priority_from_ordinal(ordinal: i64) -> Priority {
    match ordinal {
        o if o == Priority::Urgent as i64 => Priority::Urgent,
        o if o == Priority::Background as i64 => Priority::Background,
        _ => Priority::Normal,
    }
}

/// Delete `scope_key`'s OWN lease row, reporting how many rows that was (0 or 1).
///
/// **Deliberately scoped to `scope_key`'s own row**: a release naming a scope that is not the
/// current holder must be harmless, or a late release racing a new holder that already acquired
/// could delete a lease that isn't its own — the same reasoning
/// [`crate::store::ChatStore::release_consolidation_claim`] states for its own version guard.
///
/// A free function rather than a `ChatStore` method, and the plain `release_working_tree` this
/// module used to expose is gone with it (PR review of nxf 6j6v.fe0f, Finding 4): the only
/// production paths that give the lease back are the two that advance the queue in the same
/// transaction — [`ChatStore::release_working_tree_and_take_next`] and
/// [`ChatStore::reclaim_expired_working_tree_and_take_next`] — and a public "release without
/// advancing the queue" is not a missing convenience but the exact state this ticket exists to
/// prevent: a free working copy with a chain still parked behind it and nobody left to start it.
///
/// **Its return value is also what makes a DOUBLE release harmless** (nxf 6j6v.jzaj). Two `nxc
/// reply` processes can both conclude that the same claim area is finished and both call the
/// release; the second finds no row under that key — the first's transaction has already handed the
/// copy on — so it deletes nothing, and its caller aborts before taking anything off the queue.
/// `tests::a_second_release_of_the_same_scope_takes_nothing` and
/// `tests::only_one_of_four_genuinely_concurrent_releases_of_one_scope_hands_the_queue_over` pin
/// that, one deterministically and one with a real barrier.
fn release_lease_row(conn: &Connection, scope_key: &str) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM working_tree_lease WHERE tree = 'default' AND scope_key = ?1",
        params![scope_key],
    )?)
}

/// Read AND remove the queue's head in ONE statement — see
/// [`ChatStore::take_working_tree_queue`] for why that is a requirement rather than a convenience.
fn take_queue_head(conn: &Connection) -> Result<Option<QueuedTrigger>> {
    Ok(conn
        .query_row(
            &format!(
                "DELETE FROM working_tree_queue WHERE id = (
                     SELECT id FROM working_tree_queue ORDER BY priority, enqueued_at, id LIMIT 1
                 ) RETURNING {QUEUE_COLS}"
            ),
            [],
            queued_trigger_from_row,
        )
        .optional()?)
}

/// Read AND remove every entry belonging to the claim area at the queue's head — the head itself
/// plus every other entry sharing its `scope_key` (PR #336 review of nxf 6j6v.bqe0, Code Quality #1,
/// CRITICAL).
///
/// **Why a claim area rather than one entry.** A fan-out into a declared `working_tree: exclusive`
/// channel triggers each member separately, and every member resolves to the SAME scope — the
/// board's own thread ([`resolve_scope`], via `orchestration::open_declared_channel_and_fan_out`).
/// When the board's first acquire loses to an unrelated holder, so does every member's, and each one
/// enqueues its own row under that single `scope_key`. Taking one row per release then starts ONE
/// member and strands its siblings permanently: the board owes THEIR replies, so its own scope never
/// reports "nothing outstanding" and therefore never releases — the very thing that would start
/// them. Not a corner case; the shipped `examples/role-runtime-v3` `review` channel is exactly that
/// shape.
///
/// Firing the whole area together is not a new behaviour to reason about: it is precisely what the
/// UNCONTENDED path already does, where the first member acquires the lease and the rest inherit it.
/// A release that fires one sibling and parks the others is the anomaly. The same holds for the
/// other way two entries can share a key — repeated triggers into one explicit `--thread`, which
/// likewise run side by side when the lease happens to be free, and which would deadlock the same
/// way if that thread's quorum expects the parked one's reply. (`send --to <persona>` mints a NEW
/// thread per call, so ordinary persona sends never share a key.)
///
/// **The `(priority, enqueued_at, id)` order still decides WHICH area goes next** — it picks the
/// head, exactly as before, and scope-awareness applies only afterwards. No claim area can jump the
/// queue by having more members on it.
///
/// **Two statements, so this is for callers that already hold a write transaction** — today
/// [`hand_the_working_tree_on`], which both hand-off entrances run inside their own `BEGIN
/// IMMEDIATE`. Reading
/// the head and deleting its area outside one would let another process take the head in between,
/// which is the whole hazard [`ChatStore::take_working_tree_queue`]'s single statement exists to
/// avoid. `DELETE ... RETURNING` makes no promise about the ORDER it yields rows in, so the result
/// is sorted back into the queue's own order rather than trusting it.
fn take_queue_scope(conn: &Connection) -> Result<Vec<QueuedTrigger>> {
    let head: Option<String> = conn
        .query_row(
            "SELECT scope_key FROM working_tree_queue ORDER BY priority, enqueued_at, id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let Some(scope_key) = head else {
        return Ok(Vec::new());
    };
    let mut taken = {
        let mut st = conn.prepare(&format!(
            "DELETE FROM working_tree_queue WHERE scope_key = ?1 RETURNING {QUEUE_COLS}"
        ))?;
        let rows = st.query_map(params![scope_key], queued_trigger_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    taken.sort_by(|a, b| queue_order_key(a).cmp(&queue_order_key(b)));
    Ok(taken)
}

/// **What entitles a caller to take the working copy from a holder that did not offer it** (nxf
/// 6j6v.xb24) — the one thing the two reclaims below disagree about, and it is a clause of SQL.
///
/// An enum rather than a `bool`, because the two answers are two different arguments and the reader
/// of the call site is entitled to see which one is being made: a clock that has run out, or a
/// holder that has been shown to be gone. The compare-and-swap on `(scope_key, expires)` is the same
/// either way and is what makes both safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhyTheLeaseIsForfeit {
    /// The lease is past its own bound as of `now` — [`ChatStore::reclaim_expired_working_tree_and_take_next`].
    ItsBoundRanOut,
    /// The bound still stands, and the chain behind it does not —
    /// [`ChatStore::reclaim_a_dead_holders_working_tree_and_take_next`].
    ItsHolderIsGone,
}

/// Take the queue's next claim area AND write the lease row for it — the second half of every
/// hand-off, run inside the caller's own write transaction (nxf 6j6v.yd4w).
///
/// **Why the lease is GRANTED here rather than left for the promoted trigger to acquire.** Both
/// callers above delete a lease row and then take the head's area; a caller that stopped there
/// would leave the working copy FREE between its own commit and the moment
/// `orchestration::fire_queued_trigger` gets the promoted entry as far as
/// [`ChatStore::acquire_working_tree`]. That window is short and it is real, and a newcomer
/// arriving inside it wins the copy outright — after which the entry that had just been promoted is
/// fired, refused, and queued again BEHIND the newcomer it was ahead of. Nothing is lost and the
/// order stays well defined, so this was never a safety hole; it is the fairness hole 6j6v.yd4w is
/// about, in its second form. Writing the row here removes the window instead of narrowing it:
/// there is no instant at which the lease is free with an entry waiting for it.
///
/// **`granted_expires` is a BACKSTOP, not this area's real bound.** What the promoted area's lease
/// should run to is derived from its own declared windows
/// (`orchestration::operation_lease_expiry`), and that derivation needs the catalogue and a clock
/// this module does not have — and it cannot be run before the take, because which area is at the
/// head is decided inside this transaction. So the row is written with the caller's own fallback
/// and corrected microseconds later: firing the entry goes through
/// [`ChatStore::acquire_working_tree`] with the SAME key, which is an inherit — it keeps `acquired`
/// and replaces `expires` with the derived value. The backstop is therefore only ever the bound of
/// an area whose triggers all failed to start, and `orchestration::release_and_fire` hands the copy
/// on rather than sitting on it in exactly that case.
///
/// **One way to reach that ceiling IS new, and the cost being unchanged is not the same thing**
/// (independent review of PR #380, Integrity & Robustness #4). A process that dies between this
/// commit and the fire's own inherit-acquire leaves the promoted area holding a lease it never
/// derived a bound for, with no work done at all — and before the grant, a crash in the analogous
/// window left the working copy FREE. So this adds a trigger condition for the backstop that did
/// not exist, even though it adds nothing to what the backstop then costs, and even though the
/// state heals by the paths that already exist for an abandoned lease: the sweep at its two-hour
/// bound, the sweep inside it once the holder is provably dead (nxf 6j6v.xb24), and — since
/// 6j6v.yd4w — the next chain's own acquire. The window is a few statements wide with no I/O in it,
/// which is why this is written down as a shape rather than defended against with machinery.
///
/// An empty queue writes nothing at all: the lease row stays deleted and the working copy is free,
/// which is the whole of what a release with nobody waiting means.
fn hand_the_working_tree_on(
    tx: &Connection,
    now: &str,
    granted_expires: &str,
) -> Result<Vec<QueuedTrigger>> {
    let taken = take_queue_scope(tx)?;
    if let Some(head) = taken.first() {
        // A plain INSERT rather than an upsert: both callers have just deleted the row in this same
        // transaction, so a conflict here would mean the delete did not do what its own return
        // value said it did.
        tx.execute(
            "INSERT INTO working_tree_lease(tree, scope_key, acquired, expires)
             VALUES ('default', ?1, ?2, ?3)",
            params![head.scope_key, now, granted_expires],
        )?;
    }
    Ok(taken)
}

/// Whether a reply of this KIND hands the task back instead of concluding it (nxf 6j6v.1xw1) —
/// the one place the two hand-back kinds are named, read by [`ChatStore::work_scope_handed_back`].
///
/// **An EXHAUSTIVE match with no `_` arm, on purpose.** Adding a variant to
/// [`MessageKind`](crate::model::MessageKind) is a compile error right here, so its author has to
/// come and decide what the new kind means for a working copy — the same construction
/// `store::tests::every_message_kind` uses for the discharge predicate, and for the same reason: a
/// wildcard would silently sort the next kind into "concludes", and a kind that actually hands the
/// task back would then release a working copy with the task mid-flight.
///
/// An UNPARSEABLE label answers "hands it back", which is the conservative direction of this
/// mechanism (holding the copy too long is benign; releasing it early is the collision this epic
/// exists to prevent). It is unreachable in practice — `messages.kind` is written from
/// `serde_json::to_value(MessageKind)` and an envelope carrying a label this build cannot parse is
/// never folded at all (the store-don't-fold path, spec §3.1) — so this is the answer to a case that
/// cannot arise rather than a branch anything depends on.
fn hands_the_task_back(kind_label: &str) -> bool {
    let Ok(kind) = serde_json::from_value::<crate::model::MessageKind>(serde_json::Value::String(
        kind_label.to_string(),
    )) else {
        return true;
    };
    match kind {
        // "I cannot" (6j6v.wt37) and "what do you mean by X?" — two different facts, deliberately
        // kept apart everywhere else, and the same thing for a working copy: nobody is working, and
        // the task is mid-flight.
        //
        // **`Question` IS CURRENTLY UNREACHABLE, and that is said here rather than left to be
        // discovered** (nxf 6j6v.jgn6, raised again by the PR #347 review). `--kind` left the CLI
        // and the seam with 6j6v.ckeq, so no surface can produce a `question` message any more:
        // `--escalate` is the only kind an agent can still set. The arm is KEPT, not trimmed,
        // because trimming it would silently answer a question the owner has not: jgn6 puts three
        // options on the table, and only ONE of them ("a question is an escalation, live with it")
        // ends with this arm gone. The other two — give the ask its own note (`reply --ask`), or
        // carry it as a named loss until 6j6v.xr3z reopens the area — need this branch and, more
        // to the point, need the REASON above it, which is the part that would not survive a
        // deletion and a later restoration.
        //
        // Coverage did not go with the entrance: `working_tree_claim_scope.rs`'s three question
        // cases drive `orchestration::reply` through `reply_with_a_question`, and
        // `every_message_kind_is_sorted_into_hand_back_or_conclusion_by_an_exhaustive_match` still
        // pins this arm by value.
        crate::model::MessageKind::Escalation | crate::model::MessageKind::Question => true,
        // **A VERDICT CONCLUDES, it does not hand the task back** (nxf 6j6v.553s (a)) — and this is
        // the arm the construction above exists to force somebody to think about, so here is the
        // thinking.
        //
        // `--needs-rework` looks like a hand-back and is the opposite of one for a working copy. The
        // party that answered has FINISHED its turn and said something definitive about somebody
        // else's work; the run then goes on, and the very next step — the one the back edge names —
        // declares its own obligation in the same synchronous write path, which is what holds the
        // lease. Nobody is left waiting on an answer that is not coming, which is the shape the two
        // arms above are about.
        //
        // Answering `true` here would be the benign direction of this mechanism in name only. A
        // settled slot is never re-declared: a fresh slot is opened for each pass, so the reviewer's
        // thread keeps reading `needs_rework` for the rest of the workspace's life. The claim would
        // then be held past a run that COMPLETED CLEANLY, to `WORKING_TREE_LEASE_BOUND`, with
        // nothing left that could ever clear it — an over-hold with no self-clearing door, which is
        // exactly what that doc says makes the wide read tolerable.
        //
        // **AN ACCEPTANCE CONCLUDES TOO, and for the verdict arm's reason twice over** (nxf
        // 6j6v.am8j). It is written by the party that COMMISSIONED the round, on the round's own
        // thread — a party that owes nobody an answer there and is not working in the checkout at
        // all — and what it does is start the next step, whose own obligation takes the lease in the
        // same synchronous write path. Nobody is left waiting. Answering `true` would hold the
        // checkout on behalf of a human who is not in it, until the lease bound, with the round
        // running on happily beside the hold.
        crate::model::MessageKind::Accepted
        | crate::model::MessageKind::NeedsRework
        | crate::model::MessageKind::Task
        | crate::model::MessageKind::Report
        | crate::model::MessageKind::Decision
        | crate::model::MessageKind::Info => false,
    }
}

/// The `(priority, enqueued_at, id)` ordering key as a comparable tuple — the same order the SQL
/// `ORDER BY` uses, written once so the in-memory sort in [`take_queue_scope`] cannot drift from the
/// order the database picks the head with.
fn queue_order_key(q: &QueuedTrigger) -> (i64, &str, i64) {
    (
        q.priority as i64,
        q.enqueued_at.as_deref().unwrap_or(""),
        q.id,
    )
}

/// **One row of `withdrawn_holders`** (nxf 6j6v.b9nf): who withdrew the holder's running round, and
/// when — what the tick's park reason names, and — since the Integrity #2 fix of this item's review
/// — what `nxc status` now shows on the holding operation too, beside the sessions still pinning it
/// ([`crate::facade::StatusOperation::withdrawn`]).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WithdrawnHolder {
    /// The instant of the withdrawal, UTC-normalized.
    pub withdrawn_at: String,
    /// The qualified handle of whoever withdrew it.
    pub by: String,
}

/// **One row of `withdrawn_sessions`** (nxf 6j6v.27b9): a session a withdrawal asked to stop, and
/// what became of the tick's one further SIGTERM to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawnSession {
    /// When the withdrawal asked it to stop, UTC-normalized — what the tick's cadences count from.
    pub stopped_at: String,
    /// When a tick claimed the one further SIGTERM, or `None` while none has.
    pub resignalled_at: Option<String>,
    /// The worker's own sentence when that further SIGTERM did not land; `None` when it did, or
    /// when none was attempted yet.
    pub resignal_refused: Option<String>,
}

/// **Forget `scope_key`'s withdrawn marker on a connection the caller is already writing through**
/// — so the lease leaving a holder and its marker going land in ONE transaction, and no reader can
/// see a claim that has moved on still "to be parked". The release and both reclaims call it inside
/// their own `BEGIN IMMEDIATE`, and nothing calls it outside one: a withdrawal ends only with the
/// copy going on. `park::forget_park_refusal`'s twin, for its reason.
///
/// The sessions that withdrawal stopped go with it (nxf 6j6v.27b9): a row there is only ever about
/// a marker that still stands.
pub(crate) fn forget_withdrawn_holder(conn: &Connection, scope_key: &str) -> Result<bool> {
    conn.execute(
        "DELETE FROM withdrawn_sessions WHERE scope_key = ?1",
        params![scope_key],
    )?;
    Ok(conn.execute(
        "DELETE FROM withdrawn_holders WHERE scope_key = ?1",
        params![scope_key],
    )? > 0)
}

/// **Forget every withdrawn marker but `holder`'s** — for the one lease movement that names only the
/// NEW holder: [`ChatStore::acquire_working_tree`]'s compare-and-swap, which can take an expired
/// lease from a scope it never names. `park::forget_other_park_refusals`'s twin.
pub(crate) fn forget_other_withdrawn_holders(conn: &Connection, holder: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM withdrawn_sessions WHERE scope_key <> ?1",
        params![holder],
    )?;
    conn.execute(
        "DELETE FROM withdrawn_holders WHERE scope_key <> ?1",
        params![holder],
    )?;
    Ok(())
}

impl ChatStore {
    /// Acquire (or inherit) the working-tree lease for `scope`, in the single-statement
    /// compare-and-swap this module's doc comment calls out by name: `nxc send` and `nxc reply` are
    /// separate PROCESSES against the same file, so nothing but the database itself — deciding
    /// inside one implicit transaction — can arbitrate between one caller's read of "is it free" and
    /// its write of "I have it".
    ///
    /// `now` and `expires` are normalized to UTC via [`to_utc_rfc3339`] before they ever reach SQL —
    /// see that function's doc for why: the CAS below decides by comparing these as raw byte
    /// strings, which is only sound when every value ever written to the `expires`/`acquired`
    /// columns, and every `now` ever compared against them, shares one offset.
    ///
    /// Returns `true` when this call leaves `scope` holding the lease. That covers three shapes in
    /// one statement: no lease existed yet; the prior holder's lease had already expired (a
    /// **reclaim**, riding on this same acquire attempt with no cleanup run of its own — the pattern
    /// the transcript-retention ticket, 6j6v.t7pa, already uses); or `scope` already held it (an
    /// **inherit** — the same chain acquiring again for its next step). Returns `false` only when a
    /// DIFFERENT scope currently holds an unexpired lease.
    ///
    /// **The RECLAIM is blind to the queue, and one caller has to make up for that** (nxf
    /// 6j6v.yd4w). This statement's whole value is that it is one statement, and the question "is
    /// anybody already waiting?" cannot be folded into it without giving that up. So it hands an
    /// expired lease to whoever asks — including a chain that has never waited a second, while a
    /// rival stands at position 1. `orchestration::the_queue_goes_before_a_newcomer` runs
    /// [`Self::reclaim_expired_working_tree_and_take_next`] BEFORE this call for exactly that
    /// reason, which leaves this statement deciding the same three shapes it always did.
    ///
    /// Inheriting does not reset `acquired` — a holder re-acquiring for its next step should still
    /// see how long the CHAIN has been running, not when its most recent step happened to fire.
    ///
    /// **The `false` is narrower than it looks, and one caller is right not to take it at face
    /// value** (nxf 6j6v.43kw). "A different scope" is decided here by exact `scope_key` equality,
    /// which is the correct question for a single statement to ask but not the whole rule: the claim
    /// area is a SUBTREE ([`Self::work_scope_threads`]), so a claim opened from INSIDE the held area
    /// carries a different key while being the holder's own work. Refusing it queues a chain behind
    /// itself. Widening the SQL is the wrong place for that — the answer needs the thread forest, and
    /// this statement's whole value is that it is one statement — so
    /// `orchestration::inherits_the_held_claim` asks the subtree question after this returns `false`,
    /// and its doc carries the argument for why that stays sound outside the CAS.
    pub fn acquire_working_tree(
        &mut self,
        scope: &WorkScope,
        now: &str,
        expires: &str,
    ) -> Result<bool> {
        let scope_key = scope.key();
        let now = to_utc_rfc3339(now)?;
        let expires = to_utc_rfc3339(expires)?;
        let acquired = self.connection().execute(
            "INSERT INTO working_tree_lease(tree, scope_key, acquired, expires)
             VALUES ('default', ?1, ?2, ?3)
             ON CONFLICT(tree) DO UPDATE SET
                 scope_key = excluded.scope_key,
                 acquired  = CASE WHEN working_tree_lease.scope_key = excluded.scope_key
                                  THEN working_tree_lease.acquired ELSE excluded.acquired END,
                 expires   = excluded.expires,
                 -- The contention memo belongs to the AREA, not to the row (nxf 6j6v.de9s): an
                 -- inherit keeps it, a RECLAIM by a different scope must not walk into the previous
                 -- holder's clock. Same `CASE` as `acquired` one line up, and for the same reason.
                 -- `orchestration::note_the_contention` would clear it at the next look anyway; this
                 -- is what makes the row never hold a start that belongs to somebody else, rather
                 -- than merely not hold one for long.
                 contended_since = CASE WHEN working_tree_lease.scope_key = excluded.scope_key
                                        THEN working_tree_lease.contended_since ELSE NULL END
             WHERE working_tree_lease.scope_key = excluded.scope_key
                OR working_tree_lease.expires <= ?2",
            params![scope_key, now, expires],
        )?;
        if acquired == 1 {
            // **A refused park is about the claim that holds the copy, and this scope holds it now**
            // (nxf 6j6v.8bv9). The compare-and-swap above can RECLAIM an expired lease from a scope it
            // never names, so it is the one lease movement that cannot forget that scope's refusal
            // by key — it forgets everybody else's instead. On an inherit or a fresh acquire there
            // is nothing else to forget and the statement matches no row. A separate statement
            // rather than part of the swap, which has to stay one: a refusal left standing for the
            // instant between the two is a status line that is one statement stale, not a claim.
            crate::park::forget_other_park_refusals(self.connection(), &scope_key)?;
            // …and the same for a withdrawn holder's marker (nxf 6j6v.b9nf), for the same reason:
            // the marker is about the claim that holds the copy, and this scope holds it now. This
            // scope's OWN marker stays, on an inherit as on a fresh acquire: a withdrawal ends only
            // when the copy goes on, never when a session starts in the withdrawn claim (see
            // `note_withdrawn_holder`).
            forget_other_withdrawn_holders(self.connection(), &scope_key)?;
        }
        Ok(acquired == 1)
    }

    /// The scope key currently holding an unexpired lease, or `None` when the working tree is
    /// free — either no lease was ever taken, or the row on record has already expired. An expired
    /// row stays physically present until the next [`Self::acquire_working_tree`] reclaims it, but it
    /// holds nothing as of `now`, so this reads it as free rather than reporting a stale holder.
    ///
    /// `now` is normalized via [`to_utc_rfc3339`] before the comparison, for the same reason
    /// [`Self::acquire_working_tree`] normalizes its own arguments.
    pub fn working_tree_holder(&self, now: &str) -> Result<Option<String>> {
        let now = to_utc_rfc3339(now)?;
        Ok(self
            .connection()
            .query_row(
                "SELECT scope_key FROM working_tree_lease WHERE tree = 'default' AND expires > ?1",
                params![now],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The lease row as it stands, EXPIRY AND ALL — `(scope_key, expires)`, or `None` when no lease
    /// was ever taken (nxf 6j6v.fabb).
    ///
    /// [`Self::working_tree_holder`] deliberately reads an expired row as "free", which is the right
    /// answer to "may I take the working copy?" and the wrong one to the two questions this ticket
    /// asks. Both need the row itself:
    ///
    /// - **when does this lease run out?** — so a trigger that PARKS can schedule a tick for that
    ///   instant instead of waiting for some later chain to release. Nothing polled the queue, so a
    ///   commission parked behind a lease that then expired waited past the bound indefinitely
    ///   (`working_tree_two_process_e2e.rs`'s
    ///   `a_hard_killed_holder_loses_the_lease_past_the_bound_but_the_queue_does_not_self_drain`
    ///   pins that, and 6j6v.yd4w named the gap).
    /// - **whose lease is it?** — a release has to name the scope it is releasing
    ///   ([`Self::release_working_tree_and_take_next`] is a no-op for any other), and an EXPIRED row
    ///   is exactly the one a sweep wants to name.
    ///
    /// Read-only, and it reports the row rather than judging it: whether `expires` has passed is the
    /// caller's comparison to make, against the `now` it is already carrying.
    pub fn working_tree_lease_row(&self) -> Result<Option<(String, String)>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT scope_key, expires FROM working_tree_lease WHERE tree = 'default'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// **When this device first saw the lease CONTENDED** (nxf 6j6v.de9s) — both halves standing at
    /// once: the holding area has handed its task back, and something else is waiting in the queue.
    ///
    /// `None` is "not contended right now, as far as the last look could tell", which is also what a
    /// lease that predates this column reads as. It is deliberately a memo of a derivable fact
    /// rather than a state machine — see [`ChatStore::note_working_tree_contention`], which is the
    /// only writer, and `orchestration::note_the_contention`, which is the only derivation.
    pub fn working_tree_contended_since(&self) -> Result<Option<String>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT contended_since FROM working_tree_lease WHERE tree = 'default'",
                [],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Record — or clear — the contention memo, and answer with the instant that now stands.
    ///
    /// **Idempotent by construction, and that is what makes the clock start at the FIRST look
    /// rather than the latest one.** Passing `Some(now)` writes `now` only where the column is NULL;
    /// an already-recorded instant is left exactly as it is, so repeated ticks over a standing
    /// contention do not push the deadline away from itself. Passing `None` clears it, which is what
    /// every point that finds the pair no longer standing does — an answered escalation and an
    /// emptied queue both end the contention, and neither should leave a stale start behind for the
    /// next one to be measured from.
    ///
    /// A single statement, like every other write in this module, for the reason its doc gives: two
    /// `nxc` processes must not be able to interleave between reading the column and writing it.
    /// The lease row itself is never created here — no lease, nothing contended.
    pub fn note_working_tree_contention(&mut self, since: Option<&str>) -> Result<Option<String>> {
        match since {
            Some(now) => {
                let now = to_utc_rfc3339(now)?;
                self.connection().execute(
                    "UPDATE working_tree_lease
                        SET contended_since = ?1
                      WHERE tree = 'default' AND contended_since IS NULL",
                    params![now],
                )?;
            }
            None => {
                self.connection().execute(
                    "UPDATE working_tree_lease
                        SET contended_since = NULL
                      WHERE tree = 'default' AND contended_since IS NOT NULL",
                    [],
                )?;
            }
        }
        self.working_tree_contended_since()
    }

    /// Release `scope`'s lease AND take the queue's next CLAIM AREA in ONE `BEGIN IMMEDIATE`
    /// transaction — the hand-off half of the release path (nxf 6j6v.fe0f). The returned entries are
    /// the triggers the caller must now fire, in the queue's own order; empty means there was
    /// nothing waiting, **or** `scope` was not the holder — in which case nothing at all happened,
    /// including the take.
    ///
    /// **The whole area at the head, not one entry** (PR #336 review, Code Quality #1): several
    /// entries share one `scope_key` whenever a multi-member `working_tree: exclusive` board had to
    /// wait, and starting one of them while its siblings stay parked deadlocks the board for good.
    /// [`take_queue_scope`] carries that argument in full, including why the queue's order still
    /// decides which area goes next.
    ///
    /// **Why one transaction, and why IMMEDIATE** (PR review of nxf 6j6v.303b, the other half of the
    /// acquire-side race). Every other statement in this module is a single-statement
    /// compare-and-swap precisely because each `ChatStore` call is its own implicit transaction and
    /// `nxc send`/`nxc reply` are separate PROCESSES against the same file. Release and drain are
    /// two statements, so on their own they publish an intermediate state nobody should ever see:
    /// the lease FREE while the queue head is still queued. A second process that observes exactly
    /// that wins the lease outright, and the head this call is about to take then meets a tree it
    /// cannot have — it is fired, refused, and re-queued behind the newcomer it was ahead of. Under
    /// one writer lock that state is never published: another writer sees the queue intact with the
    /// lease still held, or both changes at once.
    ///
    /// It also makes the pair atomic in the ordinary sense: a take that FAILS after a successful
    /// release would otherwise leave the working copy free with an entry still parked behind it and
    /// nobody left to start it, which is the one shape this ticket exists to prevent.
    ///
    /// IMMEDIATE rather than `unchecked_transaction()`'s DEFERRED for the reason
    /// [`ChatStore::append_transcript`] spells out in full at this crate's other multi-statement
    /// write: a deferred transaction takes the write lock only at its first write and fails a
    /// read→write upgrade INSTANTLY against a concurrent writer, without consulting `busy_timeout`.
    /// Here the first statement is already a write, so the practical difference is smaller — but the
    /// argument for taking the lock up front is identical and there is no reason to hold the two
    /// paths to different rules.
    ///
    /// **Release FIRST, then take, then GRANT**, inside the transaction as well as outside it: an
    /// entry taken before the lease is given up would be fired against a lease that is still
    /// standing, and would be queued straight back behind the very holder that was releasing it.
    ///
    /// **The third step is why the lease never becomes free at all when somebody is waiting** (nxf
    /// 6j6v.yd4w, its second note). Release-and-take being atomic closes the window this doc argues
    /// about above; it does not close the one AFTER the commit, between here and
    /// `orchestration::fire_queued_trigger`. A newcomer acquiring in THAT window wins outright, and
    /// the entry this call just promoted is fired, refused, and queued again — behind the very
    /// chain it was ahead of a moment ago. Safety held there and fairness did not, and this doc
    /// used to say "never published" in a way that read as covering both. So the same transaction
    /// hands the copy straight on: it deletes the outgoing lease, takes the head's claim area, and
    /// writes the lease row for THAT area. There is no instant at which the working copy is free
    /// with an entry waiting for it — see [`hand_the_working_tree_on`].
    pub fn release_working_tree_and_take_next(
        &mut self,
        scope: &WorkScope,
        now: &str,
        granted_expires: &str,
    ) -> Result<Vec<QueuedTrigger>> {
        let scope_key = scope.key();
        let now = to_utc_rfc3339(now)?;
        let granted_expires = to_utc_rfc3339(granted_expires)?;
        let tx = rusqlite::Transaction::new_unchecked(
            self.connection(),
            rusqlite::TransactionBehavior::Immediate,
        )?;
        if release_lease_row(&tx, &scope_key)? != 1 {
            // Not the holder — a foreign release is a no-op (see `release_lease_row`), and taking
            // a head on the strength of it would hand the queue's turn out while somebody else
            // still has the working copy. Dropping the transaction rolls the (no-op) delete back.
            return Ok(Vec::new());
        }
        // The claim is gone, so a refused park about it is too — in this transaction, so no reader
        // sees the copy moved on while the old holder is still "retrying" (nxf 6j6v.8bv9).
        crate::park::forget_park_refusal(&tx, &scope_key)?;
        // And so is a withdrawn holder's marker (nxf 6j6v.b9nf): the copy has gone on, so there is
        // nothing left for the tick's withdrawn occasion to park — and a marker left standing would
        // park this key's NEXT claim the moment it took the copy.
        forget_withdrawn_holder(&tx, &scope_key)?;
        let next = hand_the_working_tree_on(&tx, &now, &granted_expires)?;
        tx.commit()?;
        Ok(next)
    }

    /// **Take an EXPIRED lease away from a chain that is not coming back, and hand the working copy
    /// to whoever is already in the queue** — the same one transaction as
    /// [`Self::release_working_tree_and_take_next`], entered from the other end (nxf 6j6v.yd4w).
    ///
    /// This is the ticket's first acceptance criterion, and until it there was nothing to call.
    /// [`Self::acquire_working_tree`] reclaims an expired lease in a single compare-and-swap that
    /// never looks at the queue, so the chain that ASKS at that moment takes the working copy while
    /// the rival standing at position 1 — who has been waiting for it the whole time — stays exactly
    /// where it is. Waiting was punished by waiting. The reclaim and the queue's turn have to be one
    /// decision, and that is what this is: the expired row goes, the head's claim area comes off the
    /// queue, and the lease is written for THAT area, all under one writer lock.
    ///
    /// **`holder_expires` is the guard, and it is what makes this safe to call from outside a
    /// transaction.** Every caller reads the lease row first (to ask whose it is and whether the
    /// chain behind it still has a live process — questions no SQL statement can answer), and
    /// between that read and this call the holder may have RENEWED: every trigger a chain makes
    /// rides its bound forward. So the delete names both the key and the exact `expires` that was
    /// read, and additionally requires that it is still in the past. A renewal changes the column,
    /// the delete matches nothing, and this reports "nothing taken" rather than tearing the copy out
    /// from under a chain that just proved it is alive.
    ///
    /// Returns the entries the caller must now fire, exactly as the release does — empty when the
    /// lease was renewed, when it is not the row that was read, or when nobody was waiting after
    /// all.
    pub fn reclaim_expired_working_tree_and_take_next(
        &mut self,
        holder_key: &str,
        holder_expires: &str,
        now: &str,
        granted_expires: &str,
    ) -> Result<Vec<QueuedTrigger>> {
        self.reclaim_a_forfeit_lease_and_take_next(
            holder_key,
            holder_expires,
            now,
            granted_expires,
            WhyTheLeaseIsForfeit::ItsBoundRanOut,
        )
    }

    /// **Take the working copy from a holder that is GONE, although its bound has not run out** —
    /// [`Self::reclaim_expired_working_tree_and_take_next`]'s twin for the occasion nxf 6j6v.xb24
    /// added, and the same one transaction (nxf 6j6v.xb24).
    ///
    /// A chain that dies hard inside its two-hour bound used to hold the queue for the whole of what
    /// was left of it: nothing releases (the process is gone), the bound is the only clock, and the
    /// sweep next door refuses to look before it. The evidence that it IS gone is not a clock and is
    /// not this module's to weigh — `orchestration::holder_is_provably_dead` decides it, from the
    /// worker's liveness answer and the claim area's own register — so what this adds is only the
    /// ability to ACT on it atomically.
    ///
    /// **The compare-and-swap is the same one, minus the clause the clock provided.** The delete
    /// still names the key AND the exact `expires` the caller read, which is the whole guard: every
    /// trigger a chain makes rides its bound forward, so a holder that renewed between the caller's
    /// read and this call is not the row that was decided about, the delete matches nothing, and this
    /// answers "nothing taken". A renewal is also the one event that disproves the caller's evidence
    /// — a chain that triggers is a chain with a process — so the guard the expiry check used to give
    /// for free is exactly the guard that still matters. What is dropped is only `expires <= now`,
    /// which for this occasion would refuse every row it is called about.
    ///
    /// Returns the entries the caller must now fire, exactly as its two siblings — empty when the
    /// lease was renewed, when it is not the row that was read, or when nobody was waiting after all.
    pub fn reclaim_a_dead_holders_working_tree_and_take_next(
        &mut self,
        holder_key: &str,
        holder_expires: &str,
        now: &str,
        granted_expires: &str,
    ) -> Result<Vec<QueuedTrigger>> {
        self.reclaim_a_forfeit_lease_and_take_next(
            holder_key,
            holder_expires,
            now,
            granted_expires,
            WhyTheLeaseIsForfeit::ItsHolderIsGone,
        )
    }

    /// The body both reclaims share — one transaction, one compare-and-swap, one hand-on — so the
    /// two occasions cannot drift apart in what they do, only in what entitles them to do it.
    fn reclaim_a_forfeit_lease_and_take_next(
        &mut self,
        holder_key: &str,
        holder_expires: &str,
        now: &str,
        granted_expires: &str,
        why: WhyTheLeaseIsForfeit,
    ) -> Result<Vec<QueuedTrigger>> {
        let now = to_utc_rfc3339(now)?;
        let granted_expires = to_utc_rfc3339(granted_expires)?;
        let tx = rusqlite::Transaction::new_unchecked(
            self.connection(),
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let reclaimed = match why {
            WhyTheLeaseIsForfeit::ItsBoundRanOut => tx.execute(
                "DELETE FROM working_tree_lease
                 WHERE tree = 'default' AND scope_key = ?1 AND expires = ?2 AND expires <= ?3",
                params![holder_key, holder_expires, now],
            )?,
            WhyTheLeaseIsForfeit::ItsHolderIsGone => tx.execute(
                "DELETE FROM working_tree_lease
                 WHERE tree = 'default' AND scope_key = ?1 AND expires = ?2",
                params![holder_key, holder_expires],
            )?,
        };
        if reclaimed != 1 {
            // Renewed, released, or never the row the caller read — dropping the transaction rolls
            // the (no-op) delete back and the caller competes for the lease as it always did.
            return Ok(Vec::new());
        }
        // The release's own reason, from the other entrance (nxf 6j6v.8bv9) — and the withdrawn
        // marker with it (nxf 6j6v.b9nf), for the release's reason too.
        crate::park::forget_park_refusal(&tx, holder_key)?;
        forget_withdrawn_holder(&tx, holder_key)?;
        let next = hand_the_working_tree_on(&tx, &now, &granted_expires)?;
        tx.commit()?;
        Ok(next)
    }

    /// **Record that `scope_key`'s running round was withdrawn and its work is to be parked** (nxf
    /// 6j6v.b9nf) — the marker `orchestration::withdraw` writes when the round it stopped holds
    /// the working copy, and the one `orchestration::park_a_withdrawn_holder` acts on once nothing
    /// in that claim area is running any more.
    ///
    /// One row per claim, in one statement: a second withdrawal of the same holder — a round with
    /// two running sessions taken back in two calls — keeps the FIRST instant and takes the newest
    /// name. What the instant answers is "since when has this holder been waiting to be parked",
    /// which is the same question whichever call asked it last.
    ///
    /// **Where the row goes again — every one of them, so a marker cannot outlive what it is
    /// about**, and it is exactly `park_refusals`' list ([`Self::note_park_refusal`]): the lease
    /// leaving this holder by ANY path — a release and either reclaim, inside the very transaction
    /// that moves it ([`forget_withdrawn_holder`]), and a different scope taking an expired lease in
    /// the acquire compare-and-swap ([`forget_other_withdrawn_holders`]). **And nowhere else.** A
    /// session STARTING in the withdrawn claim does not end the withdrawal — not a follow-up into
    /// the withdrawn thread that resumes the stopped session before the tick parked, not a nested
    /// session under the holder resumed on its own. The first cut dropped the marker at the spawn
    /// funnel for the former, and the review of this item showed what that costs on the latter: a
    /// nested shared session takes no lease and puts nothing of its own in the tree, yet its
    /// resume shares the claim root, so it ended the withdrawal of the exclusive holder above it,
    /// whose normal end then handed the copy on with the half-finished work still in it. Under the
    /// literal rule the early follow-up costs a park the round may not have needed; the alternative
    /// cost the work. `orchestration::park_a_withdrawn_holder` carries the argument.
    pub fn note_withdrawn_holder(&mut self, scope_key: &str, by: &str, now: &str) -> Result<()> {
        let now = to_utc_rfc3339(now)?;
        // **Conditional on the lease it is about** (fix round 3 of this item's review, Code
        // Quality #5). `withdraw` reads the lease row and only afterwards discharges the round and
        // calls this — real work runs in between, so the lease this key names can be gone by the
        // time the write lands (the round's own final reply released it). An unconditional upsert
        // would write a marker for a key that, right now, holds nothing; a LATER acquire by that
        // same claim key — the advertised way back — keeps a marker it finds standing (see this
        // module's `every_lease_movement_forgets_the_withdrawn_marker_of_the_holder_it_moves_away_
        // from`, "an INHERIT keeps it"), so that claim's own ordinary end would decline and park a
        // round that never needed one. `SELECT ... WHERE EXISTS` in the one statement keeps the
        // read and the write atomic — no window between them for the lease to move in.
        self.connection().execute(
            "INSERT INTO withdrawn_holders(scope_key, withdrawn_at, by)
             SELECT ?1, ?2, ?3 WHERE EXISTS (
                 SELECT 1 FROM working_tree_lease WHERE scope_key = ?1
             )
             ON CONFLICT(scope_key) DO UPDATE SET by = excluded.by",
            params![scope_key, now, by],
        )?;
        Ok(())
    }

    /// The marker recorded for `scope_key`, or `None` when its round was not withdrawn (or the copy
    /// has gone on since).
    pub fn withdrawn_holder(&self, scope_key: &str) -> Result<Option<WithdrawnHolder>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT withdrawn_at, by FROM withdrawn_holders WHERE scope_key = ?1",
                params![scope_key],
                |r| {
                    Ok(WithdrawnHolder {
                        withdrawn_at: r.get(0)?,
                        by: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// **Record the sessions a withdrawal of `scope_key` asked to stop, and when** (nxf 6j6v.27b9) —
    /// the only sessions the tick may ever send a further SIGTERM to. Written by
    /// `orchestration::withdraw` beside the marker, for the running sessions inside the claim it
    /// marked, and **conditional on that marker standing** for [`Self::note_withdrawn_holder`]'s own
    /// reason: a row about a claim that is not marked would be about nothing. A session already
    /// recorded keeps its row as it is — a second withdrawal of the same still-running session does
    /// not re-open a further signal already sent, nor restart its clock.
    pub fn note_withdrawn_sessions(
        &mut self,
        scope_key: &str,
        sessions: &[String],
        now: &str,
    ) -> Result<()> {
        let now = to_utc_rfc3339(now)?;
        for session in sessions {
            self.connection().execute(
                "INSERT INTO withdrawn_sessions(scope_key, session, stopped_at)
                 SELECT ?1, ?2, ?3 WHERE EXISTS (
                     SELECT 1 FROM withdrawn_holders WHERE scope_key = ?1
                 )
                 ON CONFLICT(scope_key, session) DO NOTHING",
                params![scope_key, session, now],
            )?;
        }
        Ok(())
    }

    /// **`session` has been put in motion again, so it is not what a withdrawal stopped any more**
    /// (nxf 6j6v.27b9) — called at the one funnel every trigger passes through, AFTER the worker
    /// accepted the spawn. A follow-up that resumes a withdrawn session before the tick parked it
    /// runs a NEW process under the same id, and a further SIGTERM meant for the old one would stop
    /// the work that follow-up asked for. A spawn the worker REFUSED — the stopped process is still
    /// there and holds the session's claim — started nothing, so the row stays and the old process
    /// is still the one the tick may ask again. A no-op for a session no withdrawal recorded.
    pub fn forget_withdrawn_session(&mut self, session: &str) -> Result<()> {
        self.connection().execute(
            "DELETE FROM withdrawn_sessions WHERE session = ?1",
            params![session],
        )?;
        Ok(())
    }

    /// The row the withdrawal of `scope_key` left for `session`, or `None` if it did not stop that
    /// session (or the session has been put back in motion since).
    pub fn withdrawn_session(
        &self,
        scope_key: &str,
        session: &str,
    ) -> Result<Option<WithdrawnSession>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT stopped_at, resignalled_at, resignal_refused FROM withdrawn_sessions
                  WHERE scope_key = ?1 AND session = ?2",
                params![scope_key, session],
                |r| {
                    Ok(WithdrawnSession {
                        stopped_at: r.get(0)?,
                        resignalled_at: r.get(1)?,
                        resignal_refused: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// **Claim the one further SIGTERM for `session`, BEFORE it is sent** (nxf 6j6v.27b9): a
    /// compare-and-swap on `resignalled_at`, `true` only for the one call that set it. Two ticks
    /// that overlap — the background service's and a person's `nxc tick` — both read "not sent
    /// yet"; only one of them wins this statement, and only the winner signals. Claiming first is
    /// what the "stamped whatever came of it" rule costs nothing to keep: a signal that fails after
    /// its claim is recorded as refused ([`Self::note_withdrawn_session_resignal_refused`]), never
    /// retried.
    pub fn claim_withdrawn_session_resignal(
        &mut self,
        scope_key: &str,
        session: &str,
        now: &str,
    ) -> Result<bool> {
        let now = to_utc_rfc3339(now)?;
        Ok(self.connection().execute(
            "UPDATE withdrawn_sessions SET resignalled_at = ?3
              WHERE scope_key = ?1 AND session = ?2 AND resignalled_at IS NULL",
            params![scope_key, session, now],
        )? == 1)
    }

    /// Record that the further SIGTERM this tick claimed did not land, in the worker's own words —
    /// so no later tick says "sent" about it.
    pub fn note_withdrawn_session_resignal_refused(
        &mut self,
        scope_key: &str,
        session: &str,
        why: &str,
    ) -> Result<()> {
        self.connection().execute(
            "UPDATE withdrawn_sessions SET resignal_refused = ?3
              WHERE scope_key = ?1 AND session = ?2",
            params![scope_key, session, why],
        )?;
        Ok(())
    }

    /// **Every `withdrawn_holders` row, by scope key** — `nxc status`'s own read
    /// ([`crate::facade::withdrawn_holders_for`]), in [`ChatStore::all_park_refusals`]'s shape: one
    /// statement over a table that holds at most one row per claim, so a report over many
    /// operations pays for this once rather than once per candidate thread.
    pub fn all_withdrawn_holders(
        &self,
    ) -> Result<std::collections::HashMap<String, WithdrawnHolder>> {
        let conn = self.connection();
        let mut st = conn.prepare("SELECT scope_key, withdrawn_at, by FROM withdrawn_holders")?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    WithdrawnHolder {
                        withdrawn_at: r.get(1)?,
                        by: r.get(2)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?;
        Ok(rows)
    }

    /// Enqueue a trigger that lost the working-tree lease race, returning its assigned `id`.
    ///
    /// The ORDERING key is `q.priority`'s canonical ORDINAL, not [`crate::model::Priority`]'s serde
    /// label — smaller sorts more urgent. `crates/chat/src/model.rs::Priority` has no ordinal
    /// persisted anywhere else in this crate: `messages.priority` (the only other place this crate
    /// persists a `Priority`, via `message_reducer.rs`) stores the LABEL (`"urgent"`) because that
    /// column is user-facing history, not an ordering key. This table's `priority` IS an ordering
    /// key — the `ORDER BY` below sorts on it directly — so it takes the ordinal
    /// [`crate::model::Priority`]'s OWN declaration spells out (`Urgent = 0`, `Normal = 1`,
    /// `Background = 2`), converted here and read back by [`priority_from_ordinal`] rather than by
    /// a second, independent numbering nothing else in the crate uses. Those discriminants are
    /// written out explicitly THERE, not merely implied by declaration order, precisely because
    /// this column makes them a persisted format — see that enum's own doc.
    ///
    /// `now` is normalized to UTC via [`to_utc_rfc3339`] before it is stored as `enqueued_at`, for
    /// the same reason [`Self::acquire_working_tree`] normalizes `now`/`expires`: `enqueued_at` is an
    /// ORDERING key compared as a raw SQL byte string (`ORDER BY priority, enqueued_at, id`), which
    /// only sorts correctly when every stored value shares one offset and precision.
    pub fn enqueue_working_tree(&mut self, q: &QueuedTrigger, now: &str) -> Result<i64> {
        // `now` is the stamp for an entry meeting the queue for the FIRST time; an entry that
        // carries one already is a promotion going back into the queue, and it keeps the place in
        // line it has already waited for — see [`QueuedTrigger::enqueued_at`].
        let enqueued_at = to_utc_rfc3339(q.enqueued_at.as_deref().unwrap_or(now))?;
        let model = q.model.map(|m| {
            serde_json::to_value(m)
                .expect("Model serializes")
                .as_str()
                .expect("Model serializes to a string")
                .to_string()
        });
        self.connection().execute(
            "INSERT INTO working_tree_queue(
                 scope_key, role, session, thread, message, model, depth, priority, enqueued_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                q.scope_key,
                q.role,
                q.session,
                q.thread,
                q.message,
                model,
                q.depth,
                q.priority as i64,
                enqueued_at,
            ],
        )?;
        Ok(self.connection().last_insert_rowid())
    }

    /// The queue's current head — the entry [`Self::take_working_tree_queue`] would return — without
    /// consuming it. `None` when the queue is empty.
    pub fn peek_working_tree_queue(&self) -> Result<Option<QueuedTrigger>> {
        Ok(self
            .connection()
            .query_row(
                &format!(
                    "SELECT {QUEUE_COLS} FROM working_tree_queue
                     ORDER BY priority, enqueued_at, id
                     LIMIT 1"
                ),
                [],
                queued_trigger_from_row,
            )
            .optional()?)
    }

    /// Every currently queued trigger, in the SAME `(priority, enqueued_at, id)` order
    /// [`Self::take_working_tree_queue`] drains it in — the bulk read `threads list`/`show`'s
    /// working-tree visibility needs (nxf 6j6v.qk5b): which thread is waiting, and at what
    /// position, in ONE query over the whole queue rather than [`Self::working_tree_queue_position`]
    /// called once per thread a board happens to list. A plain, non-consuming `SELECT` — unlike
    /// [`Self::peek_working_tree_queue`] this returns every row, not just the head, and unlike
    /// [`Self::take_working_tree_queue`] it removes nothing.
    pub fn list_working_tree_queue(&self) -> Result<Vec<QueuedTrigger>> {
        let sql = format!(
            "SELECT {QUEUE_COLS} FROM working_tree_queue ORDER BY priority, enqueued_at, id"
        );
        let conn = self.connection();
        let mut st = conn.prepare(&sql)?;
        let rows = st
            .query_map([], queued_trigger_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Read AND remove the queue's head in one statement, so a second process racing this one
    /// cannot pull the same head — the same single-statement requirement
    /// [`Self::acquire_working_tree`]'s doc explains for the lease CAS, here satisfied by SQLite's
    /// `DELETE ... RETURNING` rather than `INSERT ... ON CONFLICT`. `None` when the queue is empty.
    ///
    /// The release path does NOT call this, for two reasons: it needs the take and its own release
    /// to be one indivisible step, and what it takes is the head's whole CLAIM AREA rather than the
    /// single head ([`take_queue_scope`] says why). It therefore runs its own take inside its
    /// transaction ([`Self::release_working_tree_and_take_next`]); this is the standalone,
    /// single-statement pop, and the two share only the [`QUEUE_COLS`] shape.
    pub fn take_working_tree_queue(&mut self) -> Result<Option<QueuedTrigger>> {
        take_queue_head(self.connection())
    }

    /// `id`'s 1-based position in the same `(priority, enqueued_at, id)` order `take` drains in —
    /// for the receipt a caller reports back to whoever got queued ("you are #3"). Errors if `id` is
    /// not currently queued (already taken, or never enqueued).
    pub fn working_tree_queue_position(&self, id: i64) -> Result<i64> {
        let target: Option<(i64, String)> = self
            .connection()
            .query_row(
                "SELECT priority, enqueued_at FROM working_tree_queue WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (priority, enqueued_at) = target.ok_or_else(|| {
            NxfError::not_found(format!("no working-tree queue entry with id {id}"))
        })?;
        let ahead: i64 = self.connection().query_row(
            "SELECT COUNT(*) FROM working_tree_queue
             WHERE priority < ?1
                OR (priority = ?1 AND enqueued_at < ?2)
                OR (priority = ?1 AND enqueued_at = ?2 AND id < ?3)",
            params![priority, enqueued_at, id],
            |r| r.get(0),
        )?;
        Ok(ahead + 1)
    }

    /// Remove ONE entry by id, reporting whether it was still there. Not a drain and not a
    /// scheduling decision: this is how a caller that just enqueued takes its OWN entry back out
    /// (nxf 6j6v.303b, PR review Finding 3).
    ///
    /// **Why by id rather than [`Self::take_working_tree_queue`]**: `take` pops the HEAD, which is
    /// whatever the ordering says is most urgent — not necessarily the caller's own entry. A caller
    /// undoing its own enqueue must be unable to swallow somebody else's turn, and an id is the only
    /// thing that says "mine". The scheduling pop stays `take`; this is its non-scheduling twin.
    ///
    /// `false` — the entry is already gone — is an ordinary answer, not an error: a release path
    /// draining the queue (nxf 6j6v.fe0f) can legitimately have taken it in between.
    pub fn remove_working_tree_queue_entry(&mut self, id: i64) -> Result<bool> {
        let removed = self
            .connection()
            .execute("DELETE FROM working_tree_queue WHERE id = ?1", params![id])?;
        Ok(removed == 1)
    }

    /// The threads that make up `scope` — the conversations whose outstanding replies decide when
    /// the chain holding the working tree is finished (nxf 6j6v.fe0f), **a SUBTREE since nxf
    /// 6j6v.1xw1 rather than one level of it, and rooted at the OPERATION since nxf 6j6v.8y6t**.
    ///
    /// - [`WorkScope::Thread`]: that thread and everything hanging beneath it, at any depth
    ///   ([`ChatStore::thread_subtree`]).
    /// - [`WorkScope::Session`]: none, and that is not an omission. A session-scoped trigger takes
    ///   no lease in the first place (nxf 6j6v.303b: it has no thread to declare an expectation on,
    ///   so nothing could ever discharge one). Should such a row exist anyway, an empty thread set
    ///   makes [`Self::work_scope_has_outstanding`] answer `false` — and the release path's own
    ///   in-scope gate then never matches it, so it is left to the two-hour backstop rather than
    ///   released by whichever reply happened to run next.
    ///
    /// **Why a subtree, and where it starts** (nxf 6j6v.1xw1, the owner's decision of 2026-08-14;
    /// re-anchored by nxf 6j6v.8y6t, decision (a) of 2026-08-23). A subtree is what keeps the
    /// protection standing while `#coding` waits on `#review`: the review round and its members hang
    /// under the coding member that opened them, so the gap between "the coder is done" and "the
    /// review is in" is INSIDE the area rather than a hole in it. That half is unchanged.
    ///
    /// What moved is the ROOT of that subtree. 1xw1 anchored it at the CLAIM THREAD — the thread of
    /// the channel that declared the need — and stopped short of the conversation above it, so that
    /// one unanswered human could not hold this device's working copy until tomorrow. Measured on
    /// 2026-08-22, that boundary cost two things: the claim fell as soon as ONE round was worked off,
    /// leaving a window between two coding rounds of the same epic that a foreign chain could step
    /// into; and an operation whose exclusive step comes late could be overtaken by one that started
    /// later. So the anchor is now the OPERATION'S ROOT
    /// ([`ChatStore::thread_root`], where the term is defined), and the claim is given back when the
    /// OPERATION is finished.
    ///
    /// **The risk 1xw1 named comes back with it, and is answered rather than dropped**: the
    /// conversation with the human is now inside the area, so an operation nobody finishes holds the
    /// checkout. What bounds it is the other half of 6j6v.8y6t — the lease's expiry is the
    /// operation's own declared windows ([`Self::declared_window_end`]) rather than a flat two hours,
    /// and the sweep reclaims an expired lease whose processes are gone. And the claim still ARISES
    /// only at the first exclusive step, so an operation that never reaches one never holds anything.
    ///
    /// **This is also what retires the two gaps a `run:` scope had**, both of them consequences of
    /// `run:` being an area that does NOT contain its threads where a tree does:
    ///
    /// * a 1:1 DM opened by a role that is mid-run used to be nowhere in the area, so the reply that
    ///   answered it released nothing and the board showed it as belonging to no lease at all (nxf
    ///   6j6v.fm8k). It is a CHILD of the thread its session was standing in, so a tree contains it.
    /// * the same for anything a channel member opens out of its own conversation, which used to be
    ///   one level too deep for [`ChatStore::supervised_children`] to reach.
    pub fn work_scope_threads(&self, scope: &WorkScope) -> Result<Vec<String>> {
        match scope {
            WorkScope::Thread(thread_id) => self.thread_subtree(thread_id),
            WorkScope::Session(_) => Ok(Vec::new()),
        }
    }

    /// Whether ANY thread of `scope` still carries an expected handle that has not answered — the
    /// release rule's whole question (nxf 6j6v.fe0f), and the reason this epic can free the working
    /// copy on a FACT rather than on an estimate of quiet.
    ///
    /// **The derivation is [`ChatStore::thread_quorums`], reused verbatim** — the same one
    /// `threads list`, the app-facade quorum bar and `reply --if-unanswered` (nxf 6j6v.ww0a) all read,
    /// down to [`crate::store::ThreadQuorum::outstanding`] being the exact set consulted. A second
    /// SQL answering "does anybody still owe a reply here?" is the drift this repo has now written
    /// down at four separate call sites; one bulk read over the scope's threads costs nothing beside
    /// it and cannot disagree with what the board shows a human.
    ///
    /// **The `Run` arm is GONE, not replaced** (nxf 6j6v.1xw1, acceptance point 6). Until this item
    /// this function answered `true` for a run that was still `Running` whatever its boards said, and
    /// the argument for that clause was:
    ///
    /// > A workflow ROLE step registers no obligation anywhere (the run engine's own step-firing
    /// > triggered with no thread; nxf 6j6v.jepk scoped `expects_reply_from` to
    /// > the persona coordinator). So the moment a
    /// > channel step's completing reply advances a run onto a role step, every board of that run is
    /// > answered while the chain is very much alive — and without this clause the working copy would
    /// > be handed to a rival chain WHILE the role step runs.
    ///
    /// That is 6j6v.rp8k, and the clause was 6j6v.fe0f's bridge over it, taken with two costs stated
    /// at the time: a wedged run (a failed advance, or a terminal role step that answers nothing)
    /// held the working copy to [`WORKING_TREE_LEASE_BOUND`], and the release came when the RUN ended
    /// rather than when the step was done.
    ///
    /// **It no longer holds, because the call path it protects no longer exists on this surface.** A
    /// channel supervisor commissions a member through `send --to <persona>` —
    /// `orchestration::coordinator_commission`, which since 6j6v.jepk declares the expectation on
    /// the
    /// fresh thread it opens — so a member is never started without a thread and without an
    /// obligation (pinned by `orchestration_verbs::
    /// every_member_is_triggered_as_a_persona_owing_a_reply_on_its_own_fresh_thread`). A thread-less
    /// trigger cannot arise there at all. What replaces the clause is the SUBTREE
    /// ([`Self::work_scope_threads`]): the claim area now CONTAINS the threads the work actually
    /// happens in, which is the property `run:<id>` never had and the reason the bridge was needed.
    ///
    /// **The residual this paragraph used to name is CLOSED** (6j6v.rp8k, closed 2026-08-21). It
    /// read: the legacy `workflow start` path still reaches the run engine's step-firing, so a run
    /// of that shape with an `exclusive` role step loses the backstop this clause gave it and can
    /// release while that step runs. 6j6v.dvyq §3 removed that path — verb, seam and run record
    /// together — which is exactly what rp8k's own closing condition asked for, so the shape is not
    /// merely unreachable from the surface but unconstructible.
    ///
    /// `false` for a scope with no threads at all — see [`Self::work_scope_threads`]. Threads that
    /// do not exist in the `threads` view simply do not come back from the bulk read, which is the
    /// same answer for the same reason: no register, nothing owed.
    ///
    /// **The scope's size is bounded before it is bound** (branch review of PR #337, Integrity #5).
    /// Since a channel scope pulls in the whole member fan-out ([`Self::work_scope_threads`]), the
    /// id list is no longer "a run's handful of boards" and is not bounded by anything a caller
    /// controls. One SQL variable per id against SQLite's 999-variable default therefore needs the
    /// SAME guard `nxc status` has had since 6j6v.a71h — [`crate::facade::MAX_BOUND_IDS`], the one
    /// constant, not a second copy of the number: past the ceiling this reads the workspace-wide
    /// quorums and filters in memory. Same answer, one query, no bind list. The alternative was an
    /// `Err` on the one path whose failure mode is a lease held to its two-hour bound.
    pub fn work_scope_has_outstanding(&self, scope: &WorkScope, now: &str) -> Result<bool> {
        self.threads_have_outstanding(&self.work_scope_threads(scope)?, now)
    }

    /// [`Self::work_scope_has_outstanding`] over an already-resolved thread list — the form the
    /// release path uses, for [`Self::threads_handed_back`]'s reason (branch review, Minor 8).
    pub(crate) fn threads_have_outstanding(&self, threads: &[String], now: &str) -> Result<bool> {
        Ok(!self.threads_owing_an_answer(threads, now)?.is_empty())
    }

    /// **WHICH of `threads` still carry an expected handle that has not answered** — the same
    /// derivation [`Self::threads_have_outstanding`] asks for a yes/no, with the threads themselves
    /// (nxf 6j6v.xb24).
    ///
    /// It exists because "is this holder DEAD or merely waiting?" cannot be answered by a bool:
    /// `orchestration::holder_is_provably_dead` has to look at the sessions of the threads that owe,
    /// and only of those. A thread that owes nobody anything has nothing to be dead in the middle
    /// of.
    ///
    /// **The bool above is now this, emptied** rather than a second `.any` over the same rows. That
    /// is this predicate's own rule, written down where it is derived: *"a second SQL answering 'does
    /// anybody still owe a reply here?' is the drift this repo has now written down at four separate
    /// call sites"*. The short-circuit the `.any` gave up costs nothing — [`ChatStore::thread_quorums`]
    /// has already materialised every row by the time either of them looks.
    ///
    /// The bound-id ceiling and its in-memory fallback are [`Self::threads_have_outstanding`]'s, for
    /// its reason, and the order is the caller's own `threads` order, which is deterministic.
    pub(crate) fn threads_owing_an_answer(
        &self,
        threads: &[String],
        now: &str,
    ) -> Result<Vec<String>> {
        if threads.is_empty() {
            return Ok(Vec::new());
        }
        let owing: BTreeSet<String> = if threads.len() > crate::facade::MAX_BOUND_IDS {
            let wanted: BTreeSet<&str> = threads.iter().map(String::as_str).collect();
            self.thread_quorums_all(now)?
                .into_iter()
                .filter(|q| wanted.contains(q.thread_id.as_str()) && !q.outstanding.is_empty())
                .map(|q| q.thread_id)
                .collect()
        } else {
            let ids: Vec<&str> = threads.iter().map(String::as_str).collect();
            self.thread_quorums(&ids, now)?
                .into_iter()
                .filter(|q| !q.outstanding.is_empty())
                .map(|q| q.thread_id)
                .collect()
        };
        Ok(threads
            .iter()
            .filter(|t| owing.contains(t.as_str()))
            .cloned()
            .collect())
    }

    /// **Was the claim HANDED BACK rather than closed?** — the other half of nxf 6j6v.1xw1's
    /// release rule, and the half [`Self::work_scope_has_outstanding`] structurally cannot answer.
    ///
    /// The item's rule is "release when the CLAIM THREAD IS CLOSED — not when nothing happens to be
    /// outstanding", and the difference between those two carries the owner's exception:
    ///
    /// > Einzige Ausnahme: eine Frage (`escalate`) dringt bis zum Menschen (T1) vor. Die Aufgabe ist
    /// > noch mitten im Gange und der Arbeitsbereich muss weiter geschuetzt sein.
    ///
    /// "Closed" means the expected party has written a NON-hand-back reply since the last obligation.
    /// Read against the three answers [`ChatStore::last_reply_kind`] can give, the two rules line up
    /// exactly:
    ///
    /// * *nobody has answered this turn* — not closed, and [`Self::work_scope_has_outstanding`]
    ///   already says so, because the register still names the party that owes.
    /// * *somebody said it is finished* — closed. Both rules agree, and the lease goes.
    /// * *somebody handed it back* — not closed, and here the two rules DISAGREE: an escalating reply
    ///   discharges the sender's turn exactly like a finished one (6j6v.cg8g gives it no special case
    ///   in the predicate, deliberately), so nothing is outstanding and the quorum rule alone would
    ///   release. This is the clause that stops it.
    ///
    /// **Which kinds hand the task back** is [`hands_the_task_back`], and the answer is two of them:
    /// an escalation ("I need help, or a decision") and an open QUESTION ("what do you mean by X?").
    /// The second is the owner's ruling of 2026-08-16 and is worth its own sentence, because
    /// 6j6v.wt37 keeps the two deliberately APART as facts: an escalation is a statement about the
    /// sender's OWN work and a question is not. For the CLAIM
    /// the difference is no difference — in both shapes nobody is working while the task is
    /// mid-flight, and releasing lets a rival start into a working copy the questioner will resume
    /// into, which is the collision epic 6j6v.bqe0 exists to prevent.
    ///
    /// **Which threads are asked: the WHOLE claim area** ([`Self::work_scope_threads`]) — the
    /// operation's subtree. One question, one list, the same list the outstanding rule reads.
    ///
    /// **A first cut of this asked only the claim thread and its supervised children, and it released
    /// the working copy early.** The argument for narrowing was: a hand-back deeper down is already
    /// covered, because if the level above it has not acted, that level's own thread is outstanding;
    /// and if it HAS acted, the hand-back is answered. The second half is FALSE, and the shape that
    /// shows it is ordinary rather than exotic — **the level above can act by CONSOLIDATING THE SET
    /// rather than by answering**, and a question is exactly the kind that lets it:
    ///
    /// * a question discharges the sender's turn (6j6v.cg8g), so the member set settles;
    /// * it is not an escalation, so `set_escalated` is false and the round folds or passes through
    ///   normally ([`crate::channel::ChannelDecl::consolidation`]);
    /// * the questioner is **never resumed** — nothing in `orchestration` treats a question as
    ///   anything but an answer.
    ///
    /// So a reviewer's question two levels down leaves every thread above it discharged and nothing
    /// outstanding anywhere, while that reviewer sits waiting for an answer that is not coming. A
    /// narrowed read cannot see it, and the working copy goes to a rival — the malign direction, on a
    /// mechanism whose benign one is holding too long. Pinned by
    /// `working_tree_claim_scope.rs::a_question_two_levels_down_holds_the_claim_although_every_thread_above_it_consolidated`.
    ///
    /// **And the over-hold this was traded against does not exist**, which is the other half worth
    /// writing down because it was argued rather than tested the first time. A subtree-wide read is
    /// SELF-CLEARING, by the turn watermark: answering a hand-back means re-commissioning that round —
    /// a reply into the channel thread from its requester, which re-declares every member's
    /// expectation (6j6v.pf6j) and moves `expects_reply_from_v`. [`ChatStore::last_reply_kind`] is
    /// scoped to the current declaration, so the moment the turn is re-opened it answers `None` on the
    /// questioner's thread and the hold is gone, at once and long before the backstop. Two tests show
    /// it end to end, one at the claim thread and one two levels down:
    /// `…::an_answered_escalation_releases_the_claim_before_the_two_hour_bound` and
    /// `…::an_answered_question_deep_in_the_subtree_clears_the_hold_before_the_bound`.
    ///
    /// **That door is the channel one, and it is not the only thread in an area** (branch re-review
    /// of this item). A hand-back written on an ad-hoc DM — a thread a session opened out of its own
    /// conversation rather than a supervisor-opened member thread — has no supervisor to re-declare
    /// it: `orchestration::resume_return_address` puts the answering session back to work but
    /// registers no obligation, so the watermark does not move and this keeps reading the hand-back
    /// until that session replies THERE itself. The direction is the benign one (the copy is held a
    /// little longer than the conversation warrants, never released early), and it is bounded by
    /// [`WORKING_TREE_LEASE_BOUND`] like every other unanswered hand-back.
    ///
    /// What is left is not an over-hold but the correct answer to a case the store cannot tell apart:
    /// a hand-back that nobody ever answers holds the copy until [`WORKING_TREE_LEASE_BOUND`]. That is
    /// this epic's own stated fail direction, and it is bounded.
    ///
    /// A [`WorkScope::Session`] has no threads, so this is `false` there — the same answer, for the
    /// same reason, as [`Self::work_scope_has_outstanding`].
    pub fn work_scope_handed_back(&self, scope: &WorkScope) -> Result<bool> {
        self.threads_handed_back(&self.work_scope_threads(scope)?)
    }

    /// [`Self::work_scope_handed_back`] over an already-resolved thread list — the form the release
    /// path uses so that one reply costs ONE [`Self::work_scope_threads`] call rather than three
    /// (branch review of this item, Minor 8). `pub(crate)`: the scope-shaped question is the public
    /// one, and this exists only so its two readers cannot resolve the same area twice.
    ///
    /// **One query per thread, deliberately, and what that costs.** Unlike
    /// [`Self::threads_have_outstanding`] — one bulk read over the whole list — this asks
    /// [`ChatStore::last_reply_kind`] per thread, because that is the ONE derivation of "what did
    /// this thread's current turn answer" and a bulk twin of it would be a second copy of the
    /// comparison this crate has already been bitten by four times. Two things keep the cost bounded
    /// rather than merely tolerable: it SHORT-CIRCUITS on the first hand-back, and the release path
    /// asks question (2) first, so this runs only on a reply that would otherwise RELEASE — the last
    /// reply of a chain, not every reply in it. Its width is the claim area, which is bounded by how
    /// many threads one chain opened.
    pub(crate) fn threads_handed_back(&self, threads: &[String]) -> Result<bool> {
        for thread_id in threads {
            if let Some(kind) = self.last_reply_kind(thread_id)? {
                if hands_the_task_back(&kind) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// **Is this claim area sitting on a hand-back that NOBODY HAS ANSWERED?** (nxf 6j6v.de9s) —
    /// the predicate the park's contention clock is derived from, and one question finer than
    /// [`Self::threads_handed_back`] next door.
    ///
    /// That one asks *what did the last REPLY say*, and its answer stays `true` for as long as the
    /// escalating party has not replied again — which is correct for the LEASE, whose rule is that a
    /// hand-back does not release. It is wrong for a PARK. A human who answers an escalation puts the
    /// operation back to work without becoming an expected replier, so the escalation goes on being
    /// the last reply while somebody is once again writing in the working copy — and parking there
    /// would commit an operation's work out from under the session that is doing it.
    ///
    /// So this asks the question the item's own title asks: *unanswered*. The newest message ANYWHERE
    /// in the claim area, from anybody, whether or not the thread expects them — and whether THAT
    /// hands the task back. An answer, a follow-up, a nudge: any of them means the escalation is no
    /// longer the last word, and the wait this mechanism exists to end is over.
    ///
    /// **`sessions_still_running` is not a substitute for it**, and it is worth saying why, because
    /// the park asks that too. Process liveness answers "is somebody writing right now", which is a
    /// race: a resumed session that has not yet been spawned, or one between turns, is not running
    /// and is very much not stranded. This is a fact about the conversation and it does not flicker.
    ///
    /// One query per thread, like [`Self::threads_handed_back`] and bounded the same way — the area
    /// is one operation's subtree, and the caller reaches this only when the queue is non-empty and
    /// nothing in the area is outstanding.
    ///
    /// **The plain newest message, not "since the declaration"** (which is what
    /// [`ChatStore::last_reply_kind`] filters on). That filter exists to answer "what did the
    /// CURRENT turn say"; here the question is whether anything at all has been said since, and a
    /// message that predates a re-declaration is still something that was said.
    ///
    /// **Said by somebody this replica can vouch for** (nxf 6j6v.pzkb): the answer decides whether
    /// the tick puts work aside in the working copy, so a message whose op may not carry an action
    /// neither causes that nor prevents it.
    pub(crate) fn threads_await_an_unanswered_hand_back(&self, threads: &[String]) -> Result<bool> {
        let conn = self.connection();
        let mut st = conn.prepare(&format!(
            "SELECT m.lamport, m.site, m.message_id, m.kind FROM messages m
              WHERE m.thread_id = ?1 AND {}
              ORDER BY m.lamport DESC, m.site DESC, m.message_id DESC
              LIMIT 1",
            crate::store::acts("m")
        ))?;
        let mut newest: Option<(i64, i64, String, String)> = None;
        for thread_id in threads {
            let row = st
                .query_row(params![thread_id], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })
                .optional()?;
            let Some(row) = row else { continue };
            let newer = match &newest {
                None => true,
                Some(seen) => (row.0, row.1, row.2.as_str()) > (seen.0, seen.1, seen.2.as_str()),
            };
            if newer {
                newest = Some(row);
            }
        }
        Ok(newest.is_some_and(|n| hands_the_task_back(&n.3)))
    }

    /// **The last window this claim area itself declared** (nxf 6j6v.8y6t) — the latest deadline
    /// among the obligations currently outstanding in `threads`, or `None` when there is no such
    /// bound to derive.
    ///
    /// `None` has exactly two causes, and they are one answer: **nothing is outstanding yet**, or
    /// **at least one outstanding obligation carries no deadline at all**. In both the area has made
    /// no promise this lease could be held to, so the caller
    /// ([`crate::orchestration::operation_lease_expiry`]) falls back to
    /// [`WORKING_TREE_LEASE_BOUND`]. That is the answer to the owner's first question — what applies
    /// to a step that declares no timeout — and it is deliberately ALL-or-nothing rather than a max
    /// over the declared subset: taking the maximum of what happens to be declared would let a
    /// five-minute coding step decide the bound for an unbounded conversation beside it, and expire
    /// the lease under work that never said when it would be done. One undeclared window makes the
    /// whole area undeclared.
    ///
    /// **"Its own deadline" is [`crate::store::ThreadQuorum::deadline`]**, the one register `stale`
    /// is derived from — the channel's declared `timeout:` resolved onto the member thread
    /// (`channel_member_deadline`), else the thread's own `--deadline`. So this reads the engine's
    /// existing fallback chain rather than a second one, and a step with no window anywhere in that
    /// chain is exactly the `None` above.
    ///
    /// Deadlines are compared as strings, which is sound for the same reason
    /// [`ChatStore::acquire_working_tree`]'s own comparison is: every deadline reaching the register
    /// has been normalized to UTC RFC3339 by `facade::resolve_instant`, and the result is
    /// re-normalized here before it is handed on.
    ///
    /// Bounded exactly as [`Self::threads_have_outstanding`] is, and for the same reason — the area
    /// is a whole operation since 6j6v.8y6t, so the id list is not something a caller controls.
    pub(crate) fn declared_window_end(
        &self,
        threads: &[String],
        now: &str,
    ) -> Result<Option<String>> {
        if threads.is_empty() {
            return Ok(None);
        }
        let quorums = if threads.len() > crate::facade::MAX_BOUND_IDS {
            let wanted: BTreeSet<&str> = threads.iter().map(String::as_str).collect();
            self.thread_quorums_all(now)?
                .into_iter()
                .filter(|q| wanted.contains(q.thread_id.as_str()))
                .collect()
        } else {
            let ids: Vec<&str> = threads.iter().map(String::as_str).collect();
            self.thread_quorums(&ids, now)?
        };
        let mut latest: Option<String> = None;
        for q in quorums.iter().filter(|q| !q.outstanding.is_empty()) {
            // **An obligation with no readable window is an obligation with no window** (review of
            // PR #376, Integrity #3). Two shapes, one answer: the column is NULL, or it holds
            // something that is not an RFC3339 instant.
            //
            // The second used to propagate, and that was an asymmetry rather than rigour: the
            // `stale` register next door reads the SAME column with a plain string comparison
            // (`crate::store`'s `assemble_quorums`), so a corrupt value is tolerated there while it
            // would have hard-failed every acquire here — one bad row taking out every commission in
            // the workspace. Falling back to the bound instead holds the checkout LONGER, which is
            // this epic's own benign direction, and leaves the row to whoever wrote it.
            let Some(deadline) = q.deadline.as_deref().and_then(|d| to_utc_rfc3339(d).ok()) else {
                return Ok(None);
            };
            if latest
                .as_deref()
                .is_none_or(|seen| seen < deadline.as_str())
            {
                latest = Some(deadline);
            }
        }
        Ok(latest)
    }

    /// How many triggers are currently waiting.
    pub fn working_tree_queue_len(&self) -> Result<i64> {
        Ok(self
            .connection()
            .query_row("SELECT COUNT(*) FROM working_tree_queue", [], |r| r.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A thread that OWES an answer, optionally with `deadline` on its register — the two facts
    /// [`ChatStore::declared_window_end`] reads and nothing else.
    fn owing_thread(s: &mut ChatStore, tid: &str, parent: Option<&str>, deadline: Option<&str>) {
        s.open_thread(
            tid,
            &crate::model::ThreadRoot {
                origin: "o".into(),
                channel_id: "c-1".into(),
                opener: "o/pm".into(),
                created: "2026-08-22T09:00:00Z".into(),
                parent: parent.map(str::to_string),
            },
            "o/pm",
        );
        s.set_expects_reply_from(tid, r#"["o/coder"]"#, "o/pm");
        if let Some(deadline) = deadline {
            s.set_deadline(tid, deadline, "o/pm");
        }
    }

    const T0: &str = "2026-08-22T09:00:00Z";

    /// **The all-or-nothing rule, in one table** (nxf 6j6v.8y6t; branch coverage added by the review
    /// of PR #376, Test Quality #1 and Integrity #3, which found every one of these arms untested).
    ///
    /// `None` means "this area declared no bound", which the caller turns into
    /// [`WORKING_TREE_LEASE_BOUND`]. It is the answer for an empty area, for an area where nothing is
    /// outstanding, and — the load-bearing one — for an area where ANY outstanding obligation is
    /// missing a usable window, whether the column is NULL or holds something unreadable.
    #[test]
    fn an_area_declares_a_window_only_when_every_outstanding_obligation_does() {
        let mut s = ChatStore::open_in_memory(1);
        assert_eq!(
            s.declared_window_end(&[], T0).unwrap(),
            None,
            "an empty claim area has nothing to derive a bound from"
        );

        owing_thread(&mut s, "t-a", None, Some("2026-08-22T09:30:00Z"));
        owing_thread(&mut s, "t-b", Some("t-a"), Some("2026-08-22T10:15:00Z"));
        let area = vec!["t-a".to_string(), "t-b".to_string()];
        assert_eq!(
            s.declared_window_end(&area, T0).unwrap().as_deref(),
            Some("2026-08-22T10:15:00Z"),
            "the LATEST of the declared windows — the operation is not done until its outermost \
             obligation is"
        );

        // One more obligation, with no window of its own: the whole area is now undeclared.
        owing_thread(&mut s, "t-c", Some("t-b"), None);
        let mut wider = area.clone();
        wider.push("t-c".to_string());
        assert_eq!(
            s.declared_window_end(&wider, T0).unwrap(),
            None,
            "ONE obligation without a window makes the area undeclared — a five-minute step must \
             not decide the bound for an unbounded conversation beside it"
        );

        // A thread nobody owes anything on contributes nothing either way.
        let mut s2 = ChatStore::open_in_memory(1);
        owing_thread(&mut s2, "t-done", None, Some("2026-08-22T09:30:00Z"));
        s2.set_expects_reply_from("t-done", "[]", "o/pm");
        assert_eq!(
            s2.declared_window_end(&["t-done".to_string()], T0).unwrap(),
            None,
            "nothing is outstanding, so there is no obligation whose window could bound the lease"
        );
    }

    /// **A deadline the register cannot parse reads as NO window, not as an error** (review of PR
    /// #376, Integrity #3). It used to propagate, which meant one corrupt row hard-failed every
    /// commission in the workspace — while the `stale` register next door tolerated the same value
    /// with a plain string comparison. The fallback holds the checkout longer, which is this epic's
    /// own benign direction.
    #[test]
    fn a_deadline_that_cannot_be_parsed_falls_back_to_the_bound_instead_of_failing_the_acquire() {
        let mut s = ChatStore::open_in_memory(1);
        owing_thread(&mut s, "t-bad", None, Some("not-an-instant"));
        assert_eq!(
            s.declared_window_end(&["t-bad".to_string()], T0).unwrap(),
            None,
            "unreadable is treated exactly like undeclared — and it does not error"
        );
    }

    /// **Past [`crate::facade::MAX_BOUND_IDS`] the answer is the same, by the other query** (review
    /// of PR #376, Test Quality #1). The wide branch reads the workspace's quorums and filters in
    /// memory instead of binding one SQL variable per id; the guard exists because a claim area is a
    /// whole operation and its size is not something a caller controls.
    #[test]
    fn the_wide_read_past_the_bind_ceiling_answers_what_the_bound_read_would() {
        let mut s = ChatStore::open_in_memory(1);
        let n = crate::facade::MAX_BOUND_IDS + 1;
        let mut area = Vec::with_capacity(n);
        for i in 0..n {
            let tid = format!("t-{i:04}");
            // The LATEST window sits in the middle, so a read that truncated or mis-ordered the set
            // would answer something else.
            let deadline = if i == n / 2 {
                "2026-08-22T18:00:00Z"
            } else {
                "2026-08-22T09:30:00Z"
            };
            owing_thread(&mut s, &tid, None, Some(deadline));
            area.push(tid);
        }
        assert!(area.len() > crate::facade::MAX_BOUND_IDS);
        assert_eq!(
            s.declared_window_end(&area, T0).unwrap().as_deref(),
            Some("2026-08-22T18:00:00Z"),
            "the wide branch finds the same latest window the bound branch would"
        );
    }

    #[test]
    fn resolve_scope_prefers_thread_over_session() {
        assert_eq!(
            resolve_scope(Some("th-1"), "s-1"),
            WorkScope::Thread("th-1".to_string())
        );
    }

    #[test]
    fn resolve_scope_falls_back_to_session_when_there_is_no_thread() {
        assert_eq!(
            resolve_scope(None, "s-1"),
            WorkScope::Session("s-1".to_string())
        );
    }

    #[test]
    fn key_prefixes_are_stable_because_they_are_the_database_key() {
        // These exact strings are the primary key the lease/queue rows are stored under — pinned
        // here so an accidental rename does not silently become a schema break.
        assert_eq!(WorkScope::Thread("th-1".to_string()).key(), "thread:th-1");
        assert_eq!(WorkScope::Session("s-1".to_string()).key(), "session:s-1");
    }

    const NOW: &str = "2026-08-02T00:00:00Z";
    const LATER: &str = "2026-08-02T01:00:00Z";
    const EXPIRES: &str = "2026-08-02T00:10:00Z";

    #[test]
    fn two_connections_race_for_different_scopes_and_exactly_one_acquires() {
        // The real race is between two PROCESSES (`nxc send` and `nxc reply` are separate
        // invocations against the same file), so the guard has to hold across connections, not just
        // within one process's in-memory state. Two stores over the same file, each with its own
        // connection, racing for DIFFERENT scopes.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("db.sqlite");
        let path = path.to_str().unwrap();
        let mut a = ChatStore::open(path, 1).unwrap();
        let mut b = ChatStore::open(path, 2).unwrap();

        let scope_a = WorkScope::Thread("th-a".to_string());
        let scope_b = WorkScope::Thread("th-b".to_string());

        let a_won = a.acquire_working_tree(&scope_a, NOW, EXPIRES).unwrap();
        let b_won = b.acquire_working_tree(&scope_b, NOW, EXPIRES).unwrap();

        assert!(a_won, "the first connection to acquire wins");
        assert!(
            !b_won,
            "the second connection is refused by the table, not by anything in-memory"
        );
        assert_eq!(
            b.working_tree_holder(NOW).unwrap(),
            Some(scope_a.key()),
            "the table, read from the OTHER connection, agrees on who holds it"
        );
    }

    #[test]
    fn the_same_scope_inherits_the_lease_without_resetting_acquired() {
        let mut store = ChatStore::open_in_memory(1);
        let scope = WorkScope::Thread("th-1".to_string());

        assert!(store.acquire_working_tree(&scope, NOW, EXPIRES).unwrap());
        assert!(
            store
                .acquire_working_tree(&scope, LATER, "2026-08-02T02:00:00Z")
                .unwrap(),
            "the same chain re-acquiring for its next step inherits, it is not refused"
        );

        let acquired: String = store
            .connection()
            .query_row(
                "SELECT acquired FROM working_tree_lease WHERE tree = 'default'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            acquired, NOW,
            "inheriting must not reset when the CHAIN started running"
        );
    }

    #[test]
    fn an_expired_holder_is_reclaimed_on_the_next_acquire_attempt_with_no_cleanup_run() {
        let mut store = ChatStore::open_in_memory(1);
        let stale = WorkScope::Thread("th-stale".to_string());
        let fresh = WorkScope::Thread("th-fresh".to_string());

        assert!(store.acquire_working_tree(&stale, NOW, EXPIRES).unwrap());

        // LATER is after `stale`'s EXPIRES, so this acquire attempt itself reclaims — nothing else
        // ever runs a sweep over the table.
        assert!(store
            .acquire_working_tree(&fresh, LATER, "2026-08-02T02:00:00Z")
            .unwrap());
        assert_eq!(store.working_tree_holder(LATER).unwrap(), Some(fresh.key()));

        // PR review finding 2: the CASE's ELSE arm — a takeover gets its OWN `acquired` timestamp,
        // not the displaced chain's — depends on `ON CONFLICT` evaluating the SET terms against the
        // PRE-UPDATE row. `working_tree_holder` alone can't distinguish that from a bug where
        // SQLite evaluated the SET terms sequentially and left `acquired` at the stale holder's
        // NOW: this assertion reads it back directly to rule that out.
        let acquired: String = store
            .connection()
            .query_row(
                "SELECT acquired FROM working_tree_lease WHERE tree = 'default'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            acquired, LATER,
            "a takeover must record its OWN acquired time, not the displaced holder's"
        );
    }

    #[test]
    fn a_live_holder_is_not_displaced_by_a_different_scope() {
        let mut store = ChatStore::open_in_memory(1);
        let holder = WorkScope::Thread("th-holder".to_string());
        let challenger = WorkScope::Thread("th-challenger".to_string());

        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        assert!(!store
            .acquire_working_tree(&challenger, NOW, "2026-08-02T00:20:00Z")
            .unwrap());
        assert_eq!(store.working_tree_holder(NOW).unwrap(), Some(holder.key()));

        // PR review finding 2: a refused acquire must leave the row COMPLETELY untouched, not just
        // `scope_key` — the challenger's own `expires` argument ("...T00:20:00Z", later than the
        // real holder's EXPIRES) is sitting right there as a value that would silently win if the
        // WHERE clause admitted the update at all.
        let expires: String = store
            .connection()
            .query_row(
                "SELECT expires FROM working_tree_lease WHERE tree = 'default'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            expires, EXPIRES,
            "a refused acquire must not touch expires either"
        );
    }

    #[test]
    fn working_tree_holder_is_none_when_nothing_was_ever_acquired() {
        let store = ChatStore::open_in_memory(1);
        assert_eq!(store.working_tree_holder(NOW).unwrap(), None);
    }

    #[test]
    fn an_offset_form_expires_does_not_mis_sort_as_already_past() {
        // PR review finding 1: `acquire_working_tree`/`working_tree_holder` compare `now`/`expires`
        // as raw SQL byte strings, so an un-normalized offset-form instant can mis-sort — a
        // negative-zone FUTURE instant sorting as PAST — and hand a still-live lease to a second
        // chain. `2026-08-02T23:59:00-05:00` denotes `2026-08-03T04:59:00Z`, i.e. more than five
        // hours AFTER `2026-08-03T00:00:00Z` — but stored raw, its `2026-08-02...` prefix would
        // byte-sort BEFORE `2026-08-03T00:00:00Z`, reading a live lease as already expired.
        let mut store = ChatStore::open_in_memory(1);
        let holder = WorkScope::Thread("th-holder".to_string());
        let challenger = WorkScope::Thread("th-challenger".to_string());

        assert!(store
            .acquire_working_tree(&holder, "2026-08-02T23:30:00Z", "2026-08-02T23:59:00-05:00")
            .unwrap());

        // Half an hour later — long before the real (UTC) expiry, but AFTER the un-normalized
        // string's byte-sort position — a challenger must still be refused.
        assert!(
            !store
                .acquire_working_tree(&challenger, "2026-08-03T00:00:00Z", "2026-08-03T01:00:00Z")
                .unwrap(),
            "an offset-form expires must not mis-sort as already past"
        );
        assert_eq!(
            store.working_tree_holder("2026-08-03T00:00:00Z").unwrap(),
            Some(holder.key())
        );

        // The same normalization applies on the READ side, and the instant here is chosen to
        // DISCRIMINATE rather than merely to be offset-form (PR review, final pass). This case
        // originally used `2026-08-02T19:00:00-05:00` — the same instant as the UTC query above,
        // but a `now` whose raw bytes still compare correctly against the stored expiry
        // (`"2026-08-03T04:59:00Z" > "2026-08-02T19:00:00-05:00"` holds byte-wise too), so deleting
        // `to_utc_rfc3339` from `working_tree_holder` broke no assertion and the case proved
        // nothing.
        //
        // `2026-08-03T09:00:00+09:00` is the SAME instant (`2026-08-03T00:00:00Z`) and does
        // discriminate: raw, its `T09` prefix byte-sorts AFTER the stored `T04:59:00Z`, so an
        // un-normalized read finds `expires > now` false and reports a live lease as FREE — the
        // read-side twin of the acquire-side mis-sort this test's first half pins, and the one that
        // would hand the working copy to a second chain.
        assert_eq!(
            store
                .working_tree_holder("2026-08-03T09:00:00+09:00")
                .unwrap(),
            Some(holder.key()),
            "an offset-form now must resolve to the same answer as its UTC equivalent"
        );
    }

    #[test]
    fn only_one_of_eight_genuinely_concurrent_racers_acquires() {
        // PR review finding 3: the two-connection test above proves the DATABASE decides, but issues
        // its acquires one after the other from a single thread — it never actually lets two writers
        // collide. This one does: eight threads, eight connections, released at once on a barrier,
        // each racing for a DIFFERENT scope against the same 'default' row. Mirrors
        // consolidation_claim.rs's `only_one_of_eight_genuinely_concurrent_racers_wins`.
        use std::sync::{Arc, Barrier};

        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("db.sqlite").to_str().unwrap().to_string();

        const RACERS: usize = 8;
        let barrier = Arc::new(Barrier::new(RACERS));
        let handles: Vec<_> = (0..RACERS)
            .map(|i| {
                let (path, barrier) = (path.clone(), Arc::clone(&barrier));
                std::thread::spawn(move || {
                    let mut store = ChatStore::open(&path, i as i64 + 1).unwrap();
                    // `ChatStore::open` delegates straight to `Substrate::open`
                    // (foundation/store.rs:85), which already sets `PRAGMA busy_timeout=5000` — and
                    // foundation's own `open_sets_a_busy_timeout_so_a_locked_db_waits_not_fails`
                    // pins that. Set it again anyway so this test does not depend on that detail
                    // holding elsewhere: a writer that meets a locked db under real contention waits
                    // rather than erroring with SQLITE_BUSY.
                    store
                        .connection()
                        .execute_batch("PRAGMA busy_timeout=5000;")
                        .unwrap();
                    let scope = WorkScope::Thread(format!("th-{i}"));
                    barrier.wait();
                    store.acquire_working_tree(&scope, NOW, EXPIRES).unwrap()
                })
            })
            .collect();
        let won = handles
            .into_iter()
            .map(|h| h.join().expect("no racer panics"))
            .filter(|w| *w)
            .count();

        assert_eq!(
            won, 1,
            "exactly one of {RACERS} concurrent racers may hold the working-tree lease"
        );
    }

    /// Open a store on `path` that waits for a busy writer instead of erroring — the same
    /// belt-and-braces `only_one_of_eight_genuinely_concurrent_racers_acquires` applies, for the
    /// same reason (`ChatStore::open` already sets it; this does not depend on that holding).
    fn racer_store(path: &str, device: i64) -> ChatStore {
        let store = ChatStore::open(path, device).unwrap();
        store
            .connection()
            .execute_batch("PRAGMA busy_timeout=5000;")
            .unwrap();
        store
    }

    #[test]
    fn a_racer_never_takes_the_working_copy_across_a_release_that_has_somebody_queued() {
        // PR #336 review, Test Quality #1, re-aimed by nxf 6j6v.yd4w. The claim it was written for
        // was "no observer may see the lease FREE while the entry it frees is still queued" — the
        // published intermediate state of a two-statement release. Since yd4w the transaction also
        // GRANTS the copy to the area it promotes, so the claim is strictly larger and simpler to
        // state: across a release with somebody queued, a rival must never end up holding the
        // working copy at all. The copy goes from the outgoing chain to the promoted area and is
        // free at no instant in between.
        //
        // The racers therefore take both halves inside their own `BEGIN IMMEDIATE` for a second
        // reason now: their evidence is not only "did I win" but "who did I lose to". A racer that
        // is refused while `th-holder` still holds has not reached the interesting moment; a racer
        // refused by the PROMOTED area has, and that is the floor which makes the zero below mean
        // something.
        //
        // **This is the BEHAVIOURAL pin and it is not the trip-wire** — 6j6v.zmb5 measured exactly
        // that and it is still true here: the releasing connection reclaims SQLite's write lock
        // fast enough that a naively split transaction is almost never observed from another
        // thread. What catches a split is
        // `the_hand_over_is_one_transaction_so_a_naive_split_shows_up_as_a_second_commit`, next
        // door, which counts commits instead of racing them.
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Barrier};

        const RACERS: usize = 3;
        const ROUNDS: usize = 60;
        /// Per racer, per round. Only exhausted when the release never lands at all.
        const ATTEMPTS: usize = 300;
        /// The key every round's queued entry is parked under — `queue_trigger`'s own scope key,
        /// and therefore who must hold the working copy once the release has run.
        const PROMOTED: &str = "thread:waiting";

        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("db.sqlite").to_str().unwrap().to_string();
        let holder = WorkScope::Thread("th-holder".to_string());

        // Every round is lined up by two barriers: `start` releases the racers and the releaser at
        // once, `finish` holds them until the main thread has rebuilt the round's state.
        let start = Arc::new(Barrier::new(RACERS + 2));
        let finish = Arc::new(Barrier::new(RACERS + 2));
        let violations = Arc::new(AtomicUsize::new(0));
        let refused_by_the_promoted_area = Arc::new(AtomicUsize::new(0));
        let promoted = Arc::new(AtomicUsize::new(0));

        let releaser = {
            let (path, start, finish, promoted) = (
                path.clone(),
                Arc::clone(&start),
                Arc::clone(&finish),
                Arc::clone(&promoted),
            );
            let holder = holder.clone();
            std::thread::spawn(move || {
                let mut store = racer_store(&path, 1);
                for _ in 0..ROUNDS {
                    start.wait();
                    let taken = store
                        .release_working_tree_and_take_next(&holder, NOW, EXPIRES)
                        .unwrap();
                    promoted.fetch_add(taken.len(), Ordering::Relaxed);
                    finish.wait();
                }
            })
        };

        let racers: Vec<_> = (0..RACERS)
            .map(|i| {
                let (path, start, finish) = (path.clone(), Arc::clone(&start), Arc::clone(&finish));
                let (violations, refused_by_the_promoted_area) = (
                    Arc::clone(&violations),
                    Arc::clone(&refused_by_the_promoted_area),
                );
                std::thread::spawn(move || {
                    let mut store = racer_store(&path, i as i64 + 2);
                    let scope = WorkScope::Thread(format!("th-racer-{i}"));
                    for _ in 0..ROUNDS {
                        start.wait();
                        for _ in 0..ATTEMPTS {
                            store
                                .connection()
                                .execute_batch("BEGIN IMMEDIATE;")
                                .unwrap();
                            let acquired =
                                store.acquire_working_tree(&scope, NOW, EXPIRES).unwrap();
                            let held = store.working_tree_holder(NOW).unwrap();
                            store.connection().execute_batch("COMMIT;").unwrap();
                            if acquired {
                                violations.fetch_add(1, Ordering::Relaxed);
                                break;
                            }
                            if held.as_deref() == Some(PROMOTED) {
                                refused_by_the_promoted_area.fetch_add(1, Ordering::Relaxed);
                                break;
                            }
                            std::thread::yield_now();
                        }
                        finish.wait();
                    }
                })
            })
            .collect();

        let mut main = racer_store(&path, 99);
        for _ in 0..ROUNDS {
            main.connection()
                .execute_batch("DELETE FROM working_tree_lease; DELETE FROM working_tree_queue;")
                .unwrap();
            assert!(main.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
            main.enqueue_working_tree(&queue_trigger("waiting"), NOW)
                .unwrap();
            start.wait();
            finish.wait();
        }
        releaser.join().expect("the releaser does not panic");
        for r in racers {
            r.join().expect("no racer panics");
        }

        assert_eq!(
            promoted.load(Ordering::Relaxed),
            ROUNDS,
            "every round's release really did hand its one queued entry over"
        );
        // The floor is what makes the zero below mean something: if no racer ever reached the state
        // AFTER the hand-off, the forbidden one was never even looked for.
        assert!(
            refused_by_the_promoted_area.load(Ordering::Relaxed) >= ROUNDS / 2,
            "a racer must actually meet the promoted area holding the copy in most rounds, or this \
             test observes nothing — met it {} times over {ROUNDS} rounds",
            refused_by_the_promoted_area.load(Ordering::Relaxed)
        );
        assert_eq!(
            violations.load(Ordering::Relaxed),
            0,
            "a newcomer took the working copy across a release that had an entry queued — the \
             hand-off published a free lease instead of granting it to the area it promoted"
        );
    }

    #[test]
    fn the_hand_over_is_one_transaction_so_a_naive_split_shows_up_as_a_second_commit() {
        // **nxf 6j6v.zmb5, and it is a MEASUREMENT rather than a race.** The behavioural racer
        // above pins the right invariant and cannot defend it: split
        // `release_working_tree_and_take_next` naively into two `BEGIN IMMEDIATE` transactions on
        // one connection — which is precisely what a future regression looks like — and it stayed
        // green over 300 rounds, five times running. The same connection reclaims SQLite's write
        // lock faster than another thread can slip in, so the window exists and is almost never
        // observed. A test that only fails when the scheduler cooperates is documentation, not a
        // trip-wire.
        //
        // So the singleness of the transaction is pinned STRUCTURALLY, from the connection itself:
        // SQLite calls `update_hook` once per row written and `commit_hook` once per COMMIT, so the
        // whole hand-off has to appear as writes-then-ONE-commit. Two transactions are two commits,
        // deterministically, on any machine, with no timing at all. This is the same kind of gate —
        // and the same dev-only `rusqlite` feature — `tests/bulk_quorum.rs` uses to count the
        // statements a read prepares.
        //
        // **Why not the other two ways out that 6j6v.zmb5 put on the table.** A real SEAM between
        // release and take would make the racer trip for real, at the price of a test hook in the
        // production path of the one function this epic's safety rests on — paying in the code
        // under test for a property the connection can be asked about directly. And marking the
        // racer as "documentation of the invariant" would have left the most likely future
        // regression with nothing watching it at all, which is the state this item was raised to
        // end.
        use std::sync::{Arc, Mutex};

        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("db.sqlite").to_str().unwrap().to_string();
        let mut store = ChatStore::open(&path, 1).unwrap();
        let holder = WorkScope::Thread("th-holder".to_string());
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        store
            .enqueue_working_tree(&queue_trigger("waiting"), NOW)
            .unwrap();

        // Installed only for the call under test, so the setup above contributes nothing.
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let writes = Arc::clone(&log);
            store.connection().update_hook(Some(
                move |_action, _db: &str, table: &str, _row_id: i64| {
                    writes.lock().unwrap().push(format!("write {table}"));
                },
            ));
            let commits = Arc::clone(&log);
            store.connection().commit_hook(Some(move || {
                commits.lock().unwrap().push("commit".to_string());
                false // do not veto the commit
            }));
        }

        let promoted = store
            .release_working_tree_and_take_next(&holder, NOW, EXPIRES)
            .unwrap();

        store
            .connection()
            .update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>);
        store.connection().commit_hook(None::<fn() -> bool>);

        assert_eq!(promoted.len(), 1, "the hand-off promoted its one entry");
        let log = log.lock().unwrap().clone();
        assert_eq!(
            log.iter().filter(|e| *e == "commit").count(),
            1,
            "release, take and grant are ONE transaction — a second commit means they were split \
             apart, and the working copy is free between them. Got: {log:?}"
        );
        assert_eq!(
            log.last().map(String::as_str),
            Some("commit"),
            "…and every write of the hand-off lands BEFORE that commit rather than after it: \
             {log:?}"
        );
        assert_eq!(
            log.iter().filter(|e| *e == "write working_tree_lease").count(),
            2,
            "the outgoing lease row is deleted and the promoted area's is written, both inside it: \
             {log:?}"
        );
        assert_eq!(
            log.iter()
                .filter(|e| *e == "write working_tree_queue")
                .count(),
            1,
            "as is the take of the entry that was waiting: {log:?}"
        );
    }

    // ---- working_tree_queue (6j6v.m3d5) ---------------------------------------------------

    use crate::model::Priority;

    fn queue_trigger(role: &str) -> QueuedTrigger {
        QueuedTrigger {
            id: 0, // ignored by enqueue_working_tree; the table assigns the real one
            scope_key: format!("thread:{role}"),
            role: role.to_string(),
            session: "s-1".to_string(),
            thread: Some("th-1".to_string()),
            message: format!("do the {role} thing"),
            model: None,
            depth: 0,
            priority: Priority::Normal,
            enqueued_at: None,
        }
    }

    /// [`queue_trigger`] at a stated urgency — the ordering tests' one varying input.
    fn queue_trigger_at(role: &str, priority: Priority) -> QueuedTrigger {
        QueuedTrigger {
            priority,
            ..queue_trigger(role)
        }
    }

    #[test]
    fn a_later_enqueued_urgent_trigger_comes_before_an_earlier_normal_one() {
        // The decision this ordering exists for (owner, 2026-08-12, ticket m3d5's own example):
        // "change the button label" must not sit forty minutes behind "build the website" just
        // because it arrived later, if its sender called it more urgent.
        let mut store = ChatStore::open_in_memory(1);

        let normal_early_id = store
            .enqueue_working_tree(&queue_trigger("build-the-website"), NOW)
            .unwrap();
        let urgent_late_id = store
            .enqueue_working_tree(
                &queue_trigger_at("change-the-button-label", Priority::Urgent),
                "2026-08-02T00:40:00Z",
            )
            .unwrap();

        let head = store.peek_working_tree_queue().unwrap().unwrap();
        assert_eq!(
            head.id, urgent_late_id,
            "the urgent trigger enqueued LATER must come out FIRST"
        );
        assert_ne!(urgent_late_id, normal_early_id);
    }

    #[test]
    fn three_mixed_entries_drain_in_priority_then_enqueued_at_then_id_order() {
        let mut store = ChatStore::open_in_memory(1);

        let a = store
            .enqueue_working_tree(&queue_trigger("a"), NOW)
            .unwrap();
        let b = store
            .enqueue_working_tree(&queue_trigger_at("b", Priority::Urgent), LATER)
            .unwrap();
        let c = store
            .enqueue_working_tree(&queue_trigger("c"), "2026-08-02T02:00:00Z")
            .unwrap();

        let drained: Vec<i64> = std::iter::from_fn(|| store.take_working_tree_queue().unwrap())
            .map(|q| q.id)
            .collect();

        assert_eq!(
            drained,
            vec![b, a, c],
            "b (urgent) first despite enqueuing after a; a before c (same priority, earlier enqueued_at)"
        );
    }

    #[test]
    fn two_entries_with_equal_priority_and_enqueued_at_drain_in_id_order() {
        // The tie-breaker: same priority, same (already-normalized) enqueued_at instant — only
        // `id` (insertion order) is left to make the order deterministic.
        let mut store = ChatStore::open_in_memory(1);
        let first = store
            .enqueue_working_tree(&queue_trigger("first"), NOW)
            .unwrap();
        let second = store
            .enqueue_working_tree(&queue_trigger("second"), NOW)
            .unwrap();

        let drained: Vec<i64> = std::iter::from_fn(|| store.take_working_tree_queue().unwrap())
            .map(|q| q.id)
            .collect();
        assert_eq!(drained, vec![first, second]);
    }

    #[test]
    fn enqueued_at_is_normalized_so_an_offset_form_instant_does_not_mis_sort() {
        // Regression for a review finding on this ticket: every other ordering test above uses
        // already-canonical UTC zero-fraction instants, which are idempotent under
        // `to_utc_rfc3339` — deleting the normalizer call in `enqueue_working_tree` would still
        // pass all of them. This one uses a genuine offset-form instant whose raw bytes sort the
        // WRONG way against the other entry's UTC form, mirroring the lease's own
        // `an_offset_form_expires_does_not_mis_sort_as_already_past`.
        //
        // `p` is enqueued FIRST (so it gets the smaller id) at `"2026-08-02T20:00:00-08:00"`,
        // which denotes `2026-08-03T04:00:00Z` (an 8-hour-behind offset: 20:00 + 8h). `q` is
        // enqueued SECOND (the larger id) at `"2026-08-02T23:30:00Z"` — already UTC, and over four
        // hours EARLIER in real time than `p` despite the later id. Byte-compared raw, however,
        // `"...T20:00:00-08:00"` sorts BEFORE `"...T23:30:00Z"` (the `'0'` in `"20"` sorts below
        // the `'3'` in `"23"`), so a store that skipped normalization would rank `p` ahead of `q` —
        // backwards. Confirmed to fail (drains `[p, q]`) with the `to_utc_rfc3339(now)?` call in
        // `enqueue_working_tree` deleted, and to pass again once restored.
        let mut store = ChatStore::open_in_memory(1);

        let p = store
            .enqueue_working_tree(&queue_trigger("p"), "2026-08-02T20:00:00-08:00")
            .unwrap();
        let q = store
            .enqueue_working_tree(&queue_trigger("q"), "2026-08-02T23:30:00Z")
            .unwrap();

        let drained: Vec<i64> = std::iter::from_fn(|| store.take_working_tree_queue().unwrap())
            .map(|entry| entry.id)
            .collect();

        assert_eq!(
            drained,
            vec![q, p],
            "q's real UTC instant is earlier despite the later id and the raw offset-form p sorting first unnormalized"
        );
    }

    #[test]
    fn list_reports_every_entry_in_take_order_without_consuming_any_of_them() {
        let mut store = ChatStore::open_in_memory(1);
        let a = store
            .enqueue_working_tree(&queue_trigger("a"), NOW)
            .unwrap();
        let b = store
            .enqueue_working_tree(&queue_trigger_at("b", Priority::Urgent), LATER)
            .unwrap();
        let c = store
            .enqueue_working_tree(&queue_trigger("c"), "2026-08-02T02:00:00Z")
            .unwrap();

        let listed: Vec<i64> = store
            .list_working_tree_queue()
            .unwrap()
            .iter()
            .map(|q| q.id)
            .collect();
        assert_eq!(
            listed,
            vec![b, a, c],
            "same order take_working_tree_queue would drain: urgent first, then enqueued_at"
        );
        assert_eq!(
            store.working_tree_queue_len().unwrap(),
            3,
            "listing must not consume anything"
        );
        assert_eq!(
            store.list_working_tree_queue().unwrap().len(),
            3,
            "a second list sees the same three entries"
        );
    }

    #[test]
    fn list_on_an_empty_queue_is_an_empty_vec_not_an_error() {
        let store = ChatStore::open_in_memory(1);
        assert_eq!(store.list_working_tree_queue().unwrap(), Vec::new());
    }

    #[test]
    fn take_reads_and_deletes_so_a_second_take_on_an_empty_queue_is_none() {
        let mut store = ChatStore::open_in_memory(1);
        let q = queue_trigger("solo");
        let id = store.enqueue_working_tree(&q, NOW).unwrap();

        let taken = store.take_working_tree_queue().unwrap().unwrap();
        assert_eq!(taken.id, id);
        assert_eq!(taken.scope_key, q.scope_key);
        assert_eq!(taken.role, q.role);
        assert_eq!(taken.session, q.session);
        assert_eq!(taken.thread, q.thread);
        assert_eq!(taken.message, q.message);
        assert_eq!(taken.model, q.model);
        assert_eq!(taken.depth, q.depth);

        assert_eq!(
            store.take_working_tree_queue().unwrap(),
            None,
            "a second take must not hand out the same entry again"
        );
        assert_eq!(store.working_tree_queue_len().unwrap(), 0);
    }

    #[test]
    fn peek_does_not_consume_the_head() {
        let mut store = ChatStore::open_in_memory(1);
        let id = store
            .enqueue_working_tree(&queue_trigger("solo"), NOW)
            .unwrap();

        assert_eq!(store.peek_working_tree_queue().unwrap().unwrap().id, id);
        assert_eq!(
            store.peek_working_tree_queue().unwrap().unwrap().id,
            id,
            "peeking twice must see the same head both times"
        );
        assert_eq!(store.working_tree_queue_len().unwrap(), 1);
    }

    #[test]
    fn working_tree_queue_position_counts_one_based_in_take_order() {
        let mut store = ChatStore::open_in_memory(1);

        let a = store
            .enqueue_working_tree(&queue_trigger("a"), NOW)
            .unwrap();
        let b = store
            .enqueue_working_tree(&queue_trigger_at("b", Priority::Urgent), LATER)
            .unwrap();
        let c = store
            .enqueue_working_tree(&queue_trigger("c"), "2026-08-02T02:00:00Z")
            .unwrap();

        assert_eq!(store.working_tree_queue_position(b).unwrap(), 1);
        assert_eq!(store.working_tree_queue_position(a).unwrap(), 2);
        assert_eq!(store.working_tree_queue_position(c).unwrap(), 3);
    }

    #[test]
    fn position_of_an_id_not_currently_queued_is_an_error() {
        let store = ChatStore::open_in_memory(1);
        assert!(store.working_tree_queue_position(999).is_err());
    }

    #[test]
    fn model_round_trips_through_its_lowercase_wire_form_or_null() {
        let mut store = ChatStore::open_in_memory(1);
        let mut with_model = queue_trigger("with-model");
        with_model.model = Some(Model::Opus);
        let without_model = queue_trigger("without-model");

        let id_with = store.enqueue_working_tree(&with_model, NOW).unwrap();
        let id_without = store.enqueue_working_tree(&without_model, NOW).unwrap();

        let taken_with = store.take_working_tree_queue().unwrap().unwrap();
        assert_eq!(taken_with.id, id_with);
        assert_eq!(taken_with.model, Some(Model::Opus));

        let taken_without = store.take_working_tree_queue().unwrap().unwrap();
        assert_eq!(taken_without.id, id_without);
        assert_eq!(taken_without.model, None);
    }

    #[test]
    fn removing_an_entry_by_id_takes_that_one_and_leaves_the_head_alone() {
        // The self-rescue primitive (nxf 6j6v.303b, PR review Finding 3): a caller undoing its OWN
        // enqueue must not be able to swallow somebody else's turn, which is exactly what
        // `take_working_tree_queue` would do — it pops whatever is most urgent. Here the caller's
        // own entry is deliberately NOT the head.
        let mut store = ChatStore::open_in_memory(1);
        let urgent = store
            .enqueue_working_tree(&queue_trigger_at("urgent", Priority::Urgent), NOW)
            .unwrap();
        let mine = store
            .enqueue_working_tree(&queue_trigger("mine"), NOW)
            .unwrap();
        assert_eq!(store.peek_working_tree_queue().unwrap().unwrap().id, urgent);

        assert!(store.remove_working_tree_queue_entry(mine).unwrap());
        assert_eq!(store.working_tree_queue_len().unwrap(), 1);
        assert_eq!(
            store.peek_working_tree_queue().unwrap().unwrap().id,
            urgent,
            "the head somebody else is waiting on must be untouched"
        );
    }

    #[test]
    fn removing_an_entry_that_is_already_gone_reports_false_rather_than_failing() {
        // An ordinary answer, not an error: a release path draining the queue (nxf 6j6v.fe0f) can
        // legitimately have taken the entry between the caller's INSERT and its undo.
        let mut store = ChatStore::open_in_memory(1);
        let id = store
            .enqueue_working_tree(&queue_trigger("solo"), NOW)
            .unwrap();
        assert!(store.take_working_tree_queue().unwrap().is_some());
        assert!(!store.remove_working_tree_queue_entry(id).unwrap());
        assert!(!store.remove_working_tree_queue_entry(9_999).unwrap());
    }

    #[test]
    fn an_empty_queue_reports_zero_length_and_no_head() {
        let mut store = ChatStore::open_in_memory(1);
        assert_eq!(store.working_tree_queue_len().unwrap(), 0);
        assert_eq!(store.peek_working_tree_queue().unwrap(), None);
        assert_eq!(store.take_working_tree_queue().unwrap(), None);
    }

    // ---- release + advance (6j6v.fe0f) -----------------------------------------------------

    #[test]
    fn every_scope_key_round_trips_through_parse_and_an_unknown_prefix_does_not() {
        // `key` and `parse` are each other's inverse and the release path depends on it: what the
        // lease table hands back is a KEY, and the question asked of it ("does this chain still owe
        // anybody a reply?") is asked of the SCOPE.
        for scope in [
            WorkScope::Thread("th-1".to_string()),
            WorkScope::Session("s-1".to_string()),
        ] {
            assert_eq!(
                WorkScope::parse(&scope.key()),
                Some(scope.clone()),
                "{scope:?}"
            );
        }
        assert_eq!(
            WorkScope::parse("wat:1"),
            None,
            "an unknown prefix must not be guessed at — the caller leaves that lease alone"
        );
        assert_eq!(
            WorkScope::parse("th-1"),
            None,
            "no prefix at all is not a scope"
        );
    }

    #[test]
    fn every_priority_round_trips_through_the_queue_row() {
        // The entry carries the trigger's raw INPUTS, and its urgency is one of them (nxf
        // 6j6v.fe0f): the release path re-fires it through `trigger_role`, which can queue it
        // again, and an entry that went in `Urgent` and came back `Normal` would silently lose the
        // one judgement the ordering exists to honour.
        let mut store = ChatStore::open_in_memory(1);
        for priority in [Priority::Urgent, Priority::Normal, Priority::Background] {
            store
                .enqueue_working_tree(&queue_trigger_at("p", priority), NOW)
                .unwrap();
            let taken = store.take_working_tree_queue().unwrap().unwrap();
            assert_eq!(taken.priority, priority);
        }
    }

    #[test]
    fn a_promotion_that_goes_back_into_the_queue_keeps_the_place_it_already_waited_for() {
        // The other half of the ordering key (PR review of nxf 6j6v.fe0f, Finding 2). A promoted
        // entry that loses the post-release race for the lease is re-queued through `trigger_role`
        // like any other trigger. It keeps its `priority` because that is a field — and it must keep
        // its `enqueued_at` for the same reason, or an entry that waited forty minutes re-enters
        // BEHIND everything that arrived while it was out.
        let mut store = ChatStore::open_in_memory(1);
        store
            .enqueue_working_tree(&queue_trigger("waited-since-midnight"), NOW)
            .unwrap();
        let promoted = store.take_working_tree_queue().unwrap().unwrap();
        assert_eq!(
            promoted.enqueued_at.as_deref(),
            Some(NOW),
            "the entry comes back out carrying when it first joined the queue"
        );

        // Somebody else joins the queue while the promotion is in flight, and then the promotion
        // loses the race and goes back in — an hour of wall clock later.
        let arrived_meanwhile = store
            .enqueue_working_tree(&queue_trigger("arrived-meanwhile"), LATER)
            .unwrap();
        let back_in = store
            .enqueue_working_tree(&promoted, "2026-08-02T05:00:00Z")
            .unwrap();

        assert!(back_in > arrived_meanwhile, "it really is the newer ROW");
        assert_eq!(
            store.peek_working_tree_queue().unwrap().unwrap().id,
            back_in,
            "and it is still the head: the wait it had already served is not thrown away"
        );
    }

    #[test]
    fn release_and_take_hands_the_lease_straight_on_to_the_head_in_one_step() {
        // The hand-off, all three halves of it (nxf 6j6v.yd4w): the outgoing lease is gone, the
        // head's area is off the queue, and the LEASE IS ALREADY THE HEAD'S. The third is what
        // this test grew: a release used to leave the copy free until the promoted trigger got as
        // far as its own acquire, and a newcomer arriving in that window took it — putting the
        // entry that had just been promoted back behind the chain it was ahead of.
        let mut store = ChatStore::open_in_memory(1);
        let holder = WorkScope::Thread("th-holder".to_string());
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        let waiting = store
            .enqueue_working_tree(&queue_trigger("waiting"), NOW)
            .unwrap();

        let next = store
            .release_working_tree_and_take_next(&holder, NOW, EXPIRES)
            .unwrap();

        assert_eq!(next.len(), 1, "one entry, one claim area: {next:?}");
        assert_eq!(next[0].id, waiting);
        assert_eq!(next[0].role, "waiting");
        assert_eq!(
            store.working_tree_holder(NOW).unwrap(),
            Some("thread:waiting".to_string()),
            "the promoted area holds it already — the working copy is never free while somebody \
             is waiting for it"
        );
        assert_eq!(store.working_tree_queue_len().unwrap(), 0);
    }

    #[test]
    fn release_and_take_with_an_empty_queue_still_frees_the_lease() {
        // The other side of the grant: with nobody waiting there is no area to hand the copy to,
        // so a release means exactly what it always did — the working copy is FREE.
        let mut store = ChatStore::open_in_memory(1);
        let holder = WorkScope::Thread("th-holder".to_string());
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());

        assert_eq!(
            store
                .release_working_tree_and_take_next(&holder, NOW, EXPIRES)
                .unwrap(),
            Vec::new()
        );
        assert_eq!(store.working_tree_holder(NOW).unwrap(), None);
        assert_eq!(
            store.working_tree_lease_row().unwrap(),
            None,
            "and the row is gone rather than granted to nobody"
        );
    }

    /// [`queue_trigger`] parked under an explicitly named claim area — the shape a fan-out leaves
    /// behind, where several entries share ONE `scope_key`.
    fn queue_trigger_under(scope_key: &str, role: &str) -> QueuedTrigger {
        QueuedTrigger {
            scope_key: scope_key.to_string(),
            ..queue_trigger(role)
        }
    }

    #[test]
    fn a_release_drains_the_whole_claim_area_at_the_head_not_just_its_first_entry() {
        // PR #336 review, Code Quality #1 (CRITICAL). A multi-member `working_tree: exclusive`
        // channel that has to wait puts ONE entry per member on the queue, all under the board's
        // single `scope_key`. Popping one per release strands the siblings: the board owes THEIR
        // replies, so the release that would free them can never happen. Draining the head's whole
        // claim area is not new behaviour — it is the behaviour the uncontended path already has,
        // where the first member acquires and the rest inherit.
        let mut store = ChatStore::open_in_memory(1);
        let holder = WorkScope::Thread("th-holder".to_string());
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        for member in ["general", "code-quality", "test-quality", "integrity"] {
            store
                .enqueue_working_tree(&queue_trigger_under("thread:th-board", member), NOW)
                .unwrap();
        }
        // A chain that is NOT part of that board, waiting behind it.
        store
            .enqueue_working_tree(&queue_trigger_under("thread:th-other", "stranger"), LATER)
            .unwrap();

        let promoted = store
            .release_working_tree_and_take_next(&holder, NOW, EXPIRES)
            .unwrap();

        assert_eq!(
            promoted.iter().map(|q| q.role.as_str()).collect::<Vec<_>>(),
            vec!["general", "code-quality", "test-quality", "integrity"],
            "every member of the head's claim area comes back, in the queue's own order"
        );
        assert_eq!(
            store.working_tree_queue_len().unwrap(),
            1,
            "and nobody else's turn was swallowed with them"
        );
        assert_eq!(
            store.peek_working_tree_queue().unwrap().unwrap().role,
            "stranger"
        );
        assert_eq!(
            store.working_tree_holder(NOW).unwrap(),
            Some("thread:th-board".to_string()),
            "and the board — the whole area, not one member of it — holds the working copy now"
        );
    }

    #[test]
    fn the_queue_order_still_decides_which_claim_area_goes_next() {
        // Scope-awareness applies AFTER the head is chosen, never before it: the `(priority,
        // enqueued_at, id)` order picks the claim area, and only then does the whole of THAT area
        // come out. A group of two that arrived first must not outrank a single urgent entry.
        let mut store = ChatStore::open_in_memory(1);
        let holder = WorkScope::Thread("th-holder".to_string());
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        for member in ["early-a", "early-b"] {
            store
                .enqueue_working_tree(&queue_trigger_under("thread:th-early", member), NOW)
                .unwrap();
        }
        store
            .enqueue_working_tree(
                &QueuedTrigger {
                    priority: Priority::Urgent,
                    ..queue_trigger_under("thread:th-urgent", "late-but-urgent")
                },
                LATER,
            )
            .unwrap();

        let promoted = store
            .release_working_tree_and_take_next(&holder, NOW, EXPIRES)
            .unwrap();

        assert_eq!(
            promoted.iter().map(|q| q.role.as_str()).collect::<Vec<_>>(),
            vec!["late-but-urgent"],
            "the urgent claim area goes first, and it takes only its OWN entries"
        );
        assert_eq!(store.working_tree_queue_len().unwrap(), 2);
    }

    #[test]
    fn a_second_release_of_the_same_scope_takes_nothing() {
        // **nxf 6j6v.jzaj, the mechanism its probe came back with.** Two `nxc reply` processes can
        // both decide that the same claim area is finished — each persists its own message, and if
        // neither check runs before the other's write lands, BOTH see "nothing outstanding" and both
        // call the release. That is the interleaving the finding is about, and this is why it is
        // harmless: the release is a DELETE guarded on the holder's own `scope_key`, in the same
        // transaction that takes the queue. The first hands the working copy on; for the second the
        // row under that key is gone, the delete matches nothing, and the take is rolled back with
        // it. The queued area cannot be promoted twice, and cannot be dropped between the two.
        //
        // `two_concurrent_replies_completing_one_claim_area_promote_the_queue_exactly_once`
        // (tests/working_tree_two_process_e2e.rs) drives the same claim through two real processes;
        // this pins the one statement that decides it, without a scheduler in the way.
        let mut store = ChatStore::open_in_memory(1);
        let holder = WorkScope::Thread("th-holder".to_string());
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        store
            .enqueue_working_tree(&queue_trigger("waiting"), NOW)
            .unwrap();
        store
            .enqueue_working_tree(&queue_trigger_under("thread:th-third", "third"), LATER)
            .unwrap();

        let first = store
            .release_working_tree_and_take_next(&holder, NOW, EXPIRES)
            .unwrap();
        let second = store
            .release_working_tree_and_take_next(&holder, NOW, EXPIRES)
            .unwrap();

        assert_eq!(
            first.iter().map(|q| q.role.as_str()).collect::<Vec<_>>(),
            vec!["waiting"],
            "the first release hands the head's area over"
        );
        assert_eq!(
            second,
            Vec::new(),
            "and the second takes NOTHING — not the area the first promoted, and not the next one \
             in line either"
        );
        assert_eq!(
            store.working_tree_holder(NOW).unwrap(),
            Some("thread:waiting".to_string()),
            "the promoted area still holds the copy the first release handed it"
        );
        assert_eq!(
            store.peek_working_tree_queue().unwrap().unwrap().role,
            "third",
            "and the chain behind it kept its turn rather than being drained by a release that \
             released nothing"
        );
    }

    /// **A refused park goes with the claim it is about — by EVERY movement of the lease** (nxf
    /// 6j6v.8bv9). The release and the expired reclaim forget the outgoing holder's refusal inside
    /// their own transaction; a different scope's acquire, which can take an expired lease from a
    /// holder it never names, forgets every refusal but its own. An inherit, a refused acquire and a
    /// release of somebody else's scope move nothing, and forget nothing.
    #[test]
    fn every_lease_movement_forgets_the_refused_park_of_the_holder_it_moves_away_from() {
        let refusal = crate::park::ParkRefusal::MidSequence("a merge is in progress".into());
        let holder = WorkScope::Thread("th-holder".to_string());
        let fresh = WorkScope::Thread("th-fresh".to_string());
        let refused = |store: &ChatStore| store.park_refusal(&holder.key()).unwrap().is_some();

        // A release — after three things that are NOT one.
        let mut store = ChatStore::open_in_memory(1);
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        store
            .note_park_refusal(
                &holder.key(),
                &refusal,
                crate::orchestration::ParkOccasion::StrandedEscalation,
                NOW,
            )
            .unwrap();
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        assert!(refused(&store), "an inherit keeps the holder's refusal");
        assert!(!store.acquire_working_tree(&fresh, NOW, EXPIRES).unwrap());
        assert!(refused(&store), "a refused acquire moves nothing");
        store
            .release_working_tree_and_take_next(&fresh, NOW, EXPIRES)
            .unwrap();
        assert!(
            refused(&store),
            "a release of somebody else's scope moves nothing"
        );
        store
            .release_working_tree_and_take_next(&holder, NOW, EXPIRES)
            .unwrap();
        assert!(!refused(&store), "a release forgets it");

        // An expired reclaim that hands the queue its turn.
        let mut store = ChatStore::open_in_memory(1);
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        store
            .note_park_refusal(
                &holder.key(),
                &refusal,
                crate::orchestration::ParkOccasion::StrandedEscalation,
                NOW,
            )
            .unwrap();
        store
            .enqueue_working_tree(&queue_trigger("waiting"), NOW)
            .unwrap();
        let (_, expires) = store.working_tree_lease_row().unwrap().unwrap();
        let taken = store
            .reclaim_expired_working_tree_and_take_next(
                &holder.key(),
                &expires,
                LATER,
                "2026-08-02T02:00:00Z",
            )
            .unwrap();
        assert_eq!(taken.len(), 1, "the reclaim happened");
        assert!(!refused(&store), "an expired reclaim forgets it");

        // A different scope taking the expired lease in the acquire's own compare-and-swap.
        let mut store = ChatStore::open_in_memory(1);
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        store
            .note_park_refusal(
                &holder.key(),
                &refusal,
                crate::orchestration::ParkOccasion::StrandedEscalation,
                NOW,
            )
            .unwrap();
        assert!(store
            .acquire_working_tree(&fresh, LATER, "2026-08-02T02:00:00Z")
            .unwrap());
        assert!(
            !refused(&store),
            "a takeover forgets the displaced holder's refusal"
        );
    }

    #[test]
    fn the_sessions_a_withdrawal_stopped_are_recorded_only_under_a_marker_and_leave_when_resumed() {
        // nxf 6j6v.27b9: the rows the tick's one further SIGTERM is decided on.
        let holder = WorkScope::Thread("th-holder".to_string());
        let mut store = ChatStore::open_in_memory(1);
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        store
            .note_withdrawn_sessions(&holder.key(), &["s-1".to_string()], NOW)
            .unwrap();
        assert_eq!(
            store.withdrawn_session(&holder.key(), "s-1").unwrap(),
            None,
            "no marker, no row: a row about a claim nobody withdrew would be about nothing"
        );

        store
            .note_withdrawn_holder(&holder.key(), "carsten", NOW)
            .unwrap();
        store
            .note_withdrawn_sessions(&holder.key(), &["s-1".to_string(), "s-2".to_string()], NOW)
            .unwrap();
        let fresh = WithdrawnSession {
            stopped_at: NOW.to_string(),
            resignalled_at: None,
            resignal_refused: None,
        };
        assert_eq!(
            store.withdrawn_session(&holder.key(), "s-1").unwrap(),
            Some(fresh.clone())
        );

        // The claim is a compare-and-swap: exactly one caller wins it, and the instant is the
        // winner's.
        assert!(store
            .claim_withdrawn_session_resignal(&holder.key(), "s-1", NOW)
            .unwrap());
        assert!(
            !store
                .claim_withdrawn_session_resignal(&holder.key(), "s-1", LATER)
                .unwrap(),
            "a second claim loses: at most one further SIGTERM"
        );
        store
            .note_withdrawn_session_resignal_refused(&holder.key(), "s-1", "no such process")
            .unwrap();
        assert_eq!(
            store.withdrawn_session(&holder.key(), "s-1").unwrap(),
            Some(WithdrawnSession {
                stopped_at: NOW.to_string(),
                resignalled_at: Some(NOW.to_string()),
                resignal_refused: Some("no such process".to_string()),
            }),
            "the claim's instant and the refusal both stand"
        );
        store
            .note_withdrawn_sessions(&holder.key(), &["s-1".to_string()], LATER)
            .unwrap();
        assert_eq!(
            store
                .withdrawn_session(&holder.key(), "s-1")
                .unwrap()
                .map(|r| (r.stopped_at, r.resignalled_at)),
            Some((NOW.to_string(), Some(NOW.to_string()))),
            "a second withdrawal of the same session neither restarts its clock nor re-opens a \
             signal already claimed"
        );

        store.forget_withdrawn_session("s-2").unwrap();
        assert_eq!(
            store.withdrawn_session(&holder.key(), "s-2").unwrap(),
            None,
            "a session put back in motion is not what the withdrawal stopped any more"
        );
        assert!(
            store.withdrawn_holder(&holder.key()).unwrap().is_some(),
            "and the withdrawal itself stands"
        );
    }

    /// The twin of `every_lease_movement_forgets_the_refused_park_of_the_holder_it_moves_away_from`
    /// above, and it is needed for a sharper reason than symmetry. `note_withdrawn_holder`'s own doc
    /// enumerates four places the row goes, and only one of them had a test — the tick-park, through
    /// the release. Deleting the clear inside the acquire's compare-and-swap, or inside the shared
    /// reclaim body, turned nothing red; and this module says at the release site itself what a
    /// marker left standing costs: it "would park this key's NEXT claim the moment it took the
    /// copy" — a `git checkout` under work nobody withdrew.
    ///
    /// The negatives carry the other half of the rule and are the reason this is a table rather than
    /// four asserts. An inherit is the ADVERTISED way back for a withdrawn round: the same claim key
    /// taking the copy again must keep its marker, or the follow-up that resumes a stopped session
    /// would end the withdrawal and let the holder's ordinary end hand the copy on with the work
    /// still in the tree — which is exactly the rule fix round 1 of this item's review reverted to.
    #[test]
    fn every_lease_movement_forgets_the_withdrawn_marker_of_the_holder_it_moves_away_from() {
        let holder = WorkScope::Thread("th-holder".to_string());
        let fresh = WorkScope::Thread("th-fresh".to_string());
        // The sessions a withdrawal stopped live and die with its marker (nxf 6j6v.27b9), so every
        // read below asks both and insists they agree.
        let withdrawn = |store: &ChatStore| {
            let marker = store.withdrawn_holder(&holder.key()).unwrap().is_some();
            let stopped = store
                .withdrawn_session(&holder.key(), "s-stopped")
                .unwrap()
                .is_some();
            assert_eq!(
                marker, stopped,
                "the stopped sessions go with the marker, never apart"
            );
            marker
        };
        let marked = |store: &mut ChatStore| {
            store
                .note_withdrawn_holder(&holder.key(), "carsten", NOW)
                .unwrap();
            store
                .note_withdrawn_sessions(&holder.key(), &["s-stopped".to_string()], NOW)
                .unwrap();
        };

        // A release — after three things that are NOT one.
        let mut store = ChatStore::open_in_memory(1);
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        marked(&mut store);
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        assert!(
            withdrawn(&store),
            "an INHERIT keeps it: the same claim taking the copy again is the way back, not the \
             end of the withdrawal"
        );
        assert!(!store.acquire_working_tree(&fresh, NOW, EXPIRES).unwrap());
        assert!(withdrawn(&store), "a refused acquire moves nothing");
        store
            .release_working_tree_and_take_next(&fresh, NOW, EXPIRES)
            .unwrap();
        assert!(
            withdrawn(&store),
            "a release of somebody else's scope moves nothing"
        );
        store
            .release_working_tree_and_take_next(&holder, NOW, EXPIRES)
            .unwrap();
        assert!(!withdrawn(&store), "a release forgets it");

        // An expired reclaim that hands the queue its turn.
        let mut store = ChatStore::open_in_memory(1);
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        marked(&mut store);
        store
            .enqueue_working_tree(&queue_trigger("waiting"), NOW)
            .unwrap();
        let (_, expires) = store.working_tree_lease_row().unwrap().unwrap();
        let taken = store
            .reclaim_expired_working_tree_and_take_next(
                &holder.key(),
                &expires,
                LATER,
                "2026-08-02T02:00:00Z",
            )
            .unwrap();
        assert_eq!(taken.len(), 1, "the reclaim happened");
        assert!(!withdrawn(&store), "an expired reclaim forgets it");

        // The DEAD-HOLDER reclaim, the other entrance into the same body — reachable for a
        // withdrawn holder whose stopped process never announced anything.
        let mut store = ChatStore::open_in_memory(1);
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        marked(&mut store);
        store
            .enqueue_working_tree(&queue_trigger("waiting"), NOW)
            .unwrap();
        let (_, expires) = store.working_tree_lease_row().unwrap().unwrap();
        let taken = store
            .reclaim_a_dead_holders_working_tree_and_take_next(
                &holder.key(),
                &expires,
                NOW,
                EXPIRES,
            )
            .unwrap();
        assert_eq!(taken.len(), 1, "the reclaim happened inside the bound");
        assert!(!withdrawn(&store), "a dead-holder reclaim forgets it too");

        // A different scope taking the expired lease in the acquire's own compare-and-swap.
        let mut store = ChatStore::open_in_memory(1);
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        marked(&mut store);
        assert!(store
            .acquire_working_tree(&fresh, LATER, "2026-08-02T02:00:00Z")
            .unwrap());
        assert!(
            !withdrawn(&store),
            "a takeover forgets the displaced holder's marker"
        );
    }

    /// **The marker's insert is CONDITIONAL on the lease it is about** (fix round 3 of this item's
    /// review, Code Quality #5). `withdraw` reads the lease row and only afterwards calls
    /// `note_withdrawn_holder` — real work runs in between, discharging the queued half of the
    /// round — so a round whose own final reply releases the lease in that window must not leave a
    /// marker standing for a key that, by the time the write lands, holds nothing. An unconditional
    /// upsert would write one anyway, and a LATER acquire by the very same claim key — the
    /// advertised way back for a withdrawn round — keeps a marker it finds standing (this file's own
    /// `every_lease_movement_forgets_the_withdrawn_marker_of_the_holder_it_moves_away_from`, "an
    /// INHERIT keeps it"), so that claim's own ordinary end would decline and park a round that
    /// never needed one.
    #[test]
    fn a_marker_is_not_written_for_a_key_that_holds_no_lease() {
        let holder = WorkScope::Thread("th-holder".to_string());

        // The plain case: this key never held the lease at all.
        let mut store = ChatStore::open_in_memory(1);
        store
            .note_withdrawn_holder(&holder.key(), "carsten", NOW)
            .unwrap();
        assert!(
            store.withdrawn_holder(&holder.key()).unwrap().is_none(),
            "a marker for a key that never held the lease would park the NEXT claim of that key \
             the moment it took the copy, for a round that was never actually holding anything"
        );

        // The race this guards: the lease existed when `withdraw` read it and is gone by the time
        // it writes the marker — the round's own final reply landing in between.
        let mut store = ChatStore::open_in_memory(1);
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        store
            .release_working_tree_and_take_next(&holder, NOW, EXPIRES)
            .unwrap();
        store
            .note_withdrawn_holder(&holder.key(), "carsten", NOW)
            .unwrap();
        assert!(
            store.withdrawn_holder(&holder.key()).unwrap().is_none(),
            "the lease left before the marker was written, so there is nothing left to park"
        );
    }

    #[test]
    fn only_one_of_four_genuinely_concurrent_releases_of_one_scope_hands_the_queue_over() {
        // The same claim as `a_second_release_of_the_same_scope_takes_nothing`, with a real barrier
        // and four real connections instead of two calls in a row — because that is the shape nxf
        // 6j6v.jzaj's finding is about: not "a release runs twice" but "two independent PROCESSES
        // are inside the release for one area at the same time". `only_one_of_eight_genuinely_
        // concurrent_racers_acquires` is this file's established form for the acquire side; this is
        // the release side's.
        //
        // **Why this and not the two-process test.** Two real `nxc reply` processes held on the
        // workspace write lock and let go at once do NOT reach this state — measured, 12 runs out of
        // 12, with the guard below deliberately removed: SQLite's busy handler has the second writer
        // deep in its back-off ramp by the time the barrier lifts, so the first process finishes its
        // whole release before the second's message is even persisted. The process-level test
        // (`two_concurrent_replies_completing_one_claim_area_promote_the_queue_exactly_once`)
        // therefore pins the serialised order end to end, and this pins the overlapped one where
        // the decision actually gets made.
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Barrier};

        const RELEASERS: usize = 4;
        const ROUNDS: usize = 40;

        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("db.sqlite").to_str().unwrap().to_string();
        let holder = WorkScope::Thread("th-holder".to_string());

        let start = Arc::new(Barrier::new(RELEASERS + 1));
        let finish = Arc::new(Barrier::new(RELEASERS + 1));
        let handed_over = Arc::new(AtomicUsize::new(0));
        let entries_taken = Arc::new(AtomicUsize::new(0));
        // A releaser's error is CARRIED to the main thread rather than unwrapped where it happens:
        // a panic between the two barriers leaves everybody else waiting on `finish` forever, and a
        // regression that hangs CI for its whole timeout says far less than one that fails.
        let errors: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));

        let releasers: Vec<_> = (0..RELEASERS)
            .map(|i| {
                let (path, start, finish) = (path.clone(), Arc::clone(&start), Arc::clone(&finish));
                let (handed_over, entries_taken, errors) = (
                    Arc::clone(&handed_over),
                    Arc::clone(&entries_taken),
                    Arc::clone(&errors),
                );
                let holder = holder.clone();
                std::thread::spawn(move || {
                    let mut store = racer_store(&path, i as i64 + 1);
                    for _ in 0..ROUNDS {
                        start.wait();
                        match store.release_working_tree_and_take_next(&holder, NOW, EXPIRES) {
                            Ok(taken) if !taken.is_empty() => {
                                handed_over.fetch_add(1, Ordering::Relaxed);
                                entries_taken.fetch_add(taken.len(), Ordering::Relaxed);
                            }
                            Ok(_) => {}
                            Err(e) => errors.lock().unwrap().push(e.to_string()),
                        }
                        finish.wait();
                    }
                })
            })
            .collect();

        let mut main = racer_store(&path, 99);
        for _ in 0..ROUNDS {
            main.connection()
                .execute_batch("DELETE FROM working_tree_lease; DELETE FROM working_tree_queue;")
                .unwrap();
            assert!(main.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
            main.enqueue_working_tree(&queue_trigger("waiting"), NOW)
                .unwrap();
            // The area behind it: what a release that released NOTHING would wrongly help itself to.
            main.enqueue_working_tree(&queue_trigger_under("thread:th-third", "third"), LATER)
                .unwrap();
            start.wait();
            finish.wait();
            assert!(
                errors.lock().unwrap().is_empty(),
                "a concurrent release failed rather than being a no-op: {:?}",
                errors.lock().unwrap()
            );
            assert_eq!(
                main.working_tree_holder(NOW).unwrap(),
                Some("thread:waiting".to_string()),
                "the head's area holds the working copy after the round, whoever got there first"
            );
            assert_eq!(
                main.peek_working_tree_queue().unwrap().unwrap().role,
                "third",
                "and the area behind it still has its turn"
            );
        }
        for r in releasers {
            r.join().expect("no releaser panics");
        }

        assert_eq!(
            handed_over.load(Ordering::Relaxed),
            ROUNDS,
            "exactly one release per round hands the queue over — never none"
        );
        assert_eq!(
            entries_taken.load(Ordering::Relaxed),
            ROUNDS,
            "and it takes exactly the head's one entry — never the area behind it as well"
        );
    }

    #[test]
    fn a_reclaim_whose_holder_renewed_between_the_read_and_the_call_takes_nothing() {
        // **The staleness guard, driven directly** (independent review of PR #380, Test Quality #1
        // / Code Quality #1 / Integrity #2 — three reviewers, one gap). Every caller of the reclaim
        // reads the lease row first, because whose lease it is and whether that chain still has a
        // live process are questions no SQL statement can answer. Between that read and this call
        // the holder can RENEW: every trigger a chain makes rides its bound forward. The delete
        // therefore names the exact `expires` that was read, and this is the case that says so —
        // the release side got a double-call test and a barrier test out of 6j6v.jzaj, and this
        // path had neither.
        let mut store = ChatStore::open_in_memory(1);
        let holder = WorkScope::Thread("th-holder".to_string());
        // A lease that has already run out as of `LATER`, which is the `now` every call below uses.
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        store
            .enqueue_working_tree(&queue_trigger("waiting"), NOW)
            .unwrap();
        let (key, stale_expires) = store
            .working_tree_lease_row()
            .unwrap()
            .expect("a lease row");

        // The holder rides its bound forward — an ordinary inherit on its next trigger.
        const RENEWED: &str = "2026-08-02T02:00:00Z";
        assert!(store.acquire_working_tree(&holder, LATER, RENEWED).unwrap());

        assert_eq!(
            store
                .reclaim_expired_working_tree_and_take_next(&key, &stale_expires, LATER, RENEWED)
                .unwrap(),
            Vec::new(),
            "the row this caller decided about is gone, so it takes nothing"
        );
        assert_eq!(
            store.working_tree_holder(LATER).unwrap(),
            Some(holder.key()),
            "the chain that proved it is alive keeps the working copy"
        );
        assert_eq!(
            store.working_tree_queue_len().unwrap(),
            1,
            "and the entry waiting behind it was not swallowed by a reclaim that reclaimed nothing"
        );

        // Named with the row as it NOW stands, the same call reclaims — so the `Vec::new()` above
        // is the guard doing its work, not the reclaim being broken.
        let (key, expires) = store
            .working_tree_lease_row()
            .unwrap()
            .expect("a lease row");
        const PAST_RENEWAL: &str = "2026-08-02T03:00:00Z";
        assert_eq!(
            store
                .reclaim_expired_working_tree_and_take_next(
                    &key,
                    &expires,
                    PAST_RENEWAL,
                    PAST_RENEWAL
                )
                .unwrap()
                .len(),
            1,
        );
    }

    #[test]
    fn a_reclaim_of_a_lease_that_has_not_run_out_takes_nothing_even_when_it_is_named_exactly() {
        // The other half of the same guard, and the one that would be a disaster to get wrong: a
        // caller that names the row correctly but is simply WRONG about the clock must not take the
        // working copy from a chain that is still inside its bound. `expires <= ?3` is what refuses
        // it, and it is a separate condition from the row-identity one above.
        let mut store = ChatStore::open_in_memory(1);
        let holder = WorkScope::Thread("th-holder".to_string());
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        store
            .enqueue_working_tree(&queue_trigger("waiting"), NOW)
            .unwrap();
        let (key, expires) = store
            .working_tree_lease_row()
            .unwrap()
            .expect("a lease row");

        assert_eq!(
            store
                .reclaim_expired_working_tree_and_take_next(&key, &expires, NOW, EXPIRES)
                .unwrap(),
            Vec::new(),
            "the lease still stands at `now`, so there is nothing to reclaim"
        );
        assert_eq!(
            store.working_tree_holder(NOW).unwrap(),
            Some(holder.key()),
            "the live holder keeps it"
        );
        assert_eq!(store.working_tree_queue_len().unwrap(), 1);
    }

    /// **The one thing the dead-holder reclaim does that its twin refuses to do** (independent
    /// review of PR #478, Test Quality #2).
    ///
    /// [`ChatStore::reclaim_a_dead_holders_working_tree_and_take_next`] exists for exactly one
    /// reason: it drops the `expires <= now` clause, so a chain that died hard INSIDE its two-hour
    /// bound does not hold the queue for whatever is left of it. Asserting "it reclaims" on its own
    /// would pass just as well with the clause still there, so the difference is driven here — the
    /// same lease row, the same instant, both doors — and only one of them opens.
    ///
    /// WHETHER a caller is entitled to open it is `orchestration::holder_is_provably_dead`'s
    /// five-condition gate and not this module's; what is pinned here is only that the ability
    /// exists and is not secretly still the sibling.
    #[test]
    fn a_dead_holders_reclaim_takes_a_lease_that_the_expiry_reclaim_will_not() {
        let mut store = ChatStore::open_in_memory(1);
        let holder = WorkScope::Thread("th-holder".to_string());
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        store
            .enqueue_working_tree(&queue_trigger("waiting"), NOW)
            .unwrap();
        let (key, expires) = store
            .working_tree_lease_row()
            .unwrap()
            .expect("a lease row");
        assert_eq!(expires, EXPIRES, "the bound is still ahead of `NOW`");

        // The clock says the holder still has its bound, so the door the clock guards stays shut.
        assert_eq!(
            store
                .reclaim_expired_working_tree_and_take_next(&key, &expires, NOW, EXPIRES)
                .unwrap(),
            Vec::new(),
            "`expires <= now` refuses this, which is what makes the call below worth having"
        );
        assert_eq!(store.working_tree_holder(NOW).unwrap(), Some(holder.key()));
        assert_eq!(store.working_tree_queue_len().unwrap(), 1);

        // …and the other door, on the same row at the same instant, hands the copy on.
        let fire = store
            .reclaim_a_dead_holders_working_tree_and_take_next(&key, &expires, NOW, EXPIRES)
            .unwrap();
        assert_eq!(
            fire.len(),
            1,
            "the entry that was waiting is the caller's to fire now"
        );
        assert_eq!(fire[0].scope_key, "thread:waiting");
        assert_eq!(
            store.working_tree_holder(NOW).unwrap(),
            Some("thread:waiting".to_string()),
            "and the copy is granted to the area that was promoted, not left free"
        );
        assert_eq!(
            store.working_tree_queue_len().unwrap(),
            0,
            "the promoted entry came off the queue in the same transaction"
        );
    }

    /// **The staleness guard is the SAME guard, and it is the only one left** — the dead-holder
    /// reclaim's half of `a_reclaim_whose_holder_renewed_between_the_read_and_the_call_takes_nothing`
    /// (independent review of PR #478, Test Quality #2).
    ///
    /// Dropping `expires <= now` leaves the exact-`expires` compare-and-swap carrying the whole
    /// weight: the caller read the lease row a moment ago to ask whose it is and whether anything
    /// behind it is alive, and a RENEWAL in that window both changes the row and disproves the
    /// evidence — a chain that triggers is a chain with a process. So a reclaim naming the expiry
    /// it read must match nothing, and the chain that just proved it is alive keeps the copy.
    #[test]
    fn a_dead_holders_reclaim_whose_holder_renewed_between_the_read_and_the_call_takes_nothing() {
        let mut store = ChatStore::open_in_memory(1);
        let holder = WorkScope::Thread("th-holder".to_string());
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        store
            .enqueue_working_tree(&queue_trigger("waiting"), NOW)
            .unwrap();
        let (key, stale_expires) = store
            .working_tree_lease_row()
            .unwrap()
            .expect("a lease row");

        // The holder rides its bound forward — an ordinary inherit on its next trigger, and the one
        // event that says the evidence this caller acted on is out of date.
        const RENEWED: &str = "2026-08-02T02:00:00Z";
        assert!(store.acquire_working_tree(&holder, NOW, RENEWED).unwrap());

        assert_eq!(
            store
                .reclaim_a_dead_holders_working_tree_and_take_next(
                    &key,
                    &stale_expires,
                    NOW,
                    RENEWED
                )
                .unwrap(),
            Vec::new(),
            "the row this caller decided about is gone, so it takes nothing"
        );
        assert_eq!(
            store.working_tree_holder(NOW).unwrap(),
            Some(holder.key()),
            "the chain that proved it is alive keeps the working copy"
        );
        assert_eq!(
            store.working_tree_queue_len().unwrap(),
            1,
            "and the entry waiting behind it was not swallowed by a reclaim that reclaimed nothing"
        );

        // Named with the row as it NOW stands, the same call reclaims — so the `Vec::new()` above is
        // the guard doing its work and not the reclaim being broken.
        let (key, expires) = store
            .working_tree_lease_row()
            .unwrap()
            .expect("a lease row");
        assert_eq!(
            store
                .reclaim_a_dead_holders_working_tree_and_take_next(&key, &expires, NOW, RENEWED)
                .unwrap()
                .len(),
            1,
        );
    }
    #[test]
    fn only_one_of_four_genuinely_concurrent_reclaims_of_one_expired_lease_hands_the_queue_over() {
        // The reclaim side's barrier test — the analogue of
        // `only_one_of_four_genuinely_concurrent_releases_of_one_scope_hands_the_queue_over`, and
        // the gap all three reviewers of PR #380 landed on independently. The release side got this
        // shape out of 6j6v.jzaj; the reclaim side is the one an EXPIRED lease goes through, and it
        // is reached by every acquire that meets one — so more than one `nxc` process can be inside
        // it for the same row at the same time, which is exactly what a fairness gate on the
        // trigger path makes ordinary rather than exotic.
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Barrier};

        const RECLAIMERS: usize = 4;
        const ROUNDS: usize = 40;
        /// Past `EXPIRES`, so the round's lease has run out for every reclaimer at once.
        const PAST: &str = LATER;
        /// The backstop the hand-off writes for the area it grants to. Ahead of [`PAST`], or the
        /// granted lease would read as expired the instant it was written and the assertion below
        /// would be about nothing.
        const GRANTED_UNTIL: &str = "2026-08-02T03:00:00Z";

        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("db.sqlite").to_str().unwrap().to_string();
        let holder = WorkScope::Thread("th-holder".to_string());

        let start = Arc::new(Barrier::new(RECLAIMERS + 1));
        let finish = Arc::new(Barrier::new(RECLAIMERS + 1));
        let handed_over = Arc::new(AtomicUsize::new(0));
        let entries_taken = Arc::new(AtomicUsize::new(0));
        // Carried rather than unwrapped in the thread — see the release-side test for why a panic
        // between two barriers is a hang instead of a failure.
        let errors: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));

        let reclaimers: Vec<_> = (0..RECLAIMERS)
            .map(|i| {
                let (path, start, finish) = (path.clone(), Arc::clone(&start), Arc::clone(&finish));
                let (handed_over, entries_taken, errors) = (
                    Arc::clone(&handed_over),
                    Arc::clone(&entries_taken),
                    Arc::clone(&errors),
                );
                std::thread::spawn(move || {
                    let mut store = racer_store(&path, i as i64 + 1);
                    for _ in 0..ROUNDS {
                        start.wait();
                        // Each reclaimer reads the row for itself, exactly as its production caller
                        // does, and then acts on what it read.
                        let read = store.working_tree_lease_row();
                        match read {
                            Ok(Some((key, expires))) => match store
                                .reclaim_expired_working_tree_and_take_next(
                                    &key,
                                    &expires,
                                    PAST,
                                    GRANTED_UNTIL,
                                ) {
                                Ok(taken) if !taken.is_empty() => {
                                    handed_over.fetch_add(1, Ordering::Relaxed);
                                    entries_taken.fetch_add(taken.len(), Ordering::Relaxed);
                                }
                                Ok(_) => {}
                                Err(e) => errors.lock().unwrap().push(e.to_string()),
                            },
                            // Somebody else's reclaim already committed — the honest answer, and
                            // not a failure.
                            Ok(None) => {}
                            Err(e) => errors.lock().unwrap().push(e.to_string()),
                        }
                        finish.wait();
                    }
                })
            })
            .collect();

        let mut main = racer_store(&path, 99);
        for _ in 0..ROUNDS {
            main.connection()
                .execute_batch("DELETE FROM working_tree_lease; DELETE FROM working_tree_queue;")
                .unwrap();
            assert!(main.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
            main.enqueue_working_tree(&queue_trigger("waiting"), NOW)
                .unwrap();
            // The area behind it: what a reclaim that reclaimed NOTHING would wrongly help itself
            // to, and the reason one queued entry would not be enough to see the difference.
            main.enqueue_working_tree(&queue_trigger_under("thread:th-third", "third"), LATER)
                .unwrap();
            start.wait();
            finish.wait();
            assert!(
                errors.lock().unwrap().is_empty(),
                "a concurrent reclaim failed rather than being a no-op: {:?}",
                errors.lock().unwrap()
            );
            assert_eq!(
                main.working_tree_holder(PAST).unwrap(),
                Some("thread:waiting".to_string()),
                "the head's area holds the working copy after the round, whoever got there first"
            );
            assert_eq!(
                main.peek_working_tree_queue().unwrap().unwrap().role,
                "third",
                "and the area behind it still has its turn"
            );
        }
        for r in reclaimers {
            r.join().expect("no reclaimer panics");
        }

        assert_eq!(
            handed_over.load(Ordering::Relaxed),
            ROUNDS,
            "exactly one reclaim per round hands the queue over — never none, never two"
        );
        assert_eq!(
            entries_taken.load(Ordering::Relaxed),
            ROUNDS,
            "and it takes exactly the head's one entry — never the area behind it as well"
        );
    }

    #[test]
    fn release_and_take_by_a_scope_that_is_not_the_holder_changes_nothing_at_all() {
        // The dangerous half: taking the head on the strength of a release that did not happen
        // would hand the queue's turn out while somebody else still has the working copy. Both
        // statements are in one transaction, so the refused release takes nothing with it.
        let mut store = ChatStore::open_in_memory(1);
        let holder = WorkScope::Thread("th-holder".to_string());
        let stranger = WorkScope::Thread("th-stranger".to_string());
        assert!(store.acquire_working_tree(&holder, NOW, EXPIRES).unwrap());
        store
            .enqueue_working_tree(&queue_trigger("waiting"), NOW)
            .unwrap();

        assert_eq!(
            store
                .release_working_tree_and_take_next(&stranger, NOW, EXPIRES)
                .unwrap(),
            Vec::new()
        );
        assert_eq!(
            store.working_tree_holder(NOW).unwrap(),
            Some(holder.key()),
            "the real holder's lease still stands"
        );
        assert_eq!(
            store.working_tree_queue_len().unwrap(),
            1,
            "and the entry waiting behind it was not swallowed"
        );
    }

    // ---- "does this scope still owe anybody a reply?" ---------------------------------------

    /// Register `expects` on `thread` — the same `expects_reply_from` write `ask` and
    /// the persona coordinator leave behind.
    fn expect_reply(store: &mut ChatStore, thread: &str, handles: &[&str]) {
        let json = serde_json::to_string(handles).unwrap();
        store.set_expects_reply_from(thread, &json, "local/pm");
    }

    /// `sender` answers in `thread`, which is what discharges its half of the register.
    fn answer(store: &mut ChatStore, thread: &str, sender: &str) {
        store.set_channel_field("c-1", "kind", "group", "local/pm");
        store.post_message(&crate::model::MessageEnvelope {
            origin: "local".into(),
            channel_id: "c-1".into(),
            sender: sender.into(),
            kind: crate::model::MessageKind::Report,
            priority: Priority::Normal,
            disposition: crate::model::Disposition::InTurn,
            thread_id: Some(thread.into()),
            refs: crate::model::Refs::default(),
            body: "done".into(),
        });
    }

    #[test]
    fn a_thread_scope_owes_until_its_expected_handle_answers() {
        let mut store = ChatStore::open_in_memory(1);
        let scope = WorkScope::Thread("th-1".to_string());
        expect_reply(&mut store, "th-1", &["local/coder"]);

        assert!(store.work_scope_has_outstanding(&scope, NOW).unwrap());
        answer(&mut store, "th-1", "local/coder");
        assert!(!store.work_scope_has_outstanding(&scope, NOW).unwrap());
    }

    #[test]
    fn a_thread_scope_with_no_register_at_all_owes_nothing() {
        // The `role_resume` shape (PR review of nxf 6j6v.303b, carried into this ticket): a resume
        // takes a thread-scoped lease without registering any obligation, and the thread's quorum
        // may already be discharged. There is no deterministic done signal for such a lease, so the
        // rule reads it as what it is — a scope with nothing outstanding — and the release path
        // frees it at the next reply into that same thread rather than holding it to the two-hour
        // backstop. See `orchestration::release_working_tree_if_scope_is_done` for the decision.
        let store = ChatStore::open_in_memory(1);
        assert!(!store
            .work_scope_has_outstanding(&WorkScope::Thread("th-never-opened".to_string()), NOW)
            .unwrap());
    }

    #[test]
    fn a_session_scope_never_reports_anything_outstanding_because_it_has_no_threads() {
        let store = ChatStore::open_in_memory(1);
        assert!(store
            .work_scope_threads(&WorkScope::Session("s-1".to_string()))
            .unwrap()
            .is_empty());
        assert!(!store
            .work_scope_has_outstanding(&WorkScope::Session("s-1".to_string()), NOW)
            .unwrap());
    }

    // Three tests stood here — the run arm's own, turned back onto the quorum rule by 6j6v.1xw1,
    // plus a finished run that still owed and a run this device had never folded. All three were
    // about a `run:` claim area, which 6j6v.dvyq §3 removed with the run record. What they were
    // really pinning — that the rule asks the BOARDS and nothing above them — is what every
    // `WorkScope::Thread` test in this module now asserts directly.

    #[test]
    fn a_scope_wider_than_the_bind_ceiling_is_answered_from_the_workspace_wide_read() {
        // Branch review of PR #337, Integrity #5. Since this PR a channel scope pulls in the whole
        // member fan-out, so the id list is bounded by nothing a CALLER controls — it is bounded by
        // how many members somebody declared. One SQL variable per id meets SQLite's 999-variable
        // default; `nxc status` has had the guard since 6j6v.a71h and this path had none, on the one
        // path whose failure mode is a working copy held to its two-hour backstop.
        //
        // The fallback reads the WHOLE workspace, so the assertion that matters is not "it does not
        // error" — it is that the answer is still about THIS scope.
        //
        // **What this test does NOT prove, said out loud**: that the ceiling is needed. SQLite's
        // COMPILED limit has been 32766 since 3.32 — 999 is the documented default the guard is
        // written against — so no test at a size that runs in reasonable time can go red by
        // deleting the guard. What it pins is the branch the guard introduces: past the ceiling the
        // answer comes from a workspace-wide read, and a workspace-wide read that forgot to filter
        // by scope is a lease released on somebody else's silence. That is the failure this can
        // catch, and it is the one worth catching.
        let mut store = ChatStore::open_in_memory(1);
        let supervisor = format!("local/{}", crate::channel::SUPERVISOR_HANDLE);
        let root = |opener: &str, parent: Option<&str>| crate::model::ThreadRoot {
            origin: "local".into(),
            channel_id: "c-1".into(),
            opener: opener.to_string(),
            created: NOW.into(),
            parent: parent.map(str::to_string),
        };

        store.open_thread("th-channel", &root("local/pm", None), "local/pm");
        // One past the ceiling once the channel thread itself is counted.
        let members = crate::facade::MAX_BOUND_IDS;
        for i in 0..members {
            let id = format!("th-m{i:04}");
            store.open_thread(&id, &root(&supervisor, Some("th-channel")), &supervisor);
            expect_reply(&mut store, &id, &["local/coder"]);
            answer(&mut store, &id, "local/coder");
        }
        // OUTSIDE the scope, and owing: the workspace-wide read sees it, the answer must not.
        store.open_thread("th-elsewhere", &root("local/pm", None), "local/pm");
        expect_reply(&mut store, "th-elsewhere", &["local/someone"]);

        let scope = WorkScope::Thread("th-channel".to_string());
        assert_eq!(
            store.work_scope_threads(&scope).unwrap().len(),
            members + 1,
            "the channel thread plus its fan-out — past the bind ceiling, which is the point"
        );
        assert!(
            !store.work_scope_has_outstanding(&scope, NOW).unwrap(),
            "every member answered, so this scope owes nothing — the owing thread outside it is \
             not this scope's business, and a workspace-wide fallback that forgot to filter would \
             say the opposite"
        );

        // …and it still notices the one member that has not answered.
        expect_reply(&mut store, "th-m0007", &["local/late"]);
        assert!(store.work_scope_has_outstanding(&scope, NOW).unwrap());
    }

    // ---- the claim area is a SUBTREE, and the claim thread is CLOSED or it is not (6j6v.1xw1) ---

    /// Open `child` under `parent` (or as a root), opened by `opener` — the `threads` rows a
    /// `send --to` leaves behind, written here so a walk can be pinned without driving a chain.
    fn open_under(store: &mut ChatStore, thread: &str, opener: &str, parent: Option<&str>) {
        store.open_thread(
            thread,
            &crate::model::ThreadRoot {
                origin: "local".into(),
                channel_id: "c-1".into(),
                opener: opener.to_string(),
                created: NOW.into(),
                parent: parent.map(str::to_string),
            },
            opener,
        );
    }

    /// `sender` answers in `thread` with a KIND of its own — the escalating and questioning shapes
    /// [`answer`] cannot express.
    fn answer_of_kind(
        store: &mut ChatStore,
        thread: &str,
        sender: &str,
        kind: crate::model::MessageKind,
    ) {
        store.set_channel_field("c-1", "kind", "group", "local/pm");
        store.post_message(&crate::model::MessageEnvelope {
            origin: "local".into(),
            channel_id: "c-1".into(),
            sender: sender.into(),
            kind,
            priority: Priority::Normal,
            disposition: crate::model::Disposition::InTurn,
            thread_id: Some(thread.into()),
            refs: crate::model::Refs::default(),
            body: "…".into(),
        });
    }

    #[test]
    fn a_thread_scope_is_the_whole_subtree_below_it_and_never_the_thread_above_it() {
        // Acceptance point 2 at the derivation: T1 → T2 (the claim thread) → T3 → T4 → T5, four
        // levels below the claim, plus a sibling of T3 that hangs under T1 and is therefore NOT part
        // of it. One level of children — what this returned before 6j6v.1xw1 — would stop at T3.
        let mut store = ChatStore::open_in_memory(1);
        open_under(&mut store, "th-1", "local/carsten", None);
        open_under(&mut store, "th-2", "local/pm", Some("th-1"));
        open_under(&mut store, "th-3", "local/lead", Some("th-2"));
        open_under(&mut store, "th-4", "local/lead", Some("th-3"));
        open_under(&mut store, "th-5", "local/reviewer", Some("th-4"));
        open_under(&mut store, "th-elsewhere", "local/pm", Some("th-1"));

        assert_eq!(
            store
                .work_scope_threads(&WorkScope::Thread("th-2".to_string()))
                .unwrap(),
            vec!["th-2", "th-3", "th-4", "th-5"],
            "everything below the claim thread, however deep — and nothing beside or above it"
        );
        assert_eq!(
            store
                .work_scope_threads(&WorkScope::Thread("th-1".to_string()))
                .unwrap()
                .len(),
            6,
            "asked ABOUT the top of the chain it is the whole chain; asked about T2 it is not"
        );
    }

    #[test]
    fn a_cycle_in_the_parent_edges_terminates_the_subtree_walk_instead_of_hanging_it() {
        // The parent edge arrives in a synced `thread`/`open` op and is not a validated foreign key,
        // so a cycle is reachable from a peer — `facade::status` already has to survive one. The
        // walk expands each thread at most once, which is the whole termination argument, and this
        // is what would otherwise be an infinite loop on the reply path.
        let mut store = ChatStore::open_in_memory(1);
        open_under(&mut store, "th-a", "local/pm", Some("th-c"));
        open_under(&mut store, "th-b", "local/pm", Some("th-a"));
        open_under(&mut store, "th-c", "local/pm", Some("th-b"));
        open_under(&mut store, "th-self", "local/pm", Some("th-self"));

        assert_eq!(
            store.thread_subtree("th-a").unwrap(),
            vec!["th-a", "th-b", "th-c"],
            "each member of the cycle once"
        );
        assert_eq!(store.thread_subtree("th-self").unwrap(), vec!["th-self"]);
    }

    #[test]
    fn an_ad_hoc_dm_a_session_opens_out_of_its_own_conversation_is_inside_the_claim_area() {
        // nxf 6j6v.fm8k, which this item resolves by construction: the `run:` area 6j6v.dvyq §3
        // has since removed was a set of board ids and did not CONTAIN the threads its sessions
        // opened, so such a DM appeared on the board as belonging to no lease at all. A tree
        // contains it, because a71h stamps it as a child of the thread its session was standing in.
        let mut store = ChatStore::open_in_memory(1);
        open_under(&mut store, "th-board", "local/pm", None);
        open_under(&mut store, "th-dm", "local/coder", Some("th-board"));
        let scope = WorkScope::Thread("th-board".to_string());

        assert_eq!(
            store.work_scope_threads(&scope).unwrap(),
            vec!["th-board", "th-dm"],
            "the claim area contains the ad-hoc DM one of its sessions opened"
        );
        expect_reply(&mut store, "th-dm", &["local/helper"]);
        assert!(
            store.work_scope_has_outstanding(&scope, NOW).unwrap(),
            "and the reply that answers it is therefore this area's business"
        );
    }

    #[test]
    fn a_finished_answer_closes_the_claim_thread_while_a_hand_back_holds_it() {
        // Acceptance points 4 and 5 at the derivation, and the reason the release rule is "the claim
        // thread is CLOSED" rather than "nothing is outstanding": all three answers below discharge
        // the turn identically (6j6v.cg8g gives escalation no special case), so the quorum rule
        // cannot tell them apart. This one can.
        let mut store = ChatStore::open_in_memory(1);
        let scope = WorkScope::Thread("th-claim".to_string());
        open_under(&mut store, "th-claim", "local/pm", None);

        for (kind, holds, what) in [
            (crate::model::MessageKind::Report, false, "I am finished"),
            (
                crate::model::MessageKind::Escalation,
                true,
                "I cannot do this",
            ),
            (
                crate::model::MessageKind::Question,
                true,
                "what do you mean by X?",
            ),
        ] {
            // A fresh declaration each round, so each answer is the answer to ITS OWN turn.
            expect_reply(&mut store, "th-claim", &["local/coder"]);
            assert!(
                store.work_scope_has_outstanding(&scope, NOW).unwrap(),
                "{what}: nobody has answered yet"
            );
            answer_of_kind(&mut store, "th-claim", "local/coder", kind);
            assert!(
                !store.work_scope_has_outstanding(&scope, NOW).unwrap(),
                "{what}: the turn is discharged either way — this is what the quorum rule sees"
            );
            assert_eq!(
                store.work_scope_handed_back(&scope).unwrap(),
                holds,
                "{what}: whether the claim thread was handed back rather than closed"
            );
        }
    }

    #[test]
    fn a_hand_back_holds_the_claim_at_every_depth_of_the_area_and_clears_when_it_is_answered() {
        // The read is the WHOLE claim area, and both halves of that are the branch review's
        // Critical 1. A first cut asked the claim thread and one level of members, on the argument
        // that anything deeper is covered by the quorum rule — which is false, because the level
        // above a hand-back can act by CONSOLIDATING rather than by answering (see
        // `work_scope_handed_back`'s doc, and the end-to-end counter-shape in
        // `working_tree_claim_scope.rs`).
        //
        // Three depths here, each asserted on its own so the test says WHICH depth is read rather
        // than merely that some depth is: the claim thread, a supervised member of it, and a child
        // of that member — the depth the narrowed read could not reach.
        let mut store = ChatStore::open_in_memory(1);
        let supervisor = format!("local/{}", crate::channel::SUPERVISOR_HANDLE);
        let scope = WorkScope::Thread("th-channel".to_string());
        open_under(&mut store, "th-channel", "local/pm", None);
        open_under(&mut store, "th-member", &supervisor, Some("th-channel"));
        open_under(&mut store, "th-deeper", "local/member", Some("th-member"));

        expect_reply(&mut store, "th-channel", &["local/__delivered__"]);
        answer_of_kind(
            &mut store,
            "th-channel",
            "local/__delivered__",
            crate::model::MessageKind::Info,
        );
        assert!(
            !store.work_scope_handed_back(&scope).unwrap(),
            "an ordinary discharge at the claim thread hands nothing back"
        );

        for (depth, thread, handle) in [
            ("a supervised member", "th-member", "local/member"),
            ("a child of that member", "th-deeper", "local/helper"),
        ] {
            expect_reply(&mut store, thread, &[handle]);
            answer_of_kind(
                &mut store,
                thread,
                handle,
                crate::model::MessageKind::Question,
            );
            assert!(
                store.work_scope_handed_back(&scope).unwrap(),
                "{depth} waiting on an answer holds the claim — the level above it can consolidate \
                 past it, so nothing else in the area will say so"
            );
            // …and it CLEARS when that turn is re-opened, which is what answering a hand-back is
            // (6j6v.pf6j re-declares every member's expectation). This is the over-hold the first
            // cut traded the malign direction away to avoid, shown not to exist.
            expect_reply(&mut store, thread, &[handle]);
            assert!(
                !store.work_scope_handed_back(&scope).unwrap(),
                "{depth}: re-declaring the turn moves the watermark, so the hand-back is no longer \
                 this turn's answer and the hold is gone at once"
            );
            answer_of_kind(
                &mut store,
                thread,
                handle,
                crate::model::MessageKind::Report,
            );
            assert!(
                !store.work_scope_handed_back(&scope).unwrap(),
                "{depth}: and the real answer keeps it clear"
            );
        }
    }

    #[test]
    fn an_escalating_board_is_handed_back_and_a_session_scope_has_nothing_to_ask() {
        // TURNED AROUND by the branch review, Important 2, and re-based onto a thread claim area by
        // 6j6v.dvyq §3 (it was written against the `run:` area that item removed). The case is the
        // one that must hold: a board that ESCALATES has nothing outstanding, so without the
        // hand-back rule the working copy would go to a rival while the escalation travels up.
        let mut store = ChatStore::open_in_memory(1);
        let scope = WorkScope::Thread("th-board".to_string());
        open_under(&mut store, "th-board", "local/pm", None);
        // And one level below the board, which the area reaches — the shape 6j6v.fm8k describes.
        open_under(&mut store, "th-dm", "local/coder", Some("th-board"));

        expect_reply(&mut store, "th-board", &["local/coder"]);
        answer_of_kind(
            &mut store,
            "th-board",
            "local/coder",
            crate::model::MessageKind::Escalation,
        );
        assert!(
            !store.work_scope_has_outstanding(&scope, NOW).unwrap(),
            "the escalation discharged the turn, so the quorum rule alone would release"
        );
        assert!(
            store.work_scope_handed_back(&scope).unwrap(),
            "but the run's own board handed the task back, and the copy stays claimed"
        );

        expect_reply(&mut store, "th-dm", &["local/helper"]);
        answer_of_kind(
            &mut store,
            "th-dm",
            "local/helper",
            crate::model::MessageKind::Question,
        );
        expect_reply(&mut store, "th-board", &["local/coder"]);
        answer_of_kind(
            &mut store,
            "th-board",
            "local/coder",
            crate::model::MessageKind::Report,
        );
        assert!(
            store.work_scope_handed_back(&scope).unwrap(),
            "…and below the board too: the board finished, the DM under it is still waiting"
        );

        // A session scope has no threads at all, so there is nothing to ask — the same answer, for
        // the same reason, as `work_scope_has_outstanding`.
        assert!(!store
            .work_scope_handed_back(&WorkScope::Session("s-1".to_string()))
            .unwrap());
    }

    #[test]
    fn every_message_kind_is_sorted_into_hand_back_or_conclusion_by_an_exhaustive_match() {
        // The set is small and the match behind it has no `_` arm, so adding a kind is a compile
        // error at `hands_the_task_back` rather than a silent extra "concludes". Pinned by value
        // here so the two hand-backs cannot be quietly reclassified either.
        //
        // **THE COMPILE ERROR IS UPSTREAM OF THIS LIST, AND THIS LIST IS WHERE IT STOPS BEING
        // ENFORCED** (independent review of PR #470, Test Quality #2). An author forced to the
        // match is not forced to come here, and two kinds proved it: `needs_rework` (nxf
        // 6j6v.553s) and `accepted` (nxf 6j6v.am8j) were both classified correctly at the match and
        // both left out of this table — so the one thing pinning "a verdict does not hold the
        // checkout" by VALUE did not exist, and a reclassification would have gone green. Derived
        // again here against `MessageKind`'s variants rather than extended by the one the review
        // named; the count below is what makes a third omission visible.
        let table = [
            ("escalation", true),
            ("question", true),
            ("report", false),
            ("decision", false),
            ("info", false),
            ("task", false),
            ("needs_rework", false),
            ("accepted", false),
        ];
        assert_eq!(
            table.len(),
            crate::store::tests::every_message_kind().len(),
            "every `MessageKind` is classified BY VALUE here, not only at the exhaustive match — \
             add the missing one rather than raising this number"
        );
        for (label, back) in table {
            assert_eq!(hands_the_task_back(label), back, "kind {label}");
        }
        assert!(
            hands_the_task_back("a-kind-from-the-future"),
            "an unparseable label answers conservatively: hold the copy rather than release it"
        );
    }
}
