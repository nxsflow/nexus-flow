//! The spawn-chain depth guard, anchored in the store instead of in the caller's own environment
//! (nxf 6j6v.m48m).
//!
//! The guard's counter used to be `NXC_HOP` — an environment variable in the process of a role that,
//! in every shipped example set, declares `tools: [Bash]`. `unset NXC_HOP` before its own `nxc send
//! --role` reset the chain to zero on every hop, and each hop is a real detached process plus a paid
//! LLM call, so what it defeated was a cost brake. The library seam widened the same hole rather than
//! closing it: `Ctx.hop` is a plain argument, and a caller that passes `0` every time gets the
//! identical effect without needing an environment at all.
//!
//! So the depth now lives in `session_map`, written by whoever SPAWNS a session, and the claimed hop
//! survives only as a floor that may raise it and never lower it. These tests drive the library seam
//! because that is where both halves are observable at once: the depth a trigger persists, and the
//! depth the next verb reads back.
//!
//! **The library seam stopped claiming a hop at all** (nxf 6j6v.07me): the `Caller.hop` field that
//! widened the hole is gone, so an app cannot present a shallower context even by accident. These
//! cases are written against `Ctx` — the shape BOTH adapters fill, and the one the CLI still puts
//! `NXC_HOP` into — so they keep testing the floor where it is still claimed. What the depth looks
//! like from the seam, with nothing passed, is `caller_seam.rs`.
//!
//! **What the counter measures is the OPEN chain** (nxf 6j6v.ka09): work commissioned and not yet
//! answered. It used to be every agent-to-agent hop a session ever took part in, which killed the
//! shape it was supposed to protect — an orchestrator and a channel owner talking over many rounds
//! gained four hops a round and went permanently uncallable after seven. The two shapes that look
//! alike to a raw counter are pinned here as a matched pair, and they are what the fix has to keep
//! apart: `two_roles_that_only_bounce_between_each_other_run_out_of_depth` (commissions only, never
//! answered — still hits the cap) and `a_bounded_dialogue_between_long_lived_roles_never_runs_out_of_depth`
//! (commissioned and answered — runs forever at a constant depth).

use std::sync::Mutex;

use nexus_chat::definitions::Definitions;
use nexus_chat::error::ErrorKind;
use nexus_chat::model::{Disposition, MessageKind, Priority, Refs};
use nexus_chat::orchestration::{self, Commission, Ctx, ReplyRequest, RoleResumeRequest, MAX_HOP};
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
    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    /// The `NXC_HOP` stamp the last trigger handed the session it spawned.
    fn last_hop_stamp(&self) -> String {
        self.seen
            .lock()
            .unwrap()
            .last()
            .expect("at least one trigger")
            .env
            .iter()
            .find(|(k, _)| k == "NXC_HOP")
            .map(|(_, v)| v.clone())
            .expect("every trigger stamps NXC_HOP")
    }
}

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> nexus_chat::worker::TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(nexus_chat::worker::TriggerOutcome::Accepted)
    }
}

/// A worker that refuses every spawn (mirrors the identical double in `orchestration_reply.rs` and
/// `orchestration_workflow.rs`) — the "no sidecar configured" shape.
struct FailingWorker;

impl Worker for FailingWorker {
    fn trigger(&self, _req: TriggerRequest) -> nexus_chat::worker::TriggerResult {
        Err(nexus_chat::error::NxfError::io("no sidecar configured").into())
    }
}

/// These suites never assert on SCHEDULING, and since the timer became an injected value
/// (PR #269 review, Code Quality #3) they no longer have to tolerate whatever `NXC_TIMER`
/// happens to be — unset, that used to mean a real `at` subprocess per armed deadline.
static NO_TIMER: nexus_chat::timer::DisabledTimer = nexus_chat::timer::DisabledTimer;

fn simple_role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"
    ))
    .expect("test role parses")
}

fn fresh_store() -> (TempDir, ChatStore) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
    let store = ws.open_chat_store().expect("open chat store");
    (tmp, store)
}

