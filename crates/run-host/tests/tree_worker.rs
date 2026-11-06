//! The tree worker skeleton (§6.6, §8.3) over `ScriptedTree`: one operation in flight, started on the first
//! `begin`; the ledger written before each spawn with owner and epoch; a recorded child killed only when its
//! owner's lease is lost or is this host's own; a per-operation deadline; and the loop under `Host::tick`.
//!
//! Real children here are `sleep`s, and every wait is "until this is observably so, or fail after a long time":
//! nothing depends on timing or CPU counters (k6nf).

mod common;

use common::tree::until;

use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nexus_run_contracts::{
    Decision, EventBody, GitReason, Outcome, ParkOutcome, ReasonCode, RunId, RunStore, Secs,
    StoreError, Timestamp, TreeRefusal,
};
use nexus_run_host::bridge;
use nexus_run_host::ports::Clock;
use nexus_run_host::session_stub::{ProcessGuard, SystemProcesses};
use nexus_run_host::test_support::scripted_backend::{Cue, SessionInfo};
use nexus_run_host::test_support::scripted_tree::{ScriptedTree, TreeStep};
use nexus_run_host::test_support::{replies_pass, Rig, SimClock, StoreProbe, CLOCK_START};
use nexus_run_host::{
    InFlight, LeaseView, LedgerEntry, TreeBinding, TreeLedger, TreeOp, TreeResult, WorkTree,
};
use nexus_run_store_sqlite::Lease;
use ulid::Ulid;

const OWNER: &str = "h-ours";
const OTHER: &str = "h-other";

/// A lease a test sets by hand.
#[derive(Default)]
struct HandLeases(Mutex<Option<Lease>>);

impl HandLeases {
    fn set(&self, owner: &str, epoch: u64, expires_at: i64) {
        *self.0.lock().unwrap() = Some(Lease {
            owner_id: owner.to_owned(),
            epoch,
            expires_at: Timestamp::from_unix_millis(expires_at),
        });
    }
}

impl LeaseView for HandLeases {
    fn lease(&self, _run: &RunId) -> Result<Option<Lease>, StoreError> {
        Ok(self.0.lock().unwrap().clone())
    }
}

struct Bench {
    _dir: tempfile::TempDir,
    ledger: TreeLedger,
    clock: Arc<SimClock>,
    leases: Arc<HandLeases>,
    run: RunId,
}

impl Bench {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let ledger = TreeLedger::new(dir.path(), "ws");
        let leases = Arc::new(HandLeases::default());
        leases.set(OWNER, 7, CLOCK_START + 30_000);
        Self {
            _dir: dir,
            ledger,
            clock: Arc::new(SimClock::at(CLOCK_START)),
            leases,
            run: RunId::new(Ulid::new()),
        }
    }

    fn bind(&self, tree: &ScriptedTree, clock: Arc<dyn Clock>) {
        tree.bind(TreeBinding {
            owner_id: OWNER.to_owned(),
            leases: self.leases.clone(),
            clock,
        });
    }

    fn tree(&self) -> ScriptedTree {
        let tree = ScriptedTree::new(self.ledger.clone());
        self.bind(&tree, self.clock.clone());
        tree
    }

    fn prepare(&self) -> TreeOp {
        TreeOp::Prepare {
            run: self.run.clone(),
            dir: self._dir.path().join("v2"),
        }
    }

    fn entry(&self, owner: &str, epoch: u64, child: Option<(u32, String)>) -> LedgerEntry {
        let (pid, start_time) = child.unzip();
        LedgerEntry {
            run: self.run.clone(),
            op: "prepare".to_owned(),
            step: "fetch".to_owned(),
            pid,
            start_time,
            began_at: self.clock.now(),
            owner_id: owner.to_owned(),
            epoch,
            lock_existed_before: false,
            spawning: false,
            locks_before: Vec::new(),
        }
    }
}

fn sleeper() -> TreeStep {
    TreeStep::Child {
        program: "sleep".to_owned(),
        args: vec!["600".to_owned()],
    }
}

/// A `sleep` in its own process group, standing for a git child a dead host left behind.
fn orphan() -> (Child, u32, String) {
    use std::os::unix::process::CommandExt;
    let child = Command::new("sleep")
        .arg("600")
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    let start = SystemProcesses.start_time(pid).expect("the child is there");
    (child, pid, start)
}

fn assert_dies(mut child: Child) {
    // The test is its parent, so it reaps; the guard saw it killed by then.
    until("the child died", || {
        child
            .try_wait()
            .map(|status| status.is_some())
            .unwrap_or(true)
    });
}

#[test]
fn begin_on_a_busy_worker_answers_in_flight_and_poll_hands_a_result_back_once() {
    let bench = Bench::new();
    let tree = bench.tree();
    tree.script(vec![TreeStep::Delay(Duration::from_secs(10))]);
    tree.begin(bench.prepare()).unwrap();
    assert_eq!(tree.begin(bench.prepare()), Err(InFlight));
    assert_eq!(tree.poll(), None);
    let mut results = Vec::new();
    until("one result", || {
        bench.clock.advance(1_000);
        results.extend(tree.poll());
        !results.is_empty()
    });
    assert!(matches!(results[0].1, TreeResult::Prepared { .. }));
    assert_eq!(tree.poll(), None, "a result is handed back once");
    tree.begin(bench.prepare()).expect("free again");
}

#[test]
fn the_worker_starts_on_the_first_begin_and_only_then() {
    let bench = Bench::new();
    let tree = bench.tree();
    assert_eq!(
        tree.workers_started(),
        0,
        "an idle workspace costs no thread"
    );
    tree.begin(bench.prepare()).unwrap();
    assert_eq!(tree.workers_started(), 1);
    until("the first result", || tree.poll().is_some());
    tree.begin(bench.prepare()).unwrap();
    until("the second result", || tree.poll().is_some());
    assert_eq!(tree.workers_started(), 1, "one worker per workspace");
}

