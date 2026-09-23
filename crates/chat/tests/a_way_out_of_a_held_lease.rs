//! **A held working copy has a way out** — once by a manual verb this file drove through the
//! LIBRARY HANDLE (nxf 6j6v.fabb), now by the sweep the background service runs unattended, since
//! `nxc release` and `Engine::release_working_tree` left the user surface AND the seam alike (nxf
//! 6j6v.b9nf: offered on the handle, the verb could take a working copy away from a running coding
//! operation, and on a worker that never implemented its liveness check it was an unconditional
//! release with no guard at all).
//!
//! The scene, measured in the proving ground 4jgn.g90w on nxs 0.63.0: an escalation held the
//! working copy — correctly; the task was mid-flight — and then nothing ever gave it back. Ten
//! threads, every one `complete: true` with `outstanding: []`, two of them `escalated: true`, no
//! process anywhere. A merging round commissioned afterwards sat on `working tree: waiting (#1)`
//! with no session, no process and no end. The documented way out (`nxc guide limits-and-safety`:
//! "answering a hand-back re-declares the round, and the hold is gone") was walked twice —
//! `posted: true` both times, `woke: null` both times — and the lease read `holding` afterwards.
//!
//! **What this file holds now** is the ACQUIRE-end guard — a rival does not steal an expired lease
//! from a chain whose process is still writing — and the PROMOTION mechanics of a hand-off: a
//! refused promotion is named rather than lost, hands the copy to the next area in line, or is
//! reported back-in-line rather than started. All four are driven through the tick's sweep past the
//! bound (this file's own `tick` helper, a raw call onto [`orchestration::tick`] with a worker built
//! for the SAME reason `working_tree_two_process_e2e.rs`'s own siblings give: the guard asks
//! [`Worker::session_is_running`], and the only seam where a test can supply that answer is
//! [`WorkerConfig::Custom`] on the handle, the same shape a host worker driving a real runtime has)
//! rather than through the departed verb, because [`orchestration::release_and_fire`]'s promotion
//! loop is the same code under every movement that reaches it — the reply path, a park, or the
//! sweep. What the removed verb's OWN refusal used to pin (a live session in the holding chain, a
//! thread that must be inside it, a caller that must be able to read it) had no successor to
//! inherit, because there is no longer a manual call to refuse: see the removal notes below for
//! each test that went with it, and `tests/seam_disposition.rs`'s row for the decision and removal
//! dates.
//!
//! **Why the guard is the whole design**, unchanged by any of this. Epic 6j6v.bqe0 refused to build
//! a manual override ("KEIN NEUES MUTATIONSVERB") and was right to: an escape hatch reached for
//! casually is how a deterministic done signal stops being maintained. `sessions_still_running`
//! refuses every hand-off in this crate while anything in the chain is still alive, and the failure
//! it would otherwise cause (two agents in one checkout) is exactly what the epic exists to
//! prevent — automatically now, everywhere this crate hands a working copy on, rather than only on
//! a verb a caller remembered to reach for.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::{self, Caller, TickReceipt, TickRequest};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{SendToRefs, SendToRequest};
use nexus_chat::timer::{DryTimer, TimerConfig};
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-08-24T21:30:48Z";
/// One second past `NOW` + `WORKING_TREE_LEASE_BOUND` ("2h").
const PAST_THE_BOUND: &str = "2026-08-24T23:30:49Z";

/// A worker that records every trigger and answers the one read the release guard makes of it.
/// `a_step_ends_when_its_session_ends.rs`'s `LiveSessionWorker` in miniature, and for its reason:
/// a real host owns the processes, so it is the only thing that can say which are still alive.
#[derive(Default)]
struct LiveSessionWorker {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
    /// Roles this worker refuses to start — how a promotion lands in
    /// [`Promotions::failed`](nexus_chat::orchestration::Promotions) rather than `started`.
    refuse: Mutex<HashSet<String>>,
}

