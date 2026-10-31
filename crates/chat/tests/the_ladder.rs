//! **The ladder: ask, continue, re-commission, escalate** (nxf 6j6v.558h, cut 1d of epic 6j6v.nmc9,
//! section B).
//!
//! A slot that has a software coordinator is never left to its window: its coordinator runs ONE rung
//! per look, under a claim. A live session silent on its slot for 20 minutes is ASKED through the
//! ask door; a session that ended without answering is CONTINUED in the same session with a text
//! that names the thread and the task; a slot that never got a session, ended with nothing written,
//! or whose resume the provider refused is RE-COMMISSIONED with its original body into the same
//! slot. The three share one budget; after it — or at the wall-clock ceiling, or on a permanent
//! refusal — the coordinator ESCALATES in the member's name with what it knows.
//!
//! Driven through the COMPUTE LAYER — the functions the CLI and the service run — with a worker that
//! records every start and every ask, answers which sessions run, and a recording timer, on an
//! injected clock. The real-process shapes are in `the_ladder_in_a_real_process.rs` and in the
//! sidecar's own tests.

mod common;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::error::Result as NxfResult;
use nexus_chat::facade::{self, StatusOperation, StatusScope, StatusThread};
use nexus_chat::orchestration::{self, ConsequenceClass, SessionEnd, TickReceipt, TickRequest};
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{self, ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{Timer, TimerHandle};
use nexus_chat::worker::{
    AskOutcome, Pulse, SessionAsk, SessionStop, TriggerError, TriggerOutcome, TriggerRequest,
    TriggerResult, Worker,
};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use nxs_service::process::SubprocessSample;
use tempfile::TempDir;

// ---- the clock ------------------------------------------------------------------------------------

const T0: &str = "2026-10-24T10:00:00Z";

fn at(secs: i64) -> String {
    use time::format_description::well_known::Rfc3339;
    let t = time::OffsetDateTime::parse(T0, &Rfc3339).unwrap() + time::Duration::seconds(secs);
    t.format(&Rfc3339).unwrap()
}

/// The seconds since [`T0`] of an instant [`at`] wrote.
fn secs_of(instant: &str) -> i64 {
    use time::format_description::well_known::Rfc3339;
    let t0 = time::OffsetDateTime::parse(T0, &Rfc3339).unwrap();
    (time::OffsetDateTime::parse(instant, &Rfc3339).unwrap() - t0).whole_seconds()
}

// ---- the worker -----------------------------------------------------------------------------------

/// Records every start and every ask, answers which sessions still run, and can be told to refuse a
/// start the way a real worker does.
#[derive(Default)]
struct Hand {
    seen: Mutex<Vec<TriggerRequest>>,
    asked: Mutex<Vec<SessionAsk>>,
    running: Mutex<HashSet<String>>,
    /// Sessions whose claim does not declare the ask door.
    no_door: Mutex<HashSet<String>>,
    /// The next start answers this instead of `Accepted`.
    refuse: Mutex<Option<&'static str>>,
    /// Every start answers this instead of `Accepted`, for as long as it is set.
    refuse_every: Mutex<Option<&'static str>>,
    /// Every session somebody asked this worker to stop.
    stopped: Mutex<Vec<String>>,
    /// Whether its runtime CANNOT continue a conversation it began.
    no_resume: Mutex<bool>,
    /// When set, the next start blocks inside the worker until the test releases it — a trigger
    /// still running while another look arrives.
    gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    /// Set while a start is blocked at the gate.
    at_the_gate: std::sync::atomic::AtomicBool,
    /// What each session's pulse file says; `Unreported` for a session not named (an older
    /// sidecar), so a test that sets none keeps the transcript-only rule.
    pulse: Mutex<HashMap<String, Pulse>>,
    /// What each session's open call samples; `None` for a session not named (a blind host).
    samples: Mutex<HashMap<String, Option<SubprocessSample>>>,
    /// Every session whose asks were withdrawn, in order.
    withdrawn: Mutex<Vec<String>>,
    /// Every pulse read for a session whose process was NOT running — a reading the ladder must
    /// never take, since the file outlives its process (review of d1ey, CQ #2 / IR #3).
    pulse_read_while_not_running: Mutex<Vec<String>>,
    /// Sessions whose process dies right after the next question whether it runs — the judge's
    /// `Live` and a later read of the same look disagree.
    dies_after_one_question: Mutex<HashSet<String>>,
}

impl Hand {
    fn all(&self) -> Vec<TriggerRequest> {
        self.seen.lock().unwrap().clone()
    }
    fn of(&self, handle: &str) -> Vec<TriggerRequest> {
        self.all()
            .into_iter()
            .filter(|r| r.role.handle == handle)
            .collect()
    }
    fn last(&self, handle: &str) -> TriggerRequest {
        self.of(handle)
            .pop()
            .unwrap_or_else(|| panic!("no trigger for {handle}"))
    }
    fn asks(&self) -> Vec<SessionAsk> {
        self.asked.lock().unwrap().clone()
    }
    fn run(&self, session: &str) {
        self.running.lock().unwrap().insert(session.to_string());
    }
    fn stop(&self, session: &str) {
        self.running.lock().unwrap().remove(session);
    }
    fn stops(&self) -> Vec<String> {
        self.stopped.lock().unwrap().clone()
    }
}

impl Worker for Hand {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            self.at_the_gate
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = gate.recv();
        }
        // One internal session, one process — the sidecar's own guard.
        if self.session_is_running(&req.internal_session) {
            return Err(TriggerError::AlreadyRunning {
                session: req.internal_session.clone(),
                pid: 4242,
            });
        }
        let refusal = self
            .refuse
            .lock()
            .unwrap()
            .take()
            .or(*self.refuse_every.lock().unwrap());
        if let Some(how) = refusal {
            return Err(match how {
                // What the field log printed on 2026-10-26 (nxf 6j6v.yts5).
                "prime_timeout" => TriggerError::Failed(nexus_chat::error::NxfError::io(
                    "`nxf prime --db .nxs/db.sqlite` exceeded the 30s wall-clock timeout \
                     (injected)",
                )),
                "already_running" => TriggerError::AlreadyRunning {
                    session: req.internal_session.clone(),
                    pid: 4242,
                },
                "forbidden" => TriggerError::Failed(nexus_chat::error::NxfError::forbidden(
                    "this runtime refuses the role (injected)",
                )),
                _ => TriggerError::Failed(nexus_chat::error::NxfError::io(
                    "the runtime could not be reached (injected)",
                )),
            });
        }
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }
    fn session_is_running(&self, internal_session: &str) -> bool {
        let running = self.running.lock().unwrap().contains(internal_session);
        if running
            && self
                .dies_after_one_question
                .lock()
                .unwrap()
                .remove(internal_session)
        {
            self.running.lock().unwrap().remove(internal_session);
        }
        running
    }
    fn answers_liveness(&self) -> bool {
        true
    }
    fn stops_sessions(&self) -> bool {
        true
    }
    fn asks_sessions(&self) -> bool {
        true
    }
    fn resumes_sessions(&self) -> bool {
        !*self.no_resume.lock().unwrap()
    }
    fn stop_session(&self, internal_session: &str) -> Result<SessionStop, String> {
        self.stopped
            .lock()
            .unwrap()
            .push(internal_session.to_string());
        Ok(SessionStop::Requested)
    }
    fn ask_session(&self, ask: &SessionAsk) -> AskOutcome {
        if !self.session_is_running(&ask.session) {
            return AskOutcome::NoProcess;
        }
        if self.no_door.lock().unwrap().contains(&ask.session) {
            return AskOutcome::CannotAsk;
        }
        self.asked.lock().unwrap().push(ask.clone());
        AskOutcome::Asked
    }
    fn session_pulse(&self, internal_session: &str) -> Pulse {
        if !self.session_is_running(internal_session) {
            self.pulse_read_while_not_running
                .lock()
                .unwrap()
                .push(internal_session.to_string());
        }
        self.pulse
            .lock()
            .unwrap()
            .get(internal_session)
            .cloned()
            .unwrap_or(Pulse::Unreported)
    }
    fn withdraw_asks(&self, internal_session: &str) -> Result<(), String> {
        self.withdrawn
            .lock()
            .unwrap()
            .push(internal_session.to_string());
        Ok(())
    }
    fn subprocess_sample(&self, internal_session: &str, _since: &str) -> Option<SubprocessSample> {
        self.samples
            .lock()
            .unwrap()
            .get(internal_session)
            .copied()
            .flatten()
    }
}

// ---- the timer ------------------------------------------------------------------------------------

/// The service's book of looks, as these tests read it: the newest look filed under a slot is the one
/// it holds. That is the replace-on-arm contract of `nxs_service::timers::arm`
/// (`crates/service/src/timers.rs`, `pub fn arm`), which this fake mirrors rather than calls.
#[derive(Default)]
struct Book {
    looks: Mutex<Vec<(String, String)>>,
}

impl Book {
    /// The look the service holds for the slot of `handle` on `thread`: its book keeps ONE look per
    /// slot, and the last one armed replaces any before it (`nxs_service::timers::arm`).
    fn held_look(&self, thread: &str, handle: &str) -> Option<String> {
        let slot = format!("{thread}-{handle}");
        self.looks
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(s, _)| *s == slot)
            .map(|(_, at)| at.clone())
    }
}

impl Timer for Book {
    fn schedule(&self, thread_id: &str, _deadline: &str, _command: &str) -> NxfResult<TimerHandle> {
        Ok(TimerHandle(format!("window:{thread_id}")))
    }
    fn schedule_resume(&self, session: &str, _deadline: &str) -> NxfResult<TimerHandle> {
        Ok(TimerHandle(format!("resume:{session}")))
    }
    fn schedule_delivery(&self, session: &str, _deadline: &str) -> NxfResult<TimerHandle> {
        Ok(TimerHandle(format!("deliver:{session}")))
    }
    fn schedule_stranded_look(
        &self,
        slot: &str,
        _thread: &str,
        deadline: &str,
    ) -> NxfResult<TimerHandle> {
        self.looks
            .lock()
            .unwrap()
            .push((slot.to_string(), deadline.to_string()));
        Ok(TimerHandle(format!("look:{slot}")))
    }
    fn cancel(&self, _handle: &TimerHandle) -> NxfResult<()> {
        Ok(())
    }
}

// ---- the team -------------------------------------------------------------------------------------

/// One verifier, then a finisher, two hours each.
const QUICK: &str = "name: quick\nmembers: [verifier, finisher]\nflow: sequential\ntimeout: 2h\n";
/// A review round of two, in parallel, two hours long.
const REVIEW: &str = "name: review\nmembers: [verifier, reviewer]\nflow: parallel\ntimeout: 2h\n";
/// A build/check cycle with a back edge, two-minute windows — for `--accept`.
const CYCLE: &str = "name: cycle\nmembers: [coder, reviewer, finisher]\ntimeout: 2m\nsteps:\n  \
                     - id: build\n    target: coder\n    next: check\n  \
                     - id: check\n    target: reviewer\n    on_needs_rework: build\n    \
                     max_passes: 3\n    next: ship\n  \
                     - id: ship\n    target: finisher\n";
/// A coding chain whose second step is the review round — a nested round.
const CODING: &str = "name: coding\nmembers: [coder, review]\nflow: sequential\ntimeout: 2h\n";

struct Team {
    tmp: TempDir,
    worker: Arc<Hand>,
    book: Arc<Book>,
    origin: String,
}

fn role(handle: &str) -> RoleDecl {
    // `builder` needs the working copy to itself — the one role here whose start can queue.
    let exclusive = if handle == "builder" {
        "working_tree: exclusive\n"
    } else {
        ""
    };
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n{exclusive}"
    ))
    .expect("test role parses")
}

fn team() -> Team {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let channels: Vec<ChannelDecl> = [QUICK, REVIEW, CODING, CYCLE]
        .iter()
        .map(|c| serde_yaml::from_str(c).expect("channel parses"))
        .collect();
    let defs = Definitions::new(
        ["pm", "coder", "verifier", "reviewer", "finisher", "builder"]
            .into_iter()
            .map(role)
            .collect(),
        channels,
    )
    .expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
    let origin = nexus_chat::workspace::origin_of(&ws).to_string();
    Team {
        tmp,
        worker: Arc::new(Hand::default()),
        book: Arc::new(Book::default()),
        origin,
    }
}

/// The ladder's columns of one `owed_starts` row.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ladder {
    session: Option<String>,
    attempts: i64,
    recovery_started_at: Option<String>,
    asked_at: Option<String>,
    recovering_until: Option<String>,
}

/// One `recovery_log` row: (action, outcome).
type Logged = (String, Option<String>);

impl Team {
    fn store(&self) -> ChatStore {
        Workspace::resolve(None, self.tmp.path())
            .expect("resolve workspace")
            .open_chat_store()
            .expect("open chat store")
    }

    fn with<R>(
        &self,
        now: &str,
        actor: &str,
        session: Option<&str>,
        f: impl FnOnce(&orchestration::Ctx, &mut ChatStore) -> R,
    ) -> R {
        let ws = Workspace::resolve(None, self.tmp.path()).expect("resolve workspace");
        let db_path = ws.db_path_str().expect("db path");
        let mut store = ws.open_chat_store().expect("open chat store");
        let defs = Definitions::resolve(self.tmp.path()).expect("declarations");
        let ctx = orchestration::Ctx {
            now,
            origin: &self.origin,
            actor,
            session,
            hop: 0,
            defs: &defs,
            worker: self.worker.as_ref(),
            timer: self.book.as_ref(),
            namer: &nexus_chat::naming::DryNamer,
            db_path: &db_path,
            project_claude_md: None,
            module_primes: None,
            machines: None,
            peers: None,
        };
        f(&ctx, &mut store)
    }