fn defs() -> Definitions {
    Definitions::new(vec![simple_role("pm"), simple_role("coder")], vec![]).unwrap()
}

/// The three-role catalogue the long-lived dialogue below runs on: an orchestrator that commissions
/// a channel owner, which commissions a worker.
fn orchestrator_defs() -> Definitions {
    Definitions::new(
        vec![
            simple_role("orchestrator"),
            simple_role("pm"),
            simple_role("coder"),
        ],
        vec![],
    )
    .unwrap()
}

/// A caller with an ambient `session` that CLAIMS to be at hop `hop`.
fn ctx<'a>(
    defs: &'a Definitions,
    worker: &'a dyn Worker,
    session: Option<&'a str>,
    hop: u32,
) -> Ctx<'a> {
    ctx_as(defs, worker, "alice", session, hop)
}

/// [`ctx`] with an explicit acting agent — what a chain of three DISTINCT roles needs, since the
/// auto-opened DM is derived from `origin/actor` and a role may not DM itself.
fn ctx_as<'a>(
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

fn trigger_req<'a>(role: &'a str, body: &'a str) -> Commission<'a> {
    Commission {
        machine: None,
        role,
        body,
        channel: None,
        kind: MessageKind::Task,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread: None,
        refs: Refs::default(),
        model: None,
        continue_session: None,
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

fn resume_req<'a>(session: &'a str, body: &'a str) -> RoleResumeRequest<'a> {
    RoleResumeRequest {
        session,
        body,
        channel: None,
        kind: MessageKind::Task,
        priority: Priority::Normal,
        disposition: Disposition::InTurn,
        thread: None,
        refs: Refs::default(),
        model: None,
    }
}

/// A session that already exists at a given depth — the shape a role's own session has by the time
/// it is running and calling `nxc` itself.
fn seed_session(store: &mut ChatStore, id: &str, role: &str, depth: u32) {
    store.create_pending_session(id, role).unwrap();
    store.record_trigger_depth(id, depth).unwrap();
}

fn message_count(store: &ChatStore) -> i64 {
    store
        .connection()
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap()
}

// ---- what a trigger writes ------------------------------------------------------------------

#[test]
fn a_trigger_stamps_the_spawned_session_one_hop_past_its_own() {
    // The write half. `NXC_HOP` was already handed to the spawned process; now the SAME value is
    // also recorded against its session, by this process, before the spawn.
    let (_tmp, mut store) = fresh_store();
    let defs = defs();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, None, 0);

    let receipt =
        orchestration::coordinator_commission(&c, &mut store, trigger_req("coder", "go")).unwrap();

    assert_eq!(store.session_depth(&receipt.session).unwrap(), Some(1));
    assert_eq!(
        worker.last_hop_stamp(),
        "1",
        "the persisted depth and the env stamp are the same number, by construction"
    );
}

#[test]
fn the_depth_of_a_chain_comes_from_the_store_not_from_what_the_caller_claims() {
    // The read half, and the ticket's core case: a role five hops deep summons another with its
    // hop counter wiped (`hop: 0`). The chain must still count from five.
    let (_tmp, mut store) = fresh_store();
    let defs = defs();
    seed_session(&mut store, "s-pm", "pm", 5);
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, Some("s-pm"), 0);

    let receipt =
        orchestration::coordinator_commission(&c, &mut store, trigger_req("coder", "go")).unwrap();

    assert_eq!(
        worker.last_hop_stamp(),
        "6",
        "the next hop counts from the PERSISTED depth of the caller's session, not from its claim"
    );
    assert_eq!(store.session_depth(&receipt.session).unwrap(), Some(6));
}

// ---- what the guard refuses -----------------------------------------------------------------