#[test]
fn the_ledger_is_written_before_each_spawn_with_owner_epoch_and_pid_and_goes_with_the_operation() {
    let bench = Bench::new();
    let tree = bench.tree();
    tree.script(vec![sleeper()]);
    tree.begin(bench.prepare()).unwrap();
    let mut entry = None;
    until("the child is recorded", || {
        entry = tree.ledger().read().filter(|e| e.pid.is_some());
        entry.is_some()
    });
    let entry = entry.unwrap();
    assert_eq!(entry.owner_id, OWNER);
    assert_eq!(entry.epoch, 7);
    assert_eq!(entry.run, bench.run);
    assert_eq!(entry.op, "prepare");
    assert_eq!(entry.step, "child");
    assert!(!entry.lock_existed_before);
    let (pid, start) = (entry.pid.unwrap(), entry.start_time.clone().unwrap());
    assert!(
        SystemProcesses.is_alive(pid, &start),
        "the recorded child is the live one"
    );

    // `tree.in_flight` of status is the file with its pid checked.
    let shown = tree
        .ledger()
        .in_flight(&SystemProcesses)
        .expect("in flight");
    assert_eq!(shown.op, "prepare");
    assert_eq!(shown.step, "child");

    tree.cancel();
    until("the cancelled child is gone", || {
        !SystemProcesses.is_alive(pid, &start)
    });
    let mut result = None;
    until("the result", || {
        result = tree.poll();
        result.is_some()
    });
    assert!(matches!(
        result.unwrap().1,
        TreeResult::Refused {
            reason: TreeRefusal::Git {
                reason: GitReason::BoundExceeded
            },
            ..
        }
    ));
    assert_eq!(
        tree.ledger().read(),
        None,
        "the entry goes with the operation"
    );
}

#[test]
fn a_dead_child_in_the_ledger_reads_as_nothing_in_flight() {
    let bench = Bench::new();
    let (mut child, pid, start) = orphan();
    bench
        .ledger
        .write(&bench.entry(OTHER, 1, Some((pid, start))))
        .unwrap();
    assert!(bench.ledger.in_flight(&SystemProcesses).is_some());
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(bench.ledger.in_flight(&SystemProcesses), None);
}

#[test]
fn a_live_owners_child_answers_in_flight_and_is_left_alone() {
    let bench = Bench::new();
    let tree = bench.tree();
    bench.leases.set(OTHER, 3, CLOCK_START + 30_000);
    let (child, pid, start) = orphan();
    bench
        .ledger
        .write(&bench.entry(OTHER, 3, Some((pid, start.clone()))))
        .unwrap();
    assert_eq!(tree.begin(bench.prepare()), Err(InFlight));
    assert!(
        SystemProcesses.is_alive(pid, &start),
        "another host's child was killed"
    );
    SystemProcesses.kill_group(pid, &start);
    assert_dies(child);
}

#[test]
fn a_child_whose_owner_lost_the_lease_is_killed_and_the_operation_runs() {
    let bench = Bench::new();
    let tree = bench.tree();
    bench.leases.set(OWNER, 4, CLOCK_START + 30_000);
    let (child, pid, start) = orphan();
    bench
        .ledger
        .write(&bench.entry(OTHER, 3, Some((pid, start))))
        .unwrap();
    tree.begin(bench.prepare())
        .expect("the owner's lease moved on");
    assert_dies(child);
    until("the operation's result", || tree.poll().is_some());
}

#[test]
fn a_child_recorded_under_an_expired_lease_is_killed() {
    let bench = Bench::new();
    let tree = bench.tree();
    bench.leases.set(OTHER, 3, CLOCK_START - 1);
    let (child, pid, start) = orphan();
    bench
        .ledger
        .write(&bench.entry(OTHER, 3, Some((pid, start))))
        .unwrap();
    bench.leases.set(OTHER, 3, CLOCK_START - 1);
    // The bound lease is the other host's and expired: this host may not run under it, but it kills.
    assert!(tree.begin(bench.prepare()).is_ok());
    assert_dies(child);
}

#[test]
fn this_hosts_own_recorded_child_is_killed() {
    let bench = Bench::new();
    let tree = bench.tree();
    let (child, pid, start) = orphan();
    bench
        .ledger
        .write(&bench.entry(OWNER, 7, Some((pid, start))))
        .unwrap();
    tree.begin(bench.prepare()).unwrap();
    assert_dies(child);
}

/// The crash between a child's spawn and its record: the child wrote its own pid as its first act, and a restarted
/// host kills by that, so no orphan writes beside the operation it re-runs (§8.3).
#[test]
fn a_child_only_it_recorded_is_killed_by_its_own_record() {
    let bench = Bench::new();
    let tree = bench.tree();
    bench.leases.set(OWNER, 4, CLOCK_START + 30_000);
    let (child, pid, _) = orphan();
    let mut entry = bench.entry(OTHER, 3, None);
    entry.spawning = true;
    bench.ledger.write(&entry).unwrap();
    std::fs::write(bench.ledger.child_record(), pid.to_string()).unwrap();
    tree.begin(bench.prepare())
        .expect("the owner's lease moved on");
    assert_dies(child);
    until("the operation's result", || tree.poll().is_some());
    assert!(
        !bench.ledger.child_record().exists(),
        "the record goes with the entry"
    );
}

/// With no record yet the child may still be starting: `InFlight`, and the grace bounds it.
#[test]
fn an_announced_spawn_with_no_record_is_waited_on_for_the_grace_only() {
    let bench = Bench::new();
    let tree = bench.tree();
    bench.leases.set(OWNER, 4, CLOCK_START + 30_000);
    let mut entry = bench.entry(OTHER, 3, None);
    entry.spawning = true;
    bench.ledger.write(&entry).unwrap();
    assert_eq!(
        tree.begin(bench.prepare()),
        Err(InFlight),
        "it may be starting"
    );
    until("the grace ended", || tree.begin(bench.prepare()).is_ok());
    until("the operation's result", || tree.poll().is_some());
}

/// The recorded pid is the one the worker's own spawn wrote: the wrapper's `exec` keeps it.
#[test]
fn a_real_child_records_itself_with_the_pid_the_ledger_holds() {
    let bench = Bench::new();
    let tree = bench.tree();
    tree.script(vec![sleeper()]);
    tree.begin(bench.prepare()).unwrap();
    let mut entry = None;
    until("the child is recorded by the parent", || {
        entry = tree.ledger().read().filter(|e| e.pid.is_some());
        entry.is_some()
    });
    let entry = entry.unwrap();
    let record = bench.ledger.child_record();
    until("the child recorded itself", || {
        std::fs::read_to_string(&record).ok().as_deref()
            == Some(entry.pid.unwrap().to_string().as_str())
    });
    tree.cancel();
    until("the result", || tree.poll().is_some());
}

