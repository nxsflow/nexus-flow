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
    assert_eq!(stranded.observed, 2);
    assert_eq!(stranded.session, s);
    assert_eq!(stranded.escalation, StrandedEscalation::Posted);
    assert!(stranded.escalated);
    assert!(team.needs_decision(&at(180), &root));
    assert_eq!(
        team.worker.starts(),
        vec!["verifier"],
        "exactly the round's own start — the sweep starts nothing"
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
