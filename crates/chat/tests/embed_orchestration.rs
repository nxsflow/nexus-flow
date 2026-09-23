//! The orchestration verbs as they are reachable from the LIBRARY handle (nxf 6j6v.d49b) — the
//! acceptance for the seam app-foundations actually links.
//!
//! `crates/chat/tests/embed.rs` is the messaging half of this file: the same long-lived
//! [`Engine`] a Tauri backend holds in managed state, here driving the role runtime. What only this
//! seam can show — and what neither the black-box CLI suites nor `orchestration_*.rs`'s direct
//! `Ctx` calls can — is that an app reaches the whole runtime through ONE handle, and that the
//! handle's reply verb carries the completion routing it silently dropped before this ticket. (That
//! verb was `Engine::reply` until nxf 6j6v.ckeq cut the writing seam to two; it is
//! `Engine::reply_thread` now, running the same `orchestration::reply` body, so the claim is
//! unchanged and only the door moved.)
//!
//! **The catalogue is no longer injected** (nxf 6j6v.dvyq step 6): this file used to say the app
//! supplied its own definitions with no declaration folder in the user's project at all, and both
//! halves are now false. `DefinitionSource::Supplied` and `Engine::set_definitions` are gone —
//! personas and channels come from `.nxs-personas/` for an app exactly as for the CLI, and even a
//! later in-app role editor writes into that folder. So a case here DECLARES its team — through
//! `common::write_declarations`, or by writing the yaml itself where that is the point
//! (`open_keeps_todays_defaults`) — and then drives the real folder read, which is strictly more
//! than injection could reach.
//!
//! Everything here injects `now`/`origin`/`actor`/`session`/`hop` explicitly, exactly as a host
//! must: there is no ambient `NXC_*` in an app — and since nxf 6j6v.570x there is no exception. The
//! two tests that observe the real trigger stream used to set process-global `NXC_DRY_LOG` under a
//! mutex, because `DryWorker::trigger` read it; they now hand the path to
//! [`WorkerConfig::Dry`] like every other value on this seam, which is what stopped a THIRD test —
//! one that wants no dry log and therefore took no lock — from inheriting a `TempDir` path that was
//! already deleted.

mod common;

use std::path::Path;
use std::sync::Mutex;

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::facade::StatusScope;
use nexus_chat::model::{Disposition, MessageKind, Priority, Refs, ThreadRoot};
use nexus_chat::orchestration::{Caller, ConsequenceClass, WakeSkipReason, MAX_HOP};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToReceipt, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{
    TriggerError, TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig,
};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-08-01T10:00:00Z";

/// The `<origin>` half every qualified handle in `root`'s workspace carries: that workspace's own
/// replica prefix, read back from the workspace rather than spelled (nxf 6j6v.07me).
fn origin(root: &Path) -> String {
    nexus_chat::workspace::origin_of(&Workspace::resolve(None, root).expect("resolve workspace"))
        .to_string()
}

/// `<origin>/<agent>` in `root`'s workspace — what [`Engine`] mints for `agent` there, and
/// therefore also what a store-level fixture seeding a member or an opener into that same
/// workspace has to write.
///
/// A `const ORIGIN: &str = "local"` stood here and every handle in this file was spelled against
/// it. Since the owner's decision of 2026-08-21 (nxf 6j6v.07me) the origin is the replica prefix,
/// random per `TempDir`, so a literal is not merely wrong — it is non-deterministically wrong. And
/// it has to move on BOTH sides at once: the seeds below and the `Engine` calls that act on them
/// share ONE workspace, so a fixture adding `local/bob` as a member while the handle replied as
/// `<prefix>/bob` would reproduce, inside a test, the split member set this item exists to prevent.
fn handle(root: &Path, agent: &str) -> String {
    format!("{}/{}", origin(root), agent)
}

// ---- workspace + declaration helpers ----------------------------------------------------------

/// A fresh chat workspace with **no declaration folder yet** — the state before a team is declared,
/// which is what a case wants when it is about to write one (or about to prove that a workspace
/// declaring nothing behaves).
///
/// Named `_without_roles` from when the folder carried the legacy name `roles/` and an app
/// supplied its catalogue instead of writing one; the assertion below is on `.nxs-personas/` and
/// is the authority.
fn workspace_without_roles() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    assert!(
        !tmp.path().join(".nxs-personas").exists(),
        "the whole point: nothing on disk declares anything"
    );
    tmp
}

/// `send --to <target> "<body>"` at the seam — the ONE entrance a declared channel or a persona
/// has since 6j6v.dvyq §3 removed `Engine::channel_open`/`Engine::ask`.
fn send_req<'a>(to: &'a str, body: &'a str) -> SendToRequest<'a> {
    SendToRequest {
        machine: None,
        to,
        body,
        refs: SendToRefs::ExplicitlyNone,
    }
}

