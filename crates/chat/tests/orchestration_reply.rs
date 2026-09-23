//! `reply` — including the requester wake and the completion routing — driven through the library
//! seam (nxf 6j6v.7dw6).
//!
//! This is the ticket's behaviour fix, not just a move: `Engine::reply` used to call
//! `facade::reply` and stop there, so an app replying through the handle left boards that never
//! completed and workflow runs that never advanced — silently, behind a successful receipt. The
//! whole post-then-route chain now lives in `orchestration` behind an explicit `Ctx`, which is what
//! these tests drive: a `Ctx`-supplied catalogue with no declaration folder anywhere, no `NXC_*` in
//! the environment, a recording worker instead of a spawned sidecar. (The catalogue is the
//! `&Definitions` the verb borrows — not the seam injection nxf 6j6v.dvyq step 6 removed, which is
//! a storey up.)
//!
//! The receipt is part of the deliverable: every test asserts what `ReplyReceipt` REPORTED
//! (`woke`/`wake_skipped`/`completed`) alongside the store/worker state it produced, because a
//! caller that has to guess whether its reply completed a board is exactly the failure mode this
//! ticket exists to fix. `wake_skipped` is nxf 6j6v.bxdd's half of that: a wake that was attempted
//! and did not land says so, on BOTH completion paths, instead of hiding behind a bare `Ok`. Since
//! nxf 6j6v.0akf the direct 1:1 return-address resume reports it the same way — one contract across
//! every wake site, with no path left that returns `Err` for a message it already persisted.

use std::sync::Mutex;

use nexus_chat::channel::{ChannelDecl, DELIVERED_HANDLE, SYNTHESIS_HANDLE};
use nexus_chat::definitions::Definitions;
use nexus_chat::error::ErrorKind;
use nexus_chat::facade::{self, AskRequest};
use nexus_chat::model::{Disposition, MessageKind, Priority, Refs, ThreadRoot};
use nexus_chat::orchestration::{
    self, Ctx, ReplyRequest, RoleResumeRequest, WakeSkipReason, WakeSkipped, MAX_HOP,
};
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::worker::{TriggerRequest, Worker};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-08-01T10:00:00Z";

#[derive(Default)]
struct RecordingWorker {
    seen: Mutex<Vec<TriggerRequest>>,
}

impl RecordingWorker {
    /// The triggered role handles, in trigger order.
    fn handles(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.role.handle.clone())
            .collect()
    }

    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    /// The one and only trigger this worker saw.
    fn only(&self) -> TriggerRequest {
        let seen = self.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "expected exactly one trigger, got {seen:?}");
        seen[0].clone()
    }

    fn last(&self) -> TriggerRequest {
        self.seen
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("at least one trigger")
    }
}

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> nexus_chat::worker::TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(nexus_chat::worker::TriggerOutcome::Accepted)
    }
}

/// A worker that refuses every trigger — the "the spawn itself failed" case, which must still leave
/// the persisted reply successful.
struct FailingWorker;

impl Worker for FailingWorker {
    fn trigger(&self, _req: TriggerRequest) -> nexus_chat::worker::TriggerResult {
        Err(nexus_chat::error::NxfError::io("no sidecar configured").into())
    }
}

/// A worker that records every trigger it is handed but refuses exactly one role — the "one of the
/// promoted siblings could not be started" case (PR #336 review, Code Quality #1), which must leave
/// both the reply and the other siblings standing.
struct RefusingWorker {
    refuse: &'static str,
    seen: Mutex<Vec<TriggerRequest>>,
}

impl RefusingWorker {
    fn refusing(role: &'static str) -> Self {
        RefusingWorker {
            refuse: role,
            seen: Mutex::new(Vec::new()),
        }
    }

    /// The triggered role handles, in trigger order — including the one that was refused, which is
    /// the point: it was ATTEMPTED, and its failure stopped nothing after it.
    fn handles(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.role.handle.clone())
            .collect()
    }
}

impl Worker for RefusingWorker {
    fn trigger(&self, req: TriggerRequest) -> nexus_chat::worker::TriggerResult {
        let refused = req.role.handle == self.refuse;
        self.seen.lock().unwrap().push(req);
        if refused {
            return Err(nexus_chat::error::NxfError::io("no sidecar configured").into());
        }
        Ok(nexus_chat::worker::TriggerOutcome::Accepted)
    }
}

/// A worker whose runtime has REAPED the session it is asked to resume — the routine cloud case
/// (nxf 6j6v.5x9j: silent death at 15 minutes idle), which every wake site must classify apart from
/// an ordinary refused spawn.
struct ReapedWorker;

impl Worker for ReapedWorker {
    fn trigger(&self, req: TriggerRequest) -> nexus_chat::worker::TriggerResult {
        Err(nexus_chat::worker::TriggerError::SessionGone {
            session: req.resume_real.unwrap_or_else(|| "<unbound>".into()),
            detail: "runtime reaped the session after 15 minutes idle".into(),
        })
    }
}

/// These suites never assert on SCHEDULING, and since the timer became an injected value
/// (PR #269 review, Code Quality #3) they no longer have to tolerate whatever `NXC_TIMER`
/// happens to be — unset, that used to mean a real `at` subprocess per armed deadline.
static NO_TIMER: nexus_chat::timer::DisabledTimer = nexus_chat::timer::DisabledTimer;

fn role(yaml: &str) -> RoleDecl {
    serde_yaml::from_str(yaml).expect("test role parses")
}

fn simple_role(handle: &str) -> RoleDecl {
    role(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
}

fn channel_decl(yaml: &str) -> ChannelDecl {
    serde_yaml::from_str(yaml).expect("test channel parses")
}

/// The `NXC_HOP` a trigger handed the session it woke — the half of the depth contract that lives
/// in the spawned process's environment rather than in the store (nxf 6j6v.ka09).
fn hop_stamp(req: &TriggerRequest) -> String {
    req.env
        .iter()
        .find(|(k, _)| k == "NXC_HOP")
        .map(|(_, v)| v.clone())
        .expect("every trigger stamps NXC_HOP")
}

/// A fresh workspace + store. No declaration folder is ever created — that is the point.
fn fresh_store() -> (TempDir, ChatStore) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
    let store = ws.open_chat_store().expect("open chat store");
    (tmp, store)
}

/// A `Ctx` acting as `actor`, with no ambient session.
fn ctx<'a>(defs: &'a Definitions, worker: &'a dyn Worker, actor: &'a str) -> Ctx<'a> {
    ctx_in_session(defs, worker, actor, None, 0)
}

fn ctx_in_session<'a>(
    defs: &'a Definitions,
    worker: &'a dyn Worker,
    actor: &'a str,
    session: Option<&'a str>,
    hop: u32,
) -> Ctx<'a> {
    Ctx {
        now: NOW,
        origin: "local",
        actor,
        session,
        hop,
        defs,
        worker,
        timer: &NO_TIMER,
        namer: &nexus_chat::naming::DryNamer,
        db_path: "/w/.nxs/db.sqlite",
        project_claude_md: None,
        module_primes: None,
        machines: None,
    }
}

fn reply_req<'a>(target: &'a str, body: &'a str) -> ReplyRequest<'a> {
    ReplyRequest {
        target,
        body,
        kind: MessageKind::Report,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        refs: Refs::default(),
        model: None,
        if_unanswered: false,
    }
}

