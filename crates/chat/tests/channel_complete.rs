//! Channel completion + `on_complete` (nxf ticket 6j6v.ja81): when a declared channel's quorum
//! completes, `pass_through` wakes the requester with the raw collected replies; `summarize` spawns
//! an ephemeral, non-declared synthesizer session and delivers ONLY its summary (+ a structured
//! `outcome:` token for a `WorkflowRun` requester). Since `Worker::trigger` is fire-and-forget
//! (`worker.rs`'s own doc comment — "Detached: spawn and DO NOT wait"), `summarize` is necessarily
//! TWO-PHASE: phase 1 (a real quorum completes) re-targets `expects_reply_from` to a single
//! synthetic, QUALIFIED identity (`local/__synth__`) and triggers the synthesizer, delivering
//! nothing yet; phase 2 (that SAME thread's `expects` already equals the synthesis marker) is the
//! synthesizer's own later reply landing, which `cli.rs`'s `reply` PUSH-wake block recognizes and
//! routes back through the same orchestration function to actually deliver.
//!
//! Most tests here call [`nexus_chat::orchestration::on_channel_complete`] DIRECTLY (it lived in
//! `cli.rs` until nxf 6j6v.7dw6 moved it behind an explicit `Ctx`), after seeding a real thread via
//! `facade::ask`/`facade::reply` (public API, not a subprocess). The context is built per test by
//! [`TestCtx`] from this test's own temp workspace — its `.nxs-personas/` folder, its db path — so
//! nothing in the function under test resolves a workspace from the CURRENT PROCESS working
//! directory, which is shared and arbitrary under `cargo test`'s default multi-threaded runner.
//!
//! Process env (`NXC_WORKER`/`NXC_SIDECAR`/`NXC_DRY_LOG`) is NOT similarly isolated — it IS
//! process-global — so every DIRECT (non-subprocess) test acquires [`lock_env`] for its whole body,
//! serializing against every other direct test in this file (mirrors `session_map.rs`'s own noted
//! precedent: "serialize with a lock if this ever races another env test in the same binary").
//! `[EnvGuard]` clears every var it's responsible for on drop, even across a panicking assertion, so
//! one failing test can never leak a stale env value into the next.
//!
//! One test (`live_reply_path_...`) exercises the LIVE `nxc reply` CLI path end-to-end via real
//! subprocesses (mirrors `channel_fanout.rs`'s black-box style — each subprocess is its own
//! isolated process, no shared-env hazard), proving the `reply` PUSH-wake routing modification
//! itself works, not just the orchestration function in isolation. A second live-CLI test proves
//! the non-declared-channel fallback is byte-identical to before this ticket (the regression guard
//! the brief asks for, alongside `verbs.rs`'s own pre-existing equivalent coverage).

use assert_cmd::Command;
use nexus_chat::channel::{
    ChannelDecl, ChannelKind, CompletionOutcome, Consolidation, Expects, Flow, OnComplete,
    Visibility, DELIVERED_HANDLE, SYNTHESIS_HANDLE,
};
use nexus_chat::definitions::Definitions;
use nexus_chat::facade::{self, AskRequest, ReplyRequest};
use nexus_chat::model::{Disposition, MessageKind, Priority, Refs};
use nexus_chat::orchestration::{on_channel_complete, CheckedHop, Ctx};
use nexus_chat::store::ChatStore;
use nexus_chat::worker::{TriggerRequest, Worker};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
use tempfile::TempDir;

const NOW: &str = "2026-07-22T00:00:00Z";

// ---- direct-call test serialization (see module docs) ----------------------------------------

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard<'a>(#[allow(dead_code)] std::sync::MutexGuard<'a, ()>);

impl Drop for EnvGuard<'_> {
    fn drop(&mut self) {
        for k in [
            "NXC_WORKER",
            "NXC_SIDECAR",
            "NXC_DRY_LOG",
            "NXC_ORIGIN",
            "NXC_NOW",
        ] {
            std::env::remove_var(k);
        }
    }
}

/// Acquire the whole-body serialization guard for a direct (non-subprocess) test that sets
/// process-global env vars `on_channel_complete` reads. A poisoned mutex (a PRIOR test panicked
/// while holding it) is recovered rather than propagated — one test's panic must not cascade-fail
/// every later test in this file.
fn lock_env() -> EnvGuard<'static> {
    EnvGuard(ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner()))
}

// ---- workspace/store plumbing (mirrors channel_fanout.rs's own helpers) -----------------------

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

fn personas_dir(tmp: &TempDir) -> PathBuf {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    roles
}

fn write_role(roles: &std::path::Path, handle: &str) {
    std::fs::write(
        roles.join(format!("{handle}.yaml")),
        format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
    )
    .unwrap();
}

fn write_channels_yaml(roles: &std::path::Path, yaml: &str) {
    std::fs::write(roles.join("channels.yaml"), yaml).unwrap();
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// The real on-disk db path a spawned session would be pointed at (`NXC_DB`), stamped into the
/// context below rather than re-derived inside the function under test.
fn db_path_str(tmp: &TempDir) -> String {
    tmp.path()
        .join(".nxs")
        .join("db.sqlite")
        .to_str()
        .unwrap()
        .to_string()
}

/// The ambient context `cli.rs` would have resolved for this call — the direct-call equivalent of
/// its `CliCtx`, built from this test's own temp workspace and the process env it holds the lock on
/// (nxf 6j6v.7dw6: `on_channel_complete` now takes all of this explicitly instead of reading it).
/// This suite never asserts on SCHEDULING; an injected no-op timer keeps it from depending on
/// whatever `NXC_TIMER` happens to be (PR #269 review, Code Quality #3).
static NO_TIMER: nexus_chat::timer::DisabledTimer = nexus_chat::timer::DisabledTimer;

struct TestCtx {
    defs: Definitions,
    worker: AmbientWorker,
    origin: String,
    db_path: String,
}

impl TestCtx {
    fn resolve(tmp: &TempDir) -> TestCtx {
        TestCtx {
            defs: Definitions::from_dir(&tmp.path().join(".nxs-personas"))
                .expect("definitions load from the roles dir"),
            worker: AmbientWorker {
                cwd: tmp.path().to_path_buf(),
            },
            // Read exactly as `cli.rs::origin()` does, so the non-default-origin test below still
            // drives the WHOLE call under `NXC_ORIGIN=acme`.
            origin: std::env::var("NXC_ORIGIN")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "local".to_string()),
            db_path: db_path_str(tmp),
        }
    }

    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            now: NOW,
            origin: &self.origin,
            actor: "opener",
            session: None,
            hop: 0,
            defs: &self.defs,
            worker: &self.worker,
            timer: &NO_TIMER,
            namer: &nexus_chat::naming::DryNamer,
            db_path: &self.db_path,
            project_claude_md: None,
            module_primes: None,
            machines: None,
        }
    }
}