/// The session a PERSONA summon minted. [`SendToReceipt::session`] is an `Option` because a
/// CHANNEL target starts a declared flow rather than one session and reports none; on the persona
/// path it is always set, and every case reaching for it here is on that path.
///
/// It became an `Option` for these cases when nxf 6j6v.dvyq §3 took `Engine::role_trigger` — whose
/// `TriggerReceipt::session` was a plain `String` — and left `send_to` as the one door.
fn minted(receipt: &SendToReceipt) -> &str {
    receipt
        .session
        .as_deref()
        .expect("a persona summon reports the session it minted")
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

/// DECLARE the catalogue in the workspace's own `.nxs-personas/` folder, then hand back the config
/// the handle opens with.
///
/// Named for what it used to do — `EngineConfig { definitions: DefinitionSource::Supplied(defs) }`
/// — and doing the only thing left that means it (nxf 6j6v.dvyq step 6). The injection path is
/// gone: personas and channels come from the folder for an app exactly as for the CLI, so a
/// "supplied" catalogue is now one this helper writes down. Every test in this file therefore
/// drives the REAL read path, which the injection form could never reach.
fn supplied(root: &Path, defs: Definitions, worker: WorkerConfig) -> EngineConfig {
    common::write_declarations(root, &defs);
    EngineConfig {
        worker,
        timer: TimerConfig::Disabled,
        ..EngineConfig::default()
    }
}

/// A store opened SEPARATELY from the handle — for seeding the state a verb under test acts on
/// (channel membership, a pending role session, a board), exactly as `embed.rs` seeds its own.
fn seed_store(dir: &Path) -> nexus_chat::store::ChatStore {
    Workspace::resolve(None, dir)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// Who is calling on ONE call, as the single named-field value every orchestration verb takes. A
/// test that needs a live session writes `Caller { session: Some(&s), ..caller("coder") }` —
/// `Caller` is `Copy`, so varying one field never means restating the rest.
///
/// This says `actor` because these callers are HUMANS with no session. A call made from inside a
/// running persona session names the session instead and lets the session map say which persona it
/// is (nxf 6j6v.07me); `tests/caller_seam.rs` is where that is pinned.
fn caller(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

/// The thread the two wake-classification tests below reply INTO: a plain group-channel thread that
/// declares no board at all, carrying `pm`'s bound session as its return address.
///
/// It used to be a THREADLESS post, replied to by MESSAGE id — the shape `Engine::reply` took and
/// `Engine::reply_thread` cannot (nxf 6j6v.ckeq: a conversation has one address, and it is the
/// thread). The property both tests assert is unchanged by the move: `pm`'s session is still the
/// return address, `orchestration::resume_return_address` is still what classifies the failure to
/// wake it, and the receipt is still the only place an app learns of it. What differs is which
/// lookup finds the address — `surface::thread_return_address` rather than
/// `store.message_return_address` — and neither test was ever about that.
const ADDRESSED_THREAD: &str = "t-addressed";

fn seed_addressed_thread(root: &Path, real_session: &str) {
    let (org, pm, bob) = (origin(root), handle(root, "pm"), handle(root, "bob"));
    let mut s = seed_store(root);
    s.create_pending_session("s-pm", "pm").unwrap();
    s.bind_session("s-pm", real_session).unwrap();
    s.set_channel_field("c-1", "kind", "group", &pm);
    s.add_member("c-1", &pm, &pm);
    s.add_member("c-1", &bob, &pm);
    s.open_thread(
        ADDRESSED_THREAD,
        &ThreadRoot {
            origin: org.clone(),
            channel_id: "c-1".to_string(),
            opener: pm.clone(),
            created: NOW.to_string(),
            parent: None,
        },
        &pm,
    );
    // Deliberately NO `set_expects_reply_from`: nothing declared a board here, so the reply below
    // hands the turn back through the thread's return address rather than through a completion.
    nexus_chat::facade::send(
        &mut s,
        nexus_chat::facade::SendRequest {
            now: NOW,
            origin: &org,
            actor: "pm",
            channel: "c-1",
            body: "a task for you",
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: Some(ADDRESSED_THREAD),
            refs: Refs {
                session_id: Some("s-pm".to_string()),
                ..Default::default()
            },
        },
    )
    .unwrap();
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

// ---- the handle stays what a Tauri backend can hold -------------------------------------------

#[test]
fn engine_is_still_clone_send_sync() {
    // Pinned at COMPILE time, with definitions and a worker aboard: a `Definitions` behind the same
    // lock discipline as the store and an `Arc<dyn Worker>` must not cost the handle its Tauri
    // managed-state bound (`embed.rs`'s own assertion is the pre-orchestration mirror of this).
    fn assert_clone_send_sync<T: Clone + Send + Sync + 'static>() {}
    assert_clone_send_sync::<Engine>();

    // And at run time: a clone of a fully configured handle really does cross a thread boundary.
    let tmp = workspace_without_roles();
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(tmp.path(), defs, WorkerConfig::Dry { log: None }),
    )
    .unwrap();
    let moved = eng.clone();
    let handles = std::thread::spawn(move || moved.definitions().unwrap().roles().len())
        .join()
        .unwrap();
    assert_eq!(handles, 1, "the clone reads the declared catalogue");
}

#[test]
fn open_keeps_todays_defaults() {
    // The compatibility constraint: `Engine::open` is EXACTLY what it was — declarations from the
    // workspace's own folder, and no worker. Existing embedders (messaging + quorum reads) are
    // untouched; orchestration is an opt-in through `open_with`.
    //
    // Since nxf 6j6v.dvyq step 6 the folder is not even a CHOICE any more: `EngineConfig` carries
    // no definition source at all, so "declarations come from the workspace" is a property of the
    // type rather than of its default value.
    assert_eq!(
        EngineConfig::default(),
        EngineConfig {
            worker: WorkerConfig::Disabled,
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
        "the default config IS what `open` uses"
    );

    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    std::fs::create_dir_all(tmp.path().join(".nxs-personas")).unwrap();
    std::fs::write(
        tmp.path().join(".nxs-personas/coder.yaml"),
        "handle: coder\nsystem_prompt: You are coder.\n",
    )
    .unwrap();

    let eng = Engine::open(None, tmp.path()).unwrap();
    assert_eq!(
        eng.definitions().unwrap().roles().len(),
        1,
        "`open` still reads the declarations off disk"
    );
    assert!(eng.definitions().unwrap().role("coder").is_ok());

    // …and the worker is `Disabled`, so a verb that would spawn says so by name — in `warnings`
    // rather than as a bare `Err` (nxf 6j6v.hpv8: `coordinator_commission` has already posted the
    // message and minted the pending session by the time the worker is asked, so the receipt
    // carries the refusal instead of discarding both). See
    // `a_commission_on_a_disabled_worker_still_returns_the_receipt_with_a_warning` for the full
    // assertion; this is just the "still named, not a silent no-op" half.
    let receipt = eng
        .send_to(caller("alice"), send_req("coder", "go"))
        .expect("the message is persisted even though the worker refused — this is Ok, not Err");
    assert!(!receipt.spawned, "{receipt:?}");
    assert_eq!(receipt.warnings.len(), 1, "{receipt:?}");
    assert!(
        receipt.warnings[0]
            .detail
            .contains("no worker is configured"),
        "the named validation error, not a silent no-op: {receipt:?}"
    );
}

// ---- the calling convention: who is calling, ONE named-field parameter -----------------------

#[test]
fn a_caller_is_read_by_field_name_not_by_position() {
    // Why the ambient values became one parameter (owner decision, 2026-08-01): as a fixed-order
    // positional prefix, transposing `origin` and `actor` — the workspace identity access rights
    // will later hang off, and the acting agent — still compiled, and the message came out
    // attributed to someone who never sent it. Two of those three left the caller entirely in nxf
    // 6j6v.07me, and the hazard did NOT leave with them: `session` and `actor` are both
    // `Option<&str>`, so a positional pair would swap in silence just as readily. Here the literal
    // lists the fields in a deliberately SCRAMBLED order; every value must still land exactly where
    // the positional call put it.
    let tmp = workspace_without_roles();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            Definitions::new(vec![simple_role("coder")], vec![]).unwrap(),
            WorkerConfig::Dry { log: None },
        ),
    )
    .unwrap();

    let receipt = eng
        .send_to(
            Caller {
                actor: Some("alice"),
                now: Some(NOW),
                session: Some("s-caller"),
            },
            send_req("coder", "go"),
        )
        .unwrap();

    let msgs = common::channel_messages(tmp.path(), &receipt.channel, &handle(tmp.path(), "alice"))
        .unwrap();
    assert_eq!(msgs.len(), 1, "the task message is where it always was");
    let msg = &msgs[0];
    assert_eq!(
        msg.sender,
        handle(tmp.path(), "alice"),
        "`origin/actor`, in that order — never `alice/local`"
    );
    assert_eq!(
        msg.origin,
        eng.origin(),
        "the minting workspace identity — the HANDLE's, not a value this call could name"
    );
    assert_eq!(msg.created.as_deref(), Some(NOW), "the injected wall clock");
    assert_eq!(
        msg.refs.session_id.as_deref(),
        Some("s-caller"),
        "the caller's own session, stamped as the return address"
    );
    // `s-caller` names no session this workspace minted, which is exactly when `actor` is still
    // read: the session map has no persona to offer, so the caller's own account of itself stands
    // (nxf 6j6v.07me). Where the map DOES answer, it wins — `tests/caller_seam.rs` pins that.

    // And what its doc claims about its cost: `Copy` (so varying one field never restates the rest)
    // and free to carry wherever the handle itself goes.
    fn assert_copy_send_sync<T: Copy + Send + Sync>() {}
    assert_copy_send_sync::<Caller<'static>>();
}

#[test]
fn the_depth_the_next_hop_is_handed_comes_from_the_callers_session() {
    // `Caller.hop` was the one non-string field and the one whose meaning is load-bearing, and nxf
    // 6j6v.07me took it off the caller altogether. Both halves it used to pin are pinned here
    // still, now against the SESSION MAP: what the next hop is handed (the caller session's own
    // recorded depth + 1, stamped into the spawned session's `NXC_HOP`), and the refusal above the
    // cap. The observation point is the sidecar spec file, which `SidecarWorker::trigger` writes
    // before it spawns anything — whether `node` exists on this machine is irrelevant to what the
    // engine stamped.
    //
    // The chain that walks this all the way to the cap one real `send_to` at a time is
    // `tests/caller_seam.rs`; this is the same rule read off a single seeded hop, which is what
    // lets it name the exact stamped number.
    let tmp = workspace_without_roles();
    let logs = tmp.path().join(".nxs/agent-logs");
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            Definitions::new(vec![simple_role("coder")], vec![]).unwrap(),
            WorkerConfig::Sidecar {
                sidecar: tmp.path().join("no-such-sidecar.mjs"),
                cwd: tmp.path().to_path_buf(),
            },
        ),
    )
    .unwrap();
    // A session seven hops deep, recorded the way a real spawn records it.
    {
        let mut store = seed_store(tmp.path());
        store.create_pending_session("s-deep", "alice").unwrap();
        store.record_trigger_depth("s-deep", 7).unwrap();
        store.create_pending_session("s-over", "alice").unwrap();
        store.record_trigger_depth("s-over", MAX_HOP + 1).unwrap();
    }

    let _ = eng.send_to(
        Caller {
            session: Some("s-deep"),
            ..caller("alice")
        },
        send_req("coder", "go"),
    );
    let spec: serde_json::Value = std::fs::read_dir(&logs)
        .expect("the worker wrote its spec file")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with(".spec.json"))
        .map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap())
        .expect("one spec per trigger");
    assert_eq!(
        spec["env"]["NXC_HOP"], "8",
        "the next hop is handed the caller session's recorded depth + 1: {spec}"
    );

    let err = eng
        .send_to(
            Caller {
                session: Some("s-over"),
                ..caller("alice")
            },
            send_req("coder", "go"),
        )
        .map(|_| ())
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains("depth guard"),
        "the cap is refused by name: {}",
        err.msg
    );
}

// ---- expects_reply_from (nxf 6j6v.jepk) --------------------------------------------------------
//
// The engine-seam counterpart of `orchestration_trigger.rs`'s commission suite: manufakt.io
// and nexflow.it reach this behavior through the seam, never through `Ctx` directly, so the
// CLI/`Ctx`-level coverage does not stand in for it (`engine-seam-test-rule`).
//
// The DOOR changed with nxf 6j6v.dvyq §3 — `Engine::role_trigger` is gone and `Engine::send_to` is
// what a persona summon runs through — but the body under it
// (`orchestration::coordinator_commission`)
// and therefore this behavior did not. What the seeded `--thread` reuse in the first case used to
// express has no entrance left at all: `send_to` opens the thread it declares the expectation on,
// which is the whole reason the collapse could happen (a threadless trigger left its answer
// unaddressable by `reply --thread`).

#[test]
fn engine_persona_send_declares_the_summoned_role_expected_on_its_own_thread() {
    let tmp = workspace_without_roles();

    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            Definitions::new(vec![simple_role("coder")], vec![]).unwrap(),
            WorkerConfig::Dry { log: None },
        ),
    )
    .unwrap();

    let receipt = eng
        .send_to(caller("alice"), send_req("coder", "go"))
        .unwrap();
    // nxf 6j6v.hpv8, PR review round 2: this is the one Engine-seam happy path for a persona summon
    // (the products' own path) — `spawned`/`warnings` were pinned on both CLI entry points but not
    // here, so a future `skip_serializing_if` on `warnings` or a `spawned` mis-set on a clean spawn
    // would pass this file untouched.
    assert!(receipt.spawned, "{receipt:?}");
    assert!(receipt.warnings.is_empty(), "{receipt:?}");

    // Through `status` with an EXPLICIT SET since nxf 6j6v.yr59, which is what `thread_quorums`
    // became. `outstanding` is `expects` minus whoever answered, so "nobody has answered yet" is
    // the two lists being equal — the `replied` field the quorum carried is that difference.
    let report = eng
        .status(NOW, StatusScope::Threads(&[&receipt.thread_id]))
        .unwrap();
    let q = &report
        .operations
        .first()
        .expect("the thread exists")
        .threads[0];
    assert_eq!(q.expects, vec![handle(tmp.path(), "coder")], "{q:?}");
    assert_eq!(
        q.outstanding, q.expects,
        "the just-summoned role has not answered yet: {q:?}"
    );
    assert_eq!(q.outstanding, vec![handle(tmp.path(), "coder")]);
    // The receipt says the same thing the register does — the field an app renders instead of
    // re-deriving the quorum itself.
    assert_eq!(
        receipt.expects,
        vec![handle(tmp.path(), "coder")],
        "{receipt:?}"
    );
}

// ---- the working-tree lease (nxf 6j6v.303b) ----------------------------------------------------
//
// The engine-seam counterpart of `orchestration_trigger.rs`'s lease suite, for the same reason the
// section above exists (`engine-seam-test-rule`): a host that summons a busy `exclusive` role gets
// its ONLY signal that nothing started from the typed `TriggerReceipt` it holds — there is no
// `--json` for it to read and no terminal for it to watch — so a receipt that stayed silent here
// would leave an app showing "the coder is on it" over an empty queue slot.

