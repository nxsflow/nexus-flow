//! The real adapter under the real host and the real reducer (epic DoD 9, §12.7): a real child and a real tool process,
//! a simulated clock for the timers, and a sample at every tick. Nothing rests on how long anything really takes: where
//! the test needs something to have happened it waits for it to have happened, and only then moves the clock.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nexus_run_contracts::{Decision, EndCause, EventBody, ReasonCode, RunId, Secs, SessionFact};
use nexus_run_host::test_support::scripted_backend::Cue;
use nexus_run_host::test_support::{Rig, CLOCK_START};
use nexus_run_host::{Host, Ports};
use nexus_run_session_claude::ClaudeBackend;

const HANG_CEILING: Duration = Duration::from_secs(300);

/// A sidecar with no SDK: declares itself, says `started` and an open call, and starts the tool the directory holds, in a
/// session of its own, as the CLI starts one.
const FAKE: &str = r#"
import { spawn } from "node:child_process";
import { appendFileSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
const spec = JSON.parse(readFileSync(process.argv[process.argv.indexOf("--spec") + 1], "utf8"));
const ledger = spec.ledger_dir;
process.on("SIGTERM", () => process.exit(143));
process.on("SIGUSR2", () => appendFileSync(join(ledger, "signals"), "USR2\n"));
writeFileSync(join(ledger, "c.tmp"), JSON.stringify({ pid: process.pid, start_time: new Date(Date.now() - process.uptime() * 1000).toISOString(), handlers: true }));
renameSync(join(ledger, "c.tmp"), join(ledger, "child.json"));
const fact = (f) => appendFileSync(join(ledger, "facts.jsonl"), JSON.stringify(f) + "\n");
fact({ kind: "started", backend_session_id: "s-1", pid: process.pid });
fact({ kind: "heartbeat", beat: "calls_open" });
const tool = readFileSync(join(spec.cwd, "tool.sh"), "utf8");
const child = spawn("perl", ["-e", `use POSIX; POSIX::setsid(); open(F, ">${ledger}/tool"); print F $$; close(F); exec("sh", "-c", q(${tool}))`], { stdio: "ignore" });
child.on("error", () => {});
setInterval(() => {}, 1000);
"#;

struct RealRig {
    rig: Rig,
    run: RunId,
    ledger: PathBuf,
    _temp: tempfile::TempDir,
}

fn tool_pid(ledger: &Path) -> Option<u32> {
    std::fs::read_to_string(ledger.join("tool"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

impl RealRig {
    /// A run whose coder is the fake, in a call, with the tool running and the first sample taken.
    fn commissioned(tool: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap().join("rig");
        let mut rig = Rig::scripted(&dir, CLOCK_START, Box::new(|_| Cue::Silent));
        let bundle = dir.join("fake.mjs");
        std::fs::write(&bundle, FAKE).unwrap();
        let layout = rig.host_layout();
        std::fs::create_dir_all(layout.v2_dir()).unwrap();
        std::fs::write(layout.v2_dir().join("tool.sh"), tool).unwrap();
        let backend =
            ClaudeBackend::new(layout.agent_logs.clone(), bundle).with_sample_every(Duration::ZERO);
        rig.host = Host::new(
            rig.store.clone(),
            Ports {
                messages: rig.ports.clone(),
                declarations: rig.ports.clone(),
                sidecar: rig.ports.clone(),
                layout: layout.clone(),
            },
            Arc::new(backend),
            rig.tree.clone(),
            rig.clock.clone(),
        );
        rig.policy.quiet_after = Secs(600);
        rig.policy.subprocess_idle = Secs(1800);
        rig.policy.confirm_within = Secs(300);
        rig.policy.call_bound = Secs(3 * 3600);
        // The clock stands still until the tool runs: `start_deadline` is the fake's to meet, not the machine's.
        rig.step_ms = 0;
        let run = rig.commission("work").run;
        let ledger = layout.ledger_dir(&format!("{run}/build/v1/coder/a1"));
        let mut real = Self {
            rig,
            run,
            ledger,
            _temp: temp,
        };
        real.tick_until("the tool runs and the call is open", |r| {
            tool_pid(&r.ledger).is_some() && r.moved_at().is_some()
        });
        real
    }

    fn tick_until(&mut self, what: &str, mut done: impl FnMut(&Self) -> bool) {
        let limit = Instant::now() + HANG_CEILING;
        while !done(self) {
            assert!(Instant::now() < limit, "never happened: {what}");
            self.rig.tick();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn moved_at(&self) -> Option<i64> {
        let bytes = std::fs::read(self.ledger.join("sample.json")).ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        value.get("moved_at_ms")?.as_i64()
    }

    fn liveness(&self) -> Vec<ReasonCode> {
        self.rig
            .events(&self.run)
            .iter()
            .filter_map(|e| match &e.body {
                EventBody::Decision(Decision::Liveness { reason, .. }) => Some(reason.clone()),
                _ => None,
            })
            .collect()
    }

    fn asks(&self) -> usize {
        std::fs::read_dir(&self.ledger)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("ask."))
            .count()
    }

    fn end_processes(&self) {
        let child: serde_json::Value =
            serde_json::from_slice(&std::fs::read(self.ledger.join("child.json")).unwrap())
                .unwrap();
        let leader = child["pid"].as_u64().unwrap();
        let tool = tool_pid(&self.ledger).unwrap();
        for target in [format!("-{leader}"), tool.to_string()] {
            let _ = Command::new("kill").args(["-KILL", "--", &target]).status();
        }
    }
}

#[test]
fn a_call_whose_tool_only_sleeps_is_nudged_after_subprocess_idle() {
    let mut real = RealRig::commissioned("exec sleep 600");

    real.rig.step_ms = 60_000;
    real.tick_until("the slot went quiet", |r| !r.liveness().is_empty());

    assert_eq!(real.liveness(), vec![ReasonCode::Quiet]);
    real.tick_until("the nudge reached the session", |r| r.asks() == 1);
    real.end_processes();
}

/// A poll loop: nothing but which process runs changes between two looks. Three simulated hours, past `quiet_after`,
/// past `subprocess_idle` and to `call_bound`, and it is never nudged.
#[test]
fn a_call_whose_tool_moves_is_never_nudged() {
    let mut real = RealRig::commissioned("while :; do sleep 0.05; done");

    for _ in 0..36 {
        let before = real.moved_at();
        // The next sample that sees another process, however long the machine takes to run one.
        real.rig.step_ms = 0;
        real.tick_until("the sample moved", |r| r.moved_at() != before);
        real.rig.clock.advance(5 * 60 * 1000);
    }
    real.rig.tick();

    assert_eq!(real.liveness(), Vec::<ReasonCode>::new());
    assert_eq!(real.asks(), 0);
    real.end_processes();
}

#[test]
fn a_killed_sidecar_is_lost_without_the_clock_moving() {
    let mut real = RealRig::commissioned("exec sleep 600");
    let at = real.rig.now();

    real.end_processes();
    real.tick_until("the end reached the log", |r| {
        r.rig.events(&r.run).iter().any(|e| {
            matches!(
                &e.body,
                EventBody::SessionFact {
                    fact: SessionFact::Ended {
                        cause: EndCause::Lost
                    },
                    ..
                }
            )
        })
    });

    assert_eq!(real.rig.now(), at);
    assert_eq!(real.liveness(), Vec::<ReasonCode>::new());
}
