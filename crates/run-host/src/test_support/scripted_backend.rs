//! A backend with no process: a session is files in its ledger directory, with the ledger rules of §8.3, and
//! its life is a line in a timeline outside the host. So a session outlives an abort of the host, the restarted
//! host must find it through the ledger, and an overlap of two sessions CAN occur and be seen.
//!
//! Sessions move in [`ScriptedBackend::advance`], which the rig calls after each tick, on cue.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nexus_run_contracts::{
    BackendError, Capabilities, Capability, DrainedFact, EndCause, HeartbeatKind, Liveness,
    SessionBackend, SessionFact, StartOutcome, StartSpec, Timestamp,
};
use serde::{Deserialize, Serialize};

use crate::boundary::{crash_at, Boundary};
use crate::bridge::{reply, BridgeCtx, Caller, ReplyFlags};

/// One line of the timeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub session_key: String,
    pub what: String,
}

/// The parts of a session key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub run: String,
    pub step: String,
    pub visit: u32,
    pub member: String,
    pub attempt: u32,
}

impl SessionInfo {
    pub fn parse(key: &str) -> Option<Self> {
        let parts: Vec<&str> = key.split('/').collect();
        let [run, step, visit, member, attempt] = parts.as_slice() else {
            return None;
        };
        Some(Self {
            run: (*run).to_owned(),
            step: (*step).to_owned(),
            visit: visit.strip_prefix('v')?.parse().ok()?,
            member: (*member).to_owned(),
            attempt: attempt.strip_prefix('a')?.parse().ok()?,
        })
    }
}

/// What a session does once it has started.
#[derive(Clone, Debug, PartialEq)]
pub enum Cue {
    Reply {
        text: String,
        flags: ReplyFlags,
    },
    /// Stays alive saying nothing, and answers a nudge with a heartbeat.
    Silent,
    /// Stays alive saying nothing, answers no nudge, and a SIGTERM does not end it.
    Hung,
    /// Writes a file into its working directory, then stays silent.
    WriteFile {
        name: String,
    },
    /// Reports these facts, one per tick, then stays silent.
    Facts(Vec<SessionFact>),
    /// Ends on its own with this cause, without a reply.
    End(EndCause),
}

pub type Scenario = Box<dyn Fn(&SessionInfo) -> Cue + Send + Sync>;

pub fn pass() -> Cue {
    Cue::Reply {
        text: "done".to_owned(),
        flags: ReplyFlags::default(),
    }
}

pub struct ScriptedBackend {
    agent_logs: PathBuf,
    timeline: PathBuf,
    cwd: PathBuf,
    scenario: Mutex<Scenario>,
}

fn io_error(error: std::io::Error) -> BackendError {
    BackendError {
        what: error.to_string(),
    }
}

impl ScriptedBackend {
    pub fn new(agent_logs: &Path, timeline: &Path, cwd: &Path, scenario: Scenario) -> Self {
        Self {
            agent_logs: agent_logs.to_owned(),
            timeline: timeline.to_owned(),
            cwd: cwd.to_owned(),
            scenario: Mutex::new(scenario),
        }
    }

    pub fn set_scenario(&self, scenario: Scenario) {
        *self
            .scenario
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = scenario;
    }

    fn dir(&self, key: &str) -> PathBuf {
        self.agent_logs.join("v2").join(key)
    }

    fn note(&self, key: &str, what: &str) -> Result<(), BackendError> {
        let line = serde_json::to_string(&TimelineEntry {
            session_key: key.to_owned(),
            what: what.to_owned(),
        })
        .map_err(|error| BackendError {
            what: error.to_string(),
        })?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.timeline)
            .map_err(io_error)?;
        file.write_all(format!("{line}\n").as_bytes())
            .map_err(io_error)
    }

    pub fn timeline(&self) -> Vec<TimelineEntry> {
        read_timeline(&self.timeline)
    }

    /// A session outside the file: what `subprocess_moved` and `ended { lost }` go through in the adapter.
    pub fn push_fact(&self, key: &str, fact: &SessionFact) {
        let line = serde_json::to_string(fact).expect("a fact is plain data");
        let path = self.dir(key).join("facts.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("the facts file opens");
        file.write_all(format!("{line}\n").as_bytes())
            .expect("the fact is written");
    }

    /// Ends a session as if its process died or finished.
    pub fn end(&self, key: &str, cause: EndCause) {
        if self.dir(key).join("ended").exists() {
            return;
        }
        self.push_fact(key, &SessionFact::Ended { cause });
        fs::write(self.dir(key).join("ended"), b"").expect("the end is recorded");
        self.note(key, "end").expect("the timeline is writable");
    }

    fn is_spawned(&self, key: &str) -> bool {
        self.dir(key).join("spawned").exists()
    }

    fn is_ended(&self, key: &str) -> bool {
        self.dir(key).join("ended").exists()
    }

    fn live_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        for entry in self.timeline() {
            if entry.what == "spawn" && !keys.contains(&entry.session_key) {
                keys.push(entry.session_key);
            }
        }
        keys.retain(|key| !self.is_ended(key));
        keys
    }

    fn cue_of(&self, key: &str) -> Cue {
        let scenario = self
            .scenario
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        SessionInfo::parse(key).map_or(Cue::Silent, |info| scenario(&info))
    }

    fn phase(&self, key: &str) -> u32 {
        fs::read_to_string(self.dir(key).join("phase"))
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0)
    }

    fn set_phase(&self, key: &str, phase: u32) {
        fs::write(self.dir(key).join("phase"), phase.to_string()).expect("the phase is written");
    }

    /// Moves every live session one step, on cue.
    pub fn advance(&self, bridge: &BridgeCtx, now: Timestamp) {
        for key in self.live_keys() {
            self.advance_one(&key, bridge, now);
        }
    }

    fn advance_one(&self, key: &str, bridge: &BridgeCtx, now: Timestamp) {
        let dir = self.dir(key);
        if !dir.join("child.json").exists() {
            fs::write(dir.join("child.json"), b"{}").expect("child.json is written");
        }
        let info = SessionInfo::parse(key).expect("a session key has five parts");
        let phase = self.phase(key);
        if phase == 0 {
            let backend_session_id = format!("b-{}-{}", info.member, info.visit);
            self.push_fact(
                key,
                &SessionFact::Started {
                    backend_session_id,
                    pid: None,
                },
            );
            self.set_phase(key, 1);
            return;
        }
        let cue = self.cue_of(key);
        if dir.join("nudged").exists() && !matches!(cue, Cue::Hung) {
            fs::remove_file(dir.join("nudged")).expect("the nudge is taken");
            self.push_fact(
                key,
                &SessionFact::Heartbeat {
                    beat: HeartbeatKind::ModelOutput,
                },
            );
        }
        match (phase, cue) {
            (1, Cue::Reply { text, flags }) => {
                let thread = fs::read_to_string(dir.join("thread")).expect("the thread is known");
                let caller = Caller {
                    actor: info.member.clone(),
                    session: Some(key.to_owned()),
                };
                // A refused reply is retried next tick, as a session would.
                if reply(bridge, &thread, &caller, &text, flags, now).is_ok() {
                    self.set_phase(key, 2);
                }
            }
            (2, Cue::Reply { .. }) => self.end(key, EndCause::TurnComplete),
            (1, Cue::WriteFile { name }) => {
                let cwd = fs::read_to_string(dir.join("cwd")).expect("the cwd is known");
                fs::write(Path::new(&cwd).join(name), b"work").expect("the file is written");
                self.set_phase(key, 2);
            }
            (1, Cue::End(cause)) => self.end(key, cause),
            (n, Cue::Facts(facts)) => {
                if let Some(fact) = facts.get(n as usize - 1) {
                    self.push_fact(key, fact);
                    self.set_phase(key, n + 1);
                }
            }
            _ => {}
        }
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
}