#[test]
fn engine_persona_send_reports_the_working_tree_queue_instead_of_a_phantom_session() {
    let tmp = workspace_without_roles();

    // Two summons on the same direct conversation: two independent chains, one working copy. The
    // threads are the ones `send_to` opens — since nxf 6j6v.dvyq §3 there is no entrance that takes
    // a thread it did not mint, so seeding two by hand would model a shape no caller can produce.
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            Definitions::new(
                vec![serde_yaml::from_str(
                    "handle: coder\nsystem_prompt: You are coder.\nworking_tree: exclusive\n",
                )
                .unwrap()],
                vec![],
            )
            .unwrap(),
            WorkerConfig::Dry { log: None },
        ),
    )
    .unwrap();

    let first = eng
        .send_to(caller("alice"), send_req("coder", "build the website"))
        .unwrap();
    assert_eq!(
        first.queue_position, None,
        "the first chain takes the lease"
    );
    assert_eq!(first.queued_behind, None);

    let second = eng
        .send_to(
            caller("alice"),
            send_req("coder", "change the button label"),
        )
        .unwrap();
    assert_eq!(
        second.queued_behind,
        Some(format!("thread:{}", first.thread_id)),
        "the second names the chain holding the working copy"
    );
    assert_eq!(second.queue_position, Some(1));
    assert!(
        !second.message_id.is_empty(),
        "the MESSAGE is persisted either way — what did not happen is the spawn"
    );
}

// `engine_role_resume_reports_the_working_tree_queue_too` stood here. REMOVED with its subject by
// nxf 6j6v.dvyq §3: `Engine::role_resume` is on §5's closed list of what leaves the seam, and a test
// whose door no longer exists cannot be migrated to another one — `send_to` opens a FRESH session,
// which is the opposite of the resume this pinned.
//
// What it claimed — a resume is scoped by the THREAD it pushes into, so pushing into a second
// thread queues behind the first while pushing back into the holding one inherits the lease — is
// unchanged in `orchestration::role_resume`, whose body stays.
//
// THAT PAIR IS NO LONGER PINNED ANYWHERE, and saying so is the point of this note (PR review of
// this block, Test Quality Medium: an earlier draft of it claimed the coverage "stays at the `Ctx`
// level", and that was checked and is not true). What survives is the GENERIC gate — acquire,
// inherit, queue — exercised through `trigger_role` in `orchestration_trigger.rs`, and the release
// rule for a scope that owes nothing, in `orchestration_reply.rs`'s
// `a_resume_holds_a_scope_that_owes_nothing_and_only_a_reply_in_that_scope_frees_it`. What is gone
// is the split observed through `role_resume`'s OWN wiring (`RoleSpawn { thread:
// receipt.thread_id, .. }`).
//
// Not replaced with a `Ctx`-level test, deliberately: `role_resume` has no caller left outside
// this crate's own tests, so such a test would pin a behaviour no surface can reach — the shape
// this whole item exists to remove. If nxf 6j6v.xr3z ever gives that body a surface again
// (`reply --thread <id> --force`), the test belongs there, written against the entrance that
// actually reaches it, and this note is the pointer to what it has to cover.

#[test]
fn engine_reply_releases_the_working_tree_and_starts_the_chain_that_was_waiting() {
    // The engine seam for nxf 6j6v.fe0f (`engine-seam-test-rule`): the release rides
    // `Engine::reply_thread` — the call manufakt.io and nexflow.it actually make, and since nxf
    // 6j6v.ckeq the only reply the seam has — not `orchestration::reply`'s `Ctx`,
    // which `orchestration_reply.rs` covers and which does not stand in for this. A host that
    // queues a second chain and then relays the first chain's answer must see the second one start;
    // there is no daemon and no `--json` here to watch, so the dry worker's log is the evidence.
    let tmp = workspace_without_roles();
    let dry = tmp.path().join("dry.log");

    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            Definitions::new(
                vec![serde_yaml::from_str(
                    "handle: coder\nsystem_prompt: You are coder.\nworking_tree: exclusive\n",
                )
                .unwrap()],
                vec![],
            )
            .unwrap(),
            WorkerConfig::Dry {
                log: Some(dry.clone()),
            },
        ),
    )
    .unwrap();

    // Two chains on one working copy. Each summon opens its OWN thread — since nxf 6j6v.dvyq §3 no
    // entrance takes a thread it did not mint, so the two ids come off the receipts.
    let first = eng
        .send_to(caller("alice"), send_req("coder", "build the website"))
        .unwrap();
    let queued = eng
        .send_to(
            caller("alice"),
            send_req("coder", "change the button label"),
        )
        .unwrap();
    assert_eq!(queued.queue_position, Some(1), "the second chain waits");
    let started = |log: &str| log.lines().filter(|l| l.starts_with("trigger ")).count();
    assert_eq!(
        started(&std::fs::read_to_string(&dry).unwrap()),
        1,
        "and nothing is running for it yet"
    );

    // The first chain answers its own obligation — through the handle, as a host relays it.
    let receipt = eng
        .reply_thread(caller("coder"), reply_req(&first.thread_id, "shipped"))
        .unwrap();

    assert!(receipt.posted);
    let log = std::fs::read_to_string(&dry).unwrap();
    assert_eq!(
        started(&log),
        2,
        "the reply that discharged the holder's last expectation started the waiting chain:\n{log}"
    );
    assert!(
        log.contains(&format!(
            "session={}",
            queued
                .session
                .as_deref()
                .expect("a persona summon reports its session")
        )),
        "and it is the session the queued receipt already named:\n{log}"
    );
    let store = seed_store(tmp.path());
    assert_eq!(
        store.working_tree_holder(NOW).unwrap(),
        Some(format!("thread:{}", queued.thread_id)),
        "the working copy is the promoted chain's now"
    );
    assert_eq!(store.working_tree_queue_len().unwrap(), 0);
}

// `engine_reply_if_unanswered_posts_once_then_is_a_silent_no_op` stood here, under a section header
// that read "the Engine seam for `ReplyRequest::if_unanswered` (`engine-seam-test-rule`): manufakt.io
// and nexflow.it reach this behaviour through `Engine::reply`". REMOVED with the field and the
// method both (nxf 6j6v.ckeq, decision 4).
//
// The premise stopped being true, and that is the removal: `if_unanswered` is not a behaviour a host
// reaches for. It is the idempotency switch of ONE caller — `agent-sidecar/src/main.mjs`'s teardown,
// which runs `nxc reply --thread <id> --if-unanswered <text>` when an SDK session ends without
// answering the thread it owed, unconditionally, because it cannot tell whether the session already
// answered. An app holds its own sessions and knows. The mechanism keeps its name
// (`surface::settle_if_unanswered`), its CLI flag and its coverage, in the shape its only caller
// uses: `surface_cli.rs`'s own `reply_thread_if_unanswered_*` pair, and 6j6v.ckeq's acceptance point
// `surface_cli.rs::the_sidecar_teardown_still_writes_its_failure_reply_when_a_session_ends_unanswered`.

// ---- the whole point: one handle drives a run to completion -----------------------------------

// `one_handle_drives_a_whole_workflow_to_completion` stood here — one `Engine` driving a declared
// run from `workflow_start` to `done`. REMOVED with the run record (6j6v.dvyq §3). The same claim
// on the surviving surface is
// `channel_flow.rs::a_declared_flow_runs_the_fixed_order_that_used_to_need_a_run`.

/// The shared fixture the tests below open an engine over: a `coder`, a `bob`, and a declared
/// `review` channel `bob` answers. It carried a declared `ship` WORKFLOW too until 6j6v.dvyq §3;
/// what the tests around it actually need from it is a role to summon and a channel to fan out to.
fn ship_definitions() -> Definitions {
    Definitions::new(
        vec![simple_role("coder"), simple_role("bob")],
        vec![channel_decl(
            "name: review\nmembers: [bob]\non_complete: pass_through\n",
        )],
    )
    .unwrap()
}

// ---- runtime replacement ----------------------------------------------------------------------

#[test]
fn a_declaration_written_to_the_folder_is_visible_to_the_next_verb_and_to_an_existing_clone() {
    // The realistic desktop case: a user edits an agent in the app. The next trigger must see it
    // WITHOUT the app tearing down its handle, its `subscribe` receiver and its UI state — and a
    // clone taken BEFORE the edit is the same handle, so it must see it too.
    //
    // This used to be `Engine::set_definitions`. With the injection path gone (nxf 6j6v.dvyq step
    // 6) the property is unchanged and its mechanism is simpler: nothing is cached, so an edit to
    // `.nxs-personas/` IS what the next verb reads.
    let tmp = workspace_without_roles();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            Definitions::default(),
            WorkerConfig::Dry { log: None },
        ),
    )
    .unwrap();
    let clone = eng.clone();

    let err = eng
        .send_to(caller("alice"), send_req("coder", "go"))
        .map(|_| ())
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound, "nothing is declared yet");

    common::replace_declarations(
        tmp.path(),
        &Definitions::new(vec![simple_role("coder")], vec![]).unwrap(),
    );
    assert!(
        eng.definitions().unwrap().role("coder").is_ok(),
        "the catalogue read back through the handle is the edited one"
    );

    // The NEXT verb call on the clone — never reopened — resolves the freshly declared role.
    let receipt = clone
        .send_to(caller("alice"), send_req("coder", "go"))
        .unwrap();
    assert!(receipt.channel.starts_with("dm:"), "auto-opened DM");
    assert_eq!(
        clone
            .transcript_page(
                receipt
                    .session
                    .as_deref()
                    .expect("a persona summon reports its session"),
                -1,
                None,
            )
            .unwrap()
            .role
            .as_deref(),
        Some("coder"),
        "the trigger really minted a pending session for the edited declaration"
    );
    assert_eq!(
        common::channel_messages(tmp.path(), &receipt.channel, &handle(tmp.path(), "alice"))
            .unwrap()
            .len(),
        1,
        "and the task message is readable back through the same handle"
    );
}

/// Two catalogues that differ ONLY in `coder`'s prompt marker, and BOTH declare every handle the
/// racing verbs below resolve — so a `not_found` from a verb can only mean it read a catalogue that
/// never existed as a whole.
fn catalogue(tag: &str) -> Definitions {
    Definitions::new(
        vec![
            role(&format!(
                "handle: coder\nsystem_prompt: You are coder {tag}.\n"
            )),
            simple_role("reviewer"),
        ],
        vec![],
    )
    .unwrap()
}