#[test]
fn a_bash_role_that_wipes_its_own_hop_counter_is_still_refused_past_the_cap() {
    // The bypass this ticket exists for, end to end: the caller claims a fresh chain (`hop: 0`,
    // which is exactly what `unset NXC_HOP` produces on the CLI seam), and its own session says
    // otherwise. Refused with nothing persisted and nothing spawned — the same shape the guard has
    // always had for an honest caller.
    let (_tmp, mut store) = fresh_store();
    let defs = defs();
    seed_session(&mut store, "s-pm", "pm", MAX_HOP + 1);
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, Some("s-pm"), 0);
    let before = message_count(&store);

    let err = orchestration::coordinator_commission(&c, &mut store, trigger_req("coder", "go"))
        .unwrap_err();

    assert_eq!(err.kind, ErrorKind::Validation, "{err:?}");
    assert!(err.msg.contains("depth guard"), "{}", err.msg);
    assert_eq!(worker.count(), 0, "nothing was spawned");
    assert_eq!(message_count(&store), before, "nothing was persisted");
}

#[test]
fn the_boundary_holds_at_the_persisted_depth_too() {
    // `MAX_HOP` is the last ALLOWED depth (the guard is `> MAX_HOP`), and that has to be true of the
    // persisted counter as well as the claimed one — an off-by-one here either strands legitimate
    // chains one hop early or lets one through.
    let (_tmp, mut store) = fresh_store();
    let defs = defs();
    seed_session(&mut store, "s-edge", "pm", MAX_HOP);
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, Some("s-edge"), 0);

    orchestration::coordinator_commission(&c, &mut store, trigger_req("coder", "go"))
        .expect("a chain exactly at the cap still gets its last hop");
    assert_eq!(worker.count(), 1);
}

#[test]
fn a_claimed_hop_above_the_persisted_one_still_counts() {
    // The claim is a FLOOR, not dead weight: a host that drives a chain of its own and tracks the
    // count itself keeps getting its value honoured. Only the downward direction is taken away.
    let (_tmp, mut store) = fresh_store();
    let defs = defs();
    seed_session(&mut store, "s-shallow", "pm", 1);
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, Some("s-shallow"), MAX_HOP + 1);

    let err = orchestration::coordinator_commission(&c, &mut store, trigger_req("coder", "go"))
        .unwrap_err();

    assert_eq!(err.kind, ErrorKind::Validation, "{err:?}");
    assert_eq!(worker.count(), 0);
}

#[test]
fn a_caller_with_no_session_is_still_judged_by_what_it_claims() {
    // The unchanged case, pinned because it is what keeps a bare-terminal kickoff and an app's first
    // call working: no session means nothing to look up, so the claim is all there is.
    let (_tmp, mut store) = fresh_store();
    let defs = defs();
    let worker = RecordingWorker::default();

    let over = ctx(&defs, &worker, None, MAX_HOP + 1);
    assert_eq!(
        orchestration::coordinator_commission(&over, &mut store, trigger_req("coder", "go"))
            .unwrap_err()
            .kind,
        ErrorKind::Validation
    );

    let at = ctx(&defs, &worker, None, MAX_HOP);
    orchestration::coordinator_commission(&at, &mut store, trigger_req("coder", "go")).unwrap();
    assert_eq!(worker.count(), 1);
}

#[test]
fn an_unknown_session_id_contributes_nothing_and_is_not_an_error() {
    // A hand-set `NXC_SESSION` naming nothing, or a session id from another workspace, reads as "no
    // session of ours" rather than as depth zero-with-a-row or as a failure: the guard must not
    // reject a caller for holding an id it cannot resolve.
    let (_tmp, mut store) = fresh_store();
    let defs = defs();
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, Some("s-never-minted"), 3);

    orchestration::coordinator_commission(&c, &mut store, trigger_req("coder", "go")).unwrap();

    assert_eq!(worker.last_hop_stamp(), "4", "falls back to the claim");
}