/// What `lock_existed_before` is for (§5): a child killed here may have left `index.lock` behind, and the call can
/// prove it created it, because the ledger says none was there before it. Left in place, the park that re-runs would
/// fail on it for ever.
#[test]
fn a_lock_the_killed_child_created_is_cleared_and_one_that_was_there_before_is_not() {
    for existed_before in [false, true] {
        let bench = Bench::new();
        let tree = bench.tree();
        bench.leases.set(OWNER, 4, CLOCK_START + 30_000);
        let git_dir = bench._dir.path().join("v2").join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        let lock = git_dir.join("index.lock");
        std::fs::write(&lock, b"").unwrap();
        let (child, pid, start) = orphan();
        let mut entry = bench.entry(OTHER, 3, Some((pid, start)));
        entry.lock_existed_before = existed_before;
        bench.ledger.write(&entry).unwrap();
        tree.begin(bench.prepare()).unwrap();
        assert_dies(child);
        assert_eq!(
            lock.exists(),
            existed_before,
            "lock_existed_before = {existed_before}"
        );
        until("the operation's result", || tree.poll().is_some());
    }
}

/// With no child recorded and none that recorded itself, nothing proves who made the lock.
#[test]
fn a_lock_with_no_recorded_child_behind_it_is_left_alone() {
    let bench = Bench::new();
    let tree = bench.tree();
    bench.leases.set(OWNER, 4, CLOCK_START + 30_000);
    let git_dir = bench._dir.path().join("v2").join(".git");
    std::fs::create_dir_all(&git_dir).unwrap();
    let lock = git_dir.join("index.lock");
    std::fs::write(&lock, b"").unwrap();
    bench.ledger.write(&bench.entry(OTHER, 3, None)).unwrap();
    tree.begin(bench.prepare()).unwrap();
    assert!(lock.exists(), "a lock nobody can answer for was removed");
    until("the operation's result", || tree.poll().is_some());
}

/// A recorded pid now held by a process with another start time is not the child: pid reuse never kills it.
#[test]
fn a_pid_with_another_start_time_is_not_signalled() {
    let bench = Bench::new();
    let tree = bench.tree();
    bench.leases.set(OWNER, 4, CLOCK_START + 30_000);
    let (child, pid, start) = orphan();
    bench
        .ledger
        .write(&bench.entry(OTHER, 3, Some((pid, "Thu Jan  1 00:00:00 1970".to_owned()))))
        .unwrap();
    tree.begin(bench.prepare()).unwrap();
    until("the operation's result", || tree.poll().is_some());
    assert!(
        SystemProcesses.is_alive(pid, &start),
        "a foreign process was killed"
    );
    SystemProcesses.kill_group(pid, &start);
    assert_dies(child);
}

#[test]
fn the_per_operation_deadline_fires() {
    let bench = Bench::new();
    let tree = ScriptedTree::with_deadline(bench.ledger.clone(), Duration::from_secs(60));
    bench.bind(&tree, bench.clock.clone());
    tree.script(vec![TreeStep::Delay(Duration::from_secs(3_600))]);
    tree.begin(bench.prepare()).unwrap();
    let mut result = None;
    until("the deadline's refusal", || {
        bench.clock.advance(1_000);
        result = tree.poll();
        result.is_some()
    });
    assert!(matches!(
        result.unwrap().1,
        TreeResult::Refused {
            reason: TreeRefusal::Git {
                reason: GitReason::BoundExceeded
            },
            ..
        }
    ));
}

#[test]
fn a_real_childs_bound_passing_with_the_deadline_kills_it() {
    let bench = Bench::new();
    let tree = ScriptedTree::with_deadline(bench.ledger.clone(), Duration::from_secs(60));
    bench.bind(&tree, bench.clock.clone());
    tree.script(vec![sleeper()]);
    tree.begin(bench.prepare()).unwrap();
    let mut entry = None;
    until("the child is recorded", || {
        entry = tree.ledger().read().filter(|e| e.pid.is_some());
        entry.is_some()
    });
    let entry = entry.unwrap();
    bench.clock.advance(61_000);
    let (pid, start) = (entry.pid.unwrap(), entry.start_time.unwrap());
    until("the child is gone", || {
        !SystemProcesses.is_alive(pid, &start)
    });
    until("the result", || tree.poll().is_some());
}

/// A worker that lost its run finishes after the new owner's worker wrote: that entry is not its to remove, or
/// the new owner's child could no longer be named.
#[test]
fn a_stale_worker_finishing_late_leaves_the_new_owners_ledger_entry_alone() {
    let bench = Bench::new();
    let stale = bench.tree();
    stale.script(vec![TreeStep::Delay(Duration::from_secs(10))]);
    stale.begin(bench.prepare()).unwrap();
    until("the stale worker wrote its entry", || {
        bench.ledger.read().is_some()
    });

    bench.leases.set(OTHER, 8, CLOCK_START + 60_000);
    let (child, pid, start) = orphan();
    bench
        .ledger
        .write(&bench.entry(OTHER, 8, Some((pid, start.clone()))))
        .unwrap();
    let mut result = None;
    until("the stale worker finished", || {
        bench.clock.advance(1_000);
        result = stale.poll();
        result.is_some()
    });
    let kept = bench.ledger.read().expect("the new owner's entry stays");
    assert_eq!(kept.owner_id, OTHER);
    assert_eq!(kept.pid, Some(pid));
    SystemProcesses.kill_group(pid, &start);
    assert_dies(child);
}

/// A clock that panics on demand, which is how a panic reaches the worker while a child runs.
struct ArmedClock {
    inner: Arc<SimClock>,
    armed: Mutex<bool>,
}

impl Clock for ArmedClock {
    fn now(&self) -> Timestamp {
        assert!(
            !*self.armed.lock().unwrap(),
            "a scripted panic in the worker"
        );
        self.inner.now()
    }
}

#[test]
fn a_panicked_workers_child_is_killed_and_the_refusal_is_counted_as_exit_nonzero() {
    let bench = Bench::new();
    let tree = ScriptedTree::new(bench.ledger.clone());
    let clock = Arc::new(ArmedClock {
        inner: bench.clock.clone(),
        armed: Mutex::new(false),
    });
    bench.bind(&tree, clock.clone());
    tree.script(vec![sleeper()]);
    tree.begin(bench.prepare()).unwrap();
    let mut entry = None;
    until("the child is recorded", || {
        entry = tree.ledger().read().filter(|e| e.pid.is_some());
        entry.is_some()
    });
    let entry = entry.unwrap();
    *clock.armed.lock().unwrap() = true;
    let (pid, start) = (entry.pid.unwrap(), entry.start_time.unwrap());
    let mut result = None;
    until("the worker's refusal", || {
        result = tree.poll();
        result.is_some()
    });
    assert!(matches!(
        result.unwrap().1,
        TreeResult::Refused {
            reason: TreeRefusal::Git {
                reason: GitReason::ExitNonzero
            },
            ..
        }
    ));
    assert!(
        !SystemProcesses.is_alive(pid, &start),
        "the child outlived its worker"
    );
    *clock.armed.lock().unwrap() = false;
    tree.begin(bench.prepare()).expect("the worker lives on");
    until("the next result", || tree.poll().is_some());
}