/// Open a quorum board directly through the facade — the state a `reply` under test lands in.
/// `opener_session` becomes the thread root's return address, exactly as `ask`'s own ambient stamp
/// would leave it.
fn seed_ask(
    store: &mut ChatStore,
    channel_id: &str,
    opener: &str,
    opener_session: Option<&str>,
    expect: &[String],
    body: &str,
) -> String {
    store.set_channel_field(channel_id, "kind", "group", &format!("local/{opener}"));
    facade::ask(
        store,
        AskRequest {
            now: NOW,
            origin: "local",
            actor: opener,
            channel: channel_id,
            body,
            expect,
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs {
                session_id: opener_session.map(str::to_string),
                ..Default::default()
            },
        },
    )
    .expect("ask succeeds")
    .thread_id
}

/// A plain post carrying `session` as its return address — the state the direct 1:1 resume in step
/// (3) acts on. `None` leaves the message with no return address at all.
fn seed_addressed_post(store: &mut ChatStore, session: Option<&str>) -> String {
    store.set_channel_field("c-1", "kind", "group", "local/pm");
    facade::send(
        store,
        facade::SendRequest {
            now: NOW,
            origin: "local",
            actor: "pm",
            channel: "c-1",
            body: "a task for you",
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: None,
            refs: Refs {
                session_id: session.map(str::to_string),
                ..Default::default()
            },
        },
    )
    .expect("the post succeeds")
    .message_id
}

fn quorum_expects(store: &ChatStore, thread_id: &str) -> Vec<String> {
    store
        .thread_quorum(thread_id, NOW)
        .unwrap()
        .expect("the thread exists")
        .expects
}

fn message_count(store: &ChatStore) -> i64 {
    store
        .connection()
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap()
}

/// The body a message id has in the `messages` view, or `None` if it is not there at all — the
/// "it WAS persisted" check for a reply that lands in no thread (a reply to a plain, unthreaded
/// post inherits its target's `None` thread, so `messages_in_thread` has nothing to ask about).
fn persisted_body(store: &ChatStore, message_id: &str) -> Option<String> {
    store
        .connection()
        .query_row(
            "SELECT body FROM messages WHERE message_id=?1",
            [message_id],
            |r| r.get(0),
        )
        .ok()
}

// ---- the quorum wake ------------------------------------------------------------------------

#[test]
fn reply_completing_a_quorum_wakes_the_opener_role() {
    // The fan-in PUSH wake: the board's opener stamped its own session as the thread root's return
    // address, and the LAST expected handle's reply resumes it — exactly once, exactly on the
    // completing reply, with a synthesized wake message rather than the reviewer's raw text.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let worker = RecordingWorker::default();

    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "real-pm").unwrap();
    let tid = seed_ask(
        &mut store,
        "c-1",
        "pm",
        Some("s-pm"),
        &["local/bob".to_string(), "local/carol".to_string()],
        "please review",
    );

    // 1 of 2 — nothing to wake yet.
    let first = orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&tid, "looks fine"),
    )
    .unwrap();
    assert_eq!(first.woke, None, "the board is not complete yet");
    assert_eq!(worker.count(), 0);

    // 2 of 2 — the completing reply.
    let second = orchestration::reply(
        &ctx(&defs, &worker, "carol"),
        &mut store,
        reply_req(&tid, "approved"),
    )
    .unwrap();

    assert_eq!(
        second.woke.as_deref(),
        Some("s-pm"),
        "the receipt names the session it woke instead of making the caller guess"
    );
    assert_eq!(
        second.completed, None,
        "a plain channel has no declared on_complete policy to route through"
    );
    let t = worker.only();
    assert_eq!(t.role.handle, "pm");
    assert_eq!(t.internal_session, "s-pm");
    assert_eq!(t.resume_real.as_deref(), Some("real-pm"));
    assert!(
        t.message.contains("(2/2 replied)"),
        "a synthesized wake, not the raw reviewer text: {}",
        t.message
    );
}

/// The completing reply is the sender's first **since the obligation was declared**, not their first
/// in the thread (nxf 6j6v.pf6j, carried over from 6j6v.cg8g).
///
/// `is_completing_reply` was a second, INDEPENDENT expression of what a discharge is, written before
/// the turn watermark existed: "this handle has exactly one message in this thread". From turn two on
/// that is never true of anybody, so a re-declared board's second completion routed NOWHERE — the
/// opener was woken for turn one and then never again, silently, with the board reading `complete`
/// the whole time. Multi-turn threads are the normal case since this item, so the two readings had to
/// become one; the question is now asked of `ChatStore::replies_since_the_declaration`, which splices
/// the same comparison `replied`/`outstanding` are computed from.
#[test]
fn the_second_turns_completing_reply_wakes_the_opener_exactly_as_the_first_turns_did() {
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let worker = RecordingWorker::default();

    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "real-pm").unwrap();
    let tid = seed_ask(
        &mut store,
        "c-1",
        "pm",
        Some("s-pm"),
        &["local/bob".to_string()],
        "please review",
    );

    // Turn one: the ordinary shape, and the one the old reading also got right.
    let first = orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&tid, "turn one"),
    )
    .unwrap();
    assert_eq!(first.woke.as_deref(), Some("s-pm"), "{first:?}");
    assert_eq!(worker.count(), 1);

    // The opener asks again — `facade::set_expects`, the seam write that opens the next turn and the
    // one the channel supervisor itself uses. Since 6j6v.cg8g a re-declaration is a QUESTION, so bob
    // owes an answer to THIS declaration whether or not he answered the last one.
    facade::set_expects(
        &mut store,
        NOW,
        "local/pm",
        &tid,
        &["local/bob".to_string()],
    )
    .unwrap();
    assert!(
        !store.thread_quorum(&tid, NOW).unwrap().unwrap().complete,
        "the debt is genuinely re-opened"
    );

    // Turn two. Bob's post is no longer his FIRST in the thread — under the old reading nothing
    // routed at all, and the opener was never told the board had completed again.
    let second = orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&tid, "turn two"),
    )
    .unwrap();
    assert_eq!(
        second.woke.as_deref(),
        Some("s-pm"),
        "the second turn's completion wakes the opener exactly as the first did: {second:?}"
    );
    assert_eq!(
        worker.count(),
        2,
        "one wake per turn — not one for the first turn and silence forever after"
    );

    // …and a FURTHER post from bob into the now-settled board is not a third completion: he has
    // spoken since this declaration, so nothing about it flipped `complete` false→true.
    let extra = orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&tid, "one more thought"),
    )
    .unwrap();
    assert_eq!(
        extra.woke, None,
        "a follow-up inside the same turn wakes nobody: {extra:?}"
    );
    assert_eq!(worker.count(), 2);
}