#[test]
fn a_spawn_that_fails_has_still_spent_a_hop() {
    // The documented trade-off in `trigger_role`, pinned so it stays a decision rather than becoming
    // an accident. The depth is written BEFORE `Worker::trigger`, because the trigger is
    // fire-and-forget and the session it starts can be reading its own depth while the call is still
    // returning — so a spawn that fails has already spent budget.
    //
    // The direction matters more than the number: this is the fail-closed side of the trade. If
    // someone later "fixes" this by writing the depth only on success, the guard starts depending on
    // a detached process losing a race, and this test is what says that was not an oversight.
    let (_tmp, mut store) = fresh_store();
    let defs = defs();
    seed_session(&mut store, "s-coder", "coder", 0);
    let worker = FailingWorker;
    let c = ctx(&defs, &worker, None, 0);

    let err = orchestration::role_resume(&c, &mut store, resume_req("s-coder", "go")).unwrap_err();

    assert_eq!(err.kind, ErrorKind::Io, "{err:?}");
    assert_eq!(
        store.session_depth("s-coder").unwrap(),
        Some(1),
        "the hop was spent before the spawn was attempted, and is not rolled back"
    );
}

// ---- the chain the guard exists for ---------------------------------------------------------

#[test]
fn two_roles_that_only_bounce_between_each_other_run_out_of_depth() {
    // The loop the guard was written for, with BOTH roles wiping their counters on every hop
    // (`hop: 0` throughout). It terminates only because each trigger deepens the session it
    // resumes — a depth recorded at mint time alone would leave these two at a fixed depth apiece
    // and let them bounce forever.
    //
    // **The counterpart of `a_bounded_dialogue_between_long_lived_roles_never_runs_out_of_depth`,
    // and the reason the two are distinguishable at all** (nxf 6j6v.ka09). Both are two long-lived
    // roles taking turns; what separates them is that this one only ever COMMISSIONS — `role_resume`
    // hands a LIVE session a fresh task each way, and nobody ever answers — so the open chain grows
    // without bound and hits the cap. The dialogue below commissions and is answered, so its chain
    // unwinds every round and stays three deep forever.
    let (_tmp, mut store) = fresh_store();
    let defs = defs();
    seed_session(&mut store, "s-pm", "pm", 0);
    seed_session(&mut store, "s-coder", "coder", 0);
    let worker = RecordingWorker::default();

    let mut caller = "s-pm";
    let mut target = "s-coder";
    for _ in 0..(MAX_HOP + 5) {
        let c = ctx(&defs, &worker, Some(caller), 0);
        match orchestration::role_resume(&c, &mut store, resume_req(target, "ping")) {
            Ok(_) => std::mem::swap(&mut caller, &mut target),
            Err(e) => {
                assert_eq!(e.kind, ErrorKind::Validation, "{e:?}");
                assert!(e.msg.contains("depth guard"), "{}", e.msg);
                assert!(
                    worker.count() > MAX_HOP as usize,
                    "the cap should bound the bounce, not cut it short: {} hops",
                    worker.count()
                );
                return;
            }
        }
    }
    panic!(
        "the bounce never ran out of depth after {} hops",
        MAX_HOP + 5
    );
}

// ---- the dialogue the guard must NOT bound (nxf 6j6v.ka09) ----------------------------------

/// The three long-lived sessions of an orchestrator chain, built the way the runtime builds them: a
/// human summons the orchestrator, the orchestrator summons the channel owner, the owner summons the
/// worker. Returns `(orchestrator, owner, worker)` internal session ids at depths 1, 2 and 3.
fn seed_orchestrator_chain(
    store: &mut ChatStore,
    defs: &Definitions,
    worker: &dyn Worker,
) -> (String, String, String) {
    let s_o = orchestration::coordinator_commission(
        &ctx_as(defs, worker, "alice", None, 0),
        store,
        trigger_req("orchestrator", "run the board"),
    )
    .unwrap()
    .session;
    let s_pm = orchestration::coordinator_commission(
        &ctx_as(defs, worker, "orchestrator", Some(&s_o), 0),
        store,
        trigger_req("pm", "own this channel"),
    )
    .unwrap()
    .session;
    let s_coder = orchestration::coordinator_commission(
        &ctx_as(defs, worker, "pm", Some(&s_pm), 0),
        store,
        trigger_req("coder", "first task"),
    )
    .unwrap()
    .session;
    (s_o, s_pm, s_coder)
}

