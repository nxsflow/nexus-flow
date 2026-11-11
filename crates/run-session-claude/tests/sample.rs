//! The sampler (§6.5, tv58's throwaway run). The tables are what the real CLI's process tree looked like on macOS and
//! Linux; the real children prove the same through the system's answers. No test here rests on a CPU counter moving by
//! itself, or on how long anything takes: a process appears because the test says so.

mod common;

use std::collections::BTreeMap;

use common::*;
use nexus_run_contracts::{HeartbeatKind, Liveness, SessionBackend, SessionFact};
use nexus_run_session_claude::procs::{ProcIdentity, ProcInfo};
use nexus_run_session_claude::sample::{moved, work_of};
use serde_json::json;

const LEADER: u32 = 100;
const START: i64 = 1_000_000;

fn proc(pid: u32, ppid: u32, pgid: u32, sid: u32, cpu: u64) -> ProcInfo {
    ProcInfo {
        pid,
        ppid,
        pgid,
        sid,
        start_ms: START + 1_000,
        cpu,
        zombie: false,
    }
}

/// The tree of a session in a call, as measured: leader, CLI, a stdio MCP server, a Bash tool's shell.
fn session() -> Vec<ProcInfo> {
    vec![
        proc(LEADER, 1, LEADER, 50, 1),
        proc(101, LEADER, LEADER, 50, 900), // the CLI
        proc(102, 101, LEADER, 50, 900),    // a stdio MCP server
        proc(200, 101, 200, 200, 5),        // a Bash tool's shell: session and group of its own
        proc(201, 200, 201, 200, 0),        // its child, a group of its own on Linux
    ]
}

fn work_pids(all: &[ProcInfo], recorded: &[ProcIdentity]) -> Vec<u32> {
    work_of(all, LEADER, START, recorded).into_keys().collect()
}

#[test]
fn the_cli_and_an_mcp_server_are_not_work_and_a_tools_processes_are() {
    assert_eq!(work_pids(&session(), &[]), vec![200, 201]);
}

#[test]
fn the_cli_and_an_mcp_server_cannot_hide_a_hang() {
    let before = work_of(&session(), LEADER, START, &[]);
    let mut later = session();
    later[1].cpu = 5_000; // the CLI waits for a tool, spending CPU as it does
    later[2].cpu = 5_000; // and so does the MCP server
    later[0].cpu = 50;
    assert!(!moved(&before, &work_of(&later, LEADER, START, &[])));
}

#[test]
fn a_process_that_spends_cpu_is_movement() {
    let before = work_of(&session(), LEADER, START, &[]);
    let mut later = session();
    later[4].cpu += 1;
    assert!(moved(&before, &work_of(&later, LEADER, START, &[])));
}

/// k6nf, measured on Linux: `sleep 60; gh ...` spends less than a tick, so count and CPU read the same at every look.
#[test]
fn a_poll_loop_moves_by_which_processes_run_with_count_and_cpu_the_same() {
    let before = work_of(&session(), LEADER, START, &[]);
    let mut later = session();
    later[4].pid = 301; // the next `sleep`: same count, same CPU, another process
    assert!(moved(&before, &work_of(&later, LEADER, START, &[])));
}

/// Measured with vitest: a worker's child that reparents to `init` leaves the lineage at once and keeps the session.
#[test]
fn a_child_reparented_to_init_is_found_by_the_session_of_the_shell_it_came_from() {
    let mut all = session();
    all.push(proc(202, 1, 201, 200, 7));
    assert_eq!(work_pids(&all, &[]), vec![200, 201, 202]);
}

/// Measured with vitest: `setsid` in a worker: below the parent while it lives, and only a record can follow it after.
#[test]
fn a_child_in_a_session_of_its_own_is_seen_below_its_parent_and_followed_by_the_record() {
    let mut all = session();
    all.push(proc(203, 201, 203, 203, 7));
    let seen = work_of(&all, LEADER, START, &[]);
    assert!(seen.contains_key(&203));

    let recorded: Vec<ProcIdentity> = seen
        .iter()
        .map(|(pid, (start_ms, _))| ProcIdentity {
            pid: *pid,
            start_ms: *start_ms,
        })
        .collect();
    let mut after = all.clone();
    after.retain(|p| p.pid != 201);
    after.iter_mut().find(|p| p.pid == 203).unwrap().ppid = 1;

    assert!(work_of(&after, LEADER, START, &recorded).contains_key(&203));
    assert!(!work_of(&after, LEADER, START, &[]).contains_key(&203));
}

/// A pid the system handed to somebody else is no work of this session.
#[test]
fn a_recorded_pid_with_another_start_is_dropped() {
    let mut all = session();
    all.push(ProcInfo {
        start_ms: START + 3_600_000,
        ..proc(400, 1, 400, 400, 0)
    });
    let recorded = [ProcIdentity {
        pid: 400,
        start_ms: START,
    }];
    assert!(!work_pids(&all, &recorded).contains(&400));
}

#[test]
fn what_started_before_the_session_is_not_its_work() {
    let mut all = session();
    all.push(ProcInfo {
        start_ms: START - 3_600_000,
        ..proc(500, 101, 500, 500, 0)
    });
    assert!(!work_pids(&all, &[]).contains(&500));
}

