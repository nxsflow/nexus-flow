//! The new `nxc` surface as it is reachable from the LIBRARY handle (nxf 6j6v.p6m1).
//!
//! The app half of this slice is built in manufakt.io, and it reaches the engine through
//! [`Engine`] — not through a subprocess and not through `facade::*`. So the acceptance for
//! `send --to` / `reply --thread` / `list` / `prime --persona` has to be taken HERE: a suite that
//! drives only the CLI proves the path the products do not walk (project rule
//! `engine-seam-test-rule`, which this file exists to satisfy for these four verbs).
//!
//! The team is DECLARED in the workspace's own `.nxs-personas/` (`common::write_declarations`,
//! called by this file's `engine` helper), which since nxf 6j6v.dvyq step 6 is the only shape there
//! is: `DefinitionSource::Supplied` and `Engine::set_definitions` are gone, so an app reads the
//! same folder the CLI does — and even a later in-app role editor writes into it. This file used to
//! say the opposite (definitions supplied, no folder in the user's project) and that was the design
//! this item reversed.

mod common;

use std::path::Path;

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::facade::{StatusScope, ThreadState};
use nexus_chat::model::{
    Disposition, MessageEnvelope, MessageKind, Priority, Refs, ThreadRoot, DOMAIN_MESSAGE,
    FIELD_ENVELOPE, FIELD_ROOT, KIND_MESSAGE, KIND_THREAD, OP_OPEN, OP_POST,
};
use nexus_chat::orchestration::Caller;
use nexus_chat::role::{AddressBookEntry, RoleDecl, Stage};
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest, TargetKind};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerError, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use nxs_foundation::model::Op;
use tempfile::TempDir;

const NOW: &str = "2026-08-05T10:00:00Z";

/// The qualified handle `<origin>/<agent>` this file's [`Engine`] mints, ASKED of the handle rather
/// than spelled out (nxf 6j6v.07me).
///
/// A `const ORIGIN: &str = "local"` stood here. The `<origin>` half is the workspace's own replica
/// prefix since the owner's decision of 2026-08-21, so it is random per `TempDir` and a literal is
/// not merely wrong, it is non-deterministically wrong. Every value below that carries an identity
/// — a receipt's `expects`, a quorum's `expects`/`outstanding`, the `consumer` a read is scoped to —
/// is `<origin>/<agent>` written once and matched by exact string, so this joins the two halves in
/// one place and takes the first from the seam under test. Tests further down that seed the store
/// with their own `local/…` handles keep them: nothing derives those, they are a fixture's private
/// strings.
fn handle(eng: &Engine, agent: &str) -> String {
    format!("{}/{}", eng.origin(), agent)
}

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

fn role(yaml: &str) -> RoleDecl {
    serde_yaml::from_str(yaml).expect("test role parses")
}