    /// `role`'s session `session` commissions `to` with its own `nxc send`.
    fn session_sends(
        &self,
        now: &str,
        role: &str,
        session: &str,
        to: &str,
        body: &str,
    ) -> surface::SendToReceipt {
        self.with(now, role, Some(session), |ctx, store| {
            surface::send_to(
                ctx,
                store,
                SendToRequest {
                    machine: None,
                    to,
                    body,
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
        })
        .expect("the session's own send is accepted")
    }

    fn interrupted(&self, now: &str, session: &str, until: Option<&str>) {
        self.with(now, "service", None, |ctx, store| {
            orchestration::session_interrupted(ctx, store, session, "five_hour", until, "quota")
        })
        .expect("the hold is recorded");
    }

    fn resume(&self, now: &str, thread: &str) -> orchestration::ResumeReceipt {
        self.with(now, "owner", None, |ctx, store| {
            orchestration::resume_interrupted(
                ctx,
                store,
                orchestration::ResumeRequest {
                    scope: orchestration::ResumeScope::Thread(thread),
                    force: true,
                },
            )
        })
        .expect("the resume decides")
    }

    fn deadline_of(&self, thread: &str) -> Option<String> {
        self.status(T0)
            .into_iter()
            .flat_map(|o| o.threads)
            .find(|t| t.thread_id == thread)
            .and_then(|t| t.deadline)
    }

    /// A person, `owner`, commissions `to` at `now`.
    fn commission(&self, now: &str, to: &str, body: &str) -> surface::SendToReceipt {
        self.with(now, "owner", None, |ctx, store| {
            surface::send_to(
                ctx,
                store,
                SendToRequest {
                    machine: None,
                    to,
                    body,
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
        })
        .expect("the commission is accepted")
    }

    fn reply(&self, now: &str, role: &str, session: &str, thread: &str, body: &str) {
        self.with(now, role, Some(session), |ctx, store| {
            surface::reply_in_thread(
                ctx,
                store,
                ReplyThreadRequest {
                    machine: None,
                    thread,
                    body,
                    escalate: false,
                    needs_rework: false,
                    accept: false,
                },
            )
        })
        .expect("the reply is posted");
    }

    /// The commissioner, `owner`, answers in `thread`.
    fn owner_says(&self, now: &str, thread: &str, body: &str) {
        self.with(now, "owner", None, |ctx, store| {
            surface::reply_in_thread(
                ctx,
                store,
                ReplyThreadRequest {
                    machine: None,
                    thread,
                    body,
                    escalate: false,
                    needs_rework: false,
                    accept: false,
                },
            )
        })
        .expect("the commissioner's reply is posted");
    }

    fn tick(&self, now: &str, thread: &str) -> TickReceipt {
        self.with(now, "service", None, |ctx, store| {
            orchestration::tick(ctx, store, TickRequest { thread_id: thread })
        })
        .expect("tick")
    }

    /// The session bound to a runtime id and wrote `entries` transcript entries, the last at `last`.
    fn bound_with_transcript(&self, session: &str, entries: usize, last: &str) {
        let mut store = self.store();
        store
            .bind_session(session, &format!("real-{session}"))
            .expect("bind");
        for i in 0..entries {
            store
                .connection()
                .execute(
                    "INSERT INTO agent_transcript(internal_session, seq, kind, at, data)
                     VALUES (?1, (SELECT COALESCE(MAX(seq), -1) + 1 FROM agent_transcript
                                   WHERE internal_session = ?1), 'assistant', ?2, ?3)",
                    rusqlite::params![session, last, format!("{{\"text\":\"work {i}\"}}")],
                )
                .expect("transcript entry");
        }
    }

    /// The sidecar opened a turn of `session` for the slot on `thread` at `when` — a `turn_start`.
    fn turn(&self, session: &str, thread: &str, when: &str) {
        self.entry(
            session,
            "turn_start",
            when,
            &format!("{{\"thread\":\"{thread}\"}}"),
        );
    }

    /// `session` wrote one transcript entry at `when`.
    fn work(&self, session: &str, when: &str) {
        self.entry(session, "assistant", when, "{\"text\":\"working\"}");
    }

    /// What `session`'s pulse file says from now on.
    fn pulse(&self, session: &str, p: Pulse) {
        self.worker
            .pulse
            .lock()
            .unwrap()
            .insert(session.to_string(), p);
    }

    /// What `session`'s open call samples from now on.
    fn sample(&self, session: &str, count: u32, cpu_units: u64) {
        self.worker.samples.lock().unwrap().insert(
            session.to_string(),
            Some(SubprocessSample { count, cpu_units }),
        );
    }

    /// The slot's log with the instant of every row: (action, outcome, seconds since T0).
    fn log_at(&self, thread: &str, handle: &str) -> Vec<(String, Option<String>, i64)> {
        let store = self.store();
        let mut st = store
            .connection()
            .prepare(
                "SELECT action, outcome, at FROM recovery_log WHERE thread = ?1 AND role = ?2
                  ORDER BY id",
            )
            .unwrap();
        st.query_map([thread, &self.q(handle)], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .into_iter()
        .map(|(a, o, t)| (a, o, secs_of(&t)))
        .collect()
    }

    /// One column of the slot's owed row, as text (NULL as `None`).
    fn owed(&self, thread: &str, handle: &str, column: &str) -> Option<String> {
        self.store()
            .connection()
            .query_row(
                &format!(
                    "SELECT CAST({column} AS TEXT) FROM owed_starts WHERE thread = ?1 AND role = ?2"
                ),
                [thread, &self.q(handle)],
                |r| r.get(0),
            )
            .ok()
            .flatten()
    }

    /// The escalations in `thread` with the instant each was posted, seconds since T0.
    fn escalated_at(&self, thread: &str) -> Vec<i64> {
        self.store()
            .messages_in_thread(thread)
            .expect("messages")
            .into_iter()
            .filter(|m| m.kind == "escalation")
            .filter_map(|m| m.created.as_deref().map(secs_of))
            .collect()
    }

    /// `session`'s main conversation called the tool `name` at `when` — a `tool_use` with its id.
    fn tool_call(&self, session: &str, id: &str, name: &str, when: &str) {
        self.entry_with_id(
            session,
            "tool_use",
            id,
            when,
            &format!("{{\"name\":\"{name}\",\"input\":{{}}}}"),
        );
    }

    /// The call `id` of `session` returned at `when` — its `tool_result`.
    fn tool_returned(&self, session: &str, id: &str, when: &str) {
        self.entry_with_id(
            session,
            "tool_result",
            id,
            when,
            "{\"content\":\"ok\",\"isError\":false}",
        );
    }

    fn entry(&self, session: &str, kind: &str, when: &str, data: &str) {
        self.store()
            .connection()
            .execute(
                "INSERT INTO agent_transcript(internal_session, seq, kind, at, data)
                 VALUES (?1, (SELECT COALESCE(MAX(seq), -1) + 1 FROM agent_transcript
                               WHERE internal_session = ?1), ?2, ?3, ?4)",
                rusqlite::params![session, kind, when, data],
            )
            .expect("transcript entry");
    }

    fn entry_with_id(&self, session: &str, kind: &str, id: &str, when: &str, data: &str) {
        self.store()
            .connection()
            .execute(
                "INSERT INTO agent_transcript(internal_session, seq, kind, tool_use_id, at, data)
                 VALUES (?1, (SELECT COALESCE(MAX(seq), -1) + 1 FROM agent_transcript
                               WHERE internal_session = ?1), ?2, ?3, ?4, ?5)",
                rusqlite::params![session, kind, id, when, data],
            )
            .expect("transcript entry");
    }

    /// `coder`'s session `session` answers a nudge on `thread` with a rest — `nxc confirm --rest`.
    fn confirm(
        &self,
        now: &str,
        session: &str,
        thread: &str,
        minutes: i64,
    ) -> NxfResult<surface::ConfirmReceipt> {
        self.with(now, "coder", Some(session), |ctx, store| {
            surface::confirm_rest(ctx, store, thread, minutes)
        })
    }

    /// The session's process ends, announcing `cause`, as the sidecar's teardown does.
    fn ended(&self, now: &str, session: &str, cause: Option<&str>) {
        self.with(now, "service", None, |ctx, store| {
            orchestration::session_ended_with(
                ctx,
                store,
                session,
                SessionEnd {
                    cause,
                    attempts: Some(1),
                },
            )
        })
        .expect("session ended");
    }

    fn q(&self, handle: &str) -> String {
        format!("{}/{handle}", self.origin)
    }

    fn ladder(&self, thread: &str, handle: &str) -> Option<Ladder> {
        self.store()
            .connection()
            .query_row(
                "SELECT session, attempts, recovery_started_at, asked_at, recovering_until
                   FROM owed_starts WHERE thread = ?1 AND role = ?2",
                [thread, &self.q(handle)],
                |r| {
                    Ok(Ladder {
                        session: r.get(0)?,
                        attempts: r.get(1)?,
                        recovery_started_at: r.get(2)?,
                        asked_at: r.get(3)?,
                        recovering_until: r.get(4)?,
                    })
                },
            )
            .ok()
    }

    fn log(&self, thread: &str, handle: &str) -> Vec<Logged> {
        let store = self.store();
        let mut st = store
            .connection()
            .prepare(
                "SELECT action, outcome FROM recovery_log WHERE thread = ?1 AND role = ?2
                  ORDER BY id",
            )
            .unwrap();
        st.query_map([thread, &self.q(handle)], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    fn sql(&self, statement: &str) {
        self.store()
            .connection()
            .execute_batch(statement)
            .expect("the statement runs");
    }

    fn status(&self, now: &str) -> Vec<StatusOperation> {
        facade::status(
            &self.store(),
            self.worker.as_ref(),
            now,
            StatusScope::All(None),
        )
        .expect("status")
        .operations
    }

    fn status_of(&self, now: &str, thread: &str) -> StatusThread {
        self.status(now)
            .into_iter()
            .flat_map(|o| o.threads)
            .find(|t| t.thread_id == thread)
            .unwrap_or_else(|| panic!("no status row for {thread}"))
    }

    /// The bodies of the thread's messages, oldest first, with their kinds.
    fn messages(&self, thread: &str) -> Vec<(String, String, String)> {
        self.store()
            .messages_in_thread(thread)
            .expect("messages")
            .into_iter()
            .map(|m| (m.sender, m.kind, m.body))
            .collect()
    }
}

fn escalations(messages: &[(String, String, String)]) -> Vec<String> {
    messages
        .iter()
        .filter(|(_, kind, _)| kind == "escalation")
        .map(|(_, _, body)| body.clone())
        .collect()
}

// ---- rung C: continue the SAME session (field case 1) --------------------------------------------

/// **Field case 1, at the compute layer.** A coder commissioned by a person binds, writes, and ends
/// without answering — told to stop. Nothing is recorded stranded and nothing is escalated: at the
/// end + grace its coordinator continues the SAME session, resuming the runtime session it bound,
/// with the continue text naming the thread and quoting the commissioning message's first line —
/// and the text offers no `nxc confirm`. One attempt is counted and logged.
#[test]
fn an_ended_session_is_continued_in_the_same_session_with_the_thread_and_task_named() {
    let team = team();
    let thread = team
        .commission(T0, "coder", "Add a --since flag.\n\nDetails follow.")
        .thread_id;
    let first = team.worker.last("coder");
    let session = first.internal_session.clone();
    team.bound_with_transcript(&session, 3, &at(30));
    team.ended(&at(60), &session, Some("told to stop (SIGTERM)"));

    let before = team.tick(&at(60 + 119), &thread);
    assert_eq!(
        team.worker.of("coder").len(),
        1,
        "inside the grace: nothing"
    );
    assert!(before
        .warnings
        .iter()
        .all(|f| f.class != ConsequenceClass::SessionStranded));

    team.tick(&at(60 + 120), &thread);
    let starts = team.worker.of("coder");
    assert_eq!(starts.len(), 2, "exactly one continuation");
    let cont = &starts[1];
    assert_eq!(cont.internal_session, session, "the SAME session");
    assert_eq!(
        cont.resume_real.as_deref(),
        Some(format!("real-{session}").as_str())
    );
    assert_eq!(cont.reply_thread.as_deref(), Some(thread.as_str()));
    assert!(cont.coordinated);
    assert!(
        cont.message.contains(&format!(
            "Your turn for thread {thread} ended without a reply"
        )) && cont.message.contains("the task was: Add a --since flag.")
            && !cont.message.contains("Details follow."),
        "{}",
        cont.message
    );
    assert!(!cont.message.contains("nxc confirm"), "{}", cont.message);
    let ladder = team.ladder(&thread, "coder").expect("the row stands");
    assert_eq!(ladder.attempts, 1);
    assert_eq!(
        ladder.recovery_started_at.as_deref(),
        Some(at(180).as_str())
    );
    assert_eq!(ladder.recovering_until, None, "the claim is released");
    assert_eq!(
        team.log(&thread, "coder"),
        vec![("continued".to_string(), Some("triggered".to_string()))]
    );
    assert!(escalations(&team.messages(&thread)).is_empty());
    assert!(team.status_of(&at(180), &thread).stranded.is_none());

    // The continued session answers: accepted once, the row goes.
    team.reply(&at(300), "coder", &session, &thread, "done: --since is in");
    team.tick(&at(301), &thread);
    assert_eq!(team.ladder(&thread, "coder"), None);
    assert_eq!(team.worker.of("coder").len(), 2);
}

/// The silent-three-times variant: every continuation ends without an answer. Three attempts, then
/// rung E escalates ONCE in the member's name, listing the three attempts and the cause verbatim.
#[test]
fn a_session_that_ends_silent_three_times_is_escalated_once_with_every_attempt_and_the_cause() {
    let team = team();
    let thread = team
        .commission(T0, "coder", "Add a --since flag.")
        .thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 2, &at(20));
    let mut now = 60;
    for round in 1..=3 {
        team.ended(&at(now), &session, Some("told to stop (SIGTERM)"));
        now += 120;
        team.tick(&at(now), &thread);
        assert_eq!(team.worker.of("coder").len(), 1 + round, "round {round}");
        now += 60;
    }
    team.ended(&at(now), &session, Some("killed (exit 137)"));
    now += 120;
    let r = team.tick(&at(now), &thread);
    assert_eq!(team.worker.of("coder").len(), 4, "no fourth continuation");
    let posted = escalations(&team.messages(&thread));
    assert_eq!(posted.len(), 1, "{posted:?}");
    let body = &posted[0];
    for needle in [
        "attempt 1/3",
        "attempt 2/3",
        "attempt 3/3",
        "continued",
        "killed (exit 137)",
        &session,
        "nxc withdraw",
    ] {
        assert!(body.contains(needle), "{needle:?} missing from:\n{body}");
    }
    assert!(r
        .warnings
        .iter()
        .any(|f| f.class == ConsequenceClass::SessionStranded));
    let log = team.log(&thread, "coder");
    assert_eq!(log.len(), 4, "{log:?}");
    assert_eq!(log[3].0, "escalated");
    // And nothing after: the slot is discharged by the escalation.
    team.tick(&at(now + 600), &thread);
    assert_eq!(escalations(&team.messages(&thread)).len(), 1);
    assert_eq!(team.worker.of("coder").len(), 4);
}

// ---- rung D: re-commission (field case 3) ---------------------------------------------------------

/// **Field case 3**: a review member whose mint failed has a row with no session. At +180 s it is
/// `never_started`, and its coordinator RE-COMMISSIONS it: a fresh session, the ORIGINAL body, into
/// the SAME slot thread — never "continuing with your own transcript". The round then consolidates
/// once, on its answer.
#[test]
fn a_slot_that_never_got_a_session_is_recommissioned_with_its_original_body_into_the_same_slot() {
    let team = team();
    team.sql(
        "CREATE TRIGGER no_verifier_mint BEFORE INSERT ON session_map WHEN NEW.role = 'verifier'
         BEGIN SELECT RAISE(ABORT, 'the session could not be minted (injected)'); END;",
    );
    let refused = team.with(T0, "owner", None, |ctx, store| {
        surface::send_to(
            ctx,
            store,
            SendToRequest {
                machine: None,
                to: "review",
                body: "review the parser change",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
    });
    assert!(refused.is_err());
    team.sql("DROP TRIGGER no_verifier_mint;");
    let root = team
        .status(T0)
        .into_iter()
        .find(|o| {
            o.channel_id
                .as_deref()
                .is_some_and(|c| c.contains("review"))
        })
        .expect("the round")
        .root;
    let slot = team
        .status(T0)
        .into_iter()
        .flat_map(|o| o.threads)
        .find(|t| t.expects == vec![team.q("verifier")])
        .expect("the verifier's slot")
        .thread_id;
    let commissioning = team.messages(&slot)[0].2.clone();

    team.tick(&at(120), &root);
    team.tick(&at(180), &root);
    let starts = team.worker.of("verifier");
    assert_eq!(starts.len(), 1, "re-commissioned once");
    let fresh = &starts[0];
    assert_eq!(fresh.resume_real, None, "a fresh session, never a resume");
    assert_eq!(fresh.reply_thread.as_deref(), Some(slot.as_str()));
    assert!(fresh.coordinated);
    assert!(
        fresh.message.contains(&commissioning),
        "the original body:\n{}",
        fresh.message
    );
    assert!(!fresh.message.contains("ended without a reply"));
    let ladder = team.ladder(&slot, "verifier").unwrap();
    assert_eq!(
        ladder.session.as_deref(),
        Some(fresh.internal_session.as_str())
    );
    assert_eq!(ladder.attempts, 1);
    assert_eq!(
        team.log(&slot, "verifier"),
        vec![("recommissioned".to_string(), Some("triggered".to_string()))]
    );
    assert!(
        team.status_of(&at(180), &slot).stranded.is_none(),
        "no record, no escalation"
    );
    // **The member's window is armed onto the new session** (must-fix, design review round 2 CQ
    // #3) — here the channel's declared two hours, since the failed mint never armed one.
    let (armed_for, window, deadline): (String, i64, String) = team
        .store()
        .connection()
        .query_row(
            "SELECT session, window_secs, deadline FROM channel_member_deadline WHERE thread_id = ?1",
            [&slot],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("a member clock");
    assert_eq!(
        (armed_for.as_str(), window, deadline.as_str()),
        (
            fresh.internal_session.as_str(),
            7_200,
            at(180 + 7_200).as_str()
        )
    );

    // The member answers; the round consolidates once. (The fan-out stopped at the failed mint, so
    // the verifier's is the round's only slot.)
    team.reply(
        &at(210),
        "verifier",
        &fresh.internal_session,
        &slot,
        "verified",
    );
    let root_messages = team.messages(&root);
    let delivered = root_messages
        .iter()
        .filter(|(sender, _, _)| sender.contains("/__"))
        .count();
    assert_eq!(delivered, 1, "{root_messages:?}");
}

/// An ended session with ZERO transcript entries — bound or not — is re-commissioned, never
/// continued: there is no turn of its own to continue (must-fix: the rung after an announced end is
/// chosen by (bound, entries) inside the announced-end branch).
#[test]
fn an_ended_session_that_wrote_nothing_is_recommissioned_not_continued() {
    let team = team();
    let thread = team
        .commission(T0, "coder", "Add a --since flag.")
        .thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 0, &at(5));
    team.ended(
        &at(10),
        &session,
        Some("the runtime never ran this session"),
    );
    team.tick(&at(130), &thread);
    let starts = team.worker.of("coder");
    assert_eq!(starts.len(), 2);
    assert_ne!(starts[1].internal_session, session, "a NEW session");
    assert_eq!(starts[1].resume_real, None);
    assert!(starts[1].message.contains("Add a --since flag."));
    assert_eq!(team.log(&thread, "coder")[0].0, "recommissioned");
}

// ---- whom a recovered slot's reply belongs to (nxf 6j6v.f5rf) ----------------------------------

/// `role`'s session `session` answers in `thread` — the reply's own result, refusal included.
fn a_reply_from(
    team: &Team,
    now: &str,
    role: &str,
    session: &str,
    thread: &str,
    body: &str,
) -> NxfResult<orchestration::ReplyReceipt> {
    team.with(now, role, Some(session), |ctx, store| {
        surface::reply_in_thread(
            ctx,
            store,
            ReplyThreadRequest {
                machine: None,
                thread,
                body,
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
    })
}

/// **A re-commissioned session serves the slot it was re-commissioned into** (nxf 6j6v.f5rf, review
/// TQ #1). The fresh session never posted there and is not where the slot was first opened from, so
/// only the record its trigger writes makes it the slot's — without it the answer is refused and the
/// slot stays owed for good. A session of ANOTHER commission of the same persona is refused there.
#[test]
fn a_recommissioned_session_answers_its_slot_and_another_commissions_session_may_not() {
    let team = team();
    let thread = a_slot_due_for_a_recommission(&team);
    let old = team.worker.last("coder").internal_session;
    let other = team.commission(T0, "coder", "Something else.").thread_id;
    let stranger = team.worker.last("coder").internal_session;
    assert_ne!(stranger, old);

    team.tick(&at(130), &thread);
    assert_eq!(team.log(&thread, "coder")[0].0, "recommissioned");
    let fresh = team.worker.last("coder");
    assert_ne!(fresh.internal_session, old, "a NEW session");
    assert_eq!(fresh.reply_thread.as_deref(), Some(thread.as_str()));

    let before = team.messages(&thread).len();
    let refused = a_reply_from(&team, &at(200), "coder", &stranger, &thread, "mine, really")
        .expect_err("another commission's session does not serve this slot");
    assert_eq!(refused.kind, nxs_foundation::error::ErrorKind::Forbidden);
    assert!(
        refused.msg.contains(&thread) && refused.msg.contains(&other),
        "it names the slot asked for and the one the session serves: {}",
        refused.msg
    );
    assert_eq!(team.messages(&thread).len(), before, "nothing written");

    a_reply_from(
        &team,
        &at(210),
        "coder",
        &fresh.internal_session,
        &thread,
        "done: --since is in",
    )
    .expect("the re-commissioned session answers its slot");
    team.tick(&at(211), &thread);
    assert_eq!(team.ladder(&thread, "coder"), None, "the slot is answered");
    // The replaced session still serves the slot it was started for — its own thread (DoD 2).
    assert!(team.store().session_serves(&old, &thread).unwrap());
}

/// **`nxc confirm` is bound like a reply** (nxf 6j6v.f5rf, review TQ #2; nxf 6j6v.1r61): a
/// confirmation from another commission's session is refused, and the slot it named keeps its nudge,
/// its rests and its messages — a stranger's "still working" must not keep somebody else's slot
/// looking alive. The session's own confirmation is granted as before.
#[test]
fn a_confirm_from_another_commissions_session_is_refused_and_moves_nothing() {
    let team = team();
    let thread = team
        .commission(T0, "coder", "Add a --since flag.")
        .thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.tick(&at(600), &thread);
    let asked = team.ladder(&thread, "coder").unwrap();
    assert!(asked.asked_at.is_some(), "a nudge is pending: {asked:?}");
    team.commission(T0, "coder", "Something else.");
    let stranger = team.worker.last("coder").internal_session;
    let before = team.messages(&thread);

    let refused = team
        .confirm(&at(630), &stranger, &thread, 20)
        .expect_err("another commission's session does not serve this slot");
    assert_eq!(refused.kind, nxs_foundation::error::ErrorKind::Forbidden);
    assert_eq!(team.messages(&thread), before, "nothing written");
    assert_eq!(
        team.ladder(&thread, "coder").unwrap(),
        asked,
        "the nudge, the episode and the budget are untouched"
    );
    assert_eq!(
        team.owed(&thread, "coder", "confirmations"),
        Some("0".into())
    );

    let r = team
        .confirm(&at(640), &session, &thread, 20)
        .expect("the slot's own session is granted its rest");
    assert_eq!(r.granted_minutes, 20);
}

// ---- a re-commission whose start fails (nxf 6j6v.yts5) ------------------------------------------

/// The slot of a coder whose session ended having written nothing — the ladder's next rung for it
/// is a re-commission, at the end + grace (+130 s).
fn a_slot_due_for_a_recommission(team: &Team) -> String {
    let thread = team
        .commission(T0, "coder", "Add a --since flag.")
        .thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 0, &at(5));
    team.ended(
        &at(10),
        &session,
        Some("the runtime never ran this session"),
    );
    thread
}

/// **Field case 2026-10-26/27, at the compute layer.** A re-commission whose start fails on an `nxf
/// prime` timeout is looked at again one cadence later, and that look re-commissions it — the slot
/// is answered. Before, the +60 s look replaced the +120 s one the trigger had armed, the judge
/// found the new clock's grace not yet reached and returned with nothing armed, and nothing looked
/// again for 5.5 h.
#[test]
fn a_recommission_whose_start_fails_on_a_prime_timeout_is_retried_at_the_next_look_and_answered() {
    let team = team();
    let thread = a_slot_due_for_a_recommission(&team);
    *team.worker.refuse.lock().unwrap() = Some("prime_timeout");

    team.tick(&at(130), &thread);

    assert_eq!(
        team.worker.of("coder").len(),
        1,
        "the re-commission did not start"
    );
    let log = team.log(&thread, "coder");
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0].0, "recommissioned");
    assert!(
        log[0]
            .1
            .as_deref()
            .is_some_and(|o| o.starts_with("failed:")),
        "{log:?}"
    );
    let next = team
        .book
        .held_look(&thread, "coder")
        .expect("the service holds a look for the slot");
    assert_eq!(next, at(190), "one cadence after the failure");

    team.tick(&next, &thread);

    let starts = team.worker.of("coder");
    assert_eq!(starts.len(), 2, "re-commissioned at the look");
    let fresh = &starts[1];
    assert_eq!(fresh.resume_real, None, "a fresh session");
    assert!(fresh.message.contains("Add a --since flag."));
    assert_eq!(
        team.ladder(&thread, "coder").unwrap().attempts,
        1,
        "the retry is the same re-commission: one attempt (round 2 TQ #6)"
    );
    assert_eq!(
        team.log(&thread, "coder"),
        vec![("recommissioned".to_string(), Some("triggered".to_string()))],
        "and one row, settled again"
    );
    team.reply(
        &at(200),
        "coder",
        &fresh.internal_session,
        &thread,
        "--since is in.",
    );
    let owed = team
        .store()
        .thread_quorum(&thread, &at(200))
        .unwrap()
        .is_some_and(|q| q.outstanding.contains(&team.q("coder")));
    assert!(!owed, "answered");
    assert!(escalations(&team.messages(&thread)).is_empty());
}

/// **A failed continue is continued again at the next look** (nxf 6j6v.yts5): the session bound and
/// wrote, so the rung after an end is a continue, and the retry asks for that rung again — the SAME
/// session resumed, never a fresh one, and on the same row and attempt (round 2 TQ #6).
#[test]
fn a_continue_whose_start_fails_is_continued_again_at_the_next_look_and_answered() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    *team.worker.refuse.lock().unwrap() = Some("prime_timeout");

    team.tick(&at(130), &thread);

    assert_eq!(
        team.worker.of("coder").len(),
        1,
        "the continue did not start"
    );
    assert_eq!(team.book.held_look(&thread, "coder"), Some(at(190)));

    team.tick(&at(190), &thread);

    let log = team.log(&thread, "coder");
    assert_eq!(
        log,
        vec![("continued".to_string(), Some("triggered".to_string()))],
        "continued again on the same row — one continue with its retry (round 2 TQ #6)"
    );
    assert_eq!(team.ladder(&thread, "coder").unwrap().attempts, 1);
    let again = team.worker.last("coder");
    assert_eq!(again.internal_session, session, "the same session");
    assert_eq!(
        again.resume_real.as_deref(),
        Some(format!("real-{session}").as_str())
    );
    team.reply(&at(200), "coder", &session, &thread, "built");
    let owed = team
        .store()
        .thread_quorum(&thread, &at(200))
        .unwrap()
        .is_some_and(|q| q.outstanding.contains(&team.q("coder")));
    assert!(!owed, "answered");
}

/// **A failed rung older than the slot's clock is not retried at once.** A later trigger moved the
/// clock after the failure, so the failure says nothing about the start now owed: the look waits
/// out the grace on the new clock, as for any start.
#[test]
fn a_failed_rung_older_than_the_slots_clock_waits_out_the_new_clocks_grace() {
    let team = team();
    let thread = a_slot_due_for_a_recommission(&team);
    *team.worker.refuse.lock().unwrap() = Some("prime_timeout");
    team.tick(&at(130), &thread);
    // A later trigger owed the slot again at +200 — the clock moves past the failed rung.
    team.sql(&format!(
        "UPDATE owed_starts SET trigger_clock = '{}' WHERE thread = '{thread}'",
        at(200)
    ));

    team.tick(&at(210), &thread);

    assert_eq!(team.worker.of("coder").len(), 1, "nothing started at once");
    assert_eq!(team.log(&thread, "coder").len(), 1, "no second rung");
    assert_eq!(
        team.book.held_look(&thread, "coder"),
        Some(at(320)),
        "the grace on the new clock"
    );
}

/// A re-commission that never starts is escalated by the looks the service actually holds — each
/// one replacing the last, as its book does — within the budget, long before the slot's deadline;
/// and never without a look armed while the slot is owed.
#[test]
fn a_recommission_that_never_starts_is_escalated_by_the_looks_the_service_holds() {
    let team = team();
    let thread = a_slot_due_for_a_recommission(&team);
    *team.worker.refuse_every.lock().unwrap() = Some("prime_timeout");

    let mut now = 130;
    team.tick(&at(now), &thread);
    let mut looks = 0;
    while escalations(&team.messages(&thread)).is_empty() {
        let next = team
            .book
            .held_look(&thread, "coder")
            .unwrap_or_else(|| panic!("no look held for the owed slot at +{now}s"));
        let next = secs_of(&next);
        assert!(next > now, "the held look at +{next}s is not after +{now}s");
        assert!(
            next <= 130 + nexus_chat::ladder::RECOVERY_CEILING_FLOOR_SECS,
            "the held look at +{next}s is past the ceiling"
        );
        now = next;
        team.tick(&at(now), &thread);
        looks += 1;
        assert!(looks < 50, "escalated within 50 looks");
    }

    // Nine starts a cadence apart from +130 (three attempts of three starts each, round 2 TQ #6),
    // then rung E at the next look: +670.
    assert!(
        now <= 700,
        "escalated at +{now}s, within the budget's few minutes"
    );
    assert_eq!(team.worker.of("coder").len(), 1, "nothing ever started");
    let log = team.log(&thread, "coder");
    let failed = log
        .iter()
        .filter(|(a, o)| {
            a == "recommissioned" && o.as_deref().is_some_and(|o| o.starts_with("failed:"))
        })
        .count();
    assert_eq!(failed, 3, "{log:?}");
    // Three re-commissions, each with its start retried twice on its own row (round 2 TQ #6):
    // nine starts, three attempts, then rung E — still minutes, not the deadline.
    let retries: Vec<i64> = {
        let store = team.store();
        let mut st = store
            .connection()
            .prepare("SELECT retries FROM recovery_log WHERE action = 'recommissioned' ORDER BY id")
            .unwrap();
        st.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(retries, vec![2, 2, 2]);
    assert_eq!(log.last().map(|(a, _)| a.as_str()), Some("escalated"));
    let posted = escalations(&team.messages(&thread));
    assert_eq!(posted.len(), 1, "{posted:?}");
    assert!(posted[0].contains("wall-clock timeout"), "{}", posted[0]);
}

/// **A look inside the grace arms the grace's end** (nxf 6j6v.yts5, the judge's half). A rung that
/// did nothing — its start refused `already_running` — arms the next look one cadence out, which
/// replaces the one its trigger armed at the grace. That look finds the new clock's grace not yet
/// reached; it must arm the grace's end, or nothing looks at the slot again.
#[test]
fn a_look_inside_the_grace_arms_the_graces_end_and_the_slot_is_taken_up() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    *team.worker.refuse.lock().unwrap() = Some("already_running");
    team.tick(&at(130), &thread);
    assert_eq!(team.book.held_look(&thread, "coder"), Some(at(190)));

    team.tick(&at(190), &thread);

    assert_eq!(
        team.book.held_look(&thread, "coder"),
        Some(at(250)),
        "the end of the grace on the clock the refused trigger moved"
    );
    let mut now = 250;
    while team.worker.of("coder").len() < 2 {
        team.tick(&at(now), &thread);
        let next = team
            .book
            .held_look(&thread, "coder")
            .map(|l| secs_of(&l))
            .unwrap_or_else(|| panic!("no look held at +{now}s"));
        if team.worker.of("coder").len() < 2 {
            assert!(next > now, "the held look at +{next}s is not after +{now}s");
        }
        now = next;
        assert!(now < 1_000, "taken up within minutes");
    }
}

/// The same for the second look: a look that arrives between the two — the service's one look for
/// the slot replaced by an earlier one — arms the second look's instant again.
#[test]
fn a_look_between_the_two_looks_arms_the_second() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    team.tick(&at(120), &thread);
    assert_eq!(
        team.book.held_look(&thread, "coder"),
        Some(at(180)),
        "the first look on unchanged evidence arms the second"
    );
    // Something armed an earlier look for the slot, replacing the second one in the service's book.
    team.book
        .looks
        .lock()
        .unwrap()
        .push((format!("{thread}-coder"), at(150)));

    team.tick(&at(150), &thread);

    assert_eq!(team.book.held_look(&thread, "coder"), Some(at(180)));
    assert!(escalations(&team.messages(&thread)).is_empty());
}

/// **The ceiling fires at the ceiling, whatever the look judges** — here a slot whose continued
/// session went on hold until an instant five hours out: rung B leaves a hold that names its
/// instant to the resume, and before this nothing looked at the slot until then.
#[test]
fn the_ceiling_fires_at_the_ceiling_while_the_slot_is_held() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    team.tick(&at(130), &thread);
    assert_eq!(team.log(&thread, "coder")[0].0, "continued");
    let episode = team
        .ladder(&thread, "coder")
        .unwrap()
        .recovery_started_at
        .expect("the episode began");
    assert_eq!(episode, at(130));
    team.interrupted(&at(200), &session, Some(&at(5 * 3_600)));

    team.tick(&at(250), &thread);

    let ceiling = 130 + nexus_chat::ladder::RECOVERY_CEILING_FLOOR_SECS;
    assert_eq!(
        team.book.held_look(&thread, "coder"),
        Some(at(ceiling)),
        "the look the service holds is the ceiling's"
    );
    assert!(escalations(&team.messages(&thread)).is_empty());

    team.tick(&at(ceiling), &thread);

    let posted = escalations(&team.messages(&thread));
    assert_eq!(posted.len(), 1, "rung E at the ceiling: {posted:?}");
    assert!(posted[0].contains("ceiling"), "{}", posted[0]);
    assert!(
        posted[0].contains(&format!("while on hold until {}", at(5 * 3_600)))
            && posted[0].contains("a resume may still answer"),
        "the escalation says the hold still stands: {}",
        posted[0]
    );
}

/// **A hold naming no instant whose grace would land past the ceiling** — rung B's other branch: the
/// hold's grace is armed, clamped to the ceiling, and the ceiling fires there before the grace ends.
#[test]
fn a_hold_naming_no_instant_whose_grace_lands_past_the_ceiling_escalates_at_the_ceiling() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    team.tick(&at(130), &thread);
    assert_eq!(team.log(&thread, "coder")[0].0, "continued");
    let ceiling = 130 + nexus_chat::ladder::RECOVERY_CEILING_FLOOR_SECS;
    team.interrupted(&at(ceiling - 60), &session, None);

    team.tick(&at(ceiling - 30), &thread);

    assert_eq!(
        team.book.held_look(&thread, "coder"),
        Some(at(ceiling)),
        "the hold's grace ends past the ceiling, so the look is the ceiling's"
    );
    assert!(escalations(&team.messages(&thread)).is_empty());

    team.tick(&at(ceiling), &thread);

    let posted = escalations(&team.messages(&thread));
    assert_eq!(posted.len(), 1, "rung E at the ceiling: {posted:?}");
    assert!(posted[0].contains("ceiling"), "{}", posted[0]);
    assert!(
        !posted[0].contains("while on hold until"),
        "a hold naming no instant names none: {}",
        posted[0]
    );
}

/// **The ceiling fires while the answer is still coming by another route** (nxf 6j6v.yts5, review
/// round 1): a coordinated slot in an open episode whose start failed and whose retry is due only
/// past the ceiling. The sweep leaves a still-coming slot to that other clock — and that clock would
/// fire an hour too late. The ceiling's look is armed, and the slot escalates at the ceiling.
#[test]
fn a_coordinated_slot_whose_retry_is_due_past_the_ceiling_escalates_at_the_ceiling() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    team.tick(&at(130), &thread);
    assert_eq!(team.log(&thread, "coder")[0].0, "continued");
    let ceiling = 130 + nexus_chat::ladder::RECOVERY_CEILING_FLOOR_SECS;
    team.sql(&format!(
        "INSERT INTO failed_starts(session, scope_key, role, thread, message, model, depth,
             priority, enqueued_at, first_failed_at, last_failed_at, attempts, error, retry_at,
             in_flight_until)
         VALUES ('{session}', 'scope', '{role}', '{thread}', 'build', NULL, 1, 0, '{t}', '{t}',
             '{t}', 1, 'prime_timeout', '{retry}', NULL)",
        role = team.q("coder"),
        t = at(200),
        retry = at(ceiling + 3_600),
    ));

    team.tick(&at(250), &thread);

    assert_eq!(
        team.book.held_look(&thread, "coder"),
        Some(at(ceiling)),
        "the slot's look is the ceiling's, not the retry's an hour past it"
    );
    assert!(escalations(&team.messages(&thread)).is_empty());

    team.tick(&at(ceiling), &thread);

    let posted = escalations(&team.messages(&thread));
    assert_eq!(posted.len(), 1, "rung E at the ceiling: {posted:?}");
    assert!(posted[0].contains("ceiling"), "{}", posted[0]);
}

/// **No look reaches past the ceiling.** A live session's next look is one cadence out; thirty
/// seconds before the ceiling that would land thirty seconds past it, and the ceiling would fire
/// late. The look is armed AT the ceiling instead, and fires there. The session here is a resumed
/// one in its first minutes (its process started just before; nothing of its own yet), so its
/// episode stands — a session with life of its own since its start would end the episode instead
/// (`a_recovered_episode_is_closed_and_a_later_silence_opens_a_second`).
#[test]
fn a_look_that_would_land_past_the_ceiling_is_armed_at_it() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    team.tick(&at(130), &thread);
    assert_eq!(team.log(&thread, "coder")[0].0, "continued");
    team.worker.run(&session);
    let ceiling = 130 + nexus_chat::ladder::RECOVERY_CEILING_FLOOR_SECS;
    // A later resume started its process 45 s before the look: P moves, the episode stands.
    team.sql(&format!(
        "UPDATE owed_starts SET trigger_clock = '{}' WHERE thread = '{thread}'",
        at(ceiling - 75)
    ));
    team.turn(&session, &thread, &at(ceiling - 75));