/// The checked depth these direct calls used to pass as a bare `0`. Since PR #267's review it has to
/// be a [`CheckedHop`], which only `resolve_hop` mints — so even a test cannot hand the completion
/// routing a depth nobody guarded, and these calls now go through the same resolution the verbs do.
/// The context here carries no ambient session, so it resolves to the claimed `0` as before.
fn fresh_hop(ctx: &Ctx, store: &ChatStore) -> CheckedHop {
    nexus_chat::orchestration::resolve_hop(ctx, store).expect("a fresh chain passes the guard")
}

/// Resolves the real worker from `NXC_WORKER`/`NXC_SIDECAR` at TRIGGER time, exactly as the CLI's
/// own lazy worker does. That timing is what keeps the forced-spawn-failure tests below honest:
/// their failure IS a `select_worker` failure, and it has to surface from the trigger (where
/// `on_channel_complete` catches it and reports an outcome), not from building the context.
struct AmbientWorker {
    cwd: PathBuf,
}

impl Worker for AmbientWorker {
    fn trigger(&self, req: TriggerRequest) -> nexus_chat::worker::TriggerResult {
        nexus_chat::worker::select_worker(self.cwd.clone())?.trigger(req)
    }
}

fn nxc(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env("NXC_ACTOR", "opener")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW);
    c
}

/// The member thread `handle` was given below `channel_thread` (nxf 6j6v.pf6j) — the two-ended
/// conversation a channel member actually answers in. Before this item every member replied into the
/// channel thread itself; there is no such shape any more.
fn member_thread(tmp: &TempDir, channel_thread: &str, handle: &str) -> String {
    let store = open_store(tmp);
    let want = format!("local/{handle}");
    store
        .supervised_children(channel_thread)
        .unwrap()
        .into_iter()
        .find(|t| {
            store
                .thread_quorum(t, NOW)
                .unwrap()
                .is_some_and(|q| q.expects == [want.clone()])
        })
        .unwrap_or_else(|| panic!("no member thread for {handle} under {channel_thread}"))
}

fn json_of(out: &[u8]) -> Value {
    serde_json::from_str(String::from_utf8_lossy(out).trim()).expect("valid json")
}

/// Every dry-log line that opens a trigger entry (mirrors `channel_fanout.rs`'s identical helper —
/// the message field itself may embed real newlines).
fn trigger_header_lines(dry: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(dry)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("trigger role="))
        .map(str::to_string)
        .collect()
}

fn field(line: &str, key: &str) -> String {
    let marker = format!("{key}=");
    let start = line
        .find(&marker)
        .unwrap_or_else(|| panic!("no {marker} in {line}"))
        + marker.len();
    line[start..]
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Seed a declared-channel-backed thread directly through `facade::ask` (public API): materializes
/// `channel_id` (a bare `set_channel_field`, mirroring `ensure_declared_channel`'s minimal
/// requirement for `channel_exists`), then opens the board with `opener_session` stamped as its
/// return address (mirroring `ask`'s own ambient-stamping, done by hand here since this is a
/// direct-call test, not a live `nxc ask`). Origin is explicit (not read from env) so a test can
/// drive the WHOLE seed under a non-default origin — see [`seed_ask`] for this file's usual
/// `"local"` convention, and `summarize_role_requester_phase2_recognition_fires_under_a_non_
/// default_origin` for why a non-default one matters here (review finding: pinning every test to
/// `"local"` cannot catch `on_channel_complete`'s qualification ever regressing to a hardcoded
/// `"local/__synth__"` instead of the actual ambient `origin()`).
#[allow(clippy::too_many_arguments)]
fn seed_ask_with_origin(
    store: &mut ChatStore,
    origin: &str,
    channel_id: &str,
    opener_actor: &str,
    opener_session: Option<&str>,
    expect: &[String],
    body: &str,
) -> String {
    store.set_channel_field(
        channel_id,
        "kind",
        "group",
        &format!("{origin}/{opener_actor}"),
    );
    let refs = Refs {
        session_id: opener_session.map(str::to_string),
        ..Default::default()
    };
    facade::ask(
        store,
        AskRequest {
            now: NOW,
            origin,
            actor: opener_actor,
            channel: channel_id,
            body,
            expect,
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs,
        },
    )
    .expect("ask succeeds")
    .thread_id
}

/// [`seed_ask_with_origin`] pinned to `"local"` — this file's usual convention, matching every
/// other chat integration test.
fn seed_ask(
    store: &mut ChatStore,
    channel_id: &str,
    opener_actor: &str,
    opener_session: Option<&str>,
    expect: &[String],
    body: &str,
) -> String {
    seed_ask_with_origin(
        store,
        "local",
        channel_id,
        opener_actor,
        opener_session,
        expect,
        body,
    )
}

fn reply_as_with_origin(
    store: &mut ChatStore,
    origin: &str,
    target: &str,
    actor: &str,
    body: &str,
) {
    facade::reply(
        store,
        ReplyRequest {
            now: NOW,
            origin,
            actor,
            target,
            body,
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            refs: Refs::default(),
            if_unanswered: false,
        },
    )
    .expect("reply succeeds");
}

fn reply_as(store: &mut ChatStore, target: &str, actor: &str, body: &str) {
    reply_as_with_origin(store, "local", target, actor, body)
}

/// The consolidation these tests drive: the channel's own DECLARED consolidator over a settled set
/// that carried NO escalation (nxf 6j6v.e9qj). The escalating case changes which output form runs at
/// all — an escalation is never folded away — so it has its own tests, in `channel_consolidator.rs`
/// and `reply_escalate.rs`, rather than a flag threaded through this suite.
fn declared(channel: &ChannelDecl) -> Consolidation {
    channel.consolidation(false)
}

fn pass_through_channel(members: &[&str]) -> ChannelDecl {
    ChannelDecl {
        name: "review".to_string(),
        members: members.iter().map(|s| s.to_string()).collect(),
        kind: ChannelKind::Group,
        description: None,
        expects: Expects::All,
        timeout: None,
        on_complete: OnComplete::PassThrough,
        summary_prompt: None,
        summary_model: None,
        visibility: Visibility::RequesterOnly,
        working_tree: Default::default(),
        flow: Flow::Parallel,
        preconditions: Vec::new(),
        steps: Vec::new(),
        rework_notice: None,
    }
}

fn summarize_channel(members: &[&str]) -> ChannelDecl {
    ChannelDecl {
        name: "review".to_string(),
        members: members.iter().map(|s| s.to_string()).collect(),
        kind: ChannelKind::Group,
        description: None,
        expects: Expects::All,
        timeout: None,
        on_complete: OnComplete::Summarize,
        summary_prompt: Some("Summarize the discussion.".to_string()),
        summary_model: None,
        visibility: Visibility::RequesterOnly,
        working_tree: Default::default(),
        flow: Flow::Parallel,
        preconditions: Vec::new(),
        steps: Vec::new(),
        rework_notice: None,
    }
}

// ---- pass_through ------------------------------------------------------------------------------

#[test]
fn pass_through_wakes_the_role_requester_with_the_raw_collected_replies() {
    let _guard = lock_env();
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    let dry = tmp.path().join("dry.log");
    std::env::set_var("NXC_WORKER", "dry");
    std::env::set_var("NXC_DRY_LOG", dry.to_str().unwrap());

    let mut store = open_store(&tmp);
    store.create_pending_session("s-req", "pm").unwrap();
    store.bind_session("s-req", "real-req").unwrap();
    let tid = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-req"),
        &["local/bob".to_string(), "local/carol".to_string()],
        "please review",
    );
    reply_as(&mut store, &tid, "bob", "looks fine");
    reply_as(&mut store, &tid, "carol", "approved, ship it");

    let q = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert!(q.complete, "{q:?}");

    let channel = pass_through_channel(&["bob", "carol"]);
    let tctx = TestCtx::resolve(&tmp);
    let ctx = tctx.ctx();
    let hop = fresh_hop(&ctx, &store);
    let outcome = on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &q,
        "s-req",
        None,
        hop,
    )
    .unwrap()
    .outcome;

    assert!(outcome.delivered.contains("looks fine"), "{outcome:?}");
    assert!(
        outcome.delivered.contains("approved, ship it"),
        "{outcome:?}"
    );

    let log = std::fs::read_to_string(&dry).unwrap();
    assert!(log.contains("role=pm"), "{log}");
    assert!(log.contains("session=s-req"), "{log}");
    assert!(log.contains("resume=real-req"), "{log}");
    assert!(log.contains("looks fine"), "{log}");
    assert!(log.contains("approved, ship it"), "{log}");
}