#[test]
fn a_quorum_completion_wake_does_not_deepen_the_opener_it_wakes() {
    // nxf 6j6v.ka09 (PR #278 review, Test Quality #1). The fan-in wake hands the opener the result
    // of the board IT commissioned, so it is a `ChainMove::Unwind` and must leave the opener's own
    // depth alone. The functional test above pins WHO was woken and with what message, never at
    // what depth — so a regression flipping this one site back to `Deeper` would pass the whole
    // suite in silence, which is exactly what this covers.
    //
    // **The reviewer has to be DEEPER than the opener**, or the two arms compute the same number
    // and the test cannot tell them apart. At opener 2 / reviewer 3, `Deeper` would write
    // MAX(2, 3 + 1) = 4 and stamp `NXC_HOP=4`; `Unwind` leaves both at 2.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let worker = RecordingWorker::default();

    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "real-pm").unwrap();
    store.record_trigger_depth("s-pm", 2).unwrap();
    store.create_pending_session("s-carol", "pm").unwrap();
    store.record_trigger_depth("s-carol", 3).unwrap();
    let tid = seed_ask(
        &mut store,
        "c-1",
        "pm",
        Some("s-pm"),
        &["local/bob".to_string(), "local/carol".to_string()],
        "please review",
    );

    orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&tid, "looks fine"),
    )
    .unwrap();
    let second = orchestration::reply(
        &ctx_in_session(&defs, &worker, "carol", Some("s-carol"), 0),
        &mut store,
        reply_req(&tid, "approved"),
    )
    .unwrap();

    assert_eq!(second.woke.as_deref(), Some("s-pm"), "the board completed");
    assert_eq!(
        store.session_depth("s-pm").unwrap(),
        Some(2),
        "the opener commissioned this board from depth 2 and re-enters at depth 2"
    );
    assert_eq!(
        hop_stamp(&worker.only()),
        "2",
        "and is TOLD the same number it is recorded at — `resolve_hop` takes the larger of the \
         two, so a stamp of the reviewer's depth + 1 would re-arm the ratchet on the opener's very \
         next verb"
    );
}

#[test]
fn a_bad_wake_target_skips_the_wake_instead_of_failing_the_persisted_reply() {
    // `resolve_quorum_wake_target`'s documented contract: a return address whose role is gone
    // (deleted declaration, malformed YAML, a session-map row that no longer resolves) SKIPS the
    // wake. It must never fail the reply that discovered it — that reply is already persisted, and
    // its text is valid content regardless of whether a downstream wake happens to fire.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("bob")], vec![]).unwrap();
    let worker = RecordingWorker::default();

    // The opener's session is registered, but the role it names is no longer declared.
    store.create_pending_session("s-gone", "ghost").unwrap();
    let tid = seed_ask(
        &mut store,
        "c-1",
        "ghost",
        Some("s-gone"),
        &["local/bob".to_string()],
        "please review",
    );

    let receipt = orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&tid, "lgtm"),
    )
    .expect("a bad wake target must not fail the reply");

    assert_eq!(
        receipt.woke, None,
        "nothing was woken, and the receipt says so"
    );
    assert_eq!(
        receipt.wake_skipped,
        Some(WakeSkipped {
            session: "s-gone".to_string(),
            reason: WakeSkipReason::Unresolved,
            detail: None,
        }),
        "and the skip is VISIBLE, not silent — `woke: None` alone reads as \"nothing to wake\""
    );
    assert_eq!(receipt.completed, None);
    assert_eq!(worker.count(), 0, "nothing may be triggered");
    assert!(
        store
            .messages_in_thread(&tid)
            .unwrap()
            .iter()
            .any(|m| Some(&m.message_id) == receipt.message_id.as_ref()),
        "the reply itself is persisted"
    );
}

#[test]
fn a_failing_wake_on_a_plain_channel_leaves_the_reply_ok_and_reports_the_skip() {
    // nxf 6j6v.bxdd, the owner's decision. `route_completion`'s NON-declared fallback arm used to
    // call `trigger_role_at(…)?` and propagate: a completing reply into a plain channel whose opener
    // could not be spawned returned `Err` with the reply ALREADY persisted — an error for a message
    // that was in fact written. It now behaves exactly like the declared path below: stderr
    // breadcrumb, `Ok`, and the finding in the receipt so a bare `Ok` cannot swallow the fact that
    // the next role was never woken.
    //
    // This is the test that pins the four parts of that contract together — persisted, wake failed,
    // `Ok`, finding in the receipt — because any one of them alone is re-derivable and the
    // COMBINATION is the decision.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("pm"), simple_role("bob")], vec![]).unwrap();
    let worker = FailingWorker;

    // The wake target resolves perfectly — a live session bound to a declared role. Only the spawn
    // itself fails, which is what separates this from the `Unresolved` case above.
    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "real-pm").unwrap();
    let tid = seed_ask(
        &mut store,
        "c-1",
        "pm",
        Some("s-pm"),
        &["local/bob".to_string()],
        "please review",
    );

    let receipt = orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&tid, "lgtm"),
    )
    .expect("a refused spawn must not fail the already-persisted reply");

    assert!(
        store
            .messages_in_thread(&tid)
            .unwrap()
            .iter()
            .any(|m| Some(&m.message_id) == receipt.message_id.as_ref() && m.body == "lgtm"),
        "the reply IS persisted — which is exactly why returning `Err` was a lie"
    );
    assert_eq!(receipt.woke, None, "the wake did not land");
    let skipped = receipt
        .wake_skipped
        .expect("…and the receipt says so, instead of a bare Ok swallowing it");
    assert_eq!(skipped.session, "s-pm", "which session was left un-woken");
    assert_eq!(
        skipped.reason,
        WakeSkipReason::TriggerFailed,
        "the worker refused the spawn — distinct from a target that no longer resolves, because \
         the two call for different responses"
    );
    assert!(
        skipped
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("no sidecar configured")),
        "the underlying failure travels with it: {:?}",
        skipped.detail
    );
    assert_eq!(
        receipt.completed, None,
        "a plain channel has no declared on_complete policy to route through"
    );
}

#[test]
fn a_reaped_runtime_session_is_classified_apart_from_a_refused_spawn_at_every_wake_site() {
    // PR #269 review, Test Quality #3. `SessionGone` -> `WakeSkipReason::SessionGone` was pinned at
    // exactly ONE of the sites that route through it, even though all three call the same shared
    // classifier — and "they share a function" is a claim that stops being true the moment someone
    // inlines it. The distinction is load-bearing: a refused spawn is worth retrying against the
    // same session, a reaped one never is, and on a cloud runtime the reaped case is the ROUTINE
    // one (silent death at 15 minutes idle), not the exception.
    //
    // The two sites covered here are the completion wakes, non-declared and declared; the direct
    // 1:1 resume is covered at the library seam in `embed_orchestration.rs`.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![simple_role("pm"), simple_role("bob")],
        vec![channel_decl(
            "name: review\nmembers: [bob]\non_complete: pass_through\n",
        )],
    )
    .unwrap();
    let worker = ReapedWorker;
    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "agentcore://sess-dead").unwrap();

    // (a) the NON-declared quorum wake.
    let plain = seed_ask(
        &mut store,
        "c-1",
        "pm",
        Some("s-pm"),
        &["local/bob".to_string()],
        "please review",
    );
    let receipt = orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&plain, "lgtm"),
    )
    .expect("a vanished session must not fail the already-persisted reply either");
    let skipped = receipt.wake_skipped.expect("the skip is reported");
    assert_eq!(
        skipped.reason,
        WakeSkipReason::SessionGone,
        "the non-declared quorum wake classifies a reaped session as its own case"
    );
    assert!(
        skipped
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("agentcore://sess-dead"),
        "and names what vanished: {skipped:?}"
    );

    // (b) the DECLARED channel's completion wake, same worker, same expectation.
    store.create_pending_session("s-pm2", "pm").unwrap();
    store
        .bind_session("s-pm2", "agentcore://sess-dead-2")
        .unwrap();
    let declared = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-pm2"),
        &["local/bob".to_string()],
        "please review",
    );
    let receipt = orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&declared, "lgtm"),
    )
    .expect("…on the declared path too");
    assert_eq!(
        receipt.wake_skipped.expect("the skip is reported").reason,
        WakeSkipReason::SessionGone,
        "one classifier, one answer, at every site that routes through it"
    );
}

