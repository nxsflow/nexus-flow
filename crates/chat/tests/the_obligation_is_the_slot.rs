//! **The obligation is the slot** (nxf 6j6v.kgfn, cut 1e of epic 6j6v.nmc9, section A).
//!
//! Every open slot has exactly one coordinator, and it is software: `coordinator_of(slot)` is the
//! one predicate that says which thread's tick that is, or that there is none. A coordinated slot
//! is owed a row keyed `(thread, role)` from the instant it is OPENED — before any session is
//! minted — and the trigger that serves it tells the sidecar `coordinated`, so the sidecar posts no
//! substitute for it. A slot with no coordinator keeps today's behaviour in full. Slots opened
//! before this cut are backfilled once, and the ones nothing will take up any more are recorded
//! `abandoned` with their reason, rendered, and never nudged.
//!
//! Driven through the COMPUTE LAYER — the functions the CLI and the service run — with a worker
//! whose three capabilities the test switches, and a recording timer, because the looks armed are
//! part of what is asserted. The two-machine and replica cases use two in-memory replicas of one
//! stream exchanging ops the way a relay would, as `the_executing_machine.rs` does.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::error::Result as NxfResult;
use nexus_chat::facade::{self, SlotRecovery, StatusOperation, StatusScope, StatusThread};
use nexus_chat::machine::{FixedMachines, MachineRef, MachineSeen, Machines};
use nexus_chat::orchestration::{
    self, coordinator_of, ConsequenceClass, CoordinatorKind, CoordinatorRef, TickReceipt,
    TickRequest,
};
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{self, ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{Timer, TimerHandle};
use nexus_chat::worker::{SessionStop, TriggerOutcome, TriggerRequest, TriggerResult, Worker};
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

/// Records every start, answers which sessions still run, and claims the three capabilities a
/// coordinator needs — each of which a test can take away.
#[derive(Default)]
struct Hand {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
    lacks: Mutex<Option<&'static str>>,
}

impl Hand {
    fn lacking(what: &'static str) -> Hand {
        Hand {
            lacks: Mutex::new(Some(what)),
            ..Hand::default()
        }
    }
    fn has(&self, what: &str) -> bool {
        *self.lacks.lock().unwrap() != Some(what)
    }
    fn all(&self) -> Vec<TriggerRequest> {
        self.seen.lock().unwrap().clone()
    }
    fn last(&self, handle: &str) -> TriggerRequest {
        self.all()
            .into_iter()
            .rev()
            .find(|r| r.role.handle == handle)
            .unwrap_or_else(|| panic!("no trigger for {handle}"))
    }
    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

impl Worker for Hand {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }
    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running.lock().unwrap().contains(internal_session)
    }
    fn answers_liveness(&self) -> bool {
        self.has("liveness")
    }
    fn stops_sessions(&self) -> bool {
        self.has("stop")
    }
    fn asks_sessions(&self) -> bool {
        self.has("ask")
    }
    fn stop_session(&self, _internal_session: &str) -> Result<SessionStop, String> {
        Ok(SessionStop::Requested)
    }
}

// ---- the timer ------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Look {
    slot: String,
    thread: String,
    due: String,
}

#[derive(Default)]
struct Book {
    looks: Mutex<Vec<Look>>,
}

impl Book {
    fn looks_at(&self, slot: &str) -> Vec<Look> {
        self.looks
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.slot == slot)
            .cloned()
            .collect()
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
        thread: &str,
        deadline: &str,
    ) -> NxfResult<TimerHandle> {
        self.looks.lock().unwrap().push(Look {
            slot: slot.to_string(),
            thread: thread.to_string(),
            due: deadline.to_string(),
        });
        Ok(TimerHandle(format!("look:{slot}")))
    }
    fn cancel(&self, _handle: &TimerHandle) -> NxfResult<()> {
        Ok(())
    }
}

// ---- the team -------------------------------------------------------------------------------------

/// A review round of two, in parallel, two hours long.
const REVIEW: &str = "name: review\nmembers: [verifier, reviewer]\nflow: parallel\ntimeout: 2h\n";
/// A coding chain whose second step is the review round.
const CODING: &str = "name: coding\nmembers: [coder, review]\nflow: sequential\ntimeout: 2h\n";
/// One verifier, two minutes long — a window that lapses inside a test.
const QUICK: &str = "name: quick\nmembers: [verifier, finisher]\nflow: sequential\ntimeout: 2m\n";

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

fn team_with(worker: Hand) -> Team {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let channels: Vec<ChannelDecl> = [REVIEW, CODING, QUICK]
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
        worker: Arc::new(worker),
        book: Arc::new(Book::default()),
        origin,
    }
}

fn team() -> Team {
    team_with(Hand::default())
}