    team.tick(&at(ceiling - 30), &thread);

    assert_eq!(
        team.book.held_look(&thread, "coder"),
        Some(at(ceiling)),
        "one cadence out would be past the ceiling"
    );
    assert!(escalations(&team.messages(&thread)).is_empty());

    team.tick(&at(ceiling), &thread);

    let posted = escalations(&team.messages(&thread));
    assert_eq!(posted.len(), 1, "{posted:?}");
    assert!(posted[0].contains("ceiling"), "{}", posted[0]);
}

/// **The ceiling fires inside a grace too.** A second continue thirty seconds before the ceiling
/// moves the slot's clock, so the look at the ceiling finds the new clock's grace not yet reached —
/// a look that waits. It escalates all the same.
#[test]
fn the_ceiling_fires_at_the_ceiling_while_the_look_waits_out_a_grace() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    team.tick(&at(130), &thread);
    let ceiling = 130 + nexus_chat::ladder::RECOVERY_CEILING_FLOOR_SECS;
    team.ended(&at(ceiling - 150), &session, None);
    team.tick(&at(ceiling - 30), &thread);
    assert_eq!(
        team.log(&thread, "coder")
            .iter()
            .filter(|(a, _)| a == "continued")
            .count(),
        2,
        "continued again inside the episode"
    );
    assert_eq!(team.book.held_look(&thread, "coder"), Some(at(ceiling)));

    team.tick(&at(ceiling), &thread);

    let posted = escalations(&team.messages(&thread));
    assert_eq!(posted.len(), 1, "{posted:?}");
    assert!(posted[0].contains("ceiling"), "{}", posted[0]);
}