// ---------------------------------------------------------------------- under `Host::tick`

fn silent(_: &SessionInfo) -> Cue {
    Cue::Silent
}

fn delayed_rig(dir: &std::path::Path, delay: Duration) -> Rig {
    let ledger = TreeLedger::new(&dir.join("agent-logs"), "ws-1");
    let tree = ScriptedTree::new(ledger).with_delay(delay);
    Rig::open(dir, CLOCK_START, Box::new(replies_pass), tree)
}

/// §12.11's loop case: an operation that blocks longer than the lease leaves the tick in its budget, the other
/// runs advancing, the lease renewed, and folds its result once.
#[test]
fn an_operation_blocking_longer_than_the_lease_leaves_the_tick_going() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = delayed_rig(dir.path(), Duration::from_secs(45));
    let first = rig.commission("first").run;
    rig.clock.advance(1_000);
    let second = rig.commission("second").run;
    let mut slowest = Duration::ZERO;
    let mut epochs = Vec::new();
    let mut expires = Vec::new();
    for _ in 0..30 {
        let started = Instant::now();
        rig.host.tick(rig.now());
        slowest = slowest.max(started.elapsed());
        let lease = rig.store.lease(&first).unwrap().unwrap();
        epochs.push(lease.epoch);
        expires.push(lease.expires_at);
        rig.clock.advance(1_000);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        rig.tree.busy(),
        "the operation is still blocking after 30 s"
    );
    assert!(!rig.has(&first, "tree_prepared"), "no result yet");
    // Against a machine with other work: a tick that waited on the operation would take its whole span.
    assert!(slowest < Duration::from_secs(5), "a tick took {slowest:?}");
    assert!(rig.has(&second, "root_opened"), "the other run advanced");
    assert!(
        rig.has(&second, "claim_busy"),
        "and waits visibly behind the first"
    );
    // The worker is a thread: when it gets to the result past the delay is the machine's to say.
    until("the result folded", || {
        rig.host.tick(rig.now());
        epochs.push(rig.store.lease(&first).unwrap().unwrap().epoch);
        expires.push(rig.store.lease(&first).unwrap().unwrap().expires_at);
        rig.clock.advance(1_000);
        rig.has(&first, "tree_prepared")
    });
    assert!(
        epochs.iter().all(|epoch| *epoch == 1),
        "the lease never changed hands"
    );
    assert!(
        expires.last().unwrap().unix_millis() - CLOCK_START >= 45_000,
        "the lease was renewed past the operation"
    );
    let prepared = rig
        .events(&first)
        .iter()
        .filter(|e| matches!(e.body, EventBody::TreePrepared { .. }))
        .count();
    assert_eq!(prepared, 1, "the result folded once");
    assert_eq!(rig.tree.begins(), 1, "and was started once");
}

#[test]
fn withdrawn_calls_cancel_on_an_in_flight_prepare() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = delayed_rig(dir.path(), Duration::from_secs(3_600));
    let receipt = rig.commission("work");
    let run = receipt.run;
    let check = run.clone();
    rig.run_until(50, move |rig| {
        rig.tree.busy() && rig.has(&check, "step_started")
    });
    assert_eq!(rig.tree.cancels(), 0);
    bridge::withdraw(
        &rig.bridge,
        &receipt.root_thread,
        &Rig::requester(),
        "stop",
        rig.now(),
    )
    .unwrap();
    for _ in 0..5 {
        rig.host.tick(rig.now());
    }
    assert_eq!(rig.tree.cancels(), 1, "cancelled once");
    rig.tree.script(Vec::new());
    let tree_free = run.clone();
    rig.run_until(600, move |rig| rig.is_finished(&tree_free));
    assert!(rig.has(&run, "tree_parked"));
}

/// A result that was already on its way when the withdrawal folded: `ignored { already_finishing }`.
#[test]
fn a_late_prepare_result_in_finishing_folds_to_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent));
    let receipt = rig.commission("work");
    let run = receipt.run;
    let check = run.clone();
    rig.run_until(50, move |rig| {
        rig.tree.begins() == 1 && rig.has(&check, "step_started")
    });
    until("the worker finished", || !rig.tree.ledger().path().exists());
    // Between the worker's finish and the host's poll, the requester withdraws.
    bridge::withdraw(
        &rig.bridge,
        &receipt.root_thread,
        &Rig::requester(),
        "stop",
        rig.now(),
    )
    .unwrap();
    let done = run.clone();
    rig.run_until(600, move |rig| rig.is_finished(&done));
    let events = rig.events(&run);
    let prepared = events
        .iter()
        .position(|e| matches!(e.body, EventBody::TreePrepared { .. }));
    if let Some(position) = prepared {
        assert!(
            matches!(
                &events[position + 1].body,
                EventBody::Decision(Decision::Ignored { reason, .. })
                    if *reason == ReasonCode::AlreadyFinishing
            ),
            "{:?}",
            events[position + 1].body
        );
    } else {
        panic!("the result never folded");
    }
}

/// After a restart the owed operation is re-derived from `pending_effects`, never from the ledger: an entry
/// for a run that owes nothing starts nothing, and one that owes re-runs from the start.
#[test]
fn after_a_restart_the_owed_operation_is_re_derived_and_runs_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = delayed_rig(dir.path(), Duration::from_secs(3_600));
    let run = rig.commission("work").run;
    let check = run.clone();
    rig.run_until(50, move |rig| {
        rig.tree.busy() && rig.has(&check, "step_started")
    });
    assert_eq!(rig.tree.begins(), 1);
    // The service dies with its worker. Nothing in memory says what was running.
    let ledger = TreeLedger::new(&dir.path().join("agent-logs"), "ws-1");
    rig.restart_with(ScriptedTree::new(ledger));
    rig.clock.advance(31_000);
    let check = run.clone();
    rig.step_ms = 1_000;
    rig.run_until(100, move |rig| rig.has(&check, "tree_prepared"));
    assert_eq!(rig.tree.begins(), 1, "the new tree ran what the state owed");
}

