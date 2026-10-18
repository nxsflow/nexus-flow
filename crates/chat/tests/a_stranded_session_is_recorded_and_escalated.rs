//! **A member session that will not answer is RECORDED as stranded, and the round escalates to its
//! commissioner** (nxf 6j6v.ccvy, cut 1a of epic 6j6v.nmc9).
//!
//! Field evidence, read rather than repeated here: 6j6v.sv38 (a review member whose session never
//! starts — the pb9y verifier of 2026-10-17 is the same shape), 6j6v.cn4p (a resumed session that
//! re-hits the limit and is booked as a clean end), 6j6v.0990 (a review round consolidated while two
//! reviewers sat on a hold), 6j6v.5dzm (a quota-held step skipped at its window). One cause under
//! all of them: a run's liveness is derived from several independently written facts and nothing
//! reconciled them, so a session that never answered was advanced past or waited on forever.
//!
//! Driven through the COMPUTE LAYER — the same functions the CLI and the background service run —
//! with a custom worker that records every start and answers which sessions still run, and a
//! RECORDING TIMER, because the armed instants are part of what is asserted. The tick is the
//! service's verb; the look it runs is a tick.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::error::{NxfError, Result as NxfResult};
use nexus_chat::facade::{self, StatusReport, StatusScope, StatusThread, StrandedEscalation};
use nexus_chat::orchestration::{
    self, ConsequenceClass, FailedConsequence, TickReceipt, TickRequest, STRANDED_GRACE_SECS,
};
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{self, ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{Timer, TimerHandle};
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

// ---- the clock --------------------------------------------------------------------------------

const T0: &str = "2026-10-18T10:00:00Z";

/// The `pm` persona's own session, for the rounds a persona commissions.
const PM_SESSION: &str = "s-pm";

/// `T0` plus `secs` seconds, in the canonical form every stored instant uses.
fn at(secs: i64) -> String {
    at_from(T0, secs)
}

fn at_from(base: &str, secs: i64) -> String {
    use time::format_description::well_known::Rfc3339;
    let t = time::OffsetDateTime::parse(base, &Rfc3339).unwrap() + time::Duration::seconds(secs);
    t.format(&Rfc3339).unwrap()
}

// ---- the worker ---------------------------------------------------------------------------------

/// Records every start it accepted, answers which sessions still run from a set the test controls,
/// and says it LOOKS (`answers_liveness`) unless told otherwise.
#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
    blind: Mutex<bool>,
}

impl Recorder {
    fn starts(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.role.handle.clone())
            .collect()
    }

    fn started(&self, handle: &str) -> usize {
        self.starts().iter().filter(|h| *h == handle).count()
    }

    fn last_for(&self, handle: &str) -> TriggerRequest {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|r| r.role.handle == handle)
            .unwrap_or_else(|| panic!("no trigger for {handle}: {:?}", self.starts()))
            .clone()
    }

    fn mark_running(&self, session: &str) {
        self.running.lock().unwrap().insert(session.to_string());
    }

    fn mark_gone(&self, session: &str) {
        self.running.lock().unwrap().remove(session);
    }

    fn go_blind(&self) {
        *self.blind.lock().unwrap() = true;
    }
}

impl Worker for Recorder {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        // A live session refuses a second process, as the sidecar's single-process guard does —
        // which is what makes the engine HOLD an answer for a caller that is still mid-turn.
        if self.session_is_running(&req.internal_session) {
            return Err(nexus_chat::worker::TriggerError::AlreadyRunning {
                session: req.internal_session.clone(),
                pid: 4242,
            });
        }
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }

    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running.lock().unwrap().contains(internal_session)
    }

    fn answers_liveness(&self) -> bool {
        !*self.blind.lock().unwrap()
    }
}

// ---- the timer ------------------------------------------------------------------------------------

/// Records what it was asked to arm: the channel windows (`schedule`) and the stranded looks, each
/// with its instant — and can be told to refuse the looks.
#[derive(Default)]
struct Book {
    windows: Mutex<Vec<(String, String)>>,
    looks: Mutex<Vec<Look>>,
    resumes: Mutex<Vec<String>>,
    refuse_looks: Mutex<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Look {
    session: String,
    thread: String,
    due: String,
}

impl Book {
    fn looks_for(&self, session: &str) -> Vec<Look> {
        self.looks
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.session == session)
            .cloned()
            .collect()
    }

    /// The look that stands for `session` — the last one armed, as the service book keeps one per
    /// session.
    fn standing_look(&self, session: &str) -> Option<Look> {
        self.looks_for(session).last().cloned()
    }

    fn refuse_looks(&self, refuse: bool) {
        *self.refuse_looks.lock().unwrap() = refuse;
    }
}

impl Timer for Book {
    fn schedule(&self, thread_id: &str, deadline: &str, _command: &str) -> NxfResult<TimerHandle> {
        self.windows
            .lock()
            .unwrap()
            .push((thread_id.to_string(), deadline.to_string()));
        Ok(TimerHandle(format!("window:{thread_id}")))
    }

    fn schedule_resume(&self, session: &str, _deadline: &str) -> NxfResult<TimerHandle> {
        self.resumes.lock().unwrap().push(session.to_string());
        Ok(TimerHandle(format!("resume:{session}")))
    }

    fn schedule_delivery(&self, session: &str, _deadline: &str) -> NxfResult<TimerHandle> {
        Ok(TimerHandle(format!("deliver:{session}")))
    }

    fn schedule_stranded_look(
        &self,
        session: &str,
        thread: &str,
        deadline: &str,
    ) -> NxfResult<TimerHandle> {
        if *self.refuse_looks.lock().unwrap() {
            return Err(NxfError::io("the look book is not writable (injected)"));
        }
        self.looks.lock().unwrap().push(Look {
            session: session.to_string(),
            thread: thread.to_string(),
            due: deadline.to_string(),
        });
        Ok(TimerHandle(format!("look:{session}")))
    }

    fn cancel(&self, _handle: &TimerHandle) -> NxfResult<()> {
        Ok(())
    }
}

// ---- the team -------------------------------------------------------------------------------------

struct Team {
    tmp: TempDir,
    worker: Arc<Recorder>,
    book: Arc<Book>,
    origin: String,
}

fn role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

/// One review step, two hours long — long enough that only the stranded rule can notice a member
/// that never started.
const REVIEW: &str = "name: review\nmembers: [verifier]\nflow: sequential\ntimeout: 2h\n";

fn team(channels: &[&str]) -> Team {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let channels: Vec<ChannelDecl> = channels
        .iter()
        .map(|c| serde_yaml::from_str(c).expect("channel parses"))
        .collect();
    let defs = Definitions::new(
        ["pm", "coder", "verifier", "reviewer", "judge", "finisher"]
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
        worker: Arc::new(Recorder::default()),
        book: Arc::new(Book::default()),
        origin,
    }
}

impl Team {
    fn store(&self) -> ChatStore {
        Workspace::resolve(None, self.tmp.path())
            .expect("resolve workspace")
            .open_chat_store()
            .expect("open chat store")
    }

    /// Run `f` under a context of this workspace, as `actor` (and `session`, when one is named).
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