// ---- the claim -----------------------------------------------------------------------------------

/// A claim another look holds is honoured: zero triggers while `recovering_until` is in the future,
/// exactly one once the injected clock is past it.
#[test]
fn a_held_claim_fires_nothing_and_one_past_it_fires_exactly_once() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    team.sql(&format!(
        "UPDATE owed_starts SET recovering_until = '{}' WHERE thread = '{thread}'",
        at(200)
    ));
    team.tick(&at(130), &thread);
    assert_eq!(
        team.worker.of("coder").len(),
        1,
        "claimed elsewhere: nothing"
    );
    assert_eq!(team.ladder(&thread, "coder").unwrap().attempts, 0);
    team.tick(&at(201), &thread);
    assert_eq!(
        team.worker.of("coder").len(),
        2,
        "past the claim: exactly one"
    );
    team.tick(&at(202), &thread);
    assert_eq!(team.worker.of("coder").len(), 2, "and only one");
}

/// A trigger refused `AlreadyRunning` releases the claim and counts nothing.
#[test]
fn an_already_running_refusal_releases_the_claim_and_counts_nothing() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    *team.worker.refuse.lock().unwrap() = Some("already_running");
    team.tick(&at(130), &thread);
    let ladder = team.ladder(&thread, "coder").unwrap();
    assert_eq!(ladder.attempts, 0, "nothing counted");
    assert_eq!(ladder.recovery_started_at, None);
    assert_eq!(ladder.recovering_until, None, "the claim is released");
    assert_eq!(
        team.log(&thread, "coder"),
        vec![("continued".to_string(), Some("already_running".to_string()))]
    );
}

/// A permanent refusal of the trigger goes straight to rung E.
#[test]
fn a_permanent_refusal_escalates_at_once() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    *team.worker.refuse.lock().unwrap() = Some("forbidden");
    team.tick(&at(130), &thread);
    let posted = escalations(&team.messages(&thread));
    assert_eq!(posted.len(), 1, "{posted:?}");
    assert!(posted[0].contains("refuses the role"), "{}", posted[0]);
}

// ---- rung A: ask (field case 2b) -----------------------------------------------------------------

/// A live session with no life of its own on its slot for 10 minutes is NUDGED, through the door,
/// with the one text that offers `nxc confirm --rest` — once, and counting no attempt. Nothing is
/// stopped and nothing is started. Before the 10 minutes, nothing at all (nxf 6j6v.1r61).
#[test]
fn a_live_session_silent_on_its_slot_for_ten_minutes_is_nudged_once_with_the_confirm_text() {
    let team = team();
    let thread = team
        .commission(T0, "coder", "Add a --since flag.")
        .thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.tick(&at(599), &thread);
    assert!(team.worker.asks().is_empty(), "not yet");
    team.tick(&at(600), &thread);
    let asks = team.worker.asks();
    assert_eq!(asks.len(), 1, "{asks:?}");
    let ask = &asks[0];
    assert_eq!(ask.session, session);
    assert_eq!(ask.thread, thread);
    assert_eq!(ask.role, team.q("coder"));
    assert_eq!(ask.text, nexus_chat::ladder::ask_text(&thread, 10, 90));
    for needle in [
        "no sign of life from you for 10 minutes".to_string(),
        format!("If you have already replied on thread {thread}, do nothing and end your turn."),
        format!("`nxc reply --thread {thread} -`"),
        format!("`nxc confirm --thread {thread} --rest <minutes>` (at most 60; 90 minutes of rest remain)"),
        format!("`nxc reply --thread {thread} --escalate -`"),
    ] {
        assert!(ask.text.contains(&needle), "{needle:?} missing from {}", ask.text);
    }
    assert!(!ask.text.contains("--progress"), "{}", ask.text);
    let ladder = team.ladder(&thread, "coder").unwrap();
    assert_eq!(ladder.attempts, 0, "a nudge counts no attempt");
    assert_eq!(ladder.asked_at.as_deref(), Some(at(600).as_str()));
    assert_eq!(
        ladder.recovery_started_at.as_deref(),
        Some(at(600).as_str()),
        "the nudge opened the episode"
    );
    assert_eq!(
        team.log(&thread, "coder"),
        vec![("asked".to_string(), Some("asked".to_string()))]
    );
    assert_eq!(team.worker.of("coder").len(), 1, "nothing started");
    assert!(team.worker.stops().is_empty(), "nothing stopped");
    // The next look inside the nudge's wait asks nothing more.
    team.tick(&at(660), &thread);
    assert_eq!(team.worker.asks().len(), 1);
    // The status row says so, never "stale".
    let row = team.status_of(&at(601), &thread);
    let recovery = row.recovery.expect("under recovery");
    assert_eq!(recovery.action, nexus_chat::ladder::RecoveryAction::Asked);
    assert_eq!((recovery.attempt, recovery.of), (0, 3));
    assert!(!row.stale);
}

/// **A live session that is writing is never stopped and never settled** (must-fix, design review
/// round 2 CQ #3): a coordinated member of a two-minute-window flow writes on its slot long past its
/// window. Nothing asks it, nothing stops it, and the flow does not move past it.
#[test]
fn a_transcript_written_past_the_window_stops_nothing_and_settles_nothing() {
    let team = team();
    let root = team.commission(T0, "quick", "verify it").thread_id;
    let verifier = team.worker.last("verifier");
    let slot = verifier.reply_thread.clone().unwrap();
    let session = verifier.internal_session.clone();
    team.worker.run(&session);
    team.turn(&session, &slot, &at(1));
    // The window is two hours; the session works on for three, an entry every ten minutes.
    let mut now = 600;
    while now <= 3 * 3_600 {
        team.work(&session, &at(now));
        let r = team.tick(&at(now + 1), &root);
        assert_ne!(r.reason, "advanced", "at +{now}s: {:?}", r.warnings);
        now += 600;
    }
    assert!(
        team.worker.asks().is_empty(),
        "a session that writes is not asked"
    );
    assert!(
        team.worker.stopped.lock().unwrap().is_empty(),
        "nothing stopped"
    );
    assert!(
        team.worker.of("finisher").is_empty(),
        "the flow did not move past it"
    );
    assert!(team.status_of(&at(now), &slot).stranded.is_none());
}

/// **Silence is per slot, attributed by the TURN its message opened** (must-fix, design review round
/// 3 CQ #6, round 4 CQ #1 / TQ #3 / IR #5) — first order: one session serves slot X, then slot Y is
/// fired into it while it is busy on X. X writes for 25 minutes. Y has no turn of its own yet, so its
/// silence runs from its trigger and it is asked at 20 minutes; X, which is writing, is not.
#[test]
fn silence_is_per_slot_y_fired_into_a_session_busy_on_x_is_asked_and_x_is_not() {
    let team = team();
    let x = team.commission(T0, "pm", "plan the X feature").thread_id;
    let session = team.worker.last("pm").internal_session;
    let y = team.commission(&at(60), "coder", "build Y").thread_id;
    // Y's slot is served by the SAME session — one persona, one real session, many threads.
    team.sql(&format!(
        "UPDATE owed_starts SET session = '{session}' WHERE thread = '{y}'"
    ));
    team.worker.run(&session);
    team.turn(&session, &x, &at(1));
    let mut now = 120;
    while now <= 25 * 60 {
        team.work(&session, &at(now));
        now += 120;
    }
    team.tick(&at(60 + 1_200), &y);
    team.tick(&at(60 + 1_200), &x);
    let asked: Vec<String> = team.worker.asks().into_iter().map(|a| a.thread).collect();
    assert_eq!(asked, vec![y.clone()], "Y asked, X not");
}

/// …and the other order: Y's turn, then X's, then Y's again with work on it. Y's entries follow Y's
/// newest `turn_start`, so they count for Y — it is not asked — while X, whose turn ended at Y's,
/// has been silent since and is.
#[test]
fn silence_is_per_slot_work_after_ys_own_turn_counts_for_y() {
    let team = team();
    let x = team.commission(T0, "pm", "plan the X feature").thread_id;
    let session = team.worker.last("pm").internal_session;
    let y = team.commission(T0, "coder", "build Y").thread_id;
    team.sql(&format!(
        "UPDATE owed_starts SET session = '{session}' WHERE thread = '{y}'"
    ));
    team.worker.run(&session);
    team.turn(&session, &y, &at(10));
    team.turn(&session, &x, &at(20));
    team.work(&session, &at(30));
    team.turn(&session, &y, &at(40));
    team.work(&session, &at(1_000));
    team.work(&session, &at(1_190));
    team.tick(&at(1_250), &y);
    team.tick(&at(1_250), &x);
    let asked: Vec<String> = team.worker.asks().into_iter().map(|a| a.thread).collect();
    assert_eq!(asked, vec![x.clone()], "X asked; Y's own work counts for Y");
}

// ---- a session inside one tool call (nxf 6j6v.j6yk) ---------------------------------------------

/// **A session inside one tool call is working, not silent** (nxf 6j6v.j6yk; field case: the merger
/// watching a ~1 h CI run on 0.220.0 was asked three times and escalated while it worked). Its last
/// entry is a `tool_use` with no `tool_result`; an ask would only reach it at the turn's end, so the
/// coordinator neither asks nor escalates while the call runs — here for four ask intervals. Once
/// the call returns, silence counts again from its result.
#[test]
fn a_session_inside_one_tool_call_longer_than_three_ask_intervals_is_neither_asked_nor_escalated() {
    use nexus_chat::ladder::SILENCE_SECS;
    let team = team();
    let thread = team
        .commission(T0, "coder", "Merge it and watch the push run.")
        .thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.turn(&session, &thread, &at(1));
    team.tool_call(&session, "toolu_watch", "Bash", &at(60));
    let mut now = 300;
    while now <= 60 + 4 * SILENCE_SECS {
        team.tick(&at(now), &thread);
        now += 300;
    }
    assert!(
        team.worker.asks().is_empty(),
        "a session inside a tool call is not asked: {:?}",
        team.worker.asks()
    );
    assert!(escalations(&team.messages(&thread)).is_empty());
    assert!(team.log(&thread, "coder").is_empty());
    assert_eq!(team.ladder(&thread, "coder").unwrap().attempts, 0);
    assert!(team.status_of(&at(now), &thread).recovery.is_none());
    // The call returns; from its result on the session is as silent as anybody else.
    team.tool_returned(&session, "toolu_watch", &at(now));
    team.tick(&at(now + SILENCE_SECS - 1), &thread);
    assert!(team.worker.asks().is_empty(), "not yet");
    team.tick(&at(now + SILENCE_SECS), &thread);
    assert_eq!(team.worker.asks().len(), 1, "asked once the call is over");
}

/// **A session that is really silent is nudged and stopped** — none of its calls is open: one
/// returned, and one has no result on record but the model spoke after it, so it is over too. Its
/// last life is at +60; the looks every five minutes nudge it at the first look past +660 and stop
/// it five minutes later. Its process ignores the SIGTERM: the one further SIGTERM comes at the
/// stop's grace and the slot escalates at twice it, naming the process that would not stop
/// (nxf 6j6v.1r61, design 8nx4 section 4 step 1).
#[test]
fn a_session_with_no_tool_call_open_is_nudged_stopped_and_escalated_when_it_will_not_stop() {
    let team = team();
    let thread = team
        .commission(T0, "coder", "Add a --since flag.")
        .thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.turn(&session, &thread, &at(1));
    team.tool_call(&session, "toolu_read", "Read", &at(30));
    team.tool_returned(&session, "toolu_read", &at(40));
    team.tool_call(&session, "toolu_lost", "Bash", &at(50));
    team.work(&session, &at(60));
    let mut now = 300;
    while now <= 2_400 {
        team.tick(&at(now), &thread);
        now += 300;
    }
    let asks = team.worker.asks();
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert_eq!(asks[0].at, at(900));
    assert_eq!(team.worker.stops(), vec![session.clone(), session.clone()]);
    let rows: Vec<(String, i64)> = team
        .log_at(&thread, "coder")
        .into_iter()
        .map(|(a, _, t)| (a, t))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("asked".into(), 900),
            ("stopped".into(), 1_200),
            ("stopped".into(), 1_500),
            ("escalated".into(), 1_800)
        ]
    );
    let posted = escalations(&team.messages(&thread));
    assert_eq!(posted.len(), 1, "{posted:?}");
    assert!(posted[0].contains("would not stop"), "{}", posted[0]);
    assert!(
        posted[0].contains("- last pulse: this host reports no pulse"),
        "{}",
        posted[0]
    );
}

