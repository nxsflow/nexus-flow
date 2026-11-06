//! The Claude adapter (spec 6j6v.zjyv §9): spawns the sidecar in facts mode into a process group of its own, reads its
//! facts from the session ledger, samples what the session runs, and signals it through the identity guard. It holds no
//! in-memory queue a crash could lose, and it reads neither the run store nor the chat model. Nothing in the product
//! calls it yet: the bridge ticket wires it in.

pub mod causes;
pub mod ledger;
pub mod procs;
pub mod sample;

mod spawn;

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nexus_run_contracts::{
    BackendError, Capabilities, Capability, DrainedFact, EndCause, HeartbeatKind, Liveness,
    SessionBackend, SessionFact, StartOutcome, StartSpec, Timestamp,
};
use serde_json::{json, Value};

use ledger::{ChildRecord, SessionLedger};
use procs::{GroupState, ProcIdentity, Signal};
use sample::SampleRecord;

/// How often a session's processes are sampled (§6.5). `subprocess_idle` and the rest count in these.
pub const SAMPLE_EVERY: Duration = Duration::from_secs(60);

/// How long a spawn may stand with no child record before nothing can be running: the adapter writes the record right
/// after the spawn and the child as its first act, so only a crash in between leaves this. `start_deadline` decides
/// before this does; this is what lets a stop of that slot end.
const NO_CHILD_GRACE_MS: i64 = 60_000;

pub struct ClaudeBackend {
    logs: PathBuf,
    bundle: PathBuf,
    sample_every: Duration,
}

/// What is left of a session whose leader is not running, or that it is.
enum EndState {
    Running,
    /// The leader is dead and these live on: not an end (§6.4 item 9).
    Lingering(Vec<u32>),
    /// Something is there that cannot be named or signalled: not an end, the named rung of §6.5.
    Unverifiable,
    Confirmed,
}