    /// A human commissions `to` (a channel or a persona) at `now`.
    fn commission(&self, now: &str, to: &str, body: &str) -> surface::SendToReceipt {
        self.with(now, "pm", None, |ctx, store| {
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

    /// A human named `actor` commissions `to` at `now`.
    fn commission_by(
        &self,
        now: &str,
        actor: &str,
        to: &str,
        body: &str,
    ) -> surface::SendToReceipt {
        self.with(now, actor, None, |ctx, store| {
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

    /// The `pm` PERSONA's session commissions `to` at `now` — so the round has a commissioner the
    /// escalation can wake, and the worker's log shows what it was woken with.
    fn commission_as_pm(&self, now: &str, to: &str, body: &str) -> surface::SendToReceipt {
        let mut store = self.store();
        if store.session_role(PM_SESSION).unwrap().is_none() {
            store.create_pending_session(PM_SESSION, "pm").unwrap();
        }
        drop(store);
        self.with(now, "pm", Some(PM_SESSION), |ctx, store| {
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

    /// `session` (of `role`) answers `thread` at `now`.
    fn answer(&self, now: &str, role: &str, session: &str, thread: &str, body: &str) {
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
        .expect("the answer is posted");
    }

    /// The tick the background service runs (and the look runs), at `now`.
    fn tick(&self, now: &str, thread: &str) -> TickReceipt {
        self.with(now, "service", None, |ctx, store| {
            orchestration::tick(ctx, store, TickRequest { thread_id: thread })
        })
        .expect("tick")
    }

    fn status(&self, now: &str) -> StatusReport {
        facade::status(
            &self.store(),
            self.worker.as_ref(),
            now,
            StatusScope::All(None),
        )
        .expect("status")
    }

    fn status_of(&self, now: &str, thread: &str) -> StatusThread {
        self.status(now)
            .operations
            .into_iter()
            .flat_map(|op| op.threads)
            .find(|t| t.thread_id == thread)
            .unwrap_or_else(|| panic!("no status row for {thread}"))
    }

    fn needs_decision(&self, now: &str, root: &str) -> bool {
        self.status(now)
            .operations
            .into_iter()
            .find(|op| op.root == root)
            .unwrap_or_else(|| panic!("no operation {root}"))
            .needs_decision
    }

    fn qualified(&self, handle: &str) -> String {
        format!("{}/{handle}", self.origin)
    }

    /// `(sender, kind, substituted)` of every message in `thread`, in order — the message list
    /// asserted in full.
    fn messages(&self, thread: &str) -> Vec<(String, String, bool)> {
        self.store()
            .acting_messages_in_thread(thread)
            .expect("messages")
            .into_iter()
            .map(|m| {
                let substituted = m
                    .refs
                    .as_deref()
                    .and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok())
                    .and_then(|v| v.get("substituted").and_then(|s| s.as_bool()))
                    .unwrap_or(false);
                let sender = m
                    .sender
                    .rsplit_once('/')
                    .map_or(m.sender.clone(), |(_, b)| b.to_string());
                (sender, m.kind, substituted)
            })
            .collect()
    }

    /// One row of a device-local table, read the way a `sqlite3` session would.
    fn owed(&self, thread: &str, role: &str) -> Option<(String, String, bool)> {
        self.store()
            .connection()
            .query_row(
                "SELECT session, trigger_clock, triggered FROM owed_starts
                  WHERE thread = ?1 AND role = ?2",
                [thread, &self.qualified(role)],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)),
            )
            .ok()
    }

    fn suspect(&self, thread: &str) -> Option<(String, String)> {
        self.store()
            .connection()
            .query_row(
                "SELECT session, first_seen FROM stranded_suspects WHERE thread = ?1",
                [thread],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
    }

    /// `(thread, session, observed, escalation)` of every stranded record, oldest first.
    fn records(&self) -> Vec<(String, String, i64, Option<String>)> {
        let store = self.store();
        let mut st = store
            .connection()
            .prepare(
                "SELECT thread, session, observed, escalation FROM stranded_sessions
                  ORDER BY recorded_at, thread",
            )
            .unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn sql(&self, statement: &str) {
        self.store()
            .connection()
            .execute_batch(statement)
            .expect("the statement runs");
    }
}

fn classes(findings: &[FailedConsequence]) -> Vec<ConsequenceClass> {
    findings.iter().map(|f| f.class).collect()
}

fn has(findings: &[FailedConsequence], class: ConsequenceClass) -> bool {
    findings.iter().any(|f| f.class == class)
}

/// The slot a channel opened for its first member: the thread the worker was told to answer.
fn slot_of(req: &TriggerRequest) -> String {
    req.reply_thread.clone().expect("the member owes a thread")
}

// ---- AC 1: a member whose session never starts --------------------------------------------------

/// **AC 1 (a)** — the pb9y verifier's shape: the worker accepts the start, and nothing ever runs.
/// The two-hour window would have kept the round open for two hours and then advanced past it; the
/// stranded rule records it at the clock plus three minutes, escalates the round to whoever asked,
/// and starts NOTHING.
#[test]
fn a_member_that_never_starts_is_recorded_and_the_round_escalates_to_its_commissioner() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify the build").thread_id;
    let verifier = team.worker.last_for("verifier");
    let (s, slot) = (verifier.internal_session.clone(), slot_of(&verifier));

    // The obligation, from the trigger funnel, with the trigger's own instant.
    assert_eq!(team.owed(&slot, "verifier"), Some((s.clone(), at(0), true)));
    // The look, under its own key, on the CHANNEL thread — never the slot, whose own tick would
    // consolidate it.
    assert_eq!(
        team.book.standing_look(&s),
        Some(Look {
            session: s.clone(),
            thread: root.clone(),
            due: at(STRANDED_GRACE_SECS),
        })
    );

    // +30 s: nothing to judge yet.
    let r = team.tick(&at(30), &root);
    assert_eq!(team.suspect(&slot), None);
    assert!(team.records().is_empty(), "{:?}", classes(&r.warnings));

    // +120 s: the first look. A suspect, no record, and the second look one cadence out.
    let r = team.tick(&at(120), &root);
    assert_eq!(team.suspect(&slot), Some((s.clone(), at(120))));
    assert!(team.records().is_empty());
    assert!(!has(&r.warnings, ConsequenceClass::SessionStranded));
    assert_eq!(team.book.standing_look(&s).unwrap().due, at(180));

    // +180 s: the second look on unchanged evidence records it — observation 2, "never bound".
    let r = team.tick(&at(180), &root);
    assert!(
        has(&r.warnings, ConsequenceClass::SessionStranded),
        "{:?}",
        classes(&r.warnings)
    );
    assert_eq!(
        team.records(),
        vec![(slot.clone(), s.clone(), 2, Some("posted".to_string()))]
    );
    assert_eq!(team.suspect(&slot), None);

    // The slot is escalated, in the member's name, by the runtime; the round reads NEEDS DECISION
    // and nothing after it was opened.
    let row = team.status_of(&at(180), &slot);
    assert!(row.escalated, "the slot carries the escalation");
    assert!(row.substituted, "the runtime spoke for the member");
    let stranded = row.stranded.expect("the slot shows its record");
    assert_eq!(stranded.observed, facade::Observation::NeverBound);
    assert_eq!(stranded.session, s);
    assert_eq!(stranded.escalation, StrandedEscalation::Posted);
    assert!(stranded.escalated);
    assert!(team.needs_decision(&at(180), &root));
    assert_eq!(
        team.worker.starts(),
        vec!["verifier"],
        "exactly the round's own start — the sweep starts nothing"
    );
    assert!(
        team.book.resumes.lock().unwrap().is_empty(),
        "AC 6 (a): the sweep never arms a resume"
    );
    assert_eq!(
        team.messages(&slot),
        vec![
            ("__channel__".to_string(), "info".to_string(), false),
            ("verifier".to_string(), "escalation".to_string(), true),
        ]
    );
    // The escalation rides the round's own consolidation up: the pass-through marker the engine
    // writes for an escalating set, whatever the channel declared.
    assert_eq!(
        team.messages(&root),
        vec![
            ("pm".to_string(), "info".to_string(), false),
            ("__delivered__".to_string(), "escalation".to_string(), false),
        ]
    );

    // +300 s: nothing more — and the record is still the slot's, after the obligation it was judged
    // against is gone (build item 1).
    let before = (team.messages(&slot), team.messages(&root));
    team.tick(&at(300), &root);
    assert_eq!((team.messages(&slot), team.messages(&root)), before);
    assert_eq!(team.worker.starts(), vec!["verifier"]);
    assert_eq!(
        team.owed(&slot, "verifier"),
        None,
        "retention took the owed row"
    );
    assert_eq!(
        team.status_of(&at(300), &slot)
            .stranded
            .map(|s| s.escalation),
        Some(StrandedEscalation::Posted),
        "the STRANDED record outlives the owed row"
    );
}

// ---- more channels ----------------------------------------------------------------------------

/// Two ordered steps with a two-minute window — the shape where the window and the stranded rule
/// meet on the same tick.
const REVIEW_2M: &str =
    "name: review2\nmembers: [verifier, finisher]\nflow: sequential\ntimeout: 2m\n";

/// Two reviewers at once, two hours long.
const PANEL: &str = "name: panel\nmembers: [verifier, reviewer]\nflow: parallel\ntimeout: 2h\n";

impl Team {
    /// The session behind the last start of `handle`, and the thread it owes.
    fn last(&self, handle: &str) -> (String, String) {
        let req = self.worker.last_for(handle);
        (req.internal_session.clone(), slot_of(&req))
    }

    /// `session` of `role` announces its end at `now`.
    fn end(&self, now: &str, role: &str, session: &str) -> orchestration::SessionEndedReceipt {
        self.with(now, role, Some(session), |ctx, store| {
            orchestration::session_ended(ctx, store, session)
        })
        .expect("session ended")
    }

    /// Walk a never-running owed session through its two looks, ticking `look` at `clock + 120 s`
    /// and `clock + 180 s`. Returns the second receipt.
    fn two_looks(&self, clock: &str, look: &str) -> TickReceipt {
        self.tick(&at_from(clock, STRANDED_GRACE_SECS), look);
        self.tick(&at_from(clock, STRANDED_GRACE_SECS + 60), look)
    }
}

// ---- AC 1 (b)-(g) ---------------------------------------------------------------------------------

/// **AC 1 (b)** — a parallel round: A never binds, B answers at +60. The round is NOT consolidated
/// at B's answer (A has not settled), A's look still stands, and A is recorded at +180 — at which
/// point the round consolidates with the escalation, carrying B's answer.
#[test]
fn a_silent_member_of_a_parallel_round_is_recorded_while_its_sibling_answered() {
    let team = team(&[PANEL]);
    let root = team.commission(T0, "panel", "review the build").thread_id;
    let (a, a_slot) = team.last("verifier");
    let (b, b_slot) = team.last("reviewer");

    team.answer(&at(60), "reviewer", &b, &b_slot, "LGTM");
    assert!(
        !team.needs_decision(&at(60), &root),
        "one answer of two does not end the round"
    );
    assert_eq!(
        team.messages(&root).len(),
        1,
        "nothing consolidated at B's answer"
    );
    assert_eq!(
        team.book.standing_look(&a).map(|l| (l.thread, l.due)),
        Some((root.clone(), at(STRANDED_GRACE_SECS))),
        "A's look still stands after the channel's window was re-armed"
    );

    team.two_looks(T0, &root);
    assert_eq!(
        team.records(),
        vec![(a_slot.clone(), a.clone(), 2, Some("posted".to_string()))]
    );
    assert!(team.needs_decision(&at(180), &root));
    assert_eq!(team.worker.starts(), vec!["verifier", "reviewer"]);
    let consolidated = team.messages(&root);
    assert_eq!(consolidated.len(), 2, "{consolidated:?}");
    assert_eq!(consolidated[1].1, "escalation");
}

/// **AC 1 (c)** — a direct commission (`send --to <persona>`): the look names the thread ITSELF
/// (there is no channel above it), the record is made, and the escalation lands there, where its
/// opener reads it.
#[test]
fn a_persona_commissioned_directly_that_never_starts_is_escalated_on_its_own_thread() {
    let team = team(&[]);
    let thread = team
        .commission(T0, "verifier", "verify the build")
        .thread_id;
    let (s, owed) = team.last("verifier");
    assert_eq!(owed, thread);
    assert_eq!(
        team.owed(&thread, "verifier"),
        Some((s.clone(), at(0), true))
    );
    assert_eq!(team.book.standing_look(&s).unwrap().thread, thread);

    team.two_looks(T0, &thread);
    assert_eq!(
        team.records(),
        vec![(thread.clone(), s.clone(), 2, Some("posted".to_string()))]
    );
    assert!(team.needs_decision(&at(180), &thread));
    assert_eq!(
        team.messages(&thread),
        vec![
            ("pm".to_string(), "info".to_string(), false),
            ("verifier".to_string(), "escalation".to_string(), true),
        ]
    );
    assert_eq!(team.worker.starts(), vec!["verifier"]);
}

/// **AC 1 (d)** — the look cannot be armed. The commission's receipt says so; the sweep is
/// workspace-wide, so a tick by hand looks anyway; and the channel's own tick at the window's end
/// does NOT advance past the slot.
#[test]
fn a_look_that_cannot_be_armed_is_reported_and_the_next_tick_looks_anyway() {
    let team = team(&[REVIEW_2M]);
    team.book.refuse_looks(true);
    let sent = team.commission(T0, "review2", "verify the build");
    let root = sent.thread_id.clone();
    let (s, slot) = team.last("verifier");
    // The commission's receipt carries the finding — the first step's start is part of it.
    let unarmed: Vec<&FailedConsequence> = sent
        .warnings
        .iter()
        .filter(|f| {
            f.class == ConsequenceClass::TickUnscheduled && f.session.as_deref() == Some(&s)
        })
        .collect();
    assert!(!unarmed.is_empty(), "{:?}", classes(&sent.warnings));

    // Just past +120 s: the window has ended AND the first look falls. The suspect is written, the
    // second arm fails and says so, and the flow does not move past the step.
    let r = team.tick(&at(121), &root);
    assert_eq!(team.suspect(&slot).map(|s| s.1), Some(at(121)));
    assert!(
        has(&r.warnings, ConsequenceClass::TickUnscheduled),
        "{:?}",
        classes(&r.warnings)
    );
    assert!(
        has(&r.warnings, ConsequenceClass::StepNotRunning),
        "{:?}",
        classes(&r.warnings)
    );
    assert_eq!(
        team.worker.started("finisher"),
        0,
        "never advanced past the dead step"
    );

    let r = team.tick(&at(181), &root);
    assert!(has(&r.warnings, ConsequenceClass::SessionStranded));
    assert_eq!(team.records().len(), 1);
    assert!(team.needs_decision(&at(181), &root));
    assert_eq!(team.worker.started("finisher"), 0);
}

/// **AC 1 (e)** — a slot that was opened and whose trigger never ran (`triggered = 0`, seeded
/// through the store: the kill between the two writes cannot be staged through the seam). The look
/// was booked at the mint; two looks from the clock record observation 1.
#[test]
fn a_slot_whose_trigger_never_ran_is_recorded_as_never_started() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify the build").thread_id;
    let (s, slot) = team.last("verifier");
    assert!(
        team.book.looks_for(&s).len() >= 2,
        "one look at the mint, one before the spawn: {:?}",
        team.book.looks_for(&s)
    );
    team.sql("UPDATE owed_starts SET triggered = 0");
    team.two_looks(T0, &root);
    assert_eq!(
        team.records(),
        vec![(slot, s, 1, Some("posted".to_string()))]
    );
}

/// **AC 1 (f)** — a re-trigger ends the record's currency. The commissioner answers the escalation
/// with a new round; the new slot shows no STRANDED line while it runs; it strands again → a SECOND
/// record and a SECOND escalation.
#[test]
fn a_new_round_after_the_escalation_is_judged_afresh_and_can_strand_again() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify the build").thread_id;
    team.two_looks(T0, &root);
    assert_eq!(team.records().len(), 1);

    // The commissioner (a human) answers the escalation with a reply into the root.
    let again = at(600);
    team.with(&again, "pm", None, |ctx, store| {
        surface::reply_in_thread(
            ctx,
            store,
            ReplyThreadRequest {
                machine: None,
                thread: &root,
                body: "try again",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
    })
    .expect("the commissioner answers");
    assert_eq!(
        team.worker.started("verifier"),
        2,
        "{:?}",
        team.worker.starts()
    );
    let (s2, slot2) = team.last("verifier");
    assert_eq!(
        team.owed(&slot2, "verifier").map(|o| o.1),
        Some(again.clone())
    );
    assert!(
        team.status_of(&again, &slot2).stranded.is_none(),
        "no STRANDED line while the new round runs"
    );

    team.two_looks(&again, &root);
    let records = team.records();
    assert_eq!(records.len(), 2, "{records:?}");
    assert_eq!(records[1].0, slot2);
    assert_eq!(records[1].1, s2);
    let escalations = team
        .messages(&slot2)
        .into_iter()
        .filter(|m| m.1 == "escalation")
        .count();
    assert_eq!(escalations, 1, "a second escalation, on the new slot");
}

/// **AC 1 (g)** — a start that failed and was given up (`retry_at` NULL) is recorded with
/// observation 5 at its last attempt plus the grace, and the slot was never advanced past.
#[test]
fn a_start_given_up_is_recorded_and_escalated() {
    let team = team(&[REVIEW_2M]);
    let root = team.commission(T0, "review2", "verify the build").thread_id;
    let (s, slot) = team.last("verifier");
    let last = at(10);
    team.sql(&format!(
        "INSERT INTO failed_starts(session, scope_key, role, thread, message, model, depth, \
         priority, enqueued_at, first_failed_at, last_failed_at, attempts, error, retry_at) \
         VALUES ('{s}', 'k', 'verifier', '{slot}', 'verify', NULL, 1, 1, '{T0}', '{T0}', \
         '{last}', 3, 'io: timed out', NULL)"
    ));
    team.tick(&at(10 + STRANDED_GRACE_SECS - 1), &root);
    assert!(
        team.records().is_empty(),
        "not before the grace on the last attempt"
    );
    team.tick(&at(10 + STRANDED_GRACE_SECS), &root);
    assert_eq!(
        team.records(),
        vec![(slot, s, 5, Some("posted".to_string()))]
    );
    assert_eq!(team.worker.started("finisher"), 0);
}

// ---- AC 4: a held member does not lapse -----------------------------------------------------------

/// **AC 4 (a), (e)** — a `five_hour` hold with a known instant. The window (two minutes) runs out
/// while the session is on hold: the tick does not advance, does not consolidate, and arms the next
/// look for the instant the hold lifts. Resumed, the session's obligation clock moves to the resume
/// — the sweep 90 s later suspects nothing although the session holds no claim — and its answer
/// starts the next step ONCE.
#[test]
fn a_member_on_hold_is_paused_past_its_window_and_its_clock_moves_on_resume() {
    let team = team(&[REVIEW_2M]);
    let root = team.commission(T0, "review2", "verify the build").thread_id;
    let (s, slot) = team.last("verifier");
    let until = at(600);
    team.end(&at(30), "verifier", &s);
    team.with(&at(30), "verifier", Some(&s), |ctx, store| {
        orchestration::session_interrupted(ctx, store, &s, "five_hour", Some(&until), "limit")
    })
    .expect("the hold is recorded");

    let r = team.tick(&at(180), &root);
    assert_eq!(
        team.worker.started("finisher"),
        0,
        "not advanced past a held step"
    );
    assert!(!team.needs_decision(&at(180), &root));
    assert!(team.records().is_empty(), "a hold is not stranded");
    assert!(
        has(&r.warnings, ConsequenceClass::StepStillWaiting),
        "{:?}",
        classes(&r.warnings)
    );
    assert!(
        team.book
            .windows
            .lock()
            .unwrap()
            .iter()
            .any(|(t, due)| *t == root && *due == until),
        "the next look is armed for the instant the hold lifts: {:?}",
        team.book.windows.lock().unwrap()
    );

    let resumed = at(660);
    team.with(&resumed, "service", None, |ctx, store| {
        orchestration::resume_interrupted(
            ctx,
            store,
            orchestration::ResumeRequest {
                scope: orchestration::ResumeScope::Session(&s),
                force: false,
            },
        )
    })
    .expect("resume");
    assert_eq!(
        team.worker.started("verifier"),
        2,
        "{:?}",
        team.worker.starts()
    );
    assert_eq!(
        team.owed(&slot, "verifier").map(|o| o.1),
        Some(resumed.clone())
    );
    // AC 4 (e), build item 12: no claim at +90 — only the moved clock keeps it from a suspect.
    team.tick(&at_from(&resumed, 90), &root);
    assert_eq!(team.suspect(&slot), None);

    team.answer(&at(700), "verifier", &s, &slot, "verified");
    assert_eq!(
        team.worker.started("finisher"),
        1,
        "the next step starts once"
    );
    team.tick(&at(800), &root);
    assert_eq!(team.worker.started("finisher"), 1);
    assert!(team.records().is_empty());
}

/// **AC 2 (b), cn4p as the reporter saw it** — a `five_hour` hold taken up by the EXISTING resume,
/// which continues the session; the continued session ends owing an answer with NO new hold and no
/// substitute. Nothing is recorded while the hold stood; the record comes at the SECOND end plus the
/// grace, judged from that end.
#[test]
fn a_resumed_session_that_ends_again_without_a_new_hold_is_recorded_from_its_second_end() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify").thread_id;
    let (s, slot) = team.last("verifier");
    team.end(&at(30), "verifier", &s);
    team.with(&at(30), "verifier", Some(&s), |ctx, store| {
        orchestration::session_interrupted(ctx, store, &s, "five_hour", Some(&at(600)), "limit")
    })
    .unwrap();
    team.tick(&at(400), &root);
    assert!(team.records().is_empty(), "never while the hold stands");

    team.with(&at(660), "service", None, |ctx, store| {
        orchestration::resume_interrupted(
            ctx,
            store,
            orchestration::ResumeRequest {
                scope: orchestration::ResumeScope::Session(&s),
                force: false,
            },
        )
    })
    .expect("resume");
    assert_eq!(team.worker.started("verifier"), 2, "continued");
    let second_end = at(700);
    team.end(&second_end, "verifier", &s);
    team.tick(&at_from(&second_end, STRANDED_GRACE_SECS - 1), &root);
    assert!(team.records().is_empty());
    team.tick(&at_from(&second_end, STRANDED_GRACE_SECS), &root);
    assert_eq!(
        team.records(),
        vec![(slot, s, 3, Some("posted".to_string()))]
    );
}

/// **AC 4 (d)** — a hold with NO instant arms nothing for it on decline, and the member stays
/// paused — not lapsed, not recorded.
#[test]
fn a_hold_with_no_instant_arms_nothing_and_stays_paused() {
    let team = team(&[REVIEW_2M]);
    let root = team.commission(T0, "review2", "verify the build").thread_id;
    let (s, _slot) = team.last("verifier");
    team.end(&at(30), "verifier", &s);
    team.with(&at(30), "verifier", Some(&s), |ctx, store| {
        orchestration::session_interrupted(ctx, store, &s, "five_hour", None, "limit")
    })
    .expect("the hold is recorded");
    let windows_before = team.book.windows.lock().unwrap().len();
    team.tick(&at(300), &root);
    let windows: Vec<(String, String)> =
        team.book.windows.lock().unwrap()[windows_before..].to_vec();
    assert!(
        windows
            .iter()
            .all(|(_, due)| due.as_str() > at(300).as_str()),
        "nothing armed for a past instant: {windows:?}"
    );
    assert_eq!(team.worker.started("finisher"), 0);
    assert!(team.records().is_empty());
}

// ---- AC 5: false positives — each records NOTHING -------------------------------------------------

/// Asserts the false-positive contract: no suspect kept, no record, no escalation on `slot`, and no
/// start beyond `starts`.
fn nothing_recorded(team: &Team, slot: &str, starts: &[&str]) {
    assert_eq!(team.suspect(slot), None, "no suspect kept on {slot}");
    assert!(team.records().is_empty(), "{:?}", team.records());
    assert!(
        team.messages(slot).iter().all(|m| m.1 != "escalation"),
        "{:?}",
        team.messages(slot)
    );
    assert_eq!(team.worker.starts(), starts);
}

/// **AC 5 (a)** — a session that holds a live claim from its trigger on is never suspected, and
/// the look comes back one cadence out while it does.
#[test]
fn false_positive_a_running_session_is_never_suspected() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify").thread_id;
    let (s, slot) = team.last("verifier");
    team.worker.mark_running(&s);
    team.two_looks(T0, &root);
    nothing_recorded(&team, &slot, &["verifier"]);
    assert_eq!(team.book.standing_look(&s).unwrap().due, at(240));
}

/// **AC 5 (b)** — a clean end after answering.
#[test]
fn false_positive_a_clean_end_after_answering() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify").thread_id;
    let (s, slot) = team.last("verifier");
    team.answer(&at(30), "verifier", &s, &slot, "verified");
    team.end(&at(31), "verifier", &s);
    team.two_looks(T0, &root);
    team.tick(&at(400), &root);
    nothing_recorded(&team, &slot, &["verifier"]);
}

/// **AC 5 (f)** — a standing quota hold, not due.
#[test]
fn false_positive_a_standing_hold() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify").thread_id;
    let (s, slot) = team.last("verifier");
    team.end(&at(30), "verifier", &s);
    team.with(&at(30), "verifier", Some(&s), |ctx, store| {
        orchestration::session_interrupted(ctx, store, &s, "five_hour", Some(&at(3600)), "x")
    })
    .unwrap();
    team.two_looks(T0, &root);
    team.tick(&at(400), &root);
    nothing_recorded(&team, &slot, &["verifier"]);
}

/// **AC 5 (i)** — a worker that cannot look records nothing and says so ONCE per tick; the slot is
/// settled as before (here: the two-minute window lapses and the flow advances, as it always did).
#[test]
fn false_positive_a_worker_that_cannot_look() {
    let team = team(&[REVIEW_2M]);
    team.worker.go_blind();
    let root = team.commission(T0, "review2", "verify").thread_id;
    let (_s, slot) = team.last("verifier");
    let r = team.tick(&at(180), &root);
    let unknown = r
        .warnings
        .iter()
        .filter(|f| f.class == ConsequenceClass::LivenessUnknown)
        .count();
    assert_eq!(unknown, 1, "{:?}", classes(&r.warnings));
    assert_eq!(team.suspect(&slot), None);
    assert!(team.records().is_empty());
    assert_eq!(
        team.worker.started("finisher"),
        1,
        "settled by its window, as before"
    );
}

/// **AC 5 (m)** — the claim appears between the looks: the suspect goes, nothing is recorded.
#[test]
fn false_positive_a_claim_that_appears_between_the_looks() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify").thread_id;
    let (s, slot) = team.last("verifier");
    team.tick(&at(120), &root);
    assert!(team.suspect(&slot).is_some());
    team.worker.mark_running(&s);
    team.tick(&at(180), &root);
    nothing_recorded(&team, &slot, &["verifier"]);
}

/// **AC 5 (p)** — a slot opened BEFORE this version has no obligation record: no look, no record,
/// and its window lapses and advances exactly as it did.
#[test]
fn false_positive_a_slot_from_before_this_version_lapses_as_before() {
    let team = team(&[REVIEW_2M]);
    let root = team.commission(T0, "review2", "verify").thread_id;
    let (_s, slot) = team.last("verifier");
    team.sql("DELETE FROM owed_starts");
    let r = team.tick(&at(180), &root);
    assert_eq!(team.suspect(&slot), None);
    assert!(team.records().is_empty());
    assert_eq!(
        team.worker.started("finisher"),
        1,
        "{:?}",
        classes(&r.warnings)
    );
    assert!(has(&r.warnings, ConsequenceClass::StepUnanswered));
}

/// **AC 5 (r), build item 2** — the evidence tuple is load-bearing: the transcript grows between the
/// looks, so nothing is recorded; the suspect is RESET to the new count, and the look re-armed one
/// cadence out. Red when the comparison is deleted.
#[test]
fn false_positive_a_transcript_that_grows_between_the_looks_resets_the_suspect() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify").thread_id;
    let (s, slot) = team.last("verifier");
    team.tick(&at(120), &root);
    assert_eq!(team.suspect(&slot).map(|x| x.1), Some(at(120)));
    team.sql(&format!(
        "INSERT INTO agent_transcript(internal_session, seq, kind, data) \
         VALUES ('{s}', 1, 'assistant', '{{}}')"
    ));
    team.tick(&at(180), &root);
    assert!(team.records().is_empty(), "{:?}", team.records());
    assert_eq!(
        team.suspect(&slot).map(|x| x.1),
        Some(at(180)),
        "reset, not deleted"
    );
    assert_eq!(team.book.standing_look(&s).unwrap().due, at(240));
}

/// **AC 5 (l)** — a slot of a round the supervisor is finished with. A hung reviewer lapsed and the
/// parallel round was consolidated past it; its process then goes. Nothing is recorded: nothing
/// would be woken by an answer there.
#[test]
fn false_positive_a_slot_of_a_consolidated_round() {
    let team =
        team(&["name: panel2m\nmembers: [verifier, reviewer]\nflow: parallel\ntimeout: 2m\n"]);
    let root = team.commission(T0, "panel2m", "review").thread_id;
    let (a, a_slot) = team.last("verifier");
    let (b, b_slot) = team.last("reviewer");
    team.worker.mark_running(&a);
    // The hung verifier's window lapses (it is running, so it is settled as before); the reviewer's
    // late answer then completes the set and the round is consolidated past the hung member.
    team.tick(&at(180), &root);
    team.answer(&at(190), "reviewer", &b, &b_slot, "LGTM");
    assert_ne!(
        team.messages(&root).len(),
        1,
        "the round was consolidated past the hung member"
    );
    team.worker.mark_gone(&a);
    team.tick(&at(300), &root);
    team.tick(&at(400), &root);
    assert!(team.records().is_empty(), "{:?}", team.records());
    assert_eq!(team.suspect(&a_slot), None);
}

// ---- AC 6: exactly one escalation, decided from the evidence ------------------------------------

/// **AC 6 (b)** — the post LANDED and the stamp failed (an injected SQL trigger, the br25
/// precedent). Exactly one escalation is on the slot, the record reads "not posted yet", and the
/// next sweep finds the substituted escalation sent as the member, stamps it — and posts NOTHING.
#[test]
fn a_post_that_landed_while_its_stamp_failed_is_stamped_from_the_evidence_and_not_repeated() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify").thread_id;
    let (_s, slot) = team.last("verifier");
    team.sql(
        "CREATE TRIGGER inj_stamp BEFORE UPDATE OF escalation ON stranded_sessions \
         WHEN NEW.escalation = 'posted' BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    let r = team.two_looks(T0, &root);
    assert!(
        has(&r.warnings, ConsequenceClass::StrandedNotEscalated),
        "{:?}",
        classes(&r.warnings)
    );
    let escalations = |team: &Team| {
        team.messages(&slot)
            .into_iter()
            .filter(|m| m.1 == "escalation")
            .count()
    };
    assert_eq!(escalations(&team), 1);
    assert_eq!(team.records()[0].3, None);
    assert_eq!(
        team.status_of(&at(180), &slot)
            .stranded
            .map(|s| s.escalation),
        Some(StrandedEscalation::Pending)
    );

    team.sql("DROP TRIGGER inj_stamp;");
    team.tick(&at(240), &root);
    assert_eq!(team.records()[0].3.as_deref(), Some("posted"));
    assert_eq!(escalations(&team), 1, "posted NOTHING a second time");
}

/// **AC 6 (c) and the transient half of (d)** — a post that fails transiently, and a slot answered
/// meanwhile — are proven in `stranded.rs`'s unit tests through an injected post: the op-log insert
/// a SQL trigger could break here unwraps, so an injected failure there panics instead of returning
/// the error the sweep handles. The PERMANENT half of (d) is a real refusal, end to end: the depth
/// guard refuses a session past its cap (`validation`), and the escalation is given up at once.
#[test]
fn a_post_the_engine_refuses_for_good_is_given_up_on_the_first_attempt() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify").thread_id;
    let (s, slot) = team.last("verifier");
    team.sql(&format!(
        "UPDATE session_map SET depth = 99 WHERE internal_id = '{s}'"
    ));
    let r = team.two_looks(T0, &root);
    let finding = r
        .warnings
        .iter()
        .find(|f| f.class == ConsequenceClass::StrandedNotEscalated)
        .expect("the refusal is a finding");
    assert!(finding.detail.contains("given up"), "{}", finding.detail);
    assert_eq!(team.records()[0].3.as_deref(), Some("given_up"));
    let row = team.status_of(&at(180), &slot).stranded.unwrap();
    assert_eq!(row.escalation, StrandedEscalation::GivenUp);
    let looks = team.book.looks_for(&s).len();
    team.tick(&at(300), &root);
    assert_eq!(
        team.book.looks_for(&s).len(),
        looks,
        "nothing armed after giving up"
    );
    assert!(team.messages(&slot).iter().all(|m| m.1 != "escalation"));
}