pub fn read_timeline(path: &Path) -> Vec<TimelineEntry> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

impl SessionBackend for ScriptedBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            backend: "scripted".to_owned(),
            capabilities: vec![Capability::Resume, Capability::Nudge, Capability::Stop],
        }
    }

    /// The ledger of §8.3: a key whose `spawn.json` exists does not spawn.
    fn start(&self, spec: &StartSpec) -> Result<StartOutcome, BackendError> {
        let key = &spec.session_key;
        let dir = PathBuf::from(&spec.ledger_dir);
        fs::create_dir_all(&dir).map_err(io_error)?;
        let spawn = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join("spawn.json"));
        match spawn {
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Ok(if self.is_ended(key) {
                    StartOutcome::Lost
                } else if dir.join("child.json").exists() {
                    StartOutcome::Reattached
                } else {
                    StartOutcome::Waiting
                });
            }
            Err(error) => return Err(io_error(error)),
            Ok(_) => {}
        }
        crash_at(Boundary::AfterSpawnJsonBeforeSpawn);
        fs::write(dir.join("thread"), &spec.turn.thread).map_err(io_error)?;
        fs::write(dir.join("cwd"), &spec.cwd).map_err(io_error)?;
        fs::write(
            dir.join("spec.json"),
            serde_json::to_vec(spec).map_err(|error| BackendError {
                what: error.to_string(),
            })?,
        )
        .map_err(io_error)?;
        self.note(key, "spawn")?;
        fs::write(dir.join("spawned"), b"").map_err(io_error)?;
        crash_at(Boundary::AfterSpawnBeforeChildJson);
        fs::write(dir.join("child.json"), b"{}").map_err(io_error)?;
        Ok(StartOutcome::Spawned)
    }

    fn nudge(&self, key: &str, _text: &str) -> Result<(), BackendError> {
        self.note(key, "nudge")?;
        fs::write(self.dir(key).join("nudged"), b"").map_err(io_error)
    }

    fn stop(&self, key: &str, force: bool) -> Result<(), BackendError> {
        self.note(key, if force { "kill" } else { "term" })?;
        let hung = matches!(self.cue_of(key), Cue::Hung);
        if self.is_spawned(key) && (force || !hung) {
            self.end(key, EndCause::StoppedByHost);
        }
        Ok(())
    }

    fn alive(&self, key: &str) -> Result<Liveness, BackendError> {
        Ok(if self.is_spawned(key) && !self.is_ended(key) {
            Liveness::Alive {
                open_call: None,
                moved_at: None,
            }
        } else {
            Liveness::Dead
        })
    }

    /// Complete lines only, by byte offset.
    fn drain_facts(
        &self,
        key: &str,
        from_offset: u64,
    ) -> Result<(Vec<DrainedFact>, u64), BackendError> {
        let Ok(bytes) = fs::read(self.dir(key).join("facts.jsonl")) else {
            return Ok((Vec::new(), from_offset));
        };
        let mut facts = Vec::new();
        let mut offset = from_offset as usize;
        while let Some(line_len) = bytes
            .get(offset..)
            .and_then(|rest| rest.iter().position(|byte| *byte == b'\n'))
        {
            let line = &bytes[offset..offset + line_len];
            if let Ok(fact) = serde_json::from_slice(line) {
                facts.push(DrainedFact {
                    offset: offset as u64,
                    fact,
                });
            }
            offset += line_len + 1;
        }
        Ok((facts, offset as u64))
    }
}
