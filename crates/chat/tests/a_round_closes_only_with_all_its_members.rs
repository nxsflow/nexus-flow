//! ACCEPTANCE of nxf 6j6v.1q0h (and 6j6v.rkww, the same bug) — **a round closes only with every
//! member it declares**.
//!
//! # The cause, as it was measured
//!
//! A `parallel` channel's fan-out starts its members one after another in one process, and the set
//! it then waits for is whatever threads exist. Two things stop that loop early, and both leave a
//! PREFIX of the declared members behind:
//!
//! - **The process is killed.** Under `working_tree: exclusive` the round opens inside `nxc session
//!   ended`, which the sidecar runs under a 30-second timeout. A start costs 6 s on a quiet machine
//!   and 20 s on a busy one, so three of them do not fit. Measured: `spawnSync nxc ETIMEDOUT` in the
//!   verifier's log at 06:21:08, code-quality's slot at 06:20:39, test-quality's at 06:20:59,
//!   integrity's never. Nothing records an error, which is why a load of 2.6 was enough.
//! - **A start fails.** The `?` in the loop abandoned every later member: `starting integrity failed
//!   ... database is locked` left two of three.
//!
//! Whichever it was, the prefix answered, read as a settled set, and the round consolidated.
//!
//! The kill is staged here by a worker that panics inside the loop, caught around the call: what
//! that leaves behind is what SIGKILL leaves, a store with everything written so far and nothing
//! written after it. The engine handle carries every commission and reply; `orchestration::tick`
//! carries the tick, which is what the background service runs.

use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::NxfError;
use nexus_chat::model::ThreadRoot;
use nexus_chat::orchestration::{self, Caller, Ctx, TickReceipt, TickRequest};
use nexus_chat::precondition::PreconditionOutcome;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{ReplyThreadRequest, SendToReceipt, SendToRefs, SendToRequest};
use nexus_chat::timer::{Timer, TimerHandle};
use nexus_chat::worker::{
    DryWorker, TriggerError, TriggerRequest, TriggerResult, Worker, WorkerConfig,
};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

/// The round is commissioned.
const T0: &str = "2026-11-05T06:20:00Z";
/// Inside the five minutes in which a fan-out may still be running in another process.
const T1: &str = "2026-11-05T06:21:00Z";
/// Past them.
const T6: &str = "2026-11-05T06:26:00Z";
const T7: &str = "2026-11-05T06:27:00Z";

const MEMBERS: [&str; 3] = ["code-quality", "test-quality", "integrity"];

const CHANNELS: &str = "- name: review\n  members: [code-quality, test-quality, integrity]\n  \
                        timeout: 1h\n  working_tree: exclusive\n  preconditions:\n    \
                        - name: gate\n      run: the gate\n\
                        - name: judged\n  members: [code-quality, test-quality, integrity]\n  \
                        on_complete: summarize\n  summary_prompt: Fold them.\n  timeout: 1h\n  preconditions:\n    \
                        - name: gate\n      run: the gate\n\
                        - name: subset\n  members: [code-quality, test-quality, integrity]\n  \
                        expects: [code-quality, test-quality]\n  timeout: 1h\n\
                        - name: coding\n  members: []\n  timeout: 6h\n  steps:\n    \
                        - id: build\n      target: coder\n      next: judge\n    \
                        - id: judge\n      target: review\n      next: finish\n    \
                        - id: finish\n      target: finisher\n";

// ---- harness ------------------------------------------------------------------------------------

/// A worker with the things that went wrong in the field, each switched on by a test.
struct Field {
    log: PathBuf,
    working_copy: PathBuf,
    running: Mutex<HashSet<String>>,
    looks: AtomicBool,
    /// Panic at this hurdle (1-based, counted over the fan-out) — before that member's slot exists.
    kill_at_gate: AtomicI64,
    gates_asked: AtomicI64,
    /// These roles fail to start the way `nxf prime` under load does.
    prime_times_out: Mutex<HashSet<String>>,
    /// The declared hurdle says no.
    gate_refuses: AtomicBool,
}