// ---- AC 7: a pinned past clock -----------------------------------------------------------------

/// **AC 7** — there is no cutover clock to predate: a whole scenario under a clock in 2024 records
/// and escalates like any other.
#[test]
fn a_scenario_under_a_past_clock_records_like_any_other() {
    let team = team(&[REVIEW]);
    let past = "2024-03-01T08:00:00Z";
    let root = team.commission(past, "review", "verify").thread_id;
    let (s, slot) = team.last("verifier");
    team.two_looks(past, &root);
    assert_eq!(
        team.records(),
        vec![(slot, s, 2, Some("posted".to_string()))]
    );
}

// ---- AC 8: order and arming -----------------------------------------------------------------------

/// **AC 8 (a)** — the sweep runs BEFORE the thread's own move: at the tick where the window ends and
/// the second look falls, the step is recorded and escalated, never advanced past.
#[test]
fn the_sweep_runs_before_the_window_is_read_on_the_same_tick() {
    let team =
        team(&["name: review3\nmembers: [verifier, finisher]\nflow: sequential\ntimeout: 3m\n"]);
    let root = team.commission(T0, "review3", "verify").thread_id;
    team.tick(&at(120), &root);
    team.tick(&at(180), &root);
    assert_eq!(team.records().len(), 1);
    assert_eq!(
        team.worker.started("finisher"),
        0,
        "{:?}",
        team.worker.starts()
    );
    assert!(team.needs_decision(&at(180), &root));
}

