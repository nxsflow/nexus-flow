//! ACCEPTANCE of nxf 6j6v.gn8b — **the persona coordinator COLLECTS**: one wake with everything
//! that arrived while the caller was working, and it names what is still outstanding.
//!
//! # The situation this closes
//!
//! A declared CHANNEL knows when a round is complete and resumes its requester ONCE with all of it.
//! The persona path had no notion of a set: a caller that commissions three personas in parallel
//! opens three threads, and each reply resumed it separately — when it resumed it at all. A reply
//! arriving while the caller's session is mid-turn is refused by the single-process guard (nxf
//! 6j6v.7qtf), and the whole of what happened was a `wake_skipped: {reason: "already_running"}` on
//! the REPLIER's receipt: a line the caller never sees, about an answer it would never be told
//! about.
//!
//! # What is real here and what is not
//!
//! The store, the surface verbs (`send_to`, `reply_in_thread`), the coordinator's queue and the
//! delivery are all real. The one stand-in is the WORKER, and only for what a worker does:
//! [`BusyWorker`] starts no operating-system session — it records like `DryWorker` — and refuses a
//! named session with exactly the [`TriggerError::AlreadyRunning`] the real single-process guard
//! raises. That refusal IS the input this whole item turns on, and it cannot be produced by running
//! a real session in a test.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nexus_chat::collecting::Held;
use nexus_chat::orchestration::{self, Ctx};
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{self, ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{Timer, TimerHandle};
use nexus_chat::worker::{DryWorker, TriggerError, TriggerRequest, TriggerResult, Worker};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const T0: &str = "2026-09-07T09:00:00Z";
const ANSWERED: &str = "2026-09-07T09:00:08Z";
const SETTLED: &str = "2026-09-07T09:05:00Z";

// ---- harness ------------------------------------------------------------------------------------

/// A recording worker that REFUSES a named session the way the real single-process guard does.
struct BusyWorker {
    log: PathBuf,
    busy: Vec<String>,
    /// **This worker cannot answer the process question** — [`Worker::session_is_running`]'s
    /// reasoned default (`false` for everything) plus
    /// [`Worker::answers_liveness`]`() == false`, which is what every worker that does not look
    /// presents to the engine. It still REFUSES a busy session at `trigger`, because that refusal
    /// comes from the single-process guard and not from a liveness read.
    blind: bool,
}

impl Worker for BusyWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        if self.busy.contains(&req.internal_session) {
            return Err(TriggerError::AlreadyRunning {
                session: req.internal_session.clone(),
                pid: 4242,
            });
        }
        DryWorker {
            log: Some(self.log.clone()),
        }
        .trigger(req)
    }
    fn session_is_running(&self, internal_session: &str) -> bool {
        // A worker that CANNOT tell answers the reasoned default (`false`) for every session, which
        // is what `blind` models — see the sweep test that drives it.
        !self.blind && self.busy.iter().any(|s| s == internal_session)
    }
    fn answers_liveness(&self) -> bool {
        !self.blind
    }
}

/// A timer that records what would have been armed, instead of writing into a workspace's deadline
/// book and asking a background service to honour it.
///
/// The delivery deadline is the one thing `session ended` DOES about a held answer — it cannot
/// deliver it itself, because the process making the announcement is still alive while it speaks —
/// so what is under test is exactly what this records.
#[derive(Default)]
struct RecordingTimer {
    armed: Mutex<Vec<(String, String)>>,
}

impl Timer for RecordingTimer {
    fn schedule(
        &self,
        _thread: &str,
        _deadline: &str,
        _command: &str,
    ) -> nexus_chat::error::Result<TimerHandle> {
        unreachable!("this bench arms no board windows")
    }
    fn schedule_delivery(
        &self,
        session: &str,
        deadline: &str,
    ) -> nexus_chat::error::Result<TimerHandle> {
        self.armed
            .lock()
            .unwrap()
            .push((session.to_string(), deadline.to_string()));
        Ok(TimerHandle(format!("recorded:{session}")))
    }
    fn cancel(&self, _handle: &TimerHandle) -> nexus_chat::error::Result<()> {
        Ok(())
    }
}