#[test]
fn pass_through_does_not_deepen_the_requester_it_wakes() {
    // nxf 6j6v.ka09 (PR #278 review, Test Quality #1). `wake_role_requester` — the delivery path
    // BOTH `pass_through` and `summarize`'s phase 2 route through — hands the requester the result
    // of the channel IT opened, so it is a `ChainMove::Unwind`. The test above proves the requester
    // is woken and with what; this proves it is woken at its OWN depth, which is the property that
    // keeps a long-lived requester callable across many completions.
    //
    // **The caller has to be deeper than the requester** or both arms compute the same number: at
    // requester 4 / caller 5, `Deeper` would write MAX(4, 5 + 1) = 6.
    let _guard = lock_env();
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    std::env::set_var("NXC_WORKER", "dry");

    let mut store = open_store(&tmp);
    store.create_pending_session("s-req", "pm").unwrap();
    store.bind_session("s-req", "real-req").unwrap();
    store.record_trigger_depth("s-req", 4).unwrap();
    let tid = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-req"),
        &["local/bob".to_string(), "local/carol".to_string()],
        "please review",
    );
    reply_as(&mut store, &tid, "bob", "looks fine");
    reply_as(&mut store, &tid, "carol", "approved, ship it");
    let q = store.thread_quorum(&tid, NOW).unwrap().unwrap();

    let channel = pass_through_channel(&["bob", "carol"]);
    let tctx = TestCtx::resolve(&tmp);
    let ctx = Ctx {
        hop: 5,
        ..tctx.ctx()
    };
    let hop = fresh_hop(&ctx, &store);
    on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &q,
        "s-req",
        None,
        hop,
    )
    .unwrap();

    assert_eq!(
        store.session_depth("s-req").unwrap(),
        Some(4),
        "the requester opened this channel from depth 4 and re-enters at depth 4"
    );
}

// ---- summarize ----------------------------------------------------------------

#[test]
fn summarize_role_requester_phase1_spawns_exactly_one_synthesizer_and_does_not_wake_the_real_requester_yet(
) {
    let _guard = lock_env();
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    let dry = tmp.path().join("dry.log");
    std::env::set_var("NXC_WORKER", "dry");
    std::env::set_var("NXC_DRY_LOG", dry.to_str().unwrap());

    let mut store = open_store(&tmp);
    store.create_pending_session("s-req", "pm").unwrap();
    store.bind_session("s-req", "real-req").unwrap();
    let tid = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-req"),
        &["local/bob".to_string(), "local/carol".to_string()],
        "please review",
    );
    reply_as(&mut store, &tid, "bob", "looks fine");
    reply_as(&mut store, &tid, "carol", "approved");

    let q1 = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert!(q1.complete);

    let channel = summarize_channel(&["bob", "carol"]);
    let tctx = TestCtx::resolve(&tmp);
    let ctx = tctx.ctx();
    let hop = fresh_hop(&ctx, &store);
    on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &q1,
        "s-req",
        None,
        hop,
    )
    .unwrap();

    let lines = trigger_header_lines(&dry);
    assert_eq!(
        lines.len(),
        1,
        "exactly one synthesizer trigger, real requester not yet woken: {lines:?}"
    );
    assert_eq!(field(&lines[0], "role"), SYNTHESIS_HANDLE);
    assert_eq!(field(&lines[0], "resume"), "-");
    let synth_session = field(&lines[0], "session");

    let q_after = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert_eq!(
        q_after.expects,
        vec![format!("local/{SYNTHESIS_HANDLE}")],
        "expects re-targeted to the QUALIFIED synthesis identity: {q_after:?}"
    );
    assert!(
        !q_after.complete,
        "a fresh single-entry expects that hasn't replied yet: {q_after:?}"
    );
    assert_ne!(
        synth_session, "s-req",
        "the synthesizer gets its OWN distinct internal session, not the requester's"
    );
    // nxf 6j6v.m48m: the synthesizer is spawned WITHOUT going through `trigger_role` (it has no
    // declaration to compose from), so it also misses the one place the chain depth is written unless
    // that site stamps it too. It is a Bash-enabled session that posts its own `nxc reply`, i.e.
    // exactly what the guard is for — `Some(0)`, the bare `create_pending_session` default, would
    // mean it starts every chain of its own from scratch.
    assert_eq!(
        store.session_depth(&synth_session).unwrap(),
        Some(1),
        "the ephemeral synthesizer's session is inside the depth guard too"
    );
    // nxf 6j6v.a71h §3.1 (review finding F1), the exact twin of the claim above and missed for the
    // exact same reason: bypassing `trigger_role` also bypasses the one place a session's POSITION
    // is written. The synthesizer is the same Bash-enabled session, and the `summary_prompt` tells
    // it to run `nxc` itself — so without this, a `nxc send --to X` from it resolves no parent and
    // opens a ROOT, silently detaching that branch from the operation it belongs to. It stands in
    // the board thread it was minted to summarize, like every other trigger into that thread.
    assert_eq!(
        store.session_thread(&synth_session).unwrap().as_deref(),
        Some(tid.as_str()),
        "the ephemeral synthesizer stands in the board thread, so what it opens hangs under it"
    );
}