/// One full round of that chain: two commissions DOWN (`role_resume`) and the two answers back
/// UP (`reply`, each routed by the commissioning message's own return address).
///
/// Returns the first refusal rather than unwrapping, so a caller can say WHICH round broke.
fn orchestrator_round(
    store: &mut ChatStore,
    defs: &Definitions,
    worker: &dyn Worker,
    (s_o, s_pm, s_coder): (&str, &str, &str),
) -> std::result::Result<(), nexus_chat::error::NxfError> {
    // Down: the orchestrator commissions the owner...
    let commission = orchestration::role_resume(
        &ctx_as(defs, worker, "orchestrator", Some(s_o), 0),
        store,
        resume_req(s_pm, "next slice, please"),
    )?
    .message_id;
    // ...and the owner commissions the worker.
    let task = orchestration::role_resume(
        &ctx_as(defs, worker, "pm", Some(s_pm), 0),
        store,
        resume_req(s_coder, "implement it"),
    )?
    .message_id;
    // Up: the worker answers the owner, the owner answers the orchestrator.
    orchestration::reply(
        &ctx_as(defs, worker, "coder", Some(s_coder), 0),
        store,
        reply_req(&task, "implemented"),
    )?;
    orchestration::reply(
        &ctx_as(defs, worker, "pm", Some(s_pm), 0),
        store,
        reply_req(&commission, "slice delivered"),
    )?;
    Ok(())
}

#[test]
fn a_bounded_dialogue_between_long_lived_roles_never_runs_out_of_depth() {
    // The ticket's case (nxf 6j6v.ka09), and the reason the guard could not be left as it was: the
    // orchestrator's design IS this loop — commission down, answer back up — and a counter that only
    // ever rises made it terminal. Every answer travelling back up used to deepen the session it
    // returned to, so the orchestrator gained four hops a round and became permanently uncallable
    // after seven, with no way back: `MAX` never lowers, and a session handover replaces the RUNTIME
    // session, not the engine's internal one.
    //
    // Thirty rounds is four times what it used to survive, and the depths are asserted as CONSTANT
    // rather than merely under the cap: "it lasts longer now" would be the same defect with a bigger
    // number in it.
    let (_tmp, mut store) = fresh_store();
    let defs = orchestrator_defs();
    let worker = RecordingWorker::default();
    let (s_o, s_pm, s_coder) = seed_orchestrator_chain(&mut store, &defs, &worker);
    assert_eq!(store.session_depth(&s_o).unwrap(), Some(1));
    assert_eq!(store.session_depth(&s_pm).unwrap(), Some(2));
    assert_eq!(store.session_depth(&s_coder).unwrap(), Some(3));

    for round in 1..=30 {
        orchestrator_round(&mut store, &defs, &worker, (&s_o, &s_pm, &s_coder))
            .unwrap_or_else(|e| panic!("round {round} refused: {:?} — {}", e.kind, e.msg));
    }

    assert_eq!(
        (
            store.session_depth(&s_o).unwrap(),
            store.session_depth(&s_pm).unwrap(),
            store.session_depth(&s_coder).unwrap(),
        ),
        (Some(1), Some(2), Some(3)),
        "the chain is three deep and stays three deep, however many rounds it runs"
    );
}