/// **A tool call that outlives its own bound is hung** — and an ask cannot reach a session inside
/// it, so none is spent: the coordinator escalates once, naming the call and the bound.
#[test]
fn a_tool_call_that_outlives_its_bound_is_escalated_as_hung_without_an_ask() {
    use nexus_chat::ladder::TOOL_CALL_BOUND_SECS;
    let team = team();
    let thread = team
        .commission(T0, "coder", "Merge it and watch the push run.")
        .thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.turn(&session, &thread, &at(1));
    team.tool_call(&session, "toolu_watch", "Bash", &at(60));
    team.tick(&at(60 + TOOL_CALL_BOUND_SECS - 1), &thread);
    assert!(escalations(&team.messages(&thread)).is_empty(), "not yet");
    team.tick(&at(60 + TOOL_CALL_BOUND_SECS), &thread);
    let escalated = escalations(&team.messages(&thread));
    assert_eq!(escalated.len(), 1, "{:?}", team.messages(&thread));
    assert!(
        escalated[0].contains("tool call")
            && escalated[0].contains("Bash")
            && escalated[0].contains(&at(60)),
        "{}",
        escalated[0]
    );
    assert!(team.worker.asks().is_empty(), "no ask was spent on it");
    team.tick(&at(60 + TOOL_CALL_BOUND_SECS + 600), &thread);
    assert_eq!(escalations(&team.messages(&thread)).len(), 1, "once");
}

// ---- the budget and the ceiling -------------------------------------------------------------------

/// **After rung E and a commissioner reply**, a later silence gets a FRESH ask — not an immediate
/// escalation (must-fix, design review round 4 TQ #4): rung E's record ends the episode ON THE ROW,
/// clearing the budget and the ceiling clock, so the same `(thread, role)` owed again — the
/// commissioner putting the task to the same session on the same thread — starts from nothing.
#[test]
fn after_an_escalation_and_a_commissioner_reply_a_later_silence_gets_a_fresh_ask() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    let mut now = 10;
    for _ in 0..4 {
        team.ended(&at(now), &session, None);
        now += 120;
        team.tick(&at(now), &thread);
        now += 60;
    }
    assert_eq!(escalations(&team.messages(&thread)).len(), 1);
    let after = team
        .ladder(&thread, "coder")
        .expect("the row, before the next sweep");
    assert_eq!(
        (
            after.attempts,
            after.recovery_started_at.clone(),
            after.asked_at.clone()
        ),
        (0, None, None),
        "the episode ended with the record"
    );
    // The commissioner answers and puts the task to the same session on the same thread again —
    // the expectation re-declared and the slot's clock moved by that trigger, the session at work.
    // (Which reply does this on its own is cut 2's, 6j6v.q2dp; what is pinned here is the row.)
    team.owner_says(&at(now), &thread, "use the old flag name and carry on");
    let mut store = team.store();
    store.set_expects_reply_from(
        &thread,
        &serde_json::to_string(&[team.q("coder")]).unwrap(),
        &team.q("owner"),
    );
    drop(store);
    team.sql(&format!(
        "UPDATE owed_starts SET trigger_clock = '{}', triggered = 1 WHERE thread = '{thread}'",
        at(now)
    ));
    team.worker.run(&session);
    team.tick(&at(now + 599), &thread);
    assert!(team.worker.asks().is_empty(), "not at once");
    team.tick(&at(now + 600), &thread);
    assert_eq!(
        team.worker.asks().len(),
        1,
        "a fresh ask after a fresh silence"
    );
    let ladder = team.ladder(&thread, "coder").unwrap();
    assert_eq!(ladder.attempts, 0, "a nudge counts no attempt");
    assert_eq!(
        ladder.recovery_started_at.as_deref(),
        Some(at(now + 600).as_str()),
        "a fresh episode"
    );
    assert_eq!(
        escalations(&team.messages(&thread)).len(),
        1,
        "not a second escalation"
    );
}

// ---- the claim, continued ---------------------------------------------------------------------------

/// **A trigger slower than the claim never runs a second rung** (must-fix, design review round 3
/// TQ #4): a rung still in flight — its log row has no outcome — refuses the next look for up to
/// ten minutes even after the claim itself has lapsed on the injected clock; past that, the stale
/// row is settled `timed_out` and the slot is taken up again.
#[test]
fn a_rung_in_flight_past_the_claim_blocks_a_second_rung_until_the_trigger_timeout() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    team.sql(&format!(
        "INSERT INTO recovery_log(thread, role, at, action, attempt, session, detail)
         VALUES ('{thread}', '{}', '{}', 'continued', 1, '{session}', 'in flight')",
        team.q("coder"),
        at(100)
    ));
    team.tick(&at(161), &thread);
    assert_eq!(
        team.worker.of("coder").len(),
        1,
        "past the claim, still in flight"
    );
    team.tick(&at(699), &thread);
    assert_eq!(team.worker.of("coder").len(), 1);
    team.tick(&at(700), &thread);
    assert_eq!(team.worker.of("coder").len(), 2, "past the trigger timeout");
    let log = team.log(&thread, "coder");
    assert_eq!(log[0].1.as_deref(), Some("timed_out"));
}

/// **A second look while the first one's trigger runs starts nothing** — on two connections,
/// exactly one trigger. Measured by mutation: this stays green with the claim AND the in-flight
/// recheck both disabled, because the first look's trigger has already moved the slot's clock
/// before it reaches the worker. It pins that end-to-end outcome only; the guards are pinned one
/// by one by `a_held_claim_fires_nothing_and_one_past_it_fires_exactly_once`,
/// `a_rung_in_flight_past_the_claim_blocks_a_second_rung_until_the_trigger_timeout` and
/// `a_trigger_slower_than_the_claim_keeps_it_and_a_look_past_the_in_flight_guard_fires_nothing`.
#[test]
fn a_second_look_while_the_first_ones_trigger_runs_starts_nothing() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    let (release, gate) = std::sync::mpsc::channel();
    *team.worker.gate.lock().unwrap() = Some(gate);
    std::thread::scope(|scope| {
        let first = scope.spawn(|| team.tick(&at(130), &thread));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !team
            .worker
            .at_the_gate
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the first look never reached the worker"
            );
            std::thread::yield_now();
        }
        team.tick(&at(131), &thread);
        release.send(()).unwrap();
        first.join().unwrap();
    });
    assert_eq!(
        team.worker.of("coder").len(),
        2,
        "the first start and ONE continuation"
    );
    assert_eq!(team.ladder(&thread, "coder").unwrap().attempts, 1);
}

// ---- status -------------------------------------------------------------------------------------------

/// **`nxc resume` over a claimed slot** answers `BeingRecovered` with the renewal horizon, and
/// starts nothing.
#[test]
fn resume_over_a_claimed_slot_answers_being_recovered_with_the_renewal_horizon() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.with(&at(10), "service", None, |ctx, store| {
        orchestration::session_interrupted(ctx, store, &session, "five_hour", None, "quota")
    })
    .unwrap();
    team.sql(&format!(
        "UPDATE owed_starts SET recovering_until = '{}', attempts = 2 WHERE thread = '{thread}'",
        at(70)
    ));
    let r = team
        .with(&at(20), "owner", None, |ctx, store| {
            orchestration::resume_interrupted(
                ctx,
                store,
                orchestration::ResumeRequest {
                    scope: orchestration::ResumeScope::Thread(&thread),
                    force: true,
                },
            )
        })
        .unwrap();
    assert_eq!(
        r.outcome,
        orchestration::ResumeOutcome::BeingRecovered {
            attempt: 2,
            of: 3,
            action: None,
            retry_at: at(70),
        }
    );
    assert_eq!(team.worker.of("coder").len(), 1, "nothing started");
}

/// A recovering slot is where its operation hangs — `hangs_at` names it, with the rung.
#[test]
fn a_recovering_slot_is_where_its_operation_hangs() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    team.tick(&at(130), &thread);
    let op = team
        .status(&at(131))
        .into_iter()
        .find(|o| o.root == thread)
        .unwrap();
    let hangs = op.hangs_at.expect("hangs at the coder");
    assert_eq!(hangs.thread, thread);
    assert_eq!(hangs.recovery, facade::SlotRecovery::Recovering);
    let row = team.status_of(&at(131), &thread);
    let recovery = row.recovery.unwrap();
    assert_eq!(
        recovery.action,
        nexus_chat::ladder::RecoveryAction::Continued
    );
    assert_eq!(recovery.next_look.as_deref(), Some(at(250).as_str()));
}

// ---- rung B: hold ------------------------------------------------------------------------------------

/// **A resume the provider refused goes to rung D, never to rung C** (must-fix, design review round 2
/// CQ #4): the hold records `provider_cannot_resume`, and the slot's coordinator re-commissions it
/// with its original body — a resume the provider refused is not retried.
#[test]
fn a_resume_the_provider_refused_is_recommissioned_never_continued() {
    let team = team();
    let thread = team
        .commission(T0, "coder", "Add a --since flag.")
        .thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 3, &at(5));
    team.interrupted(&at(10), &session, None);
    team.ended(
        &at(10),
        &session,
        Some("on hold at an availability boundary (five_hour)"),
    );
    *team.worker.no_resume.lock().unwrap() = true;
    let r = team.resume(&at(20), &thread);
    assert_eq!(
        r.outcome,
        orchestration::ResumeOutcome::ProviderCannotResume
    );
    let outcome: Option<String> = team
        .store()
        .connection()
        .query_row(
            "SELECT outcome FROM session_interruption WHERE session = ?1",
            [&session],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(outcome.as_deref(), Some("provider_cannot_resume"));
    team.tick(&at(10 + 120), &thread);
    let starts = team.worker.of("coder");
    assert_eq!(starts.len(), 2, "one re-commission");
    assert_ne!(
        starts[1].internal_session, session,
        "a fresh session, never a continuation"
    );
    assert_eq!(starts[1].resume_real, None);
    assert!(starts[1].message.contains("Add a --since flag."));
    assert_eq!(team.log(&thread, "coder")[0].0, "recommissioned");
}

/// **A hold that names no instant and waits on nobody is continued one grace after it was taken**
/// (must-fix, design review round 2 CQ #4): no resume will ever take it up, so its coordinator does —
/// in the same session — and the hold stops standing.
#[test]
fn a_hold_without_an_instant_waiting_on_nobody_is_continued_one_grace_after_it() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 2, &at(5));
    team.interrupted(&at(10), &session, None);
    team.ended(
        &at(10),
        &session,
        Some("on hold at an availability boundary (five_hour)"),
    );
    team.tick(&at(10 + 119), &thread);
    assert_eq!(team.worker.of("coder").len(), 1, "inside the grace");
    team.tick(&at(10 + 120), &thread);
    let starts = team.worker.of("coder");
    assert_eq!(starts.len(), 2, "continued");
    assert_eq!(starts[1].internal_session, session, "the SAME session");
    let row = team.status_of(&at(131), &thread);
    assert!(row.interrupted.is_none(), "the hold no longer stands");
}