struct Bench {
    tmp: TempDir,
    timer: RecordingTimer,
    /// Whether this bench's worker can answer the liveness question at all.
    blind: RefCell<bool>,
    /// Which sessions the worker reports as working. Moved by the test, which is how "the caller is
    /// mid-turn" and "the caller has settled" are two values in this file rather than a sleep.
    busy: RefCell<Vec<String>>,
}

impl Bench {
    fn new() -> Bench {
        let tmp = TempDir::new().unwrap();
        setup(tmp.path(), &chat_config()).expect("seed chat workspace");
        let roles = tmp.path().join(".nxs-personas");
        std::fs::create_dir_all(&roles).unwrap();
        for handle in ["pm", "coder-a", "coder-b", "coder-c"] {
            std::fs::write(
                roles.join(format!("{handle}.yaml")),
                format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
            )
            .unwrap();
        }
        // A declared channel, so the THIRD wake path — the one that carries a whole channel round's
        // composed answer back to the caller that commissioned it — is reachable from this bench.
        std::fs::write(
            roles.join("channels.yaml"),
            "- name: review\n  members: [coder-a, coder-b]\n",
        )
        .unwrap();
        Bench {
            tmp,
            timer: RecordingTimer::default(),
            blind: RefCell::new(false),
            busy: RefCell::new(Vec::new()),
        }
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn working(&self, sessions: &[&str]) {
        *self.busy.borrow_mut() = sessions.iter().map(|s| s.to_string()).collect();
    }

    /// From here on the worker cannot answer the process question — see [`BusyWorker::blind`].
    fn cannot_tell_who_is_running(&self) {
        *self.blind.borrow_mut() = true;
    }

    fn store(&self) -> ChatStore {
        Workspace::resolve(None, self.root())
            .expect("resolve")
            .open_chat_store()
            .expect("open store")
    }

    /// Run `f` with a freshly built [`Ctx`] and a freshly opened store, then drop both — so every
    /// step in a test crosses a real process boundary as far as the database is concerned.
    fn with<T>(
        &self,
        now: &str,
        actor: &str,
        session: Option<&str>,
        f: impl FnOnce(&Ctx, &mut ChatStore) -> T,
    ) -> T {
        let ws = Workspace::resolve(None, self.root()).expect("resolve");
        let db_path = ws.db_path_str().expect("db path");
        let mut store = ws.open_chat_store().expect("open store");
        let defs =
            nexus_chat::definitions::Definitions::resolve(self.root()).expect("declarations");
        let worker = BusyWorker {
            log: self.root().join("dry.log"),
            busy: self.busy.borrow().clone(),
            blind: *self.blind.borrow(),
        };
        let ctx = Ctx {
            now,
            origin: "local",
            actor,
            session,
            hop: 0,
            defs: &defs,
            worker: &worker,
            timer: &self.timer,
            namer: &nexus_chat::naming::DryNamer,
            db_path: &db_path,
            project_claude_md: None,
            module_primes: None,
            machines: None,
        };
        f(&ctx, &mut store)
    }

    /// Commission `to` and hand back `(thread, the session that was minted for it)`.
    fn commission(
        &self,
        now: &str,
        actor: &str,
        session: Option<&str>,
        to: &str,
    ) -> (String, String) {
        self.with(now, actor, session, |ctx, store| {
            let r = surface::send_to(
                ctx,
                store,
                SendToRequest {
                    machine: None,
                    to,
                    body: "please do this",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("commission");
            (r.thread_id, r.session.expect("a persona send mints one"))
        })
    }

    /// Commission a declared CHANNEL — the same verb, the other kind of target. A channel send
    /// mints no single session, so this hands back the board thread alone.
    fn commission_channel(
        &self,
        now: &str,
        actor: &str,
        session: Option<&str>,
        to: &str,
    ) -> String {
        self.with(now, actor, session, |ctx, store| {
            surface::send_to(
                ctx,
                store,
                SendToRequest {
                    machine: None,
                    to,
                    body: "please look at this",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("commission")
            .thread_id
        })
    }

    /// The member threads a supervised channel opened under `board`, in slot order.
    fn member_threads(&self, board: &str) -> Vec<String> {
        self.store()
            .supervised_children(board)
            .expect("the supervisor opened them")
    }

    fn reply(
        &self,
        now: &str,
        actor: &str,
        session: &str,
        thread: &str,
        escalate: bool,
        body: &str,
    ) -> orchestration::ReplyReceipt {
        self.with(now, actor, Some(session), |ctx, store| {
            surface::reply_in_thread(
                ctx,
                store,
                ReplyThreadRequest {
                    machine: None,
                    thread,
                    body,
                    escalate,
                    needs_rework: false,
                    accept: false,
                },
            )
            .expect("reply is posted and routed")
        })
    }

    fn session_ended(&self, now: &str, session: &str) -> orchestration::SessionEndedReceipt {
        self.with(now, "sidecar", None, |ctx, store| {
            orchestration::session_ended(ctx, store, session).expect("the announcement is recorded")
        })
    }

    fn tick(&self, now: &str, thread: &str) -> orchestration::TickReceipt {
        self.with(now, "service", None, |ctx, store| {
            orchestration::tick(ctx, store, orchestration::TickRequest { thread_id: thread })
                .expect("tick")
        })
    }

    fn armed_deliveries(&self) -> Vec<(String, String)> {
        self.timer.armed.lock().unwrap().clone()
    }

    fn deliver(&self, now: &str, session: &str) -> orchestration::DeliveryReceipt {
        self.with(now, "service", None, |ctx, store| {
            orchestration::deliver_held(ctx, store, session).expect("delivery")
        })
    }

    /// Every message the worker was handed, in order.
    fn triggers(&self) -> Vec<String> {
        std::fs::read_to_string(self.root().join("dry.log"))
            .unwrap_or_default()
            .split("trigger role=")
            .skip(1)
            .map(|s| s.to_string())
            .collect()
    }
}

/// The PM, commissioned by a human, with the two coders it then commissions itself. Returns
/// `(pm session, thread to coder-a, thread to coder-b)`.
fn a_pm_with_two_commissions_out(bench: &Bench) -> (String, String, String) {
    let (_, pm) = bench.commission(T0, "carsten", None, "pm");
    let (a, _) = bench.commission(T0, "pm", Some(&pm), "coder-a");
    let (b, _) = bench.commission(T0, "pm", Some(&pm), "coder-b");
    (pm, a, b)
}

// ---- the run ------------------------------------------------------------------------------------

#[test]
fn answers_that_arrive_while_the_caller_works_are_held_and_delivered_once() {
    let bench = Bench::new();
    let (pm, a, b) = a_pm_with_two_commissions_out(&bench);

    // The PM is mid-turn — which is the ordinary case, because it commissioned three personas in
    // one turn and the first answers eight seconds later.
    bench.working(&[&pm]);

    let first = bench.reply(ANSWERED, "coder-a", "s-a", &a, false, "A is done");
    assert_eq!(
        first.held.as_deref(),
        Some(pm.as_str()),
        "the answer landed and is HELD for its caller — not lost, and not reported as a failure"
    );
    assert!(
        first.wake_skipped.is_none(),
        "a held delivery is not a skipped one: {first:?}"
    );
    assert!(
        first.warnings.is_empty(),
        "and nothing failed, so nothing is warned about: {first:?}"
    );

    let second = bench.reply(ANSWERED, "coder-b", "s-b", &b, false, "B is done");
    assert_eq!(second.held.as_deref(), Some(pm.as_str()));

    // **Durable between "arrived" and "settled".** Both replies ran in their own `Ctx` with their
    // own store handle, which was dropped; this is a fresh open of the file on disk.
    assert_eq!(
        bench
            .store()
            .held_wakes(&pm)
            .unwrap()
            .iter()
            .map(|r| r.held.body.clone())
            .collect::<Vec<_>>(),
        vec!["A is done".to_string(), "B is done".to_string()],
        "held in arrival order, and still there after the process that held them is gone"
    );

    let before = bench.triggers().len();
    bench.working(&[]);
    let receipt = bench.deliver(SETTLED, &pm);

    assert_eq!(receipt.delivered, 2);
    assert_eq!(receipt.woke.as_deref(), Some(pm.as_str()));
    assert_eq!(
        bench.triggers().len(),
        before + 1,
        "ONE wake, not one per answer"
    );
    let delivery = bench.triggers().last().unwrap().clone();
    assert!(
        delivery.contains("While you were working, 2 message(s) arrived"),
        "{delivery}"
    );
    assert!(delivery.contains("A is done"), "{delivery}");
    assert!(delivery.contains("B is done"), "{delivery}");
    // The channel delivery shape: one framed block per message, carrying the engine's own record of
    // who posted and WHICH commission it answers.
    assert!(
        delivery.contains(&format!("thread=\"{a}\"")),
        "the answer says which commission it belongs to: {delivery}"
    );
    assert!(delivery.contains("from=\"local/coder-a\""), "{delivery}");
    assert!(
        delivery.contains("<untrusted_channel_replies>"),
        "the same framing a channel round's pass-through uses: {delivery}"
    );

    // Released only once the resume was handed over.
    assert!(bench.store().held_wakes(&pm).unwrap().is_empty());
    // …and running the delivery again is a clean no-op, because a scheduled job and a sweep may
    // both reach a session whose queue the other has drained.
    let again = bench.deliver(SETTLED, &pm);
    assert_eq!(again.delivered, 0);
    assert_eq!(bench.triggers().len(), before + 1, "and it wakes nobody");
}

#[test]
fn the_delivery_names_what_is_still_outstanding() {
    // "Everything that arrived" is not "all the answers": two of three answer in a minute and the
    // third in twenty. A wake with two is honest only if it SAYS the third is still owed — without
    // that the wakes are merely batched and the bookkeeping still sits with the caller.
    let bench = Bench::new();
    let (pm, a, _b) = a_pm_with_two_commissions_out(&bench);
    let (slow, _) = bench.commission(T0, "pm", Some(&pm), "coder-c");

    bench.working(&[&pm]);
    bench.reply(ANSWERED, "coder-a", "s-a", &a, false, "A is done");
    bench.working(&[]);

    let receipt = bench.deliver(SETTLED, &pm);
    assert_eq!(receipt.delivered, 1);
    assert!(
        receipt.outstanding.contains(&slow),
        "the commission nobody has answered is on the receipt: {receipt:?}"
    );
    let delivery = bench.triggers().last().unwrap().clone();
    assert!(
        delivery.contains("Still outstanding: 2 commission(s)"),
        "…and in the text the caller actually reads: {delivery}"
    );
    assert!(
        delivery.contains(&format!("{slow} — expects local/coder-c")),
        "named by thread AND by who still owes it: {delivery}"
    );
}

#[test]
fn a_hand_back_is_delivered_set_apart_and_never_batched_away() {
    // "I cannot carry this out" is the one message that stops a chain. Delivering it in a stack of
    // four spends exactly the urgency it exists for.
    let bench = Bench::new();
    let (pm, a, b) = a_pm_with_two_commissions_out(&bench);

    bench.working(&[&pm]);
    bench.reply(ANSWERED, "coder-a", "s-a", &a, false, "A is done");
    bench.reply(
        ANSWERED,
        "coder-b",
        "s-b",
        &b,
        true,
        "I cannot reach the repository",
    );
    bench.working(&[]);

    let receipt = bench.deliver(SETTLED, &pm);
    assert_eq!(receipt.delivered, 2);
    assert_eq!(receipt.escalations, 1);

    let delivery = bench.triggers().last().unwrap().clone();
    let notice = delivery
        .find("ESCALATION — this is NOT a result")
        .expect("the hand-back announces itself in full");
    let handed_back = delivery
        .find("I cannot reach the repository")
        .expect("and its own words are in the delivery");
    let ordinary = delivery
        .find("A is done")
        .expect("beside the ordinary answer");
    assert!(
        notice < handed_back && handed_back < ordinary,
        "the hand-back and its notice stand AHEAD of the ordinary answers, in their own block: \
         {delivery}"
    );
    assert!(
        delivery.contains("The hand-back(s) — 1 of the 2:"),
        "and the delivery says which is which: {delivery}"
    );
}

#[test]
fn a_resume_that_does_not_land_leaves_the_queue_exactly_where_it_was() {
    // The other half of "durable": the rows are deleted AFTER the hand-over, so a caller that
    // started working again between the settle and the delivery loses nothing.
    let bench = Bench::new();
    let (pm, a, _b) = a_pm_with_two_commissions_out(&bench);

    bench.working(&[&pm]);
    bench.reply(ANSWERED, "coder-a", "s-a", &a, false, "A is done");

    // Still working when the delivery is attempted.
    let receipt = bench.deliver(SETTLED, &pm);
    assert_eq!(receipt.delivered, 1, "it tried");
    assert!(receipt.woke.is_none(), "and it did not land: {receipt:?}");
    assert!(
        receipt.wake_skipped.is_some(),
        "which is reported rather than swallowed: {receipt:?}"
    );
    assert_eq!(
        bench.store().held_wakes(&pm).unwrap().len(),
        1,
        "the answer is still held — nothing is deleted before it has been handed over"
    );

    // And the next settle carries it.
    bench.working(&[]);
    assert_eq!(bench.deliver(SETTLED, &pm).delivered, 1);
    assert!(bench.store().held_wakes(&pm).unwrap().is_empty());
}

#[test]
fn a_caller_that_cannot_be_resumed_at_all_produces_no_lost_message_and_no_silent_skip() {
    // The case that does not disappear: the session behind the return address is not a role this
    // workspace knows any more. Nothing is held — there is nothing that could ever be handed over —
    // and the replier is told so on its own receipt rather than reading a bare success.
    let bench = Bench::new();
    let (a, _) = bench.commission(T0, "carsten", None, "coder-a");
    let receipt = bench.reply(ANSWERED, "coder-a", "s-a", &a, false, "done");
    assert!(
        receipt.held.is_none() && receipt.woke.is_none(),
        "a human at a terminal has no session to resume: {receipt:?}"
    );
    assert!(
        bench.store().sessions_holding_wakes().unwrap().is_empty(),
        "and nothing is queued for a caller nobody could ever wake"
    );
}

#[test]
fn the_queue_is_idempotent_so_a_re_attempted_wake_cannot_deliver_an_answer_twice() {
    let bench = Bench::new();
    let (pm, a, _b) = a_pm_with_two_commissions_out(&bench);
    bench.working(&[&pm]);
    bench.reply(ANSWERED, "coder-a", "s-a", &a, false, "A is done");

    let held = bench.store().held_wakes(&pm).unwrap();
    assert_eq!(held.len(), 1);
    let item: Held = held[0].held.clone();
    assert!(
        !bench.store().hold_wake(&pm, &item, SETTLED).unwrap(),
        "holding the same message a second time writes nothing"
    );
    assert_eq!(bench.store().held_wakes(&pm).unwrap().len(), 1);
}

#[test]
fn the_announcement_of_a_session_end_arms_the_delivery_rather_than_making_it() {
    // **The settle signal, and why it is an ARMING and not a delivery.** The announcement is made BY
    // the ending session, whose process is necessarily still alive while it speaks — so a resume
    // from inside `session ended` is refused by the very guard that put the answer in the queue.
    // What `session ended` can do is say when to try, and that is what this asserts.
    let bench = Bench::new();
    let (pm, a, _b) = a_pm_with_two_commissions_out(&bench);
    bench.working(&[&pm]);
    bench.reply(ANSWERED, "coder-a", "s-a", &a, false, "A is done");

    let receipt = bench.session_ended(SETTLED, &pm);
    assert!(
        receipt.warnings.is_empty(),
        "arming succeeded, so there is nothing to report: {receipt:?}"
    );
    assert_eq!(
        bench.armed_deliveries(),
        vec![(pm.clone(), "2026-09-07T09:05:10Z".to_string())],
        "one deadline, for THIS caller, a grace after the announcement"
    );
    assert_eq!(
        bench.store().held_wakes(&pm).unwrap().len(),
        1,
        "and nothing was delivered from inside the announcement itself"
    );
}

#[test]
fn a_session_end_with_nothing_held_arms_nothing() {
    // The ordinary case, and it must stay free: every persona turn ends with this call.
    let bench = Bench::new();
    let (pm, _a, _b) = a_pm_with_two_commissions_out(&bench);
    bench.session_ended(SETTLED, &pm);
    assert!(bench.armed_deliveries().is_empty());
}

#[test]
fn a_tick_sweeps_the_queue_of_a_caller_that_was_killed_before_it_could_announce_anything() {
    // The backstop. A session that was killed hard never runs `session ended`, so nothing arms its
    // delivery — and the answers would sit in the queue for good. The tick already asks two
    // workspace-wide questions before it looks at the thread it was given (an expired lease, a
    // stranded holder); this is the third, on the same terms.
    let bench = Bench::new();
    let (pm, a, _b) = a_pm_with_two_commissions_out(&bench);
    bench.working(&[&pm]);
    bench.reply(ANSWERED, "coder-a", "s-a", &a, false, "A is done");
    // No announcement at all — the caller is simply gone.
    bench.working(&[]);

    let before = bench.triggers().len();
    let receipt = bench.tick(SETTLED, &a);
    assert!(
        receipt.warnings.is_empty(),
        "the sweep reports nothing because nothing failed: {receipt:?}"
    );
    assert_eq!(
        bench.triggers().len(),
        before + 1,
        "the held answer was delivered by the tick"
    );
    assert!(bench.store().held_wakes(&pm).unwrap().is_empty());
}

#[test]
fn a_tick_leaves_a_caller_that_is_still_working_exactly_where_it_is() {
    // The other half of the sweep, and the one that keeps it cheap: a session the worker reports as
    // running is skipped without a resume attempt at all.
    let bench = Bench::new();
    let (pm, a, _b) = a_pm_with_two_commissions_out(&bench);
    bench.working(&[&pm]);
    bench.reply(ANSWERED, "coder-a", "s-a", &a, false, "A is done");

    let before = bench.triggers().len();
    bench.tick(SETTLED, &a);
    assert_eq!(bench.triggers().len(), before, "nothing was started");
    assert_eq!(bench.store().held_wakes(&pm).unwrap().len(), 1);
}

#[test]
fn a_sweep_under_a_worker_that_cannot_tell_who_is_running_still_cannot_lose_an_answer() {
    // **The fail-open direction, executed rather than argued** (review of this branch, Test
    // Quality). `Worker::session_is_running`'s default is `false` — "not running" — for a worker
    // that never looks, and the sweep reads it. So a blind worker makes the sweep ATTEMPT every
    // held delivery, including one for a caller that is in fact mid-turn.
    //
    // That is harmless, and this is why: the attempt goes through the single-process guard, which
    // is a pid file and not a liveness opinion. It refuses, the rows stay exactly where they are,
    // and the next settle carries them. The fail-open answer costs one refused spawn, never an
    // answer.
    let bench = Bench::new();
    let (pm, a, _b) = a_pm_with_two_commissions_out(&bench);
    bench.working(&[&pm]);
    bench.reply(ANSWERED, "coder-a", "s-a", &a, false, "A is done");

    // The worker stops being able to answer the question — and the PM is still working.
    bench.cannot_tell_who_is_running();
    let before = bench.triggers().len();
    bench.tick(SETTLED, &a);
    assert_eq!(
        bench.triggers().len(),
        before,
        "the guard refused the spawn, so nothing was started"
    );
    assert_eq!(
        bench.store().held_wakes(&pm).unwrap().len(),
        1,
        "and the answer is still held — a worker that cannot tell cannot lose one"
    );

    // …and once the caller really has settled, the same blind sweep delivers it.
    bench.working(&[]);
    bench.tick(SETTLED, &a);
    assert_eq!(bench.triggers().len(), before + 1);
    assert!(bench.store().held_wakes(&pm).unwrap().is_empty());
}

#[test]
fn a_channel_round_that_completes_while_its_requester_works_is_held_like_any_other_answer() {
    // **The third wake path** (review of this branch, Integrity & Robustness · High). A declared
    // channel collects across its MEMBERS — which is why this path looked as though it needed
    // nothing — but the caller that commissioned the CHANNEL is a requester like any other, and it
    // can be mid-turn when the composed answer finally fires. Before the fix that caller read
    // `wake_skipped: {reason: "already_running"}`, nothing was queued, and the whole consolidated
    // answer was recoverable only by going and looking.
    let bench = Bench::new();
    let (_, pm) = bench.commission(T0, "carsten", None, "pm");
    let board = bench.commission_channel(T0, "pm", Some(&pm), "review");
    let members = bench.member_threads(&board);
    assert_eq!(
        members.len(),
        2,
        "both members were commissioned: {members:?}"
    );

    // The PM is mid-turn by the time the round settles — the ordinary shape, because it commissioned
    // the channel and something else in the same turn.
    bench.working(&[&pm]);
    bench.reply(
        ANSWERED,
        "coder-a",
        "s-a",
        &members[0],
        false,
        "A: looks fine",
    );
    let last = bench.reply(ANSWERED, "coder-b", "s-b", &members[1], false, "B: ship it");
    assert_eq!(
        last.held.as_deref(),
        Some(pm.as_str()),
        "the completing reply's receipt says the channel's answer is HELD, not skipped: {last:?}"
    );
    assert!(
        last.wake_skipped.is_none(),
        "and it is not reported as a failure: {last:?}"
    );

    let held = bench.store().held_wakes(&pm).unwrap();
    assert_eq!(held.len(), 1, "one held delivery for the whole round");
    assert!(
        held[0].held.body.contains("A: looks fine") && held[0].held.body.contains("B: ship it"),
        "…and it is the COMPOSED answer, held verbatim: {}",
        held[0].held.body
    );

    // …and it goes out with the rest when the caller settles.
    bench.working(&[]);
    let receipt = bench.deliver(SETTLED, &pm);
    assert_eq!(receipt.delivered, 1);
    assert_eq!(receipt.woke.as_deref(), Some(pm.as_str()));
    let delivery = bench.triggers().last().unwrap().clone();
    assert!(delivery.contains("A: looks fine"), "{delivery}");
    assert!(
        delivery.contains("While you were working, 1 message(s) arrived"),
        "{delivery}"
    );
}

#[test]
fn a_channel_round_delivers_one_answer_however_often_the_completion_is_re_driven() {
    // The synthetic key `<thread>#channel`: the three outcomes of one round are mutually exclusive,
    // so a re-driven completion must not put the same consolidated answer in front of a caller
    // twice. `nxc tick` is idempotent by design and reaches the same routing.
    let bench = Bench::new();
    let (_, pm) = bench.commission(T0, "carsten", None, "pm");
    let board = bench.commission_channel(T0, "pm", Some(&pm), "review");
    let members = bench.member_threads(&board);
    bench.working(&[&pm]);
    bench.reply(
        ANSWERED,
        "coder-a",
        "s-a",
        &members[0],
        false,
        "A: looks fine",
    );
    bench.reply(ANSWERED, "coder-b", "s-b", &members[1], false, "B: ship it");
    bench.tick(SETTLED, &board);
    assert_eq!(
        bench.store().held_wakes(&pm).unwrap().len(),
        1,
        "still one — the round has one answer, not one per attempt to deliver it"
    );
}

#[test]
fn two_rounds_of_one_channel_thread_are_two_held_answers_and_not_one() {
    // **The collision the round key exists to prevent** (review of this branch, second pass). A
    // thread can legitimately hold a SECOND round: the opener re-declares `expects` back to real
    // members and they answer again — `consolidation_claim`'s own doc names that as supported, and
    // keys its claim on `(expects_reply_from_v, expects_reply_from_site)` for exactly that reason.
    //
    // If the opener's session is STILL busy across both — one persona turn can issue many `nxc`
    // calls before it ever settles — then with a key derived from the thread alone the second
    // round's answer collides with the first's still-queued row and `INSERT OR IGNORE` drops it. No
    // error, no warning, no row: the exact silent loss this whole queue exists to close, one layer
    // further out than the wake paths it was built for.
    let bench = Bench::new();
    let (_, pm) = bench.commission(T0, "carsten", None, "pm");
    let board = bench.commission_channel(T0, "pm", Some(&pm), "review");
    let members = bench.member_threads(&board);

    // Round one completes while the PM works.
    bench.working(&[&pm]);
    bench.reply(
        ANSWERED,
        "coder-a",
        "s-a",
        &members[0],
        false,
        "round one: A",
    );
    bench.reply(
        ANSWERED,
        "coder-b",
        "s-b",
        &members[1],
        false,
        "round one: B",
    );
    assert_eq!(
        bench.store().held_wakes(&pm).unwrap().len(),
        1,
        "round one is held"
    );

    // The opener re-declares the board back to a real member — the next round — WITHOUT ever having
    // settled, and that round completes too.
    bench.with(SETTLED, "pm", Some(&pm), |ctx, store| {
        nexus_chat::facade::set_expects(
            store,
            ctx.now,
            "local/pm",
            &board,
            &["local/coder-a".to_string()],
        )
        .expect("the opener may re-declare its own board");
    });
    bench.reply(
        SETTLED,
        "coder-a",
        "s-a",
        &board,
        false,
        "round two: A again",
    );

    let held = bench.store().held_wakes(&pm).unwrap();
    assert_eq!(
        held.len(),
        2,
        "TWO answers held, not one — the second round did not overwrite the first: {:?}",
        held.iter().map(|r| &r.held.message_id).collect::<Vec<_>>()
    );
    assert!(
        held[0].held.body.contains("round one: A") && held[1].held.body.contains("round two"),
        "…and both are there, in arrival order: {held:?}"
    );

    // …and one wake carries both when the caller finally settles.
    bench.working(&[]);
    assert_eq!(bench.deliver(SETTLED, &pm).delivered, 2);
    let delivery = bench.triggers().last().unwrap().clone();
    assert!(delivery.contains("round one: A"), "{delivery}");
    assert!(delivery.contains("round two"), "{delivery}");
}

#[test]
fn a_raw_quorum_board_that_completes_while_its_opener_works_is_held_too() {
    // `route_completion`'s OWN hold branch (review of this branch, Test Quality · High): the
    // completion path for a board that no declared channel backs — what `facade::ask` opens, and
    // what reaches this workspace by sync. Everything else in this file drives the DM path through
    // `resume_return_address`, so this is the one branch a regression here would go uncaught in.
    let bench = Bench::new();
    let (_, pm) = bench.commission(T0, "carsten", None, "pm");

    let board = {
        let mut store = bench.store();
        store.set_channel_field("c-raw", "kind", "group", "local/pm");
        store.add_member("c-raw", "local/pm", "local/pm");
        store.add_member("c-raw", "local/coder-a", "local/pm");
        store.set_wall_clock(T0);
        nexus_chat::facade::ask(
            &mut store,
            nexus_chat::facade::AskRequest {
                now: T0,
                origin: "local",
                actor: "pm",
                channel: "c-raw",
                body: "have a look",
                expect: &["local/coder-a".to_string()],
                deadline: None,
                kind: nexus_chat::model::MessageKind::Question,
                priority: nexus_chat::model::Priority::Normal,
                // The return address is what makes this the PM's board rather than nobody's.
                refs: nexus_chat::model::Refs {
                    session_id: Some(pm.clone()),
                    ..Default::default()
                },
            },
        )
        .expect("the board opens")
        .thread_id
    };

    bench.working(&[&pm]);
    let receipt = bench.reply(ANSWERED, "coder-a", "s-a", &board, false, "the raw answer");
    assert_eq!(
        receipt.held.as_deref(),
        Some(pm.as_str()),
        "the completing reply into a raw board holds for its busy opener: {receipt:?}"
    );
    assert!(receipt.wake_skipped.is_none(), "{receipt:?}");

    bench.working(&[]);
    assert_eq!(bench.deliver(SETTLED, &pm).delivered, 1);
    assert!(bench.triggers().last().unwrap().contains("the raw answer"));
}