#[test]
fn a_folder_edit_races_an_in_flight_verb_without_deadlocking_or_losing_a_write() {
    // Test-quality review of PR #265 (#1) pinned this against `Engine::defs`'s lock discipline —
    // "neither lock is ever HELD while the other is acquired". That lock is GONE with the injection
    // path (nxf 6j6v.dvyq step 6), and the hazard it guarded moved rather than vanished: the
    // catalogue is now read off the FILESYSTEM per call, and that read must still happen before the
    // store lock is taken. A future edit that resolved declarations INSIDE `with_state_mut` (the
    // same obvious "simplification") would hold the store across a directory walk, and nothing else
    // in this file would notice.
    //
    // **What this test can show**: with four threads driving a real verb while two others rewrite
    // the declaration folder underneath them, the handle does not deadlock, does not panic, every
    // verb resolves a role that exists in EVERY catalogue written here, and the store ends up with
    // exactly one message per trigger — no lost and no duplicated write.
    //
    // **What it cannot show, and is not claimed**: that the store lock and the filesystem read
    // never overlap. That is a statement about all interleavings; this is one scheduler and a
    // finite number of iterations. The failure mode it is most likely to catch — a verb holding the
    // store while the folder is being rewritten — would surface as a HUNG or erroring test rather
    // than a failed assertion, which is evidence, not proof. The discipline itself remains a
    // code-level invariant readable in `resolve_definitions`/`ctx_inputs`: the catalogue is
    // resolved before `with_state_mut` is ever entered.
    //
    // A rewrite is not atomic on a filesystem the way a whole-value replace was in memory, so the
    // two catalogues here differ ONLY in `coder`'s prompt marker and BOTH declare every handle a
    // verb below resolves — a reader that catches the folder mid-write still finds a usable role,
    // and what is under test is the locking, not YAML atomicity.
    // `log: None` — this storm wants no dry log at all, and since 6j6v.570x that is something it
    // can simply SAY. It used to have to take the env lock instead, purely so its 160 triggers did
    // not land in a concurrent test's log file (or, once that test's `TempDir` was gone, fail on a
    // path that no longer existed — which is the flake this ticket closed).
    let tmp = workspace_without_roles();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(tmp.path(), catalogue("v1"), WorkerConfig::Dry { log: None }),
    )
    .unwrap();

    // One call up front, purely to learn the auto-opened DM every later trigger reuses.
    let channel = eng
        .send_to(caller("alice"), send_req("coder", "warm up"))
        .unwrap()
        .channel;

    const VERB_THREADS: usize = 4;
    const ITERATIONS: usize = 40;

    let root = tmp.path();
    std::thread::scope(|scope| {
        for w in 0..2 {
            let e = &eng;
            scope.spawn(move || {
                for i in 0..ITERATIONS {
                    let tag = if (i + w) % 2 == 0 { "v1" } else { "v2" };
                    // OVERWRITE, not replace: both catalogues declare the same handles, so there is
                    // nothing to delete, and `write_declarations` renames each file into place
                    // atomically. A `remove_dir_all` + rewrite would leave a window in which the
                    // folder genuinely has no `coder.yaml`, and a reader landing there would be
                    // reporting the test's own teardown rather than a locking defect.
                    common::write_declarations(root, &catalogue(tag));
                    let _ = e;
                }
            });
        }
        for t in 0..VERB_THREADS {
            let e = &eng;
            scope.spawn(move || {
                for i in 0..ITERATIONS {
                    let body = format!("t{t}-{i}");
                    if let Err(err) = e.send_to(caller("alice"), send_req("coder", &body)) {
                        panic!(
                            "a verb resolved against a catalogue that was never installed as a \
                             whole: {err:?}"
                        );
                    }
                }
            });
        }
    });

    let posted =
        common::channel_messages(tmp.path(), &channel, &handle(tmp.path(), "alice")).unwrap();
    assert_eq!(
        posted.len(),
        1 + VERB_THREADS * ITERATIONS,
        "every trigger posted exactly once — no write lost or doubled under the replacement storm"
    );

    // The handle is still usable, and still serves ONE of the two catalogues that were written —
    // a sanity check on the surviving state.
    let defs = eng.definitions().unwrap();
    assert_eq!(defs.roles().len(), 2);
    let prompt = defs.role("coder").unwrap().system_prompt.clone();
    assert!(
        prompt == "You are coder v1." || prompt == "You are coder v2.",
        "the surviving catalogue is one of the two that were installed: {prompt:?}"
    );
    eng.send_to(caller("alice"), send_req("reviewer", "still working"))
        .expect("the handle still serves verbs after the storm");
}

#[test]
fn editing_the_declaration_folder_changes_which_prompt_the_next_trigger_composes() {
    // The observation point for a COMPOSED prompt is the sidecar spec file, which
    // `SidecarWorker::trigger` writes before it spawns anything (the dry worker's log records the
    // trigger, not the prompt). The spawn itself is deliberately NOT asserted on: whether `node`
    // exists on this machine is irrelevant to what the engine composed, and the spec is already on
    // disk either way.
    let tmp = workspace_without_roles();
    let logs = tmp.path().join(".nxs/agent-logs");
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            Definitions::new(
                vec![role("handle: coder\nsystem_prompt: You are coder v1.\n")],
                vec![],
            )
            .unwrap(),
            WorkerConfig::Sidecar {
                sidecar: tmp.path().join("no-such-sidecar.mjs"),
                cwd: tmp.path().to_path_buf(),
            },
        ),
    )
    .unwrap();

    let _ = eng.send_to(caller("alice"), send_req("coder", "one"));
    common::replace_declarations(
        tmp.path(),
        &Definitions::new(
            vec![role("handle: coder\nsystem_prompt: You are coder v2.\n")],
            vec![],
        )
        .unwrap(),
    );
    let _ = eng.send_to(caller("alice"), send_req("coder", "two"));

    let prompts: Vec<String> = std::fs::read_dir(&logs)
        .expect("the worker wrote its spec files")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".spec.json"))
        .map(|p| {
            let spec: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            spec["systemPrompt"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    assert_eq!(prompts.len(), 2, "one spec per trigger");
    assert!(
        prompts.iter().any(|p| p.contains("You are coder v1.")),
        "the first trigger composed the ORIGINAL declaration: {prompts:?}"
    );
    assert!(
        prompts.iter().any(|p| p.contains("You are coder v2.")),
        "and the next one composed the REPLACED declaration: {prompts:?}"
    );
}

// ---- the Disabled worker ----------------------------------------------------------------------

#[test]
fn a_disabled_worker_leaves_the_reads_intact_and_the_refusals_unchanged() {
    // "A `Disabled` worker means reads and messaging writes behave identically" — the compatibility
    // half of the opt-in. Nothing about a handle that never asked for orchestration changes.
    //
    // The message is SEEDED through the store rather than posted through the handle since nxf
    // 6j6v.ckeq: `Engine::send` — the raw post into a channel id — is gone, and `c-1` is a plain
    // substrate channel no declaration names, so there is no verb that addresses it. That does not
    // weaken the claim; it sharpens what the claim is about. What a `Disabled` handle must still do
    // with such a channel is READ it, which is what is asserted below.
    //
    // **The name said `…leaves_reads_and_messaging_writes_identical` until nxf 6j6v.4d2z, and by
    // then it was a claim this body could not make** (PR #450 review, Test Quality #3). The one
    // successful messaging write left that needs no worker was `mark_read`, and it went with the
    // unread apparatus; both surviving messaging writes SUMMON, so on a `Disabled` handle there is
    // no successful one left to demonstrate. What the body actually proves — and what is worth
    // proving — is that the reads are untouched and that a write which fails fails for ITS OWN
    // reason (an unknown thread is `not_found`) rather than with a worker complaint. Renamed to
    // that rather than propped up with a write it cannot do.
    let tmp = workspace_without_roles();
    let alice = handle(tmp.path(), "alice");
    {
        let mut s = seed_store(tmp.path());
        s.set_channel_field("c-1", "kind", "group", &alice);
        s.add_member("c-1", &alice, &alice);
        s.set_wall_clock(NOW);
        s.post_message(&nexus_chat::model::MessageEnvelope {
            origin: origin(tmp.path()),
            channel_id: "c-1".into(),
            sender: alice.clone(),
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: None,
            refs: Refs::default(),
            body: "hi".into(),
        });
    }
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            Definitions::new(vec![simple_role("coder")], vec![]).unwrap(),
            WorkerConfig::Disabled,
        ),
    )
    .unwrap();

    assert_eq!(
        common::channel_messages(tmp.path(), "c-1", &alice)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(common::channels_of(tmp.path(), &alice).unwrap().len(), 1);
    // A messaging WRITE that needed no worker stood here — `mark_read`, advancing the synced read
    // cursor. It went with the unread apparatus (nxf 6j6v.4d2z), and every write left on this
    // handle either starts a session or is answered by the reads above.

    // A reply that needs no spawn is unaffected too: an unknown thread is still `not_found`, not
    // the worker complaint.
    let err = eng
        .reply_thread(caller("alice"), reply_req("m-nope", "hi"))
        .map(|_| ())
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

#[test]
fn a_spawning_verb_on_a_disabled_worker_returns_the_named_validation_error() {
    // …and the other half: a verb that WOULD spawn a session fails loudly, by name, instead of
    // silently doing nothing behind a successful receipt.
    //
    // the persona commission is deliberately NOT in this loop any more (nxf 6j6v.hpv8, split out below):
    // `Disabled` refusing the spawn is reached from INSIDE `Worker::trigger`, exactly like every
    // other worker failure — by the time it fires, `coordinator_commission` has already posted the
    // message and minted the pending session, so an opaque `Err` here was always this ticket's bug
    // too, just with "no worker configured" as its cause instead of a real spawn failing. Only
    // `coordinator_commission`'s own callers get the fix (see that function's doc for the exact
    // scope);
    // `channel_open`'s fan-out calls `trigger_role` directly and is untouched.
    let tmp = workspace_without_roles();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(tmp.path(), ship_definitions(), WorkerConfig::Disabled),
    )
    .unwrap();

    // ONE verb here, where there were two: this was a loop over `workflow_start` and `send_to`
    // until 6j6v.dvyq §3 removed the first. The CLAIM is still about a class — every verb that
    // would spawn refuses by NAME rather than silently doing nothing — and `send_to` is the only
    // member of it left, so the loop went with the second verb rather than being kept as a loop
    // over one element.
    let err = eng
        .send_to(caller("alice"), send_req("review", "please review"))
        .map(|_| ())
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation, "{}", err.msg);
    assert!(
        err.msg.contains("no worker is configured"),
        "the error names the missing configuration: {}",
        err.msg
    );
}

