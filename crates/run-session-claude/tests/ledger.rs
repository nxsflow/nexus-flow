//! The session ledger and the read by offset (§8.3, §9): complete lines only, what is written atomically, and what a
//! start answers from what it finds.

mod common;

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use common::*;
use nexus_run_contracts::{EndCause, HeartbeatKind, SessionBackend, SessionFact, StartOutcome};
use nexus_run_session_claude::ledger::{ChildRecord, SessionLedger, SpawnRecord};
use nexus_run_session_claude::procs;
use serde_json::json;

fn beat(line: &str) -> String {
    format!("{{\"kind\":\"heartbeat\",\"beat\":\"{line}\"}}\n")
}

/// A ledger whose spawn and child are recorded and whose child is this test's own live process, so nothing ends it.
fn ledger_of_a_live_child(bench: &Bench) -> (SessionLedger, Reaped) {
    let ledger = SessionLedger::new(bench.ledger_dir(KEY));
    ledger.create().unwrap();
    let mut command = Command::new("sleep");
    command
        .arg("600")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = Reaped(command.spawn().unwrap());
    let info = procs::info(child.0.id()).unwrap();
    ledger
        .write_child(&ChildRecord {
            pid: child.0.id(),
            start_time: procs::start_text(info.start_ms),
            handlers: true,
        })
        .unwrap();
    assert!(ledger.claim_spawn(&SpawnRecord::default()).unwrap());
    (ledger, child)
}

fn facts_file(bench: &Bench) -> std::path::PathBuf {
    bench.ledger_dir(KEY).join("facts.jsonl")
}

#[test]
fn the_second_claim_of_a_spawn_loses() {
    let bench = Bench::new();
    let ledger = SessionLedger::new(bench.ledger_dir(KEY));
    ledger.create().unwrap();
    assert!(ledger.claim_spawn(&SpawnRecord::default()).unwrap());
    assert!(!ledger.claim_spawn(&SpawnRecord::default()).unwrap());
}

#[test]
fn facts_are_read_from_their_offset_by_byte_and_the_offset_is_the_key() {
    let bench = Bench::new();
    let (_ledger, _child) = ledger_of_a_live_child(&bench);
    let backend = bench.backend();
    let first = beat("model_output");
    let second = beat("calls_open");
    std::fs::write(facts_file(&bench), format!("{first}{second}")).unwrap();

    let (facts, next) = backend.drain_facts(KEY, 0).unwrap();
    assert_eq!(facts.len(), 2);
    assert_eq!(facts[0].offset, 0);
    assert_eq!(facts[1].offset, first.len() as u64);
    assert_eq!(next, (first.len() + second.len()) as u64);

    // A new reader, from that offset: nothing again.
    let (again, at) = bench.backend().drain_facts(KEY, next).unwrap();
    assert!(again.is_empty());
    assert_eq!(at, next);
}

#[test]
fn a_torn_last_line_waits_for_its_newline() {
    let bench = Bench::new();
    let (_ledger, _child) = ledger_of_a_live_child(&bench);
    let backend = bench.backend();
    let whole = beat("model_output");
    std::fs::write(facts_file(&bench), format!("{whole}{{\"kind\":\"heart")).unwrap();

    let (facts, next) = backend.drain_facts(KEY, 0).unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(next, whole.len() as u64, "the torn line was consumed");

    std::fs::write(facts_file(&bench), format!("{whole}{}", beat("calls_open"))).unwrap();
    let (later, _) = backend.drain_facts(KEY, next).unwrap();
    assert!(matches!(
        later[0].fact,
        SessionFact::Heartbeat {
            beat: HeartbeatKind::CallsOpen
        }
    ));
}

#[test]
fn a_malformed_complete_line_is_skipped_and_surfaced() {
    let bench = Bench::new();
    let (_ledger, _child) = ledger_of_a_live_child(&bench);
    let backend = bench.backend();
    let bad = "this is no json\n";
    let good = beat("calls_open");
    std::fs::write(facts_file(&bench), format!("{bad}{good}")).unwrap();

    let (facts, next) = backend.drain_facts(KEY, 0).unwrap();

    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].offset, bad.len() as u64);
    assert_eq!(next, (bad.len() + good.len()) as u64);
    let findings = backend.findings(KEY);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].contains("offset 0"), "{findings:?}");
    assert!(
        !findings[0].contains("this is no json"),
        "a line's text is no finding"
    );
}

#[test]
fn an_offset_past_the_end_reads_nothing_and_is_surfaced() {
    let bench = Bench::new();
    let (_ledger, _child) = ledger_of_a_live_child(&bench);
    let backend = bench.backend();
    std::fs::write(facts_file(&bench), beat("calls_open")).unwrap();

    let (facts, next) = backend.drain_facts(KEY, 10_000).unwrap();

    assert!(facts.is_empty());
    assert_eq!(next, 10_000);
    assert_eq!(backend.findings(KEY).len(), 1);
}

#[test]
fn starting_a_key_twice_spawns_once() {
    let bench = Bench::new();
    let backend = bench.backend();
    let spec = bench.spec(KEY, json!({ "steps": [{ "wait": "never" }] }));

    assert_eq!(backend.start(&spec).unwrap(), StartOutcome::Spawned);
    let pid = bench.child_record(KEY).pid;
    assert_eq!(backend.start(&spec).unwrap(), StartOutcome::Reattached);
    assert_eq!(bench.running_sidecars(), 1);
    kill_hard(pid);
}