/// nxf 6j6v.296n — the double-spawn race, reproduced deterministically.
///
/// The phase-1 arm is a check-then-act over `thread.expects`: read the quorum, decide it just
/// completed, re-target `expects` to the synthesis marker, spawn. Two callers completing the SAME
/// thread at nearly the same moment — a real completing reply landing exactly as `workflow tick`'s
/// scheduled timer fires for that thread — both read the state BEFORE either retargeted it, so both
/// take the phase-1 arm and each spawns its own ephemeral synthesizer for the same board.
///
/// Handing the second call the same pre-retarget `ThreadQuorum` snapshot IS that interleaving: it is
/// exactly what the losing caller holds in its hands when it reaches this function, and it needs no
/// threads or sleeps to reproduce. Two synthesizers on one board burn two paid sessions and can each
/// `nxc reply` into the thread; only the last reply's outcome is ever consulted (phase-2 recognition
/// keys on the marker), so the damage is waste and confusion rather than a wrong answer — which is
/// why this was filed rather than hot-fixed. The guard is a compare-and-swap in the store, so the
/// loser is decided by the database, not by who happens to run first.
#[test]
fn two_completions_racing_on_the_same_pre_retarget_snapshot_spawn_exactly_one_synthesizer() {
    let _guard = lock_env();
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    let dry = tmp.path().join("dry.log");
    std::env::set_var("NXC_WORKER", "dry");
    std::env::set_var("NXC_DRY_LOG", dry.to_str().unwrap());

    let mut store = open_store(&tmp);
    store.create_pending_session("s-req", "pm").unwrap();
    store.bind_session("s-req", "real-req").unwrap();
    let tid = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-req"),
        &["local/bob".to_string(), "local/carol".to_string()],
        "please review",
    );
    reply_as(&mut store, &tid, "bob", "looks fine");
    reply_as(&mut store, &tid, "carol", "approved");

    // The ONE snapshot both racers hold: complete, not yet re-targeted.
    let q = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert!(q.complete);

    let channel = summarize_channel(&["bob", "carol"]);
    let tctx = TestCtx::resolve(&tmp);
    let ctx = tctx.ctx();
    let hop = fresh_hop(&ctx, &store);
    let first = on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &q,
        "s-req",
        None,
        hop,
    )
    .unwrap();
    let second = on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &q,
        "s-req",
        None,
        hop,
    )
    .unwrap();

    let lines = trigger_header_lines(&dry);
    assert_eq!(
        lines.len(),
        1,
        "exactly ONE synthesizer for one completed board, however many callers raced: {lines:?}"
    );
    assert!(
        first.outcome.delivered.contains("synthesizer spawned"),
        "the winner reports the spawn it made: {:?}",
        first.outcome
    );
    assert_eq!(
        second.woke, None,
        "the loser wakes nobody either: {second:?}"
    );
    assert!(
        second.outcome.delivered.contains("already"),
        "the loser says plainly that another completion got there first, so a receipt does not \
         read as a second spawn: {:?}",
        second.outcome
    );

    // And the winner's own state is untouched by the losing call: still exactly one pending
    // synthesizer, still expecting only it.
    let q_after = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert_eq!(q_after.expects, vec![format!("local/{SYNTHESIS_HANDLE}")]);
    assert!(!q_after.complete, "{q_after:?}");
}

/// nxf 6j6v.e9qj — **the SAME race on the OTHER output form**, which was open until this item.
///
/// The fold's arm was guarded by the compare-and-swap above; the pass-through arm was gated only on
/// "the channel thread still expects the supervisor" (`supervisor_consider_set`). That makes a LATE
/// second completion a clean no-op — good, and unchanged — but two SIMULTANEOUS ones both hold the
/// pre-retarget snapshot, so both deliver: the requester is woken twice with the same answers, and
/// the thread carries two `[pass_through delivered]` markers. Once the consolidator is one mechanism
/// with a declared difference between two output forms, one of them being unprotected is not a
/// defensible asymmetry, so both now take the same claim, each keyed on its own retarget marker.
///
/// Reproduced exactly as the fold's race above is: handing the second call the same pre-retarget
/// `ThreadQuorum` snapshot IS the interleaving, and it needs no threads or sleeps. (The
/// genuinely-concurrent, eight-writer version of the CAS itself lives in `consolidation_claim.rs`
/// and is driven for BOTH markers there.)
#[test]
fn two_completions_racing_on_the_same_pre_retarget_snapshot_deliver_a_pass_through_exactly_once() {
    let _guard = lock_env();
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    let dry = tmp.path().join("dry.log");
    std::env::set_var("NXC_WORKER", "dry");
    std::env::set_var("NXC_DRY_LOG", dry.to_str().unwrap());

    let mut store = open_store(&tmp);
    store.create_pending_session("s-req", "pm").unwrap();
    store.bind_session("s-req", "real-req").unwrap();
    let tid = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-req"),
        &["local/bob".to_string(), "local/carol".to_string()],
        "please review",
    );
    reply_as(&mut store, &tid, "bob", "looks fine");
    reply_as(&mut store, &tid, "carol", "approved");

    // The ONE snapshot both racers hold: complete, not yet re-targeted.
    let q = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert!(q.complete);

    let channel = pass_through_channel(&["bob", "carol"]);
    let tctx = TestCtx::resolve(&tmp);
    let ctx = tctx.ctx();
    let hop = fresh_hop(&ctx, &store);
    let first = on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &q,
        "s-req",
        None,
        hop,
    )
    .unwrap();
    let second = on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &q,
        "s-req",
        None,
        hop,
    )
    .unwrap();

    // ONE wake. This is the whole finding: `wake_role_requester` is what hands the requester the
    // answers, and before the claim it ran on every racer.
    let lines = trigger_header_lines(&dry);
    assert_eq!(
        lines.len(),
        1,
        "the requester is woken ONCE, however many callers raced: {lines:?}"
    );
    assert_eq!(first.woke.as_deref(), Some("s-req"), "{first:?}");
    assert_eq!(second.woke, None, "the loser wakes nobody: {second:?}");
    assert!(
        second.outcome.delivered.contains("already"),
        "the loser says plainly that another completion got there first: {:?}",
        second.outcome
    );

    // …and ONE idempotency marker, so the thread's own history does not read as two deliveries.
    let markers = store
        .messages_in_thread(&tid)
        .unwrap()
        .into_iter()
        .filter(|m| m.body == "[pass_through delivered]")
        .count();
    assert_eq!(markers, 1, "exactly one delivered marker for one delivery");

    // The winner's own state is what a late third caller meets, and it is refused on the marker
    // itself rather than on the key — the same second line of defence the fold has.
    let q_after = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert_eq!(
        q_after.expects,
        vec![format!("local/{DELIVERED_HANDLE}")],
        "{q_after:?}"
    );
    assert!(
        q_after.complete,
        "a delivered pass-through stays complete, so the PULL surfaces still show it: {q_after:?}"
    );
}