#[test]
fn a_commission_on_a_disabled_worker_still_returns_the_receipt_with_a_warning() {
    // The commission case split out of the test above (nxf 6j6v.hpv8): a `Disabled` engine
    // refuses INSIDE `Worker::trigger`, which by construction runs AFTER `coordinator_commission`
    // has
    // already posted the message and minted the pending session — the exact shape the bug names,
    // regardless of WHY the worker refused. The receipt is `Ok`, names the failure in `warnings`,
    // and the message really is there to be read back.
    let tmp = workspace_without_roles();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(tmp.path(), ship_definitions(), WorkerConfig::Disabled),
    )
    .unwrap();

    let receipt = eng
        .send_to(caller("alice"), send_req("coder", "go"))
        .expect("the message is persisted even though the worker refused — this is Ok, not Err");
    assert!(receipt.message_id.starts_with("m-"), "{receipt:?}");
    assert!(
        !receipt.spawned,
        "the typed discriminator, not just non-empty warnings: {receipt:?}"
    );
    assert_eq!(receipt.warnings.len(), 1, "{receipt:?}");
    assert!(
        receipt.warnings[0]
            .detail
            .contains("no worker is configured"),
        "the warning names the same refusal the old Err carried: {receipt:?}"
    );
    // nxf 6j6v.6m6x, the class this receipt used to lose: WHY it did not start is a VALUE now.
    assert_eq!(
        receipt.warnings[0].class,
        ConsequenceClass::StepSkipped,
        "{receipt:?}"
    );
    assert_eq!(
        receipt.warnings[0].reason,
        Some(WakeSkipReason::TriggerFailed),
        "a worker that refuses is retryable plumbing, not a vanished session: {receipt:?}"
    );
    assert_eq!(
        receipt.warnings[0].session.as_deref(),
        receipt.session.as_deref(),
        "…and it names the session that was minted and never started: {receipt:?}"
    );

    let posted =
        common::channel_messages(tmp.path(), &receipt.channel, &handle(tmp.path(), "alice"))
            .unwrap();
    assert_eq!(
        posted.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
        vec!["go"]
    );
}

/// A worker that fails EVERY trigger — the Engine-seam fixture nxf 6j6v.hpv8 points at
/// (`crates/chat/tests/worker.rs`'s `HostWorker::new(|_| Err(...))`, private to that binary).
/// Unlike the `Disabled`-worker tests above (no worker configured at all), this stands in for a
/// REAL worker that was asked to spawn and refused — the shape the bug's own description names
/// ("Schlaegt der Spawn fehl…").
struct FailingWorker;
impl Worker for FailingWorker {
    fn trigger(&self, _req: TriggerRequest) -> TriggerResult {
        Err(TriggerError::Failed(nexus_chat::error::NxfError::io(
            "the runtime refused this trigger",
        )))
    }
}

#[test]
fn a_trigger_that_did_not_start_names_the_runtimes_own_two_classes_apart() {
    // **nxf 6j6v.6m6x, closed here.** Since nxf 6j6v.t5w2/6j6v.hpv8 a failed role trigger returns
    // its receipt instead of an `Err` — and the `Err` it replaced carried an `ErrorKind` that a
    // formatted string cannot give back. The runtime itself already distinguishes ONE class on
    // purpose (`TriggerError::SessionGone`, because a vanished session needs a fresh one rather than
    // a retry), and that distinction stopped reaching the receipt.
    //
    // It reaches it again as [`FailedConsequence::reason`] (nxf 6j6v.93zd's record), classified by
    // the same `wake_skip_reason` the wake sites go through — so "why did this not start" has ONE
    // answer across the engine rather than one per site. One test per distinguished class, at the
    // Engine seam; the CLI seam's half is in `surface_cli.rs`, where only `trigger_failed` is
    // reachable (a bare `nxc send` mints a fresh session, so there is none for a runtime to have
    // reaped).
    let tmp = workspace_without_roles();
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();

    // Class 1: the spawn plumbing failed. Retryable against this same session.
    let refused = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            defs.clone(),
            WorkerConfig::Custom(std::sync::Arc::new(FailingWorker)),
        ),
    )
    .unwrap()
    .send_to(caller("alice"), send_req("coder", "go"))
    .expect("the receipt, not an Err");
    assert_eq!(
        refused.warnings[0].reason,
        Some(WakeSkipReason::TriggerFailed),
        "{refused:?}"
    );

    // Class 2: the runtime session is GONE. Never retryable — it needs a fresh session, which is
    // exactly why the runtime bothers to tell the two apart.
    let reaped = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            defs,
            WorkerConfig::Custom(RemoteRuntimeWorker::reaped()),
        ),
    )
    .unwrap()
    .send_to(caller("alice"), send_req("coder", "go"))
    .expect("the receipt, not an Err");
    assert_eq!(
        reaped.warnings[0].reason,
        Some(WakeSkipReason::SessionGone),
        "{reaped:?}"
    );

    // Both are the same CLASS of consequence — a step that did not start — and both name the session
    // that was minted for it, which is what a caller needs to act on either answer.
    for receipt in [&refused, &reaped] {
        assert!(!receipt.spawned, "{receipt:?}");
        assert_eq!(receipt.warnings.len(), 1, "{receipt:?}");
        assert_eq!(
            receipt.warnings[0].class,
            ConsequenceClass::StepSkipped,
            "{receipt:?}"
        );
        assert_eq!(
            receipt.warnings[0].session.as_deref(),
            receipt.session.as_deref(),
            "{receipt:?}"
        );
    }
}

#[test]
fn a_commission_returns_the_receipt_with_a_warning_when_a_real_worker_refuses_the_spawn() {
    // nxf 6j6v.hpv8, `send --role`'s Engine seam: by the time `trigger_role` calls the worker,
    // `coordinator_commission` has already posted the message and minted the pending session, so a
    // refusing worker must not turn into a bare `Err` that discards both. This is the DELIBERATE
    // shape for `Engine::role_trigger`/`Engine::send_to` (see `TriggerReceipt::spawned`'s own doc):
    // the call still returns `Ok`, and `spawned: false` — the typed discriminator, never omitted —
    // is what a caller checks instead of the exit code `nxc` uses for the same fact; `warnings` is
    // the human-readable detail beside it.
    let tmp = workspace_without_roles();
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            defs,
            WorkerConfig::Custom(std::sync::Arc::new(FailingWorker)),
        ),
    )
    .unwrap();

    let receipt = eng
        .send_to(caller("alice"), send_req("coder", "ship the thing"))
        .expect("the message is persisted even though the spawn failed — this is Ok, not Err");

    assert!(receipt.message_id.starts_with("m-"), "{receipt:?}");
    assert!(
        receipt.session.as_deref().is_some_and(|s| !s.is_empty()),
        "a session was minted: {receipt:?}"
    );
    assert!(!receipt.spawned, "{receipt:?}");
    assert_eq!(receipt.warnings.len(), 1, "{receipt:?}");
    assert!(
        receipt.warnings[0]
            .detail
            .contains("the runtime refused this trigger"),
        "{receipt:?}"
    );
    assert_eq!(
        receipt.warnings[0].reason,
        Some(WakeSkipReason::TriggerFailed),
        "nxf 6j6v.6m6x: a configured worker that failed the spawn, told apart from a session that \
         is gone — the ONE distinction the runtime itself makes: {receipt:?}"
    );

    // The message really landed, read back through the SAME handle.
    let msgs = common::channel_messages(tmp.path(), &receipt.channel, &handle(tmp.path(), "alice"))
        .unwrap();
    assert_eq!(
        msgs.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
        vec!["ship the thing"]
    );
}

// `send --to <persona>`'s Engine-seam equivalent lives in `embed_surface.rs` (nxf 6j6v.hpv8) —
// `send_to_a_persona_reports_a_warning_when_a_real_worker_refuses_the_spawn` — because that file,
// not this one, is what the `engine-seam-test-rule` designates for `send --to`/`reply --thread`/
// `list`/`prime --persona`: "the acceptance for `send --to` … has to be taken HERE" (its own
// doc). The commission's two tests above stay in this file, alongside every other `send_to`
// acceptance.

#[test]
fn a_completing_reply_on_a_disabled_worker_still_posts_and_reports_the_skip() {
    // The interaction the two halves have to survive together: `reply`'s completion wake is
    // "skip, don't fail" by contract, so a `Disabled` engine must still POST the completing reply
    // and route the declared channel's completion — and then say honestly, in the receipt, that
    // nothing was woken.
    let tmp = workspace_without_roles();
    let (org, pm, bob) = (
        origin(tmp.path()),
        handle(tmp.path(), "pm"),
        handle(tmp.path(), "bob"),
    );
    let tid = {
        let mut s = seed_store(tmp.path());
        s.create_pending_session("s-pm", "pm").unwrap();
        s.set_channel_field("decl:review", "kind", "group", &pm);
        s.add_member("decl:review", &pm, &pm);
        s.add_member("decl:review", &bob, &pm);
        nexus_chat::facade::ask(
            &mut s,
            nexus_chat::facade::AskRequest {
                now: NOW,
                origin: &org,
                actor: "pm",
                channel: "decl:review",
                body: "please review",
                expect: std::slice::from_ref(&bob),
                deadline: None,
                kind: MessageKind::Question,
                priority: Priority::Normal,
                refs: Refs {
                    session_id: Some("s-pm".to_string()),
                    ..Default::default()
                },
            },
        )
        .unwrap()
        .thread_id
    };

    let defs = Definitions::new(
        vec![simple_role("pm"), simple_role("bob")],
        vec![channel_decl(
            "name: review\nmembers: [bob]\non_complete: pass_through\n",
        )],
    )
    .unwrap();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(tmp.path(), defs, WorkerConfig::Disabled),
    )
    .unwrap();

    let receipt = eng
        .reply_thread(caller("bob"), reply_req(&tid, "lgtm"))
        .expect("the reply is persisted even though the wake cannot spawn");
    assert!(receipt.posted);
    assert!(receipt.message_id.as_deref().unwrap().starts_with("m-"));
    assert_eq!(
        receipt.woke, None,
        "the wake was skipped, and the receipt says so instead of leaving the app to guess"
    );
    assert!(
        receipt
            .completed
            .as_ref()
            .expect("the board's completion was still routed")
            .delivered
            .contains("lgtm"),
        "{:?}",
        receipt.completed
    );
    // The reply itself is real content, readable back through the same handle.
    assert!(
        common::channel_messages(tmp.path(), "decl:review", &bob)
            .unwrap()
            .iter()
            .any(|m| m.body == "lgtm"),
        "the completing reply is persisted regardless of the wake"
    );
    assert_eq!(
        receipt.wake_skipped.as_ref().map(|s| &s.reason),
        Some(&WakeSkipReason::TriggerFailed),
        "the app learns WHY nothing was woken: {:?}",
        receipt.wake_skipped
    );
}

