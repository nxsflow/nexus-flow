//! The real bundle in facts mode, the real SDK and the real CLI running a real Bash tool, against a local Messages API, so
//! it needs no credentials and costs nothing. What the sampler's exclusions rest on is where THIS CLI puts its processes
//! (tv58's throwaway run), so this is what a bump of the SDK re-measures.
//!
//! Ignored, because it needs `npm ci && npm run build` in `agent-sidecar/`:
//!
//!     cargo test -p nexus-run-session-claude --test real_sidecar -- --ignored --test-threads=1

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use common::*;
use nexus_run_contracts::{
    EndCause, EnvVar, HeartbeatKind, Liveness, SessionBackend, SessionFact, StartOutcome,
};
use nexus_run_session_claude::sample::SampleRecord;
use nexus_run_session_claude::ClaudeBackend;
use serde_json::json;

struct Api(Child, String);

impl Drop for Api {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_api(dir: &Path) -> Api {
    let port_file = dir.join("port");
    let child = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-messages-api.mjs"))
        .arg(&port_file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("node runs");
    until("the API listens", || {
        std::fs::read_to_string(&port_file).is_ok_and(|text| !text.is_empty())
    });
    let port = std::fs::read_to_string(&port_file).unwrap();
    Api(child, format!("http://127.0.0.1:{port}"))
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The directory of the native `claude` the SDK's platform package brought.
fn claude_dir() -> PathBuf {
    let scope = repository().join("agent-sidecar/node_modules/@anthropic-ai");
    std::fs::read_dir(&scope)
        .expect("agent-sidecar's dependencies: npm ci")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("claude").is_file())
        .expect("a platform package holding `claude`")
}

fn backend(bench: &Bench) -> ClaudeBackend {
    ClaudeBackend::new(
        bench.logs.clone(),
        repository().join("agent-sidecar/dist/nxc-agent-sidecar.mjs"),
    )
}

fn real_spec(bench: &Bench, api: &Api, command: &str) -> nexus_run_contracts::StartSpec {
    let mut spec = bench.spec(KEY, json!({}));
    let path = std::env::var("PATH").unwrap_or_default();
    spec.env = vec![
        EnvVar {
            name: "PATH".to_owned(),
            value: format!("{}:{path}", claude_dir().display()),
        },
        EnvVar {
            name: "HOME".to_owned(),
            value: bench.logs.to_string_lossy().into_owned(),
        },
        EnvVar {
            name: "ANTHROPIC_BASE_URL".to_owned(),
            value: api.1.clone(),
        },
        EnvVar {
            name: "ANTHROPIC_API_KEY".to_owned(),
            value: "not-a-key".to_owned(),
        },
        EnvVar {
            name: "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_owned(),
            value: "1".to_owned(),
        },
    ];
    let call =
        json!({ "input": { "command": command, "timeout": 600_000, "description": "long" } });
    spec.turn.text = format!("RUN::{call}");
    spec.model = Some("claude-haiku-4-5".to_owned());
    spec.permissions = Some("bypassPermissions".to_owned());
    spec.tools = vec!["Bash".to_owned()];
    spec
}

fn facts_until(
    backend: &ClaudeBackend,
    seen: &mut Vec<SessionFact>,
    offset: &mut u64,
    what: &str,
    done: impl Fn(&[SessionFact]) -> bool,
) {
    until(what, || {
        let (drained, next) = backend.drain_facts(KEY, *offset).expect("a drain");
        *offset = next;
        seen.extend(drained.into_iter().map(|d| d.fact));
        done(seen)
    });
}

fn sample_record(bench: &Bench) -> SampleRecord {
    serde_json::from_slice(&std::fs::read(bench.ledger_dir(KEY).join("sample.json")).unwrap())
        .unwrap()
}

fn is_beat(fact: &SessionFact, beat: &HeartbeatKind) -> bool {
    matches!(fact, SessionFact::Heartbeat { beat: b } if b == beat)
}

/// The whole of one session: a stop with the CLI's own help, and a sampler that leaves the CLI out.
#[test]
#[ignore = "needs the built bundle: see the head of this file"]
fn a_real_session_in_a_long_bash_call_is_seen_stopped_and_confirmed() {
    let bench = Bench::new();
    let api = start_api(&bench.logs);
    let backend = backend(&bench);
    let spec = real_spec(&bench, &api, "sleep 600");
    assert_eq!(backend.start(&spec).unwrap(), StartOutcome::Spawned);

    let (mut seen, mut offset) = (Vec::new(), 0);
    facts_until(
        &backend,
        &mut seen,
        &mut offset,
        "started and a call",
        |f| {
            f.iter().any(|f| matches!(f, SessionFact::Started { .. }))
                && f.iter().any(|f| is_beat(f, &HeartbeatKind::CallsOpen))
        },
    );
    until("the tool is in the sample", || {
        backend.sample_now(KEY).unwrap();
        !sample_record(&bench).work.is_empty()
    });
    let tool: Vec<u32> = sample_record(&bench).work.into_keys().collect();
    let leader = bench.child_record(KEY).pid;
    // Neither the leader nor the CLI nor anything else of its group: their CPU is what hid hangs.
    for pid in &tool {
        let output = Command::new("ps")
            .args(["-o", "pgid=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let pgid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        assert_ne!(pgid, leader.to_string(), "{pid} is in the leader's group");
    }

    // A tool that only sleeps stops moving.
    let mut still = 0;
    until("the sample settled", || {
        let before = sample_record(&bench).work;
        backend.sample_now(KEY).unwrap();
        still = if sample_record(&bench).work == before {
            still + 1
        } else {
            0
        };
        still >= 3
    });

    backend.stop(KEY, false).unwrap();
    until("the end is confirmed", || {
        matches!(backend.alive(KEY), Ok(Liveness::Dead))
    });
    for pid in tool {
        assert!(
            !pid_alive(pid),
            "the tool's process {pid} outlived the stop"
        );
    }
    facts_until(&backend, &mut seen, &mut offset, "the end", |f| {
        f.iter().any(|f| matches!(f, SessionFact::Ended { .. }))
    });
    assert!(
        seen.iter().any(|f| matches!(
            f,
            SessionFact::Ended {
                cause: EndCause::StoppedByHost
            }
        )),
        "{seen:?}"
    );
}

/// With no SIGTERM first the CLI cannot end its tools (measured): the adapter has to reach them by pid.
#[test]
#[ignore = "needs the built bundle: see the head of this file"]
fn a_forced_stop_of_a_real_session_ends_the_tool_the_group_signal_cannot_reach() {
    let bench = Bench::new();
    let api = start_api(&bench.logs);
    let backend = backend(&bench);
    backend
        .start(&real_spec(&bench, &api, "sleep 600"))
        .unwrap();
    until("the tool is in the sample", || {
        backend.sample_now(KEY).unwrap();
        !sample_record(&bench).work.is_empty()
    });
    let tool: Vec<u32> = sample_record(&bench).work.into_keys().collect();

    backend.stop(KEY, true).unwrap();

    until("the end is confirmed", || {
        matches!(backend.alive(KEY), Ok(Liveness::Dead))
    });
    for pid in tool {
        assert!(
            !pid_alive(pid),
            "the tool's process {pid} outlived the kill"
        );
    }
}
