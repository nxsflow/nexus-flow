//! What the adapter's tests share: a start spec, the fake sidecar and a wait that has only a hang ceiling.
#![allow(dead_code)] // Every test binary compiles this file and uses a different part of it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use nexus_run_contracts::{EnvVar, Prompt, SessionBackend, SessionFact, StartSpec, Turn};
use nexus_run_session_claude::ledger::ChildRecord;
use nexus_run_session_claude::ClaudeBackend;

/// What ends a hung test, and nothing else: no assertion rests on it.
pub const HANG_CEILING: Duration = Duration::from_secs(300);

pub const KEY: &str = "run1/build/v1/coder/a1";

pub fn fake_sidecar() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-sidecar.mjs")
}

/// A process of a test's own, ended and reaped however the test ends, so a red test leaves nothing behind.
pub struct Reaped(pub std::process::Child);

impl Reaped {
    pub fn end(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for Reaped {
    fn drop(&mut self) {
        self.end();
    }
}

/// A ledger, its spec and a backend on them, in a directory that dies with the test.
pub struct Bench {
    pub temp: tempfile::TempDir,
    pub logs: PathBuf,
}

impl Bench {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("a temp directory");
        let logs = temp.path().canonicalize().expect("a real path");
        Self { temp, logs }
    }

    pub fn backend(&self) -> ClaudeBackend {
        ClaudeBackend::new(self.logs.clone(), fake_sidecar())
    }

    pub fn ledger_dir(&self, key: &str) -> PathBuf {
        self.logs.join("v2").join(key)
    }

    pub fn spec(&self, key: &str, script: serde_json::Value) -> StartSpec {
        let var = |name: &str| EnvVar {
            name: name.to_owned(),
            value: std::env::var(name).unwrap_or_default(),
        };
        StartSpec {
            v: 1,
            session_key: key.to_owned(),
            cwd: self.logs.to_string_lossy().into_owned(),
            env: vec![var("PATH"), var("HOME")],
            prompt: Prompt {
                system_prompt: String::new(),
                use_claude_code_preset: false,
                declaration_hash: "h".to_owned(),
            },
            turn: Turn {
                thread: "m-thread".to_owned(),
                text: script.to_string(),
            },
            resume: None,
            model: None,
            tools: Vec::new(),
            permissions: None,
            ledger_dir: self.ledger_dir(key).to_string_lossy().into_owned(),
        }
    }

    /// A cue the fake sidecar is waiting on.
    pub fn cue(&self, key: &str, name: &str) {
        std::fs::write(self.ledger_dir(key).join(name), b"").expect("the cue");
    }

    /// The processes of this bench alive right now: their command line names its spec.
    pub fn running_sidecars(&self) -> usize {
        let output = Command::new("ps")
            .args(["-A", "-ww", "-o", "stat=", "-o", "command="])
            .output()
            .expect("ps runs");
        let marker = self.logs.to_string_lossy().into_owned();
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| line.contains(&marker) && line.contains("--facts"))
            .filter(|line| !line.trim_start().starts_with('Z'))
            .count()
    }

    /// The record of a child that has its signal handlers in place, written by the child itself.
    pub fn child_record(&self, key: &str) -> ChildRecord {
        let path = self.ledger_dir(key).join("child.json");
        let read =
            || -> Option<ChildRecord> { serde_json::from_slice(&std::fs::read(&path).ok()?).ok() };
        until("the child's own record", || {
            read().is_some_and(|record| record.handlers)
        });
        read().expect("a record")
    }
}

/// The pid a step of the fake sidecar left in the ledger: the file is there before its bytes are.
pub fn left_pid(bench: &Bench, key: &str, name: &str) -> u32 {
    let path = bench.ledger_dir(key).join(name);
    until("the pid was left", || {
        std::fs::read_to_string(&path).is_ok_and(|text| !text.trim().is_empty())
    });
    std::fs::read_to_string(path)
        .expect("the pid's file")
        .trim()
        .parse()
        .expect("a pid")
}

/// Waits for what a process elsewhere does, however long a loaded machine takes.
pub fn until(what: &str, mut done: impl FnMut() -> bool) {
    let limit = Instant::now() + HANG_CEILING;
    while !done() {
        assert!(Instant::now() < limit, "never happened: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Reads on from `from` until an `ended` arrives; the facts and the next offset.
pub fn drain_until_ended(backend: &ClaudeBackend, key: &str, from: u64) -> (Vec<SessionFact>, u64) {
    let mut facts = Vec::new();
    let mut offset = from;
    until("an ended fact", || {
        let (drained, next) = backend.drain_facts(key, offset).expect("a drain");
        offset = next;
        facts.extend(drained.into_iter().map(|d| d.fact));
        facts.iter().any(|f| matches!(f, SessionFact::Ended { .. }))
    });
    (facts, offset)
}

pub fn pid_alive(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    let stat = String::from_utf8_lossy(&output.stdout);
    !stat.trim().is_empty() && !stat.trim().starts_with('Z')
}

pub fn kill_hard(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status();
}