fn backend_error(what: impl std::fmt::Display) -> BackendError {
    BackendError {
        what: what.to_string(),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

fn identity_of(record: &ChildRecord) -> Option<ProcIdentity> {
    Some(ProcIdentity {
        pid: record.pid,
        start_ms: Timestamp::parse(&record.start_time)?.unix_millis(),
    })
}

fn parse_line(bytes: &[u8]) -> Option<SessionFact> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    // The sidecar reports an end raw, and this adapter says what it means (§9).
    if value.get("kind").and_then(Value::as_str) == Some("ended") {
        if let Some(raw) = value.get("raw") {
            return Some(SessionFact::Ended {
                cause: causes::map_raw(raw),
            });
        }
    }
    serde_json::from_value(value).ok()
}

impl ClaudeBackend {
    pub fn new(agent_logs: PathBuf, bundle: PathBuf) -> Self {
        Self {
            logs: agent_logs,
            bundle,
            sample_every: SAMPLE_EVERY,
        }
    }

    /// The sample's interval, for a test that drives a real child through the host with a simulated clock.
    #[must_use]
    pub fn with_sample_every(mut self, every: Duration) -> Self {
        self.sample_every = every;
        self
    }

    fn ledger(&self, key: &str) -> SessionLedger {
        SessionLedger::new(self.logs.join("v2").join(key))
    }

    /// The lines a reader could not use, for status.
    pub fn findings(&self, key: &str) -> Vec<String> {
        self.ledger(key).findings()
    }

    /// One sample now, whatever the clock says.
    pub fn sample_now(&self, key: &str) -> Result<(), BackendError> {
        let ledger = self.ledger(key);
        match ledger.child().as_ref().and_then(identity_of) {
            Some(leader) => self.sample(&ledger, &leader),
            None => Ok(()),
        }
    }

    fn sample(&self, ledger: &SessionLedger, leader: &ProcIdentity) -> Result<(), BackendError> {
        let mut record = ledger.sample();
        let now = now_ms();
        let Some(all) = procs::list() else {
            // A host that cannot sample says so once: `call_bound` applies to it alone (§6.5).
            if !record.blind_said {
                ledger
                    .append_fact(&json!({ "kind": "heartbeat", "beat": HeartbeatKind::Blind }))
                    .map_err(backend_error)?;
                record.blind_said = true;
                record.at_ms = now;
                ledger.write_sample(&record).map_err(backend_error)?;
            }
            return Ok(());
        };
        let work = sample::work_of(&all, leader.pid, leader.start_ms, &record.recorded());
        if sample::moved(&record.work, &work) {
            // Appended before the record, so a crash in between says it twice and never loses it.
            ledger
                .append_fact(
                    &json!({ "kind": "heartbeat", "beat": HeartbeatKind::SubprocessMoved }),
                )
                .map_err(backend_error)?;
            record.moved_at_ms = Some(now);
        }
        record.work = work;
        record.at_ms = now;
        ledger.write_sample(&record).map_err(backend_error)
    }

    fn sample_if_due(
        &self,
        ledger: &SessionLedger,
        leader: &ProcIdentity,
    ) -> Result<(), BackendError> {
        let every = i64::try_from(self.sample_every.as_millis()).unwrap_or(i64::MAX);
        if now_ms().saturating_sub(ledger.sample().at_ms) >= every {
            self.sample(ledger, leader)?;
        }
        Ok(())
    }

    /// Asked fresh, at decision time. Reaps first, or an unreaped leader reads as a live one.
    fn end_state(&self, ledger: &SessionLedger, leader: &ProcIdentity) -> EndState {
        procs::reap(leader);
        if procs::verify(leader) {
            return EndState::Running;
        }
        let Some(all) = procs::list() else {
            return EndState::Unverifiable;
        };
        let record = ledger.sample();
        let work = sample::work_of(&all, leader.pid, leader.start_ms, &record.recorded());
        let mut left: Vec<u32> = work.into_keys().collect();
        match procs::group_state(leader.pid, leader.start_ms) {
            GroupState::Empty | GroupState::Foreign => {}
            GroupState::Members(members) => left.extend(members.iter().map(|m| m.pid)),
            GroupState::Unverifiable => return EndState::Unverifiable,
        }
        left.sort_unstable();
        left.dedup();
        if left.is_empty() {
            EndState::Confirmed
        } else {
            EndState::Lingering(left)
        }
    }

    /// What the adapter has to say itself, before the file is read: the fact of a lingering group, once for each set of
    /// pids; `ended { lost }` once the leader is gone with no `ended` in the file.
    fn observe(
        &self,
        ledger: &SessionLedger,
        leader: &ProcIdentity,
    ) -> Result<EndState, BackendError> {
        let state = self.end_state(ledger, leader);
        match &state {
            EndState::Running => self.sample_if_due(ledger, leader)?,
            EndState::Lingering(pids) => {
                let mut record = ledger.sample();
                if &record.lingering_said != pids {
                    ledger
                        .append_fact(&json!({ "kind": "group_lingering", "pids": pids }))
                        .map_err(backend_error)?;
                    record.lingering_said.clone_from(pids);
                    ledger.write_sample(&record).map_err(backend_error)?;
                }
            }
            EndState::Confirmed => {
                if !self.has_ended_line(ledger)? {
                    let cause = EndCause::Lost;
                    ledger
                        .append_fact(&json!({ "kind": "ended", "cause": cause }))
                        .map_err(backend_error)?;
                }
            }
            EndState::Unverifiable => {}
        }
        Ok(state)
    }

    fn has_ended_line(&self, ledger: &SessionLedger) -> Result<bool, BackendError> {
        let lines = ledger.lines_from(0).map_err(backend_error)?;
        Ok(lines
            .lines
            .iter()
            .any(|(_, bytes)| matches!(parse_line(bytes), Some(SessionFact::Ended { .. }))))
    }
}

impl SessionBackend for ClaudeBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            backend: "claude".to_owned(),
            capabilities: vec![
                Capability::Resume,
                Capability::Nudge,
                Capability::Stop,
                Capability::Usage,
                Capability::PermissionRules,
                Capability::SubprocessSample,
            ],
        }
    }

    /// Idempotent by session key, and answered from the ledger (§8.3).
    fn start(&self, spec: &StartSpec) -> Result<StartOutcome, BackendError> {
        let ledger = self.ledger(&spec.session_key);
        ledger.create().map_err(backend_error)?;
        if spawn::claim(&ledger, spec, now_ms()).map_err(backend_error)? {
            spawn::spawn(&ledger, spec, &self.bundle).map_err(backend_error)?;
            return Ok(StartOutcome::Spawned);
        }
        let Some(leader) = ledger.child().as_ref().and_then(identity_of) else {
            return Ok(StartOutcome::Waiting);
        };
        procs::reap(&leader);
        if procs::verify(&leader) || self.has_ended_line(&ledger)? {
            // A session that ended while nobody watched left its own end, and the facts say it.
            return Ok(StartOutcome::Reattached);
        }
        Ok(StartOutcome::Lost)
    }

    fn nudge(&self, key: &str, text: &str) -> Result<(), BackendError> {
        let ledger = self.ledger(key);
        let child = ledger
            .child()
            .ok_or_else(|| backend_error("no child record to nudge"))?;
        // Before its handlers stand, a SIGUSR2 would end the process that was only being asked.
        if !child.handlers {
            return Err(backend_error("the child has not said it can be nudged"));
        }
        let leader = identity_of(&child)
            .filter(procs::verify)
            .ok_or_else(|| backend_error("no live leader to nudge"))?;
        spawn::write_ask(&ledger, text, now_ms()).map_err(backend_error)?;
        if procs::signal_member(&leader, Signal::Usr2) {
            Ok(())
        } else {
            Err(backend_error("the leader could not be signalled"))
        }
    }

    /// The group, and the work outside it by pid: a tool's shell is in a session of its own, so a signal to the group
    /// never reaches it (measured, tv58 step 0). Everything under the guard. A process the guard will not signal stays,
    /// and `alive` goes on saying so.
    fn stop(&self, key: &str, force: bool) -> Result<(), BackendError> {
        let ledger = self.ledger(key);
        let Some(leader) = ledger.child().as_ref().and_then(identity_of) else {
            return Ok(());
        };
        let signal = if force { Signal::Kill } else { Signal::Term };
        procs::reap(&leader);
        let mut record: SampleRecord = ledger.sample();
        if let Some(all) = procs::list() {
            // At decision time: the last sample may be a minute old. Kept, so a restart can still reach what this sees.
            record.work = sample::work_of(&all, leader.pid, leader.start_ms, &record.recorded());
            ledger.write_sample(&record).map_err(backend_error)?;
        }
        if !procs::signal_group(&leader, signal) {
            if let GroupState::Members(members) = procs::group_state(leader.pid, leader.start_ms) {
                for member in &members {
                    procs::signal_member(member, signal);
                }
            }
        }
        for member in record.recorded() {
            procs::signal_member(&member, signal);
        }
        Ok(())
    }

    fn alive(&self, key: &str) -> Result<Liveness, BackendError> {
        let ledger = self.ledger(key);
        let Some(child) = ledger.child() else {
            let starting = ledger
                .spawn_record()
                .is_some_and(|spawn| now_ms().saturating_sub(spawn.at_ms) < NO_CHILD_GRACE_MS);
            return Ok(if starting {
                Liveness::Alive {
                    open_call: None,
                    moved_at: None,
                }
            } else {
                Liveness::Dead
            });
        };
        let leader =
            identity_of(&child).ok_or_else(|| backend_error("an unreadable child record"))?;
        Ok(match self.end_state(&ledger, &leader) {
            EndState::Confirmed => Liveness::Dead,
            EndState::Running | EndState::Lingering(_) | EndState::Unverifiable => {
                Liveness::Alive {
                    // The reducer knows when a call opened, from the fact it folded; a fact carries no instant here.
                    open_call: None,
                    moved_at: ledger.sample().moved_at_ms.map(Timestamp::from_unix_millis),
                }
            }
        })
    }

    /// The complete facts from `from_offset` on. An `ended` line waits until the end is confirmed: it says how the
    /// session ended, and that is an end only with the leader reaped, the group empty and the work gone. The read position
    /// stays on it, and what stands behind it, `group_lingering` above all, is delivered meanwhile: the inbox's key is the
    /// byte offset, so delivering it again at the next tick is a `Duplicate` and nothing more.
    fn drain_facts(
        &self,
        key: &str,
        from_offset: u64,
    ) -> Result<(Vec<DrainedFact>, u64), BackendError> {
        let ledger = self.ledger(key);
        let state = match ledger.child().as_ref().and_then(identity_of) {
            Some(leader) => self.observe(&ledger, &leader)?,
            // No child record yet: nothing of its can have ended, and a stop of the slot is `alive`'s to answer.
            None => EndState::Running,
        };
        let confirmed = matches!(state, EndState::Confirmed);
        let lines = ledger.lines_from(from_offset).map_err(backend_error)?;
        if lines.past_the_end {
            ledger.add_finding(&format!(
                "offset {from_offset} lies past the end of the facts file"
            ));
        }
        let mut next = lines.next;
        let mut held = false;
        let mut facts = Vec::new();
        for (offset, bytes) in lines.lines {
            match parse_line(&bytes) {
                Some(SessionFact::Ended { .. }) if !confirmed => {
                    if !held {
                        next = offset;
                        held = true;
                    }
                }
                Some(fact) => facts.push(DrainedFact { offset, fact }),
                None => ledger.add_finding(&format!(
                    "the line at offset {offset} is no fact, and was skipped"
                )),
            }
        }
        Ok((facts, next))
    }
}