#[test]
fn a_declared_channels_failing_wake_still_leaves_the_reply_successful() {
    // The same contract on the declared-channel side, where it has always held: the wake target
    // resolves fine, but the worker itself refuses the spawn. `wake_role_requester` never propagates
    // — stderr breadcrumb and `Ok` — because the completion is already delivered/recorded by then,
    // and the receipt stays honest about the wake not having landed. Since nxf 6j6v.bxdd "honest"
    // means the SAME `wake_skipped` finding the non-declared twin above reports: one contract, two
    // paths, no asymmetry left for a reader to rediscover.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![simple_role("pm"), simple_role("bob")],
        vec![channel_decl(
            "name: review\nmembers: [bob]\non_complete: pass_through\n",
        )],
    )
    .unwrap();
    let worker = FailingWorker;

    store.create_pending_session("s-pm", "pm").unwrap();
    let tid = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-pm"),
        &["local/bob".to_string()],
        "please review",
    );

    let receipt = orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&tid, "lgtm"),
    )
    .expect("a failed wake spawn must not fail the reply");

    assert!(receipt
        .completed
        .expect("the completion itself still happened")
        .delivered
        .contains("lgtm"));
    assert_eq!(
        receipt.woke, None,
        "the wake did not land, and the receipt is honest about it"
    );
    assert_eq!(
        receipt.wake_skipped,
        Some(WakeSkipped {
            session: "s-pm".to_string(),
            reason: WakeSkipReason::TriggerFailed,
            detail: Some("io: no sidecar configured".to_string()),
        }),
        "the declared path reports the skip in exactly the shape the non-declared one does"
    );
}

#[test]
fn a_reply_never_wakes_the_session_that_posted_it() {
    // A role replying into a board it opened itself must not re-trigger its own session.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let worker = RecordingWorker::default();

    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "real-pm").unwrap();
    let tid = seed_ask(
        &mut store,
        "c-1",
        "pm",
        Some("s-pm"),
        &["local/pm".to_string()],
        "please review",
    );

    let receipt = orchestration::reply(
        &ctx_in_session(&defs, &worker, "pm", Some("s-pm"), 0),
        &mut store,
        reply_req(&tid, "answering myself"),
    )
    .unwrap();

    assert_eq!(receipt.woke, None);
    assert_eq!(worker.count(), 0, "no self-resume");
}

// ---- the direct return-address resume, and the quorum-thread guard ----------------------------

#[test]
fn a_hand_back_on_the_direct_resume_announces_itself_before_the_answer() {
    // nxf 6j6v.gk9j's THIRD path, and the one the review of PR #361 found untested (Test Quality
    // #2). A hand-back reaches whoever must act on it three ways — a channel's pass-through, the
    // 1:1 fallback in `crate::surface`, and THIS one: step (3) of `orchestration::reply`, the direct
    // resume into a thread that declares no board.
    //
    // It is tested HERE and not through the handle, and that is the finding rather than a
    // convenience: reaching step (3) needs a MESSAGE-id target, because the branch resolves its
    // address with `message_return_address(req.target)` and a thread id names no message. Every
    // surface above this function passes a thread (`Engine::reply` went in 6j6v.ckeq), so a test
    // written against the handle silently exercises the `surface` fallback instead — which is what
    // a first attempt at this test did, and only a mutation probe said so: removing the notice from
    // THIS branch left it green.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let worker = RecordingWorker::default();

    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "real-pm").unwrap();
    let posted = seed_addressed_post(&mut store, Some("s-pm"));

    let mut req = reply_req(&posted, "The migration needs a credential I do not have.");
    req.kind = MessageKind::Escalation;
    let receipt = orchestration::reply(&ctx(&defs, &worker, "coder"), &mut store, req).unwrap();

    assert!(
        receipt.resumed,
        "the premise: this really is step (3), and it really did resume the return address"
    );
    let handed = worker.only().message;
    assert!(
        handed.starts_with("ESCALATION"),
        "the requester meets the hand-back in the first thing it reads, on this path as on the \
         other two, got:\n{handed}"
    );
    assert!(
        handed.contains("working copy") && handed.contains("Decide now"),
        "…with the cost while it stands and what is expected of it, got:\n{handed}"
    );
    assert!(
        handed.contains("credential I do not have"),
        "and the answer itself travels with it — this path carries the reply text, unlike the \
         quorum wake, which hands over a pointer, got:\n{handed}"
    );
}

#[test]
fn an_ordinary_reply_on_the_direct_resume_is_handed_over_untouched() {
    // The control for the test above, and it is not decoration: the notice rides on a branch every
    // ordinary 1:1 answer takes, so a finished exchange must reach its requester as itself. Pinned
    // as an EQUALITY — `assert!(!contains("ESCALATION"))` would also pass for a body that had grown
    // some other preamble.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let worker = RecordingWorker::default();

    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "real-pm").unwrap();
    let posted = seed_addressed_post(&mut store, Some("s-pm"));

    orchestration::reply(
        &ctx(&defs, &worker, "coder"),
        &mut store,
        reply_req(&posted, "the migration ran; 412 rows moved"),
    )
    .unwrap();

    assert_eq!(worker.only().message, "the migration ran; 412 rows moved");
}

#[test]
fn a_reply_resumes_the_targets_own_return_address_session() {
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let worker = RecordingWorker::default();

    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "real-pm").unwrap();
    let posted = seed_addressed_post(&mut store, Some("s-pm"));

    let receipt = orchestration::reply(
        &ctx(&defs, &worker, "coder"),
        &mut store,
        reply_req(&posted, "which db?"),
    )
    .unwrap();

    assert!(receipt.resumed);
    assert_eq!(receipt.woke.as_deref(), Some("s-pm"));
    assert_eq!(
        receipt.wake_skipped, None,
        "a resume that LANDED reports no finding — the field means what it says"
    );
    let t = worker.only();
    assert_eq!(t.role.handle, "pm");
    assert_eq!(t.resume_real.as_deref(), Some("real-pm"));
    assert_eq!(
        t.message, "which db?",
        "the direct resume forwards the reply verbatim"
    );
}