impl Worker for Field {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        if self
            .prime_times_out
            .lock()
            .unwrap()
            .contains(&req.role.handle)
        {
            return Err(TriggerError::Failed(NxfError::io(
                "`nxf prime --db db.sqlite` exceeded the 30s wall-clock timeout and was killed",
            )));
        }
        self.running
            .lock()
            .unwrap()
            .insert(req.internal_session.clone());
        DryWorker {
            log: Some(self.log.clone()),
        }
        .trigger(req)
    }
    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running.lock().unwrap().contains(internal_session)
    }
    fn answers_liveness(&self) -> bool {
        self.looks.load(Ordering::SeqCst)
    }
    fn working_copy(&self) -> Option<PathBuf> {
        Some(self.working_copy.clone())
    }
    fn stops_sessions(&self) -> bool {
        true
    }
    fn resumes_sessions(&self) -> bool {
        true
    }
    fn asks_sessions(&self) -> bool {
        true
    }
    fn run_precondition(&self, _command: &str) -> PreconditionOutcome {
        let asked = self.gates_asked.fetch_add(1, Ordering::SeqCst) + 1;
        if asked == self.kill_at_gate.load(Ordering::SeqCst) {
            panic!("the sidecar's timeout killed this process");
        }
        PreconditionOutcome::Ran {
            status: Some(i32::from(self.gate_refuses.load(Ordering::SeqCst))),
            stdout: String::new(),
            stderr: "the gate says no".into(),
        }
    }
}

/// Records every look it is asked to arm: an armed look is the only thing that brings a killed
/// fan-out back, because nothing else will ever call again.
#[derive(Default)]
struct RecordingTimer {
    looks: Mutex<Vec<(String, String, String)>>,
}

impl Timer for RecordingTimer {
    fn schedule(
        &self,
        thread_id: &str,
        _deadline: &str,
        _command: &str,
    ) -> nexus_chat::error::Result<TimerHandle> {
        Ok(TimerHandle(format!("recorded:{thread_id}")))
    }
    fn cancel(&self, _handle: &TimerHandle) -> nexus_chat::error::Result<()> {
        Ok(())
    }
    fn schedule_stranded_look(
        &self,
        slot: &str,
        thread: &str,
        deadline: &str,
    ) -> nexus_chat::error::Result<TimerHandle> {
        self.looks.lock().unwrap().push((
            slot.to_string(),
            thread.to_string(),
            deadline.to_string(),
        ));
        Ok(TimerHandle(format!("recorded:stranded-look-{slot}")))
    }
}

struct Bench {
    tmp: TempDir,
    worker: Arc<Field>,
    timer: Arc<RecordingTimer>,
    origin: String,
}

impl Bench {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        setup(tmp.path(), &chat_config()).expect("seed chat workspace");
        let roles = tmp.path().join(".nxs-personas");
        std::fs::create_dir_all(&roles).unwrap();
        for handle in MEMBERS.iter().chain(&["coder", "finisher"]) {
            std::fs::write(
                roles.join(format!("{handle}.yaml")),
                format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
            )
            .unwrap();
        }
        std::fs::write(roles.join("channels.yaml"), CHANNELS).unwrap();
        let worker = Arc::new(Field {
            log: tmp.path().join("dry.log"),
            working_copy: tmp.path().to_path_buf(),
            running: Mutex::new(HashSet::new()),
            looks: AtomicBool::new(true),
            kill_at_gate: AtomicI64::new(0),
            gates_asked: AtomicI64::new(0),
            prime_times_out: Mutex::new(HashSet::new()),
            gate_refuses: AtomicBool::new(false),
        });
        let mut b = Bench {
            tmp,
            worker,
            timer: Arc::new(RecordingTimer::default()),
            origin: String::new(),
        };
        b.origin = b.engine().origin();
        b
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn engine(&self) -> Engine {
        Engine::open_with(
            None,
            self.root(),
            EngineConfig {
                worker: WorkerConfig::Custom(self.worker.clone()),
                timer: nexus_chat::timer::TimerConfig::Disabled,
                ..EngineConfig::default()
            },
        )
        .expect("open engine")
    }

    fn store(&self) -> ChatStore {
        Workspace::resolve(None, self.root())
            .expect("resolve workspace")
            .open_chat_store()
            .expect("open chat store")
    }