/// The claim is won BEFORE the retarget it guards, so a retarget that fails must hand it back
/// (PR #271 review — found independently by all three reviews: Code Quality #1, Test Quality #1,
/// Integrity & Robustness #2).
///
/// Without that, winning the claim and then failing is indistinguishable from having spawned: every
/// later completion of that round is refused as "already spawned", and the board only recovers by a
/// re-declaration through `facade::set_expects` — which, since 6j6v.dvyq removed `nxc threads
/// expect`, no human can type at all. That is strictly worse than the double spawn this guard exists
/// to prevent — a wedged thread instead of a wasteful one.
///
/// The failure is forced through a real path rather than a mock: `facade::set_expects` is
/// OPENER-ONLY, and a quorum whose `opener` did not resolve makes `on_channel_complete` present an
/// empty handle, which that check refuses. That is a genuine degenerate state (a thread whose root
/// never folded), not a contrivance.
#[test]
fn a_retarget_that_fails_after_the_claim_leaves_the_round_claimable_again() {
    let _guard = lock_env();
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    let dry = tmp.path().join("dry.log");
    std::env::set_var("NXC_WORKER", "dry");
    std::env::set_var("NXC_DRY_LOG", dry.to_str().unwrap());

    let mut store = open_store(&tmp);
    store.create_pending_session("s-req", "pm").unwrap();
    store.bind_session("s-req", "real-req").unwrap();
    let tid = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-req"),
        &["local/bob".to_string()],
        "please review",
    );
    reply_as(&mut store, &tid, "bob", "lgtm");
    let q = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert!(q.complete);

    let channel = summarize_channel(&["bob"]);
    let tctx = TestCtx::resolve(&tmp);
    let ctx = tctx.ctx();
    let hop = fresh_hop(&ctx, &store);

    let mut openerless = q.clone();
    openerless.opener = None;
    let err = on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &openerless,
        "s-req",
        None,
        hop,
    )
    .expect_err("the retarget is refused, and that failure still surfaces");
    assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Forbidden);
    assert!(
        trigger_header_lines(&dry).is_empty(),
        "nothing was spawned, so nothing may look spawned"
    );

    // The round must still be claimable: the real completion, arriving after, has to work.
    let routed = on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &q,
        "s-req",
        None,
        hop,
    )
    .expect("the round is not wedged by the failed attempt");
    assert!(
        routed.outcome.delivered.contains("synthesizer spawned"),
        "the retry spawns for real rather than being refused as a duplicate: {:?}",
        routed.outcome
    );
    assert_eq!(
        trigger_header_lines(&dry).len(),
        1,
        "exactly one synthesizer, from the attempt that actually got through"
    );
}

#[test]
fn summarize_role_requester_phase2_wakes_the_real_requester_with_the_synthesizers_reply_body() {
    let _guard = lock_env();
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    let dry = tmp.path().join("dry.log");
    std::env::set_var("NXC_WORKER", "dry");
    std::env::set_var("NXC_DRY_LOG", dry.to_str().unwrap());

    let mut store = open_store(&tmp);
    store.create_pending_session("s-req", "pm").unwrap();
    store.bind_session("s-req", "real-req").unwrap();
    let tid = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-req"),
        &["local/bob".to_string()],
        "please review",
    );
    reply_as(&mut store, &tid, "bob", "lgtm");
    let q1 = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert!(q1.complete);

    let channel = summarize_channel(&["bob"]);
    let tctx = TestCtx::resolve(&tmp);
    let ctx = tctx.ctx();
    // Phase 1: spawn the synthesizer (re-targets expects).
    let hop = fresh_hop(&ctx, &store);
    on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &q1,
        "s-req",
        None,
        hop,
    )
    .unwrap();
    assert_eq!(trigger_header_lines(&dry).len(), 1);

    // Phase 2: the synthesizer's own eventual reply lands, into the SAME thread.
    reply_as(&mut store, &tid, SYNTHESIS_HANDLE, "final summary text");
    let q2 = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert!(q2.complete, "{q2:?}");
    assert_eq!(q2.expects, vec![format!("local/{SYNTHESIS_HANDLE}")]);

    let hop = fresh_hop(&ctx, &store);
    let outcome2 = on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &q2,
        "s-req",
        None,
        hop,
    )
    .unwrap()
    .outcome;
    assert_eq!(outcome2.delivered, "final summary text");

    let lines = trigger_header_lines(&dry);
    assert_eq!(
        lines.len(),
        2,
        "phase 2 wakes the real requester exactly once: {lines:?}"
    );
    assert_eq!(field(&lines[1], "role"), "pm");
    assert_eq!(field(&lines[1], "session"), "s-req");
    assert_eq!(field(&lines[1], "resume"), "real-req");
    let log = std::fs::read_to_string(&dry).unwrap();
    assert!(log.contains("final summary text"), "{log}");
}