#[test]
fn a_zombie_is_no_work_and_nothing_moves_when_it_goes() {
    let mut all = session();
    all.push(ProcInfo {
        zombie: true,
        ..proc(600, 200, 201, 200, 0)
    });
    assert!(!work_pids(&all, &[]).contains(&600));
}

/// A leader dead and its CLI reparented, still in the group (measured): its tool is still found below it.
#[test]
fn with_the_leader_dead_the_work_is_found_below_what_is_left_of_its_group() {
    let mut all = session();
    all.remove(0);
    all[0].ppid = 1;
    assert_eq!(work_pids(&all, &[]), vec![200, 201]);
}

/// A process of another start at the leader's pid: nothing below it, and nothing in its group, is this session's.
#[test]
fn a_foreign_process_at_the_leaders_pid_gives_no_work() {
    let mut all = session();
    all[0].start_ms = START + 3_600_000;
    assert!(work_pids(&all, &[]).is_empty());
}

#[test]
fn nothing_moved_means_equal_samples_and_an_empty_one_compares_too() {
    let empty = BTreeMap::new();
    assert!(!moved(&empty, &empty));
    let some = work_of(&session(), LEADER, START, &[]);
    assert!(moved(&empty, &some));
    assert!(moved(&some, &empty));
}

// ---- real processes --------------------------------------------------------------------------

fn subprocess_moves(facts: &[SessionFact]) -> usize {
    facts
        .iter()
        .filter(|f| {
            matches!(
                f,
                SessionFact::Heartbeat {
                    beat: HeartbeatKind::SubprocessMoved
                }
            )
        })
        .count()
}

fn sampled_moves(
    bench: &Bench,
    key: &str,
    backend: &nexus_run_session_claude::ClaudeBackend,
    offset: &mut u64,
) -> usize {
    backend.sample_now(key).unwrap();
    let (drained, next) = backend.drain_facts(key, *offset).unwrap();
    *offset = next;
    let _ = bench;
    subprocess_moves(&drained.into_iter().map(|d| d.fact).collect::<Vec<_>>())
}

fn backend_work(bench: &Bench) -> Vec<u32> {
    let record: nexus_run_session_claude::sample::SampleRecord =
        serde_json::from_slice(&std::fs::read(bench.ledger_dir(KEY).join("sample.json")).unwrap())
            .unwrap();
    record.work.into_keys().collect()
}

/// Starts a shell in a session of its own, as the CLI starts a tool, and leaves what it says on stderr out of the way.
fn tool_shell(script: &str) -> serde_json::Value {
    json!({ "spawn": format!(
        "exec perl -e 'use POSIX; POSIX::setsid(); exec(\"sh\", \"-c\", q({script}))'"
    ), "pid_file": "tool" })
}

/// §12.7 with a real child: a tool call that only sleeps stops moving, with no open call or with one.
#[test]
fn a_real_tool_that_only_sleeps_reaches_no_movement() {
    let bench = Bench::new();
    let backend = bench.backend();
    backend
        .start(&bench.spec(
            KEY,
            json!({ "steps": [tool_shell("exec sleep 600"), { "wait": "never" }] }),
        ))
        .unwrap();
    let pid = bench.child_record(KEY).pid;
    let tool = left_pid(&bench, KEY, "tool");

    // It settles once it sleeps, and however long that takes on this machine it then stays settled. Only a sampler that
    // moves by itself never gets here, and that is red at the ceiling.
    let mut offset = 0;
    let mut still = 0;
    until("the tool is seen and the sample stays settled", || {
        let moves = sampled_moves(&bench, KEY, &backend, &mut offset);
        let seen = !backend_work(&bench).is_empty();
        still = if moves == 0 && seen { still + 1 } else { 0 };
        still >= 20
    });
    kill_hard(tool);
    kill_hard(pid);
}

/// The same tool, and a new process appears because the test says so: no CPU counter is involved.
#[test]
fn a_real_tool_with_a_new_process_moves_though_no_call_is_open() {
    let bench = Bench::new();
    let backend = bench.backend();
    let cue = bench.ledger_dir(KEY).join("next");
    let script = format!(
        "while [ ! -e {0} ]; do sleep 0.05; done; exec sleep 600",
        cue.display()
    );
    backend
        .start(&bench.spec(
            KEY,
            json!({ "steps": [tool_shell(&script), { "wait": "never" }] }),
        ))
        .unwrap();
    let pid = bench.child_record(KEY).pid;
    let mut offset = 0;
    until("the tool is seen", || {
        sampled_moves(&bench, KEY, &backend, &mut offset) > 0
    });
    std::fs::write(&cue, b"").unwrap();

    until("the lineup moved", || {
        sampled_moves(&bench, KEY, &backend, &mut offset) > 0
    });

    let tool = left_pid(&bench, KEY, "tool");
    assert!(matches!(
        backend.alive(KEY),
        Ok(Liveness::Alive {
            moved_at: Some(_),
            ..
        })
    ));
    kill_hard(tool);
    kill_hard(pid);
}

