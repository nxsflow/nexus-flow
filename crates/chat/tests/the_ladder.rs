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

use std::collections::HashSet;
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
    AskOutcome, SessionAsk, SessionStop, TriggerError, TriggerOutcome, TriggerRequest,
    TriggerResult, Worker,
};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

// ---- the clock ------------------------------------------------------------------------------------

const T0: &str = "2026-10-24T10:00:00Z";

fn at(secs: i64) -> String {
    use time::format_description::well_known::Rfc3339;
    let t = time::OffsetDateTime::parse(T0, &Rfc3339).unwrap() + time::Duration::seconds(secs);
    t.format(&Rfc3339).unwrap()
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
    /// Every session somebody asked this worker to stop.
    stopped: Mutex<Vec<String>>,
    /// Whether its runtime CANNOT continue a conversation it began.
    no_resume: Mutex<bool>,
    /// When set, the next start blocks inside the worker until the test releases it — a trigger
    /// still running while another look arrives.
    gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    /// Set while a start is blocked at the gate.
    at_the_gate: std::sync::atomic::AtomicBool,
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
}

impl Worker for Hand {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            self.at_the_gate
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = gate.recv();
        }
        if let Some(how) = self.refuse.lock().unwrap().take() {
            return Err(match how {
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
        self.running.lock().unwrap().contains(internal_session)
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
}

// ---- the timer ------------------------------------------------------------------------------------

#[derive(Default)]
struct Book {
    looks: Mutex<Vec<(String, String)>>,
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
/// A coding chain whose second step is the review round — a nested round.
const CODING: &str = "name: coding\nmembers: [coder, review]\nflow: sequential\ntimeout: 2h\n";

struct Team {
    tmp: TempDir,
    worker: Arc<Hand>,
    book: Arc<Book>,
    origin: String,
}

fn role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

fn team() -> Team {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let channels: Vec<ChannelDecl> = [QUICK, REVIEW, CODING]
        .iter()
        .map(|c| serde_yaml::from_str(c).expect("channel parses"))
        .collect();
    let defs = Definitions::new(
        ["pm", "coder", "verifier", "reviewer", "finisher"]
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
    last_progress_at: Option<String>,
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

    /// `role`'s session says "still working" on `thread` — `nxc reply --progress`.
    fn progress(
        &self,
        now: &str,
        role: &str,
        session: &str,
        thread: &str,
        body: &str,
    ) -> surface::ProgressReceipt {
        self.with(now, role, Some(session), |ctx, store| {
            surface::reply_progress(ctx, store, thread, body)
        })
        .expect("the progress is posted")
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
                "SELECT session, attempts, recovery_started_at, asked_at, last_progress_at,
                        recovering_until
                   FROM owed_starts WHERE thread = ?1 AND role = ?2",
                [thread, &self.q(handle)],
                |r| {
                    Ok(Ladder {
                        session: r.get(0)?,
                        attempts: r.get(1)?,
                        recovery_started_at: r.get(2)?,
                        asked_at: r.get(3)?,
                        last_progress_at: r.get(4)?,
                        recovering_until: r.get(5)?,
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
/// and the text offers no `--progress`. One attempt is counted and logged.
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
    assert!(!cont.message.contains("--progress"), "{}", cont.message);
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

/// A live session with no activity on its slot for 20 minutes is ASKED, through the door, with the
/// one text that offers `--progress` — once. Nothing is stopped and nothing is started. Before the
/// 20 minutes, nothing at all.
#[test]
fn a_live_session_silent_on_its_slot_for_twenty_minutes_is_asked_once_with_the_progress_text() {
    let team = team();
    let thread = team
        .commission(T0, "coder", "Add a --since flag.")
        .thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.tick(&at(1_199), &thread);
    assert!(team.worker.asks().is_empty(), "not yet");
    team.tick(&at(1_200), &thread);
    let asks = team.worker.asks();
    assert_eq!(asks.len(), 1, "{asks:?}");
    let ask = &asks[0];
    assert_eq!(ask.session, session);
    assert_eq!(ask.thread, thread);
    assert_eq!(ask.role, team.q("coder"));
    assert_eq!(ask.text, nexus_chat::ladder::ask_text(&thread));
    assert!(
        ask.text
            .contains(&format!("nxc reply --thread {thread} --progress"))
            && ask.text.contains("20 minutes"),
        "{}",
        ask.text
    );
    let ladder = team.ladder(&thread, "coder").unwrap();
    assert_eq!(ladder.attempts, 1);
    assert_eq!(ladder.asked_at.as_deref(), Some(at(1_200).as_str()));
    assert_eq!(
        team.log(&thread, "coder"),
        vec![("asked".to_string(), Some("asked".to_string()))]
    );
    assert_eq!(team.worker.of("coder").len(), 1, "nothing started");
    assert!(
        team.worker.stopped.lock().unwrap().is_empty(),
        "nothing stopped"
    );
    // The next look before another 20 minutes asks nothing more.
    team.tick(&at(1_200 + 600), &thread);
    assert_eq!(team.worker.asks().len(), 1);
    // The status row says so, never "stale".
    let row = team.status_of(&at(1_201), &thread);
    let recovery = row.recovery.expect("under recovery");
    assert_eq!(recovery.action, nexus_chat::ladder::RecoveryAction::Asked);
    assert_eq!((recovery.attempt, recovery.of), (1, 3));
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

// ---- `--progress` ---------------------------------------------------------------------------------

/// **A requested `--progress`** resets the budget and the silence clock and answers the ask — and
/// discharges nothing, starts nothing, wakes nobody. The later real answer is accepted.
#[test]
fn a_requested_progress_resets_attempts_and_discharges_nothing() {
    let team = team();
    let thread = team
        .commission(T0, "coder", "Add a --since flag.")
        .thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.tick(&at(1_200), &thread);
    assert_eq!(team.ladder(&thread, "coder").unwrap().attempts, 1);
    let starts = team.worker.all().len();

    let r = team.progress(&at(1_300), "coder", &session, &thread, "running the suite");
    assert!(r.counted, "{r:?}");
    assert_eq!(r.receipt, surface::PROGRESS_TAKEN);
    let ladder = team.ladder(&thread, "coder").unwrap();
    assert_eq!(ladder.attempts, 0, "the budget is back");
    assert_eq!(ladder.asked_at, None, "the ask is answered");
    assert_eq!(ladder.last_progress_at.as_deref(), Some(at(1_300).as_str()));
    assert_eq!(
        ladder.recovery_started_at.as_deref(),
        Some(at(1_200).as_str()),
        "the ceiling clock stands"
    );
    let row = team.status_of(&at(1_301), &thread);
    assert_eq!(row.outstanding, vec![team.q("coder")], "nothing discharged");
    assert!(!row.awaiting_human, "nobody woken: the root has no answer");
    assert_eq!(team.worker.all().len(), starts, "nothing started");
    let kinds: Vec<String> = team
        .messages(&thread)
        .into_iter()
        .map(|(_, k, _)| k)
        .collect();
    assert_eq!(
        kinds,
        vec!["info", "progress"],
        "the progress is on the record"
    );

    // Silent again from the progress on: asked again at +20 min from it, not before.
    team.tick(&at(1_300 + 1_199), &thread);
    assert_eq!(team.worker.asks().len(), 1);
    team.tick(&at(1_300 + 1_200), &thread);
    assert_eq!(team.worker.asks().len(), 2);

    team.reply(
        &at(3_000),
        "coder",
        &session,
        &thread,
        "done: --since is in",
    );
    assert!(
        team.status_of(&at(3_001), &thread).awaiting_human,
        "the real answer is accepted"
    );
}

/// **An unrequested `--progress`** is stored and counts for nothing.
#[test]
fn an_unrequested_progress_is_stored_and_counts_nothing() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    team.ended(&at(10), &session, None);
    team.tick(&at(130), &thread);
    assert_eq!(
        team.ladder(&thread, "coder").unwrap().attempts,
        1,
        "continued once"
    );
    let r = team.progress(&at(200), "coder", &session, &thread, "still on it");
    assert!(!r.counted);
    assert_eq!(r.receipt, surface::PROGRESS_UNREQUESTED);
    let ladder = team.ladder(&thread, "coder").unwrap();
    assert_eq!(ladder.attempts, 1, "unchanged");
    assert_eq!(ladder.last_progress_at, None);
    assert_eq!(
        team.status_of(&at(201), &thread).outstanding,
        vec![team.q("coder")]
    );
}

/// A real answer mistakenly sent as `--progress` discharges nothing; the silence clock asks again
/// later, and the proper reply is accepted.
#[test]
fn a_real_answer_sent_as_progress_is_asked_for_again_and_the_proper_reply_accepted() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    team.tick(&at(1_200), &thread);
    team.progress(&at(1_210), "coder", &session, &thread, "done: it is built");
    assert!(!team.status_of(&at(1_211), &thread).awaiting_human);
    team.tick(&at(1_210 + 1_200), &thread);
    assert_eq!(team.worker.asks().len(), 2, "asked again");
    team.reply(&at(2_500), "coder", &session, &thread, "done: it is built");
    assert!(team.status_of(&at(2_501), &thread).awaiting_human);
}

// ---- the budget and the ceiling -------------------------------------------------------------------

/// **The ceiling** (must-fix, design review round 3 TQ #3 / IR #4, round 4 TQ #4): a session that
/// answers EVERY ask with `--progress` is never escalated for being slow — until the wall clock
/// from the episode's first attempt reaches `clamp(3 × window, 2 h, 6 h)`. A direct commission
/// declares no member window, so the floor holds: two hours.
#[test]
fn a_worker_answering_every_ask_with_progress_reaches_rung_e_at_the_ceiling() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.worker.run(&session);
    let first_ask = 1_200;
    let mut now = first_ask;
    let mut asks = 0;
    while now < first_ask + 7_200 {
        team.tick(&at(now), &thread);
        if team.worker.asks().len() > asks {
            asks = team.worker.asks().len();
            team.progress(&at(now + 5), "coder", &session, &thread, "still at it");
        }
        assert!(
            escalations(&team.messages(&thread)).is_empty(),
            "at +{now}s"
        );
        now += 300;
    }
    assert!(asks >= 5, "asked {asks} times, each answered");
    assert_eq!(team.ladder(&thread, "coder").unwrap().attempts, 0);
    team.tick(&at(first_ask + 7_200), &thread);
    let posted = escalations(&team.messages(&thread));
    assert_eq!(posted.len(), 1, "rung E at the ceiling");
    assert!(posted[0].contains("ceiling"), "{}", posted[0]);
}

/// The continue text offers no `--progress`, so a session that answers every CONTINUATION with one
/// is not reset — the progress is unrequested — and it is escalated after three.
#[test]
fn a_worker_answering_every_continuation_with_progress_is_escalated_after_three() {
    let team = team();
    let thread = team.commission(T0, "coder", "build").thread_id;
    let session = team.worker.last("coder").internal_session;
    team.bound_with_transcript(&session, 1, &at(5));
    let mut now = 10;
    for _ in 0..3 {
        team.ended(&at(now), &session, None);
        now += 120;
        team.tick(&at(now), &thread);
        let r = team.progress(&at(now + 5), "coder", &session, &thread, "working");
        assert!(!r.counted);
        now += 60;
    }
    team.ended(&at(now), &session, None);
    team.tick(&at(now + 120), &thread);
    assert_eq!(escalations(&team.messages(&thread)).len(), 1);
    assert_eq!(
        team.worker.of("coder").len(),
        4,
        "the first start and three continuations"
    );
}

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
    team.tick(&at(now + 1_199), &thread);
    assert!(team.worker.asks().is_empty(), "not at once");
    team.tick(&at(now + 1_200), &thread);
    assert_eq!(
        team.worker.asks().len(),
        1,
        "a fresh ask after a fresh silence"
    );
    assert_eq!(team.ladder(&thread, "coder").unwrap().attempts, 1);
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

/// **Two looks racing** — on two connections, the second arriving while the first one's trigger is
/// still running inside the worker: exactly one trigger. Measured by mutation: with the claim AND
/// the in-flight recheck both disabled this stays green, because the first look's trigger has moved
/// the slot's clock and reopened its session before it reaches the worker — so this pins the
/// end-to-end outcome, and `a_held_claim_fires_nothing_and_one_past_it_fires_exactly_once` and
/// `a_rung_in_flight_past_the_claim_blocks_a_second_rung_until_the_trigger_timeout` pin the two
/// guards one by one.
#[test]
fn two_looks_racing_on_one_slot_fire_exactly_one_trigger() {
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
