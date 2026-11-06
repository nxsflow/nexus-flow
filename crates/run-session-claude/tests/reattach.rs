//! A service restart neither kills a session nor makes a second one (§8.3, §12.8, epic DoD 8): a real child in its own
//! process group, a new reader, and no second spawn.

mod common;

use common::*;
use nexus_run_contracts::{SessionBackend, SessionFact, StartOutcome};
use serde_json::json;

fn started() -> serde_json::Value {
    json!({ "fact": { "kind": "started", "backend_session_id": "s-1" } })
}

fn ended() -> serde_json::Value {
    json!({ "fact": { "kind": "ended", "raw": { "kind": "result", "subtype": "success", "is_error": false } } })
}

#[test]
fn a_restarted_reader_finds_the_live_child_and_spawns_no_second() {
    let bench = Bench::new();
    let spec = bench.spec(
        KEY,
        json!({ "steps": [started(), { "wait": "go" }, ended(), { "exit": 0 }] }),
    );
    let backend = bench.backend();
    assert_eq!(backend.start(&spec).unwrap(), StartOutcome::Spawned);
    let pid = bench.child_record(KEY).pid;
    drop(backend); // the service dies, the child lives on
    assert!(pid_alive(pid));

    let again = bench.backend();
    assert_eq!(again.start(&spec).unwrap(), StartOutcome::Reattached);
    assert_eq!(bench.running_sidecars(), 1, "a second spawn");

    bench.cue(KEY, "go");
    let (facts, _) = drain_until_ended(&again, KEY, 0);
    let starts = facts
        .iter()
        .filter(|f| matches!(f, SessionFact::Started { .. }))
        .count();
    assert_eq!(starts, 1);
}

#[test]
fn the_child_runs_in_the_specs_cwd_and_in_a_process_group_of_its_own() {
    let bench = Bench::new();
    let spec = bench.spec(
        KEY,
        json!({ "steps": [{ "cwd_to": "cwd" }, { "wait": "go" }] }),
    );
    let backend = bench.backend();
    backend.start(&spec).unwrap();
    let pid = bench.child_record(KEY).pid;
    let cwd = bench.ledger_dir(KEY).join("cwd");
    until("the child said where it runs", || cwd.exists());

    let output = std::process::Command::new("ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let pgid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    kill_hard(pid);

    assert_eq!(pgid, pid.to_string());
    assert_eq!(std::fs::read_to_string(cwd).unwrap(), spec.cwd);
}