/// **AC 8 (c)** — a session that ends still owing its thread arms the look at `ended + 120 s`; one
/// that answered first arms nothing.
#[test]
fn a_session_end_arms_a_look_only_while_its_thread_is_still_owed() {
    let team = team(&[PANEL]);
    team.commission(T0, "panel", "review");
    let (a, _a_slot) = team.last("verifier");
    let (b, b_slot) = team.last("reviewer");
    let before_b = team.book.looks_for(&b).len();
    team.answer(&at(30), "reviewer", &b, &b_slot, "LGTM");
    team.end(&at(31), "reviewer", &b);
    assert_eq!(
        team.book.looks_for(&b).len(),
        before_b,
        "an answered end arms nothing"
    );

    let ended = at(40);
    let r = team.end(&ended, "verifier", &a);
    assert!(r.warnings.is_empty(), "{:?}", classes(&r.warnings));
    assert_eq!(
        team.book.standing_look(&a).unwrap().due,
        at_from(&ended, STRANDED_GRACE_SECS)
    );
}

/// **AC 8 (d)** — the look's tick on a channel whose window is two hours away changes nothing but
/// the suspect row.
#[test]
fn the_looks_tick_on_a_long_window_changes_nothing_but_the_suspect() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify").thread_id;
    let (_s, slot) = team.last("verifier");
    let before = (team.messages(&root), team.messages(&slot));
    let r = team.tick(&at(120), &root);
    assert!(!r.acted);
    assert_eq!(r.reason, "not_due");
    assert_eq!((team.messages(&root), team.messages(&slot)), before);
    assert!(team.suspect(&slot).is_some());
}