/// A two-hosts tree case with an operation in flight during takeover: the new owner kills the old owner's child
/// and runs its own, and the old owner's result is dropped.
#[test]
fn a_takeover_with_an_operation_in_flight_kills_the_child_and_folds_one_result() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = TreeLedger::new(&dir.path().join("agent-logs"), "ws-1");
    let tree = ScriptedTree::new(ledger);
    tree.script(vec![sleeper()]);
    let mut rig = Rig::open(dir.path(), CLOCK_START, Box::new(silent), tree);
    let (mut second, second_tree) = rig.second_host();
    let run = rig.commission("work").run;
    let check = run.clone();
    rig.run_until(50, move |rig| {
        rig.has(&check, "step_started") && rig.tree.ledger().read().is_some_and(|e| e.pid.is_some())
    });
    let entry = rig.tree.ledger().read().unwrap();
    let (pid, start) = (entry.pid.unwrap(), entry.start_time.clone().unwrap());
    assert_eq!(entry.owner_id, rig.host.owner_id());
    assert_eq!(entry.epoch, 1);

    // Inside the lease the other host asks nothing of the tree, and the child runs on.
    second.tick(rig.now());
    assert_eq!(second_tree.begins(), 0);
    assert!(SystemProcesses.is_alive(pid, &start));

    rig.clock.advance(31_000);
    until("the new owner's prepare folded", || {
        second.tick(rig.now());
        rig.has(&run, "tree_prepared")
    });
    assert_eq!(second_tree.begins(), 1, "the new owner began its own");
    assert!(
        !SystemProcesses.is_alive(pid, &start),
        "the old owner's child was killed"
    );
    assert!(rig.has(&run, "tree_prepared"));

    // The old host's worker saw its child die. Its result is dropped, whatever it says.
    for _ in 0..10 {
        rig.host.tick(rig.now());
        second.tick(rig.now());
        std::thread::sleep(Duration::from_millis(5));
    }
    let prepared = rig
        .events(&run)
        .iter()
        .filter(|e| matches!(e.body, EventBody::TreePrepared { .. }))
        .count();
    assert_eq!(prepared, 1, "one result folded");
}

/// A result is enqueued only under a live lease (round 6): what the old owner's worker found out is dropped, even
/// though it finished first, and the run folds what the new owner found.
#[test]
fn a_result_of_a_host_that_lost_the_lease_is_dropped_and_the_new_owners_folds() {
    let prepared_at = |commit: &str| {
        TreeStep::Answer(TreeResult::Prepared {
            base: nexus_run_contracts::Base {
                branch: "main".to_owned(),
                commit: commit.to_owned(),
            },
            found: None,
        })
    };
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent));
    rig.tree.script(vec![
        TreeStep::Delay(Duration::from_secs(40)),
        prepared_at("from-the-old-owner"),
    ]);
    let (mut second, second_tree) = rig.second_host();
    second_tree.script(vec![
        TreeStep::Delay(Duration::from_secs(600)),
        prepared_at("from-the-new-owner"),
    ]);
    let run = rig.commission("work").run;
    let check = run.clone();
    rig.step_ms = 0;
    rig.run_until(50, move |rig| {
        rig.has(&check, "step_started") && rig.tree.begins() == 1
    });
    rig.clock.advance(31_000);
    second.tick(rig.now());
    assert_eq!(second_tree.begins(), 1, "the new owner began its own");

    // The old owner's operation ends, and its host polls the result with the lease gone.
    rig.clock.advance(10_000);
    until("the old host polled its worker's result", || {
        rig.host.tick(rig.now());
        !rig.tree.busy()
    });
    for _ in 0..5 {
        rig.host.tick(rig.now());
        second.tick(rig.now());
    }
    assert!(!rig.has(&run, "tree_prepared"), "the stale result folded");
    let waiting = rig.store.waiting_inputs(&run).unwrap();
    assert!(
        !waiting
            .iter()
            .flatten()
            .any(|e| matches!(e.body, EventBody::TreePrepared { .. })),
        "the stale result reached the inbox"
    );

    until("the new owner's prepare folded", || {
        rig.clock.advance(10_000);
        second.tick(rig.now());
        rig.has(&run, "tree_prepared")
    });
    let base = rig.events(&run).iter().find_map(|event| match &event.body {
        EventBody::TreePrepared { base, .. } => Some(base.commit.clone()),
        _ => None,
    });
    assert_eq!(base.as_deref(), Some("from-the-new-owner"));
}

// ---------------------------------------------------------------------- refusals and the window

fn refused(text: &str) -> TreeStep {
    TreeStep::Answer(TreeResult::Refused {
        reason: TreeRefusal::Git {
            reason: GitReason::FetchFailed,
        },
        text: text.to_owned(),
    })
}

fn refusing_rig(dir: &std::path::Path, window: u64) -> Rig {
    let mut rig = Rig::scripted(dir, CLOCK_START, Box::new(replies_pass));
    rig.policy.tree_window = Secs(window);
    rig
}

fn stored_refusal(rig: &Rig, run: &RunId) -> Option<serde_json::Value> {
    let bytes = rig.store.get(&format!("tree_refusal/{run}")).unwrap()?;
    Some(serde_json::from_slice(&bytes).unwrap())
}

#[test]
fn a_transient_refusal_inside_the_window_stays_a_kv_row_and_a_later_prepare_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = refusing_rig(dir.path(), 600);
    for _ in 0..3 {
        rig.tree.script(vec![refused("the network is down")]);
    }
    let run = rig.commission("work").run;
    let check = run.clone();
    rig.run_until(400, move |rig| rig.has(&check, "tree_prepared"));
    assert!(
        !rig.has(&run, "tree_refused"),
        "a transient refusal is no input"
    );
    assert!(!rig.has(&run, "parked"));
    assert_eq!(
        rig.tree.begins(),
        4,
        "three refusals and the prepare that worked"
    );
    assert_eq!(stored_refusal(&rig, &run), None, "cleared by the prepare");
}