#[test]
fn a_non_dm_thread_with_no_expects_still_resumes_the_return_address_directly() {
    // Round-2 regression (review of nxf 6j6v.jepk, owner ruling 2026-08-12): round 1's fix keyed
    // `reply`'s `is_quorum_thread` on the CHANNEL alone (`!is_dm_channel`), which correctly kept a
    // persona DM out of the quorum branch but, on its own, ALSO pulled every non-DM thread with an
    // EMPTY `expects_reply_from` into it — a completely ordinary shape (a plain group-channel
    // message naming a return address, no board ever declared on it) — skipping the direct 1:1
    // resume at step (3) for THAT thread too.
    //
    // MOVED HERE from `embed_orchestration.rs` by nxf 6j6v.ckeq, and the move is the point rather
    // than tidying. The regression is only visible on a MESSAGE-addressed reply: `reply --thread`
    // (`surface::reply_in_thread`) has a return-address fallback of its own that would mask it, and
    // with `Engine::reply` gone the seam takes a thread and nothing else. So the one entrance that
    // can still ask this question is `orchestration::reply` itself, which is this file's subject.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("pm"), simple_role("bob")], vec![]).unwrap();
    let worker = RecordingWorker::default();

    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "real-pm").unwrap();
    store.set_channel_field("c-1", "kind", "group", "local/pm");
    store.open_thread(
        "t-plain",
        &ThreadRoot {
            origin: "local".to_string(),
            channel_id: "c-1".to_string(),
            opener: "local/pm".to_string(),
            created: NOW.to_string(),
            parent: None,
        },
        "local/pm",
    );
    // Deliberately NO `set_expects_reply_from` — nothing ever declared a board on this thread.
    let target = facade::send(
        &mut store,
        facade::SendRequest {
            now: NOW,
            origin: "local",
            actor: "pm",
            channel: "c-1",
            body: "a task for you",
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: Some("t-plain"),
            refs: Refs {
                session_id: Some("s-pm".to_string()),
                ..Default::default()
            },
        },
    )
    .expect("the post succeeds")
    .message_id;

    let receipt = orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&target, "which db?"),
    )
    .expect("the reply is persisted");

    assert!(
        receipt.resumed,
        "a non-DM thread that declares no board is not a quorum thread: {receipt:?}"
    );
    assert_eq!(receipt.woke.as_deref(), Some("s-pm"));
    assert_eq!(receipt.wake_skipped, None);
}

#[test]
fn a_failing_direct_resume_leaves_the_reply_ok_and_reports_the_skip() {
    // nxf 6j6v.0akf, the owner's decision — the fourth and last site of this defect, and by far the
    // most commonly hit of the four. Step (3) called `trigger_role_at(…)?` AFTER the reply was
    // persisted, so a target whose session could not be spawned came back as `Err` for a message
    // that was in the store and readable: exactly what 6j6v.bxdd removed from the completion wake,
    // one level up.
    //
    // The four parts of the contract are pinned TOGETHER here, because each alone is re-derivable
    // and the COMBINATION is the decision: persisted, `Ok`, `resumed: false`, and a `wake_skipped`
    // naming the session and the reason. `resumed: false` + `woke: None` is ambiguous on its own —
    // it is also what an ordinary reply to a plain post reports (the test below) — and this finding
    // is the only thing that separates the two. Without it, switching to `Ok` would be a silent
    // failure rather than a visibly skipped one.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let worker = FailingWorker;

    // The resume target resolves perfectly — a live session bound to a declared role. Only the
    // spawn itself fails, which is what separates this from the `Unresolved` case below.
    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "real-pm").unwrap();
    let posted = seed_addressed_post(&mut store, Some("s-pm"));

    let receipt = orchestration::reply(
        &ctx(&defs, &worker, "coder"),
        &mut store,
        reply_req(&posted, "which db?"),
    )
    .expect("a refused spawn must not fail the already-persisted reply");

    assert_eq!(
        persisted_body(&store, receipt.message_id.as_deref().unwrap()).as_deref(),
        Some("which db?"),
        "the reply IS persisted — which is exactly why returning `Err` was a lie"
    );
    assert!(!receipt.resumed, "the resume did not happen");
    assert_eq!(receipt.woke, None, "so nothing was woken");
    assert_eq!(
        receipt.wake_skipped,
        Some(WakeSkipped {
            session: "s-pm".to_string(),
            reason: WakeSkipReason::TriggerFailed,
            detail: Some("io: no sidecar configured".to_string()),
        }),
        "…and the receipt says so, in the very shape both completion paths already use: one \
         contract across all four wake sites, not four sites with two possible answers"
    );
    assert_eq!(receipt.completed, None, "no board was completed here");
}

#[test]
fn a_direct_resume_whose_role_is_gone_reports_unresolved_rather_than_failing() {
    // The second failure point in the same block, and the reason `WakeSkipReason` has two variants:
    // the session-map row still names a role, but no catalogue declares it any more. The return
    // address is orphaned — nothing to retry — where a refused spawn is retryable plumbing, so the
    // two call for different responses and both deserve their own coverage.
    //
    // `detail` stays `None`, exactly as `WakeSkipped`'s own doc says for `Unresolved`: the three
    // lookups behind this outcome (`session_role`, `resolve_real`, the catalogue) are collapsed to
    // "did not resolve", the same collapse `resolve_quorum_wake_target` makes for the completion
    // wake.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let worker = RecordingWorker::default();

    // The session IS registered, so a wake is genuinely owed — to a role that is gone.
    store.create_pending_session("s-gone", "ghost").unwrap();
    let posted = seed_addressed_post(&mut store, Some("s-gone"));

    let receipt = orchestration::reply(
        &ctx(&defs, &worker, "coder"),
        &mut store,
        reply_req(&posted, "which db?"),
    )
    .expect("an unresolvable return address must not fail the already-persisted reply");

    assert_eq!(
        persisted_body(&store, receipt.message_id.as_deref().unwrap()).as_deref(),
        Some("which db?"),
        "the reply is written either way"
    );
    assert!(!receipt.resumed);
    assert_eq!(receipt.woke, None);
    assert_eq!(
        receipt.wake_skipped,
        Some(WakeSkipped {
            session: "s-gone".to_string(),
            reason: WakeSkipReason::Unresolved,
            detail: None,
        }),
        "an orphaned return address is reported as `unresolved`, not as a trigger failure"
    );
    assert_eq!(worker.count(), 0, "nothing may be triggered");
}

#[test]
fn a_return_address_that_names_no_role_session_reports_nothing_to_skip() {
    // The companion guard, and the whole reason the finding has to exist at all. A plain human or
    // ad-hoc post — no return address, or one that was never minted as a role session — is not a
    // wake that failed, it is a wake nobody was ever owed. It stays `resumed: false`, `woke: None`,
    // `wake_skipped: None`, which must remain distinguishable from the two tests above; if this
    // path reported a finding too, the field's presence would mean nothing.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let worker = FailingWorker;

    for addr in [None, Some("s-never-registered")] {
        let posted = seed_addressed_post(&mut store, addr);
        let receipt = orchestration::reply(
            &ctx(&defs, &worker, "coder"),
            &mut store,
            reply_req(&posted, "just a reply"),
        )
        .unwrap_or_else(|e| panic!("an ordinary reply must succeed (addr {addr:?}): {e:?}"));

        assert!(!receipt.resumed, "addr {addr:?}");
        assert_eq!(receipt.woke, None, "addr {addr:?}");
        assert_eq!(
            receipt.wake_skipped, None,
            "there was nothing to wake, so there is nothing to report (addr {addr:?})"
        );
    }
}

#[test]
fn a_reply_into_a_quorum_thread_skips_the_direct_resume_and_defers_to_completion() {
    // The 6j6v.zebd correction: replying to the ask's own ROOT MESSAGE id (which every review-role
    // YAML sanctions, and which carries the same return address `ask` stamped) must not resume the
    // opener on the FIRST reviewer's reply with that reviewer's raw text — it defers entirely to
    // the completion block below.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let worker = RecordingWorker::default();

    store.create_pending_session("s-pm", "pm").unwrap();
    let tid = seed_ask(
        &mut store,
        "c-1",
        "pm",
        Some("s-pm"),
        &["local/bob".to_string(), "local/carol".to_string()],
        "please review",
    );
    let root_id = store.messages_in_thread(&tid).unwrap()[0]
        .message_id
        .clone();

    let receipt = orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&root_id, "looks fine"),
    )
    .unwrap();

    assert_eq!(receipt.woke, None, "1 of 2 — nothing may be resumed yet");
    assert_eq!(worker.count(), 0);
}

