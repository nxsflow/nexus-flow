//! Two hosts on one store (§12.5, epic DoD 10): only the lease holder appends and executes, the other's append
//! is `Fenced`, a takeover comes within one expiry with a higher epoch, the stale holder executes nothing, and
//! the run completes exactly once. Time is the simulated clock, and the assertions read the log and timeline.

mod common;

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use common::Crashbox;
use nexus_run_contracts::{Decision, Event, EventBody, Origin, RunStore, StoreError};
use nexus_run_host::test_support::{replies_pass, Rig, CLOCK_START};
use nexus_run_host::LeaseView;

fn finished_count(rig: &Rig, run: &nexus_run_contracts::RunId) -> usize {
    rig.events(run)
        .iter()
        .filter(|e| matches!(e.body, EventBody::Decision(Decision::Finished { .. })))
        .count()
}

#[test]
fn only_the_holder_steps_and_the_other_takes_over_within_one_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let (mut second, _tree) = rig.second_host();
    let run = rig.commission("work").run;
    rig.tick();
    let lease = rig.store.lease(&run).unwrap().unwrap();
    assert_eq!(lease.owner_id, rig.host.owner_id());
    assert_eq!(lease.epoch, 1);

    // Inside the lease the other host writes nothing for this run: no input, no effect.
    let before = rig.events(&run).len();
    for _ in 0..5 {
        let report = second.tick(rig.now());
        assert_eq!(report.folded, 0, "the other host folded an input");
        rig.clock.advance(1_000);
    }
    assert_eq!(rig.events(&run).len(), before);
    assert_eq!(rig.store.lease(&run).unwrap().unwrap().epoch, 1);

    // The holder stops renewing. Within one expiry the other holds the run, at a higher epoch.
    rig.clock.advance(30_000);
    second.tick(rig.now());
    let taken = rig.store.lease(&run).unwrap().unwrap();
    assert_eq!(taken.owner_id, second.owner_id());
    assert_eq!(taken.epoch, 2);

    // The stale holder's next append is `Fenced`, and its tick executes nothing for the run.
    let stale = Event::new(
        run.clone(),
        0,
        rig.now(),
        "stale/append",
        Origin::Host,
        EventBody::ClaimReleased,
    );
    let seq = rig.events(&run).len() as u64;
    assert_eq!(
        rig.store.append(&run, seq, 1, &[stale], &[], &[]),
        Err(StoreError::Fenced)
    );
    let spawned_before = rig.spawned().len();
    let report = rig.host.tick(rig.now());
    assert_eq!(report.folded, 0, "a stale host folded an input");
    assert_eq!(rig.spawned().len(), spawned_before, "a stale host spawned");

    for _ in 0..2_000 {
        if rig.is_finished(&run) {
            break;
        }
        second.tick(rig.now());
        rig.backend.advance(&rig.bridge, rig.now());
        rig.clock.advance(2_000);
    }
    assert!(
        rig.is_finished(&run),
        "the new owner did not finish the run"
    );
    assert_eq!(finished_count(&rig, &run), 1, "the run completes once");
}

/// A stale host's tick writes nothing: it finds the lease held and skips the run, inputs and effects alike.
#[test]
fn a_stale_hosts_tick_writes_nothing_to_the_chat_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let (mut second, _tree) = rig.second_host();
    rig.commission("work");
    rig.tick();
    rig.clock.advance(31_000);
    second.tick(rig.now());
    let lines = rig.ports.lines().len();
    let report = rig.host.tick(rig.now());
    assert_eq!(report.folded, 0);
    assert_eq!(
        rig.ports.lines().len(),
        lines,
        "a stale host wrote to the chat store"
    );
}

