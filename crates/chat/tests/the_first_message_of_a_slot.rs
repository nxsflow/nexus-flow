//! **The first message of a slot** (nxf 6j6v.558h, epic 6j6v.nmc9 section B, field case 4).
//!
//! The field case (this workspace, 2026-10-23, 0.218.0): a planning round's pm step waited behind
//! the working-copy lease at queue position 1. The pm persona runs ONE real SDK session across its
//! threads, and between the queueing and the release that session worked on other threads. At the
//! release it got a two-turn, seven-second wake for the queued slot and answered "this session was
//! never handed this thread's task … please resend the commission" — and the chain delivered that
//! as the round's result.
//!
//! The rule: whichever path starts a slot's session, the first message that session receives FOR
//! THAT SLOT carries the slot's commissioning body, and a queued entry fires with `reply_thread` and
//! the obligation it was queued with. These tests stage the field's shape — a queued entry whose
//! internal session is the persona's shared one, bound and standing on another thread in between —
//! and read what the worker was handed.
//!
//! **Written to compile against the code before the cut**, because the design (round 3 IR #6, round
//! 4 TQ #5) asks for case 4 to be reproduced RED on today's code first.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::{self, Caller, TickRequest};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::{DryTimer, TimerConfig};
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-10-23T16:30:00Z";
const BETWEEN: &str = "2026-10-23T16:40:00Z";
const RELEASED: &str = "2026-10-23T16:49:47Z";
const ANSWERED: &str = "2026-10-23T16:52:00Z";

const HOLDER: &str =
    "name: holding\nmembers: [builder]\nflow: sequential\nworking_tree: exclusive\n";
const PLANNING: &str =
    "name: planning\nmembers: [pm]\nflow: sequential\nworking_tree: exclusive\ntimeout: 2h\n";

/// The planning round's own words — what the pm's first message for its slot must carry.
const ROUND_3: &str = "Round 3 of the design: rework section B after the review.";

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
}

impl Recorder {
    fn of(&self, handle: &str) -> Vec<TriggerRequest> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.role.handle == handle)
            .cloned()
            .collect()
    }
}

impl Worker for Recorder {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        // One internal session, one process — the sidecar's own guard, which is what makes the
        // engine HOLD an answer for a session that is still mid-turn.
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
        true
    }
}

fn role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

fn team() -> (TempDir, Engine, Arc<Recorder>) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let channels: Vec<ChannelDecl> = [HOLDER, PLANNING]
        .iter()
        .map(|y| serde_yaml::from_str(y).expect("channel parses"))
        .collect();
    let defs = Definitions::new(vec![role("builder"), role("pm")], channels).expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let worker = Arc::new(Recorder::default());
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker.clone()),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    (tmp, engine, worker)
}

fn caller<'a>(actor: &'a str, now: &'a str) -> Caller<'a> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(now),
    }
}

fn as_session<'a>(session: &'a str, now: &'a str) -> Caller<'a> {
    Caller {
        session: Some(session),
        actor: None,
        now: Some(now),
    }
}