#[test]
fn a_reply_stamps_the_ambient_session_as_its_own_return_address() {
    // Without this, a multi-hop resume chain only survives as long as every intermediate reply
    // passes its own session by hand — forgetting it kills the chain with no error.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![], vec![]).unwrap();
    let worker = RecordingWorker::default();

    store.set_channel_field("c-1", "kind", "group", "local/pm");
    let posted = facade::send(
        &mut store,
        facade::SendRequest {
            now: NOW,
            origin: "local",
            actor: "pm",
            channel: "c-1",
            body: "a plain post",
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: None,
            refs: Refs::default(),
        },
    )
    .unwrap();

    let receipt = orchestration::reply(
        &ctx_in_session(&defs, &worker, "coder", Some("s-coder"), 0),
        &mut store,
        reply_req(&posted.message_id, "on it"),
    )
    .unwrap();

    assert_eq!(
        store
            .message_return_address(receipt.message_id.as_deref().unwrap())
            .unwrap()
            .as_deref(),
        Some("s-coder")
    );
}

#[test]
fn reply_rejects_a_hop_past_the_cap_before_persisting() {
    // The depth guard, checked before any work: a pair of roles that only ever bounce `reply` back
    // and forth cannot loop forever, and the refusal must not leave a message behind either.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![], vec![]).unwrap();
    let worker = RecordingWorker::default();

    store.set_channel_field("c-1", "kind", "group", "local/pm");
    let posted = facade::send(
        &mut store,
        facade::SendRequest {
            now: NOW,
            origin: "local",
            actor: "pm",
            channel: "c-1",
            body: "a plain post",
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: None,
            refs: Refs::default(),
        },
    )
    .unwrap();
    let before = message_count(&store);

    let err = orchestration::reply(
        &ctx_in_session(&defs, &worker, "coder", None, MAX_HOP + 1),
        &mut store,
        reply_req(&posted.message_id, "and again"),
    )
    .map(|_| ())
    .unwrap_err();

    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(err.msg.contains("depth guard"), "{}", err.msg);
    assert_eq!(message_count(&store), before, "nothing may be persisted");
}

// ---- declared-channel completion routing ------------------------------------------------------

fn summarize_defs() -> Definitions {
    Definitions::new(
        vec![simple_role("pm"), simple_role("bob")],
        vec![channel_decl(
            "name: review\nmembers: [bob]\non_complete: summarize\n\
             summary_prompt: Summarize the discussion.\n",
        )],
    )
    .unwrap()
}

#[test]
fn reply_completing_a_declared_summarize_channel_triggers_the_synthesizer() {
    // Phase 1: a real quorum completed, so `expects` is re-targeted to the QUALIFIED synthesis
    // identity and an ephemeral synthesizer is spawned. The real requester is NOT woken yet — there
    // is nothing to deliver until the synthesizer replies.
    let (_tmp, mut store) = fresh_store();
    let defs = summarize_defs();
    let worker = RecordingWorker::default();

    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "real-pm").unwrap();
    let tid = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-pm"),
        &["local/bob".to_string()],
        "please review",
    );

    let receipt = orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&tid, "lgtm"),
    )
    .unwrap();

    let completed = receipt
        .completed
        .expect("a declared channel's completion is routed, and the receipt says so");
    assert!(
        completed.delivered.contains("synthesizer spawned"),
        "{completed:?}"
    );
    assert_eq!(
        receipt.woke, None,
        "the requester waits for the synthesis; it is not woken in phase 1"
    );

    let t = worker.only();
    assert_eq!(t.role.handle, SYNTHESIS_HANDLE);
    assert_eq!(
        t.resume_real, None,
        "the synthesizer is always a fresh session"
    );
    assert!(
        t.role.system_prompt.contains("Summarize the discussion."),
        "the channel's own summary_prompt drives it: {}",
        t.role.system_prompt
    );
    assert_ne!(
        t.internal_session, "s-pm",
        "the synthesizer runs on its own session, not the requester's"
    );
    assert_eq!(
        store.session_role(&t.internal_session).unwrap().as_deref(),
        Some(SYNTHESIS_HANDLE),
        "the synthesizer's session is registered, or its own `session bind` callback would fail"
    );
    assert_eq!(
        quorum_expects(&store, &tid),
        vec![format!("local/{SYNTHESIS_HANDLE}")],
        "re-targeted to the QUALIFIED synthesis identity"
    );
}

#[test]
fn the_synthesizers_own_reply_delivers_the_summary_to_the_requester() {
    // Phase 2: the synth-retarget marker holds, so the synthesizer's own reply is recognized as
    // completing (first message in the thread or not) and its body is delivered to the requester.
    let (_tmp, mut store) = fresh_store();
    let defs = summarize_defs();
    let worker = RecordingWorker::default();

    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "real-pm").unwrap();
    let tid = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-pm"),
        &["local/bob".to_string()],
        "please review",
    );
    orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&tid, "lgtm"),
    )
    .unwrap();

    let receipt = orchestration::reply(
        &ctx(&defs, &worker, SYNTHESIS_HANDLE),
        &mut store,
        reply_req(&tid, "final summary text"),
    )
    .unwrap();

    let completed = receipt.completed.expect("phase 2 routes too");
    assert_eq!(completed.delivered, "final summary text");
    assert_eq!(receipt.woke.as_deref(), Some("s-pm"));
    assert_eq!(worker.handles(), vec![SYNTHESIS_HANDLE, "pm"]);
    let t = worker.last();
    assert_eq!(t.internal_session, "s-pm");
    assert_eq!(t.resume_real.as_deref(), Some("real-pm"));
    assert_eq!(t.message, "final summary text");
}

#[test]
fn reply_completing_a_pass_through_channel_delivers_to_the_requester() {
    // `pass_through` is single-phase: the requester is woken NOW with the raw collected replies,
    // and the thread is immediately self-satisfied against the delivered marker so a later tick
    // cannot deliver the same wake twice.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(
        vec![simple_role("pm"), simple_role("bob")],
        vec![channel_decl(
            "name: review\nmembers: [bob]\non_complete: pass_through\n",
        )],
    )
    .unwrap();
    let worker = RecordingWorker::default();

    store.create_pending_session("s-pm", "pm").unwrap();
    store.bind_session("s-pm", "real-pm").unwrap();
    let tid = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-pm"),
        &["local/bob".to_string()],
        "please review",
    );

    let receipt = orchestration::reply(
        &ctx(&defs, &worker, "bob"),
        &mut store,
        reply_req(&tid, "looks fine"),
    )
    .unwrap();

    let completed = receipt
        .completed
        .expect("a declared channel's completion is routed");
    assert!(
        completed.delivered.contains("looks fine"),
        "the raw collected replies: {completed:?}"
    );
    assert_eq!(receipt.woke.as_deref(), Some("s-pm"));

    let t = worker.only();
    assert_eq!(t.role.handle, "pm");
    assert_eq!(t.internal_session, "s-pm");
    assert_eq!(t.resume_real.as_deref(), Some("real-pm"));
    assert!(t.message.contains("looks fine"), "{}", t.message);

    let q = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert_eq!(
        q.expects,
        vec![format!("local/{DELIVERED_HANDLE}")],
        "retargeted to the delivered marker so a second call is a no-op"
    );
    assert!(
        q.complete,
        "and self-satisfied synchronously, so the board stays discoverable: {q:?}"
    );
}