#[test]
fn summarize_role_requester_phase2_recognition_fires_under_a_non_default_origin() {
    // Review finding: every OTHER test in this file pins `NXC_ORIGIN` to the default `"local"` on
    // both the phase-1 and phase-2 sides — so if `on_channel_complete`'s qualification (the
    // `format!("{}/{}", origin(), SYNTHESIS_HANDLE)` call) ever regressed to a hardcoded
    // `"local/__synth__"`, every one of them would still pass. Exactly the bare-vs-qualified bug
    // class Task 8 shipped. This test runs the SAME phase-1 -> phase-2 flow under a NON-default
    // `NXC_ORIGIN=acme` on both sides, asserting the PERSISTED `expects` value directly against the
    // store (never trusting `on_channel_complete`'s own internal computation) — so a hardcoded
    // `"local"` regression fails loudly here even though it would pass silently everywhere else.
    let _guard = lock_env();
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    let dry = tmp.path().join("dry.log");
    std::env::set_var("NXC_WORKER", "dry");
    std::env::set_var("NXC_DRY_LOG", dry.to_str().unwrap());
    std::env::set_var("NXC_ORIGIN", "acme");

    let mut store = open_store(&tmp);
    store.create_pending_session("s-req", "pm").unwrap();
    store.bind_session("s-req", "real-req").unwrap();
    let tid = seed_ask_with_origin(
        &mut store,
        "acme",
        "decl:review",
        "pm",
        Some("s-req"),
        &["acme/bob".to_string()],
        "please review",
    );
    reply_as_with_origin(&mut store, "acme", &tid, "bob", "lgtm");
    let q1 = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert!(q1.complete);

    let channel = summarize_channel(&["bob"]);
    let tctx = TestCtx::resolve(&tmp);
    let ctx = tctx.ctx();

    // Phase 1: spawn the synthesizer — this must re-target `expects` to the QUALIFIED
    // `acme/__synth__`, using the ACTUAL ambient origin, not a hardcoded `"local"`.
    let hop = fresh_hop(&ctx, &store);
    on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &q1,
        "s-req",
        None,
        hop,
    )
    .unwrap();
    let lines = trigger_header_lines(&dry);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(field(&lines[0], "role"), SYNTHESIS_HANDLE);

    let q_after = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert_eq!(
        q_after.expects,
        vec![format!("acme/{SYNTHESIS_HANDLE}")],
        "expects must be re-targeted under the ACTUAL ambient origin \"acme\", not a hardcoded \
         \"local\": {q_after:?}"
    );
    assert_ne!(
        q_after.expects,
        vec![format!("local/{SYNTHESIS_HANDLE}")],
        "must never hardcode \"local\" — exactly Task 8's bare/qualified bug class: {q_after:?}"
    );

    // Phase 2: the synthesizer's own eventual reply, ALSO under `NXC_ORIGIN=acme` (a real
    // synthesizer session would inherit this same ambient origin via `trigger_env`). Recognition
    // must fire — a hardcoded-"local" regression would leave `expects` stuck at
    // `local/__synth__`, so this qualified `acme/__synth__` reply would never match and the
    // thread would never complete.
    reply_as_with_origin(
        &mut store,
        "acme",
        &tid,
        SYNTHESIS_HANDLE,
        "final summary under acme",
    );
    let q2 = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert!(
        q2.complete,
        "phase-2 recognition must fire under a non-default origin too: {q2:?}"
    );

    let hop = fresh_hop(&ctx, &store);
    let outcome2 = on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &q2,
        "s-req",
        None,
        hop,
    )
    .unwrap()
    .outcome;
    assert_eq!(outcome2.delivered, "final summary under acme");

    let lines = trigger_header_lines(&dry);
    assert_eq!(
        lines.len(),
        2,
        "phase 2 wakes the real requester exactly once: {lines:?}"
    );
    assert_eq!(field(&lines[1], "role"), "pm");
    assert_eq!(field(&lines[1], "session"), "s-req");
    assert_eq!(field(&lines[1], "resume"), "real-req");
    let log = std::fs::read_to_string(&dry).unwrap();
    assert!(log.contains("final summary under acme"), "{log}");
}

// ---- forced synthesizer error: never silent, for both requester shapes -------------------------

#[test]
fn forced_synthesizer_spawn_failure_wakes_the_role_requester_with_an_error_note() {
    let _guard = lock_env();
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    // Force the synthesizer's `select_worker` to fail BEFORE constructing any real spawn. An
    // UNKNOWN `NXC_WORKER` is the one forcing that depends on nothing but the variable itself.
    //
    // It used to be `NXC_WORKER=sidecar` with `NXC_SIDECAR` unset, which stopped forcing anything
    // once the bundle became part of the binary (nxf 6j6v.smsz): `select_worker` then finds the
    // EMBEDDED sidecar and succeeds — and, worse, that made the test's outcome depend on whether
    // the machine's build had a bundle to embed, so it would have gone on passing in half the
    // world. What is under test here is that a failed synthesis is a reported OUTCOME rather than a
    // silent drop; how the spawn was made to fail is incidental, and now says so.
    std::env::set_var("NXC_WORKER", "no-such-worker");
    std::env::remove_var("NXC_SIDECAR");

    let mut store = open_store(&tmp);
    store.create_pending_session("s-req", "pm").unwrap();
    store.bind_session("s-req", "real-req").unwrap();
    let tid = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-req"),
        &["local/bob".to_string()],
        "please review",
    );
    reply_as(&mut store, &tid, "bob", "lgtm");
    let q = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert!(q.complete);

    let channel = summarize_channel(&["bob"]);
    let tctx = TestCtx::resolve(&tmp);
    let ctx = tctx.ctx();
    // Never panics, never returns Err — a failed synthesis is a reportable OUTCOME.
    let hop = fresh_hop(&ctx, &store);
    let outcome = on_channel_complete(
        &ctx,
        &mut store,
        &channel,
        &declared(&channel),
        &q,
        "s-req",
        None,
        hop,
    )
    .unwrap()
    .outcome;

    assert!(
        !outcome.delivered.is_empty() && outcome.delivered.to_lowercase().contains("fail"),
        "the failure must be named, never a silent drop: {outcome:?}"
    );
}

/// Also cover: `CompletionOutcome` derives `Debug`/`Clone`/`PartialEq`/`Eq` — used above via
/// `assert_eq!`/`{outcome:?}`; a direct construction test guards the shape itself stays as
/// documented (this is what the type-design review would ask for on a freshly introduced type).
#[test]
fn completion_outcome_is_a_plain_comparable_value_type() {
    let a = CompletionOutcome {
        delivered: "x".to_string(),
    };
    let b = a.clone();
    assert_eq!(a, b);
}

// ---- live `nxc reply` CLI round trip (both phases through the real PUSH-wake wiring) -----------

#[test]
fn live_reply_path_routes_a_declared_summarize_channel_through_both_phases() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    write_role(&roles, "bob");
    write_role(&roles, "carol");
    write_channels_yaml(
        &roles,
        "- name: review\n  members: [bob, carol]\n  on_complete: summarize\n  \
         summary_prompt: \"Summarize the discussion.\"\n",
    );

    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    let dry = tmp.path().join("dry.log");

    // PM (s-pm) opens the declared channel via `ask` — fans out to bob + carol (Task 8 mechanism,
    // untouched by this ticket).
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-pm")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let tid = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    let lines = trigger_header_lines(&dry);
    assert_eq!(lines.len(), 2, "fan-out to bob + carol: {lines:?}");

    // bob replies in HIS OWN thread — 1/2, no completion yet.
    nxc(&tmp)
        .env("NXC_ACTOR", "bob")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args([
            "reply",
            "--thread",
            &member_thread(&tmp, &tid, "bob"),
            "looks fine",
        ])
        .assert()
        .success();
    assert_eq!(
        trigger_header_lines(&dry).len(),
        2,
        "not complete yet — no synthesizer spawned"
    );

    // carol replies in hers — the SET is settled, so the supervisor consolidates (phase 1): spawns
    // exactly one synthesizer, does NOT yet wake pm.
    nxc(&tmp)
        .env("NXC_ACTOR", "carol")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args([
            "reply",
            "--thread",
            &member_thread(&tmp, &tid, "carol"),
            "approved",
        ])
        .assert()
        .success();
    let lines = trigger_header_lines(&dry);
    assert_eq!(
        lines.len(),
        3,
        "phase 1 spawns exactly one synthesizer: {lines:?}"
    );
    assert_eq!(field(&lines[2], "role"), "__synth__");
    assert_eq!(field(&lines[2], "resume"), "-");
    assert!(
        !lines.iter().any(|l| l.contains("role=pm")),
        "pm must not be woken yet: {lines:?}"
    );

    // The thread's expects has been re-targeted to the QUALIFIED synthesis identity.
    let show = json_of(
        &nxc(&tmp)
            .args(["--json", "threads", "show", &tid])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(
        show["expects"],
        serde_json::json!(["local/__synth__"]),
        "{show}"
    );
    assert_eq!(show["complete"], false, "{show}");

    // Phase 2: the synthesizer's own eventual reply lands (simulated here exactly as a real
    // synthesizer session eventually would: an ordinary `nxc reply` as the synthesis identity).
    nxc(&tmp)
        .env("NXC_ACTOR", "__synth__")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["reply", "--thread", &tid, "final synthesized summary"])
        .assert()
        .success();
    let lines = trigger_header_lines(&dry);
    assert_eq!(lines.len(), 4, "phase 2 wakes pm exactly once: {lines:?}");
    assert_eq!(field(&lines[3], "role"), "pm");
    assert_eq!(field(&lines[3], "session"), "s-pm");
    assert_eq!(field(&lines[3], "resume"), "real-pm");
    let log = std::fs::read_to_string(&dry).unwrap();
    assert!(log.contains("final synthesized summary"), "{log}");
}