/// The backoff, doubling from 1 s, spaces the retries: a refusal is not retried on every tick.
#[test]
fn a_refused_prepare_backs_off() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = refusing_rig(dir.path(), 3_600);
    for _ in 0..20 {
        rig.tree.script(vec![refused("the network is down")]);
    }
    rig.commission("work");
    rig.step_ms = 100;
    for _ in 0..200 {
        rig.tick();
    }
    // 20 s of ticks at 100 ms: 1, 2, 4, 8 s of backoff allow about five attempts, never two hundred.
    let begins = rig.tree.begins();
    assert!((2..=7).contains(&begins), "{begins} attempts in 20 s");
}

#[test]
fn first_refused_at_stays_apart_from_the_at_of_the_refusal_text() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = refusing_rig(dir.path(), 3_600);
    rig.tree.script(vec![refused("first text")]);
    rig.tree.script(vec![refused("first text")]);
    rig.tree.script(vec![refused("second text")]);
    for _ in 0..40 {
        rig.tree.script(vec![refused("second text")]);
    }
    let run = rig.commission("work").run;
    let check = run.clone();
    rig.run_until(100, move |rig| stored_refusal(rig, &check).is_some());
    let first = stored_refusal(&rig, &run).unwrap();
    let check = run.clone();
    rig.run_until(400, move |rig| {
        stored_refusal(rig, &check).is_some_and(|r| r["text"] == "second text")
    });
    let second = stored_refusal(&rig, &run).unwrap();
    assert_eq!(second["first_refused_at"], first["first_refused_at"]);
    assert_ne!(second["at"], first["at"], "`at` moved with the text");
    let at_of_second = second["at"].clone();
    for _ in 0..40 {
        rig.tick();
    }
    assert_eq!(
        stored_refusal(&rig, &run).unwrap()["at"],
        at_of_second,
        "the same refusal again writes nothing"
    );
}

#[test]
fn a_refusal_standing_when_the_window_closes_parks_the_run_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = refusing_rig(dir.path(), 60);
    for _ in 0..200 {
        rig.tree.script(vec![refused("the network is down")]);
    }
    let receipt = rig.commission("work");
    let run = receipt.run;
    let check = run.clone();
    rig.run_until(400, move |rig| rig.has(&check, "tree_refused"));
    let reason = rig.events(&run).iter().find_map(|event| match &event.body {
        EventBody::TreeRefused { reason } => Some(reason.clone()),
        _ => None,
    });
    assert_eq!(
        reason,
        Some(TreeRefusal::Git {
            reason: GitReason::FetchFailed
        })
    );
    let status = bridge::status(&rig.bridge, &run, rig.now())
        .unwrap()
        .unwrap();
    assert_eq!(status.reason, Some(ReasonCode::TreeRefused));
    let local = status.local.expect("this machine holds the run");
    assert_eq!(
        local.tree.refusal.expect("the refusal is in status").text,
        "the network is down"
    );
    let begins = rig.tree.begins();
    for _ in 0..50 {
        rig.tick();
    }
    assert_eq!(rig.tree.begins(), begins, "a parked run stops retrying");
    bridge::withdraw(
        &rig.bridge,
        &receipt.root_thread,
        &Rig::requester(),
        "stop",
        rig.now(),
    )
    .unwrap();
    rig.run_to_finished(&run);
    assert_eq!(stored_refusal(&rig, &run), None, "gone at `finished`");
}

#[test]
fn a_permanent_refusal_parks_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = refusing_rig(dir.path(), 3_600);
    rig.tree.script(vec![TreeStep::Answer(TreeResult::Refused {
        reason: TreeRefusal::ForeignDirectory,
        text: "another repository".to_owned(),
    })]);
    let run = rig.commission("work").run;
    let check = run.clone();
    rig.run_until(50, move |rig| rig.has(&check, "parked"));
    assert_eq!(rig.tree.begins(), 1);
    assert!(rig.has(&run, "tree_refused"));
}

/// The host cancels and counts an operation in flight when `tree_window` fires (§6.5).
#[test]
fn a_closing_window_cancels_the_operation_in_flight() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = refusing_rig(dir.path(), 30);
    rig.tree.script(vec![refused("one")]);
    rig.tree.script(vec![refused("two")]);
    for _ in 0..3 {
        rig.tree
            .script(vec![TreeStep::Delay(Duration::from_secs(36_000))]);
    }
    let run = rig.commission("work").run;
    until("the run is parked on the refusal", || {
        rig.host.tick(rig.now());
        rig.clock.advance(1_000);
        rig.has(&run, "tree_refused")
    });
    assert!(
        rig.tree.cancels() >= 1,
        "the operation in flight was cancelled"
    );
}

/// The minimum attempt count beside the elapsed time: a machine that slept through the window has not tried.
#[test]
fn a_clock_jump_past_the_window_does_not_park_before_the_minimum_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = refusing_rig(dir.path(), 60);
    for _ in 0..50 {
        rig.tree.script(vec![refused("the network is down")]);
    }
    let run = rig.commission("work").run;
    let check = run.clone();
    rig.run_until(50, move |rig| {
        rig.tree.begins() >= 1 && rig.has(&check, "step_started")
    });
    let begun = rig.tree.begins();
    assert_eq!(begun, 1);
    rig.clock.advance(3_600 * 1_000);
    let check = run.clone();
    rig.run_until(600, move |rig| rig.has(&check, "tree_refused"));
    assert!(
        rig.tree.begins() >= 3,
        "parked after {} attempts",
        rig.tree.begins()
    );
}

/// The same without a jump: a window so short it closes before three attempts could run still waits for them.
#[test]
fn a_short_window_does_not_park_before_the_minimum_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = refusing_rig(dir.path(), 1);
    for _ in 0..50 {
        rig.tree.script(vec![refused("the network is down")]);
    }
    let run = rig.commission("work").run;
    let check = run.clone();
    rig.run_until(600, move |rig| rig.has(&check, "tree_refused"));
    assert!(
        rig.tree.begins() >= 3,
        "parked after {} attempts",
        rig.tree.begins()
    );
}