// ---- the working-tree release (nxf 6j6v.fe0f) --------------------------------------------------
//
// The counter-movement to 6j6v.303b's acquire: the reply that discharges a scope's LAST outstanding
// expectation gives the working copy back and starts whoever was waiting for it — same process, no
// daemon, no waiter. What is under test here is a FACT ("nothing in this scope owes a reply any
// more"), never an estimate of quiet, so every test states the register it is deciding on.

/// A role that needs the working copy to itself.
fn exclusive_role(handle: &str) -> RoleDecl {
    role(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\nworking_tree: exclusive\n"
    ))
}

fn trigger_into_thread(
    ctx: &Ctx,
    store: &mut ChatStore,
    role: &str,
    thread: &str,
    body: &str,
) -> orchestration::TriggerReceipt {
    orchestration::coordinator_commission(
        ctx,
        store,
        orchestration::Commission {
            machine: None,
            role,
            body,
            channel: None,
            kind: MessageKind::Task,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: Some(thread),
            refs: Refs::default(),
            model: None,
            continue_session: None,
        },
    )
    .expect("the trigger verb succeeds whether it spawned or queued")
}

fn holder(store: &ChatStore) -> Option<String> {
    store.working_tree_holder(NOW).unwrap()
}

#[test]
fn a_reply_that_discharges_the_last_expectation_releases_the_tree_and_starts_the_waiting_chain() {
    // Definition of done #1, end to end through the real verbs: chain A holds the working copy,
    // chain B is parked behind it, and the reply that answers A's only outstanding expectation both
    // frees the lease AND starts B — in the same process that posted the reply.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![exclusive_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();

    let first = trigger_into_thread(
        &ctx(&defs, &worker, "pm"),
        &mut store,
        "coder",
        "th-a",
        "build the website",
    );
    let second = trigger_into_thread(
        &ctx(&defs, &worker, "pm"),
        &mut store,
        "coder",
        "th-b",
        "change the button label",
    );
    assert_eq!(
        first.queue_position, None,
        "the first chain takes the lease"
    );
    assert_eq!(second.queue_position, Some(1), "the second one waits");
    assert_eq!(worker.count(), 1, "only the first chain is running");

    let receipt = orchestration::reply(
        &ctx(&defs, &worker, "coder"),
        &mut store,
        reply_req("th-a", "done"),
    )
    .unwrap();

    assert!(receipt.posted);
    assert_eq!(
        worker.count(),
        2,
        "the waiting chain was started by the very reply that freed the tree"
    );
    let promoted = worker.last();
    assert_eq!(promoted.internal_session, second.session);
    // The plumbing line is the coordinator's ADDRESSEE BLOCK since nxf 6j6v.dq59, so this asserts
    // the body followed by a block that names `th-b` rather than one fixed sentence.
    assert!(
        promoted.message.starts_with("change the button label\n\n")
            && promoted.message.contains("nxc reply --thread th-b"),
        "a trigger fired from the queue is started with what a trigger that never waited would \
         have been started with — the caller's words PLUS the coordinator's plumbing line (nxf \
         6j6v.ntp9). It used to depend on which entrance queued it: `send --to` composed that line \
         one level up and the crate verb did not, so the same commission told the persona which \
         thread to answer into or did not, according to who had posted it. Got: {}",
        promoted.message
    );
    assert_eq!(
        promoted.reply_thread.as_deref(),
        Some("th-b"),
        "a trigger fired from the queue carries the SAME obligation it would have had immediately \
         — re-derived from the register, never frozen at enqueue time (nxf 6j6v.fe0f, item c)"
    );
    assert_eq!(
        holder(&store),
        Some("thread:th-b".to_string()),
        "the promoted chain now holds the working copy — granted by the release's own transaction \
         and then INHERITED by its trigger, rather than won on an acquire against a copy that was \
         briefly free (nxf 6j6v.yd4w)"
    );
    assert_eq!(store.working_tree_queue_len().unwrap(), 0);
}

#[test]
fn a_failure_starting_the_promoted_trigger_leaves_the_reply_standing() {
    // "Skip, don't fail", one link further down the same chain every wake site in this module is on
    // (nxf 6j6v.fe0f DoD #3): the reply is persisted and durable by the time the promotion runs, so
    // a worker that refuses the spawn is a breadcrumb, never an `Err` for a message that WAS
    // written. And the lease does not fall back to the chain that just gave it up.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![exclusive_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();

    trigger_into_thread(
        &ctx(&defs, &worker, "pm"),
        &mut store,
        "coder",
        "th-a",
        "build the website",
    );
    trigger_into_thread(
        &ctx(&defs, &worker, "pm"),
        &mut store,
        "coder",
        "th-b",
        "change the button label",
    );

    // The reply runs against a worker that refuses everything.
    let failing = FailingWorker;
    let receipt = orchestration::reply(
        &ctx(&defs, &failing, "coder"),
        &mut store,
        reply_req("th-a", "done"),
    )
    .unwrap();

    assert!(receipt.posted, "the reply itself is unaffected");
    assert_eq!(
        persisted_body(&store, receipt.message_id.as_deref().unwrap()).as_deref(),
        Some("done")
    );
    assert_ne!(
        holder(&store),
        Some("thread:th-a".to_string()),
        "the released lease is NOT handed back to the chain that gave it up"
    );
    assert_eq!(
        holder(&store),
        None,
        "and it is not left standing with the chain that never started either (nxf 6j6v.yd4w). \
         This used to assert `thread:th-b` — the promoted chain held the copy because its trigger \
         had acquired before the spawn was refused, and only the two-hour bound got it back. The \
         hand-off grants the lease now, so nothing-started is a case it can see: with nobody left \
         in the queue, handing it on means letting it go"
    );
    assert_eq!(store.working_tree_queue_len().unwrap(), 0);
}

/// A four-member `review` channel that fans out, whose members declare nothing about the working
/// copy, plus the unrelated `coder` chain that can be holding the lease when it opens.
///
/// It used to be introduced as "the shipped v3 example's own shape", and that is no longer true:
/// the shipped quorum is `flow: sequential` since nxf 6j6v.2d00, because its members all build in
/// one checkout. This shape is still very much reachable — a quorum whose members only READ wants
/// exactly it — and what it exercises here is the LEASE, which does not care about the flow.
fn four_member_review_defs() -> Definitions {
    Definitions::new(
        vec![
            exclusive_role("coder"),
            simple_role("general"),
            simple_role("code-quality"),
            simple_role("test-quality"),
            simple_role("integrity"),
        ],
        vec![channel_decl(
            "name: review\nmembers: [general, code-quality, test-quality, integrity]\n\
             on_complete: pass_through\nworking_tree: exclusive\n",
        )],
    )
    .unwrap()
}

/// Open the four-member board through the real fan-out, and return its thread id.
///
/// Through `send_to` since 6j6v.dvyq §3 removed `orchestration::ask`: the same mechanism
/// (`open_declared_channel_and_fan_out`), reached by naming the channel as a target.
fn open_review_board(ctx: &Ctx, store: &mut ChatStore) -> String {
    nexus_chat::surface::send_to(
        ctx,
        store,
        nexus_chat::surface::SendToRequest {
            machine: None,
            to: "review",
            body: "review PR #336",
            refs: nexus_chat::surface::SendToRefs::ExplicitlyNone,
        },
    )
    .expect("the board opens whether its members spawned or queued")
    .thread_id
}