#[test]
fn a_completing_reply_into_a_plain_channel_reports_the_skip_to_the_app_too() {
    // nxf 6j6v.bxdd through the seam the decision was made for. The NON-declared quorum wake used to
    // propagate its trigger failure, so this exact call — an app replying into an ordinary channel
    // whose opener cannot be spawned — came back as `Err` with the message already written. It is
    // now the same "skip, don't fail" the declared path above gives, and the finding reaches the
    // embedding app through the receipt it already gets: stderr is the CLI's diagnostic channel, not
    // an app's.
    let tmp = workspace_without_roles();
    let (org, pm, bob) = (
        origin(tmp.path()),
        handle(tmp.path(), "pm"),
        handle(tmp.path(), "bob"),
    );
    let tid = {
        let mut s = seed_store(tmp.path());
        s.create_pending_session("s-pm", "pm").unwrap();
        s.bind_session("s-pm", "real-pm").unwrap();
        s.set_channel_field("c-1", "kind", "group", &pm);
        s.add_member("c-1", &pm, &pm);
        s.add_member("c-1", &bob, &pm);
        nexus_chat::facade::ask(
            &mut s,
            nexus_chat::facade::AskRequest {
                now: NOW,
                origin: &org,
                actor: "pm",
                channel: "c-1",
                body: "please review",
                expect: std::slice::from_ref(&bob),
                deadline: None,
                kind: MessageKind::Question,
                priority: Priority::Normal,
                refs: Refs {
                    session_id: Some("s-pm".to_string()),
                    ..Default::default()
                },
            },
        )
        .unwrap()
        .thread_id
    };

    let defs = Definitions::new(vec![simple_role("pm"), simple_role("bob")], vec![]).unwrap();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(tmp.path(), defs, WorkerConfig::Disabled),
    )
    .unwrap();

    let receipt = eng
        .reply_thread(caller("bob"), reply_req(&tid, "lgtm"))
        .expect("the reply is persisted, so it must not come back as an error");
    assert_eq!(receipt.woke, None);
    assert_eq!(
        receipt.completed, None,
        "a plain channel has no on_complete policy"
    );
    let skipped = receipt
        .wake_skipped
        .expect("the app is told the hand-off did not happen");
    assert_eq!(skipped.session, "s-pm");
    assert_eq!(skipped.reason, WakeSkipReason::TriggerFailed);
    assert!(
        skipped
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("no worker is configured")),
        "the DisabledWorker's own refusal travels with it: {:?}",
        skipped.detail
    );
    assert!(
        common::channel_messages(tmp.path(), "c-1", &bob)
            .unwrap()
            .iter()
            .any(|m| m.body == "lgtm"),
        "the reply IS content, which is the whole argument against returning Err"
    );
}

#[test]
fn a_failing_direct_resume_reports_the_skip_to_the_app_instead_of_failing() {
    // nxf 6j6v.0akf at the same seam, on the path an app hits far more often than any board: a
    // plain 1:1 reply to a message that names a session to continue. It used to come back as `Err`
    // with the message already written — and `DisabledWorker`'s own doc claims the opposite, since
    // a `Disabled` engine refuses INSIDE `Worker::trigger`, exactly where "skip, don't fail" is
    // implemented. That claim is what this pins.
    let tmp = workspace_without_roles();
    seed_addressed_thread(tmp.path(), "real-pm");

    let defs = Definitions::new(vec![simple_role("pm"), simple_role("bob")], vec![]).unwrap();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(tmp.path(), defs, WorkerConfig::Disabled),
    )
    .unwrap();

    let receipt = eng
        .reply_thread(caller("bob"), reply_req(ADDRESSED_THREAD, "which db?"))
        .expect("the reply is persisted, so it must not come back as an error");
    assert!(!receipt.resumed, "the resume did not land");
    assert_eq!(receipt.woke, None);
    let skipped = receipt.wake_skipped.expect(
        "…and `resumed: false` alone cannot tell the app that, so the finding must be here",
    );
    assert_eq!(skipped.session, "s-pm");
    assert_eq!(skipped.reason, WakeSkipReason::TriggerFailed);
    assert!(
        common::channel_messages(tmp.path(), "c-1", &handle(tmp.path(), "bob"))
            .unwrap()
            .iter()
            .any(|m| m.body == "which db?"),
        "the reply IS content, which is the whole argument against returning Err"
    );
}

// `a_non_dm_thread_with_no_expects_still_resumes_the_return_address_directly` stood here. MOVED to
// `orchestration_reply.rs` by nxf 6j6v.ckeq rather than deleted — the regression it guards (round 2
// of the 6j6v.jepk review: `is_quorum_thread` keyed on the channel alone pulled every non-DM thread
// with an empty `expects_reply_from` into the quorum branch and skipped the direct 1:1 resume) is
// only VISIBLE on a message-addressed reply, and its own comment already said why: "`reply --thread`
// has its own fallback that masks this". With `Engine::reply` gone the seam takes a thread and
// nothing else, so the question can no longer be asked from here at all — and asking it through
// `reply_thread` would have left a green test that proves the fallback works and nothing about the
// guard.

#[test]
fn reply_through_the_handle_completes_a_board_and_wakes_the_opener() {
    // The regression the epic exists to close: the handle's reply called `facade::reply` and
    // stopped, so an app replying through the handle left boards that never completed and requesters
    // that were never woken — silently, behind a successful receipt. Now the same call resumes the
    // opener's own session, and the receipt names what it woke. (The method was `Engine::reply`
    // then and is `Engine::reply_thread` now — nxf 6j6v.ckeq — over the same body.)
    let tmp = workspace_without_roles();
    let dry = tmp.path().join("dry.log");
    let (org, pm, bob) = (
        origin(tmp.path()),
        handle(tmp.path(), "pm"),
        handle(tmp.path(), "bob"),
    );

    let tid = {
        let mut s = seed_store(tmp.path());
        s.create_pending_session("s-pm", "pm").unwrap();
        s.bind_session("s-pm", "real-pm").unwrap();
        s.set_channel_field("c-1", "kind", "group", &pm);
        s.add_member("c-1", &pm, &pm);
        s.add_member("c-1", &bob, &pm);
        nexus_chat::facade::ask(
            &mut s,
            nexus_chat::facade::AskRequest {
                now: NOW,
                origin: &org,
                actor: "pm",
                channel: "c-1",
                body: "please review",
                expect: std::slice::from_ref(&bob),
                deadline: None,
                kind: MessageKind::Question,
                priority: Priority::Normal,
                refs: Refs {
                    session_id: Some("s-pm".to_string()),
                    ..Default::default()
                },
            },
        )
        .unwrap()
        .thread_id
    };

    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            Definitions::new(vec![simple_role("pm"), simple_role("bob")], vec![]).unwrap(),
            WorkerConfig::Dry {
                log: Some(dry.clone()),
            },
        ),
    )
    .unwrap();

    // Before: the board is outstanding and its opener has nothing to act on.
    assert!(!common::quorums_of(tmp.path(), &[&tid], NOW).unwrap()[0].complete);
    assert!(eng
        .prime_as(&pm, None, NOW)
        .unwrap()
        .wake
        .complete
        .is_empty());

    let receipt = eng
        .reply_thread(caller("bob"), reply_req(&tid, "approved"))
        .unwrap();

    assert_eq!(
        receipt.woke.as_deref(),
        Some("s-pm"),
        "the completing reply woke the board's opener"
    );
    assert!(
        !receipt.resumed,
        "a quorum board defers to the completion wake, never the direct return-address resume"
    );
    assert_eq!(
        receipt.completed, None,
        "a plain channel has no declared on_complete policy to route through"
    );
    assert!(
        common::quorums_of(tmp.path(), &[&tid], NOW).unwrap()[0].complete,
        "the board completed"
    );

    // The wake really reached the worker: the opener's session, resumed on its real SDK id, with a
    // synthesized wake message rather than the reviewer's raw text.
    let log = std::fs::read_to_string(&dry).unwrap_or_default();
    let woke_line = log
        .lines()
        .find(|l| l.contains("session=s-pm"))
        .unwrap_or_else(|| panic!("no wake trigger recorded in: {log}"));
    assert!(woke_line.contains("role=pm"), "{woke_line}");
    assert!(woke_line.contains("resume=real-pm"), "{woke_line}");
    assert!(woke_line.contains("(1/1 replied)"), "{woke_line}");
    assert!(
        !woke_line.contains("approved"),
        "a synthesized wake, not the raw reply body: {woke_line}"
    );
}

// ---- the remaining verbs are wired to the same implementation ----------------------------------