/// **The instant a paused window ends** (must-fix, design review round 2 CQ #4):
/// `resumed_at + (deadline - hold.at)` — the remainder the window had at the hold, counted from the
/// take-up, not a whole window again.
#[test]
fn a_paused_window_ends_at_the_take_up_plus_the_remainder_it_had_at_the_hold() {
    let team = team();
    team.commission(T0, "quick", "verify it");
    let verifier = team.worker.last("verifier");
    let slot = verifier.reply_thread.clone().unwrap();
    let session = verifier.internal_session.clone();
    let armed: String = team
        .store()
        .connection()
        .query_row(
            "SELECT deadline FROM channel_member_deadline WHERE thread_id = ?1",
            [&slot],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(armed, at(7_200), "the premise: a two-hour window from T0");
    team.bound_with_transcript(&session, 1, &at(5));
    team.interrupted(&at(600), &session, Some(&at(4_200)));
    team.ended(&at(600), &session, None);
    let r = team.resume(&at(4_200), &slot);
    assert_eq!(r.outcome, orchestration::ResumeOutcome::Resumed, "{r:?}");
    let resumed: String = team
        .store()
        .connection()
        .query_row(
            "SELECT deadline FROM channel_member_deadline WHERE thread_id = ?1",
            [&slot],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        resumed,
        at(4_200 + (7_200 - 600)),
        "resumed_at + (deadline - hold.at)"
    );
}

// ---- never settled by staleness: the four paths ---------------------------------------------------------

/// **The parallel set** (path ii): a review round whose reviewer answered and whose verifier — its
/// slot coordinated — is still working past the round's window. The set is not settled and the round
/// is not consolidated; the window settles nothing for the coordinated slot.
#[test]
fn a_parallel_round_is_not_consolidated_past_its_window_while_a_coordinated_member_works() {
    let team = team();
    // Commissioned by a SESSION, so the round has a return address and its tick consolidates it.
    team.commission(T0, "pm", "plan it");
    let pm = team.worker.last("pm").internal_session;
    let root = team
        .session_sends(T0, "pm", &pm, "review", "review the change")
        .thread_id;
    let verifier = team.worker.last("verifier");
    let reviewer = team.worker.last("reviewer");
    team.worker.run(&verifier.internal_session);
    team.reply(
        &at(60),
        "reviewer",
        &reviewer.internal_session,
        reviewer.reply_thread.as_deref().unwrap(),
        "looks fine",
    );
    let slot = verifier.reply_thread.clone().unwrap();
    team.turn(&verifier.internal_session, &slot, &at(1));
    team.work(&verifier.internal_session, &at(7_200));
    let r = team.tick(&at(7_201), &root);
    assert!(!r.acted, "{r:?}");
    let delivered = team
        .messages(&root)
        .into_iter()
        .filter(|(sender, _, _)| sender.contains("/__"))
        .count();
    assert_eq!(delivered, 0, "the round waits for its coordinated member");
    assert_eq!(
        team.status_of(&at(7_201), &slot).outstanding,
        vec![team.q("verifier")]
    );
}

/// …and its sibling: the SAME round with the silent slot owed by a HUMAN — a slot nobody coordinates
/// — lapses at its window and is consolidated as today.
#[test]
fn a_parallel_round_whose_silent_slot_is_human_owed_lapses_as_today() {
    let team = team();
    // Commissioned by a SESSION, so the round has a return address and its tick consolidates it.
    team.commission(T0, "pm", "plan it");
    let pm = team.worker.last("pm").internal_session;
    let root = team
        .session_sends(T0, "pm", &pm, "review", "review the change")
        .thread_id;
    let verifier = team.worker.last("verifier");
    let reviewer = team.worker.last("reviewer");
    let slot = verifier.reply_thread.clone().unwrap();
    let mut store = team.store();
    store.set_expects_reply_from(
        &slot,
        &serde_json::to_string(&[team.q("owner")]).unwrap(),
        &team.q("__channel__"),
    );
    drop(store);
    team.reply(
        &at(60),
        "reviewer",
        &reviewer.internal_session,
        reviewer.reply_thread.as_deref().unwrap(),
        "looks fine",
    );
    team.tick(&at(7_201), &root);
    let delivered = team
        .messages(&root)
        .into_iter()
        .filter(|(sender, _, _)| sender.contains("/__"))
        .count();
    assert_eq!(
        delivered, 1,
        "the human-owed slot lapsed and the round consolidated"
    );
}

/// **A nested round's slot is not settled while a slot inside it is coordinated** (clause 6): the
/// coding chain's review step stands for the review round; its verifier works past the window, and
/// neither the round nor the coding chain moves.
#[test]
fn a_nested_rounds_slot_is_not_settled_while_a_member_inside_it_is_coordinated() {
    let team = team();
    // Commissioned by a SESSION, so both levels have a return address and would consolidate.
    team.commission(T0, "pm", "plan it");
    let pm = team.worker.last("pm").internal_session;
    let coding = team
        .session_sends(T0, "pm", &pm, "coding", "build and review")
        .thread_id;
    let coder = team.worker.last("coder");
    team.reply(
        &at(10),
        "coder",
        &coder.internal_session,
        coder.reply_thread.as_deref().unwrap(),
        "built",
    );
    let verifier = team.worker.last("verifier");
    let reviewer = team.worker.last("reviewer");
    team.worker.run(&verifier.internal_session);
    team.reply(
        &at(60),
        "reviewer",
        &reviewer.internal_session,
        reviewer.reply_thread.as_deref().unwrap(),
        "looks fine",
    );
    let slot = verifier.reply_thread.clone().unwrap();
    team.turn(&verifier.internal_session, &slot, &at(11));
    team.work(&verifier.internal_session, &at(7_300));
    let round = team.store().thread_parent(&slot).unwrap().unwrap();
    for thread in [&round, &coding] {
        let r = team.tick(&at(7_400), thread);
        assert!(!r.acted, "{thread}: {r:?}");
    }
    assert!(team.worker.of("finisher").is_empty());
    assert_eq!(
        team.status_of(&at(7_400), &slot).outstanding,
        vec![team.q("verifier")],
        "the member is still owed"
    );
    assert_eq!(
        team.status_of(&at(7_400), &round).outstanding,
        vec![team.q("__channel__")],
        "the round is not consolidated: it still waits on its supervisor"
    );
    assert!(
        !team.status_of(&at(7_400), &coding).outstanding.is_empty(),
        "and the chain waits on it"
    );
}

// ---- the child's bound (field case 2) -------------------------------------------------------------------

/// **A child a session opened inherits `clamp(parent_remaining_window, 30 min, 2 h)`** (must-fix,
/// design review round 3 TQ #7) — at the floor, inside the bounds, and above two hours.
#[test]
fn a_child_a_session_opens_is_bound_by_its_parents_remaining_window_within_the_clamp() {
    for (remaining, bound) in [(900, 1_800), (3_600, 3_600), (10_000, 7_200)] {
        let team = team();
        team.commission(T0, "quick", "verify it");
        let verifier = team.worker.last("verifier");
        let slot = verifier.reply_thread.clone().unwrap();
        // The parent slot's armed member clock — what its quorum reads as its deadline.
        team.sql(&format!(
            "UPDATE channel_member_deadline SET deadline = '{}' WHERE thread_id = '{slot}'",
            at(100 + remaining)
        ));
        let child = team
            .session_sends(
                &at(100),
                "verifier",
                &verifier.internal_session,
                "coder",
                "check X",
            )
            .thread_id;
        assert_eq!(
            team.deadline_of(&child),
            Some(at(100 + bound)),
            "parent has {remaining} s left"
        );
        assert!(
            team.ladder(&child, "coder").is_some(),
            "its row is armed at the open"
        );
    }
}

/// **The unsupervised branch** (path iii): a child round past its bound, coordinated, is never
/// consolidated by its window.
#[test]
fn a_coordinated_child_past_its_bound_is_never_settled_by_it() {
    let team = team();
    let parent = team.commission(T0, "pm", "plan it").thread_id;
    let pm = team.worker.last("pm").internal_session;
    let child = team
        .session_sends(&at(10), "pm", &pm, "coder", "check X")
        .thread_id;
    assert_eq!(
        team.deadline_of(&child),
        Some(at(10 + 7_200)),
        "a parent with no window: two hours"
    );
    let coder = team.worker.last("coder").internal_session;
    team.worker.run(&coder);
    team.turn(&coder, &child, &at(11));
    team.work(&coder, &at(7_300));
    let r = team.tick(&at(7_400), &child);
    assert!(!r.acted, "{r:?}");
    assert_eq!(
        team.status_of(&at(7_400), &child).outstanding,
        vec![team.q("coder")]
    );
    let _ = parent;
}

// ---- field case 2: a parent and its orphaned child -----------------------------------------------

/// **Field case 2 (a)**: a pm session commissions a verifier with its own `nxc send` and ends its
/// turn waiting. The child binds, writes, and is killed — no end announced, no process. Its
/// coordinator (the parent's thread) finds it at the grace and continues it; silent every time, it
/// is escalated after three attempts — INTO the child's thread, in the member's name — and the
/// parent session, which has no live process, is continued with that outcome exactly once.
#[test]
fn field_case_2a_a_killed_child_is_continued_then_escalated_and_its_parent_is_woken_once() {
    let team = team();
    let parent = team.commission(T0, "pm", "plan the release").thread_id;
    let pm = team.worker.last("pm").internal_session;
    let child = team
        .session_sends(&at(10), "pm", &pm, "verifier", "check the release notes")
        .thread_id;
    team.ended(&at(20), &pm, Some("ended"));
    let verifier = team.worker.last("verifier").internal_session;
    team.bound_with_transcript(&verifier, 2, &at(30));
    // Killed: no end, no process. Two looks a cadence apart, from its clock.
    let mut now = 10;
    for attempt in 1..=3 {
        now += 120;
        team.tick(&at(now), &parent);
        now += 90;
        team.tick(&at(now), &parent);
        assert_eq!(
            team.worker.of("verifier").len(),
            1 + attempt,
            "attempt {attempt}: continued"
        );
    }
    let pm_starts_before = team.worker.of("pm").len();
    now += 120;
    team.tick(&at(now), &parent);
    now += 90;
    team.tick(&at(now), &parent);
    let posted = escalations(&team.messages(&child));
    assert_eq!(posted.len(), 1, "escalated into the child's thread");
    assert!(posted[0].contains("attempt 3/3"), "{}", posted[0]);
    let woken: Vec<TriggerRequest> = team.worker.of("pm")[pm_starts_before..].to_vec();
    assert_eq!(woken.len(), 1, "the parent is continued exactly once");
    assert_eq!(woken[0].internal_session, pm, "its own session");
    assert!(
        woken[0].message.contains("ESCALATION") && woken[0].message.contains(&child),
        "with the child's outcome:\n{}",
        woken[0].message
    );
    team.tick(&at(now + 600), &parent);
    assert_eq!(
        team.worker.of("pm").len(),
        pm_starts_before + 1,
        "and only once"
    );
}

/// **Field case 2 (c)**: the child answers while its parent session is still running — the answer is
/// HELD for it. The parent's process then goes without a word; its held answer is delivered by the
/// next tick, through the parent slot's claim, exactly once.
#[test]
fn field_case_2c_a_held_answer_for_a_parent_with_no_live_process_is_delivered_once() {
    let team = team();
    let parent = team.commission(T0, "pm", "plan the release").thread_id;
    let pm = team.worker.last("pm").internal_session;
    team.worker.run(&pm);
    let child = team
        .session_sends(&at(10), "pm", &pm, "verifier", "check the release notes")
        .thread_id;
    let verifier = team.worker.last("verifier").internal_session;
    team.reply(&at(60), "verifier", &verifier, &child, "notes are fine");
    assert!(
        !team.store().held_wakes(&pm).unwrap().is_empty(),
        "held for the running parent"
    );
    let before = team.worker.of("pm").len();
    team.worker.stop(&pm);
    team.tick(&at(120), &parent);
    team.tick(&at(240), &parent);
    let woken = team.worker.of("pm")[before..].to_vec();
    assert_eq!(woken.len(), 1, "delivered exactly once");
    assert!(
        woken[0].message.contains("notes are fine"),
        "{}",
        woken[0].message
    );
}

// ---- `--accept` names a recovering slot ------------------------------------------------------------

/// **`--accept`'s refusal names a slot its coordinator is recovering** (nxf 6j6v.558h): the rework
/// step past its window is neither silent nor dead — its coordinator owns it — and accepting over it
/// would open the next step beside it, so the refusal says which thread is being recovered.
#[test]
fn an_acceptance_over_a_recovering_slot_is_refused_and_names_it() {
    let team = team();
    team.commission(T0, "pm", "ship it");
    let pm = team.worker.last("pm").internal_session;
    let board = team
        .session_sends(T0, "pm", &pm, "cycle", "build the thing")
        .thread_id;
    let coder = team.worker.last("coder");
    team.reply(
        &at(10),
        "coder",
        &coder.internal_session,
        coder.reply_thread.as_deref().unwrap(),
        "first cut is in",
    );
    let reviewer = team.worker.last("reviewer");
    team.with(
        &at(20),
        "reviewer",
        Some(&reviewer.internal_session),
        |ctx, store| {
            surface::reply_in_thread(
                ctx,
                store,
                ReplyThreadRequest {
                    machine: None,
                    thread: reviewer.reply_thread.as_deref().unwrap(),
                    body: "not yet",
                    escalate: false,
                    needs_rework: true,
                    accept: false,
                },
            )
        },
    )
    .expect("the verdict is posted");
    let rework = team.worker.last("coder").reply_thread.unwrap();
    // The rework step: past its two-minute window, its session alive — its coordinator owns it.
    team.worker.run(&team.worker.last("coder").internal_session);
    let err = team
        .with(&at(400), "pm", Some(&pm), |ctx, store| {
            surface::reply_in_thread(
                ctx,
                store,
                ReplyThreadRequest {
                    machine: None,
                    thread: &board,
                    body: "ship it anyway",
                    escalate: false,
                    needs_rework: false,
                    accept: true,
                },
            )
        })
        .expect_err("refused");
    let err = err.to_string();
    assert!(err.contains("this round is still moving"), "{err}");
    assert!(err.contains("its coordinator is recovering"), "{err}");
    assert!(err.contains(&rework), "names the slot: {err}");
}

/// **A trigger slower than the claim keeps it** (must-fix, design review round 3 TQ #4; review of
/// pass 1, CQ #1 / TQ #1): the first look's trigger blocks inside the worker. While it runs, the
/// claim's renewal moves `recovering_until` forward — seen here, past the first claim and past the
/// ten-minute in-flight horizon — and the looks after it, at instants where the in-flight guard no
/// longer refuses (its rung row is older than the trigger timeout) and where the slot again reads
/// as a session with no process, fire ZERO triggers: the renewed claim is what holds them off. With the claim or the renewal off, that look starts a second
/// continuation.
#[test]
fn a_trigger_slower_than_the_claim_keeps_it_and_a_look_past_the_in_flight_guard_fires_nothing() {
    nexus_chat::ladder::renew_claims_every(std::time::Duration::from_millis(5));
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    let (release, gate) = std::sync::mpsc::channel();
    *team.worker.gate.lock().unwrap() = Some(gate);
    let claimed_until = || {
        team.ladder(&thread, "coder")
            .and_then(|l| l.recovering_until)
    };
    // Nothing is asserted while the first look sits at the gate: a failing assertion there would
    // leave it blocked for ever. What the looks saw is collected, the gate released, then read.
    let (renewed, starts_meanwhile) = std::thread::scope(|scope| {
        let first = scope.spawn(|| team.tick(&at(130), &thread));
        // **An upper bound, not a timing assumption** (nxf 6j6v.yts5). The wait ends the moment the
        // renewal passes the horizon — 26 renewals from +130 s — and each renewal is a connection
        // and a committed write: measured 4.6 s on an idle machine and up to 6.9 s with the CPU
        // oversubscribed twice. The bound of 10 s it had left a margin of two, and the test's one
        // recorded failure came in a full-file run that took 64 s instead of the usual 15 — so the
        // bound is generous now: it decides nothing on any run that is going to pass.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let renewed_past =
            |secs: i64| claimed_until().is_some_and(|u| u.as_str() > at(secs).as_str());
        let mut renewed = false;
        while std::time::Instant::now() < deadline {
            let gated = team
                .worker
                .at_the_gate
                .load(std::sync::atomic::Ordering::SeqCst);
            if gated && renewed_past(130 + 600 + 60 + 90 + 60) {
                renewed = true;
                break;
            }
            std::thread::yield_now();
        }
        let mut starts_meanwhile = None;
        if renewed {
            // The first claim (130 + 60) and the in-flight guard (130 + 600) are both behind these
            // looks. The trigger moved the slot's clock before it reached the worker, so the slot
            // reads as a session with no process again: one look suspects it, the next one cadence
            // later would take it up — and that is the look the renewed claim has to hold off.
            team.tick(&at(130 + 600 + 60), &thread);
            team.tick(&at(130 + 600 + 60 + 90), &thread);
            starts_meanwhile = Some(team.worker.of("coder").len());
        }
        release.send(()).unwrap();
        first.join().unwrap();
        (renewed, starts_meanwhile)
    });
    assert!(
        renewed,
        "the claim was renewed past the in-flight horizon while the trigger ran: {:?}",
        claimed_until()
    );
    assert_eq!(
        starts_meanwhile,
        Some(1),
        "the looks started nothing while the first one's trigger ran"
    );
    assert_eq!(
        team.worker.of("coder").len(),
        2,
        "the first start and ONE continuation"
    );
    assert_eq!(
        team.log(&thread, "coder"),
        vec![("continued".to_string(), Some("triggered".to_string()))],
        "one rung, never timed out by a second look"
    );
    assert_eq!(
        team.ladder(&thread, "coder").unwrap().recovering_until,
        None,
        "released"
    );
}

// ---- the liveness ladder: nudge at 10, dead at 15, stop, resume cap (nxf 6j6v.1r61) ----------------

/// A coder commissioned at T0 whose session runs, opened its turn at T0, bound, and produced one
/// model entry at T0 — the slot of the liveness tests.
fn a_running_coder_with_own_life_at_t0(team: &Team) -> (String, String) {
    let thread = team
        .commission(T0, "coder", "Add a --since flag.")
        .thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.turn(&session, &thread, &at(0));
    team.bound_with_transcript(&session, 1, &at(0));
    (thread, session)
}

/// **Test 1 and the B path of test 9** (design 8nx4 sections 2, 4 and 10): a session silent from
/// T0 is nudged at the look at T0+600 exactly, stopped at T0+900, continued once its end is
/// announced (T0+1 020); the continuation writes only its `turn_start` and `session_init` — not
/// life — so it is dead at T0+1 620 without a nudge, stopped, and under policy B the slot escalates
/// at that same look, the `stopped` row before the `escalated` one. No `recovered` row anywhere,
/// and the episode's clock stands at T0+600 until the escalation clears it.
#[test]
fn a_silent_session_is_nudged_at_ten_stopped_at_fifteen_and_escalated_at_twenty_seven_under_b() {
    let team = team();
    let (thread, session) = a_running_coder_with_own_life_at_t0(&team);
    team.pulse(&session, Pulse::At(at(0)));
    for s in (60..=900).step_by(60) {
        team.tick(&at(s), &thread);
    }
    let asks = team.worker.asks();
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert_eq!(asks[0].at, at(600), "nudged at the look at T0+600 exactly");
    assert_eq!(team.worker.stops(), vec![session.clone()]);
    assert_eq!(
        team.log_at(&thread, "coder"),
        vec![
            ("asked".into(), Some("asked".into()), 600),
            ("stopped".into(), Some("stop_requested".into()), 900),
        ]
    );
    assert_eq!(
        team.owed(&thread, "coder", "recovery_started_at"),
        Some(at(600))
    );
    // The process ends at the SIGTERM; its teardown announces the end.
    team.worker.stop(&session);
    team.ended(&at(900), &session, Some("told to stop (SIGTERM)"));
    team.tick(&at(960), &thread);
    team.tick(&at(1_020), &thread);
    let resumed = team.worker.last("coder");
    assert_eq!(
        resumed.internal_session, session,
        "continued, the same session"
    );
    team.worker.run(&session);
    team.pulse(&session, Pulse::NoneYet);
    team.turn(&session, &thread, &at(1_020));
    team.entry(&session, "session_init", &at(1_021), "{}");
    assert_eq!(
        team.owed(&thread, "coder", "recovery_started_at"),
        Some(at(600)),
        "a resume ends nothing"
    );
    for s in (1_080..=1_620).step_by(60) {
        team.tick(&at(s), &thread);
    }
    let log = team.log_at(&thread, "coder");
    let actions: Vec<(&str, i64)> = log.iter().map(|(a, _, t)| (a.as_str(), *t)).collect();
    assert_eq!(
        actions,
        [
            ("asked", 600),
            ("stopped", 900),
            ("continued", 1_020),
            ("stopped", 1_620),
            ("escalated", 1_620)
        ],
        "{log:?}"
    );
    assert_eq!(
        team.worker.asks().len(),
        1,
        "the continuation is not nudged"
    );
    assert_eq!(team.worker.stops(), vec![session.clone(), session.clone()]);
    assert_eq!(team.escalated_at(&thread), vec![1_620]);
    let body = &escalations(&team.messages(&thread))[0];
    assert!(
        body.contains("dead after 1 recovery attempts on this turn; this workspace allows 1"),
        "{body}"
    );
    // Rung E's "Signs of life" block (design 8nx4 section 6): built from the slot's whole log.
    for needle in [
        "Signs of life:".to_string(),
        "- last pulse: none yet".to_string(),
        format!("- nudge at {}: asked", at(600)),
        format!("- stop at {} (its pid unknown): stop_requested", at(900)),
        format!("- stop at {} (its pid unknown): stop_requested", at(1_620)),
        format!(
            "the SIGTERM of {} to its pid unknown is UNCONFIRMED",
            at(1_620)
        ),
    ] {
        assert!(body.contains(&needle), "{needle:?} missing from:\n{body}");
    }
    assert_eq!(team.owed(&thread, "coder", "recovery_started_at"), None);
    assert!(
        team.worker
            .pulse_read_while_not_running
            .lock()
            .unwrap()
            .is_empty(),
        "the pulse is read only for a session whose process runs"
    );
}

/// Ticks the slot's coordinator at every look the service holds for it, from `from` up to `to`
/// (inclusive) — the service's book keeps one look per slot and fires it at its instant.
fn follow_looks(team: &Team, thread: &str, handle: &str, from: i64, to: i64) {
    let mut now = from;
    while now <= to {
        team.tick(&at(now), thread);
        let next = team
            .book
            .held_look(thread, handle)
            .map(|l| secs_of(&l))
            .unwrap_or(now + 60);
        now = if next > now { next } else { now + 60 };
    }
}

/// **The pulse is read only for a session whose process runs** (review of d1ey, CQ #2 / IR #3):
/// the file outlives its process, so a look whose process died between the judge's question and
/// the read must not take the dead process's last output for life. Here the process dies right
/// after the judge said `Live`; the look reads no pulse.
#[test]
fn the_pulse_is_never_read_for_a_process_that_no_longer_runs() {
    let team = team();
    let (thread, session) = a_running_coder_with_own_life_at_t0(&team);
    team.pulse(&session, Pulse::At(at(590)));
    team.worker
        .dies_after_one_question
        .lock()
        .unwrap()
        .insert(session.clone());
    team.tick(&at(600), &thread);
    assert!(
        team.worker
            .pulse_read_while_not_running
            .lock()
            .unwrap()
            .is_empty(),
        "no pulse read for a process that is gone"
    );
}

// ---- test 2: `nxc confirm --rest` (design 8nx4 section 5) ------------------------------------------

/// (a) A confirmation of 20 minutes 30 s after the nudge is granted until then plus 20 minutes, and
/// no nudge comes before the rest's end plus its grace — the look there nudges again, since the
/// session showed no life since.
#[test]
fn a_confirm_of_twenty_minutes_holds_every_nudge_until_its_end_plus_the_grace() {
    let team = team();
    let (thread, session) = a_running_coder_with_own_life_at_t0(&team);
    team.pulse(&session, Pulse::At(at(0)));
    team.tick(&at(600), &thread);
    assert_eq!(team.worker.asks().len(), 1);
    let r = team
        .confirm(&at(630), &session, &thread, 20)
        .expect("granted");
    assert_eq!(r.granted_minutes, 20);
    assert_eq!(r.until, at(630 + 1_200));
    assert_eq!(r.next_look, at(630 + 1_200 + 300));
    assert!(!r.clamped);
    assert!(r.receipt.contains(&at(630 + 1_200)), "{}", r.receipt);
    assert_eq!(
        *team.worker.withdrawn.lock().unwrap(),
        vec![session.clone()]
    );
    assert_eq!(
        team.log(&thread, "coder").last().cloned(),
        Some(("confirmed".to_string(), Some("granted".to_string())))
    );
    for s in (660..630 + 1_200 + 300).step_by(60) {
        team.tick(&at(s), &thread);
    }
    assert_eq!(
        team.worker.asks().len(),
        1,
        "no nudge inside the rest and its grace"
    );
    assert!(team.worker.stops().is_empty());
    team.tick(&at(630 + 1_200 + 300), &thread);
    let asks = team.worker.asks();
    assert_eq!(
        asks.len(),
        2,
        "nudged again at the rest's end plus the grace"
    );
    assert_eq!(asks[1].at, at(2_130));
    assert!(
        asks[1].text.contains("70 minutes of rest remain"),
        "{}",
        asks[1].text
    );
}

/// (b) Three confirmations, then the look at the fourth nudge-time escalates as STUCK, naming the
/// three confirmations and their rests — a fourth nudge never comes.
#[test]
fn after_three_confirmations_the_look_at_the_fourth_nudge_escalates_as_stuck() {
    let team = team();
    let (thread, session) = a_running_coder_with_own_life_at_t0(&team);
    team.pulse(&session, Pulse::At(at(0)));
    let mut now = 600;
    let mut confirmed = 0;
    while escalations(&team.messages(&thread)).is_empty() {
        team.tick(&at(now), &thread);
        if team.worker.asks().len() > confirmed {
            confirmed += 1;
            team.confirm(&at(now + 5), &session, &thread, 20)
                .expect("each nudge is answered with a rest");
        }
        now = team
            .book
            .held_look(&thread, "coder")
            .map(|l| secs_of(&l))
            .filter(|n| *n > now)
            .unwrap_or(now + 60);
        assert!(now < 7_000, "escalated before the ceiling");
    }
    assert_eq!(team.worker.asks().len(), 3, "a fourth nudge never comes");
    assert_eq!(
        team.owed(&thread, "coder", "confirmations"),
        Some("0".into()),
        "rung E ended the episode"
    );
    let body = &escalations(&team.messages(&thread))[0];
    assert!(body.contains("stuck: it confirmed 3 times"), "{body}");
    assert!(body.contains("rest 20 min until"), "{body}");
    assert!(
        team.worker.stops().is_empty(),
        "stuck is escalated, not stopped"
    );
}

/// (c) A confirmation of 60 after a first of 60 is granted the 30 minutes the episode has left, and
/// its receipt says the cap cut it short.
#[test]
fn a_second_confirm_of_sixty_is_clamped_to_the_thirty_left() {
    let team = team();
    let (thread, session) = a_running_coder_with_own_life_at_t0(&team);
    team.pulse(&session, Pulse::At(at(0)));
    team.tick(&at(600), &thread);
    let first = team.confirm(&at(605), &session, &thread, 60).unwrap();
    assert_eq!((first.granted_minutes, first.clamped), (60, false));
    team.tick(&at(605 + 3_600 + 300), &thread);
    assert_eq!(team.worker.asks().len(), 2);
    assert!(team.worker.asks()[1]
        .text
        .contains("30 minutes of rest remain"));
    let second = team
        .confirm(&at(605 + 3_600 + 310), &session, &thread, 60)
        .unwrap();
    assert_eq!((second.granted_minutes, second.clamped), (30, true));
    assert!(
        second.receipt.contains("30 of the 60 minutes"),
        "{}",
        second.receipt
    );
    assert!(
        second.receipt.contains("capped at 90 min"),
        "{}",
        second.receipt
    );
}

/// (d) No confirmation cancels a stop: one sent while the stop is in flight is refused with the
/// stop's text, and the one further SIGTERM still comes at the stop's grace.
#[test]
fn a_confirm_while_the_stop_is_in_flight_is_refused_and_the_second_sigterm_still_comes() {
    let team = team();
    let (thread, session) = a_running_coder_with_own_life_at_t0(&team);
    team.pulse(&session, Pulse::At(at(0)));
    team.tick(&at(600), &thread);
    team.tick(&at(900), &thread);
    assert_eq!(team.worker.stops().len(), 1, "stopped at +900");
    let refused = team
        .confirm(&at(930), &session, &thread, 20)
        .expect_err("a stop is in flight");
    assert_eq!(refused.kind, nxs_foundation::error::ErrorKind::Validation);
    assert!(
        refused.msg.contains("your session is being stopped"),
        "{}",
        refused.msg
    );
    team.tick(&at(1_199), &thread);
    assert_eq!(team.worker.stops().len(), 1);
    team.tick(&at(1_200), &thread);
    assert_eq!(
        team.worker.stops().len(),
        2,
        "the second SIGTERM at the stop's grace"
    );
}

/// (e) A resumed session is a fresh process: the nudge its predecessor got is not its own, so its
/// confirmation is refused — until it is nudged itself.
#[test]
fn a_confirm_on_a_resumed_session_is_refused_until_it_is_nudged_itself() {
    let team = team();
    let (thread, session) = a_running_coder_with_own_life_at_t0(&team);
    team.pulse(&session, Pulse::At(at(0)));
    team.tick(&at(600), &thread);
    team.tick(&at(900), &thread);
    team.worker.stop(&session);
    team.ended(&at(900), &session, None);
    team.tick(&at(1_020), &thread);
    assert_eq!(team.log(&thread, "coder")[2].0, "continued");
    team.worker.run(&session);
    team.pulse(&session, Pulse::Unreported);
    team.turn(&session, &thread, &at(1_020));
    team.work(&session, &at(1_030));
    let refused = team
        .confirm(&at(1_040), &session, &thread, 20)
        .expect_err("the predecessor's nudge is not this process's");
    assert!(
        refused.msg.contains("nothing is asked of you"),
        "{}",
        refused.msg
    );
    team.tick(&at(1_630), &thread);
    assert_eq!(team.worker.asks().len(), 2, "nudged itself at +1 630");
    let granted = team
        .confirm(&at(1_640), &session, &thread, 20)
        .expect("now it was asked");
    assert_eq!(granted.granted_minutes, 20);
}

/// (f) The verb at the command line: absent from `nxc --help` and `nxc guide`; with no nudge
/// pending it exits 1 and writes nothing; `--rest 0` and `--rest 61` are refused naming the bounds.
#[test]
fn nxc_confirm_is_hidden_and_refuses_with_exit_one_and_no_row() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    let nxc = || {
        let mut c = nxs_test_support::cargo_bin("nxc");
        c.current_dir(team.tmp.path())
            .env_remove("NXC_DB")
            .env("NXC_ACTOR", "coder")
            .env("NXC_SESSION", &session)
            .env("NXC_NOW", at(60))
            .env("NXC_WORKER", "dry")
            .env("NXC_TIMER", "dry")
            .env("NXC_DRY_LOG", team.tmp.path().join("dry.log"));
        c
    };
    let help = nxc().arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(!String::from_utf8_lossy(&help.stdout).contains("confirm"));
    let guide = nxc().arg("guide").output().unwrap();
    assert!(!String::from_utf8_lossy(&guide.stdout).contains("confirm"));
    for (rest, needle) in [
        ("5", "nothing is asked of you"),
        ("0", "--rest takes 1 to 60 minutes"),
        ("61", "--rest takes 1 to 60 minutes"),
    ] {
        let out = nxc()
            .args(["confirm", "--thread", &thread, "--rest", rest])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "--rest {rest}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(needle), "--rest {rest}: {err}");
    }
    assert!(team.log(&thread, "coder").is_empty(), "no row written");
    assert_eq!(
        team.owed(&thread, "coder", "confirmations"),
        Some("0".into())
    );
}