/// The re-check immediately before an effect (§7): the lease ran out after this tick acquired it, as it does
/// when a tick stalls, and the owed effect does not run.
#[test]
fn a_lease_that_ran_out_inside_the_tick_stops_the_next_effect() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let run = rig.commission("work").run;
    let acquired_at = rig.now();
    rig.store
        .acquire_or_renew(&run, rig.host.owner_id(), acquired_at)
        .unwrap();
    // The tick starts inside the lease, and its clock has moved past it by the time an effect is due.
    rig.clock.advance(31_000);
    rig.host.tick(acquired_at);
    assert!(
        rig.has(&run, "commissioned"),
        "the input folded under the lease"
    );
    assert_eq!(
        rig.ports.lines(),
        Vec::new(),
        "the effect ran on a lost lease"
    );
}

#[test]
fn two_hosts_in_two_threads_race_and_the_run_completes_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(replies_pass));
    let (mut second, _tree) = rig.second_host();
    let run = rig.commission("work").run;
    let stop = AtomicBool::new(false);
    let clock = rig.clock.clone();
    let backend = rig.backend.clone();
    let bridge = rig.bridge.clone();
    let done = rig.store.clone();
    let host = &mut rig.host;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            use nexus_run_host::ports::Clock;
            while !stop.load(Ordering::SeqCst) {
                host.tick(clock.now());
            }
        });
        scope.spawn(|| {
            use nexus_run_host::ports::Clock;
            while !stop.load(Ordering::SeqCst) {
                second.tick(clock.now());
            }
        });
        use nexus_run_host::ports::Clock;
        for _ in 0..200_000 {
            backend.advance(&bridge, clock.now());
            clock.advance(100);
            std::thread::sleep(std::time::Duration::from_micros(200));
            let open = done.open_runs().unwrap_or_default();
            if !open.iter().any(|row| row.run == run) {
                break;
            }
        }
        stop.store(true, Ordering::SeqCst);
    });
    assert!(
        rig.is_finished(&run),
        "the run did not finish: {:?}",
        rig.kinds()
    );
    assert_eq!(finished_count(&rig, &run), 1);
    let log = rig.events(&run);
    let delivered = log
        .iter()
        .filter(|e| matches!(e.body, EventBody::Delivered { .. }))
        .count();
    assert_eq!(delivered, 1);
    let timeline = common::Timeline(rig.backend.timeline());
    assert_eq!(timeline.first_overlap(), None);
}

/// The same, as two processes: nothing is shared but the files and the clock. The first host aborts in the
/// middle of the run, and the second, which has been skipping it, takes it over and finishes it once.
#[test]
fn two_hosts_as_two_processes_on_one_store_finish_the_run_once() {
    const BOX: &str = env!("CARGO_BIN_EXE_run-host-crashbox");
    let harness = Crashbox::new("happy");
    let spawn = |first: bool| {
        let mut command = Command::new(BOX);
        command
            .arg(harness.dir())
            .arg("happy")
            .env("NXR_SHARED_CLOCK", "1")
            .stderr(Stdio::null());
        if first {
            command
                .env("NXR_NO_DRIVER", "1")
                .env("NXR_CRASH_AT", "before_spawn:3");
        }
        command.spawn().expect("the box starts")
    };
    let mut first = spawn(true);
    // The second starts once the first has commissioned, so one commission is the whole script.
    let marker = harness.dir().join("run-happy");
    for _ in 0..5_000 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let mut second = spawn(false);
    let first_ended = first.wait().unwrap();
    assert!(
        second.wait().unwrap().success(),
        "the second host did not finish"
    );
    let log = harness.log();
    assert_eq!(log.finished_outcome().as_deref(), Some("delivered"));
    assert_eq!(log.count("finished"), 1, "the run completes once");
    assert_eq!(log.count("delivered"), 1);
    assert_eq!(harness.timeline().first_overlap(), None);
    let store = harness.store();
    assert!(store.claim_absent());
    if first_ended.code().is_none() {
        let lease = store.store.lease(&harness.run_id()).unwrap().unwrap();
        assert!(
            lease.epoch >= 2,
            "the takeover raised the epoch: {}",
            lease.epoch
        );
    }
}