#[test]
fn an_answer_travelling_back_up_the_chain_does_not_deepen_the_session_it_returns_to() {
    // The mechanism, in one hop. A reply is routed by the commissioning message's return address, so
    // the session it wakes is the one that COMMISSIONED the work — the wake closes that link rather
    // than opening a new one, and the depth it re-enters at is its own.
    //
    // Both halves are asserted, because `NXC_HOP` is the half that used to smuggle the ratchet back
    // in: `resolve_hop` takes the larger of the persisted depth and the claim, so a stamp of
    // `caller + 1` would raise the woken session's depth on its very next verb even with the
    // persisted write held flat.
    let (_tmp, mut store) = fresh_store();
    let defs = orchestrator_defs();
    let worker = RecordingWorker::default();
    let (_s_o, s_pm, s_coder) = seed_orchestrator_chain(&mut store, &defs, &worker);

    let task = orchestration::role_resume(
        &ctx_as(&defs, &worker, "pm", Some(&s_pm), 0),
        &mut store,
        resume_req(&s_coder, "implement it"),
    )
    .unwrap()
    .message_id;
    let receipt = orchestration::reply(
        &ctx_as(&defs, &worker, "coder", Some(&s_coder), 0),
        &mut store,
        reply_req(&task, "implemented"),
    )
    .unwrap();

    assert_eq!(receipt.woke.as_deref(), Some(s_pm.as_str()));
    assert_eq!(
        store.session_depth(&s_pm).unwrap(),
        Some(2),
        "the owner is two deep before the answer and two deep after it"
    );
    assert_eq!(
        worker.last_hop_stamp(),
        "2",
        "the resumed session is handed its OWN depth, not the answering session's plus one"
    );
}

#[test]
fn an_answer_restores_the_targets_depth_as_it_is_now_not_as_it_was_when_it_asked() {
    // `Unwind` keeps the target's own depth — it does NOT roll it back to whatever it was when the
    // commissioning message went out, which is the bypass a "restore the depth of the moment" reading
    // would open: a role five hops deep need only get an old message of its own replied to in order
    // to come back shallow. Here the owner is driven to 20 AFTER commissioning, and the answer finds
    // it at 20.
    //
    // This is 6j6v.m48m's monotonicity holding from the new direction, which is why it is pinned
    // separately from the depth arithmetic above: `record_trigger_depth` still refuses to lower, and
    // the `NXC_HOP` stamp agrees with it rather than handing the woken session a smaller claim.
    let (_tmp, mut store) = fresh_store();
    let defs = orchestrator_defs();
    let worker = RecordingWorker::default();
    let (_s_o, s_pm, s_coder) = seed_orchestrator_chain(&mut store, &defs, &worker);

    let task = orchestration::role_resume(
        &ctx_as(&defs, &worker, "pm", Some(&s_pm), 0),
        &mut store,
        resume_req(&s_coder, "implement it"),
    )
    .unwrap()
    .message_id;
    store.record_trigger_depth(&s_pm, 20).unwrap();

    orchestration::reply(
        &ctx_as(&defs, &worker, "coder", Some(&s_coder), 0),
        &mut store,
        reply_req(&task, "implemented"),
    )
    .unwrap();

    assert_eq!(store.session_depth(&s_pm).unwrap(), Some(20));
    assert_eq!(
        worker.last_hop_stamp(),
        "20",
        "an answer is not a way back down the chain"
    );
}

#[test]
fn resuming_a_deep_session_from_a_fresh_caller_does_not_hand_it_a_new_budget() {
    // Monotonicity, from the outside: a human at the terminal (no session, no claim) resuming a
    // session a deep chain already drove must not reset that session's depth. Assignment instead of
    // `MAX` would reopen the bypass from this direction — a role need only arrange for a shallow
    // caller to touch the session it wants refreshed.
    let (_tmp, mut store) = fresh_store();
    let defs = defs();
    seed_session(&mut store, "s-deep", "coder", 20);
    let worker = RecordingWorker::default();
    let c = ctx(&defs, &worker, None, 0);

    orchestration::role_resume(&c, &mut store, resume_req("s-deep", "carry on")).unwrap();

    assert_eq!(store.session_depth("s-deep").unwrap(), Some(20));
    assert_eq!(
        worker.last_hop_stamp(),
        "1",
        "this particular hop is shallow — the SESSION's own depth is what is preserved"
    );
}