#[test]
fn send_to_is_the_only_path_and_it_resolves_the_declaration() {
    // Was `send_stays_the_raw_post_while_channel_open_is_the_declared_path`, and it asserted the
    // CONTRAST: `Engine::send` took an explicit channel id and resolved no declaration, so a
    // declared channel's NAME was simply an unknown channel to it, while the declared path
    // materialised `decl:<name>` and fanned out. There is no contrast left to draw — `Engine::send`
    // is gone (nxf 6j6v.ckeq) — so what survives is the half that was always the point: naming a
    // declared channel resolves the DECLARATION, materialises the channel with its members joined,
    // and opens the board under the channel's own policy.
    //
    // The raw post's own refusal did not go with it, and is not this test's any more: `send_to`
    // refuses a channel that EXISTS but that no declaration names, by name, in
    // `orchestration_verbs.rs::send_to_refuses_a_channel_that_exists_but_no_declaration_names`.
    let tmp = workspace_without_roles();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            ship_definitions(),
            WorkerConfig::Dry { log: None },
        ),
    )
    .unwrap();

    let receipt = eng
        .send_to(caller("alice"), send_req("review", "please review"))
        .unwrap();
    assert_eq!(
        receipt.expects,
        vec![handle(tmp.path(), "__channel__")],
        "the channel thread has two ends: the requester and the supervisor (nxf 6j6v.pf6j)"
    );
    assert_eq!(
        common::channel_messages(tmp.path(), "decl:review", "bob")
            .unwrap()
            .len(),
        2,
        "the declared channel was materialised with its declared member joined, and carries both \
         levels: the request and the supervisor's own send to bob"
    );
}

#[test]
fn send_to_reaches_the_shared_implementation_on_both_of_its_branches() {
    // One walk over the verbs the other tests do not drive, so a mis-wired delegate cannot hide:
    // each must reach the SAME `orchestration` body the CLI reaches, with its own receipt.
    let tmp = workspace_without_roles();
    let alice = handle(tmp.path(), "alice");
    {
        let mut s = seed_store(tmp.path());
        s.set_channel_field("c-1", "kind", "group", &alice);
        s.add_member("c-1", &alice, &alice);
    }
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            ship_definitions(),
            WorkerConfig::Dry { log: None },
        ),
    )
    .unwrap();

    // send --to a DECLARED channel: opens the board and fans out under the channel's own policy.
    // This replaces `ask` on a plain channel id, which stood here until 6j6v.dvyq §3 — that verb
    // needed an explicit `--expect` precisely BECAUSE nothing declared who answers, which is the
    // shape the consolidation removes. The channel thread expects the SUPERVISOR (nxf 6j6v.pf6j).
    let board = eng
        .send_to(caller("alice"), send_req("review", "who is on this?"))
        .unwrap();
    assert_eq!(board.expects, vec![handle(tmp.path(), "__channel__")]);

    // A `tick` on that fresh board was walked here too until 6j6v.dvyq §3 took `Engine::tick` off
    // the seam with §5's closed list. The verb survives on the CLI alone — nobody but the scheduled
    // `at` job types it — and `channel_tick.rs` drives it through the binary.

    // send --to a DECLARED PERSONA: the other branch of the same verb, reaching
    // `orchestration::coordinator_commission`. `Engine::role_trigger` and `Engine::role_resume`
    // were
    // walked here too until nxf 6j6v.dvyq §3 took both off the seam.
    let first = eng
        .send_to(caller("alice"), send_req("coder", "start"))
        .unwrap();
    let second = eng
        .send_to(caller("alice"), send_req("coder", "keep going"))
        .unwrap();
    assert_eq!(
        first.channel, second.channel,
        "the same deterministic DM, materialised once"
    );
    assert_eq!(
        common::channel_messages(tmp.path(), &first.channel, &alice)
            .unwrap()
            .len(),
        2,
        "both turns are posted"
    );
    // **A FRESH session each time, and a fresh thread with it.** This is what the removal of
    // `role_resume` costs, stated where it can be seen rather than left as an inference: the seam
    // has no way left to push a second turn into the session the first summon minted. Continuing an
    // EXISTING conversation is `reply --thread`, which needs the persona to have answered first;
    // delivering into a session that is mid-turn is nxf 6j6v.xr3z.
    assert_ne!(first.session, second.session, "a summon always mints");
    assert_ne!(first.thread_id, second.thread_id);
    assert!(first.session.is_some() && second.session.is_some());
}

// ---- the host brings its own worker (nxf 6j6v.5x9j) --------------------------------------------
//
// Spec §3.3's escape hatch, finally reachable — and in the shape the Bedrock AgentCore probe
// (app-foundations 41j0.9t68) said a REMOTE runtime needs, not the shape that was closest to hand.
// This is the seam an app docks a second agent runtime onto, so the acceptance for it belongs here,
// at the library handle app-foundations actually links, and not at a direct `Ctx` call.

/// One trigger as the host worker below saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SeenTrigger {
    /// Which persona was put to work. Recorded since the flow test below, which is about the ORDER
    /// members start in and can say nothing without it.
    role: String,
    internal_session: String,
    resume_real: Option<String>,
    message: String,
    model: Option<String>,
}

/// A host worker standing in for a remote agent runtime: it never spawns a process, it answers with
/// an opaque session id its own runtime minted, and it records what the engine handed it.
struct RemoteRuntimeWorker {
    seen: Mutex<Vec<SeenTrigger>>,
    /// What the runtime answers. `Ok(id)` provisions and hands back that opaque id; `Err(())`
    /// stands for the session having been reaped out from under the caller.
    answer: std::result::Result<String, ()>,
    /// The sessions this runtime still has running — what
    /// [`Worker::session_is_running`](nexus_chat::worker::Worker::session_is_running) answers from
    /// (nxf 6j6v.10yb; review of PR #361, Integrity #3).
    ///
    /// It is here because this type is the PATTERN a host integrator copies — its own doc says so —
    /// and leaving it on the trait default would teach the omission that already cost this project a
    /// production incident: `cli.rs`'s `LazyWorker` forwarded only `trigger`, the suite stayed green,
    /// and the shipped path was inert until a live run caught it. A reference implementation that
    /// answers every method is the cheapest place to say "answer every method".
    running: Mutex<std::collections::HashSet<String>>,
}

impl RemoteRuntimeWorker {
    fn provisioning(runtime_session: &str) -> std::sync::Arc<RemoteRuntimeWorker> {
        std::sync::Arc::new(RemoteRuntimeWorker {
            seen: Mutex::new(vec![]),
            answer: Ok(runtime_session.to_string()),
            running: Mutex::new(std::collections::HashSet::new()),
        })
    }
    fn reaped() -> std::sync::Arc<RemoteRuntimeWorker> {
        std::sync::Arc::new(RemoteRuntimeWorker {
            seen: Mutex::new(vec![]),
            answer: Err(()),
            running: Mutex::new(std::collections::HashSet::new()),
        })
    }
}

impl Worker for RemoteRuntimeWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(SeenTrigger {
                role: req.role.handle.clone(),
                internal_session: req.internal_session.clone(),
                resume_real: req.resume_real.clone(),
                message: req.message.clone(),
                model: req.role.model.map(|m| m.sdk_id().to_string()),
            });
        match &self.answer {
            Ok(runtime_session) => {
                // A provisioned session is RUNNING until this runtime says otherwise — the fact the
                // engine asks for below, and the half a host that only implements `trigger` never
                // supplies.
                self.running
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(req.internal_session.clone());
                Ok(TriggerOutcome::Started {
                    runtime_session: runtime_session.clone(),
                })
            }
            Err(()) => Err(TriggerError::SessionGone {
                session: req.resume_real.unwrap_or_else(|| "<none>".into()),
                detail: "runtime reaped the session after 15 minutes idle".into(),
            }),
        }
    }

    /// **A host worker answers this too** (nxf 6j6v.10yb) — see the `running` field for why the
    /// reference implementation must, rather than resting on the trait's `false` default.
    fn session_is_running(&self, internal_session: &str) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(internal_session)
    }
}

#[test]
fn a_declared_sequential_flow_runs_its_order_through_the_engine_handle_alone() {
    // **THE MECHANISM THIS WHOLE ITEM REPLACED THE RUN ENGINE WITH, at the seam an app actually
    // talks to** (nxf 6j6v.dvyq §3, blocks c+d; `NEXUS_MEMORY.md`'s `engine-seam-test-rule`).
    //
    // `channel_flow.rs::a_declared_flow_runs_the_fixed_order_that_used_to_need_a_run` proves the
    // same claim through the `nxc` BINARY, and that does not stand in for this one: manufakt.io and
    // nexflow.it reach `Engine::*` directly, and the ticket names manufakt.io's own workflow tab as
    // the next consumer that has to migrate onto exactly this mechanism. The deleted
    // `one_handle_drives_a_whole_workflow_to_completion` proved the OLD mechanism's
    // one-handle-drives-many-steps property here; this is its successor, and without it that
    // property would have left the seam with the runs.
    //
    // The claim, in one sentence: **a second step fires only when the first has answered, and both
    // happen through `Engine::send_to` / `Engine::reply_thread` with no CLI anywhere.**
    let tmp = workspace_without_roles();
    let host = RemoteRuntimeWorker::provisioning("runtime://sess-flow");
    let defs = Definitions::new(
        vec![simple_role("coder"), simple_role("qa")],
        vec![
            channel_decl("name: review\nmembers: [qa]\non_complete: pass_through\n"),
            // A role step, then a CHANNEL step — the shape every declared workflow's graph had.
            channel_decl("name: ship\nmembers: [coder, review]\nflow: sequential\n"),
        ],
    )
    .unwrap();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(tmp.path(), defs, WorkerConfig::Custom(host.clone())),
    )
    .unwrap();

    // Step one: sending to the channel opens the operation AND starts its first member. Nothing
    // else — `flow: sequential` is the order, and `qa` has not been asked anything yet.
    let board = eng
        .send_to(caller("alice"), send_req("ship", "ship the release"))
        .expect("the declared channel takes the work order");
    assert_eq!(
        board.session, None,
        "a CHANNEL target starts a flow rather than one session, so it reports none"
    );
    let started = |host: &RemoteRuntimeWorker| -> Vec<String> {
        host.seen
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.role.clone())
            .collect()
    };
    assert_eq!(
        started(&host),
        vec!["coder".to_string()],
        "the declared order's FIRST member, and only it"
    );

    // The coder answers on its own thread — through the handle, as the persona the supervisor
    // commissioned. Finding its thread is the store read an app would do too (`Engine::status`
    // renders the same tree); the SESSION is what the worker was handed, which is what makes this
    // caller the persona rather than a stranger posting into somebody's board.
    let coder_session = host.seen.lock().unwrap()[0].internal_session.clone();
    let coder_thread = {
        let store = seed_store(tmp.path());
        store
            .supervised_children(&board.thread_id)
            .unwrap()
            .into_iter()
            .find(|t| {
                store
                    .thread_quorum(t, NOW)
                    .unwrap()
                    .is_some_and(|q| q.expects == [handle(tmp.path(), "coder")])
            })
            .expect("the supervisor opened a thread for the first member")
    };
    eng.reply_thread(
        Caller {
            // The session is a real one the host worker was handed, so the session map already says
            // it is `coder`'s — `..caller("coder")` leaves the bare name behind it as the fallback
            // that is not consulted (nxf 6j6v.07me).
            session: Some(&coder_session),
            ..caller("coder")
        },
        ReplyThreadRequest {
            machine: None,
            thread: &coder_thread,
            body: "implemented",
            escalate: false,
            needs_rework: false,
            accept: false,
        },
    )
    .expect("the member answers on its own thread");

    // Step two fired, and that is the whole mechanism: the supervisor read the answer, saw the set
    // settled, and commissioned the next member of the declared order — a CHANNEL, whose own
    // supervisor fanned out to its member. No second verb, no run record, no branching token.
    assert_eq!(
        started(&host),
        vec!["coder".to_string(), "qa".to_string()],
        "the reply moved the flow on to its second step, which is a channel of its own"
    );
}