/// (g) Round 3 IR #4: after a rest of one minute, only a `tool_result` arrives — a tool ending is
/// not the model being alive, so the look at the rest's end plus its grace nudges again and writes
/// no `recovered`.
#[test]
fn a_tool_result_after_a_rest_is_not_life_since_the_confirmation() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.turn(&session, &thread, &at(0));
    team.tool_call(&session, "toolu_run", "Bash", &at(100));
    // The host samples the call: nothing runs under it, so silence counts from the call.
    team.sample(&session, 0, 0);
    follow_looks(&team, &thread, "coder", 120, 700);
    assert_eq!(team.worker.asks().len(), 1);
    assert_eq!(team.worker.asks()[0].at, at(700));
    team.confirm(&at(705), &session, &thread, 1).unwrap();
    team.tool_returned(&session, "toolu_run", &at(800));
    team.tick(&at(705 + 60 + 300), &thread);
    let actions: Vec<String> = team
        .log(&thread, "coder")
        .into_iter()
        .map(|(a, _)| a)
        .collect();
    assert_eq!(
        actions,
        ["asked", "confirmed", "asked"],
        "nudged again, not recovered"
    );
}

// ---- test 3, the ladder's half: the subprocess under an open call ----------------------------------

/// A session inside one call: a turn at T0, a `tool_use` at +60 that stays open.
fn a_coder_inside_one_call(team: &Team) -> (String, String) {
    let thread = team.commission(T0, "coder", "Watch the CI run.").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.turn(&session, &thread, &at(0));
    team.tool_call(&session, "toolu_watch", "Bash", &at(60));
    (thread, session)
}

/// Moving samples under an open call keep every look arming for 40 minutes, with no nudge.
#[test]
fn a_moving_subprocess_keeps_the_open_call_alive_for_forty_minutes() {
    let team = team();
    let (thread, session) = a_coder_inside_one_call(&team);
    for (i, s) in (120..=60 + 2_400).step_by(60).enumerate() {
        team.sample(&session, 2, 1_000 * (i as u64 + 1));
        team.tick(&at(s), &thread);
    }
    assert!(team.worker.asks().is_empty(), "{:?}", team.worker.asks());
    assert!(team.log(&thread, "coder").is_empty());
}

/// **The named test that kills a CPU floor** (design 8nx4 test 8, mutation 6b): a subprocess whose
/// counter moves by ONE unit per look — `gh run watch` spends tens of milliseconds a minute — is
/// still progress, and the call is not nudged for 40 minutes.
#[test]
fn a_subprocess_that_works_tens_of_milliseconds_a_minute_keeps_the_call_alive() {
    let team = team();
    let (thread, session) = a_coder_inside_one_call(&team);
    for (i, s) in (120..=60 + 2_400).step_by(60).enumerate() {
        team.sample(&session, 1, 500 + i as u64);
        team.tick(&at(s), &thread);
    }
    assert!(team.worker.asks().is_empty(), "{:?}", team.worker.asks());
}

/// A sample that freezes is nudged at the look at its last movement plus 30 minutes exactly.
#[test]
fn a_frozen_subprocess_is_nudged_at_its_last_movement_plus_thirty_minutes() {
    let team = team();
    let (thread, session) = a_coder_inside_one_call(&team);
    for (i, s) in (120..=2_460).step_by(60).enumerate() {
        if s <= 600 {
            team.sample(&session, 1, 100 * (i as u64 + 1));
        }
        team.tick(&at(s), &thread);
    }
    let asks = team.worker.asks();
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert_eq!(asks[0].at, at(600 + 1_800), "M + 1 800 exactly");
    assert_eq!(
        team.owed(&thread, "coder", "subprocess_moved_at"),
        Some(at(600))
    );
}

/// A call with nothing running under it has no `M`: it is the silence rule's, nudged at its last
/// own life plus ten minutes.
#[test]
fn an_open_call_with_no_subprocess_is_nudged_at_its_last_life_plus_ten_minutes() {
    let team = team();
    let (thread, session) = a_coder_inside_one_call(&team);
    team.sample(&session, 0, 0);
    for s in (120..=720).step_by(60) {
        team.tick(&at(s), &thread);
    }
    let asks = team.worker.asks();
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert_eq!(asks[0].at, at(660), "the call's own instant plus 600");
}

// ---- test 4: the chain -----------------------------------------------------------------------------

/// **A dead verdict whose re-commission keeps failing reaches the escalation after the budget, not
/// at the deadline** (design 8nx4 test 4, on yts5's retried start): a fresh process that never
/// produced anything is dead at its start plus ten minutes, stopped, and — having written nothing —
/// re-commissioned; every start fails, each failure is an attempt, and the third escalates.
#[test]
fn a_dead_verdict_whose_recommission_keeps_failing_escalates_after_the_budget() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.pulse(&session, Pulse::NoneYet);
    team.tick(&at(600), &thread);
    assert_eq!(
        team.worker.stops(),
        vec![session.clone()],
        "dead without a nudge"
    );
    assert!(team.worker.asks().is_empty());
    team.worker.stop(&session);
    team.ended(&at(600), &session, None);
    *team.worker.refuse_every.lock().unwrap() = Some("prime_timeout");
    follow_looks(&team, &thread, "coder", 660, 1_500);
    let log = team.log(&thread, "coder");
    let failed = log
        .iter()
        .filter(|(a, o)| {
            a == "recommissioned" && o.as_deref().is_some_and(|o| o.starts_with("failed:"))
        })
        .count();
    assert_eq!(failed, 3, "{log:?}");
    assert_eq!(
        log.last().map(|(a, _)| a.as_str()),
        Some("escalated"),
        "{log:?}"
    );
    let at_ = team.escalated_at(&thread);
    assert_eq!(at_.len(), 1);
    assert!(
        at_[0] < 1_500,
        "escalated at +{}s, long before the deadline",
        at_[0]
    );
}

// ---- test 5: the episode --------------------------------------------------------------------------

/// **Two episodes, not one** (design 8nx4 test 5): nudged at +10 min, life at +12, the nudge
/// overtaken and the first episode closed by `recovered` at +20; silence from +40 gives a NUDGE at
/// +50 (a second episode), not a stop. A rest of 20 minutes confirmed then is granted until +70;
/// life after it ends the second episode at +75 and resets its confirmations.
#[test]
fn a_recovered_episode_is_closed_and_a_later_silence_opens_a_second() {
    let team = team();
    let (thread, session) = a_running_coder_with_own_life_at_t0(&team);
    team.pulse(&session, Pulse::At(at(0)));
    for s in (60..=3_000).step_by(60) {
        if (720..=2_400).contains(&s) {
            team.pulse(&session, Pulse::At(at(s)));
        }
        team.tick(&at(s), &thread);
    }
    let rows: Vec<(String, Option<String>, i64)> = team.log_at(&thread, "coder");
    assert_eq!(
        rows,
        vec![
            ("asked".into(), Some("overtaken".into()), 600),
            ("recovered".into(), Some("life".into()), 1_200),
            ("asked".into(), Some("asked".into()), 3_000),
        ]
    );
    assert!(team.worker.stops().is_empty(), "a nudge, not a stop");
    assert_eq!(
        *team.worker.withdrawn.lock().unwrap(),
        vec![session.clone()]
    );
    assert_eq!(
        team.owed(&thread, "coder", "recovery_started_at"),
        Some(at(3_000))
    );
    let r = team.confirm(&at(3_000), &session, &thread, 20).unwrap();
    assert_eq!(r.until, at(4_200), "until T0+70, in the future");
    assert_eq!(
        team.owed(&thread, "coder", "confirmations"),
        Some("1".into())
    );
    team.pulse(&session, Pulse::At(at(4_300)));
    team.tick(&at(4_500), &thread);
    let last = team.log_at(&thread, "coder").pop().unwrap();
    assert_eq!(last, ("recovered".into(), Some("life".into()), 4_500));
    assert_eq!(
        team.owed(&thread, "coder", "confirmations"),
        Some("0".into())
    );
    assert_eq!(team.owed(&thread, "coder", "recovery_started_at"), None);
}