/// The member of the leader's group never counts: this is what makes a real MCP server unable to hide a hang.
#[test]
fn a_real_process_of_the_leaders_group_is_never_movement() {
    let bench = Bench::new();
    let backend = bench.backend();
    backend
        .start(&bench.spec(
            KEY,
            json!({ "steps": [{ "spawn": "while :; do sleep 0.02; done", "pid_file": "server" }, { "wait": "never" }] }),
        ))
        .unwrap();
    let pid = bench.child_record(KEY).pid;
    let member = left_pid(&bench, KEY, "server");

    let mut offset = 0;
    for _ in 0..5 {
        assert_eq!(sampled_moves(&bench, KEY, &backend, &mut offset), 0);
    }
    backend.stop(KEY, true).unwrap();
    until("the group is gone", || {
        !pid_alive(member) && !pid_alive(pid)
    });
}

/// Every sample is a fact in the same file with the same key scheme, and the sample's clock is the minute.
#[test]
fn the_sample_runs_on_its_own_only_when_a_minute_has_passed() {
    let bench = Bench::new();
    let backend = bench.backend();
    backend
        .start(&bench.spec(
            KEY,
            json!({ "steps": [tool_shell("exec sleep 600"), { "wait": "never" }] }),
        ))
        .unwrap();
    let pid = bench.child_record(KEY).pid;
    let tool = left_pid(&bench, KEY, "tool");
    let sample = bench.ledger_dir(KEY).join("sample.json");

    let (_, _) = backend.drain_facts(KEY, 0).unwrap();
    let first = std::fs::read(&sample).expect("the first drain samples");
    for _ in 0..3 {
        let _ = backend.drain_facts(KEY, 0).unwrap();
    }
    assert_eq!(
        std::fs::read(&sample).unwrap(),
        first,
        "sampled again inside the minute"
    );

    kill_hard(tool);
    kill_hard(pid);
}

/// PROBE (6j6v.nap3), removed before hand-over: how often the scenario of
/// `a_real_tool_with_a_new_process_moves_though_no_call_is_open` sees no movement after its cue within a short wait,
/// what the first sample and the last one held, and whether a tool that execs in place without a new process moves.
#[test]
fn probe_lineup_after_the_cue() {
    fn stat_of(pid: u32) -> String {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_else(|_| "gone".to_owned())
    }
    let rounds = 150;
    let mut never = Vec::new();
    for round in 0..rounds {
        let bench = Bench::new();
        let backend = bench.backend();
        let cue = bench.ledger_dir(KEY).join("next");
        let script = format!(
            "while [ ! -e {0} ]; do sleep 0.05; done; exec sleep 600",
            cue.display()
        );
        backend
            .start(&bench.spec(
                KEY,
                json!({ "steps": [tool_shell(&script), { "wait": "never" }] }),
            ))
            .unwrap();
        let pid = bench.child_record(KEY).pid;
        let mut offset = 0;
        until("the tool is seen", || {
            sampled_moves(&bench, KEY, &backend, &mut offset) > 0
        });
        let first = backend_work(&bench);
        std::fs::write(&cue, b"").unwrap();
        let limit = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut moved = false;
        while std::time::Instant::now() < limit {
            if sampled_moves(&bench, KEY, &backend, &mut offset) > 0 {
                moved = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let tool = left_pid(&bench, KEY, "tool");
        if !moved {
            let record: nexus_run_session_claude::sample::SampleRecord = serde_json::from_slice(
                &std::fs::read(bench.ledger_dir(KEY).join("sample.json")).unwrap(),
            )
            .unwrap();
            never.push(format!(
                "round={round} tool={tool} first={first:?} last={:?} tool_stat=[{}]",
                record.work,
                stat_of(tool).trim()
            ));
        }
        kill_hard(tool);
        kill_hard(pid);
    }

    // The same exec in place with the cue there before the tool starts: no new process ever appears.
    let mut forced_moves = 0;
    let forced_rounds = 30;
    for _ in 0..forced_rounds {
        let bench = Bench::new();
        let backend = bench.backend();
        let cue = bench.logs.join("ready");
        std::fs::write(&cue, b"").unwrap();
        let script = format!(
            "while [ ! -e {0} ]; do sleep 0.05; done; exec sleep 600",
            cue.display()
        );
        backend
            .start(&bench.spec(
                KEY,
                json!({ "steps": [tool_shell(&script), { "wait": "never" }] }),
            ))
            .unwrap();
        let pid = bench.child_record(KEY).pid;
        let mut offset = 0;
        until("the tool is seen", || {
            sampled_moves(&bench, KEY, &backend, &mut offset) > 0
        });
        for _ in 0..40 {
            forced_moves += sampled_moves(&bench, KEY, &backend, &mut offset);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let tool = left_pid(&bench, KEY, "tool");
        kill_hard(tool);
        kill_hard(pid);
    }
    panic!(
        "probe-result no_movement_after_cue={}/{rounds} forced_exec_in_place_moves={forced_moves} over {forced_rounds} rounds of 40 samples; cases: {never:#?}",
        never.len()
    );
}