#[test]
fn a_host_supplied_worker_drives_an_orchestration_verb_and_binds_its_own_session_id() {
    // The ticket's definition of done, at the seam that matters: a host implements `Worker`, hands
    // it to `Engine` through `WorkerConfig::Custom`, and a real orchestration verb drives it — with
    // no sidecar, no `node`, no process anywhere in the picture. The opaque id the runtime hands
    // back is bound to the internal session by the engine, which is what makes a LATER resume able
    // to address a session that was never a local process.
    let tmp = workspace_without_roles();
    let host = RemoteRuntimeWorker::provisioning("agentcore://sess-7f3a");
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(tmp.path(), defs, WorkerConfig::Custom(host.clone())),
    )
    .unwrap();

    let receipt = eng
        .send_to(caller("alice"), send_req("coder", "ship the thing"))
        .expect("the host worker started the session");

    let seen = host.seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "the engine drove the HOST's worker");
    assert_eq!(
        seen[0].internal_session,
        minted(&receipt),
        "…for the session it minted"
    );
    assert_eq!(seen[0].resume_real, None, "a fresh trigger resumes nothing");
    // The caller's words PLUS the reply obligation naming the thread — `surface::wake_message`.
    // `Engine::role_trigger` handed the worker the bare body and this line asserted it; that door
    // went with nxf 6j6v.dvyq §3, and the difference is exactly why the collapse went this way
    // round: a persona told which thread to answer into produces an answer `reply --thread` can
    // address, and a threadless trigger did not.
    // Since nxf 6j6v.dq59 the plumbing line is the COORDINATOR'S ADDRESSEE BLOCK, composed against
    // the conversations that are actually open for the summoned persona, so the claim is stated as
    // "the body, then the block that names this thread" rather than against a fixed sentence.
    let wake = &seen[0].message;
    assert!(
        wake.starts_with("ship the thing\n\n"),
        "the host worker is handed the wake message, not the raw body: {wake}"
    );
    assert!(
        wake.contains(&format!("nxc reply --thread {}", receipt.thread_id)),
        "…and the wake names the thread it must answer into: {wake}"
    );
    drop(seen);

    assert_eq!(
        seed_store(tmp.path())
            .resolve_real(minted(&receipt))
            .unwrap()
            .as_deref(),
        Some("agentcore://sess-7f3a"),
        "the runtime's own OPAQUE id is bound — the engine stores it and never parses it"
    );
}

#[test]
fn a_vanished_runtime_session_is_reported_as_its_own_wake_skip_not_as_a_trigger_failure() {
    // Requirement 2 of the probe, all the way out to the app: a cloud session dies silently at 15
    // minutes idle, so a resume that finds nothing there is ROUTINE — and it calls for the opposite
    // response to a failed spawn (start fresh / escalate, never retry the same session). Before
    // this ticket both collapsed into `trigger_failed` and an app had only the free-text `detail`
    // to tell them apart.
    let tmp = workspace_without_roles();
    seed_addressed_thread(tmp.path(), "agentcore://sess-dead");

    let defs = Definitions::new(vec![simple_role("pm"), simple_role("bob")], vec![]).unwrap();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(
            tmp.path(),
            defs,
            WorkerConfig::Custom(RemoteRuntimeWorker::reaped()),
        ),
    )
    .unwrap();

    let receipt = eng
        .reply_thread(caller("bob"), reply_req(ADDRESSED_THREAD, "which db?"))
        .expect("the reply is persisted, so `skip, don't fail` still holds");
    let skipped = receipt.wake_skipped.expect("the skip is reported");
    assert_eq!(skipped.session, "s-pm");
    assert_eq!(
        skipped.reason,
        WakeSkipReason::SessionGone,
        "the vanished session is its own case — not `trigger_failed`"
    );
    assert!(
        skipped
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("agentcore://sess-dead"),
        "and it names the runtime session that is gone: {skipped:?}"
    );
}

/// A host worker that behaves the way [`Worker`]'s rules say a remote one must: it answers
/// `Accepted` immediately and does the binding LATER, from its own thread, exactly as a host would
/// once its provisioning call resolves.
struct AsyncBindingWorker {
    /// Filled after the `Engine` exists — the same chicken-and-egg a real host has (it constructs
    /// its worker, opens the handle with it, and only then can hand the worker a way back in).
    engine: std::sync::OnceLock<Engine>,
    threads: Mutex<Vec<std::thread::JoinHandle<Result<(), nexus_chat::error::NxfError>>>>,
}

impl Worker for AsyncBindingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        let engine = self
            .engine
            .get()
            .expect("the host installs the handle before driving a verb")
            .clone();
        let internal = req.internal_session.clone();
        // The whole point: this call is what a real host's executor callback does, and it happens
        // while the verb that triggered it is STILL RUNNING and still holding the engine's store
        // mutex. It must block and then succeed — never deadlock.
        self.threads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(std::thread::spawn(move || {
                engine.bind_runtime_session(&internal, "agentcore://sess-async")
            }));
        Ok(TriggerOutcome::Accepted)
    }
}

#[test]
fn a_host_worker_binds_from_its_own_thread_while_the_verb_that_triggered_it_is_still_running() {
    // PR #269 review, Test Quality #2 / Integrity #1: the `Accepted` → `bind_runtime_session` flow
    // was only ever exercised sequentially on one thread, which is precisely NOT the situation the
    // design exists for. A remote host's provisioning call resolves on its own executor, and the
    // callback it makes lands whenever it lands — including mid-verb, against a handle whose single
    // store mutex is held by the thread that called `trigger` in the first place.
    //
    // This pins that it BLOCKS AND THEN SUCCEEDS rather than deadlocking, and that the handle is
    // still usable afterwards. It is also the test that would have caught the shape the review
    // warned about: the unsupported pattern is the same call made from INSIDE `trigger`, on the
    // caller's own thread, which is why `Worker`'s rule 2 says to return `Started` instead.
    let tmp = workspace_without_roles();
    let host = std::sync::Arc::new(AsyncBindingWorker {
        engine: std::sync::OnceLock::new(),
        threads: Mutex::new(vec![]),
    });
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(tmp.path(), defs, WorkerConfig::Custom(host.clone())),
    )
    .unwrap();
    host.engine
        .set(eng.clone())
        .unwrap_or_else(|_| panic!("the handle is installed once"));

    let receipt = eng
        .send_to(caller("alice"), send_req("coder", "ship the thing"))
        .expect("the verb returns rather than hanging on the host's callback");

    for t in host
        .threads
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
    {
        t.join()
            .expect("the binding thread did not panic")
            .expect("…and the binding itself succeeded once the verb released the store");
    }

    assert_eq!(
        seed_store(tmp.path())
            .resolve_real(minted(&receipt))
            .unwrap()
            .as_deref(),
        Some("agentcore://sess-async"),
        "the id the host reported out of band is bound"
    );
    // The handle survives its own re-entry: a further verb still works on this clone and on a new one.
    assert_eq!(eng.definitions().unwrap().roles().len(), 1);
    assert!(eng
        .send_to(caller("alice"), send_req("coder", "and again"))
        .is_ok());
}

#[test]
fn bind_runtime_session_completes_a_binding_the_worker_could_not_answer_synchronously() {
    // Requirement 1, made concrete. A remote host's `trigger` hands its provisioning call to its own
    // executor and answers `Accepted` — nothing about the engine is async, and nothing blocks a
    // network round-trip on the caller's thread. This is the callback it uses when the call
    // resolves; without it, a host worker had no way to report an id at all, because the local
    // route (the spawned session calling `nxc session bind` from inside itself) does not exist for
    // a session that is not on this machine.
    let tmp = workspace_without_roles();
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();
    let eng = Engine::open_with(
        None,
        tmp.path(),
        supplied(tmp.path(), defs, WorkerConfig::Dry { log: None }),
    )
    .unwrap();

    let receipt = eng
        .send_to(caller("alice"), send_req("coder", "ship the thing"))
        .unwrap();
    assert_eq!(
        seed_store(tmp.path())
            .resolve_real(minted(&receipt))
            .unwrap(),
        None,
        "nothing is bound yet — the worker answered `Accepted`"
    );

    eng.bind_runtime_session(minted(&receipt), "agentcore://sess-late")
        .expect("the host reports the id its runtime handed back");
    assert_eq!(
        seed_store(tmp.path())
            .resolve_real(minted(&receipt))
            .unwrap()
            .as_deref(),
        Some("agentcore://sess-late")
    );

    // A binding against a session nobody minted must not silently succeed.
    let err = eng
        .bind_runtime_session("never-minted", "agentcore://x")
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}