fn commission(engine: &Engine, who: Caller<'_>, to: &str, body: &str) -> String {
    engine
        .send_to(
            who,
            SendToRequest {
                machine: None,
                to,
                body,
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the commission is accepted")
        .thread_id
}

fn answer(engine: &Engine, session: &str, thread: &str, now: &str, body: &str) {
    engine
        .reply_thread(
            as_session(session, now),
            ReplyThreadRequest {
                machine: None,
                thread,
                body,
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply is posted");
}

fn store(tmp: &TempDir) -> nexus_chat::store::ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

fn tick(tmp: &TempDir, worker: &Recorder, now: &str, thread_id: &str) {
    let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
    let db_path = ws.db_path_str().expect("db path");
    let origin = nexus_chat::workspace::origin_of(&ws).to_string();
    let mut store = ws.open_chat_store().expect("open chat store");
    let defs = Definitions::resolve(tmp.path()).expect("declarations");
    let ctx = orchestration::Ctx {
        now,
        origin: &origin,
        actor: "service",
        session: None,
        hop: 0,
        defs: &defs,
        worker,
        timer: &DryTimer,
        namer: &nexus_chat::naming::DryNamer,
        db_path: &db_path,
        project_claude_md: None,
        module_primes: None,
        machines: None,
        peers: None,
        liveness: Default::default(),
    };
    orchestration::tick(&ctx, &mut store, TickRequest { thread_id }).expect("tick");
}

/// The field's shape, up to the release: the persona's shared session `S` is bound and standing on
/// another thread; the planning round's pm step waits at queue position 1, and its queued entry
/// names `S`. Returns (the round's root, the pm slot, S, the other thread, the holder's session and
/// thread).
fn staged(tmp: &TempDir, engine: &Engine, worker: &Recorder) -> (String, String, String, String) {
    // The pm persona's session, at work on another conversation.
    let other = commission(engine, caller("carsten", NOW), "pm", "Plan the a0y8 work.");
    let shared = worker.of("pm")[0].internal_session.clone();
    let mut s = store(tmp);
    s.bind_session(&shared, "0104b3f5-real").unwrap();
    drop(s);
    answer(engine, &shared, &other, BETWEEN, "a0y8 is planned");

    // Something holds the working copy.
    commission(engine, caller("carsten", NOW), "holding", "build T5");
    let builder = worker.of("builder")[0].clone();

    // The planning round waits behind it, at position 1.
    let root = commission(engine, caller("carsten", NOW), "planning", ROUND_3);
    let queue = store(tmp).list_working_tree_queue().unwrap();
    assert_eq!(queue.len(), 1, "the premise: one entry waits: {queue:?}");
    assert_eq!(queue[0].role, "pm");
    let slot = queue[0]
        .thread
        .clone()
        .expect("the queued pm names its slot");
    // …and its entry names the persona's SHARED session, which stands on the other thread.
    store(tmp)
        .connection()
        .execute(
            "UPDATE working_tree_queue SET session = ?1 WHERE thread = ?2",
            [&shared, &slot],
        )
        .unwrap();
    let mut s = store(tmp);
    s.record_session_thread(&shared, Some(&other)).unwrap();
    drop(s);

    // The holder lets go.
    answer(
        engine,
        &builder.internal_session,
        builder.reply_thread.as_deref().unwrap(),
        RELEASED,
        "built",
    );
    engine
        .session_ended(caller("builder", RELEASED), &builder.internal_session)
        .expect("the session end is accepted");
    tick(
        tmp,
        worker,
        RELEASED,
        builder.reply_thread.as_deref().unwrap(),
    );
    (root, slot, shared, other)
}

/// **Case 4, staged**: the queued entry fires into the shared session WITH the slot's body and with
/// the slot as its reply thread, and the session's answer is the round's result.
#[test]
fn a_queued_commission_into_a_shared_session_fires_with_its_body_and_its_slot() {
    let (tmp, engine, worker) = team();
    let (root, slot, shared, _other) = staged(&tmp, &engine, &worker);
    let fired: Vec<TriggerRequest> = worker
        .of("pm")
        .into_iter()
        .filter(|r| r.internal_session == shared && r.thread_is(&slot))
        .collect();
    assert_eq!(
        fired.len(),
        1,
        "fired once for the slot: {:?}",
        worker.of("pm")
    );
    let first = &fired[0];
    assert!(
        first.message.contains(ROUND_3),
        "the first message for the slot carries its body:\n{}",
        first.message
    );
    assert_eq!(
        first.reply_thread.as_deref(),
        Some(slot.as_str()),
        "and names the slot"
    );
    answer(&engine, &shared, &slot, ANSWERED, "section B reworked");
    // The round is delivered, and what it delivers is the slot's answer.
    let root_bodies: Vec<String> = store(&tmp)
        .messages_in_thread(&root)
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect();
    assert!(
        root_bodies.iter().any(|b| b.contains("delivered")),
        "the round is delivered: {root_bodies:?}"
    );
    let slot_answer = store(&tmp)
        .messages_in_thread(&slot)
        .unwrap()
        .into_iter()
        .last()
        .unwrap();
    assert_eq!(
        slot_answer.body, "section B reworked",
        "and its result is the slot's answer"
    );
}

/// The second candidate cause the design names, in order: an answer HELD for the shared session
/// (from the other thread) is waiting when the copy is released — the held delivery and the queued
/// slot both want the session. The first message the session gets FOR THE SLOT still carries the
/// slot's body.
#[test]
fn a_held_delivery_for_the_shared_session_does_not_take_the_slots_first_message() {
    let (tmp, engine, worker) = team();
    let (_root, slot, shared, other) = {
        // Stage up to, but not including, the release, with an answer held for the shared session.
        let other = commission(&engine, caller("carsten", NOW), "pm", "Plan the a0y8 work.");
        let shared = worker.of("pm")[0].internal_session.clone();
        let mut s = store(&tmp);
        s.bind_session(&shared, "0104b3f5-real").unwrap();
        drop(s);
        worker.running.lock().unwrap().insert(shared.clone());
        let consult = engine
            .send_to(
                as_session(&shared, BETWEEN),
                SendToRequest {
                    machine: None,
                    to: "builder",
                    body: "Is T5 still building?",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("the session's own send")
            .thread_id;
        let asked = worker
            .of("builder")
            .last()
            .unwrap()
            .internal_session
            .clone();
        answer(&engine, &asked, &consult, BETWEEN, "yes, still building");
        assert!(
            !store(&tmp).held_wakes(&shared).unwrap().is_empty(),
            "the premise: an answer is held for the shared session"
        );
        commission(&engine, caller("carsten", NOW), "holding", "build T5");
        let builder = worker.of("builder").last().unwrap().clone();
        let root = commission(&engine, caller("carsten", NOW), "planning", ROUND_3);
        let slot = store(&tmp).list_working_tree_queue().unwrap()[0]
            .thread
            .clone()
            .unwrap();
        store(&tmp)
            .connection()
            .execute(
                "UPDATE working_tree_queue SET session = ?1 WHERE thread = ?2",
                [&shared, &slot],
            )
            .unwrap();
        worker.running.lock().unwrap().remove(&shared);
        answer(
            &engine,
            &builder.internal_session,
            builder.reply_thread.as_deref().unwrap(),
            RELEASED,
            "built",
        );
        engine
            .session_ended(caller("builder", RELEASED), &builder.internal_session)
            .expect("the session end is accepted");
        tick(
            &tmp,
            &worker,
            RELEASED,
            builder.reply_thread.as_deref().unwrap(),
        );
        (root, slot, shared, other)
    };
    let _ = other;
    let for_the_slot: Vec<TriggerRequest> = worker
        .of("pm")
        .into_iter()
        .filter(|r| {
            r.internal_session == shared && r.reply_thread.as_deref() == Some(slot.as_str())
        })
        .collect();
    assert!(
        !for_the_slot.is_empty(),
        "the slot was fired: {:?}",
        worker.of("pm")
    );
    assert!(
        for_the_slot[0].message.contains(ROUND_3),
        "the first message FOR THE SLOT carries its body:\n{}",
        for_the_slot[0].message
    );
}

trait ThreadIs {
    fn thread_is(&self, thread: &str) -> bool;
}

impl ThreadIs for TriggerRequest {
    /// Whether this trigger serves `thread` — by its reply thread, or, for a trigger that owes
    /// nothing, by the body naming it.
    fn thread_is(&self, thread: &str) -> bool {
        self.reply_thread.as_deref() == Some(thread) || self.message.contains(thread)
    }
}

/// **The pin: a queued entry fires with the obligation it was QUEUED with** (nxf 6j6v.558h) — not
/// one re-derived at the release. Between the queueing and the release the slot's expectation is
/// re-recorded with the role's BARE handle; asked again at the fire, the exact-match derivation
/// reads that as owing nothing, and the pm would be started on its body with no thread to answer.
/// Fired from the row, it is told its slot. (No production writer records a bare expectation; the
/// re-recording is here to make the two answers differ, which is what the pin is about.)
#[test]
fn a_queued_entry_fires_with_the_reply_thread_it_was_queued_with() {
    let (tmp, engine, worker) = team();
    commission(&engine, caller("carsten", NOW), "holding", "build T5");
    let builder = worker.of("builder")[0].clone();
    commission(&engine, caller("carsten", NOW), "planning", ROUND_3);
    let slot = store(&tmp).list_working_tree_queue().unwrap()[0]
        .thread
        .clone()
        .unwrap();
    let mut s = store(&tmp);
    s.set_expects_reply_from(&slot, r#"["pm"]"#, "local/__channel__");
    drop(s);
    answer(
        &engine,
        &builder.internal_session,
        builder.reply_thread.as_deref().unwrap(),
        RELEASED,
        "built",
    );
    let pm = worker.of("pm");
    assert_eq!(pm.len(), 1, "the queued pm fired: {pm:?}");
    assert_eq!(
        pm[0].reply_thread.as_deref(),
        Some(slot.as_str()),
        "with its slot, from the row"
    );
    assert!(pm[0].message.contains(ROUND_3));
}