/// A park's window changes nothing observable (§6.5): the claim stays held and it retries, however long, status
/// says so, and only the recorded park releases.
#[test]
fn a_refused_park_retries_past_any_window_and_holds_the_claim() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = refusing_rig(dir.path(), 60);
    let receipt = rig.commission("work");
    let run = receipt.run;
    rig.backend.set_scenario(Box::new(silent));
    let check = run.clone();
    rig.run_until(100, move |rig| rig.has(&check, "tree_prepared"));
    rig.tree.script(vec![refused("index.lock is held")]);
    for _ in 0..5 {
        rig.tree.script(vec![refused("index.lock is held")]);
    }
    bridge::withdraw(
        &rig.bridge,
        &receipt.root_thread,
        &Rig::requester(),
        "stop",
        rig.now(),
    )
    .unwrap();
    let check = run.clone();
    rig.run_until(200, move |rig| stored_refusal(rig, &check).is_some());
    let status = bridge::status(&rig.bridge, &run, rig.now())
        .unwrap()
        .unwrap();
    assert_eq!(status.waiting_for.as_deref(), Some("the park"));
    assert_eq!(
        status.local.unwrap().tree.refusal.unwrap().text,
        "index.lock is held"
    );
    assert!(rig.store.get_claim_holder().is_some(), "the claim is held");
    assert!(
        !rig.has(&run, "tree_refused"),
        "a park's refusal is no input"
    );
    assert!(!rig.has(&run, "claim_released"));
    let done = run.clone();
    rig.run_until(2_000, move |rig| rig.is_finished(&done));
    assert_eq!(rig.store.get_claim_holder(), None);
    let outcome = rig.events(&run).iter().find_map(|event| match &event.body {
        EventBody::TreeParked { outcome } => Some(outcome.clone()),
        _ => None,
    });
    assert!(matches!(outcome, Some(ParkOutcome::Clean { .. })));
    let ended = rig.events(&run).iter().find_map(|event| match &event.body {
        EventBody::Decision(Decision::Finished { outcome, .. }) => Some(outcome.clone()),
        _ => None,
    });
    assert_eq!(ended, Some(Outcome::Withdrawn));
}

/// A queued run withdrawn while another holds the claim runs no git at all: `not_held`.
#[test]
fn a_run_that_never_held_the_claim_records_not_held_and_begins_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent));
    let holder = rig.commission("first").run;
    rig.clock.advance(1_000);
    let queued = rig.commission("second");
    let check = queued.run.clone();
    rig.run_until(50, move |rig| rig.has(&check, "claim_busy"));
    let begins = rig.tree.begins();
    bridge::withdraw(
        &rig.bridge,
        &queued.root_thread,
        &Rig::requester(),
        "stop",
        rig.now(),
    )
    .unwrap();
    rig.run_to_finished(&queued.run);
    assert_eq!(
        rig.tree.begins(),
        begins,
        "a withdrawn queued run ran a tree operation"
    );
    let outcome = rig
        .events(&queued.run)
        .iter()
        .find_map(|event| match &event.body {
            EventBody::TreeParked { outcome } => Some(outcome.clone()),
            _ => None,
        });
    assert_eq!(outcome, Some(ParkOutcome::NotHeld));
    assert_eq!(
        rig.store.get_claim_holder().as_deref(),
        Some(holder.as_str()),
        "the holder keeps its claim"
    );
}

/// `ps` that cannot name a child: no identity means no group kill, so the child is not let go at all.
struct NoStartTime;

impl ProcessGuard for NoStartTime {
    fn start_time(&self, _pid: u32) -> Option<String> {
        None
    }

    fn kill_group(&self, _pid: u32, _start_time: &str) -> bool {
        false
    }

    fn group_gone(&self, _pgid: u32) -> bool {
        true
    }
}

fn shell(script: String) -> TreeStep {
    TreeStep::Child {
        program: "sh".to_owned(),
        args: vec!["-c".to_owned(), script],
    }
}

/// m8we's review: an empty start time disarms `kill_group`, so a deadline reaches the direct child only and an
/// orphan can keep `index.lock`.
#[test]
fn a_child_whose_start_time_cannot_be_read_is_a_spawn_failure_and_never_runs() {
    let bench = Bench::new();
    let marker = bench._dir.path().join("ran");
    let tree = ScriptedTree::with_guard(bench.ledger.clone(), Arc::new(NoStartTime));
    bench.bind(&tree, bench.clock.clone());
    tree.script(vec![shell(format!("touch {}", marker.display()))]);
    tree.begin(bench.prepare()).unwrap();
    let mut result = None;
    until("the result", || {
        result = tree.poll();
        result.is_some()
    });
    let (_, result) = result.unwrap();
    assert!(
        matches!(&result, TreeResult::Refused { text, .. } if text.contains("could not be started")),
        "{result:?}"
    );
    assert!(
        !marker.exists(),
        "the program ran with nobody able to kill it"
    );
    assert!(!tree.busy());
}

/// Slow to name a child, so a program with no gate in front of it would start well before the ledger names it.
struct SlowToName;

impl ProcessGuard for SlowToName {
    fn start_time(&self, pid: u32) -> Option<String> {
        std::thread::sleep(Duration::from_millis(300));
        SystemProcesses.start_time(pid)
    }

    fn kill_group(&self, pid: u32, start_time: &str) -> bool {
        SystemProcesses.kill_group(pid, start_time)
    }

    fn group_gone(&self, pgid: u32) -> bool {
        SystemProcesses.group_gone(pgid)
    }
}

/// The gate: the program starts only once the ledger names its pid, so there is no moment a program runs with
/// nobody able to name it. What the program reads is the ledger at the moment it starts.
#[test]
fn a_program_starts_only_after_the_ledger_names_it() {
    let bench = Bench::new();
    let tree = ScriptedTree::with_guard(bench.ledger.clone(), Arc::new(SlowToName));
    bench.bind(&tree, bench.clock.clone());
    let seen = bench._dir.path().join("seen");
    tree.script(vec![shell(format!(
        "cat {} > {}; echo $$ >> {}",
        tree.ledger().path().display(),
        seen.display(),
        seen.display()
    ))]);
    tree.begin(bench.prepare()).unwrap();
    until("the result", || tree.poll().is_some());
    let seen = std::fs::read_to_string(&seen).unwrap();
    let (entry, pid) = seen.rsplit_once('}').unwrap();
    let entry: LedgerEntry = serde_json::from_str(&format!("{entry}}}")).unwrap();
    assert_eq!(
        entry.pid.map(|pid| pid.to_string()).as_deref(),
        Some(pid.trim())
    );
    assert!(entry.start_time.is_some_and(|start| !start.is_empty()));
}

/// The program gets nothing of the gate on its stdin.
#[test]
fn a_program_reads_an_empty_stdin_behind_the_gate() {
    let bench = Bench::new();
    let tree = bench.tree();
    let seen = bench._dir.path().join("seen");
    tree.script(vec![shell(format!("cat > {}", seen.display()))]);
    tree.begin(bench.prepare()).unwrap();
    until("the result", || tree.poll().is_some());
    assert_eq!(std::fs::read_to_string(&seen).unwrap(), "");
}