impl LiveSessionWorker {
    fn handles(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.role.handle.clone())
            .collect()
    }
    fn only_for(&self, handle: &str) -> TriggerRequest {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.role.handle == handle)
            .unwrap_or_else(|| panic!("no trigger for {handle}, saw {:?}", self.handles()))
            .clone()
    }
    fn mark_running(&self, session: &str) {
        self.running.lock().unwrap().insert(session.to_string());
    }
    fn mark_gone(&self, session: &str) {
        self.running.lock().unwrap().remove(session);
    }
    fn refuse_to_start(&self, role: &str) {
        self.refuse.lock().unwrap().insert(role.to_string());
    }
}

impl Worker for LiveSessionWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        if self.refuse.lock().unwrap().contains(&req.role.handle) {
            return Err(nexus_chat::worker::TriggerError::Failed(
                nexus_chat::error::NxfError::io("stub: this worker refuses to start that role"),
            ));
        }
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }
    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running.lock().unwrap().contains(internal_session)
    }
}

fn role(handle: &str, exclusive: bool) -> RoleDecl {
    let claim = if exclusive {
        "working_tree: exclusive\n"
    } else {
        ""
    };
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n{claim}"
    ))
    .expect("test role parses")
}

fn channel(yaml: &str) -> ChannelDecl {
    serde_yaml::from_str(yaml).expect("test channel parses")
}

/// `coder` claims the working copy for itself; `review` is a board that claims it for the round.
/// Two DIFFERENT claim areas, which is what makes one of them queue behind the other.
fn team() -> (TempDir, Engine, Arc<LiveSessionWorker>) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(
        vec![role("coder", true), role("reviewer", false)],
        vec![channel(
            "name: review\nmembers: [reviewer]\nworking_tree: exclusive\n",
        )],
    )
    .expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let worker = Arc::new(LiveSessionWorker::default());
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker.clone()),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    (tmp, engine, worker)
}

fn caller<'a>(actor: &'a str, now: &'a str) -> Caller<'a> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(now),
    }
}

/// **The tick the background service runs** — through the compute layer, because since nxf
/// 6j6v.b9nf it is the only way to reach a working copy's lease this file's `nxc release` used to
/// name by hand. `thread_id` only picks which board's OWN completion this tick also re-checks; the
/// working-tree sweep at its front runs unconditionally, whichever thread is named, so any thread
/// this workspace knows does the job.
fn tick(tmp: &TempDir, worker: &LiveSessionWorker, now: &str, thread_id: &str) -> TickReceipt {
    let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
    let db_path = ws.db_path_str().expect("db path");
    let mut store = ws.open_chat_store().expect("open chat store");
    let defs = Definitions::resolve(tmp.path()).expect("declarations");
    let ctx = orchestration::Ctx {
        now,
        origin: "local",
        actor: "pm",
        session: None,
        hop: 0,
        defs: &defs,
        worker,
        timer: &DryTimer,
        namer: &nexus_chat::naming::DryNamer,
        db_path: &db_path,
        project_claude_md: None,
        module_primes: None,
        machines: None,
    };
    orchestration::tick(&ctx, &mut store, TickRequest { thread_id }).expect("tick")
}