#[test]
fn a_spawn_with_no_child_record_yet_waits() {
    let bench = Bench::new();
    let ledger = SessionLedger::new(bench.ledger_dir(KEY));
    ledger.create().unwrap();
    assert!(ledger.claim_spawn(&SpawnRecord::default()).unwrap());

    let spec = bench.spec(KEY, json!({}));
    assert_eq!(bench.backend().start(&spec).unwrap(), StartOutcome::Waiting);
    assert_eq!(bench.running_sidecars(), 0);
}

/// The pid in the ledger now belongs to somebody else, who is alive (must-fix, round 1: CQ #12, TQ #13, IR #15).
#[test]
fn a_recorded_pid_alive_with_another_start_time_is_lost_and_not_signalled() {
    let bench = Bench::new();
    let (ledger, mut foreign) = ledger_of_a_live_child(&bench);
    let pid = foreign.0.id();
    ledger
        .write_child(&ChildRecord {
            pid,
            start_time: "2001-01-01T00:00:00.000Z".to_owned(),
            handlers: true,
        })
        .unwrap();
    let backend = bench.backend();

    assert_eq!(
        backend.start(&bench.spec(KEY, json!({}))).unwrap(),
        StartOutcome::Lost
    );
    backend.stop(KEY, true).unwrap();
    assert!(pid_alive(pid), "a foreign process was signalled");
    assert!(foreign.0.try_wait().unwrap().is_none());
}

#[test]
fn a_recorded_child_that_is_dead_is_lost() {
    let bench = Bench::new();
    let (_ledger, mut child) = ledger_of_a_live_child(&bench);
    child.end();
    assert_eq!(
        bench.backend().start(&bench.spec(KEY, json!({}))).unwrap(),
        StartOutcome::Lost
    );
}

/// A session that finished while nobody watched left its own end: that is what happened, and `lost` would hide it.
#[test]
fn a_dead_child_whose_ended_line_is_there_is_read_on_not_lost() {
    let bench = Bench::new();
    let (_ledger, mut child) = ledger_of_a_live_child(&bench);
    std::fs::write(
        facts_file(&bench),
        "{\"kind\":\"ended\",\"raw\":{\"kind\":\"result\",\"subtype\":\"success\",\"is_error\":false}}\n",
    )
    .unwrap();
    child.end();
    let backend = bench.backend();
    assert_eq!(
        backend.start(&bench.spec(KEY, json!({}))).unwrap(),
        StartOutcome::Reattached
    );
    let (facts, _) = backend.drain_facts(KEY, 0).unwrap();
    assert!(matches!(
        facts.last().map(|f| &f.fact),
        Some(SessionFact::Ended {
            cause: EndCause::TurnComplete
        })
    ));
}

/// A dead leader with no `ended` line, once the file is drained: and an `ended { lost }` appended after a torn line is
/// read (must-fix, round 4: TQ #15), not swallowed by it.
#[test]
fn a_lost_appended_after_a_torn_line_is_read() {
    let bench = Bench::new();
    let (_ledger, mut child) = ledger_of_a_live_child(&bench);
    let whole = beat("model_output");
    std::fs::write(facts_file(&bench), format!("{whole}{{\"kind\":\"tor")).unwrap();
    child.end();
    let backend = bench.backend();

    // In one read: without its own newline the fact would be swallowed by the torn line, and only said again later.
    let (facts, _) = backend.drain_facts(KEY, 0).unwrap();

    assert!(
        facts.iter().any(|f| matches!(
            f.fact,
            SessionFact::Ended {
                cause: EndCause::Lost
            }
        )),
        "{facts:?}"
    );
    assert_eq!(backend.findings(KEY).len(), 1, "the torn line is a finding");
}

#[test]
fn lost_is_said_once() {
    let bench = Bench::new();
    let (_ledger, mut child) = ledger_of_a_live_child(&bench);
    child.end();
    let backend = bench.backend();
    let (first, offset) = backend.drain_facts(KEY, 0).unwrap();
    let (second, _) = backend.drain_facts(KEY, offset).unwrap();
    assert_eq!(first.len(), 1);
    assert!(second.is_empty(), "{second:?}");
    assert_eq!(
        std::fs::read_to_string(facts_file(&bench))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn no_temp_file_is_left_beside_the_records() {
    let bench = Bench::new();
    let (ledger, child) = ledger_of_a_live_child(&bench);
    ledger
        .write_child(&ChildRecord {
            pid: child.0.id(),
            start_time: procs::start_text(0),
            handlers: true,
        })
        .unwrap();
    let names: Vec<String> = std::fs::read_dir(bench.ledger_dir(KEY))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
}

#[test]
fn the_adapters_record_never_replaces_the_childs_own() {
    let bench = Bench::new();
    let ledger = SessionLedger::new(bench.ledger_dir(KEY));
    ledger.create().unwrap();
    let own = ChildRecord {
        pid: 7,
        start_time: procs::start_text(0),
        handlers: true,
    };
    ledger.write_child(&own).unwrap();
    ledger
        .write_child_unless_there(&ChildRecord {
            pid: 7,
            start_time: procs::start_text(0),
            handlers: false,
        })
        .unwrap();
    assert_eq!(ledger.child(), Some(own));
}