// ---- regression: 6j6v.04es -- the synthesizer must actually be able to deliver -----------------
//
// Confirmed live (`crates/chat/tests/smoke_v3.rs`, nxf ticket 6j6v.8e45 — every OTHER test in this
// file runs under `NXC_WORKER=dry`, which never caught either half of this): the synthesizer used
// to be spawned with `tools: Some(vec![])` (disables ALL tools, including Bash, so it could never
// run the `nxc reply ...` its own `summary_prompt` instructs), and its internal session was never
// registered via `create_pending_session` (so `main.mjs`'s post-query `nxc session bind` crashed
// with `not_found` every time). Neither is observable through `DryWorker`'s log line (it never
// records `tools`, and a dry trigger never touches `session_map`) — this test reuses
// `trigger_workflow.rs`'s own established pattern instead: point `NXC_SIDECAR` at a NONEXISTENT
// script so the real `SidecarWorker` still writes the real spec JSON (with the real `tools` field)
// before its doomed spawn — no real API call needed. Deliberately a live-subprocess test, not a
// direct in-process `on_channel_complete` call like most of this file: `cli.rs`'s `cwd()` reads the
// REAL process's actual working directory, and the `forced_synthesizer_spawn_failure_*` tests above
// deliberately avoid ever reaching `SidecarWorker::trigger` in-process for exactly that reason (it
// would write `.nxs/agent-logs/` under this test BINARY's own shared cwd, not an isolated tmp dir).
// A subprocess's `Command::current_dir` isolates this correctly, mirroring `live_reply_path_...`
// just above. `bob`/`carol`/`pm`'s own triggering stays under `NXC_WORKER=dry` — only the LAST,
// completing reply needs the real spec path, so no other spec.json exists to confuse which one is
// the synthesizer's.
#[test]
fn synthesizer_spawn_grants_bash_and_registers_a_pending_session_6j6v_04es() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    write_role(&roles, "bob");
    write_role(&roles, "carol");
    write_channels_yaml(
        &roles,
        "- name: review\n  members: [bob, carol]\n  on_complete: summarize\n  \
         summary_prompt: \"Summarize the discussion.\"\n",
    );

    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-pm")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let tid = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    nxc(&tmp)
        .env("NXC_ACTOR", "bob")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args([
            "reply",
            "--thread",
            &member_thread(&tmp, &tid, "bob"),
            "looks fine",
        ])
        .assert()
        .success();

    // carol's reply settles the set — runs under the REAL `SidecarWorker` (a nonexistent
    // sidecar script path) so the synthesizer's real spec.json gets written before its doomed spawn.
    let agent_logs = tmp.path().join(".nxs/agent-logs");
    let before: std::collections::HashSet<_> = std::fs::read_dir(&agent_logs)
        .map(|it| it.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    let carol_thread = member_thread(&tmp, &tid, "carol");
    nxc(&tmp)
        .env("NXC_ACTOR", "carol")
        .env("NXC_WORKER", "sidecar")
        .env(
            "NXC_SIDECAR",
            tmp.path().join("does-not-exist").join("main.mjs"),
        )
        .args(["reply", "--thread", &carol_thread, "approved"])
        .assert()
        .success();

    let new_files: Vec<_> = std::fs::read_dir(&agent_logs)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .filter(|n| !before.contains(n))
        .collect();
    let spec_file = new_files
        .iter()
        .find(|n| n.to_string_lossy().ends_with(".spec.json"))
        .unwrap_or_else(|| {
            panic!("no new spec.json written for the synthesizer, saw: {new_files:?}")
        });
    let spec_path = agent_logs.join(spec_file);
    let spec: Value = serde_json::from_str(&std::fs::read_to_string(&spec_path).unwrap()).unwrap();
    assert_eq!(spec["role"], "__synth__", "{spec}");
    assert_eq!(
        spec["tools"],
        serde_json::json!(["Bash"]),
        "the synthesizer must be granted exactly Bash so it can run its own `nxc reply`, got: {spec}"
    );

    // The session-registration fix: the synthesizer's internal session must now be a KNOWN pending
    // session — `nxc session bind` (what `main.mjs` calls after every real session, and what
    // crashed with `not_found` before this fix) must succeed, not error.
    let internal_session = spec_file
        .to_string_lossy()
        .strip_suffix(".spec.json")
        .unwrap()
        .to_string();
    nxc(&tmp)
        .args(["session", "bind", &internal_session, "fake-real-id"])
        .assert()
        .success();
}

// ---- fail-closed re-validation at completion time (independent review, Integrity #4, Medium) ---