fn simple_role(handle: &str) -> RoleDecl {
    role(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
}

fn channel_decl(yaml: &str) -> ChannelDecl {
    serde_yaml::from_str(yaml).expect("test channel parses")
}

/// Declare `defs` in the workspace's own `.nxs-personas/` folder, then open a handle over it.
///
/// The catalogue used to be injected (`DefinitionSource::Supplied`); nxf 6j6v.dvyq step 6 removed
/// that path, so a test team is now declared the way a real one is — which also means these cases
/// drive the real folder read rather than stepping around it.
fn engine(dir: &Path, defs: Definitions) -> Engine {
    common::write_declarations(dir, &defs);
    Engine::open_with(
        None,
        dir,
        EngineConfig {
            // A real (dry) worker: the persona path SPAWNS, and a handle that refuses to spawn
            // would make this suite prove the wrong thing. `log: None` — nothing here reads a
            // trigger log, and saying so is what keeps this suite independent of what any other
            // test in this binary put in the environment (nxf 6j6v.570x).
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine")
}

/// Who is calling: a HUMAN, which is exactly the case that still names its `actor` — there is no
/// session for the map to read a persona off (nxf 6j6v.07me). A case that speaks from inside a
/// running session writes `Caller { session: Some(&s), ..caller("coder") }` and the name behind it
/// is then the fallback that is never reached.
fn caller(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

fn send_req<'a>(to: &'a str, body: &'a str) -> SendToRequest<'a> {
    SendToRequest {
        machine: None,
        to,
        body,
        refs: SendToRefs::ExplicitlyNone,
    }
}

fn reply_req<'a>(thread: &'a str, body: &'a str) -> ReplyThreadRequest<'a> {
    ReplyThreadRequest {
        machine: None,
        thread,
        body,
        escalate: false,
        needs_rework: false,
        accept: false,
    }
}

// `msg_reply_req` was here — the `ReplyRequest` builder for `Engine::reply`, "distinct from
// `reply_req`'s `ReplyThreadRequest`: this targets a MESSAGE or thread id". REMOVED with that
// method (nxf 6j6v.ckeq): a conversation has ONE address now, and it is the thread. Every call site
// in this file addressed a thread anyway or had its message id one read away; the one that did not
// says so where it stands.

fn seed_store(dir: &Path) -> nexus_chat::store::ChatStore {
    Workspace::resolve(None, dir)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

// ---- send --to --------------------------------------------------------------------------------

#[test]
fn sending_to_a_persona_opens_a_thread_and_starts_it() {
    let tmp = workspace();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let eng = engine(tmp.path(), defs);

    let receipt = eng
        .send_to(caller("carsten"), send_req("pm", "plan the release"))
        .expect("send --to a persona");

    assert_eq!(receipt.target, TargetKind::Persona);
    assert!(!receipt.thread_id.is_empty(), "a thread is always stamped");
    assert!(
        receipt.session.is_some(),
        "the persona was started — that session is what a later reply resumes"
    );
    assert!(
        receipt.channel.starts_with("dm:"),
        "the direct conversation is materialised for the caller: {}",
        receipt.channel
    );
    // nxf 6j6v.hpv8, PR review Important #1: a clean spawn's receipt must show `spawned: true` and
    // an empty `warnings` — the happy-path half of the same assertion the CLI seam pins on the
    // rendered `--json` (`surface_cli.rs`, where the concern is sharper: a JSON key can silently
    // disappear behind `skip_serializing_if`, which no Rust-side field read here would ever catch).
    assert!(receipt.spawned, "{receipt:?}");
    assert!(receipt.warnings.is_empty(), "{receipt:?}");

    // The message is in the thread, and it is the caller's own words — not the plumbing the
    // persona was additionally told.
    let messages = eng
        .thread(&receipt.thread_id, &handle(&eng, "carsten"))
        .expect("read the thread back");
    assert_eq!(messages.messages.len(), 1);
    assert_eq!(messages.messages[0].body, "plan the release");
}

/// A worker that fails EVERY trigger — the Engine-seam fixture nxf 6j6v.hpv8 points at
/// (`crates/chat/tests/worker.rs`'s `HostWorker::new(|_| Err(...))`, private to that binary).
struct FailingWorker;
impl Worker for FailingWorker {
    fn trigger(&self, _req: TriggerRequest) -> TriggerResult {
        Err(TriggerError::Failed(nexus_chat::error::NxfError::io(
            "the runtime refused this trigger",
        )))
    }
}

#[test]
fn send_to_a_persona_reports_a_warning_when_a_real_worker_refuses_the_spawn() {
    // nxf 6j6v.hpv8: the persona branch posts the message, mints the thread and THEN triggers the
    // role (`surface::send_to` → `orchestration::coordinator_commission`), so a worker that
    // refuses must
    // not discard the receipt behind a bare `Err` — the message and the open thread are already
    // durable by the time the worker is even asked. This is the `send --to <persona>` half of the
    // ticket's DoD, taken at the seam manufakt.io/nexflow.it actually call through (this file's own
    // `engine-seam-test-rule` doc above), not only over the CLI subprocess boundary.
    let tmp = workspace();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    common::write_declarations(tmp.path(), &defs);
    let eng = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(std::sync::Arc::new(FailingWorker)),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    let receipt = eng
        .send_to(caller("carsten"), send_req("pm", "please help"))
        .expect("the thread and message are persisted even though the spawn failed");

    assert_eq!(receipt.target, TargetKind::Persona);
    assert!(!receipt.thread_id.is_empty());
    assert!(
        receipt.session.as_deref().is_some_and(|s| !s.is_empty()),
        "a session was minted even though it never started: {receipt:?}"
    );
    assert!(
        !receipt.spawned,
        "the typed discriminator, not just non-empty warnings: {receipt:?}"
    );
    let warnings = &receipt.warnings;
    assert_eq!(warnings.len(), 1, "{receipt:?}");
    assert!(
        warnings[0]
            .detail
            .contains("the runtime refused this trigger"),
        "{receipt:?}"
    );
    // …and the SEAM relays the class and the reason, not just the sentence (nxf 6j6v.6m6x): the
    // persona path is `TriggerReceipt::warnings` handed through verbatim.
    assert_eq!(
        warnings[0].reason,
        Some(nexus_chat::orchestration::WakeSkipReason::TriggerFailed),
        "{receipt:?}"
    );
    assert_eq!(
        warnings[0].thread.as_deref(),
        Some(receipt.thread_id.as_str()),
        "the thread the message is durably sitting in, with nobody working on it: {receipt:?}"
    );

    // The message and the open thread really are there — `nxc reply --thread` (or an app reading
    // the same store, as here) can still find them.
    let messages = eng
        .thread(&receipt.thread_id, &handle(&eng, "carsten"))
        .expect("read the thread back");
    assert_eq!(
        messages
            .messages
            .iter()
            .map(|m| m.body.as_str())
            .collect::<Vec<_>>(),
        vec!["please help"]
    );
}

#[test]
fn sending_to_a_persona_now_plans_the_reply_it_expects_on_the_new_thread() {
    // The gap this ticket closes (nxf 6j6v.jepk): a bare `send --to @persona` used to plan
    // NOTHING — no deadline, no liveness, no required reply — so a session that died on this, the
    // MOST COMMON trigger path, was never noticed by anything. The thread `send_to` just stamped
    // must now carry the summoned persona in `expects_reply_from`, exactly as `ask`'s board and the
    // declared-channel fan-out already leave behind for their own triggers.
    let tmp = workspace();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let eng = engine(tmp.path(), defs);

    let receipt = eng
        .send_to(caller("carsten"), send_req("pm", "plan the release"))
        .expect("send --to a persona");

    // `status` with an EXPLICIT SET is what `thread_quorums` became (nxf 6j6v.yr59).
    let report = eng
        .status(NOW, StatusScope::Threads(&[&receipt.thread_id]))
        .unwrap();
    let q = &report
        .operations
        .first()
        .expect("the thread `send_to` just opened is readable back")
        .threads[0];
    assert_eq!(
        q.expects,
        vec![handle(&eng, "pm")],
        "the summoned persona is the one this thread now owes a reply: {q:?}"
    );
    assert_eq!(q.outstanding, vec![handle(&eng, "pm")]);
    assert_eq!(
        q.state,
        nexus_chat::facade::ThreadState::Open,
        "nobody has replied yet"
    );

    // PR review, Finding 3: the RECEIPT must agree with the bulk read, not under-report it. An
    // app that only looks at the receipt (never re-reading the thread) must be able to see the same
    // thing the bulk read shows, instead of concluding — wrongly — that nobody owes a reply.
    assert_eq!(
        receipt.expects,
        vec![handle(&eng, "pm")],
        "the receipt reports what it just created, not an empty placeholder"
    );
}

#[test]
fn replying_to_a_personas_own_message_still_resumes_it_directly() {
    // CRITICAL regression (review of nxf 6j6v.jepk, 2026-08-12): once `send_to` started declaring
    // `expects_reply_from` on the persona's DM thread, `orchestration::reply`'s `is_quorum_thread`
    // gate — which used to read `!expects.is_empty()` — started reading `true` for EVERY persona
    // thread, forever (the register never clears once set). That skipped the direct 1:1 resume at
    // `reply`'s step (3) for good: the persona session a human is continuing a plain DM with stopped
    // being resumed. (The entrances that showed it then — `nxc reply <message-id>` and
    // `Engine::reply` — are both gone; the gate itself is not, which is why this test is not.) Owner
    // ruling: whether a thread is a
    // quorum board is a property of the CHANNEL (a `dm:`-prefixed id is never one), not of whether
    // `expects_reply_from` happens to be non-empty.
    let tmp = workspace();
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();
    let eng = engine(tmp.path(), defs);

    // Alice opens a DM with the persona — the thread now expects `coder` (nxf 6j6v.jepk).
    let opened = eng
        .send_to(caller("alice"), send_req("coder", "please help"))
        .expect("send --to a persona");
    let coder_session = opened.session.clone().expect("the persona was started");

    // The persona answers, AS coder, so its own reply is stamped with coder's session as the
    // message's return address (`with_return_address`) — the same shape a real sidecar reply takes.
    eng.reply_thread(
        Caller {
            session: Some(&coder_session),
            ..caller("coder")
        },
        reply_req(&opened.thread_id, "here you go"),
    )
    .expect("the persona replies");

    // Alice continues the SAME DM — this must resume coder's session, not fall through to the
    // quorum push-wake (which only fires for a sender that is itself in `expects`, i.e. never for
    // alice here, and would silently swallow the turn).
    //
    // ADDRESSED BY THREAD, not by the persona's MESSAGE id. `Engine::reply` took either and is gone
    // (nxf 6j6v.ckeq): a conversation has one address now, and it is the thread. The property under
    // test is unchanged and so is the code that carries it — `orchestration::reply`'s
    // `is_quorum_thread` still asks whether the thread lives in a DM channel rather than whether
    // `expects_reply_from` happens to be non-empty, and it is still the reason this resumes. The
    // message-target form of that same question survives at `orchestration::reply`, whose own suite
    // (`orchestration_reply.rs`) drives it.
    let continued = eng
        .reply_thread(
            caller("alice"),
            reply_req(&opened.thread_id, "one more thing"),
        )
        .expect("alice continues the conversation");

    assert!(
        continued.resumed,
        "a reply into a DM thread must resume the persona, regardless of expects_reply_from: \
         {continued:?}"
    );
    assert_eq!(continued.woke.as_deref(), Some(coder_session.as_str()));
}

#[test]
fn sending_to_a_declared_channel_opens_its_board_under_its_own_policy() {
    let tmp = workspace();
    let defs = Definitions::new(
        vec![simple_role("coder"), simple_role("reviewer")],
        vec![channel_decl(
            "name: code-review\nmembers: [coder, reviewer]\ntimeout: 20m\n",
        )],
    )
    .unwrap();
    let eng = engine(tmp.path(), defs);

    let receipt = eng
        .send_to(caller("carsten"), send_req("code-review", "look at PR 12"))
        .expect("send --to a channel");

    assert_eq!(receipt.target, TargetKind::Channel);
    assert_eq!(receipt.channel, "decl:code-review");
    assert_eq!(
        receipt.expects,
        [handle(&eng, "__channel__")],
        "the channel THREAD has two ends — the requester and the supervisor (nxf 6j6v.pf6j)"
    );
    // The channel's own policy still decides who must answer; it decides it one level down, one
    // expectation per member thread, which is what the caller reads back off the tree.
    let members: Vec<Vec<String>> = eng
        .status(NOW, StatusScope::Threads(&[&receipt.thread_id]))
        .unwrap()
        .operations
        .into_iter()
        .flat_map(|op| op.threads)
        .filter(|t| t.parent.as_deref() == Some(receipt.thread_id.as_str()))
        .map(|t| t.expects)
        .collect();
    assert_eq!(
        members,
        vec![vec![handle(&eng, "coder")], vec![handle(&eng, "reviewer")]],
        "the caller said nothing about it, and each member owes on a thread of its own"
    );
    assert!(
        receipt.deadline.is_some(),
        "and its declared timeout is resolved to an instant"
    );
    assert!(
        receipt.session.is_none(),
        "a fan-out starts several sessions; the board is the addressable thing, not one of them"
    );
}

/// A host worker that refuses to start ONE named role and starts everything else — so a channel's
/// fan-out can succeed while the WAKE that follows it fails, which is the shape a failed consequence
/// actually has (a fixture that refuses everything cannot produce one, because nothing gets far
/// enough to have a consequence).
struct RefusesOneRole(&'static str);
impl Worker for RefusesOneRole {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        if req.role.handle == self.0 {
            return Err(TriggerError::Failed(nexus_chat::error::NxfError::io(
                format!("the runtime refused to start {}", self.0),
            )));
        }
        Ok(nexus_chat::worker::TriggerOutcome::Started {
            runtime_session: format!("real-{}", req.internal_session),
        })
    }
}

#[test]
fn the_answering_session_learns_of_the_failed_consequence_in_the_very_call_it_is_still_running() {
    // **nxf 6j6v.93zd's second place, at the seam an app actually calls** — and hq71 §1's decisive
    // argument for a synchronous supervisor, made observable: the consolidation runs INSIDE the
    // reply that settled the set, so the party still executing that call is there to be told. Run
    // asynchronously, the answering session would have ended long before the wake was even
    // attempted, and the failure would be nobody's.
    //
    // The consequence is broken at the real path: the requester's role is one the runtime refuses to
    // start, so the fan-out succeeds, the board completes, the answers are delivered — and the
    // session that asked for them is never woken.
    let tmp = workspace();
    let defs = Definitions::new(
        vec![simple_role("pm"), simple_role("reviewer")],
        vec![channel_decl("name: review\nmembers: [reviewer]\n")],
    )
    .unwrap();
    common::write_declarations(tmp.path(), &defs);
    let eng = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(std::sync::Arc::new(RefusesOneRole("pm"))),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .unwrap();

    // The pm is commissioned (its own start is refused, which the receipt already says) and then
    // commissions the channel from the session that WAS minted for it.
    let started = eng
        .send_to(caller("carsten"), send_req("pm", "plan the release"))
        .expect("the message is persisted even though the pm could not be started");
    let pm_session = started.session.clone().expect("a session was minted");
    let board = eng
        .send_to(
            Caller {
                session: Some(&pm_session),
                ..caller("pm")
            },
            send_req("review", "please review"),
        )
        .expect("the fan-out itself succeeds — the reviewer starts fine");

    let member = eng
        .status(NOW, StatusScope::Threads(&[&board.thread_id]))
        .unwrap()
        .operations
        .remove(0)
        .threads
        .into_iter()
        .find(|t| t.parent.as_deref() == Some(board.thread_id.as_str()))
        .expect("the supervisor opened a thread for the one member")
        .thread_id;

    // The reviewer answers. Everything downstream of that answer happens in THIS call.
    let receipt = eng
        .reply_thread(caller("reviewer"), reply_req(&member, "lgtm"))
        .expect("the reply is posted and the channel consolidated inside it");

    assert!(receipt.posted, "{receipt:?}");
    assert!(
        receipt.completed.is_some(),
        "the channel DID consolidate — only the wake failed: {receipt:?}"
    );
    assert!(receipt.woke.is_none(), "nobody was woken: {receipt:?}");
    assert_eq!(
        receipt
            .warnings
            .iter()
            .map(|w| (w.class, w.session.as_deref()))
            .collect::<Vec<_>>(),
        vec![(
            nexus_chat::orchestration::ConsequenceClass::RequesterNotWoken,
            Some(pm_session.as_str())
        )],
        "the one defined place, on the receipt of the call that caused it: {receipt:?}"
    );
    assert!(
        receipt.warnings[0]
            .detail
            .contains("the runtime refused to start pm"),
        "…carrying the underlying failure for a human beside the class: {receipt:?}"
    );

    // And the reason it matters: the answers really are there, and the party that asked for them
    // really has not been told.
    assert!(
        receipt
            .completed
            .as_ref()
            .is_some_and(|c| c.delivered.contains("lgtm")),
        "{receipt:?}"
    );
}

/// The same fixture as `sending_to_a_plain_channel_posts_and_wakes_nobody`, which stood here and
/// asserted the opposite: a substrate channel nobody declared used to be a target
/// (`TargetKind::Conversation`), the message was posted and nobody was woken.
///
/// 6j6v.dvyq §3 removes that, and this is the seam half of the removal. The channel is REAL — it is
/// in the store with a kind and a member — so the answer cannot be "no such target"; it has to say
/// that nothing declares it, and where a declaration goes. Same shape as the empty-catalogue
/// rejection beside it.
#[test]
fn sending_to_a_plain_channel_is_refused_because_nothing_declares_it() {
    let tmp = workspace();
    let defs = Definitions::new(vec![], vec![]).unwrap();
    let eng = engine(tmp.path(), defs);
    {
        let mut store = seed_store(tmp.path());
        store.set_channel_field("m-plain", "kind", "group", "local/carsten");
        store.add_member("m-plain", "local/carsten", "local/carsten");
    }

    let err = eng
        .send_to(caller("carsten"), send_req("m-plain", "note to self"))
        .map(|_| ())
        .expect_err("a channel no declaration names is not a target");

    assert!(
        err.msg.contains("m-plain") && err.msg.contains("no declaration names it"),
        "{}",
        err.msg
    );
}

#[test]
fn a_channel_only_persona_refuses_a_direct_message_and_says_where_to_go() {
    // A restriction that reads the same for EVERY caller, so it cannot be widened by dropping an
    // identity (see `nexus_chat::persona`). That used to be the whole rule; since nxf 6j6v.st83 it
    // is the rule for `general` and `none` and this DEPRECATED channel-list form, while a
    // whitelist answers per caller — `addressable_names_its_callers.rs` covers that half and says
    // why it is admissible.
    //
    // The fixture keeps the deprecated spelling deliberately: it is the migration path, and a
    // workspace that has not been rewritten yet must keep refusing exactly like this, naming the
    // channel its own file declares rather than a derived one.
    let tmp = workspace();
    let defs = Definitions::new(
        vec![role(
            "handle: reviewer\nsystem_prompt: p\naddressable: [code-review]\n",
        )],
        vec![channel_decl("name: code-review\nmembers: [reviewer]\n")],
    )
    .unwrap();
    let eng = engine(tmp.path(), defs);

    let err = eng
        .send_to(caller("carsten"), send_req("reviewer", "hi"))
        .expect_err("a channel-only persona is not directly addressable");
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains("code-review"),
        "the refusal has to say how to reach it instead: {}",
        err.msg
    );
}

#[test]
fn an_unknown_target_is_not_found_and_names_what_was_searched() {
    // A FURNISHED workspace: `ghost` is a miss among real declarations, so the useful answer names
    // the three namespaces that were searched. The EMPTY workspace is a different question and gets
    // a different answer — see `declaration_source.rs`, where the bootstrap rejection lives
    // (nxf 6j6v.dvyq step 4).
    let tmp = workspace();
    let eng = engine(
        tmp.path(),
        Definitions::new(vec![simple_role("pm")], vec![]).unwrap(),
    );
    let err = eng
        .send_to(caller("carsten"), send_req("ghost", "hello?"))
        .expect_err("nothing by that name");
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert!(err.msg.contains("nxc list"), "point at the way to find out");
}

#[test]
fn a_read_surface_names_the_ops_author_not_the_sender_its_payload_claims() {
    // 6j6v.aym3 at the seam the PRODUCTS read (project rule `engine-seam-test-rule`): manufakt.io
    // and nexflow.it call `Engine::messages`/`Engine::inbox`, not the CLI and not `facade::*`, so
    // the acceptance for "the views hang on the identity E4 authenticates" has to be taken here.
    //
    // A foreign op — one a peer wrote and sync delivered — claims `sender: acme/alice` in its
    // payload while the op itself is authored by `acme/mallory`. Nothing reconciles the two (§7
    // forbids rejecting the op), so the app must be shown the author. Once 6j6v.6aza signs ops
    // that is the authenticated identity; the payload's own `sender` never becomes one.
    let tmp = workspace();
    {
        let mut store = seed_store(tmp.path());
        store.set_channel_field("m-chan", "kind", "group", "local/carsten");
        store.add_member("m-chan", "local/carsten", "local/carsten");
        store.apply(&[Op {
            op_id: "01KXFORGED000000000000000".into(),
            lamport: 99,
            site: 9,
            domain: DOMAIN_MESSAGE.into(),
            target_kind: KIND_MESSAGE.into(),
            target_id: "m-forged".into(),
            field: FIELD_ENVELOPE.into(),
            op_type: OP_POST.into(),
            value: Some(
                serde_json::to_string(&MessageEnvelope {
                    origin: "acme".into(),
                    channel_id: "m-chan".into(),
                    sender: "acme/alice".into(),
                    kind: MessageKind::Task,
                    priority: Priority::Normal,
                    disposition: Disposition::InTurn,
                    thread_id: None,
                    refs: Refs::default(),
                    body: "ship the release".into(),
                })
                .unwrap(),
            ),
            author: "acme/mallory".into(),
            wall_clock: NOW.into(),
            key_id: None,
            sig: None,
        }]);
    }
    // The handle is opened even though the two reads below go to the store: opening it is what
    // proves the workspace this suite forged into is one an app can hold at all.
    let _eng = engine(tmp.path(), Definitions::new(vec![], vec![]).unwrap());

    let msgs =
        common::channel_messages(tmp.path(), "m-chan", "local/carsten").expect("read the channel");
    let forged = msgs
        .iter()
        .find(|m| m.message_id == "m-forged")
        .expect("the foreign message is visible");
    assert_eq!(
        forged.sender, "acme/mallory",
        "the app is shown who wrote the op, not who the payload claims wrote it"
    );

    // A second read stood here — the same assertion over the unread inbox, "the same identity on
    // the surface an agent acts from". The inbox went with nxf 6j6v.4d2z and there is no second
    // message surface to disagree with the first any more: what an agent acts from is the
    // conversation, which is the read above.
}

/// A foreign `thread`/`open` op whose declared `ThreadRoot.opener` is NOT the op's author — the
/// thread-side twin of the forged message envelope, and the one that carries an authorization
/// claim rather than a display string.
fn forged_thread_open(thread_id: &str, claims_opener: &str, really_authored_by: &str) -> Op {
    Op {
        op_id: format!("01KXFORGEDTHREAD{thread_id:>010}"),
        lamport: 98,
        site: 9,
        domain: DOMAIN_MESSAGE.into(),
        target_kind: KIND_THREAD.into(),
        target_id: thread_id.into(),
        field: FIELD_ROOT.into(),
        op_type: OP_OPEN.into(),
        value: Some(
            serde_json::to_string(&ThreadRoot {
                origin: "acme".into(),
                channel_id: "m-chan".into(),
                opener: claims_opener.into(),
                created: NOW.into(),
                parent: None,
            })
            .unwrap(),
        ),
        author: really_authored_by.into(),
        wall_clock: NOW.into(),
        key_id: None,
        sig: None,
    }
}

#[test]
fn a_forged_thread_opener_neither_shows_on_the_seam_nor_passes_the_opener_only_gate() {
    // 6j6v.aym3's thread half, at the two places it actually matters (PR #314 review, Test Quality
    // #1). `threads.opener` is not decoration: `facade::set_expects` — the seam write that opens a
    // role's next turn (6j6v.cg8g) — is OPENER-ONLY and compares the caller against exactly that
    // column. So a foreign op that declares someone else as its `ThreadRoot.opener` was writing an
    // authorization claim, and the reducer-internal check alone does not show that the gate moved
    // with the fold.
    //
    // `set_expects` lives on `facade`, not on `Engine`, so the two halves are taken where each one
    // is: the READ on the Engine seam the products call, the AUTHORIZATION at the facade — which
    // since 6j6v.dvyq removed `nxc threads expect` is reached from inside the engine (the channel
    // supervisor's re-declaration), not from a CLI verb.
    let tmp = workspace();
    {
        let mut store = seed_store(tmp.path());
        store.set_channel_field("m-chan", "kind", "group", "local/carsten");
        for handle in ["local/carsten", "acme/alice", "acme/mallory"] {
            store.add_member("m-chan", handle, "local/carsten");
        }
        // Mallory opens a thread, claiming Alice opened it.
        store.apply(&[forged_thread_open(
            "m-forged-thread",
            "acme/alice",
            "acme/mallory",
        )]);
        // A board only derives once someone is expected to reply; set it as the substrate would.
        store.set_expects_reply_from("m-forged-thread", "[\"local/carsten\"]", "acme/mallory");
    }
    let eng = engine(tmp.path(), Definitions::new(vec![], vec![]).unwrap());

    // The Engine seam: the board names the op's author, not the opener the payload declared. Read
    // through `status` since nxf 6j6v.yr59 folded `threads` into it.
    let report = eng
        .status(NOW, StatusScope::Threads(&["m-forged-thread"]))
        .expect("read the boards");
    let board = report
        .operations
        .iter()
        .flat_map(|op| op.threads.iter())
        .find(|b| b.thread_id == "m-forged-thread")
        .expect("the foreign board is visible");
    assert_eq!(
        board.opener.as_deref(),
        Some("acme/mallory"),
        "the app is shown who opened the thread, not who the payload claims did"
    );

    // The authorization gate moved with it: the handle the payload named cannot amend the board…
    let mut store = seed_store(tmp.path());
    let err = nexus_chat::facade::set_expects(
        &mut store,
        NOW,
        "acme/alice",
        "m-forged-thread",
        &["acme/mallory".to_string()],
    )
    .expect_err("the declared opener is not the opener");
    assert_eq!(err.kind, ErrorKind::Forbidden);

    // …and the identity that actually wrote the op can. (Which is the honest outcome: 6j6v.6aza
    // has yet to make `op.author` unforgeable — what this fixes is that the gate now reads the
    // field that slice authenticates, instead of one it never will.)
    let q = nexus_chat::facade::set_expects(
        &mut store,
        NOW,
        "acme/mallory",
        "m-forged-thread",
        &["acme/mallory".to_string()],
    )
    .expect("the op's author is the opener");
    assert_eq!(q.expects, vec!["acme/mallory"]);
}

// ---- reply --thread ---------------------------------------------------------------------------

#[test]
fn a_reply_into_a_thread_hands_the_turn_back_to_the_other_side() {
    // The whole multi-turn claim in one test: the human opens a conversation, the persona answers,
    // the human answers back — and the second turn RESUMES the persona's own session rather than
    // posting into silence.
    let tmp = workspace();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let eng = engine(tmp.path(), defs);

    let opened = eng
        .send_to(caller("carsten"), send_req("pm", "plan the release"))
        .expect("open the conversation");
    let pm_session = opened.session.clone().expect("the persona was started");

    // The persona answers, from inside its own session — as a spawned one always does.
    let answered = eng
        .reply_thread(
            Caller {
                session: Some(&pm_session),
                ..caller("pm")
            },
            reply_req(&opened.thread_id, "here is the plan"),
        )
        .expect("the persona replies");
    assert_eq!(
        answered.thread_id.as_deref(),
        Some(opened.thread_id.as_str())
    );
    assert!(
        !answered.resumed,
        "the human it answered has no session to resume — and that is not a failure"
    );
    assert!(answered.wake_skipped.is_none(), "nothing was owed a wake");

    // The human answers back: THIS is the turn that must reach the persona.
    let follow_up = eng
        .reply_thread(
            caller("carsten"),
            reply_req(&opened.thread_id, "and the risks?"),
        )
        .expect("the human replies");
    assert!(
        follow_up.resumed,
        "the thread knows which session to continue"
    );
    assert_eq!(
        follow_up.woke.as_deref(),
        Some(pm_session.as_str()),
        "and it is the persona's own session, not a fresh one"
    );
}

#[test]
fn replying_into_a_thread_nobody_has_answered_yet_posts_and_says_it_woke_nobody() {
    let tmp = workspace();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let eng = engine(tmp.path(), defs);
    let opened = eng
        .send_to(caller("carsten"), send_req("pm", "first"))
        .expect("open the conversation");

    let again = eng
        .reply_thread(
            caller("carsten"),
            reply_req(&opened.thread_id, "one more thing"),
        )
        .expect("a second message before any answer is still a message");
    assert!(!again.resumed);
    assert!(again.woke.is_none());
    assert!(
        again.wake_skipped.is_none(),
        "nothing was ATTEMPTED, so nothing is reported as skipped"
    );
}

#[test]
fn an_unknown_thread_is_not_found_and_names_the_thread() {
    let tmp = workspace();
    let eng = engine(tmp.path(), Definitions::new(vec![], vec![]).unwrap());
    let err = eng
        .reply_thread(caller("carsten"), reply_req("t-nope", "hello?"))
        .expect_err("no such thread");
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert!(err.msg.contains("no such thread"), "{}", err.msg);
}

#[test]
fn a_completing_reply_into_a_board_keeps_the_quorum_routing() {
    // The guard on the added resume: a board's own completion routing must still be what runs, or
    // this surface would have quietly replaced the mechanism it sits on.
    let tmp = workspace();
    let defs = Definitions::new(
        vec![simple_role("coder"), simple_role("pm")],
        vec![channel_decl("name: standup\nmembers: [coder]\n")],
    )
    .unwrap();
    let eng = engine(tmp.path(), defs);

    let board = eng
        .send_to(caller("pm"), send_req("standup", "status?"))
        .expect("open the board");
    assert_eq!(board.expects, [handle(&eng, "__channel__")]);
    // `coder` answers in the thread the supervisor opened for it (nxf 6j6v.pf6j) — the level a
    // member is triggered into and the only one it is ever pointed at.
    let member_thread = eng
        .status(NOW, StatusScope::Threads(&[&board.thread_id]))
        .unwrap()
        .operations
        .into_iter()
        .flat_map(|op| op.threads)
        .find(|t| t.parent.as_deref() == Some(board.thread_id.as_str()))
        .map(|t| t.thread_id)
        .expect("one member thread");

    let done = eng
        .reply_thread(caller("coder"), reply_req(&member_thread, "all green"))
        .expect("the expected member replies");
    assert!(
        !done.resumed,
        "a quorum board completes through its own routing, never through the thread resume"
    );
}

#[test]
fn an_app_replying_into_a_round_that_is_over_meets_the_same_refusal_the_cli_does() {
    // **The seam half of nxf 6j6v.0vd9**, and it is here rather than only over the CLI because
    // this file's own rule says so: an app reaches this through `Engine::reply_thread`, the change
    // is declared `facade: breaking` in its changelog fragment, and a behavioural break is exactly
    // the kind `cargo-semver-checks` cannot see — a call that used to return `Ok` now returns
    // `Err`. A suite that proves it only through a subprocess proves the path manufakt.io and
    // nexflow.it do not walk.
    //
    // The state is reached the way a run reaches it: one member, its answer consolidates the
    // channel, and the round is then over. A second word on that member's thread wakes nobody and
    // used to be accepted in silence behind a receipt that named a due date.
    let tmp = workspace();
    let defs = Definitions::new(
        vec![simple_role("coder"), simple_role("pm")],
        vec![channel_decl("name: standup\nmembers: [coder]\n")],
    )
    .unwrap();
    let eng = engine(tmp.path(), defs);

    let board = eng
        .send_to(caller("pm"), send_req("standup", "status?"))
        .expect("open the board");
    let member_thread = eng
        .status(NOW, StatusScope::Threads(&[&board.thread_id]))
        .unwrap()
        .operations
        .into_iter()
        .flat_map(|op| op.threads)
        .find(|t| t.parent.as_deref() == Some(board.thread_id.as_str()))
        .map(|t| t.thread_id)
        .expect("one member thread");
    eng.reply_thread(caller("coder"), reply_req(&member_thread, "all green"))
        .expect("the expected member replies and the channel consolidates");

    let messages_before = eng
        .thread(&member_thread, &handle(&eng, "coder"))
        .expect("read the member thread")
        .messages
        .len();

    let err = eng
        .reply_thread(
            caller("carsten"),
            reply_req(&member_thread, "one more thing"),
        )
        .expect_err("the round this thread served is over");
    assert_eq!(
        err.kind,
        ErrorKind::Forbidden,
        "a rule looked at the world and said no — nothing about the call is malformed: {}",
        err.msg
    );
    assert!(
        err.msg.contains("this round is over"),
        "the app is told WHAT is wrong, not merely that something is: {}",
        err.msg
    );
    assert!(
        err.msg.contains("nxc send --to standup"),
        "…and which door does start work, with the channel filled in: {}",
        err.msg
    );
    assert_eq!(
        eng.thread(&member_thread, &handle(&eng, "coder"))
            .expect("read the member thread")
            .messages
            .len(),
        messages_before,
        "a refused reply writes nothing — the refusal is a preflight, not a rollback"
    );
}

// `engine_reply_thread_if_unanswered_posts_and_resumes_the_return_address_when_owed` and
// `engine_reply_thread_if_unanswered_is_a_silent_no_op_and_wakes_nobody_when_not_owed` stood here.
// REMOVED with the field they drove (nxf 6j6v.ckeq, decision 4): `ReplyThreadRequest` carries no
// `if_unanswered`, because it was never an app's question. It is the idempotency switch of ONE
// caller — `agent-sidecar/src/main.mjs`'s teardown, which runs `nxc reply --thread <id>
// --if-unanswered <text>` when an SDK session ends without answering the thread it owed, and runs it
// unconditionally precisely because it cannot tell whether the session already answered. An app
// holds its own sessions and knows.
//
// The `engine-seam-test-rule` these two were written under (`Engine::reply_thread`, not
// `surface::reply_in_thread`'s `Ctx` entry point, is what an app actually calls) is therefore
// satisfied by their removal rather than violated by it: there is no app call to pin. The MECHANISM
// keeps full coverage one layer down, in the shape its only caller uses — the same chained pm→coder
// fixture, so the regression these guarded (a SKIP must not find the return address and resume with
// a reply that was never written) is still driven end to end:
//
//   * `surface_cli.rs::reply_thread_if_unanswered_posts_and_resumes_the_return_address_when_owed`
//   * `surface_cli.rs::reply_thread_if_unanswered_is_a_silent_no_op_and_wakes_nobody_when_not_owed`
//   * `surface_cli.rs::the_sidecar_teardown_still_writes_its_failure_reply_when_a_session_ends_unanswered`
//     — 6j6v.ckeq's own acceptance point, which pins the teardown's exact invocation.
// ---- list / prime -----------------------------------------------------------------------------

#[test]
fn the_directory_is_the_whole_team_or_one_personas_address_book() {
    let tmp = workspace();
    let mut coder = simple_role("coder");
    coder.address_book = Some(vec![AddressBookEntry {
        to: "pm".into(),
        why: Some("hand back the finished work order".into()),
    }]);
    let mut pm = simple_role("pm");
    pm.job_title = Some("Product manager".into());
    pm.stage = Some(Stage::Senior);
    let defs = Definitions::new(
        vec![coder, pm],
        vec![channel_decl("name: standup\nmembers: [coder, pm]\n")],
    )
    .unwrap();
    let eng = engine(tmp.path(), defs);

    let all = eng.directory(None).expect("the full catalogue");
    assert_eq!(all.personas.len(), 2);
    assert_eq!(all.channels.len(), 1);
    assert_eq!(all.personas[1].stage, Some(Stage::Senior));

    let mine = eng
        .directory(Some("coder"))
        .expect("one persona's own view");
    assert_eq!(mine.persona.as_deref(), Some("coder"));
    assert_eq!(mine.personas.len(), 1, "the address book, not the team");
    assert_eq!(mine.personas[0].handle, "pm");
    assert_eq!(
        mine.personas[0].why.as_deref(),
        Some("hand back the finished work order")
    );
}

#[test]
fn the_directory_follows_the_workspaces_own_declarations() {
    // The reason this read is on the handle at all: an app renders its own directory, and it must
    // be the SAME catalogue `nxc list` prints rather than a second derivation app-side.
    //
    // It used to say "the definitions the app SUPPLIED", back when a handle could be handed one
    // (nxf 6j6v.dvyq step 6 removed that): the supply path is gone, the read is not, and what the
    // read follows is now the workspace's `.nxs-personas/` folder — which is also where an in-app
    // role editor writes.
    let tmp = workspace();
    let eng = engine(
        tmp.path(),
        Definitions::new(vec![simple_role("declared-here")], vec![]).unwrap(),
    );
    let dir = eng.directory(None).unwrap();
    assert_eq!(dir.personas.len(), 1);
    assert_eq!(dir.personas[0].handle, "declared-here");

    // And it FOLLOWS the folder rather than a snapshot taken at open: an edit lands on the next
    // read, with no reopen — the property `set_definitions` used to provide.
    common::replace_declarations(
        tmp.path(),
        &Definitions::new(vec![simple_role("edited-in")], vec![]).unwrap(),
    );
    let dir = eng.directory(None).unwrap();
    assert_eq!(dir.personas.len(), 1);
    assert_eq!(dir.personas[0].handle, "edited-in");
}

#[test]
fn priming_as_a_persona_adds_its_identity_and_leaves_a_humans_prime_alone() {
    // `prime_as` reads the workspace's own declaration folder (as `prime` always has), so this one
    // declares on disk directly rather than through the `engine` helper.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\njob_title: Product manager\nsystem_prompt: You are the PM.\n",
    )
    .unwrap();
    let eng = Engine::open(None, tmp.path()).expect("open engine");

    // `prime_as` with no persona IS `prime` — which is why the latter went (nxf 6j6v.yr59), and
    // `None` is exactly the human at the keyboard this case is about.
    let human = eng
        .prime_as("local/carsten", None, NOW)
        .expect("a human primes");
    assert!(human.persona.is_none(), "a human is told no identity");
    assert!(
        !human.directory.personas.is_empty(),
        "but IS told who can be addressed — the acceptance's first step"
    );

    let persona = eng
        .prime_as("local/pm", Some("pm"), NOW)
        .expect("a persona primes");
    let brief = persona
        .persona
        .as_ref()
        .expect("the persona knows who it is");
    assert_eq!(brief.handle, "pm");
    assert_eq!(brief.job_title.as_deref(), Some("Product manager"));
    let rendered = persona.render_markdown(false);
    assert!(rendered.contains("## You are `pm`"));
    assert!(
        rendered.contains("nxc reply --thread"),
        "and how to answer — the step the whole flow hangs on:\n{rendered}"
    );

    // A persona that is no longer declared still gets its inbox rather than an error.
    let vanished = eng
        .prime_as("local/ghost", Some("ghost"), NOW)
        .expect("a deleted declaration must not cost a session its catch-up");
    assert!(vanished.persona.is_none());
}

// ---- the working-tree lease on `send --to` (nxf 6j6v.303b) --------------------------------------
//
// `send --to <persona>` is THE path the products drive (owner, 2026-08-12), and the epic names its
// receipt in so many words: "`nxc send --to @coder` answers with 'queued behind <scope>, position n'
// instead of a receipt that suggests a started session". An app reads that off `SendToReceipt` and
// nothing else — so it is pinned at this seam, not only at the CLI's (`engine-seam-test-rule`).

// ---- `persona_prime`: what an app's OWN role sessions are told (nxf 6j6v.k8zq) ---------------

/// **The eighth read seam verb, driven from the handle** (review of PR #379, Test Quality #1).
///
/// It shipped with a metadata row in `read_surface.rs` saying it is DOCUMENTED and no test saying
/// it BEHAVES — including its deliberately divergent error contract. app-foundations composes its
/// own role prompts (`crates/agent-runtime`), so this call is the whole of what keeps an app's
/// persona and an `nxc` persona reading the same instructions; an untested seam is exactly the
/// two-contracts split it exists against.
#[test]
fn engine_persona_prime_composes_the_block_and_refuses_a_handle_nobody_declared() {
    let tmp = workspace();
    let defs = Definitions::new(vec![simple_role("pm")], vec![]).unwrap();
    let eng = engine(tmp.path(), defs);

    let block = eng.persona_prime("pm", NOW).expect("a declared persona");
    assert!(
        block.contains("## You are `pm`") && block.contains("## How a conversation moves"),
        "the block carries the persona's identity and how it answers:\n{block}"
    );
    assert!(
        !block.contains("# nexus-flow") && !block.contains("# nexus-memory"),
        "…and NOT the siblings, because this handle was configured with no `module_primes` \
         provider — the honest default, not a silent empty section:\n{block}"
    );

    // **The divergent error contract, which is the half nobody had exercised.** `prime_as` degrades
    // softly for a handle that no longer resolves — a session whose role file vanished mid-flight
    // still deserves its inbox. This does not: the caller is ASKING about a declaration, and an
    // empty string would read as "this persona is told nothing", which is a different fact.
    let err = eng
        .persona_prime("nobody", NOW)
        .expect_err("an undeclared handle is not a persona");
    assert_eq!(err.kind, ErrorKind::NotFound, "{err:?}");
    assert!(
        err.msg.contains("nobody"),
        "the error names it: {}",
        err.msg
    );
    assert!(
        eng.prime_as("local/nobody", Some("nobody"), NOW).is_ok(),
        "…while `prime_as` still degrades softly for the same handle — the two contracts differ \
         on purpose, and that is what this pins"
    );
}

/// `prime: false` reaches the seam too: a persona that opted out composes to nothing at all, and
/// the caller gets an empty string rather than an error. The forced ending is composed by
/// `compose_system_prompt` on the trigger path and is not this read's business.
#[test]
fn engine_persona_prime_of_an_unprimed_persona_is_empty_rather_than_an_error() {
    let tmp = workspace();
    let defs = Definitions::new(
        vec![role("handle: narrow\nsystem_prompt: R\nprime: false\n")],
        vec![],
    )
    .unwrap();
    let eng = engine(tmp.path(), defs);
    assert_eq!(eng.persona_prime("narrow", NOW).unwrap(), "");
}

#[test]
fn engine_send_to_a_busy_exclusive_persona_reports_the_queue_instead_of_a_started_session() {
    let tmp = workspace();
    let dry = tmp.path().join("dry.log");
    // Two `exclusive` personas: each `send --to` opens its OWN thread, so the two sends are two
    // chains competing for one working copy — the epic's "build the website" / "change the button
    // label" pair, arriving through the same door.
    let defs = Definitions::new(
        vec![
            role("handle: coder\nsystem_prompt: You are coder.\nworking_tree: exclusive\n"),
            role("handle: fixer\nsystem_prompt: You are fixer.\nworking_tree: exclusive\n"),
        ],
        vec![],
    )
    .unwrap();
    common::write_declarations(tmp.path(), &defs);
    let eng = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Dry {
                log: Some(dry.clone()),
            },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    let first = eng
        .send_to(caller("carsten"), send_req("coder", "build the website"))
        .unwrap();
    assert_eq!(
        first.queue_position, None,
        "the first chain takes the lease"
    );
    assert_eq!(first.queued_behind, None);

    let second = eng
        .send_to(
            caller("carsten"),
            send_req("fixer", "change the button label"),
        )
        .unwrap();
    assert_eq!(
        second.queued_behind,
        Some(format!("thread:{}", first.thread_id)),
        "the receipt names the chain that holds the working copy"
    );
    assert_eq!(second.queue_position, Some(1));
    assert!(
        second.session.is_some(),
        "the session IS minted — it is what the queued entry will be fired against later"
    );

    // The claim that matters, and the one the receipt alone could not make: the worker was handed
    // exactly ONE trigger. A `session:` in the receipt with no line here is precisely the phantom
    // this field exists to prevent.
    let log = std::fs::read_to_string(&dry).unwrap_or_default();
    let triggers: Vec<&str> = log.lines().filter(|l| l.starts_with("trigger ")).collect();
    assert_eq!(triggers.len(), 1, "only the lease holder spawned:\n{log}");
    assert!(triggers[0].contains("role=coder"), "{log}");
    assert!(
        !log.contains("role=fixer"),
        "the queued persona was never started:\n{log}"
    );
}

#[test]
fn engine_withdraw_takes_back_the_commission_the_queue_receipt_reported() {
    // nxf 6j6v.0djn — the OTHER half of the test above, and the reason `withdraw` stopped being a
    // `CliOnly` waiver in `verb_seam.rs`. That test proves an embedding host REACHES the queue;
    // this one proves it can now answer the receipt it gets, through the handle and nothing else.
    // No `nxc` process is involved anywhere in this test.
    let tmp = workspace();
    let dry = tmp.path().join("dry.log");
    let defs = Definitions::new(
        vec![
            role("handle: coder\nsystem_prompt: You are coder.\nworking_tree: exclusive\n"),
            role("handle: fixer\nsystem_prompt: You are fixer.\nworking_tree: exclusive\n"),
        ],
        vec![],
    )
    .unwrap();
    common::write_declarations(tmp.path(), &defs);
    let eng = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Dry {
                log: Some(dry.clone()),
            },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    eng.send_to(caller("carsten"), send_req("coder", "build the website"))
        .unwrap();
    let queued = eng
        .send_to(
            caller("carsten"),
            send_req("fixer", "change the button label"),
        )
        .unwrap();
    assert_eq!(
        queued.queue_position,
        Some(1),
        "precondition: the host is holding a receipt that says 'not started, 1st in line'"
    );

    // The thread the host was HANDED is what it names — the same value the receipt carries.
    let receipt = eng
        .withdraw(caller("carsten"), &queued.thread_id)
        .expect("the parked commission comes back through the handle");
    assert_eq!(receipt.thread_id, queued.thread_id);
    assert_eq!(
        receipt.withdrawn.len(),
        1,
        "exactly the one parked commission: {receipt:?}"
    );
    assert!(
        receipt.started_meanwhile.is_empty(),
        "nothing was running to be left alone: {receipt:?}"
    );

    // The claim the receipt alone cannot make: it really left the queue, so the lease holder's
    // release has nothing to fire. Asked through the seam's own read rather than the store — this
    // is the board an app renders.
    let report = eng
        .status(NOW, StatusScope::Threads(&[&queued.thread_id]))
        .expect("the board the host renders");
    let board = report
        .operations
        .iter()
        .flat_map(|op| op.threads.iter())
        .find(|b| b.thread_id == queued.thread_id)
        .expect("the withdrawn thread still has a board");
    assert_eq!(
        board.working_tree_queue_position, None,
        "the queue entry is gone, not merely reported gone: {board:?}"
    );

    // And the withdrawal started nothing: the worker still only ever saw the lease holder.
    let log = std::fs::read_to_string(&dry).unwrap_or_default();
    assert!(
        !log.contains("role=fixer"),
        "a withdrawn commission is never spawned:\n{log}"
    );
}

#[test]
fn engine_withdraw_refuses_an_unknown_thread_and_refuses_by_name_a_host_that_cannot_answer_liveness(
) {
    // The rejection half of the verb, ON THE HANDLE — `engine-seam-test-rule`: a CLI test of the
    // same refusal does not cover this seam, because the products speak `Engine::*`. Both sibling
    // writes carry exactly this (`an_unknown_target_is_not_found_and_names_what_was_searched`,
    // `an_unknown_thread_is_not_found_and_names_the_thread`); this is `withdraw`'s.
    //
    // TWO refusals, not one, because they are different answers to different mistakes. The first is
    // the unspecific one every write on this seam gives for an id nobody ever opened. The second
    // used to be THIS verb's own `not_found` — "nothing is parked" — but nxf 6j6v.b9nf changed what
    // that case means under this file's worker: `WorkerConfig::Dry`'s `DryWorker` does not override
    // `Worker::answers_liveness`, so it rests on the trait DEFAULT (`false`) — it does not say
    // sessions are not running, it says it CANNOT ANSWER whether they are (the distinction
    // `every_worker_answers_for_itself.rs`'s module doc argues for its third defaulted method). A
    // commission that took the lease left an unended session in the store, and a host that cannot
    // answer liveness with an unended session in the area may not claim "nothing is running" — it
    // refuses BY NAME instead, a `validation` and not a `not_found`, because what is missing is a
    // capability of this host rather than a fact about the thread. The true `not_found` — a worker
    // that DOES answer liveness and finds nothing, or an area with no unended session at all — is
    // pinned in `withdraw_a_running_round.rs`'s
    // `a_host_that_answers_liveness_and_finds_nothing_running_is_a_not_found`.
    let tmp = workspace();
    let dry = tmp.path().join("dry.log");
    let defs = Definitions::new(
        vec![role(
            "handle: coder\nsystem_prompt: You are coder.\nworking_tree: exclusive\n",
        )],
        vec![],
    )
    .unwrap();
    common::write_declarations(tmp.path(), &defs);
    let eng = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Dry { log: Some(dry) },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    // (1) A thread that was never opened.
    let err = eng
        .withdraw(caller("carsten"), "t-nope")
        .expect_err("no such thread");
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert!(err.msg.contains("no such thread"), "{}", err.msg);

    // (2) A REAL thread whose commission took the lease instead of queueing behind it — the
    // ordinary case, and the one a host will actually hit: nothing is parked, and the dry worker
    // never overrides `answers_liveness`, so it cannot say whether the coder's session is running
    // either way. The session has never reported an end, so `withdraw` refuses by name rather than
    // assume the round is over. (A session the worker DOES report as running is withdrawable since
    // nxf 6j6v.b9nf — `withdraw_a_running_round.rs`.)
    let running = eng
        .send_to(caller("carsten"), send_req("coder", "build the website"))
        .unwrap();
    assert_eq!(
        running.queue_position, None,
        "precondition: this one HOLDS the lease, it is not parked"
    );
    let session = running
        .session
        .clone()
        .expect("the coder started a session");
    let err = eng
        .withdraw(caller("carsten"), &running.thread_id)
        .expect_err("a fact this host cannot establish is not asserted");
    assert_eq!(err.kind, ErrorKind::Validation, "{}", err.msg);
    assert!(
        err.msg.contains(&session) && err.msg.contains("does not answer the liveness question"),
        "it names the session it cannot answer for, and the capability that is missing, not 'no \
         such thread' and not a claim that nothing is running: {}",
        err.msg
    );
}

#[test]
fn engine_send_to_a_shared_persona_is_untouched_by_a_held_working_tree() {
    // The default every declaration written before this epic reads as: `shared` neither takes the
    // lease nor waits for one, whatever else is running.
    let tmp = workspace();
    let dry = tmp.path().join("dry.log");
    let defs = Definitions::new(
        vec![
            role("handle: coder\nsystem_prompt: You are coder.\nworking_tree: exclusive\n"),
            simple_role("pm"),
        ],
        vec![],
    )
    .unwrap();
    common::write_declarations(tmp.path(), &defs);
    let eng = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Dry {
                log: Some(dry.clone()),
            },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    eng.send_to(caller("carsten"), send_req("coder", "build the website"))
        .unwrap();
    let shared = eng
        .send_to(caller("carsten"), send_req("pm", "what is the status?"))
        .unwrap();

    assert_eq!(shared.queue_position, None);
    assert_eq!(shared.queued_behind, None);
    let log = std::fs::read_to_string(&dry).unwrap_or_default();
    assert_eq!(
        log.lines().filter(|l| l.starts_with("trigger ")).count(),
        2,
        "both started:\n{log}"
    );
}

#[test]
fn both_entrances_contend_for_the_same_lease_persona_and_declared_channel() {
    // The epic's acceptance in one test, and the case the first cut of this ticket failed outright
    // (PR review, Finding 1): "Beide Eingaenge streiten um dieselbe Sperre: `send --to <persona>`
    // und `send --to <kanal>`". The channel declares `working_tree: exclusive`; NO member declares
    // anything, exactly as the epic's §4 example is written.
    let tmp = workspace();
    let dry = tmp.path().join("dry.log");
    let defs = Definitions::new(
        vec![
            simple_role("reviewer"),
            role("handle: coder\nsystem_prompt: You are coder.\nworking_tree: exclusive\n"),
        ],
        vec![channel_decl(
            "name: coding\nmembers: [reviewer]\nworking_tree: exclusive\n",
        )],
    )
    .unwrap();
    common::write_declarations(tmp.path(), &defs);
    let eng = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Dry {
                log: Some(dry.clone()),
            },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    // (a) The CHANNEL entrance takes the lease: its board thread is the claim area, and its member
    //     is started even though the member's own declaration says nothing about the working copy.
    let board = eng
        .send_to(caller("carsten"), send_req("coding", "build the website"))
        .unwrap();
    assert_eq!(board.target, TargetKind::Channel);
    let log = std::fs::read_to_string(&dry).unwrap_or_default();
    assert!(
        log.contains("role=reviewer"),
        "the board's member ran:\n{log}"
    );

    // (b) The PERSONA entrance is refused by the lease the CHANNEL holds — one working copy, two
    //     doors, one claim.
    let persona = eng
        .send_to(
            caller("carsten"),
            send_req("coder", "change the button label"),
        )
        .unwrap();
    assert_eq!(
        persona.queued_behind,
        Some(format!("thread:{}", board.thread_id)),
        "the persona is queued behind the CHANNEL's board thread"
    );
    assert_eq!(persona.queue_position, Some(1));
    let log = std::fs::read_to_string(&dry).unwrap_or_default();
    assert!(
        !log.contains("role=coder"),
        "and it really did not start:\n{log}"
    );

    // (c) And the other direction: a SECOND board on the same channel is refused too. Its receipt
    //     cannot say so (one `AskReceipt` covers N triggers — nxf 6j6v.qk5b puts it on the thread
    //     board instead), so the worker log is the evidence.
    eng.send_to(caller("carsten"), send_req("coding", "and now this"))
        .unwrap();
    let log = std::fs::read_to_string(&dry).unwrap_or_default();
    assert_eq!(
        log.lines().filter(|l| l.starts_with("trigger ")).count(),
        1,
        "still exactly one session on this working copy:\n{log}"
    );
}

// ---- the operation status on the seam (nxf 6j6v.a71h §5) ---------------------------------------

#[test]
fn the_operation_tree_and_its_three_states_are_readable_from_the_engine_itself() {
    // a71h §5 is explicit that this is the expensive distinction of the whole consolidation: "ein
    // CLI-Verb zu streichen ist nicht dasselbe wie eine Naht-Lesung zu streichen". An app builds its
    // operation view out of `Engine::status`, so the acceptance for it has to be taken HERE — a
    // suite that shells out to `nxc` proves the path the products do not walk (project rule
    // `engine-seam-test-rule`).
    let tmp = workspace();
    let defs = Definitions::new(vec![simple_role("pm"), simple_role("coder")], vec![]).unwrap();
    let eng = engine(tmp.path(), defs);

    // A human opens the operation: no session, so this is a ROOT.
    let opened = eng
        .send_to(caller("carsten"), send_req("pm", "plan the release"))
        .expect("send --to a persona");
    let pm_session = opened.session.clone().expect("the pm was started");

    // The pm hands the work on FROM ITS OWN THREAD — the parent edge is stamped from the ambient
    // session, so the app passes nothing extra and cannot get it wrong.
    let handed_on = eng
        .send_to(
            Caller {
                session: Some(&pm_session),
                ..caller("pm")
            },
            send_req("coder", "implement it"),
        )
        .expect("send --to from inside a session");

    let report = eng.status(NOW, StatusScope::Workspace).expect("status");
    assert_eq!(report.operations.len(), 1, "one operation: {report:?}");
    let op = &report.operations[0];
    assert_eq!(op.root, opened.thread_id);
    assert!(op.live);
    assert_eq!(
        op.threads
            .iter()
            .map(|t| (t.thread_id.as_str(), t.parent.as_deref(), t.state))
            .collect::<Vec<_>>(),
        vec![
            (opened.thread_id.as_str(), None, ThreadState::Open),
            (
                handed_on.thread_id.as_str(),
                Some(opened.thread_id.as_str()),
                ThreadState::Open,
            ),
        ],
        "the tree, both threads still owing an answer"
    );
    assert!(
        !op.threads[0].awaiting_human,
        "a root still waiting on its AGENT is not waiting on its human"
    );

    // The coder answers, then the pm answers the human: the three states, all derived.
    let coder_session = handed_on.session.clone().expect("the coder was started");
    eng.reply_thread(
        Caller {
            session: Some(&coder_session),
            ..caller("coder")
        },
        reply_req(&handed_on.thread_id, "done"),
    )
    .expect("the coder answers");
    eng.reply_thread(
        Caller {
            session: Some(&pm_session),
            ..caller("pm")
        },
        reply_req(&opened.thread_id, "shipped"),
    )
    .expect("the pm answers the human");

    // …and the same operation, this time named by its LEAF, which is the form an app uses when the
    // user clicked on one thread: the tree comes back from the root down either way.
    let report = eng
        .status(NOW, StatusScope::Threads(&[&handed_on.thread_id]))
        .expect("status --thread");
    let op = &report.operations[0];
    assert_eq!(op.root, opened.thread_id);
    assert_eq!(op.threads[0].state, ThreadState::Answered);
    assert!(
        op.threads[0].awaiting_human,
        "the root has its answer and its human has not acted — the normal end, not a standstill"
    );
    assert_eq!(
        op.threads[1].state,
        ThreadState::Answered,
        "discharged, no child, a parent that owes nothing — and the parent MOVED after it, so this \
         dead end finished rather than failed (nxf 6j6v.93zd)"
    );
    assert!(!op.threads[1].awaiting_human, "only ever true at the top");

    // The third state, at the seam, on the shape that earns it: the pm commissions one more piece of
    // work after the chain has already been closed out to its human, so the coder's answer lands
    // where nothing is left to consume it.
    let stray = eng
        .send_to(
            Caller {
                session: Some(&pm_session),
                ..caller("pm")
            },
            send_req("coder", "and the changelog"),
        )
        .expect("one more commission");
    eng.reply_thread(
        Caller {
            session: stray.session.as_deref(),
            ..caller("coder")
        },
        reply_req(&stray.thread_id, "changelog written"),
    )
    .expect("the coder answers into a chain nobody is waiting on");
    let op = eng
        .status(NOW, StatusScope::Threads(&[&stray.thread_id]))
        .expect("status --thread")
        .operations
        .remove(0);
    assert_eq!(
        op.threads
            .iter()
            .find(|t| t.thread_id == stray.thread_id)
            .expect("the stray thread")
            .state,
        ThreadState::Orphaned,
        "the answer arrived and the consequence did not — the derived place says so"
    );

    // The edge an app follows to WATCH (nxf 6j6v.1q6d, acceptance point 7): the assignee's session
    // rides on the thread, so a consumer holding a thread id reaches `Engine::transcript` without
    // ever being told a session id. The coder answered above, so its return address is in the log.
    assert_eq!(
        op.threads[1].session.as_deref(),
        Some(coder_session.as_str()),
        "the assignee's session, not the requester's: {:?}",
        op.threads[1]
    );
    let followed = eng
        .transcript_page(op.threads[1].session.as_deref().unwrap(), -1, None)
        .expect("and the EXISTING transcript path carries on from there");
    assert_eq!(followed.session, coder_session);

    // The channel form finds the operation by its ROOT's channel, and only there.
    let dm = op.threads[0]
        .channel_id
        .clone()
        .expect("the root's channel");
    assert_eq!(
        eng.status(NOW, StatusScope::Channel(&dm))
            .expect("status --channel")
            .operations
            .len(),
        1
    );
    assert_eq!(
        eng.status(NOW, StatusScope::Channel("no-such-channel"))
            .expect("status --channel")
            .operations
            .len(),
        0
    );

    // And it is a DERIVATION: reading it appends nothing to the op log (a71h §3.3 — no operation
    // record comes into being to replace `workflow_run`). Counted on the workspace db itself, from
    // a second connection, because that is the only place the claim can be checked.
    let ops = || -> i64 {
        rusqlite::Connection::open(tmp.path().join(".nxs").join("db.sqlite"))
            .unwrap()
            .query_row("SELECT COUNT(*) FROM ops", [], |r| r.get(0))
            .unwrap()
    };
    let before = ops();
    eng.status(NOW, StatusScope::Workspace).expect("status");
    eng.status(NOW, StatusScope::Threads(&[&opened.thread_id]))
        .expect("status");
    assert_eq!(ops(), before, "status writes nothing at all");
}

/// The device-local session→thread cursor `facade::parent_thread_of` resolves a parent against
/// (nxf 6j6v.a71h §3.1) — read from a SECOND connection to the same workspace, because it is
/// deliberately not on the seam (it is plumbing, not a capability an app is offered).
fn session_position(dir: &Path, session: &str) -> Option<String> {
    Workspace::resolve(None, dir)
        .unwrap()
        .open_chat_store()
        .unwrap()
        .session_thread(session)
        .unwrap()
}

#[test]
fn a_queued_trigger_does_not_move_the_position_of_a_session_that_is_still_working_elsewhere() {
    // Review finding F2. `record_session_thread` used to sit BESIDE `record_trigger_depth`, before
    // the working-tree gate. That placement is right for the depth — a MAX-based budget can only be
    // conservative when written early — and wrong for a POSITION, which is a plain assignment: a
    // trigger that merely QUEUES starts nothing, so recording it there repoints a session that is at
    // that moment still live in another thread, and its next `send --to` hangs under a thread it has
    // not been handed yet. It matters beyond tidiness: item 1xw1 derives the working-tree lease
    // scope from this tree, so a thread on the wrong parent puts a working copy outside the very
    // subtree the lease exists to protect.
    let tmp = workspace();
    let dry = tmp.path().join("dry.log");
    let defs = Definitions::new(
        vec![
            role("handle: coder\nsystem_prompt: You are coder.\nworking_tree: exclusive\n"),
            role("handle: fixer\nsystem_prompt: You are fixer.\nworking_tree: exclusive\n"),
        ],
        vec![],
    )
    .unwrap();
    common::write_declarations(tmp.path(), &defs);
    let eng = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Dry { log: Some(dry) },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    // The coder takes the working copy, and is standing in the thread it was summoned into.
    let working = eng
        .send_to(caller("carsten"), send_req("coder", "build the website"))
        .unwrap();
    let coder_session = working.session.clone().expect("the coder was started");
    assert_eq!(
        session_position(tmp.path(), &coder_session).as_deref(),
        Some(working.thread_id.as_str()),
        "an admitted trigger records where it put the session"
    );

    // A second chain now holds nothing but a queue slot: `fixer` wants the same working copy.
    let queued = eng
        .send_to(
            caller("carsten"),
            send_req("fixer", "change the button label"),
        )
        .unwrap();
    assert_eq!(queued.queue_position, Some(1), "it really did queue");
    assert_eq!(
        session_position(tmp.path(), &queued.session.clone().unwrap()),
        None,
        "a session that has not been started stands nowhere yet"
    );

    // …and the sharp half: summoning the SAME coder again, into a different thread, also queues
    // behind the lease — and must not move the session that is still working in the first thread.
    //
    // This used to be a `Engine::role_resume` of the LIVE coder session (`send --session`), which
    // made the point more sharply: the very session holding the lease was pushed at from a second
    // thread. Since nxf 6j6v.dvyq §3 that door is gone and no seam call can push into an existing
    // session at all, so the case is stated with the entrance that remains. What survives is the
    // claim that matters here — a queued chain records no position — and what cannot be stated any
    // more is a resume's, because a resume cannot be asked for.
    let elsewhere = eng
        .send_to(caller("carsten"), send_req("coder", "and this too"))
        .unwrap();
    assert!(
        elsewhere.queue_position.is_some(),
        "the second summon queued behind the lease: {elsewhere:?}"
    );
    assert_eq!(
        session_position(tmp.path(), &elsewhere.session.clone().unwrap()),
        None,
        "and it stands nowhere, having never started"
    );
    assert_eq!(
        session_position(tmp.path(), &coder_session).as_deref(),
        Some(working.thread_id.as_str()),
        "the coder is still standing where it is actually working, not where it was merely queued"
    );
}

// ---- a conversation's NAME on the handle (nxf 6j6v.e76c) ---------------------------------------

/// **`Engine::name_thread` at the seam the products actually speak**, which is the whole reason
/// this case is here rather than only on the CLI: an app that shows a conversation list is the
/// consumer the item was opened for, and it reaches this through the handle.
///
/// It drives all three of the rule's halves in one pass — the write, the read it lands on, and the
/// once-only refusal — because they are one contract: a name a surface can rely on is one that does
/// not move.
#[test]
fn engine_name_thread_names_a_conversation_once_and_the_read_carries_it() {
    let tmp = workspace();
    let eng = engine(
        tmp.path(),
        Definitions::new(vec![simple_role("coder")], vec![]).unwrap(),
    );
    let opened = eng
        .send_to(
            caller("carsten"),
            send_req("coder", "Please review the retry loop in the sync driver"),
        )
        .unwrap();

    // A handle's default namer names nothing (`EngineConfig::namer`), so the thread arrives
    // unnamed — which is the state every app must render, and the one this call changes.
    let named = eng
        .name_thread(
            caller("carsten"),
            &opened.thread_id,
            "Review the retry loop",
        )
        .expect("naming a thread this workspace opened");
    assert!(named.named);
    assert_eq!(named.name.as_deref(), Some("Review the retry loop"));

    // The read an app refreshes on carries it — the conversation list's whole reason for asking.
    let status = eng
        .status(NOW, StatusScope::Threads(&[&opened.thread_id]))
        .expect("status");
    let thread = status
        .operations
        .iter()
        .flat_map(|op| op.threads.iter())
        .find(|t| t.thread_id == opened.thread_id)
        .expect("the thread is in its own operation");
    assert_eq!(
        thread.name.as_deref(),
        Some("Review the retry loop"),
        "the name rides on the read every surface is built on"
    );

    // ONCE: a second attempt leaves the name where it is and says so, rather than erroring — the
    // caller may be a retried naming run, and a name that moves under a reader is the defect.
    let again = eng
        .name_thread(caller("carsten"), &opened.thread_id, "Something else")
        .expect("a second attempt is a no-op, never an error");
    assert!(!again.named);
    assert_eq!(again.name.as_deref(), Some("Review the retry loop"));

    // And a thread nobody minted is `not_found`, not a register invented on an unknown id.
    let missing = eng
        .name_thread(caller("carsten"), "m-nope", "whatever")
        .unwrap_err();
    assert_eq!(missing.kind, ErrorKind::NotFound, "{missing:?}");
}
