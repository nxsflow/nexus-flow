//! What the host's tests share: the crash box from the outside.
#![allow(dead_code)] // Every test binary compiles this file and uses a different part of it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use nexus_run_contracts::{Decision, Event, EventBody, RunId, RunStore, SessionFact, Timestamp};
use nexus_run_host::test_support::scripted_backend::{read_timeline, TimelineEntry};
use nexus_run_host::test_support::{read_port_lines, PortLine, WORKSPACE};
use nexus_run_host::Boundary;
use nexus_run_store_sqlite::SqliteRunStore;

const BOX: &str = env!("CARGO_BIN_EXE_run-host-crashbox");

pub struct Crashbox {
    _temp: tempfile::TempDir,
    dir: PathBuf,
    script: String,
}

impl Crashbox {
    pub fn new(script: &str) -> Self {
        let temp = tempfile::tempdir().expect("a temp directory");
        let dir = temp.path().join("box");
        Self {
            _temp: temp,
            dir,
            script: script.to_owned(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn command(&self, script: &str) -> Command {
        let mut command = Command::new(BOX);
        command
            .arg(&self.dir)
            .arg(script)
            .env_remove("NXR_CRASH_AT")
            .env_remove("NXR_BOUNDARY_COUNTS")
            .stderr(std::process::Stdio::null());
        command
    }

    fn run(&self, crash: Option<(Boundary, u32)>) -> ExitStatus {
        let mut command = self.command(&self.script);
        if let Some((boundary, nth)) = crash {
            command.env("NXR_CRASH_AT", format!("{}:{nth}", boundary.name()));
        }
        command.status().expect("the box runs")
    }

    /// `true` when the box aborted there, `false` when the run finished before that occurrence.
    pub fn run_until_abort(&self, boundary: Boundary, nth: u32) -> bool {
        let status = self.run(Some((boundary, nth)));
        if status.success() {
            return false;
        }
        assert!(
            status.code().is_none(),
            "the box failed, it did not abort at {}:{nth}: {status:?}",
            boundary.name()
        );
        true
    }

    /// How often a clean run of this script passes each boundary.
    pub fn count_boundaries(&self) -> BTreeMap<String, u32> {
        let counts = self.dir.with_extension("counts");
        let mut command = self.command(&self.script);
        command.env("NXR_BOUNDARY_COUNTS", &counts);
        assert!(
            command.status().expect("the box runs").success(),
            "a clean run"
        );
        let mut seen = BTreeMap::new();
        for line in std::fs::read_to_string(&counts).unwrap_or_default().lines() {
            if !line.starts_with("write ") {
                *seen.entry(line.to_owned()).or_insert(0) += 1;
            }
        }
        seen
    }

    /// A clean run's boundaries and writes, in order.
    pub fn journal(&self) -> Vec<String> {
        let path = self.dir.with_extension("counts");
        let mut command = self.command(&self.script);
        command.env("NXR_BOUNDARY_COUNTS", &path);
        assert!(
            command.status().expect("the box runs").success(),
            "a clean run"
        );
        std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    pub fn restart_and_run_to_completion(&self) -> Log {
        let status = self.run(None);
        assert!(
            status.success(),
            "the restarted box did not finish: {status:?}"
        );
        self.log()
    }

    /// A second crash during recovery, then the run to its end.
    pub fn restart_and_abort_again(&self, boundary: Boundary, nth: u32) -> bool {
        self.run_until_abort(boundary, nth)
    }

    pub fn run_id(&self) -> RunId {
        let text = std::fs::read_to_string(self.dir.join(format!("run-{}", self.script)))
            .expect("the box commissioned");
        RunId::parse(text.trim()).expect("a run id")
    }

    pub fn store(&self) -> StoreView {
        StoreView {
            store: SqliteRunStore::open(
                &self.dir.join("run.sqlite"),
                std::time::Duration::from_secs(5),
            )
            .expect("the store opens"),
        }
    }

    pub fn log(&self) -> Log {
        let events = self
            .store()
            .store
            .load(&self.run_id(), 0)
            .expect("the log reads");
        Log { events }
    }

    pub fn timeline(&self) -> Timeline {
        Timeline(read_timeline(&self.dir.join("timeline.jsonl")))
    }

    pub fn messages(&self) -> Vec<PortLine> {
        read_port_lines(&self.dir.join("messages.jsonl"))
    }

    pub fn v2_dir(&self) -> PathBuf {
        self.dir.join("root-v2")
    }

    /// Runs the next commission on the same directory, to its `tree_prepared`.
    pub fn run_next_to_prepared(&self) -> Log {
        let status = self.command("next").status().expect("the box runs");
        assert!(status.success(), "the next run did not prepare: {status:?}");
        let text = std::fs::read_to_string(self.dir.join("run-next")).expect("the next run");
        let run = RunId::parse(text.trim()).expect("a run id");
        Log {
            events: self.store().store.load(&run, 0).expect("the log reads"),
        }
    }
}

pub struct Log {
    events: Vec<Event>,
}

impl Log {
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn count(&self, kind: &str) -> usize {
        self.events.iter().filter(|e| e.body.kind() == kind).count()
    }

    /// Each session key's `started` facts.
    pub fn started_per_session(&self) -> BTreeMap<String, usize> {
        let mut started = BTreeMap::new();
        for event in &self.events {
            if let EventBody::SessionFact {
                session_key,
                fact: SessionFact::Started { .. },
            } = &event.body
            {
                *started.entry(session_key.clone()).or_insert(0) += 1;
            }
        }
        started
    }

    pub fn finished_outcome(&self) -> Option<String> {
        self.events.iter().find_map(|event| match &event.body {
            EventBody::Decision(Decision::Finished { outcome, .. }) => {
                Some(outcome.as_str().to_owned())
            }
            _ => None,
        })
    }
}

pub struct Timeline(pub Vec<TimelineEntry>);

fn seat(key: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = key.split('/').collect();
    match parts.as_slice() {
        [run, step, _, member, _] => {
            Some(((*run).to_owned(), (*step).to_owned(), (*member).to_owned()))
        }
        _ => None,
    }
}

impl Timeline {
    /// At no instant were two sessions of one (run, step, member) alive, for any seat at all.
    pub fn first_overlap(&self) -> Option<String> {
        let mut alive: BTreeMap<(String, String, String), String> = BTreeMap::new();
        for entry in &self.0 {
            let seat = seat(&entry.session_key)?;
            match entry.what.as_str() {
                "spawn" => {
                    if let Some(other) = alive.insert(seat, entry.session_key.clone()) {
                        return Some(format!("{other} and {}", entry.session_key));
                    }
                }
                "end" if alive.get(&seat) == Some(&entry.session_key) => {
                    alive.remove(&seat);
                }
                _ => {}
            }
        }
        None
    }

    pub fn no_two_sessions_alive_for_one(&self, (step, member): (&str, &str)) -> bool {
        let mut alive: Vec<&str> = Vec::new();
        for entry in &self.0 {
            let Some((_, in_step, in_member)) = seat(&entry.session_key) else {
                continue;
            };
            if in_step != step || in_member != member {
                continue;
            }
            match entry.what.as_str() {
                "spawn" if !alive.is_empty() => return false,
                "spawn" => alive.push(&entry.session_key),
                "end" => alive.retain(|key| *key != entry.session_key),
                _ => {}
            }
        }
        true
    }

    pub fn spawns(&self) -> Vec<&str> {
        self.0
            .iter()
            .filter(|entry| entry.what == "spawn")
            .map(|entry| entry.session_key.as_str())
            .collect()
    }

    pub fn signals(&self, key: &str) -> Vec<&str> {
        self.0
            .iter()
            .filter(|e| e.session_key == key && (e.what == "term" || e.what == "kill"))
            .map(|e| e.what.as_str())
            .collect()
    }
}

pub struct StoreView {
    pub store: SqliteRunStore,
}

impl StoreView {
    pub fn claim_absent(&self) -> bool {
        self.store
            .get(&format!("claim/{WORKSPACE}"))
            .expect("the claim reads")
            .is_none()
    }

    /// Nothing armed, however far ahead one looks.
    pub fn timers_armed(&self) -> usize {
        let far = Timestamp::parse("9000-01-01T00:00:00.000Z").expect("an instant");
        self.store.due_timers(far).expect("the timers read").len()
    }
}

/// `git` in a directory, for what a test asserts about a tree.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}