/// The defence-in-depth layer, driven at the seam it defends: `on_channel_complete` re-checks that
/// a `summarize` channel really carries a `summary_prompt` right before it spawns, and refuses
/// instead of starting a synthesizer with an empty system prompt.
///
/// **It used to be driven through the CLI by editing the declaration mid-round, and it no longer
/// can be** (nxf 6j6v.n92p): a running operation is bound to the declarations it opened under, so a
/// hand edit made while a round is in flight does not reach it — which is what
/// `a_declaration_edited_mid_round_does_not_reach_the_running_operation` below now asserts, at that
/// very path. The layer is still worth having and still reachable: a thread that arrived over SYNC
/// and a round opened by a version older than the freeze both reach a completion with no bound
/// version behind it, and then the declaration in hand is whatever the folder says now. So the
/// malformed declaration is handed in directly, which is how most of this file drives this function
/// anyway.
#[test]
fn a_summarize_channel_with_no_prompt_fails_closed_at_completion() {
    let _guard = lock_env();
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    let dry = tmp.path().join("dry.log");
    std::env::set_var("NXC_WORKER", "dry");
    std::env::set_var("NXC_DRY_LOG", dry.to_str().unwrap());

    let mut store = open_store(&tmp);
    store.create_pending_session("s-req", "pm").unwrap();
    store.bind_session("s-req", "real-req").unwrap();
    let tid = seed_ask(
        &mut store,
        "decl:review",
        "pm",
        Some("s-req"),
        &["local/bob".to_string()],
        "please review",
    );
    reply_as(&mut store, &tid, "bob", "lgtm");
    let q = store.thread_quorum(&tid, NOW).unwrap().unwrap();
    assert!(q.complete);

    let mut malformed = summarize_channel(&["bob"]);
    malformed.summary_prompt = None;
    let tctx = TestCtx::resolve(&tmp);
    let ctx = tctx.ctx();
    let hop = fresh_hop(&ctx, &store);
    let routed = on_channel_complete(
        &ctx,
        &mut store,
        &malformed,
        &declared(&malformed),
        &q,
        "s-req",
        None,
        hop,
    )
    .expect("the completion is ROUTED — the requester is told, rather than the call failing");
    assert!(
        routed
            .outcome
            .delivered
            .to_lowercase()
            .contains("summary_prompt"),
        "what is delivered names the actual problem: {:?}",
        routed.outcome
    );
    let lines = trigger_header_lines(&dry);
    assert!(
        !lines.iter().any(|l| field(l, "role") == "__synth__"),
        "and no synthesizer was spawned — one with an empty system prompt is exactly what this \
         layer exists to prevent: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| field(l, "role") == "pm" && l.to_lowercase().contains("summary_prompt")),
        "the requester is woken with the failure instead of being left waiting: {lines:?}"
    );
}

/// **An edit made while a round is in flight does not reach it** (nxf 6j6v.n92p) — the live proof,
/// through the command line, of the decision the owner took on 2026-08-29: the declaration is frozen
/// when an operation is opened, and a change takes effect in the NEXT operation.
///
/// The edit chosen here is the one that used to prove the opposite (removing `summary_prompt` from a
/// `summarize` channel), because it is unmissable in the outcome: under the folder-as-it-is-now
/// reading the round refuses and wakes the requester with an error, and under the bound version it
/// simply completes. There is no ambiguity about which of the two happened.
#[test]
fn a_declaration_edited_mid_round_does_not_reach_the_running_operation() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    write_role(&roles, "bob");
    write_channels_yaml(
        &roles,
        "- name: review\n  members: [bob]\n  on_complete: summarize\n  \
         summary_prompt: \"Summarize the discussion.\"\n",
    );

    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-pm")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let tid = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    // The declaration is edited AFTER the channel already opened, removing `summary_prompt` — still
    // `on_complete: summarize`, now malformed.
    write_channels_yaml(
        &roles,
        "- name: review\n  members: [bob]\n  on_complete: summarize\n",
    );

    nxc(&tmp)
        .env("NXC_ACTOR", "bob")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args([
            "reply",
            "--thread",
            &member_thread(&tmp, &tid, "bob"),
            "looks fine",
        ])
        .assert()
        .success();

    // The round ran to its end under the version it OPENED under: the synthesizer was spawned with
    // the `summary_prompt` that was in force then, and the edit sitting on disk changed nothing
    // about it.
    let lines = trigger_header_lines(&dry);
    assert!(
        lines.iter().any(|l| field(l, "role") == "__synth__"),
        "the round completed under the declaration it opened with: {lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|l| l.to_lowercase().contains("summary_prompt")),
        "and nobody was woken with the edit's error — the edit did not reach this operation at \
         all: {lines:?}"
    );
}

// ---- a thread whose channel NO DECLARATION names still completes through the original wake ----

/// The property `a_non_declared_channels_completion_still_uses_the_original_hardcoded_wake` held,
/// rebuilt on the fixture that survives 6j6v.dvyq §3.
///
/// That test opened its board with `channels create` + `ask <raw-id> --expect r1 --expect r2`, and
/// both verbs are gone: a channel no declaration names is no longer addressable. The PROPERTY is
/// not gone with them — undeclared channels still exist, they are just no longer something you can
/// aim at. The direct conversation `send --to <persona>` opens is one, and so is each member thread
/// the channel supervisor opens. A completion on such a thread must still take the ORIGINAL opener
/// wake and not the declared-channel `on_complete` routing, which is what this checks.
///
/// What does NOT survive the fixture change is the "1 of 2 replies → no wake yet" half: a direct
/// conversation expects exactly one handle. That half is quorum arithmetic and is pinned where it
/// belongs, on `ChatStore::thread_quorum` (`store.rs`), rather than through a CLI round trip.
#[test]
fn a_completion_on_a_channel_no_declaration_names_still_uses_the_original_hardcoded_wake() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    write_role(&roles, "r1");
    // Deliberately NO channels.yaml at all — nothing here declares a channel, and the one this
    // conversation runs in is materialised by the send itself.
    let dry = tmp.path().join("dry.log");

    {
        let mut store = open_store(&tmp);
        store.create_pending_session("s-pm", "pm").unwrap();
    }
    nxc(&tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    let opened = json_of(
        &nxc(&tmp)
            .env("NXC_SESSION", "s-pm")
            .env("NXC_WORKER", "dry")
            .env("NXC_DRY_LOG", &dry)
            .args(["--json", "send", "--to", "r1", "please review"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let tid = opened["thread_id"].as_str().unwrap().to_string();
    assert!(
        opened["channel"]
            .as_str()
            .is_some_and(|c| c.starts_with("dm:")),
        "the conversation runs in a channel no declaration names: {opened}"
    );
    // The send itself triggered r1 — clear the log so what follows is the WAKE alone.
    std::fs::remove_file(&dry).ok();

    nxc(&tmp)
        .env("NXC_ACTOR", "r1")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["reply", "--thread", &tid, "r1: fine"])
        .assert()
        .success();

    let log = std::fs::read_to_string(&dry).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one wake: {log}");
    assert!(lines[0].contains("role=pm"), "{log}");
    assert!(
        lines[0].contains(&format!("thread {tid} is complete (1/1 replied)")),
        "unchanged hardcoded wake text: {log}"
    );
}

// Every test that drove a `Requester::WorkflowRun` stood alongside the ones above — the outcome
// token a fold was asked for, the `fail` sentinel a failed synthesis substituted, and the two
// racing-completion cases seen from a run's side. REMOVED with the run record (6j6v.dvyq §3): the
// racing cases keep their `Role` twins right here, which is where the claim they pin lives.