/// One row of `owed_starts`, as the test reads it back.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    session: Option<String>,
    triggered: bool,
    coordinator: Option<String>,
    backfilled: bool,
    abandoned: Option<String>,
}

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

    fn try_commission(
        &self,
        now: &str,
        actor: &str,
        session: Option<&str>,
        to: &str,
        body: &str,
    ) -> NxfResult<surface::SendToReceipt> {
        self.with(now, actor, session, |ctx, store| {
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
    }

    /// A person, `owner`, commissions `to` at `now`.
    fn commission(&self, now: &str, to: &str, body: &str) -> surface::SendToReceipt {
        self.try_commission(now, "owner", None, to, body)
            .expect("the commission is accepted")
    }

    fn reply(
        &self,
        now: &str,
        role: &str,
        session: &str,
        thread: &str,
        body: &str,
        escalate: bool,
    ) {
        self.with(now, role, Some(session), |ctx, store| {
            surface::reply_in_thread(
                ctx,
                store,
                ReplyThreadRequest {
                    machine: None,
                    thread,
                    body,
                    escalate,
                    needs_rework: false,
                    accept: false,
                },
            )
        })
        .expect("the reply is posted");
    }

    fn tick(&self, now: &str, thread: &str) -> TickReceipt {
        self.with(now, "service", None, |ctx, store| {
            orchestration::tick(ctx, store, TickRequest { thread_id: thread })
        })
        .expect("tick")
    }

    fn coordinator(&self, now: &str, thread: &str, handle: &str) -> Option<CoordinatorRef> {
        let role = self.q(handle);
        self.with(now, "service", None, |ctx, store| {
            coordinator_of(ctx, store, thread, &role)
        })
        .expect("coordinator_of reads")
    }

    fn q(&self, handle: &str) -> String {
        format!("{}/{handle}", self.origin)
    }

    fn row(&self, thread: &str, handle: &str) -> Option<Row> {
        row_in(&self.store(), thread, &self.q(handle))
    }

    fn rows(&self) -> usize {
        rows_in(&self.store())
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

    fn op(&self, now: &str, root: &str) -> StatusOperation {
        self.status(now)
            .into_iter()
            .find(|o| o.root == root)
            .unwrap_or_else(|| panic!("no operation {root}"))
    }

    fn status_of(&self, now: &str, thread: &str) -> StatusThread {
        self.status(now)
            .into_iter()
            .flat_map(|o| o.threads)
            .find(|t| t.thread_id == thread)
            .unwrap_or_else(|| panic!("no status row for {thread}"))
    }

    fn parent(&self, thread: &str) -> String {
        self.store()
            .thread_parent(thread)
            .unwrap()
            .expect("a parent")
    }
}

fn row_in(store: &ChatStore, thread: &str, role: &str) -> Option<Row> {
    store
        .connection()
        .query_row(
            "SELECT session, triggered, coordinator, backfilled, abandoned FROM owed_starts
              WHERE thread = ?1 AND role = ?2",
            [thread, role],
            |r| {
                Ok(Row {
                    session: r.get(0)?,
                    triggered: r.get::<_, i64>(1)? != 0,
                    coordinator: r.get(2)?,
                    backfilled: r.get::<_, i64>(3)? != 0,
                    abandoned: r.get(4)?,
                })
            },
        )
        .ok()
}

fn rows_in(store: &ChatStore) -> usize {
    store
        .connection()
        .query_row("SELECT COUNT(*) FROM owed_starts", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap() as usize
}

fn slot_key(thread: &str, handle: &str) -> String {
    format!("{thread}-{handle}")
}

// ---- clause by clause ---------------------------------------------------------------------------

/// Clauses 1-5 hold for a person's `send --to <persona>`: the slot is coordinated by its ROOT, it is
/// owed a row from the open — filled with the minted session, triggered by the funnel, its
/// coordinator named — its first look is armed under the SLOT, and the sidecar is told
/// `coordinated`.
#[test]
fn a_persona_a_person_commissions_is_coordinated_by_the_root_and_owed_from_the_open() {
    let team = team();
    let sent = team.commission(T0, "verifier", "verify the build");
    let thread = sent.thread_id.clone();
    assert_eq!(
        team.coordinator(T0, &thread, "verifier"),
        Some(CoordinatorRef {
            thread: thread.clone(),
            kind: CoordinatorKind::Root,
        })
    );
    let trigger = team.worker.last("verifier");
    assert!(trigger.coordinated, "the sidecar is told the slot is owned");
    assert_eq!(
        team.row(&thread, "verifier"),
        Some(Row {
            session: Some(trigger.internal_session.clone()),
            triggered: true,
            coordinator: Some(thread.clone()),
            backfilled: false,
            abandoned: None,
        })
    );
    let looks = team.book.looks_at(&slot_key(&thread, "verifier"));
    assert!(!looks.is_empty(), "the look is filed under the slot");
    assert!(looks.iter().all(|l| l.due == at(120) && l.thread == thread));
}

/// Clause 5 reads "the first message on the slot's thread that expects the slot's role, WHOEVER
/// posted it": a session's own `nxc send` opens a coordinated slot too, coordinated by the sending
/// session's thread.
#[test]
fn a_sessions_own_send_is_coordinated_by_the_sending_sessions_thread() {
    let team = team();
    let pm_thread = team.commission(T0, "pm", "plan it").thread_id;
    let pm = team.worker.last("pm").internal_session;
    let child = team
        .try_commission(&at(5), "pm", Some(&pm), "verifier", "check the plan")
        .expect("the session's own send is accepted")
        .thread_id;
    assert_eq!(team.parent(&child), pm_thread);
    assert_eq!(
        team.coordinator(&at(5), &child, "verifier"),
        Some(CoordinatorRef {
            thread: pm_thread.clone(),
            kind: CoordinatorKind::Session,
        })
    );
    assert!(team.worker.last("verifier").coordinated);
    assert_eq!(
        team.row(&child, "verifier").and_then(|r| r.coordinator),
        Some(pm_thread)
    );
}

/// A flow step and a parallel member are coordinated by the CHANNEL thread.
#[test]
fn a_parallel_member_is_coordinated_by_its_channel_thread() {
    let team = team();
    let root = team.commission(T0, "review", "review it").thread_id;
    for handle in ["verifier", "reviewer"] {
        let slot = team.worker.last(handle).reply_thread.unwrap();
        assert_eq!(
            team.coordinator(T0, &slot, handle),
            Some(CoordinatorRef {
                thread: root.clone(),
                kind: CoordinatorKind::Channel,
            }),
            "{handle}"
        );
        assert!(team.worker.last(handle).coordinated, "{handle}");
    }
}

/// Clause 1: a slot owed by a HUMAN, or by an engine identity, has no coordinator.
#[test]
fn a_slot_owed_by_a_human_or_an_engine_identity_has_no_coordinator() {
    let team = team();
    let thread = team.commission(T0, "verifier", "verify").thread_id;
    for owed_by in ["owner", "__synth__"] {
        let mut store = team.store();
        let expects = serde_json::to_string(&[team.q(owed_by)]).unwrap();
        store.set_expects_reply_from(&thread, &expects, &team.q("owner"));
        drop(store);
        assert_eq!(
            team.coordinator(&at(1), &thread, owed_by),
            None,
            "{owed_by}"
        );
    }
}

/// Clause 2: an INBOUND border thread is coordinated on the side that commissioned it, never here;
/// an escalated slot and one whose answer is in (awaiting the human) are not open for the role.
#[test]
fn a_border_inbound_an_escalated_and_an_answered_slot_have_no_coordinator() {
    let team = team();
    let thread = team.commission(T0, "verifier", "verify").thread_id;
    assert!(team.coordinator(T0, &thread, "verifier").is_some());
    team.sql(&format!(
        "INSERT INTO border_threads(thread_id, direction, peer, persona, state, created, updated)
         VALUES ('{thread}', 'in', 'test/other', 'verifier', 'working', '{T0}', '{T0}')"
    ));
    assert_eq!(
        team.coordinator(T0, &thread, "verifier"),
        None,
        "border inbound"
    );

    let escalated = team.commission(&at(1), "reviewer", "review").thread_id;
    let s = team.worker.last("reviewer").internal_session;
    team.reply(&at(2), "reviewer", &s, &escalated, "I cannot", true);
    assert_eq!(
        team.coordinator(&at(2), &escalated, "reviewer"),
        None,
        "escalated"
    );

    let answered = team.commission(&at(3), "coder", "build").thread_id;
    let c = team.worker.last("coder").internal_session;
    team.reply(&at(4), "coder", &c, &answered, "built", false);
    assert!(team.status_of(&at(4), &answered).awaiting_human);
    assert_eq!(
        team.coordinator(&at(4), &answered, "coder"),
        None,
        "awaiting the human"
    );
}

/// Clause 4 (must-fix, design review round 2 IR #4): coordinated only when the worker on this device
/// answers liveness, stops sessions AND asks them. Without any one of the three the slot keeps
/// today's behaviour: no coordinator, the sidecar keeps its substitute, and the row cut 1a wrote
/// carries no coordinator.
#[test]
fn a_worker_without_liveness_stop_or_ask_coordinates_nothing() {
    for missing in ["liveness", "stop", "ask"] {
        let team = team_with(Hand::lacking(missing));
        let thread = team.commission(T0, "verifier", "verify").thread_id;
        assert_eq!(team.coordinator(T0, &thread, "verifier"), None, "{missing}");
        assert!(!team.worker.last("verifier").coordinated, "{missing}");
        let row = team.row(&thread, "verifier").expect("cut 1a's row stands");
        assert_eq!(row.coordinator, None, "{missing}");
    }
}

/// …and such a slot's window lapses as it did: a default custom worker's stale step is advanced
/// past at its window, the next step started.
#[test]
fn a_default_custom_workers_stale_slot_lapses_as_today() {
    let team = team_with(Hand::lacking("liveness"));
    let root = team.commission(T0, "quick", "verify").thread_id;
    assert!(!team.worker.last("verifier").coordinated);
    let r = team.tick(&at(130), &root);
    assert!(
        r.warnings
            .iter()
            .any(|f| f.class == ConsequenceClass::LivenessUnknown),
        "{:?}",
        r.warnings.iter().map(|f| f.class).collect::<Vec<_>>()
    );
    assert_eq!(
        team.worker
            .all()
            .iter()
            .filter(|t| t.role.handle == "finisher")
            .count(),
        1,
        "the window lapsed and the flow advanced, as before"
    );
}

/// Clause 5: a run that no longer has its commissioning message is not coordinated.
#[test]
fn a_run_without_its_commissioning_message_is_not_coordinated() {
    let team = team();
    let mut store = team.store();
    let thread = store.open_new_thread(
        &nexus_chat::model::ThreadRoot {
            origin: team.origin.clone(),
            channel_id: "dm".into(),
            opener: team.q("owner"),
            created: T0.into(),
            parent: None,
        },
        &team.q("owner"),
    );
    let expects = serde_json::to_string(&[team.q("verifier")]).unwrap();
    store.set_expects_reply_from(&thread, &expects, &team.q("owner"));
    drop(store);
    assert_eq!(team.coordinator(T0, &thread, "verifier"), None);
}

/// Clause 6: the slot that stands for a nested round is coordinated while any open slot inside the
/// round is — and only then.
#[test]
fn a_nested_round_is_coordinated_while_a_slot_inside_it_is() {
    let team = team();
    let coding = team.commission(T0, "coding", "build and review").thread_id;
    let coder = team.worker.last("coder");
    team.reply(
        &at(10),
        "coder",
        &coder.internal_session,
        coder.reply_thread.as_deref().unwrap(),
        "built",
        false,
    );
    let verifier = team.worker.last("verifier");
    let reviewer = team.worker.last("reviewer");
    let round = team.parent(verifier.reply_thread.as_deref().unwrap());
    assert_eq!(team.parent(&round), coding);
    assert_eq!(
        team.coordinator(&at(11), &round, "__channel__"),
        Some(CoordinatorRef {
            thread: coding.clone(),
            kind: CoordinatorKind::NestedRound,
        })
    );
    team.reply(
        &at(12),
        "verifier",
        &verifier.internal_session,
        verifier.reply_thread.as_deref().unwrap(),
        "verified",
        false,
    );
    assert!(
        team.coordinator(&at(12), &round, "__channel__").is_some(),
        "one member is still owed"
    );
    // The round's last member escalates: nothing inside it is open any more.
    team.reply(
        &at(13),
        "reviewer",
        &reviewer.internal_session,
        reviewer.reply_thread.as_deref().unwrap(),
        "I cannot",
        true,
    );
    assert_eq!(team.coordinator(&at(13), &round, "__channel__"), None);
}

// ---- the row at the open (field case 3) -----------------------------------------------------------

/// **Field case 3** at the record level: a review-round slot whose MINT fails. The row is there from
/// the open with no session, its look armed under the slot; the sweep judges it `never_started` at
/// the clock + 180 s and its coordinator re-commissions it (cut 1d's rung D, which replaced cut
/// 1a's escalation here); and the round's own slot reads coordinated while it is owed (clause 6).
#[test]
fn a_review_slot_whose_mint_fails_is_owed_with_no_session_and_recommissioned_when_never_started() {
    let team = team();
    team.sql(
        "CREATE TRIGGER no_verifier_mint BEFORE INSERT ON session_map WHEN NEW.role = 'verifier'
         BEGIN SELECT RAISE(ABORT, 'the session could not be minted (injected)'); END;",
    );
    let refused = team.try_commission(T0, "owner", None, "review", "review it");
    assert!(
        refused.is_err(),
        "the channel's spawn failure is propagated"
    );
    team.sql("DROP TRIGGER no_verifier_mint;");

    let root = team
        .status(T0)
        .into_iter()
        .find(|o| {
            o.channel_id
                .as_deref()
                .is_some_and(|c| c.contains("review"))
        })
        .expect("the round's operation")
        .root;
    let slot = team
        .status(T0)
        .into_iter()
        .flat_map(|o| o.threads)
        .find(|t| t.expects == vec![team.q("verifier")])
        .expect("the verifier's slot")
        .thread_id;
    assert_eq!(
        team.row(&slot, "verifier"),
        Some(Row {
            session: None,
            triggered: false,
            coordinator: Some(root.clone()),
            backfilled: false,
            abandoned: None,
        }),
        "owed from the open, before any session"
    );
    assert_eq!(
        team.book.looks_at(&slot_key(&slot, "verifier")),
        vec![Look {
            slot: slot_key(&slot, "verifier"),
            thread: root.clone(),
            due: at(120),
        }]
    );
    assert_eq!(
        team.coordinator(&at(1), &root, "__channel__")
            .map(|c| c.kind),
        Some(CoordinatorKind::NestedRound),
        "the round's slot is coordinated while its never-started member is"
    );

    team.tick(&at(120), &root);
    assert!(
        team.status_of(&at(120), &slot).stranded.is_none(),
        "one look is not a record"
    );
    team.tick(&at(180), &root);
    // Judged `never_started` at +180 s — and since cut 1d its coordinator RE-COMMISSIONS it (rung
    // D, `the_ladder.rs`) instead of recording and escalating it: a fresh session into the slot.
    let row = team.status_of(&at(180), &slot);
    assert!(row.stranded.is_none(), "no record, no escalation: {row:?}");
    let fresh = team.worker.last("verifier");
    assert_eq!(fresh.reply_thread.as_deref(), Some(slot.as_str()));
    assert_eq!(
        team.row(&slot, "verifier").and_then(|r| r.session),
        Some(fresh.internal_session.clone()),
        "the slot's row names the new session"
    );
}

// ---- ownership across machines (must-fix, design review round 2 CQ #2 / TQ #2 / IR #3) -------------

fn laptop() -> MachineRef {
    MachineRef {
        machine_id: "01jlaptop".into(),
        name: "laptop".into(),
    }
}

fn studio() -> MachineRef {
    MachineRef {
        machine_id: "01jstudio".into(),
        name: "studio".into(),
    }
}

fn both_online() -> Vec<MachineSeen> {
    [laptop(), studio()]
        .into_iter()
        .map(|m| MachineSeen {
            machine_id: m.machine_id,
            name: m.name,
            online: true,
            age_secs: 5,
        })
        .collect()
}

struct Replica {
    store: ChatStore,
    /// `None` is a host that names no machine (a workspace that syncs nowhere designates nothing).
    machines: Option<FixedMachines>,
    worker: Hand,
    book: Book,
    db: &'static str,
}

impl Replica {
    fn new(site: i64, here: MachineRef, db: &'static str) -> Replica {
        Replica {
            store: ChatStore::open_in_memory(site),
            machines: Some(FixedMachines::new(Some(here), Ok(both_online()))),
            worker: Hand::default(),
            book: Book::default(),
            db,
        }
    }

    fn run<R>(
        &mut self,
        defs: &Definitions,
        now: &str,
        actor: &str,
        f: impl FnOnce(&orchestration::Ctx, &mut ChatStore) -> R,
    ) -> R {
        let ctx = orchestration::Ctx {
            now,
            origin: "o",
            actor,
            session: None,
            hop: 0,
            defs,
            worker: &self.worker,
            timer: &self.book,
            namer: &nexus_chat::naming::DryNamer,
            db_path: self.db,
            project_claude_md: None,
            module_primes: None,
            machines: self.machines.as_ref().map(|m| m as &dyn Machines),
            peers: None,
        };
        f(&ctx, &mut self.store)
    }

    fn tick(&mut self, defs: &Definitions, now: &str, thread: &str) {
        self.run(defs, now, "service", |ctx, store| {
            orchestration::tick(ctx, store, TickRequest { thread_id: thread })
        })
        .expect("tick");
    }
}

/// `to` pulls everything `from` holds, trusting its key — `nxs sync trust add` and one pass.
fn pull(from: &ChatStore, to: &mut ChatStore) {
    to.trust_key(from.key_id(), "peer", T0).unwrap();
    to.apply(&from.export());
}

fn coder_only() -> Definitions {
    Definitions::new(vec![role("coder")], vec![]).unwrap()
}

/// A chat designated to the studio: exactly one row and one trigger, both on the studio, none on the
/// laptop that wrote it (the opener side keeps no row and is told nothing is coordinated there).
/// Handed back to the laptop, the next look runs on the laptop only — the studio forgets its row.
#[test]
fn a_chat_designated_to_another_machine_is_owed_there_only_and_a_hand_over_back_moves_it() {
    let d = coder_only();
    let mut lap = Replica::new(1, laptop(), "/laptop.db");
    let mut stu = Replica::new(2, studio(), "/studio.db");
    let thread = lap
        .run(&d, T0, "alice", |ctx, store| {
            surface::send_to(
                ctx,
                store,
                SendToRequest {
                    machine: Some("studio"),
                    to: "coder",
                    body: "Add a --since flag.",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
        })
        .unwrap()
        .thread_id;
    assert_eq!(lap.worker.count(), 0);
    assert_eq!(
        rows_in(&lap.store),
        0,
        "the laptop opened it and owes nothing"
    );

    pull(&lap.store, &mut stu.store);
    let served = stu
        .run(&d, &at(10), "service", |ctx, store| {
            orchestration::pick_up(ctx, store)
        })
        .unwrap();
    assert_eq!(served.served.len(), 1, "{served:?}");
    assert_eq!(stu.worker.count(), 1);
    assert!(
        stu.worker.last("coder").coordinated,
        "the executing side owns it"
    );
    assert_eq!(
        row_in(&stu.store, &thread, "o/coder").and_then(|r| r.coordinator),
        Some(thread.clone())
    );

    // The laptop looks — its first sweep runs the backfill too — and still owes nothing: it does not
    // execute the slot.
    pull(&stu.store, &mut lap.store);
    lap.tick(&d, &at(20), &thread);
    assert_eq!(rows_in(&lap.store), 0);
    assert!(lap
        .run(&d, &at(20), "service", |ctx, store| coordinator_of(
            ctx, store, &thread, "o/coder"
        ))
        .unwrap()
        .is_none());

    // Handed back: alice asks for it on the laptop.
    lap.run(&d, &at(30), "alice", |ctx, store| {
        surface::reply_in_thread(
            ctx,
            store,
            ReplyThreadRequest {
                machine: Some("laptop"),
                thread: &thread,
                body: "carry on here",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
    })
    .unwrap();
    assert_eq!(lap.worker.count(), 1, "the laptop starts it now");
    assert!(lap.worker.last("coder").coordinated);
    assert!(row_in(&lap.store, &thread, "o/coder").is_some());

    // The studio's next look: it no longer executes the slot, so its row goes and nothing is
    // recorded there; the laptop's look keeps its own.
    pull(&lap.store, &mut stu.store);
    stu.tick(&d, &at(40), &thread);
    assert_eq!(rows_in(&stu.store), 0, "the studio forgot its row");
    lap.tick(&d, &at(40), &thread);
    assert!(row_in(&lap.store, &thread, "o/coder").is_some());
    assert_eq!(stu.worker.count(), 1, "nothing more started on the studio");
}

/// **A coordinated row is not forgotten for want of a machine read** (nxf 6j6v.ndbr, the kgfn
/// review's Integrity #1): a chat designated to the studio and executed there has its row with a
/// coordinator. The studio's next look cannot say which machine it is — no machine read — and the
/// row SURVIVES, still naming its coordinator: the sidecar posted no substitute for this slot, so
/// forgetting the row would leave it with neither. Once the slot is KNOWN to run elsewhere (the hand-over
/// back to the laptop), the row goes, and that is said.
#[test]
fn a_coordinated_row_survives_a_look_that_cannot_read_the_machine() {
    let d = coder_only();
    let mut lap = Replica::new(1, laptop(), "/laptop.db");
    let mut stu = Replica::new(2, studio(), "/studio.db");
    let thread = lap
        .run(&d, T0, "alice", |ctx, store| {
            surface::send_to(
                ctx,
                store,
                SendToRequest {
                    machine: Some("studio"),
                    to: "coder",
                    body: "Add a --since flag.",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
        })
        .unwrap()
        .thread_id;
    pull(&lap.store, &mut stu.store);
    stu.run(&d, &at(10), "service", |ctx, store| {
        orchestration::pick_up(ctx, store)
    })
    .unwrap();
    let before = row_in(&stu.store, &thread, "o/coder").expect("the studio's row");
    assert_eq!(before.coordinator.as_deref(), Some(thread.as_str()));

    // The machine read is gone.
    stu.machines = None;
    stu.tick(&d, &at(20), &thread);
    let after = row_in(&stu.store, &thread, "o/coder").expect("the row survives the look");
    assert_eq!(after.coordinator, before.coordinator, "still coordinated");

    // Back with a read, and the chat handed to the laptop: now it is KNOWN to run elsewhere.
    stu.machines = Some(FixedMachines::new(Some(studio()), Ok(both_online())));
    lap.run(&d, &at(30), "alice", |ctx, store| {
        surface::reply_in_thread(
            ctx,
            store,
            ReplyThreadRequest {
                machine: Some("laptop"),
                thread: &thread,
                body: "carry on here",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
    })
    .unwrap();
    pull(&lap.store, &mut stu.store);
    let r = stu
        .run(&d, &at(40), "service", |ctx, store| {
            orchestration::tick(ctx, store, TickRequest { thread_id: &thread })
        })
        .expect("tick");
    assert!(
        row_in(&stu.store, &thread, "o/coder").is_none(),
        "forgotten"
    );
    assert!(
        r.warnings
            .iter()
            .any(|f| f.detail.contains("no longer coordinated on this device")),
        "and said: {:?}",
        r.warnings.iter().map(|f| &f.detail).collect::<Vec<_>>()
    );
}

/// A replica of a stream another site wrote the expectation in writes NO row for that slot — not at
/// the open (it opened nothing), not at the backfill.
#[test]
fn a_replica_writes_no_row_for_a_slot_another_site_opened() {
    let d = coder_only();
    let mut here = Replica::new(1, laptop(), "/laptop.db");
    let mut replica = Replica::new(2, laptop(), "/clone.db");
    // A host that names no machine: the expectation's site is what tells the two apart.
    here.machines = None;
    replica.machines = None;
    let thread = here
        .run(&d, T0, "alice", |ctx, store| {
            surface::send_to(
                ctx,
                store,
                SendToRequest {
                    machine: None,
                    to: "coder",
                    body: "go",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
        })
        .unwrap()
        .thread_id;
    assert!(row_in(&here.store, &thread, "o/coder").is_some());
    pull(&here.store, &mut replica.store);
    replica.tick(&d, &at(10), &thread);
    assert_eq!(rows_in(&replica.store), 0, "zero rows on the replica");
    assert_eq!(replica.worker.count(), 0, "and nothing started there");
}

// ---- the backfill, and abandoned (must-fix, design review round 2 CQ #7 and IR #9) ----------------

/// Stage slots as a store from before this cut holds them: opened with a worker that cannot
/// coordinate (so no slot row is written at the open), then the row cut 1a would have had removed.
/// The first sweep of a worker that CAN coordinate backfills them.
fn pre_cutover(team: &Team) {
    team.sql("DELETE FROM owed_starts; DELETE FROM owed_backfill_done;");
}

#[test]
fn the_backfill_records_each_pre_cutover_specimen_with_its_reason_and_triggers_none() {
    let team = team_with(Hand::lacking("ask"));
    // (a) a slot whose operation's root was DELIVERED while the slot the pm's own session opened
    // below it still owes — the register as a delivery leaves it: retargeted to `__delivered__`.
    let pm_root = team.commission(T0, "pm", "plan it").thread_id;
    let pm = team.worker.last("pm").internal_session;
    let orphan = team
        .try_commission(&at(1), "pm", Some(&pm), "verifier", "check")
        .unwrap()
        .thread_id;
    let mut store = team.store();
    store.set_expects_reply_from(
        &pm_root,
        &serde_json::to_string(&[team.q("__delivered__")]).unwrap(),
        &team.q("__channel__"),
    );
    drop(store);
    // (b) one aged 86 401 s with its root still open, (c) one aged 86 399 s.
    let aged = team.commission(&at(0), "coder", "build").thread_id;
    let fresh = team.commission(&at(2), "reviewer", "review").thread_id;
    // (d) a run with no commissioning message.
    let mut store = team.store();
    let consolidated = store.open_new_thread(
        &nexus_chat::model::ThreadRoot {
            origin: team.origin.clone(),
            channel_id: "dm".into(),
            opener: team.q("owner"),
            created: T0.into(),
            parent: None,
        },
        &team.q("owner"),
    );
    store.set_expects_reply_from(
        &consolidated,
        &serde_json::to_string(&[team.q("finisher")]).unwrap(),
        &team.q("owner"),
    );
    drop(store);
    pre_cutover(&team);
    let triggers_before = team.worker.count();

    *team.worker.lacks.lock().unwrap() = None;
    let now = at(86_401);
    let r = team.tick(&now, &aged);

    let reason = |thread: &str, handle: &str| team.row(thread, handle).and_then(|r| r.abandoned);
    assert_eq!(reason(&orphan, "verifier").as_deref(), Some("delivered"));
    assert_eq!(reason(&aged, "coder").as_deref(), Some("aged"));
    assert_eq!(
        reason(&fresh, "reviewer"),
        None,
        "86 399 s is not abandoned"
    );
    assert_eq!(
        reason(&consolidated, "finisher").as_deref(),
        Some("consolidated")
    );
    for (thread, handle) in [
        (&orphan, "verifier"),
        (&aged, "coder"),
        (&fresh, "reviewer"),
    ] {
        assert!(team.row(thread, handle).unwrap().backfilled, "{handle}");
    }
    let abandoned_findings = r
        .warnings
        .iter()
        .filter(|f| f.class == ConsequenceClass::SlotAbandoned)
        .count();
    assert_eq!(abandoned_findings, 3, "one finding per abandoned slot");
    assert_eq!(team.worker.count(), triggers_before, "never triggered");

    // Rendered, and never nudged: a later sweep records nothing on them and escalates nothing.
    let row = team.status_of(&now, &aged);
    let abandoned = row.abandoned.expect("status renders it");
    assert_eq!(abandoned.reason, "aged");
    let later = at(86_401 + 600);
    team.tick(&later, &aged);
    assert!(
        team.status_of(&later, &aged).stranded.is_none(),
        "never nudged"
    );
    assert!(team.status_of(&later, &orphan).stranded.is_none());
    assert_eq!(team.worker.count(), triggers_before);
}

/// A backfilled slot whose role has a session standing on the thread keeps THAT session id.
#[test]
fn a_backfilled_slot_keeps_the_session_standing_on_its_thread() {
    let team = team_with(Hand::lacking("ask"));
    let thread = team.commission(T0, "verifier", "verify").thread_id;
    let session = team.worker.last("verifier").internal_session;
    pre_cutover(&team);
    *team.worker.lacks.lock().unwrap() = None;
    team.tick(&at(60), &thread);
    let row = team.row(&thread, "verifier").expect("backfilled");
    assert!(row.backfilled);
    assert_eq!(row.session, Some(session));
    assert_eq!(row.abandoned, None);
}

/// The backfill runs ONCE: a slot that lost its row after the marker was written is not backfilled
/// again — and a worker that cannot coordinate never burns the marker.
#[test]
fn the_backfill_runs_once_and_only_where_slots_can_be_coordinated() {
    let team = team_with(Hand::lacking("stop"));
    let thread = team.commission(T0, "verifier", "verify").thread_id;
    pre_cutover(&team);
    team.tick(&at(10), &thread);
    let marker = |team: &Team| -> i64 {
        team.store()
            .connection()
            .query_row("SELECT COUNT(*) FROM owed_backfill_done", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(
        marker(&team),
        0,
        "an uncoordinating worker writes no marker"
    );
    *team.worker.lacks.lock().unwrap() = None;
    team.tick(&at(20), &thread);
    assert_eq!(marker(&team), 1);
    assert_eq!(team.rows(), 1);
    team.sql("DELETE FROM owed_starts;");
    team.tick(&at(30), &thread);
    assert_eq!(team.rows(), 0, "not backfilled a second time");
}

// ---- hangs_at ---------------------------------------------------------------------------------------

/// `hangs_at` names the DEEPEST slot of a three-level chain that is escalated or recorded, with its
/// coordinator — not the round above it, which hangs too once its whole set has escalated.
#[test]
fn hangs_at_names_the_deepest_slot_of_a_three_level_chain() {
    let team = team();
    let coding = team.commission(T0, "coding", "build and review").thread_id;
    let coder = team.worker.last("coder");
    team.reply(
        &at(10),
        "coder",
        &coder.internal_session,
        coder.reply_thread.as_deref().unwrap(),
        "built",
        false,
    );
    let verifier = team.worker.last("verifier");
    let reviewer = team.worker.last("reviewer");
    let deep = verifier.reply_thread.clone().unwrap();
    let round = team.parent(&deep);
    assert!(
        team.op(&at(11), &coding).hangs_at.is_none(),
        "nothing hangs while every slot is merely running"
    );
    team.reply(
        &at(12),
        "verifier",
        &verifier.internal_session,
        &deep,
        "I cannot",
        true,
    );
    let r_slot = reviewer.reply_thread.clone().unwrap();
    team.reply(
        &at(14),
        "reviewer",
        &reviewer.internal_session,
        &r_slot,
        "nor I",
        true,
    );
    let rows = team.op(&at(15), &coding);
    let round_row = rows.threads.iter().find(|t| t.thread_id == round).unwrap();
    assert!(
        round_row.escalated,
        "the round above hangs too — so the depth is what decides"
    );
    let hangs = rows.hangs_at.expect("it hangs");
    assert_eq!(
        hangs.thread, deep,
        "the deepest slot (and the older of the two there)"
    );
    assert_eq!(hangs.role, team.q("verifier"));
    assert_eq!(hangs.coordinator.as_deref(), Some(round.as_str()));
    assert_eq!(hangs.recovery, SlotRecovery::Watched);
}

/// The tie rule at one depth (must-fix, design review round 3 TQ #11): the oldest last activity
/// first, then the lowest thread id.
#[test]
fn hangs_at_breaks_a_tie_by_the_oldest_activity_then_the_lowest_thread_id() {
    let team = team();
    let root = team.commission(T0, "review", "review it").thread_id;
    let verifier = team.worker.last("verifier");
    let reviewer = team.worker.last("reviewer");
    let (v_slot, r_slot) = (
        verifier.reply_thread.clone().unwrap(),
        reviewer.reply_thread.clone().unwrap(),
    );
    // Both escalate; the REVIEWER's last activity is the older one.
    team.reply(
        &at(20),
        "reviewer",
        &reviewer.internal_session,
        &r_slot,
        "no",
        true,
    );
    team.reply(
        &at(30),
        "verifier",
        &verifier.internal_session,
        &v_slot,
        "no",
        true,
    );
    let hangs = team.op(&at(31), &root).hangs_at.expect("it hangs");
    assert_eq!(
        hangs.thread, r_slot,
        "the oldest last activity wins the tie"
    );

    // Same activity instant on both: the lower thread id wins.
    let team = team_with(Hand::default());
    let root = team.commission(T0, "review", "review it").thread_id;
    let verifier = team.worker.last("verifier");
    let reviewer = team.worker.last("reviewer");
    let (v_slot, r_slot) = (
        verifier.reply_thread.clone().unwrap(),
        reviewer.reply_thread.clone().unwrap(),
    );
    team.reply(
        &at(20),
        "reviewer",
        &reviewer.internal_session,
        &r_slot,
        "no",
        true,
    );
    team.reply(
        &at(20),
        "verifier",
        &verifier.internal_session,
        &v_slot,
        "no",
        true,
    );
    let hangs = team.op(&at(21), &root).hangs_at.expect("it hangs");
    assert_eq!(hangs.thread, std::cmp::min(v_slot, r_slot));
}