/// **AC 8 (e)** — retention: a stranded record older than thirty days is gone.
#[test]
fn a_stranded_record_older_than_a_month_is_dropped() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify").thread_id;
    team.two_looks(T0, &root);
    assert_eq!(team.records().len(), 1);
    team.tick(&at(31 * 24 * 3600), &root);
    assert!(team.records().is_empty());
}

// ---- AC 3: no step past a dead step, no step on a missing declared input -----------------------

/// The pb9y shape: `build -> verify -> judge -> finish`, `finish` reads the verify report and the
/// verdict, every step two minutes long.
const PB9Y: &str =
    "name: coding\nmembers: [coder, verifier, judge, finisher]\ntimeout: 2m\nsteps:\n  \
                    - id: build\n    target: coder\n    next: verify\n  \
                    - id: verify\n    target: verifier\n    next: judge\n  \
                    - id: judge\n    target: judge\n    next: finish\n  \
                    - id: finish\n    target: finisher\n    input: [verify, judge]\n";

/// [`PB9Y`] without the declared input — the shape where a hung verifier is advanced past as
/// before (nxf 6j6v.rs9k).
const PB9Y_NO_INPUT: &str =
    "name: coding\nmembers: [coder, verifier, judge, finisher]\ntimeout: 2m\nsteps:\n  \
                             - id: build\n    target: coder\n    next: verify\n  \
                             - id: verify\n    target: verifier\n    next: judge\n  \
                             - id: judge\n    target: judge\n    next: finish\n  \
                             - id: finish\n    target: finisher\n";