/// The four members, sorted — what "all of them ran" looks like whatever order they fired in.
fn sorted(mut handles: Vec<String>) -> Vec<String> {
    handles.sort();
    handles
}

#[test]
fn every_member_of_an_exclusive_board_runs_even_when_the_board_itself_had_to_wait_first() {
    // PR #336 review, Code Quality #1 (CRITICAL) — the deadlock this branch shipped. The fan-out
    // triggers each member independently and they all resolve to the SAME claim area (the board's
    // thread), so when the board's own first acquire loses to an unrelated holder, EVERY member
    // enqueues its own row under that one `scope_key`. A release that pops a single row then starts
    // one member and strands the other three — and they can never be started later, because the
    // board owes THEIR replies, so its own scope never reports "nothing outstanding" and never
    // releases. The only recovery was the two-hour backstop, and even that drains one entry per
    // unrelated release.
    //
    // The fix is that a release drains the head's whole claim area. That is not new behaviour: in
    // the uncontended path these same members already run together (the first acquires, the rest
    // inherit) — firing them together on release is the SAME behaviour, arriving later.
    let (_tmp, mut store) = fresh_store();
    let defs = four_member_review_defs();
    let worker = RecordingWorker::default();

    // An unrelated chain has the working copy, and still owes a reply.
    trigger_into_thread(
        &ctx(&defs, &worker, "pm"),
        &mut store,
        "coder",
        "th-code",
        "build the website",
    );
    assert_eq!(holder(&store), Some("thread:th-code".to_string()));

    let board = open_review_board(&ctx(&defs, &worker, "pm"), &mut store);

    assert_eq!(
        worker.handles(),
        vec!["coder".to_string()],
        "not one member of the board started — the working copy was taken"
    );
    assert_eq!(
        store.working_tree_queue_len().unwrap(),
        4,
        "all four are parked, each with its own row under the board's single claim area"
    );
    assert!(
        store
            .list_working_tree_queue()
            .unwrap()
            .iter()
            .all(|q| q.scope_key == format!("thread:{board}")),
        "precondition: they really do share one scope key — that is what makes this a deadlock"
    );

    // The holder finishes. Its release is the ONLY thing that can ever start them.
    orchestration::reply(
        &ctx(&defs, &worker, "coder"),
        &mut store,
        reply_req("th-code", "done"),
    )
    .unwrap();

    assert_eq!(
        sorted(worker.handles()),
        vec![
            "code-quality".to_string(),
            "coder".to_string(),
            "general".to_string(),
            "integrity".to_string(),
            "test-quality".to_string(),
        ],
        "EVERY member ran, not just the one that happened to be at the head"
    );
    assert_eq!(
        store.working_tree_queue_len().unwrap(),
        0,
        "and nothing is left parked behind a lease its own board holds"
    );
    assert_eq!(
        holder(&store),
        Some(format!("thread:{board}")),
        "the board now holds the working copy: the first member acquired, the rest inherited"
    );
}

#[test]
fn one_member_that_refuses_to_start_does_not_strand_the_siblings_promoted_with_it() {
    // The sibling half of the same fix, and the same "skip, don't fail" rule every wake site in
    // this module follows: the reply is persisted and durable by the time the promotion runs, so a
    // worker that refuses ONE promoted member is a breadcrumb — never an `Err` for a message that
    // was written, and never a reason to abandon the members promoted beside it.
    let (_tmp, mut store) = fresh_store();
    let defs = four_member_review_defs();
    let refusing = RefusingWorker::refusing("general");

    trigger_into_thread(
        &ctx(&defs, &refusing, "pm"),
        &mut store,
        "coder",
        "th-code",
        "build the website",
    );
    open_review_board(&ctx(&defs, &refusing, "pm"), &mut store);
    assert_eq!(store.working_tree_queue_len().unwrap(), 4);

    let receipt = orchestration::reply(
        &ctx(&defs, &refusing, "coder"),
        &mut store,
        reply_req("th-code", "done"),
    )
    .unwrap();

    assert!(receipt.posted, "the reply itself is unaffected");
    assert_eq!(
        sorted(refusing.handles()),
        vec![
            "code-quality".to_string(),
            "coder".to_string(),
            "general".to_string(),
            "integrity".to_string(),
            "test-quality".to_string(),
        ],
        "the refused member was attempted, and its three siblings were still started after it"
    );
    assert_eq!(store.working_tree_queue_len().unwrap(), 0);
}

#[test]
fn a_resume_holds_a_scope_that_owes_nothing_and_only_a_reply_in_that_scope_frees_it() {
    // The deliberate decision this ticket had to make (PR review of nxf 6j6v.303b, item b), pinned
    // so nobody changes it by accident. `role_resume` takes a thread-scoped lease while
    // registering NO obligation, and the thread may already be discharged — so the scope owes
    // nothing from the moment it is taken. The rule reads that for what it is and frees the lease
    // at the next reply INTO THAT THREAD, rather than holding this device's working copy for two
    // hours over a signal that will never come. The in-scope gate is what keeps it narrow: a reply
    // in somebody else's conversation decides nothing about this lease.
    let (_tmp, mut store) = fresh_store();
    let defs = Definitions::new(vec![exclusive_role("coder")], vec![]).unwrap();
    let worker = RecordingWorker::default();
    store.create_pending_session("s-coder", "coder").unwrap();

    orchestration::role_resume(
        &ctx(&defs, &worker, "pm"),
        &mut store,
        RoleResumeRequest {
            session: "s-coder",
            body: "one more thing",
            channel: None,
            kind: MessageKind::Task,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: Some("th-resume"),
            refs: Refs::default(),
            model: None,
        },
    )
    .unwrap();
    assert_eq!(
        holder(&store),
        Some("thread:th-resume".to_string()),
        "a resume still BELONGS to the thread it pushed into, and competes under it"
    );

    // A reply in a completely different conversation: not this chain's business.
    seed_thread_message(&mut store, "th-elsewhere");
    orchestration::reply(
        &ctx(&defs, &worker, "someone"),
        &mut store,
        reply_req("th-elsewhere", "unrelated"),
    )
    .unwrap();
    assert_eq!(
        holder(&store),
        Some("thread:th-resume".to_string()),
        "a stranger's reply must not free a lease it has nothing to do with"
    );

    // A reply in the chain's OWN thread, where nothing is outstanding, does free it.
    orchestration::reply(
        &ctx(&defs, &worker, "pm"),
        &mut store,
        reply_req("th-resume", "and that is that"),
    )
    .unwrap();
    assert_eq!(
        holder(&store),
        None,
        "nothing in the scope owes a reply, so the working copy goes back"
    );
}

/// A root post in `thread`, so `reply` can resolve the thread to a channel at all.
fn seed_thread_message(store: &mut ChatStore, thread: &str) {
    store.set_channel_field("c-1", "kind", "group", "local/pm");
    facade::send(
        store,
        facade::SendRequest {
            now: NOW,
            origin: "local",
            actor: "pm",
            channel: "c-1",
            body: "the task",
            kind: MessageKind::Task,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: Some(thread),
            refs: Refs::default(),
        },
    )
    .expect("the seed post succeeds");
}