/// Commission `to` and hand back the thread the caller was given.
fn send_to(engine: &Engine, to: &str, now: &str) -> String {
    engine
        .send_to(
            caller("pm", now),
            SendToRequest {
                machine: None,
                to,
                body: "go",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the commission is persisted")
        .thread_id
}

/// The holder, the thread it holds under, and the rival parked behind it.
fn a_held_lease_with_somebody_waiting(
    engine: &Engine,
    worker: &LiveSessionWorker,
) -> (String, String, String) {
    let holder_thread = send_to(engine, "coder", NOW);
    let holder_session = worker.only_for("coder").internal_session;
    let parked_thread = send_to(engine, "review", NOW);
    assert_eq!(
        worker.handles(),
        vec!["coder".to_string()],
        "the rival is PARKED, not started — nothing was handed to the worker for it"
    );
    (holder_thread, holder_session, parked_thread)
}

// ---- the ACQUIRE end of the same guard (nxf 6j6v.8y6t) ----------------------------------------

#[test]
fn a_rival_does_not_take_an_expired_lease_from_a_chain_whose_process_is_still_writing() {
    // **The clock alone used to be enough to take the checkout, and that was the one failure this
    // whole epic exists to prevent** — `WORKING_TREE_LEASE_BOUND`'s own doc names it as the residual
    // it could not close: a step that WORKS (not idles) past the bound loses the lease under itself
    // and the next chain walks into the checkout it is still building in.
    //
    // It became load-bearing with nxf 6j6v.8y6t, because that item lets the bound be DERIVED from
    // the operation's own declared windows — which can be minutes. So the acquire end now asks the
    // question the release end (above) and the sweep have always asked: is anything in the holding
    // chain still running? A rival meets a process, not a deadline.
    //
    // This is also the answer to the owner's second question — what happens when a step hangs
    // without tripping its timeout. The wall clock was called the net for that case; all it actually
    // did was hand the checkout to somebody else while the hung step wrote into it.
    let (_tmp, engine, worker) = team();
    let (_holder_thread, holder_session, _parked) =
        a_held_lease_with_somebody_waiting(&engine, &worker);
    worker.mark_running(&holder_session);

    // Hours later — the lease has run out, and the coder is still writing.
    let late = send_to(&engine, "review", PAST_THE_BOUND);
    assert!(!late.is_empty());
    assert_eq!(
        worker.handles(),
        vec!["coder".to_string()],
        "the expired lease was NOT taken over: nothing new was handed to the worker — {:?}",
        worker.handles()
    );

    // …and the moment that process is gone, the ORIGINALLY PARKED commission is what starts.
    //
    // **That it is the parked one, and not merely a fresh attempt succeeding, is the assertion**
    // (review of PR #376, Test Quality #4). A second `send_to` here would have gone green on a
    // different mechanism — a new trigger winning a free lease — and said nothing about the entry
    // that has been waiting since the top of this test. The sweep is what drains the queue now (`nxc
    // release` is gone, nxf 6j6v.b9nf), so the sweep is what is asked — through the tick, the way the
    // background service reaches it — and the receipt names who it promoted.
    worker.mark_gone(&holder_session);
    let receipt = tick(&_tmp, &worker, PAST_THE_BOUND, &_holder_thread);
    let swept = receipt
        .working_tree
        .as_ref()
        .expect("the lease was past its bound with nothing running: the sweep must report it");
    assert_eq!(
        swept.promotions.started.len(),
        1,
        "exactly the one commission that was parked — {swept:?}"
    );
    assert_eq!(swept.promotions.started[0].role, "reviewer");
    assert_eq!(swept.promotions.failed, Vec::new());
    assert_eq!(
        worker.handles(),
        vec!["coder".to_string(), "reviewer".to_string()],
        "and the worker really was handed it, in that order"
    );
}

// `a_release_is_refused_while_a_session_in_the_holding_chain_is_still_alive`,
// `the_same_release_succeeds_the_moment_that_session_is_gone`,
// `releasing_names_a_thread_inside_the_holding_chain_or_it_is_refused`,
// `a_caller_who_cannot_read_the_thread_cannot_release_its_checkout` and
// `releasing_when_nothing_holds_the_working_copy_says_so` were here. REMOVED by nxf 6j6v.b9nf: each
// one's subject IS `Engine::release_working_tree` itself — its liveness refusal and the identical
// release that succeeds once the session is gone, its own thread-must-be-inside-the-chain check,
// its own read-gate ("whoever may READ the board may hand its checkout on"), and its own
// nothing-holds-the-copy `not_found`. The method is gone, so there is no seam left for any of these
// five to drive. What each one guarded is either structural to a DIFFERENT verb now — `withdraw`
// has its own read gate and its own `not_found` for nothing waiting or running, held by
// `crates/chat/tests/withdraw_a_parked_commission.rs` and `withdraw_a_running_round.rs` — or was
// never a general park/reclaim property to begin with (this method's own bespoke liveness-refusal
// wording and thread-must-be-inside-the-holding-chain check had no counterpart to inherit; the
// GUARD they were built on, `sessions_still_running`, is exercised by the rewritten test above and
// by `crates/chat/tests/a_holder_past_its_bound_is_parked.rs`).

/// **A promotion that could not be STARTED is reported, not swallowed** — the half
/// [`Promotions`](nexus_chat::orchestration::Promotions) exists for, and which nothing exercised
/// until the review of this branch asked (Test Quality #4).
///
/// The hand-off has already committed by the time a promoted trigger is fired, so a failure there
/// cannot fail the hand-off — refusing it afterwards would leave the working copy held by nobody
/// with the queue still parked behind it, which is the one shape this ticket exists to prevent. The
/// entry is off the queue and its session is minted, so a caller that was told nothing would have
/// no way to learn that nothing is running for it.
///
/// Driven through the sweep past the bound rather than through `nxc release` (gone, nxf 6j6v.b9nf):
/// [`orchestration::release_and_fire`]'s promotion loop is the SAME code either way — the hand-off
/// commits, then fires whoever it promoted — so what this test pins about a refused promotion does
/// not depend on which movement reached that loop.
#[test]
fn a_promotion_that_cannot_be_started_is_named_rather_than_lost() {
    let (tmp, engine, worker) = team();
    let (holder_thread, holder_session, parked_thread) =
        a_held_lease_with_somebody_waiting(&engine, &worker);
    worker.mark_gone(&holder_session);
    worker.refuse_to_start("reviewer");

    let receipt = tick(&tmp, &worker, PAST_THE_BOUND, &holder_thread);
    let swept = receipt
        .working_tree
        .as_ref()
        .expect("the lease was past its bound with nothing running: the sweep must report it");
    assert_eq!(swept.promotions.started, Vec::new(), "{swept:?}");
    assert_eq!(swept.promotions.failed.len(), 1, "{swept:?}");
    let failed = &swept.promotions.failed[0];
    assert_eq!(failed.role, "reviewer");
    assert!(
        !failed.session.is_empty(),
        "the minted session is named: {failed:?}"
    );
    // The commission's OWN slot thread below the board `send_to` handed back — that is what the
    // queue entry carries and what a reader has to look at, not the board itself.
    assert!(
        !failed.thread.is_empty()
            && failed.thread != holder_thread
            && failed.thread != parked_thread,
        "the thread whose message is persisted is named: {failed:?}"
    );

    // **And NOBODY holds the copy afterwards** (nxf 6j6v.yd4w). This assertion used to say the
    // opposite, with the reasoning written out: the hand-off gave the lease to the promoted area,
    // the worker then refused to start it, and the checkout stayed held by a chain that never ran —
    // "pre-existing and general", freed only when the sweep reached its bound. That is now the one
    // case the hand-off itself handles: an area where nothing started is an area with no process
    // behind it, so `release_and_fire` hands the working copy straight on instead of sitting on it.
    // Here there is nobody left in the queue, so handing it on means letting it go.
    //
    // What does NOT change is the half this test is named for: the entry is still off the queue, its
    // message still persisted and its session still minted, and `promotions.failed` above is still
    // the only place a caller learns that nothing is running for it.
    let store = Workspace::resolve(None, tmp.path())
        .expect("resolve")
        .open_chat_store()
        .expect("store");
    assert_eq!(
        store.working_tree_lease_row().expect("the lease row"),
        None,
        "a promotion nothing could start must not leave the checkout claimed by a chain that never \
         ran — the queue behind it would then wait out the whole backstop for nothing"
    );
}

/// **A promotion nothing could start hands the working copy ON, rather than sitting on it**
/// (nxf 6j6v.yd4w).
///
/// The hand-off gives the lease to the claim area it promotes, inside the same transaction that
/// takes that area off the queue — which is what closes the window a newcomer used to win. The
/// price is this case: if nothing in the promoted area actually starts, the checkout is held by an
/// area with no process behind it, and everything still queued behind it would wait out the whole
/// backstop for nothing. So the hand-off gives it to the next area instead, and that one starts.
///
/// Driven through the sweep past the bound (`nxc release` is gone, nxf 6j6v.b9nf) — same reason as
/// the test above: the loop this pins is `release_and_fire`'s, shared by every movement that reaches
/// it.
#[test]
fn a_refused_promotion_hands_the_working_copy_to_the_next_area_in_line() {
    let (tmp, engine, worker) = team();
    let holder_thread = send_to(&engine, "coder", NOW);
    let holder_session = worker.only_for("coder").internal_session;
    // First in line: the board whose promotion the worker will refuse.
    let refused_thread = send_to(&engine, "review", NOW);
    // Behind it, a second claim area — `send --to <persona>` mints a new thread per call, so this
    // is a different scope and therefore a different area.
    let next_thread = send_to(&engine, "coder", NOW);
    assert_eq!(
        worker.handles(),
        vec!["coder".to_string()],
        "both rivals are parked, not started"
    );

    worker.mark_gone(&holder_session);
    worker.refuse_to_start("reviewer");

    let receipt = tick(&tmp, &worker, PAST_THE_BOUND, &holder_thread);
    let swept = receipt
        .working_tree
        .as_ref()
        .expect("the lease was past its bound with nothing running: the sweep must report it");

    assert_eq!(
        swept.promotions.failed.len(),
        1,
        "the area that could not start is named: {swept:?}"
    );
    assert_eq!(swept.promotions.failed[0].role, "reviewer");
    assert_eq!(
        swept
            .promotions
            .started
            .iter()
            .map(|p| p.role.as_str())
            .collect::<Vec<_>>(),
        vec!["coder"],
        "…and the area behind it was handed the copy and started: {swept:?}"
    );
    assert_eq!(
        worker.handles(),
        vec!["coder".to_string(), "coder".to_string()],
        "the worker really was asked to start it"
    );

    let store = Workspace::resolve(None, tmp.path())
        .expect("resolve")
        .open_chat_store()
        .expect("store");
    assert_eq!(
        store.working_tree_holder(PAST_THE_BOUND).expect("holder"),
        Some(format!("thread:{next_thread}")),
        "the chain that started holds the checkout — not the one that could not"
    );
    assert!(
        refused_thread != next_thread,
        "the two rivals really were different claim areas"
    );
}

/// **A promoted entry that does NOT end up holding the copy is reported as back-in-line, never as
/// started** (independent review of PR #380, Integrity & Robustness #1).
///
/// The hand-off grants the lease to the area it promotes, so the ordinary fire is an inherit and
/// this case does not arise. It is not unreachable, though, and the review found a reachable route
/// through a concurrent `nxc release --thread <t>` naming a thread inside the freshly granted area
/// — NOT blocked by its own liveness guard, since nothing was running there yet, so it could hand
/// the copy on in the window between the hand-off's commit and the fire. That route closed with the
/// verb (nxf 6j6v.b9nf): `nxc withdraw` cannot reopen it, because by the time the grant has run the
/// entry is off the queue and nothing is running in the granted area, so a withdraw naming it finds
/// nothing to take back and is refused as `not_found` rather than stealing the copy.
///
/// **This test reaches the state by the OTHER route the same code names**, unaffected by any of
/// that: an entry whose thread no longer resolves to the key it was parked under. `lease_claim_root`
/// re-derives the ROOT at fire time, so a re-parenting — or, as here, an entry parked under a key its
/// thread never belonged to — makes the fired trigger a different claim area than the one the grant
/// named. `inherits_the_held_claim`'s own doc calls out that same shape from the acquire end ("a
/// re-parenting or a hand-built tree can still produce" it). What is under test either way is what
/// the RECEIPT says — driven here through the sweep past the bound rather than through the departed
/// verb, since `release_and_fire`'s loop is the same code under both.
#[test]
fn a_promotion_that_does_not_end_up_holding_the_copy_is_reported_as_back_in_line() {
    use nexus_chat::model::Priority;
    use nexus_chat::working_tree::QueuedTrigger;

    let (tmp, engine, worker) = team();
    let holder_thread = send_to(&engine, "coder", NOW);
    let holder_session = worker.only_for("coder").internal_session;
    // A real parked commission, so there is a genuine session and thread to fire.
    let _parked_board = send_to(&engine, "review", NOW);
    let parked = {
        let store = Workspace::resolve(None, tmp.path())
            .expect("resolve")
            .open_chat_store()
            .expect("store");
        store
            .list_working_tree_queue()
            .expect("the queue")
            .into_iter()
            .next()
            .expect("the rival is parked")
    };

    // Re-park it under a key its own thread does not resolve to. The grant will name THIS key; the
    // fire will re-derive the entry's real root and find the copy is not its area's.
    {
        let mut store = Workspace::resolve(None, tmp.path())
            .expect("resolve")
            .open_chat_store()
            .expect("store");
        assert!(store
            .remove_working_tree_queue_entry(parked.id)
            .expect("take the real entry back out"));
        store
            .enqueue_working_tree(
                &QueuedTrigger {
                    id: 0,
                    scope_key: "thread:th-a-key-its-thread-does-not-resolve-to".to_string(),
                    priority: Priority::Normal,
                    enqueued_at: None,
                    ..parked.clone()
                },
                NOW,
            )
            .expect("park it under the other key");
    }

    worker.mark_gone(&holder_session);
    let receipt = tick(&tmp, &worker, PAST_THE_BOUND, &holder_thread);
    let swept = receipt
        .working_tree
        .as_ref()
        .expect("the lease was past its bound with nothing running: the sweep must report it");

    assert_eq!(
        swept.promotions.started,
        Vec::new(),
        "nothing is running for it, so it must not be counted as started: {swept:?}"
    );
    assert_eq!(
        swept.promotions.failed,
        Vec::new(),
        "…and it is not lost either — `failed` means nobody will come back for it: {swept:?}"
    );
    assert_eq!(
        swept.promotions.requeued.len(),
        1,
        "it is back in line, and the receipt says exactly that: {swept:?}"
    );
    assert_eq!(swept.promotions.requeued[0].role, "reviewer");
    assert_eq!(swept.promotions.requeued[0].session, parked.session);

    // The board agrees: the entry really is queued again, so the next hand-off starts it.
    let store = Workspace::resolve(None, tmp.path())
        .expect("resolve")
        .open_chat_store()
        .expect("store");
    assert_eq!(
        store.working_tree_queue_len().expect("the queue"),
        1,
        "the entry went back on the queue rather than vanishing"
    );
    let head = store
        .peek_working_tree_queue()
        .expect("the head")
        .expect("an entry");
    assert_eq!(
        (head.session.as_str(), head.role.as_str()),
        (parked.session.as_str(), parked.role.as_str()),
        "and it is the same commission — the same minted session, not a second one"
    );
    assert_eq!(
        head.enqueued_at, parked.enqueued_at,
        "re-queued with the place it had already waited for, not stamped with the current instant"
    );
}

// `past_the_bound_a_release_reports_the_expiry_and_drains_the_queue` was here — REMOVED by nxf
// 6j6v.b9nf. Its subject was specifically the ON-DEMAND half `nxc release` alone provided: naming
// a dead chain's thread and draining its queue WITHOUT waiting for a tick to come along, and a
// receipt field (`expired: bool`) that said so. Both are gone with the verb — there is no longer a
// way to ask for that drain on demand, only the sweep's own, which runs unattended at the very
// clock this test's own comment named as already covering the other half
// (`working_tree_two_process_e2e.rs`'s `a_hard_killed_holder_loses_the_lease_past_the_bound_but_the
// _queue_does_not_self_drain` and `at_expiry_the_chain_already_waiting_goes_before_the_newcomer_
// that_reclaims`). The sweep itself, past the bound, with who it promotes and what the receipt
// says, is `crates/chat/tests/a_holder_past_its_bound_is_parked.rs`'s to hold — and the test above,
// `a_rival_does_not_take_an_expired_lease_from_a_chain_whose_process_is_still_writing`, already
// drives that same sweep through this file's own worker.
