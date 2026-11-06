//! Stop, reaping and the confirmed end (§8.3, §12.7, §12.8), through a spawner that stays alive and reaps: this test
//! process. An end is confirmed only with the leader dead and reaped, the group empty and the session's work gone.

mod common;

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use common::*;
use nexus_run_contracts::{EndCause, Liveness, SessionBackend, SessionFact};
use nexus_run_session_claude::ledger::{ChildRecord, SessionLedger, SpawnRecord};
use nexus_run_session_claude::procs::{self, GroupState, ProcIdentity, Signal};
use serde_json::json;

fn group_is_gone(pgid: u32) -> bool {
    !Command::new("ps")
        .args(["-A", "-o", "pgid="])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l.trim() == pgid.to_string())
        })
        .unwrap_or(true)
}

/// The pid a step of the fake sidecar left in the ledger.
fn left_pid(bench: &Bench, name: &str) -> u32 {
    let path = bench.ledger_dir(KEY).join(name);
    until("the pid was left", || {
        std::fs::read_to_string(&path).is_ok_and(|text| !text.trim().is_empty())
    });
    std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// A shell in a session of its own that leaves its pid and sleeps: the shape of a Bash tool's shell (tv58 step 0).
fn own_session(pid_file: &str) -> serde_json::Value {
    json!({ "spawn": format!(
        "exec perl -e 'use POSIX; POSIX::setsid(); open(F, \">{pid_file}\"); print F $$; close(F); exec(\"sleep\", \"600\")'"
    ) })
}

fn until_dead(backend: &nexus_run_session_claude::ClaudeBackend) {
    until("a confirmed end", || {
        matches!(backend.alive(KEY), Ok(Liveness::Dead))
    });
}

#[test]
fn a_stop_ends_the_leader_and_reaps_it_and_only_then_says_dead() {
    let bench = Bench::new();
    let backend = bench.backend();
    backend
        .start(&bench.spec(KEY, json!({ "steps": [{ "wait": "never" }] })))
        .unwrap();
    let pid = bench.child_record(KEY).pid;

    backend.stop(KEY, false).unwrap();
    until_dead(&backend);

    assert!(!pid_alive(pid));
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    assert!(output.stdout.is_empty(), "a zombie was left behind");
    assert!(group_is_gone(pid));
    assert_eq!(procs::group_state(pid, 0), GroupState::Empty);
}

/// An unreaped leader answers `kill(pid, 0)` alive and `kill(-pgid, 0)` `EPERM` or success: neither is an end. Reaping is
/// what `alive` does first, so this reads the group with the leader still unreaped.
#[test]
fn an_unreaped_leader_is_not_a_confirmed_end() {
    let bench = Bench::new();
    let spec = bench.spec(KEY, json!({ "steps": [{ "exit": 0 }] }));
    let backend = bench.backend();
    backend.start(&spec).unwrap();
    let record = bench.child_record(KEY);
    until("the leader is a zombie", || !pid_alive(record.pid));

    assert_ne!(
        procs::group_state(record.pid, 0),
        GroupState::Empty,
        "an unreaped leader read as an empty group"
    );
    until_dead(&backend);
    assert_eq!(procs::group_state(record.pid, 0), GroupState::Empty);
}

#[test]
fn a_leader_that_ignores_term_is_not_dead_until_the_forced_stop() {
    let bench = Bench::new();
    let backend = bench.backend();
    backend
        .start(&bench.spec(
            KEY,
            json!({ "ignore_term": true, "steps": [{ "wait": "never" }] }),
        ))
        .unwrap();
    let pid = bench.child_record(KEY).pid;
    let signals = bench.ledger_dir(KEY).join("signals");

    backend.stop(KEY, false).unwrap();
    until("the term arrived", || signals.exists());

    assert!(matches!(backend.alive(KEY), Ok(Liveness::Alive { .. })));
    assert!(pid_alive(pid));
    backend.stop(KEY, true).unwrap();
    until_dead(&backend);
}

/// §12.7: a leader that exits on SIGTERM while a descendant lives is no confirmed end, and says so as a fact.
#[test]
fn a_dead_leader_with_a_live_member_of_its_group_is_lingering_not_ended() {
    let bench = Bench::new();
    let backend = bench.backend();
    backend
        .start(&bench.spec(
            KEY,
            json!({ "steps": [
                { "spawn": "exec sleep 600", "pid_file": "member" },
                { "wait": "go" },
                { "fact": { "kind": "ended", "raw": { "kind": "result", "subtype": "success", "is_error": false } } },
                { "exit": 0 }
            ] }),
        ))
        .unwrap();
    let member = left_pid(&bench, "member");
    let leader = bench.child_record(KEY).pid;
    bench.cue(KEY, "go");
    until("the leader ended", || !pid_alive(leader));

    assert!(matches!(backend.alive(KEY), Ok(Liveness::Alive { .. })));
    let (facts, offset) = backend.drain_facts(KEY, 0).unwrap();
    let kinds: Vec<&SessionFact> = facts.iter().map(|f| &f.fact).collect();
    assert!(
        kinds
            .iter()
            .any(|f| matches!(f, SessionFact::GroupLingering { pids } if pids.contains(&member))),
        "{kinds:?}"
    );
    assert!(
        !kinds.iter().any(|f| matches!(f, SessionFact::Ended { .. })),
        "an end before the group is empty: {kinds:?}"
    );
    // Written once, however many ticks it lasts: read again from the held `ended`, it is the same key.
    let lingering = |facts: &[nexus_run_contracts::DrainedFact]| -> Vec<u64> {
        facts
            .iter()
            .filter(|f| matches!(f.fact, SessionFact::GroupLingering { .. }))
            .map(|f| f.offset)
            .collect()
    };
    let (again, offset) = backend.drain_facts(KEY, offset).unwrap();
    assert_eq!(lingering(&again), lingering(&facts));
    assert_eq!(
        std::fs::read_to_string(bench.ledger_dir(KEY).join("facts.jsonl"))
            .unwrap()
            .matches("group_lingering")
            .count(),
        1
    );

    kill_hard(member);
    until_dead(&backend);
    let (end, _) = backend.drain_facts(KEY, offset).unwrap();
    assert!(
        end.iter().any(|f| matches!(
            f.fact,
            SessionFact::Ended {
                cause: EndCause::TurnComplete
            }
        )),
        "{end:?}"
    );
}

/// Measured: SIGKILL of the group leaves a tool's shell alive, because it is in a session of its own. So the work goes
/// by pid, and the end is not confirmed before it is gone.
#[test]
fn a_forced_stop_also_ends_the_work_outside_the_group_and_dead_waits_for_it() {
    let bench = Bench::new();
    let backend = bench.backend();
    backend
        .start(&bench.spec(
            KEY,
            json!({ "steps": [own_session("tool"), { "wait": "never" }] }),
        ))
        .unwrap();
    let tool = left_pid(&bench, "tool");

    backend.stop(KEY, true).unwrap();
    until_dead(&backend);

    assert!(!pid_alive(tool), "the tool's process outlived the stop");
}

#[test]
fn a_tools_process_that_outlives_its_session_holds_the_end_back() {
    let bench = Bench::new();
    let backend = bench.backend();
    backend
        .start(&bench.spec(
            KEY,
            json!({ "steps": [own_session("tool"), { "wait": "go" }, { "exit": 0 }] }),
        ))
        .unwrap();
    let tool = left_pid(&bench, "tool");
    let leader = bench.child_record(KEY).pid;
    backend.sample_now(KEY).unwrap(); // while its parent lives: what lets it be followed once it has none
    bench.cue(KEY, "go");
    until("the leader ended", || !pid_alive(leader));

    assert!(matches!(backend.alive(KEY), Ok(Liveness::Alive { .. })));
    let (facts, _) = backend.drain_facts(KEY, 0).unwrap();
    assert!(
        facts.iter().any(
            |f| matches!(&f.fact, SessionFact::GroupLingering { pids } if pids.contains(&tool))
        ),
        "{facts:?}"
    );
    kill_hard(tool);
    until_dead(&backend);
}

/// The sampled pids live in the ledger: a new reader signals what the old one saw (must-fix, round 3: CQ #17).
#[test]
fn a_restarted_reader_ends_the_work_the_old_one_sampled() {
    let bench = Bench::new();
    let first = bench.backend();
    first
        .start(&bench.spec(
            KEY,
            json!({ "steps": [own_session("tool"), { "wait": "go" }, { "exit": 0 }] }),
        ))
        .unwrap();
    let tool = left_pid(&bench, "tool");
    let leader = bench.child_record(KEY).pid;
    first.sample_now(KEY).unwrap();
    drop(first);
    bench.cue(KEY, "go");
    until("the leader ended", || !pid_alive(leader));

    let second = bench.backend();
    second.stop(KEY, true).unwrap();

    until("the tool's process ended", || !pid_alive(tool));
    until_dead(&second);
}

/// §12.8: a dead leader whose group id a foreign group holds: not signalled, and the stop still resolves.
#[test]
fn a_foreign_group_holding_the_recorded_id_is_not_signalled_and_the_end_resolves() {
    let bench = Bench::new();
    let ledger = SessionLedger::new(bench.ledger_dir(KEY));
    ledger.create().unwrap();
    assert!(ledger.claim_spawn(&SpawnRecord::default()).unwrap());
    let mut foreign = Command::new("sleep");
    foreign
        .arg("600")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut foreign = Reaped(foreign.spawn().unwrap());
    // A member of the foreign group, to prove the group's members are left alone as well.
    let mut member = Command::new("sleep");
    member
        .arg("600")
        .process_group(foreign.0.id() as i32)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut member = Reaped(member.spawn().unwrap());
    ledger
        .write_child(&ChildRecord {
            pid: foreign.0.id(),
            start_time: "2001-01-01T00:00:00.000Z".to_owned(),
            handlers: true,
        })
        .unwrap();
    let backend = bench.backend();

    backend.stop(KEY, true).unwrap();

    assert!(matches!(backend.alive(KEY), Ok(Liveness::Dead)));
    assert!(
        foreign.0.try_wait().unwrap().is_none(),
        "the foreign leader was signalled"
    );
    assert!(
        member.0.try_wait().unwrap().is_none(),
        "the foreign member was signalled"
    );
}

#[test]
fn a_group_of_one_or_zero_is_never_signalled() {
    for pid in [0_u32, 1] {
        let identity = ProcIdentity { pid, start_ms: 0 };
        assert!(!procs::signal_group(&identity, Signal::Kill));
        assert!(!procs::signal_member(&identity, Signal::Kill));
    }
    assert_eq!(procs::group_state(1, 0), GroupState::Empty);
    assert_eq!(procs::group_state(0, 0), GroupState::Empty);
}

/// The guard itself: verified immediately before the kill, with its slack on either side and not past it.
#[test]
fn the_guard_verifies_the_start_time_with_a_slack() {
    let mut child = Reaped(
        Command::new("sleep")
            .arg("600")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let pid = child.0.id();
    let real = procs::info(pid).unwrap().start_ms;
    let at = |start_ms| ProcIdentity { pid, start_ms };
    assert!(procs::verify(&at(real)));
    assert!(procs::verify(&at(real + procs::START_SLACK_MS)));
    assert!(procs::verify(&at(real - procs::START_SLACK_MS)));
    assert!(!procs::verify(&at(real + procs::START_SLACK_MS + 1_000)));
    assert!(!procs::verify(&at(real - procs::START_SLACK_MS - 1_000)));
    assert!(!procs::signal_member(
        &at(real - procs::START_SLACK_MS - 1_000),
        Signal::Kill
    ));
    assert!(child.0.try_wait().unwrap().is_none());
}

#[test]
fn a_nudge_is_an_ask_file_and_a_sigusr2_to_the_leader() {
    let bench = Bench::new();
    let backend = bench.backend();
    backend
        .start(&bench.spec(KEY, json!({ "steps": [{ "wait": "never" }] })))
        .unwrap();
    let pid = bench.child_record(KEY).pid;

    backend.nudge(KEY, "are you there?").unwrap();

    let dir = bench.ledger_dir(KEY);
    let signals = dir.join("signals");
    until("the signal arrived", || signals.exists());
    assert_eq!(std::fs::read_to_string(signals).unwrap(), "USR2\n");
    let asks: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("ask."))
        .collect();
    assert_eq!(asks.len(), 1);
    let ask: serde_json::Value = serde_json::from_slice(&std::fs::read(&asks[0]).unwrap()).unwrap();
    assert_eq!(ask["text"], "are you there?");
    kill_hard(pid);
}

/// The child has not yet said its handlers stand, and Node's default for SIGUSR2 ends a process.
#[test]
fn a_nudge_before_the_child_has_declared_itself_is_refused_and_sends_nothing() {
    let bench = Bench::new();
    let ledger = SessionLedger::new(bench.ledger_dir(KEY));
    ledger.create().unwrap();
    let mut live = Reaped(
        Command::new("sleep")
            .arg("600")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let info = procs::info(live.0.id()).unwrap();
    ledger
        .write_child_unless_there(&ChildRecord {
            pid: live.0.id(),
            start_time: procs::start_text(info.start_ms),
            handlers: false,
        })
        .unwrap();

    assert!(bench.backend().nudge(KEY, "are you there?").is_err());

    assert!(live.0.try_wait().unwrap().is_none(), "it was signalled");
}

#[test]
fn a_nudge_of_a_session_with_no_live_leader_is_refused() {
    let bench = Bench::new();
    let backend = bench.backend();
    backend
        .start(&bench.spec(KEY, json!({ "steps": [{ "exit": 0 }] })))
        .unwrap();
    let pid = bench.child_record(KEY).pid;
    until("the leader ended", || !pid_alive(pid));
    assert!(backend.nudge(KEY, "are you there?").is_err());
}

#[test]
fn it_declares_what_it_can_do() {
    use nexus_run_contracts::Capability::*;
    let caps = Bench::new().backend().capabilities();
    assert_eq!(caps.backend, "claude");
    assert_eq!(
        caps.capabilities,
        vec![
            Resume,
            Nudge,
            Stop,
            Usage,
            PermissionRules,
            SubprocessSample
        ]
    );
}