    fn send(&self, now: &str, to: &str) -> nexus_chat::error::Result<SendToReceipt> {
        self.engine().send_to(
            Caller {
                session: None,
                actor: Some("carsten"),
                now: Some(now),
            },
            SendToRequest {
                machine: None,
                to,
                body: "judge this change",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
    }

    /// `send --to <channel>` in a process that dies at the stage the test switched on. Returns the
    /// round's thread, read from the store, because the process never got to return it.
    fn send_and_die(&self, now: &str, to: &str) -> String {
        let died = catch_unwind(AssertUnwindSafe(|| self.send(now, to)));
        assert!(died.is_err(), "the process was to die inside the fan-out");
        self.worker.kill_at_gate.store(0, Ordering::SeqCst);
        self.store()
            .threads_opened_by(&format!("{}/carsten", self.origin))
            .unwrap()
            .into_iter()
            .next_back()
            .expect("the round's channel thread was written before the fan-out")
    }

    /// A member's answer, from the session the fan-out started — whose process ends with it.
    fn answer(&self, now: &str, member: &str, slot: &str) -> nexus_chat::error::Result<()> {
        let session = self.session_of(member);
        self.worker.running.lock().unwrap().remove(&session);
        self.engine()
            .reply_thread(
                Caller {
                    session: Some(&session),
                    actor: Some(member),
                    now: Some(now),
                },
                ReplyThreadRequest {
                    machine: None,
                    thread: slot,
                    body: "looks fine to me",
                    escalate: false,
                    needs_rework: false,
                    accept: false,
                },
            )
            .map(|_| ())
    }

    /// Run `body` under the context the service builds — the only way to see this test's timer,
    /// because an engine handle carries no timer of a host's own.
    fn with_ctx<R>(&self, now: &str, body: impl FnOnce(&Ctx, &mut ChatStore) -> R) -> R {
        let ws = Workspace::resolve(None, self.root()).expect("resolve workspace");
        let db_path = ws.db_path_str().expect("db path");
        let mut store = ws.open_chat_store().expect("open chat store");
        let defs =
            nexus_chat::definitions::Definitions::resolve(self.root()).expect("declarations");
        let worker = self.worker.clone();
        let ctx = Ctx {
            now,
            origin: &self.origin,
            actor: "carsten",
            session: None,
            hop: 0,
            defs: &defs,
            worker: worker.as_ref(),
            timer: self.timer.as_ref(),
            namer: &nexus_chat::naming::DryNamer,
            db_path: &db_path,
            project_claude_md: None,
            module_primes: None,
            machines: None,
            peers: None,
            liveness: Default::default(),
        };
        body(&ctx, &mut store)
    }

    fn tick(&self, now: &str, thread: &str) -> TickReceipt {
        self.with_ctx(now, |ctx, store| {
            orchestration::tick(ctx, store, TickRequest { thread_id: thread }).expect("tick")
        })
    }

    fn triggers(&self) -> Vec<String> {
        std::fs::read_to_string(self.root().join("dry.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("trigger role="))
            .map(str::to_string)
            .collect()
    }

    fn started(&self, handle: &str) -> usize {
        self.triggers()
            .iter()
            .filter(|l| l.starts_with(&format!("trigger role={handle} ")))
            .count()
    }

    fn a_synthesis_ran(&self) -> bool {
        self.triggers().iter().any(|l| l.contains("__synth__"))
    }

    fn session_of(&self, handle: &str) -> String {
        let line = self
            .triggers()
            .into_iter()
            .rfind(|l| l.starts_with(&format!("trigger role={handle} ")))
            .unwrap_or_else(|| panic!("no trigger for {handle} in {:?}", self.triggers()));
        line.split(" session=")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .expect("session field")
            .to_string()
    }

    /// The member's slot under `round`.
    fn slot_of(&self, round: &str, member: &str) -> String {
        let store = self.store();
        let wanted = format!("{}/{member}", self.origin);
        store
            .supervised_children(round)
            .unwrap()
            .into_iter()
            .rfind(|slot| {
                store
                    .thread_quorum(slot, T0)
                    .unwrap()
                    .is_some_and(|q| q.expects.first() == Some(&wanted))
            })
            .unwrap_or_else(|| panic!("no slot for {member} under {round}"))
    }

    fn slots(&self, round: &str) -> usize {
        self.store().supervised_children(round).unwrap().len()
    }

    fn supervisor(&self) -> String {
        format!("{}/__channel__", self.origin)
    }

    /// The round has not closed: its thread still owes the supervisor's answer.
    fn still_open(&self, round: &str) -> bool {
        self.store()
            .thread_quorum(round, T0)
            .unwrap()
            .expect("the round exists")
            .expects
            == [self.supervisor()]
    }

    /// What the round's own thread carries from the engine after it closed: `(kind, body)`.
    fn closing_message(&self, round: &str) -> (String, String) {
        let row = self
            .store()
            .acting_messages_in_thread(round)
            .unwrap()
            .into_iter()
            .rfind(|m| m.sender.contains("__delivered__"))
            .expect("the round closed with a message of its own");
        (row.kind, row.body)
    }

    /// Everything anybody was told about `thread`'s round, so far as a tick's receipt carries it.
    fn says_will_not_answer(receipt: &TickReceipt) -> bool {
        receipt
            .warnings
            .iter()
            .any(|f| f.detail.contains("will not answer"))
    }
}

/// Every member answers, in the order declared.
fn everyone_answers(b: &Bench, now: &str, round: &str) {
    for member in MEMBERS {
        let slot = b.slot_of(round, member);
        b.answer(now, member, &slot).expect("the answer is posted");
    }
}

// ---- the three shapes on record -----------------------------------------------------------------

/// **Shape 1: the empty fan-out** (dqem, 2026-11-03). The process died after the round's only slot
/// was opened and before its commission was written: a slot with a deadline and nothing else, no
/// reviewer thread, and an hour of `not_due` up to a channel timeout that said "delivered".
#[test]
fn an_empty_fan_out_gets_its_members_rather_than_a_delivery_at_its_timeout() {
    let b = Bench::new();
    b.worker.kill_at_gate.store(1, Ordering::SeqCst);
    let round = b.send_and_die(T0, "review");
    let opener = format!("{}/__channel__", b.origin);
    b.store().open_thread(
        "m-01EMPTYREVIEWSLOT0000000000",
        &ThreadRoot {
            origin: b.origin.clone(),
            channel_id: "decl:review".into(),
            opener: opener.clone(),
            created: T0.into(),
            parent: Some(round.clone()),
        },
        &opener,
    );
    b.store().set_deadline(
        "m-01EMPTYREVIEWSLOT0000000000",
        "2026-11-05T07:20:00Z",
        &opener,
    );

    b.tick(T6, &round);

    for member in MEMBERS {
        assert_eq!(
            b.started(member),
            1,
            "{member} was asked: {:?}",
            b.triggers()
        );
    }
    assert!(b.still_open(&round), "nobody has answered yet");

    // At what used to be the channel's timeout, with every member at work, nothing is delivered.
    let late = b.tick("2026-11-05T07:21:00Z", &round);
    assert!(b.still_open(&round), "nothing was reviewed yet: {late:?}");
    everyone_answers(&b, "2026-11-05T07:22:00Z", &round);
    assert!(!b.still_open(&round), "with all three it closes");
    assert_eq!(b.closing_message(&round).0, "info");
}

/// A round the process died in before ANY slot: nothing under the channel thread at all. Read as "a
/// thread with no set", its own window consolidated a round nobody was ever asked in.
#[test]
fn a_round_with_no_slot_at_all_is_not_consolidated_by_its_window() {
    let b = Bench::new();
    b.worker.kill_at_gate.store(1, Ordering::SeqCst);
    let round = b.send_and_die(T0, "review");
    assert_eq!(b.slots(&round), 0);

    let receipt = b.tick("2026-11-05T07:21:00Z", &round);

    assert!(b.still_open(&round), "nobody was asked: {receipt:?}");
    for member in MEMBERS {
        assert_eq!(b.started(member), 1, "{member} was asked");
    }
}

/// **Shape 2: one of three** (planning pass 2, 2026-11-04). The one member the process got to
/// answered, and the synthesis ran at once with two members never started.
#[test]
fn one_answer_of_three_does_not_close_the_round() {
    let b = Bench::new();
    b.worker.kill_at_gate.store(2, Ordering::SeqCst);
    let round = b.send_and_die(T0, "judged");
    assert_eq!(
        b.slots(&round),
        1,
        "the process died after the first member"
    );
    let slot = b.slot_of(&round, "code-quality");

    b.answer(T1, "code-quality", &slot).expect("posted");

    assert!(b.still_open(&round), "two members were never asked");
    assert!(!b.a_synthesis_ran(), "{:?}", b.triggers());
    assert_eq!(
        b.started("test-quality"),
        0,
        "a fan-out may still be running"
    );

    b.tick(T6, &round);

    assert_eq!(b.started("test-quality"), 1);
    assert_eq!(b.started("integrity"), 1);
    assert!(b.still_open(&round));
    assert!(!b.a_synthesis_ran());

    for member in ["test-quality", "integrity"] {
        let slot = b.slot_of(&round, member);
        b.answer(T7, member, &slot).expect("posted");
    }
    assert!(b.a_synthesis_ran(), "with all three the synthesis runs");
}

/// **Shape 3: two of three** (r426 coding run, 2026-11-05; d1ey, 2026-10-30). The verdict that came
/// of it said "incomplete" and still returned pass, and the change reached the finisher.
#[test]
fn two_answers_of_three_do_not_synthesise_a_verdict() {
    let b = Bench::new();
    b.worker.kill_at_gate.store(3, Ordering::SeqCst);
    let round = b.send_and_die(T0, "judged");
    assert_eq!(b.slots(&round), 2);

    for member in ["code-quality", "test-quality"] {
        let slot = b.slot_of(&round, member);
        b.answer(T1, member, &slot).expect("posted");
    }

    assert!(!b.a_synthesis_ran(), "{:?}", b.triggers());
    assert!(b.still_open(&round), "integrity was never asked");

    b.tick(T6, &round);
    assert_eq!(b.started("integrity"), 1);
    assert!(!b.a_synthesis_ran());

    let slot = b.slot_of(&round, "integrity");
    b.answer(T7, "integrity", &slot).expect("posted");
    assert!(b.a_synthesis_ran());
}

/// The kill of the measured run itself, one stage later: test-quality's slot and task exist, its
/// start never ran, and integrity was never reached.
#[test]
fn a_process_killed_inside_a_members_start_still_leaves_the_rest_to_be_asked() {
    let b = Bench::new();
    b.worker
        .prime_times_out
        .lock()
        .unwrap()
        .insert("test-quality".into());
    b.worker.kill_at_gate.store(3, Ordering::SeqCst);
    let round = b.send_and_die(T0, "review");
    assert_eq!(b.slots(&round), 2);
    b.worker.prime_times_out.lock().unwrap().clear();

    b.tick(T6, &round);

    assert_eq!(b.started("integrity"), 1, "the one never reached is asked");
    assert_eq!(
        b.slots(&round),
        3,
        "test-quality's own slot is not opened twice"
    );
    assert!(b.still_open(&round));
}

// ---- nothing is lost in the meantime --------------------------------------------------------------

/// The fan-out arms its own way back BEFORE the first member, because a killed process says nothing
/// and on an empty round no later event exists that could.
#[test]
fn a_fan_out_arms_the_look_that_brings_it_back_before_it_starts_anyone() {
    let b = Bench::new();
    b.worker.kill_at_gate.store(1, Ordering::SeqCst);
    let died = catch_unwind(AssertUnwindSafe(|| {
        b.with_ctx(T0, |ctx, store| {
            nexus_chat::surface::send_to(
                ctx,
                store,
                SendToRequest {
                    machine: None,
                    to: "review",
                    body: "judge this change",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
        })
    }));
    assert!(
        died.is_err(),
        "the process was to die before the first member"
    );
    let round = b
        .store()
        .threads_opened_by(&format!("{}/carsten", b.origin))
        .unwrap()
        .into_iter()
        .next_back()
        .expect("the round's channel thread was written before the fan-out");

    let looks = b.timer.looks.lock().unwrap().clone();
    assert!(
        looks
            .iter()
            .any(|(_, thread, at)| *thread == round && at.as_str() >= "2026-11-05T06:25"),
        "a look at the round, five minutes out: {looks:?}"
    );
}

/// Another process may still be in the middle of the same fan-out. Five minutes is what the engine
/// waits out for its own unfinished business (`STALLED_CHAIN_QUIET_SECS`); inside them nobody else
/// opens anything, and the look comes back when they are over.
#[test]
fn inside_the_quiet_nobody_opens_a_member_a_running_fan_out_may_still_open() {
    let b = Bench::new();
    b.worker.kill_at_gate.store(2, Ordering::SeqCst);
    let round = b.send_and_die(T0, "review");

    let receipt = b.tick(T1, &round);

    assert_eq!(b.started("test-quality"), 0, "{receipt:?}");
    assert_eq!(b.slots(&round), 1);
}

/// A completed round asks for nobody. The look a healthy fan-out armed fires and finds nothing.
#[test]
fn a_whole_round_opens_nobody_a_second_time() {
    let b = Bench::new();
    let round = b.send(T0, "review").expect("sent").thread_id;
    assert_eq!(b.slots(&round), 3);

    b.tick(T6, &round);

    for member in MEMBERS {
        assert_eq!(b.started(member), 1, "{member} started once");
    }
    assert_eq!(b.slots(&round), 3);
}

/// `expects: <subset>` narrows who must answer: the member left out is not missing.
#[test]
fn a_declared_subset_is_the_whole_of_its_round() {
    let b = Bench::new();
    let round = b.send(T0, "subset").expect("sent").thread_id;
    assert_eq!(b.slots(&round), 2);

    for member in ["code-quality", "test-quality"] {
        let slot = b.slot_of(&round, member);
        b.answer(T1, member, &slot).expect("posted");
    }

    assert!(!b.still_open(&round), "the two it expects are all of it");
    assert_eq!(b.started("integrity"), 0);
}

// ---- a start that fails ---------------------------------------------------------------------------

/// A failed start no longer costs the members behind it. `database is locked` on the third member
/// left two of three; on the first it left an empty round.
#[test]
fn a_member_whose_start_fails_does_not_cost_the_members_behind_it() {
    let b = Bench::new();
    b.worker
        .prime_times_out
        .lock()
        .unwrap()
        .insert("code-quality".into());

    let sent = b.send(T0, "review");

    let sent = sent.expect("the round is running");
    assert_eq!(b.started("test-quality"), 1);
    assert_eq!(b.started("integrity"), 1);
    assert_eq!(b.slots(&sent.thread_id), 3);
    assert!(
        sent.warnings
            .iter()
            .any(|f| f.detail.contains("code-quality") && f.detail.contains("tried again")),
        "the caller is told: {:?}",
        sent.warnings
    );
}

/// **While the failure is load, it is not "will not answer".** The measured run spent its budget in
/// nine minutes of a 30-second `nxf prime` timeout and recorded a reviewer as stranded, and the same
/// command took 0.6 s when the load had passed. Forty minutes of it spend nothing here.
#[test]
fn a_start_failing_on_load_is_retried_and_nobody_is_told_it_will_not_answer() {
    let b = Bench::new();
    b.worker
        .prime_times_out
        .lock()
        .unwrap()
        .insert("test-quality".into());
    let round = b.send(T0, "review").expect("sent").thread_id;
    let slot = b.slot_of(&round, "test-quality");
    for member in ["code-quality", "integrity"] {
        let answered = b.slot_of(&round, member);
        b.answer(T1, member, &answered).expect("posted");
    }

    for minute in 22..60 {
        let receipt = b.tick(&format!("2026-11-05T06:{minute}:00Z"), &round);
        assert!(
            !Bench::says_will_not_answer(&receipt),
            "minute {minute}: {:?}",
            receipt.warnings
        );
    }
    let escalated = b
        .store()
        .acting_messages_in_thread(&slot)
        .unwrap()
        .into_iter()
        .any(|m| m.kind == "escalation");
    assert!(!escalated, "nobody escalated on its behalf");
    assert!(b.still_open(&round));

    // The load passes, and the next look starts it.
    b.worker.prime_times_out.lock().unwrap().clear();
    b.tick("2026-11-05T07:00:00Z", &round);
    b.tick("2026-11-05T07:02:00Z", &round);
    assert_eq!(b.started("test-quality"), 1, "started when it could be");
}

/// **A member that finally cannot run stops the round**, past the wall-clock ceiling, and the reason
/// says what happened rather than that a session declined to answer.
#[test]
fn a_member_that_never_could_start_stops_the_round_and_says_why() {
    let b = Bench::new();
    b.worker
        .prime_times_out
        .lock()
        .unwrap()
        .insert("test-quality".into());
    let round = b.send(T0, "review").expect("sent").thread_id;
    for member in ["code-quality", "integrity"] {
        let slot = b.slot_of(&round, member);
        b.answer(T1, member, &slot).expect("posted");
    }
    assert!(b.still_open(&round));

    let mut said = Vec::new();
    for hour in 7..14 {
        for minute in [0, 15, 30, 45] {
            let at = format!("2026-11-05T{hour:02}:{minute:02}:00Z");
            let receipt = b.tick(&at, &round);
            said.extend(receipt.warnings.into_iter().map(|f| f.detail));
        }
    }

    assert!(!b.still_open(&round), "the round stopped: {said:#?}");
    let (kind, body) = b.closing_message(&round);
    assert_eq!(kind, "escalation", "it does not pass: {body}");
    assert!(
        !body.contains("delivered"),
        "it does not read as delivered: {body}"
    );
    assert!(
        said.iter().any(|d| d.contains("could not be started")),
        "the reason names the start: {said:#?}"
    );
    assert!(
        !said.iter().any(|d| d.contains("will not answer")),
        "{said:#?}"
    );
}

/// A member that can never be opened, not for load: its hurdle says no. The round stops once the
/// others are through, and the commissioner reads why.
#[test]
fn a_member_refused_at_its_repair_stops_the_round_for_the_commissioner() {
    let b = Bench::new();
    b.worker.kill_at_gate.store(2, Ordering::SeqCst);
    let round = b.send_and_die(T0, "judged");
    let slot = b.slot_of(&round, "code-quality");
    b.answer(T1, "code-quality", &slot).expect("posted");
    b.worker.gate_refuses.store(true, Ordering::SeqCst);

    let receipt = b.tick(T6, &round);

    assert!(!b.still_open(&round), "{receipt:?}");
    assert!(!b.a_synthesis_ran(), "no verdict is made of one answer");
    let (kind, body) = b.closing_message(&round);
    assert_eq!(kind, "escalation", "{body}");
    assert!(!body.contains("delivered"), "{body}");
    let told = receipt.delivered.unwrap_or_default();
    assert!(
        told.contains("did not complete") && told.contains("gate"),
        "the commissioner reads why: {told}"
    );
    assert_eq!(b.tick(T7, &round).reason, "already_handled");
}

/// A round that has to stop does not run out from under the members it has: it stops when they are
/// through, as any round closes. A shared channel, because on an exclusive one the gate for a session
/// still writing holds it anyway.
#[test]
fn a_round_that_must_stop_waits_for_the_members_it_has() {
    let b = Bench::new();
    b.worker.kill_at_gate.store(2, Ordering::SeqCst);
    let round = b.send_and_die(T0, "judged");
    b.worker.gate_refuses.store(true, Ordering::SeqCst);

    let receipt = b.tick(T6, &round);

    assert!(
        b.still_open(&round),
        "code-quality is still at work: {receipt:?}"
    );

    let slot = b.slot_of(&round, "code-quality");
    b.answer(T7, "code-quality", &slot).expect("posted");

    assert!(!b.still_open(&round), "with its answer the round stops");
    assert_eq!(b.closing_message(&round).0, "escalation");
}

/// A window that strikes on a member who said nothing, on a worker that cannot tell whether it runs.
/// The round may end, because nothing else will end it; it may not end as a pass, and it runs no
/// synthesis over the answers that came.
#[test]
fn a_member_that_lapsed_makes_the_round_an_escalation_not_a_pass() {
    let b = Bench::new();
    b.worker.looks.store(false, Ordering::SeqCst);
    let round = b.send(T0, "judged").expect("sent").thread_id;
    for member in ["code-quality", "test-quality"] {
        let slot = b.slot_of(&round, member);
        b.answer(T1, member, &slot).expect("posted");
    }
    assert!(b.still_open(&round));

    let receipt = b.tick("2026-11-05T07:30:00Z", &round);

    assert!(!b.still_open(&round), "the window ended it: {receipt:?}");
    assert!(!b.a_synthesis_ran(), "{:?}", b.triggers());
    let (kind, body) = b.closing_message(&round);
    assert_eq!(kind, "escalation", "{body}");
    assert!(
        receipt
            .delivered
            .as_deref()
            .is_some_and(|d| d.contains("integrity")),
        "the commissioner is told who is missing: {:?}",
        receipt.delivered
    );
}

// ---- the chain above the round --------------------------------------------------------------------

/// **Defect 2 of the first report: the chain went on above a round that reviewed nothing.** The step
/// the round stands on lapsed at the outer window; the flow took that as settled and moved past it.
#[test]
fn the_flow_above_does_not_move_past_a_round_that_has_not_closed() {
    let b = Bench::new();
    let root = b.send(T0, "coding").expect("sent").thread_id;
    let coder = b.session_of("coder");
    let build = b.slot_of(&root, "coder");
    b.worker.running.lock().unwrap().remove(&coder);
    b.worker.kill_at_gate.store(1, Ordering::SeqCst);
    let died = catch_unwind(AssertUnwindSafe(|| b.answer(T1, "coder", &build)));
    assert!(died.is_err(), "the process dies opening the round");
    b.worker.kill_at_gate.store(0, Ordering::SeqCst);

    let round = b
        .store()
        .supervised_children(&root)
        .unwrap()
        .into_iter()
        .rfind(|t| *t != build)
        .expect("the round's thread was written before the fan-out");

    // Past the round's window and the outer flow's: on the outer thread first, as the service may.
    let late = b.tick("2026-11-05T07:30:00Z", &root);
    assert_eq!(b.started("finisher"), 0, "{late:?}");

    b.tick("2026-11-05T07:31:00Z", &round);
    for member in MEMBERS {
        assert_eq!(
            b.started(member),
            1,
            "{member} was asked: {:?}",
            b.triggers()
        );
    }
    assert_eq!(b.started("finisher"), 0);

    everyone_answers(&b, "2026-11-05T07:40:00Z", &round);
    assert_eq!(b.started("finisher"), 1, "now the flow goes on");
}

/// The run stops at the round: the commissioner of the whole run hears, and the finisher never runs.
#[test]
fn a_round_that_stopped_stops_the_run_above_it() {
    let b = Bench::new();
    let root = b.send(T0, "coding").expect("sent").thread_id;
    let coder = b.session_of("coder");
    let build = b.slot_of(&root, "coder");
    b.worker.running.lock().unwrap().remove(&coder);
    b.worker.gate_refuses.store(false, Ordering::SeqCst);
    b.worker.kill_at_gate.store(2, Ordering::SeqCst);
    let died = catch_unwind(AssertUnwindSafe(|| b.answer(T1, "coder", &build)));
    assert!(died.is_err());
    b.worker.kill_at_gate.store(0, Ordering::SeqCst);
    let round = b
        .store()
        .supervised_children(&root)
        .unwrap()
        .into_iter()
        .rfind(|t| *t != build)
        .unwrap();
    let slot = b.slot_of(&round, "code-quality");
    b.answer("2026-11-05T06:22:00Z", "code-quality", &slot)
        .expect("posted");
    b.worker.gate_refuses.store(true, Ordering::SeqCst);

    b.tick("2026-11-05T06:28:00Z", &round);

    assert!(!b.still_open(&round));
    assert_eq!(b.started("finisher"), 0, "{:?}", b.triggers());
    let (kind, _) = b.closing_message(&root);
    assert_eq!(
        kind, "escalation",
        "the run reads as stopped, all the way up"
    );
}