impl Team {
    /// Commission the coding run and let the coder answer at `+10 s`; returns the root and the
    /// instant the verify step started.
    fn built(&self) -> (String, String) {
        let root = self.commission(T0, "coding", "build T5").thread_id;
        let (coder, coder_slot) = self.last("coder");
        let started = at(10);
        self.answer(&started, "coder", &coder, &coder_slot, "built");
        assert_eq!(
            self.worker.started("verifier"),
            1,
            "{:?}",
            self.worker.starts()
        );
        (root, started)
    }

    /// [`built`](Self::built), commissioned by the `pm` persona's session.
    fn built_for_pm(&self) -> (String, String) {
        let root = self.commission_as_pm(T0, "coding", "build T5").thread_id;
        let (coder, coder_slot) = self.last("coder");
        let started = at(10);
        self.answer(&started, "coder", &coder, &coder_slot, "built");
        assert_eq!(
            self.worker.started("verifier"),
            1,
            "{:?}",
            self.worker.starts()
        );
        (root, started)
    }

    fn counts(&self) -> [usize; 4] {
        ["coder", "verifier", "judge", "finisher"].map(|h| self.worker.started(h))
    }
}

/// **AC 3 (a)** — `verify` never starts. At its window's end (BEFORE the record) the tick answers
/// `Waiting` with `StepNotRunning` and `judge` is not opened; at the record, the round escalates.
#[test]
fn a_dead_step_is_not_advanced_past_and_the_round_escalates_at_the_record() {
    let team = team(&[PB9Y]);
    let (root, started) = team.built();
    let (_v, v_slot) = team.last("verifier");

    let window_end = at_from(&started, 121);
    let r = team.tick(&window_end, &root);
    assert!(!r.acted);
    assert!(
        has(&r.warnings, ConsequenceClass::StepNotRunning),
        "{:?}",
        classes(&r.warnings)
    );
    assert_eq!(
        team.counts(),
        [1, 1, 0, 0],
        "judge is not opened past a dead step"
    );

    team.tick(&at_from(&window_end, 60), &root);
    assert_eq!(team.records().len(), 1, "{:?}", team.records());
    assert_eq!(team.records()[0].0, v_slot);
    assert!(team.needs_decision(&at(300), &root));
    assert_eq!(team.counts(), [1, 1, 0, 0]);
    assert_eq!(
        team.messages(&v_slot),
        vec![
            ("__channel__".to_string(), "task".to_string(), false),
            ("verifier".to_string(), "escalation".to_string(), true),
        ]
    );
}

/// **AC 3 (b)** — the hung case is KEPT (nxf 6j6v.rs9k): a verifier still RUNNING past its window,
/// with no declared-input consumer behind it, is advanced past exactly as before, finding unchanged.
#[test]
fn a_hung_step_with_a_live_process_is_still_advanced_past() {
    let team = team(&[PB9Y_NO_INPUT]);
    let (root, started) = team.built();
    let (v, _v_slot) = team.last("verifier");
    team.worker.mark_running(&v);
    let r = team.tick(&at_from(&started, 121), &root);
    assert_eq!(r.reason, "advanced", "{:?}", classes(&r.warnings));
    assert_eq!(team.counts(), [1, 1, 1, 0]);
    let lapse = r
        .warnings
        .iter()
        .find(|f| f.class == ConsequenceClass::StepUnanswered)
        .expect("the lapse is a finding");
    assert_eq!(
        lapse.detail,
        "step \"verify\" gave no answer inside its declared window; the flow advanced to step \
         \"judge\" without it"
    );
    assert!(team.records().is_empty());
}

/// **AC 3 (c)** — as (b), but `finish` declares `input: [verify, judge]`: `judge` answers, and
/// `finish` is NOT opened — the round escalates, naming `finish`, `verify`, verify's slot and its
/// session's state.
#[test]
fn a_step_whose_declared_input_stayed_silent_is_not_opened() {
    let team = team(&[PB9Y]);
    let (root, started) = team.built_for_pm();
    let (v, v_slot) = team.last("verifier");
    team.worker.mark_running(&v);
    team.tick(&at_from(&started, 121), &root);
    assert_eq!(team.counts(), [1, 1, 1, 0]);
    let (j, j_slot) = team.last("judge");
    team.answer(&at(200), "judge", &j, &j_slot, "approved");

    assert_eq!(team.counts(), [1, 1, 1, 0], "finish is never opened");
    assert!(team.needs_decision(&at(200), &root));
    // The commissioner is woken ONCE, with the escalation and its note.
    assert_eq!(team.worker.started("pm"), 1, "{:?}", team.worker.starts());
    let note = team.worker.last_for("pm").message;
    for named in [
        "\"finish\"",
        "\"verify\"",
        v_slot.as_str(),
        v.as_str(),
        "still running",
    ] {
        assert!(note.contains(named), "the note names {named}: {note}");
    }
}

