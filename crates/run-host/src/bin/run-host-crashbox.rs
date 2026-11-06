//! One tick loop against a temp store (§12.4): `run-host-crashbox <dir> <script>`. It commissions on its first
//! start, runs until the run finished, and a restart is simply running it again on the same directory. It
//! aborts where `NXR_CRASH_AT` says.
//!
//! The clock is a file, because a restart continues it, and each run starts a lease expiry later than the
//! last tick: the aborted host's lease has to run out, as it does for a real restart.

use std::fs;
use std::path::Path;

use nexus_run_contracts::RunId;
use nexus_run_host::ports::{Clock, SystemClock};
use nexus_run_host::test_support::{scenario_named, Rig, CLOCK_START, REQUESTER};

const STEP_MS: i64 = 2_000;
/// How much faster than the wall the shared clock runs: a 30 s lease expires in 1.5 s.
const SHARED_CLOCK_RATE: i64 = 20;

fn read_clock(dir: &Path) -> i64 {
    fs::read_to_string(dir.join("clock"))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(CLOCK_START)
}

fn commissioned_run(rig: &Rig, marker: &Path, body: &str) -> RunId {
    if let Some(run) = fs::read_to_string(marker)
        .ok()
        .and_then(|text| RunId::parse(text.trim()))
    {
        return run;
    }
    let receipt = rig.commission(body);
    fs::write(marker, receipt.run.as_str()).expect("the marker is written");
    receipt.run
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, dir, script] = args.as_slice() else {
        eprintln!("usage: run-host-crashbox <dir> <script>");
        std::process::exit(2);
    };
    let dir = Path::new(dir);
    fs::create_dir_all(dir).expect("the box's directory");
    let mut rig = Rig::in_repo(dir, read_clock(dir), scenario_named(script));
    rig.step_ms = STEP_MS;
    let withdraws = script == "withdraw";
    let marker = dir.join(format!("run-{script}"));
    let run = commissioned_run(&rig, &marker, "the commission");
    // Two processes on one store share a clock, as two services on one machine do.
    let shared_clock = std::env::var_os("NXR_SHARED_CLOCK").is_some();
    if shared_clock {
        rig.step_ms = 0;
    }
    for _ in 0..200_000 {
        if shared_clock {
            let wall = SystemClock.now().unix_millis();
            rig.clock.set(wall.saturating_mul(SHARED_CLOCK_RATE));
        }
        if script == "next" {
            if rig.has(&run, "tree_prepared") {
                return;
            }
        } else if rig.is_finished(&run) {
            return;
        }
        if std::env::var_os("NXR_NO_DRIVER").is_some() {
            // The other host on this directory moves the sessions.
            let now = rig.now();
            rig.host.tick(now);
            rig.clock.advance(STEP_MS);
            std::thread::sleep(std::time::Duration::from_millis(1));
        } else {
            rig.tick();
        }
        fs::write(dir.join("clock"), rig.clock_millis().to_string()).expect("the clock is written");
        if withdraws && !rig.dir.join("withdrawn").exists() {
            maybe_withdraw(&rig, &run);
        }
    }
    eprintln!("no end after 200000 ticks: {:?}", rig.kinds());
    std::process::exit(3);
}

/// Once the coder has written its file, the requester withdraws.
fn maybe_withdraw(rig: &Rig, run: &RunId) {
    if !rig.bridge.layout.v2_dir().join("stray.txt").exists() {
        return;
    }
    let root = rig.state(run).root_thread.unwrap_or_default();
    let caller = nexus_run_host::bridge::Caller {
        actor: REQUESTER.to_owned(),
        session: None,
    };
    let withdrew = nexus_run_host::bridge::withdraw(&rig.bridge, &root, &caller, "stop", rig.now());
    if withdrew.is_ok() {
        fs::write(rig.dir.join("withdrawn"), b"").expect("the marker is written");
    }
}