/// A process of its own session, inheriting stderr: the shape of a daemon a helper starts. It answers to no group
/// kill, so the test ends it itself.
fn escaped_helper(pid_file: &std::path::Path) -> String {
    format!(
        "perl -e 'use POSIX; POSIX::setsid(); open(F, \">{}\"); print F $$; close(F); exec(\"sleep\", \"600\")' &",
        pid_file.display()
    )
}

fn end_helper(pid_file: &std::path::Path) {
    until("the helper wrote its pid", || {
        pid_file.exists() && !std::fs::read_to_string(pid_file).unwrap().is_empty()
    });
    let pid = std::fs::read_to_string(pid_file).unwrap();
    let _ = Command::new("kill").args(["-KILL", pid.trim()]).status();
}

/// git is gone and a helper of its still holds the pipe: the answer is unknown, so it is a refusal, never an empty
/// success, and it does not wait for the helper.
#[test]
fn a_helper_holding_the_pipe_after_a_clean_exit_is_a_refusal_not_an_empty_answer() {
    let bench = Bench::new();
    let tree = bench.tree();
    let pid_file = bench._dir.path().join("helper");
    tree.script(vec![shell(format!("{} exit 0", escaped_helper(&pid_file)))]);
    tree.begin(bench.prepare()).unwrap();
    let mut result = None;
    until("the result", || {
        result = tree.poll();
        result.is_some()
    });
    end_helper(&pid_file);
    let (_, result) = result.unwrap();
    assert!(
        matches!(&result, TreeResult::Refused { text, .. } if text.contains("output")),
        "{result:?}"
    );
}

/// Past the kill too: the bound passes, the group dies, the helper of another session lives on holding the pipe, and
/// the operation still answers.
#[test]
fn a_helper_holding_the_pipe_past_the_kill_does_not_hold_the_operation() {
    let bench = Bench::new();
    let tree = ScriptedTree::with_deadline(bench.ledger.clone(), Duration::from_secs(60));
    bench.bind(&tree, bench.clock.clone());
    let pid_file = bench._dir.path().join("helper");
    tree.script(vec![shell(format!(
        "{} sleep 600",
        escaped_helper(&pid_file)
    ))]);
    tree.begin(bench.prepare()).unwrap();
    until("the helper is up", || {
        pid_file.exists() && !std::fs::read_to_string(&pid_file).unwrap().is_empty()
    });
    bench.clock.advance(61_000);
    let mut result = None;
    until("the result", || {
        result = tree.poll();
        result.is_some()
    });
    end_helper(&pid_file);
    assert!(matches!(result.unwrap().1, TreeResult::Refused { .. }));
}

/// Every lock a killed git leaves goes by one rule (§5 names only the index's; m8we left the others to this ticket).
#[test]
fn a_lock_of_any_kind_the_killed_child_created_is_cleared_and_one_there_before_is_not() {
    for lock in [
        "index.lock",
        "HEAD.lock",
        "config.lock",
        "packed-refs.lock",
        "refs/heads/feat/x.lock",
    ] {
        for there_before in [false, true] {
            let bench = Bench::new();
            let tree = bench.tree();
            bench.leases.set(OWNER, 4, CLOCK_START + 30_000);
            let git_dir = bench._dir.path().join("v2").join(".git");
            std::fs::create_dir_all(git_dir.join("refs/heads/feat")).unwrap();
            std::fs::write(git_dir.join(lock), b"").unwrap();
            let (child, pid, start) = orphan();
            let mut entry = bench.entry(OTHER, 3, Some((pid, start)));
            if there_before {
                entry.locks_before = vec![lock.to_owned()];
            }
            bench.ledger.write(&entry).unwrap();
            tree.begin(bench.prepare()).unwrap();
            assert_dies(child);
            assert_eq!(
                git_dir.join(lock).exists(),
                there_before,
                "{lock}, there before: {there_before}"
            );
            until("the operation's result", || tree.poll().is_some());
        }
    }
}

/// A child the bound killed in this same process leaves its lock too, and the ledger entry that could prove it is
/// about to go with the operation.
#[test]
fn a_lock_of_a_child_killed_at_its_bound_is_cleared_with_the_group_gone() {
    let bench = Bench::new();
    let tree = ScriptedTree::with_deadline(bench.ledger.clone(), Duration::from_secs(60));
    bench.bind(&tree, bench.clock.clone());
    let git_dir = bench._dir.path().join("v2").join(".git");
    std::fs::create_dir_all(&git_dir).unwrap();
    let planted = git_dir.join("config.lock");
    std::fs::write(&planted, b"").unwrap();
    let made = git_dir.join("index.lock");
    tree.script(vec![shell(format!("touch {}; sleep 600", made.display()))]);
    tree.begin(bench.prepare()).unwrap();
    until("the child made its lock", || made.exists());
    bench.clock.advance(61_000);
    until("the result", || tree.poll().is_some());
    assert!(!made.exists(), "the killed child's lock stayed");
    assert!(planted.exists(), "a lock from before it was removed");
}

/// Real processes, and a group that will not be confirmed gone: nothing proves the child stopped writing.
struct GroupStays;

impl ProcessGuard for GroupStays {
    fn start_time(&self, pid: u32) -> Option<String> {
        SystemProcesses.start_time(pid)
    }

    fn kill_group(&self, pid: u32, start_time: &str) -> bool {
        SystemProcesses.kill_group(pid, start_time)
    }

    fn group_gone(&self, _pgid: u32) -> bool {
        false
    }
}

#[test]
fn a_lock_stays_while_the_group_is_not_confirmed_gone() {
    let bench = Bench::new();
    let tree = ScriptedTree::with_guard(bench.ledger.clone(), Arc::new(GroupStays));
    bench.bind(&tree, bench.clock.clone());
    bench.leases.set(OWNER, 4, CLOCK_START + 30_000);
    let git_dir = bench._dir.path().join("v2").join(".git");
    std::fs::create_dir_all(&git_dir).unwrap();
    let lock = git_dir.join("index.lock");
    std::fs::write(&lock, b"").unwrap();
    let (child, pid, start) = orphan();
    bench
        .ledger
        .write(&bench.entry(OTHER, 3, Some((pid, start))))
        .unwrap();
    tree.begin(bench.prepare()).unwrap();
    assert_dies(child);
    assert!(lock.exists(), "cleared with the group not confirmed gone");
    until("the operation's result", || tree.poll().is_some());
}