/// **AC 3 (e)** — a substitute is not an answer: verify's session ends and the runtime's teardown
/// posts a substituted, non-escalating reply that discharges its slot. `judge` opens on it (the
/// default edge, unchanged) — and `finish`, which declares verify's report, does NOT.
#[test]
fn a_step_whose_declared_input_was_answered_only_by_a_substitute_is_not_opened() {
    let team = team(&[PB9Y]);
    let (root, _started) = team.built();
    let (v, v_slot) = team.last("verifier");
    team.with(&at(20), "verifier", Some(&v), |ctx, store| {
        surface::settle_if_unanswered(
            ctx,
            store,
            ReplyThreadRequest {
                machine: None,
                thread: &v_slot,
                body: "the session ended without answering",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
    })
    .expect("the teardown's substitute");
    assert_eq!(team.counts(), [1, 1, 1, 0]);
    let (j, j_slot) = team.last("judge");
    team.answer(&at(30), "judge", &j, &j_slot, "approved");
    assert_eq!(
        team.counts(),
        [1, 1, 1, 0],
        "finish is not opened on a substitute"
    );
    assert!(team.needs_decision(&at(30), &root));
}

// ---- AC 6 (a): exactly zero starts, exactly one wake ---------------------------------------------

/// **AC 6 (a)** with a PERSONA commissioner: the worker's log holds the round's own start and
/// exactly ONE wake of the commissioner, carrying the escalation; no resume is ever armed.
#[test]
fn a_stranded_round_wakes_its_persona_commissioner_exactly_once() {
    let team = team(&[REVIEW]);
    let root = team.commission_as_pm(T0, "review", "verify").thread_id;
    team.two_looks(T0, &root);
    team.tick(&at(300), &root);
    team.tick(&at(400), &root);
    assert_eq!(
        team.worker.starts(),
        vec!["verifier", "pm"],
        "one start, one wake"
    );
    assert!(team.worker.last_for("pm").message.contains("STRANDED"));
    assert!(team.book.resumes.lock().unwrap().is_empty());
}

// ---- AC 3 (d): a back-edge pass ---------------------------------------------------------------

/// [`PB9Y`] with a way back from the verdict to the build.
const PB9Y_LOOP: &str = "name: coding\nmembers: [coder, verifier, judge, finisher]\ntimeout: 2m\nsteps:\n  \
                         - id: build\n    target: coder\n    next: verify\n  \
                         - id: verify\n    target: verifier\n    next: judge\n  \
                         - id: judge\n    target: judge\n    next: finish\n    on_needs_rework: build\n    max_passes: 2\n  \
                         - id: finish\n    target: finisher\n    input: [verify, judge]\n";

impl Team {
    fn rework(&self, now: &str, role: &str, session: &str, thread: &str, body: &str) {
        self.with(now, role, Some(session), |ctx, store| {
            surface::reply_in_thread(
                ctx,
                store,
                ReplyThreadRequest {
                    machine: None,
                    thread,
                    body,
                    escalate: false,
                    needs_rework: true,
                    accept: false,
                },
            )
        })
        .expect("the verdict is posted");
    }
}

/// **AC 3 (d), build item 11** — one run over a back edge. Pass 1 completes with verify answered;
/// the judge sends the work back; on pass 2 verify hangs and is advanced past; the judge answers —
/// and `finish` is NOT opened although verify's pass-1 answer exists: the newest slot is pass 2's.
/// The note names pass 2.
#[test]
fn an_answer_from_before_a_back_edge_does_not_satisfy_the_pass_after_it() {
    let team = team(&[PB9Y_LOOP]);
    let root = team.commission_as_pm(T0, "coding", "build T5").thread_id;
    let (c, c_slot) = team.last("coder");
    team.answer(&at(10), "coder", &c, &c_slot, "built");
    let (v, v_slot) = team.last("verifier");
    team.answer(&at(20), "verifier", &v, &v_slot, "VERIFY REPORT pass 1");
    let (j, j_slot) = team.last("judge");
    team.rework(&at(30), "judge", &j, &j_slot, "send it back");
    assert_eq!(
        team.counts(),
        [2, 1, 1, 0],
        "back to the build: {:?}",
        team.worker.starts()
    );

    let (c2, c2_slot) = team.last("coder");
    team.answer(&at(40), "coder", &c2, &c2_slot, "built again");
    assert_eq!(team.counts(), [2, 2, 1, 0]);
    let (v2, v2_slot) = team.last("verifier");
    assert_ne!(v2_slot, v_slot, "pass 2 has a slot of its own");
    team.worker.mark_running(&v2);
    team.tick(&at(40 + 121), &root);
    assert_eq!(
        team.counts(),
        [2, 2, 2, 0],
        "the hung verifier was advanced past"
    );
    let (j2, j2_slot) = team.last("judge");
    team.answer(&at(200), "judge", &j2, &j2_slot, "approved");

    assert_eq!(
        team.counts(),
        [2, 2, 2, 0],
        "finish is not opened on pass 1's report"
    );
    let note = team.worker.last_for("pm").message;
    assert!(note.contains(&v2_slot) && note.contains("pass 2"), "{note}");
    assert!(!note.contains(&v_slot), "{note}");
}

// ---- AC 4 (b), (c), (f) ---------------------------------------------------------------------------

/// **AC 4 (b)** — the gap: the window is long past when the resume stamps. A tick before the resumed
/// session's first transcript flush answers `Waiting` — no lapse, no next step — because the window
/// restarted with the resume. (Without the restart the slot is stale with a RUNNING session, the
/// one shape still advanced past: this test is red then.)
#[test]
fn a_resume_restarts_the_window_so_the_running_session_is_not_advanced_past() {
    let team = team(&[REVIEW_2M]);
    let root = team.commission(T0, "review2", "verify").thread_id;
    let (s, _slot) = team.last("verifier");
    team.end(&at(30), "verifier", &s);
    team.with(&at(30), "verifier", Some(&s), |ctx, store| {
        orchestration::session_interrupted(ctx, store, &s, "five_hour", Some(&at(600)), "x")
    })
    .unwrap();
    team.with(&at(660), "service", None, |ctx, store| {
        orchestration::resume_interrupted(
            ctx,
            store,
            orchestration::ResumeRequest {
                scope: orchestration::ResumeScope::Session(&s),
                force: false,
            },
        )
    })
    .expect("resume");
    team.worker.mark_running(&s);
    let r = team.tick(&at(700), &root);
    assert!(
        !has(&r.warnings, ConsequenceClass::StepUnanswered),
        "{:?}",
        classes(&r.warnings)
    );
    assert_eq!(team.worker.started("finisher"), 0);
    assert!(!team.needs_decision(&at(700), &root));
}

/// **AC 4 (c)** — 0990: two of three reviewers on hold. No consolidation while they are held; after
/// the resume and their answers, ONE consolidation carries all three.
#[test]
fn a_parallel_round_waits_for_its_held_members_and_consolidates_once() {
    let team =
        team(&["name: trio\nmembers: [verifier, reviewer, judge]\nflow: parallel\ntimeout: 2m\n"]);
    let root = team.commission(T0, "trio", "review").thread_id;
    let (a, a_slot) = team.last("verifier");
    let (b, b_slot) = team.last("reviewer");
    let (c, c_slot) = team.last("judge");
    team.answer(&at(20), "verifier", &a, &a_slot, "A: fine");
    for (role, s) in [("reviewer", &b), ("judge", &c)] {
        team.end(&at(30), role, s);
        team.with(&at(30), role, Some(s), |ctx, store| {
            orchestration::session_interrupted(ctx, store, s, "five_hour", Some(&at(600)), "x")
        })
        .unwrap();
    }
    team.tick(&at(300), &root);
    assert_eq!(
        team.messages(&root).len(),
        1,
        "no consolidation while two are held"
    );
    for s in [&b, &c] {
        team.with(&at(660), "service", None, |ctx, store| {
            orchestration::resume_interrupted(
                ctx,
                store,
                orchestration::ResumeRequest {
                    scope: orchestration::ResumeScope::Session(s),
                    force: false,
                },
            )
        })
        .expect("resume");
    }
    team.answer(&at(700), "reviewer", &b, &b_slot, "B: fine");
    assert_eq!(team.messages(&root).len(), 1, "one answer still owed");
    team.answer(&at(710), "judge", &c, &c_slot, "C: fine");
    let consolidations: Vec<_> = team
        .messages(&root)
        .into_iter()
        .filter(|m| m.0 != "pm")
        .collect();
    assert_eq!(consolidations.len(), 1, "{consolidations:?}");
    assert_eq!(
        consolidations[0].1, "info",
        "a clean consolidation, not an escalation"
    );
    assert!(team.records().is_empty());
}

/// **AC 4 (f), AC 5 (g)** — a member that ended its turn to wait on a round it commissioned itself is
/// PAUSED past its window: `Waiting` with the reason, no lapse, no record. When the round answers,
/// the member is woken and its clock moves to the wake.
#[test]
fn a_member_waiting_on_its_own_sub_round_is_paused_and_its_clock_moves_on_the_wake() {
    let team = team(&[REVIEW_2M]);
    let root = team.commission(T0, "review2", "verify").thread_id;
    let (v, slot) = team.last("verifier");
    team.with(&at(10), "verifier", Some(&v), |ctx, store| {
        surface::send_to(
            ctx,
            store,
            SendToRequest {
                machine: None,
                to: "coder",
                body: "what does this do?",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
    })
    .expect("the member consults");
    let (c, sub) = team.last("coder");
    team.worker.mark_running(&c);
    team.end(&at(20), "verifier", &v);
    let r = team.tick(&at(180), &root);
    assert!(
        has(&r.warnings, ConsequenceClass::StepStillWaiting),
        "{:?}",
        classes(&r.warnings)
    );
    assert!(!has(&r.warnings, ConsequenceClass::StepUnanswered));
    assert_eq!(team.worker.started("finisher"), 0);
    team.tick(&at(240), &root);
    assert!(team.records().is_empty(), "a paused member is not stranded");

    team.worker.mark_gone(&c);
    team.answer(&at(300), "coder", &c, &sub, "it parses the book");
    assert_eq!(team.worker.started("verifier"), 2, "woken with the answer");
    assert_eq!(
        team.owed(&slot, "verifier").map(|o| o.1),
        Some(at(300)),
        "the clock moved"
    );
}

// ---- AC 5: more false positives ---------------------------------------------------------------

/// **AC 5 (d)** — a failed start whose retry is due.
#[test]
fn false_positive_a_failed_start_with_a_retry_due() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify").thread_id;
    let (s, slot) = team.last("verifier");
    team.sql(&format!(
        "INSERT INTO failed_starts(session, scope_key, role, thread, message, model, depth, \
         priority, enqueued_at, first_failed_at, last_failed_at, attempts, error, retry_at) \
         VALUES ('{s}', 'k', 'verifier', '{slot}', 'verify', NULL, 1, 1, '{T0}', '{T0}', '{T0}', \
         1, 'io: timed out', '{}')",
        at(3600)
    ));
    team.two_looks(T0, &root);
    nothing_recorded(&team, &slot, &["verifier"]);
}

/// **AC 5 (e)** — a session running past its window, its claim alive across both looks: nothing is
/// recorded, and the hung step is advanced past exactly as before (rs9k).
#[test]
fn false_positive_a_session_running_past_its_window() {
    let team = team(&[REVIEW_2M]);
    let root = team.commission(T0, "review2", "verify").thread_id;
    let (s, slot) = team.last("verifier");
    team.worker.mark_running(&s);
    team.tick(&at(121), &root);
    team.tick(&at(181), &root);
    assert_eq!(team.suspect(&slot), None);
    assert!(team.records().is_empty());
    assert_eq!(
        team.worker.started("finisher"),
        1,
        "advanced past as before"
    );
}

/// **AC 5 (h)** — the member's sub-round answered while it was still mid-turn, so the answer is HELD
/// for it; the member's process then ends. The tick hands the held answer over (the member is
/// woken with it, its clock moving to the wake) — and at no point is the member recorded or the
/// flow moved past it.
#[test]
fn false_positive_a_held_delivery_pending() {
    let team = team(&[REVIEW_2M]);
    let root = team.commission(T0, "review2", "verify").thread_id;
    let (v, slot) = team.last("verifier");
    team.worker.mark_running(&v);
    team.with(&at(10), "verifier", Some(&v), |ctx, store| {
        surface::send_to(
            ctx,
            store,
            SendToRequest {
                machine: None,
                to: "coder",
                body: "what does this do?",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
    })
    .expect("the member consults");
    let (c, sub) = team.last("coder");
    team.answer(&at(20), "coder", &c, &sub, "it parses the book");
    let held = |team: &Team| -> i64 {
        team.store()
            .connection()
            .query_row("SELECT COUNT(*) FROM pending_wakes", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(held(&team), 1, "held for the member, still mid-turn");
    team.worker.mark_gone(&v);
    team.end(&at(25), "verifier", &v);
    team.tick(&at(181), &root);
    assert_eq!(held(&team), 0, "the tick handed it over");
    assert_eq!(team.worker.started("verifier"), 2, "woken with it");
    team.tick(&at(241), &root);
    assert_eq!(team.suspect(&slot), None);
    assert!(
        team.records().iter().all(|r| r.0 != slot),
        "{:?}",
        team.records()
    );
    assert_eq!(team.worker.started("finisher"), 0);
}

/// **AC 5 (n)** — a session re-triggered by the existing resume, whose claim appears 90 s after its
/// (moved) clock: nothing is recorded.
#[test]
fn false_positive_a_resumed_session_whose_claim_appears_late() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify").thread_id;
    let (s, slot) = team.last("verifier");
    team.end(&at(30), "verifier", &s);
    team.with(&at(30), "verifier", Some(&s), |ctx, store| {
        orchestration::session_interrupted(ctx, store, &s, "five_hour", Some(&at(600)), "x")
    })
    .unwrap();
    let resumed = at(660);
    team.with(&resumed, "service", None, |ctx, store| {
        orchestration::resume_interrupted(
            ctx,
            store,
            orchestration::ResumeRequest {
                scope: orchestration::ResumeScope::Session(&s),
                force: false,
            },
        )
    })
    .expect("resume");
    team.tick(&at_from(&resumed, 60), &root);
    team.worker.mark_running(&s);
    team.tick(&at_from(&resumed, 120), &root);
    team.tick(&at_from(&resumed, 180), &root);
    nothing_recorded(&team, &slot, &["verifier", "verifier"]);
}

/// **AC 5 (q), pass-2 IR #1** — two live `pm` sessions. P1 owes R1; P2 owes R2, consults the coder,
/// and is woken on the way back. R1's obligation still names P1, unmoved; nothing is recorded; P1
/// answers R1 normally.
#[test]
fn false_positive_a_way_back_wake_of_one_pm_never_re_owns_another_pms_slot() {
    let team = team(&[]);
    let r1 = team.commission_by(T0, "carsten", "pm", "plan A").thread_id;
    let (p1, _) = team.last("pm");
    let r2 = team
        .commission_by(&at(5), "carsten", "pm", "plan B")
        .thread_id;
    let (p2, _) = team.last("pm");
    assert_ne!(p1, p2);
    team.worker.mark_running(&p1);
    team.with(&at(10), "pm", Some(&p2), |ctx, store| {
        surface::send_to(
            ctx,
            store,
            SendToRequest {
                machine: None,
                to: "coder",
                body: "estimate this",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
    })
    .expect("P2 consults");
    team.end(&at(11), "pm", &p2);
    let (c, sub) = team.last("coder");
    team.answer(&at(20), "coder", &c, &sub, "two days");
    assert_eq!(
        team.owed(&r1, "pm"),
        Some((p1.clone(), at(0), true)),
        "R1 still names P1"
    );
    assert_eq!(team.owed(&r2, "pm").map(|o| o.0), Some(p2.clone()));
    team.tick(&at(200), &r1);
    team.tick(&at(260), &r1);
    assert!(
        team.records().iter().all(|r| r.0 != r1),
        "nothing recorded for R1: {:?}",
        team.records()
    );
    team.answer(&at(300), "pm", &p1, &r1, "plan A done");
    assert!(team.messages(&r1).iter().all(|m| m.1 != "escalation"));
}

// ---- the cost, as data (pass-1 IR #11, build item 17) -------------------------------------------

/// **The sweep's cost, measured, not gated** — the median of a tick over a workspace of about two
/// hundred threads, with 20 and then 200 owed sessions that are running (so every one of them is a
/// candidate the sweep has to look at), plus one cold tick. Run by hand:
/// `cargo test -p nexus-chat --test a_stranded_session_is_recorded_and_escalated -- --ignored
/// --nocapture the_sweeps_cost`.
#[test]
#[ignore = "a stopwatch: data, not a gate"]
fn the_sweeps_cost_over_two_hundred_threads_with_twenty_and_two_hundred_owed_sessions() {
    for owed in [20, 200] {
        let team = team(&[REVIEW]);
        let mut roots = Vec::new();
        for n in 0..owed {
            let root = team
                .commission(T0, "review", &format!("verify {n}"))
                .thread_id;
            let (s, _) = team.last("verifier");
            team.worker.mark_running(&s);
            roots.push(root);
        }
        // Answered direct commissions fill the workspace up to about two hundred threads.
        for n in 0..(200usize.saturating_sub(2 * owed)) {
            let thread = team.commission(T0, "coder", &format!("task {n}")).thread_id;
            let (s, _) = team.last("coder");
            team.answer(&at(1), "coder", &s, &thread, "done");
        }
        let cold = std::time::Instant::now();
        team.tick(&at(10), &roots[0]);
        let cold = cold.elapsed();
        let mut runs: Vec<std::time::Duration> = (0..15)
            .map(|n| {
                let t = std::time::Instant::now();
                team.tick(&at(20 + n), &roots[0]);
                t.elapsed()
            })
            .collect();
        runs.sort();
        eprintln!(
            "one tick with {owed} owed running sessions: median {:?}, cold {cold:?}",
            runs[runs.len() / 2]
        );
    }
}

/// **Review finding CQ #1**: "a retry is due" means the same to the sweep as to the member-set read
/// — a failed start recorded against the slot's THREAD counts, whichever session it names. Before
/// the predicate was shared, the sweep keyed it by session alone and would record this slot.
#[test]
fn false_positive_a_retry_due_on_the_slots_thread_under_another_session() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify").thread_id;
    let (_s, slot) = team.last("verifier");
    team.sql(&format!(
        "INSERT INTO failed_starts(session, scope_key, role, thread, message, model, depth, \
         priority, enqueued_at, first_failed_at, last_failed_at, attempts, error, retry_at) \
         VALUES ('m-another-session', 'k', 'verifier', '{slot}', 'verify', NULL, 1, 1, '{T0}', \
         '{T0}', '{T0}', 1, 'io: timed out', '{}')",
        at(3600)
    ));
    team.two_looks(T0, &root);
    nothing_recorded(&team, &slot, &["verifier"]);
}

/// **Review finding CQ #6**: one obligation that cannot be judged (here: an unreadable clock) is a
/// finding about THAT obligation, and every other owed session is still judged on the same tick.
#[test]
fn one_obligation_that_cannot_be_judged_does_not_stop_the_others() {
    let team = team(&[REVIEW]);
    let root = team.commission(T0, "review", "verify A").thread_id;
    let (_a, a_slot) = team.last("verifier");
    team.commission(T0, "review", "verify B");
    let (b, b_slot) = team.last("verifier");
    team.sql(&format!(
        "UPDATE owed_starts SET trigger_clock = 'not an instant' WHERE thread = '{a_slot}'"
    ));
    team.tick(&at(120), &root);
    let r = team.tick(&at(180), &root);
    assert!(
        r.warnings
            .iter()
            .any(|f| f.class == ConsequenceClass::StrandedNotJudged
                && f.thread.as_deref() == Some(a_slot.as_str())),
        "{:?}",
        classes(&r.warnings)
    );
    assert_eq!(
        team.records(),
        vec![(b_slot, b, 2, Some("posted".to_string()))],
        "the later obligation was still judged and recorded"
    );
}

/// **AC 5 (c)** — a round queued for the working copy behind another operation's lease. Its slot
/// owes an answer and its session never ran — and it is not stranded: its start is still to come.
#[test]
fn false_positive_a_round_queued_for_the_working_copy() {
    let exclusive = "name: guarded\nmembers: [verifier]\nflow: sequential\ntimeout: 2m\nworking_tree: exclusive\n";
    let team = team(&[exclusive]);
    let first = team.commission(T0, "guarded", "first").thread_id;
    let (holder, _) = team.last("verifier");
    team.worker.mark_running(&holder);
    let second = team
        .commission_by(T0, "carsten", "guarded", "second")
        .thread_id;
    assert_eq!(
        team.worker.started("verifier"),
        1,
        "the second round waits for the copy"
    );
    let queued_slot: String = team
        .store()
        .connection()
        .query_row("SELECT thread FROM working_tree_queue", [], |r| r.get(0))
        .expect("the second round's step is queued");
    team.two_looks(T0, &second);
    team.tick(&at(300), &first);
    assert_eq!(team.suspect(&queued_slot), None);
    assert!(team.records().is_empty(), "{:?}", team.records());
}