/// A session nudged and overtaken again and again over more than two hours is never escalated by
/// the ceiling: each overtaken nudge ends its episode by life, and the ceiling is the episode's.
#[test]
fn a_session_nudged_and_overtaken_for_two_hours_is_never_escalated_by_the_ceiling() {
    let team = team();
    let (thread, session) = a_running_coder_with_own_life_at_t0(&team);
    team.pulse(&session, Pulse::At(at(0)));
    for s in (60..=9_000).step_by(60) {
        if (11..=19).contains(&((s / 60) % 20)) {
            team.pulse(&session, Pulse::At(at(s)));
        }
        team.tick(&at(s), &thread);
    }
    assert!(
        team.worker.asks().len() >= 3,
        "{}",
        team.worker.asks().len()
    );
    assert!(escalations(&team.messages(&thread)).is_empty());
    assert!(team.worker.stops().is_empty());
    let recovered = team
        .log(&thread, "coder")
        .iter()
        .filter(|(a, _)| a == "recovered")
        .count();
    assert!(
        recovered + 1 >= team.worker.asks().len(),
        "every nudge's episode but the last ended by life: {recovered} of {}",
        team.worker.asks().len()
    );
}

// ---- test 9: the B path's other cases ---------------------------------------------------------------

/// The B path up to the continue at +1 020: nudged +600, stopped +900, the end announced at the
/// stop, continued at +1 020; the continuation is marked running with a `NoneYet` pulse and its
/// two bookkeeping entries.
fn continued_once(team: &Team) -> (String, String) {
    let (thread, session) = a_running_coder_with_own_life_at_t0(team);
    team.pulse(&session, Pulse::At(at(0)));
    team.tick(&at(600), &thread);
    team.tick(&at(900), &thread);
    team.worker.stop(&session);
    team.ended(&at(900), &session, None);
    team.tick(&at(960), &thread);
    team.tick(&at(1_020), &thread);
    assert_eq!(team.log(&thread, "coder")[2].0, "continued");
    team.worker.run(&session);
    team.pulse(&session, Pulse::NoneYet);
    team.turn(&session, &thread, &at(1_020));
    team.entry(&session, "session_init", &at(1_021), "{}");
    (thread, session)
}

/// **Recover-then-hang under B** (round 3 TQ #7): the continuation writes one model entry at +1 085,
/// so the first look after the continue (+1 140, the trigger's grace) writes `recovered` with
/// attempt 1; silent from there, it is nudged at +1 685 and found dead at +1 985 — stopped and
/// escalated at that look, with no second `continued`: `attempts` survives the episode's end.
#[test]
fn recover_then_hang_under_b_is_escalated_at_its_next_dead_verdict_without_a_second_resume() {
    let team = team();
    let (thread, session) = continued_once(&team);
    team.entry(&session, "assistant", &at(1_085), "{}");
    follow_looks(&team, &thread, "coder", 1_140, 2_100);
    let rows: Vec<(String, i64)> = team
        .log_at(&thread, "coder")
        .into_iter()
        .map(|(a, _, t)| (a, t))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("asked".into(), 600),
            ("stopped".into(), 900),
            ("continued".into(), 1_020),
            ("recovered".into(), 1_140),
            ("asked".into(), 1_685),
            ("stopped".into(), 1_985),
            ("escalated".into(), 1_985),
        ]
    );
    let attempt: i64 = team
        .store()
        .connection()
        .query_row(
            "SELECT attempt FROM recovery_log WHERE action = 'recovered'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attempt, 1);
}

/// **A second owed turn on the same slot is allowed its own resume** (round 3 CQ #2, the arbiter):
/// the continued session answers, and within the same minute the slot is owed again — before any
/// sweep forgets the row. The reply discharged the first turn, so its budget went with it: silent
/// again, the session is nudged, stopped and RESUMED, not escalated at its first dead verdict.
#[test]
fn a_second_owed_turn_after_a_reply_is_allowed_its_own_resume() {
    let team = team();
    let (thread, session) = continued_once(&team);
    team.work(&session, &at(1_050));
    team.reply(
        &at(1_100),
        "coder",
        &session,
        &thread,
        "done: --since is in",
    );
    assert_eq!(
        team.owed(&thread, "coder", "attempts"),
        Some("0".into()),
        "the turn's budget went"
    );
    // Owed again within the same minute — the expectation re-declared on the slot, the process
    // still running; the new turn's process start moves the clock.
    let mut store = team.store();
    store.set_expects_reply_from(
        &thread,
        &serde_json::to_string(&[team.q("coder")]).unwrap(),
        &team.q("owner"),
    );
    drop(store);
    team.sql(&format!(
        "UPDATE owed_starts SET trigger_clock = '{}' WHERE thread = '{thread}'",
        at(1_110)
    ));
    team.pulse(&session, Pulse::Unreported);
    team.turn(&session, &thread, &at(1_110));
    follow_looks(&team, &thread, "coder", 1_170, 2_010);
    team.worker.stop(&session);
    team.ended(&at(2_010), &session, None);
    follow_looks(&team, &thread, "coder", 2_070, 2_200);
    let actions: Vec<String> = team
        .log(&thread, "coder")
        .into_iter()
        .map(|(a, _)| a)
        .collect();
    assert_eq!(
        actions,
        [
            "asked",
            "stopped",
            "continued",
            "asked",
            "stopped",
            "continued"
        ],
        "the second turn got its own resume"
    );
    assert!(escalations(&team.messages(&thread)).is_empty());
}

/// The coder, at work on `thread`, commissions a reviewer at `sent`, which answers at `answered` —
/// the answer wakes the coder on the way back while its process still runs.
fn a_way_back_wake(team: &Team, thread: &str, session: &str, sent: i64, answered: i64) {
    let child = team
        .session_sends(&at(sent), "coder", session, "reviewer", "check this")
        .thread_id;
    let reviewer = team.worker.last("reviewer").internal_session;
    team.worker.run(&reviewer);
    team.reply(&at(answered), "reviewer", &reviewer, &child, "looks fine");
    let _ = thread;
}

/// **A wake into a live process moves nothing** (round 2 TQ #3): the answer of a child arrives at
/// +960 while the stop of +900 is in flight — the process still runs, so the wake is held behind
/// its turn, the row's stop stands, and the one further SIGTERM comes at +1 200.
#[test]
fn a_wake_with_the_stop_in_flight_leaves_the_stop_and_the_second_sigterm_comes() {
    let team = team();
    let (thread, session) = a_running_coder_with_own_life_at_t0(&team);
    team.pulse(&session, Pulse::At(at(0)));
    team.tick(&at(600), &thread);
    team.tick(&at(900), &thread);
    a_way_back_wake(&team, &thread, &session, 930, 960);
    assert_eq!(
        team.owed(&thread, "coder", "stop_requested_at"),
        Some(at(900))
    );
    assert_eq!(team.owed(&thread, "coder", "trigger_clock"), Some(at(0)));
    team.tick(&at(1_200), &thread);
    assert_eq!(team.worker.stops(), vec![session.clone(), session.clone()]);
}

/// …and a wake at +700 with the nudge of +600 pending leaves the nudge, and the session is stopped
/// at +900.
#[test]
fn a_wake_with_a_nudge_pending_leaves_the_nudge_and_the_session_is_stopped_at_fifteen() {
    let team = team();
    let (thread, session) = a_running_coder_with_own_life_at_t0(&team);
    team.pulse(&session, Pulse::At(at(0)));
    team.tick(&at(600), &thread);
    a_way_back_wake(&team, &thread, &session, 650, 700);
    assert_eq!(team.owed(&thread, "coder", "asked_at"), Some(at(600)));
    team.tick(&at(900), &thread);
    assert_eq!(team.worker.stops(), vec![session.clone()]);
}

/// **A funnel trigger into a live session moves nothing** (round 3 TQ #1): a continue the worker
/// refuses `already_running` started nothing, so the slot's clock, its nudge and its stop stand —
/// and the reverted opener settles its own row with that outcome.
#[test]
fn a_trigger_refused_already_running_leaves_the_clock_the_nudge_and_the_stop() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    team.sql(&format!(
        "UPDATE owed_starts SET asked_at = '{a}', stop_requested_at = '{a}', stop_signals = 1
          WHERE thread = '{thread}'",
        a = at(8)
    ));
    *team.worker.refuse.lock().unwrap() = Some("already_running");
    team.tick(&at(130), &thread);
    assert_eq!(team.owed(&thread, "coder", "trigger_clock"), Some(at(0)));
    assert_eq!(team.owed(&thread, "coder", "asked_at"), Some(at(8)));
    assert_eq!(
        team.owed(&thread, "coder", "stop_requested_at"),
        Some(at(8))
    );
    assert_eq!(
        team.log(&thread, "coder"),
        vec![("continued".to_string(), Some("already_running".to_string()))]
    );
}

/// **A re-commission queued behind the working copy resets nothing** until its released start
/// spawns (round 3 TQ #1): the session of an exclusive role produced nothing of its own, was nudged
/// and stopped; having written nothing it is re-commissioned — and the lease is another
/// operation's, so the start waits in the queue. Nothing started, and the slot's clock, nudge and
/// stop stand. (A CONTINUE never queues: it travels back up its own chain, `ChainMove::Unwind`,
/// which never waits for the working copy unless that operation's work is parked.)
#[test]
fn a_recommission_queued_behind_the_working_copy_resets_nothing() {
    let team = team();
    let thread = team.commission(T0, "builder", "build").thread_id;
    let session = team.worker.last("builder").internal_session;
    team.worker.run(&session);
    team.turn(&session, &thread, &at(0));
    team.tick(&at(600), &thread);
    team.tick(&at(900), &thread);
    assert_eq!(team.worker.stops(), vec![session.clone()]);
    team.worker.stop(&session);
    team.ended(&at(900), &session, None);
    team.sql(&format!(
        "UPDATE working_tree_lease SET scope_key = 'thread:another-operation', expires = '{}'",
        at(86_400)
    ));
    team.tick(&at(1_020), &thread);
    assert_eq!(
        team.log(&thread, "builder").last().cloned(),
        Some(("recommissioned".to_string(), Some("queued".to_string())))
    );
    assert_eq!(team.worker.of("builder").len(), 1, "nothing started");
    assert_eq!(team.owed(&thread, "builder", "trigger_clock"), Some(at(0)));
    assert_eq!(team.owed(&thread, "builder", "asked_at"), Some(at(600)));
    assert_eq!(
        team.owed(&thread, "builder", "stop_requested_at"),
        Some(at(900))
    );
}

/// **The round-3 contradiction case** (TQ #2 / IR #1): a pulse from before the process started is
/// the generous side — the session is nudged at its start plus ten minutes, not stopped, and
/// stopped five minutes later.
#[test]
fn a_pulse_older_than_the_process_start_is_nudged_not_stopped() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.pulse(&session, Pulse::At(at(-60)));
    team.tick(&at(600), &thread);
    team.tick(&at(900), &thread);
    let rows: Vec<(String, i64)> = team
        .log_at(&thread, "coder")
        .into_iter()
        .map(|(a, _, t)| (a, t))
        .collect();
    assert_eq!(rows, vec![("asked".into(), 600), ("stopped".into(), 900)]);
}

/// **A `cannot_ask` row opens no episode** (round 3 CQ #4 / IR #5): the door is closed, the row
/// says so once, and the episode's clock stays unset — until the door is still closed at the due
/// plus the nudge's wait, when the session is dead and the stop opens it.
#[test]
fn a_cannot_ask_row_opens_no_episode_and_a_door_closed_past_the_wait_is_dead() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.worker.no_door.lock().unwrap().insert(session.clone());
    for s in (600..=840).step_by(60) {
        team.tick(&at(s), &thread);
    }
    assert_eq!(
        team.log(&thread, "coder"),
        vec![("asked".to_string(), Some("cannot_ask".to_string()))],
        "one row per closed door"
    );
    assert_eq!(team.owed(&thread, "coder", "recovery_started_at"), None);
    team.tick(&at(900), &thread);
    assert_eq!(team.worker.stops(), vec![session.clone()]);
    assert_eq!(
        team.owed(&thread, "coder", "recovery_started_at"),
        Some(at(900))
    );
}

/// **A session whose model streams is alive** (design 8nx4 section 1, the pulse): a thinking block
/// that runs twenty minutes has no transcript entry until it closes, but the sidecar's pulse moves
/// with the model's output — so the session is not nudged. (Mutation 1 of design test 8: drop the
/// pulse read and this session is nudged at +600.)
#[test]
fn a_session_streaming_for_twenty_minutes_is_not_nudged() {
    let team = team();
    let (thread, session) = a_running_coder_with_own_life_at_t0(&team);
    for s in (60..=1_200).step_by(60) {
        team.pulse(&session, Pulse::At(at(s - 30)));
        team.tick(&at(s), &thread);
    }
    assert!(team.worker.asks().is_empty(), "{:?}", team.worker.asks());
    assert!(team.log(&thread, "coder").is_empty());
}

/// **A reply withdraws the nudge it answers** (design 8nx4 section 3; round 2 IR #3): a session
/// nudged at +600 that replies at +630 has its ask file withdrawn at the reply, so it is not handed
/// "no sign of life from you" at its turn's end — and the reply is admitted either way. A reply on
/// a slot nobody nudged withdraws nothing: another slot's nudge of the same session stands.
#[test]
fn a_reply_withdraws_the_nudge_it_answers_and_only_a_nudged_slots() {
    let team = team();
    let (thread, session) = a_running_coder_with_own_life_at_t0(&team);
    team.pulse(&session, Pulse::At(at(0)));
    team.tick(&at(600), &thread);
    assert_eq!(team.worker.asks().len(), 1);
    team.reply(&at(630), "coder", &session, &thread, "done: --since is in");
    assert_eq!(
        *team.worker.withdrawn.lock().unwrap(),
        vec![session.clone()]
    );
    assert_eq!(
        team.owed(&thread, "coder", "asked_at"),
        None,
        "the turn is over"
    );

    let other = team.commission(&at(700), "reviewer", "check it").thread_id;
    let reviewer = team.worker.last("reviewer").internal_session;
    team.reply(&at(710), "reviewer", &reviewer, &other, "fine");
    assert_eq!(
        *team.worker.withdrawn.lock().unwrap(),
        vec![session.clone()],
        "nothing was nudged on that slot, so nothing is withdrawn"
    );
}

/// **A sample belongs to its call** (review of pass 1, Integrity #1): call 1 ran a child that kept
/// moving and returned; call 2 runs nothing under it. Call 2 inherits no `M` from call 1 — it is
/// the silence rule's and is nudged at its own instant plus ten minutes.
#[test]
fn a_call_after_a_call_with_children_inherits_no_subprocess_movement() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.turn(&session, &thread, &at(0));
    team.tool_call(&session, "toolu_one", "Bash", &at(60));
    for (i, s) in (120..=300).step_by(60).enumerate() {
        team.sample(&session, 2, 100 * (i as u64 + 1));
        team.tick(&at(s), &thread);
    }
    team.tool_returned(&session, "toolu_one", &at(310));
    team.tool_call(&session, "toolu_two", "mcp__thing", &at(320));
    team.sample(&session, 0, 0);
    for s in (360..=960).step_by(60) {
        team.tick(&at(s), &thread);
    }
    let asks = team.worker.asks();
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert_eq!(
        asks[0].at,
        at(960),
        "the look after call 2's own instant plus 600 (920)"
    );
}

/// **One re-commission with its retries is ONE attempt and ONE `recommissioned` row** (round 2 TQ
/// #6, a contract on yts5; review of pass 1, CQ #1 / TQ #1): a start that failed is retried at the
/// next look on the SAME rung — its row is settled again and its attempt is not counted twice.
#[test]
fn one_recommission_with_its_retries_is_one_attempt_and_one_row() {
    let team = team();
    let thread = a_slot_due_for_a_recommission(&team);
    *team.worker.refuse.lock().unwrap() = Some("prime_timeout");
    team.tick(&at(130), &thread);
    team.tick(&at(190), &thread);
    assert_eq!(team.worker.of("coder").len(), 2, "started at the retry");
    assert_eq!(
        team.log(&thread, "coder"),
        vec![("recommissioned".to_string(), Some("triggered".to_string()))]
    );
    assert_eq!(team.ladder(&thread, "coder").unwrap().attempts, 1);
    let retries: i64 = team
        .store()
        .connection()
        .query_row("SELECT retries FROM recovery_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(retries, 1);
}
