//! **A running operation is bound to one version of the declarations** (nxf 6j6v.n92p, the shape
//! nxf 6j6v.nby8 asks for) — driven through [`Engine`], the seam an embedding app speaks over.
//!
//! The owner's decision of 2026-08-29, in one sentence: the declaration is frozen when an operation
//! is OPENED; an edit made while it runs is without effect on it and takes effect in the next
//! operation. This file is that sentence in both directions, plus the two questions nby8's design
//! section says must be answered before anything is built.
//!
//! The two live proofs through the command line live where their own subject does —
//! `channel_complete.rs::a_declaration_edited_mid_round_does_not_reach_the_running_operation` and
//! `channel_flow.rs::an_edit_between_two_passes_does_not_reach_the_operation_that_is_running`. What
//! is here is the rule itself, at the seam, plus what is READABLE about it afterwards.

mod common;

use std::sync::{Arc, Mutex};

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::{Caller, Ctx, TickRequest};
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-08-29T10:00:00Z";

#[derive(Default)]
struct RecordingWorker(Mutex<Vec<TriggerRequest>>, Mutex<Vec<String>>);

impl RecordingWorker {
    fn trigger_for(&self, handle: &str) -> TriggerRequest {
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.role.handle == handle)
            .unwrap_or_else(|| panic!("no trigger for {handle}"))
            .clone()
    }

    fn handles(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.role.handle.clone())
            .collect()
    }

    /// The composed system prompt a role was actually spawned with — the thing an edit to a
    /// persona file would change, and therefore the thing that says which version was in force.
    fn prompt_for(&self, handle: &str) -> String {
        self.trigger_for(handle).role.system_prompt
    }
}

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.0.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }

    /// Whichever sessions a test has declared still alive — what the brought-along liveness hurdle
    /// and nxf 6j6v.10yb's advance gate both read.
    fn session_is_running(&self, internal_session: &str) -> bool {
        self.1.lock().unwrap().iter().any(|s| s == internal_session)
    }
}

fn role_saying(handle: &str, what: &str) -> RoleDecl {
    serde_yaml::from_str(&format!("handle: {handle}\nsystem_prompt: {what}\n"))
        .expect("test role parses")
}

fn channel(yaml: &str) -> ChannelDecl {
    serde_yaml::from_str(yaml).expect("test channel parses")
}

const ORDERED: &str = "name: coding\nmembers: [coder, finisher]\nflow: sequential\n";
/// The same flow with a WINDOW, so a member can lapse and a `tick` has something to advance.
const ORDERED_WINDOWED: &str =
    "name: coding\nmembers: [coder, finisher]\nflow: sequential\ntimeout: 10m\n";
/// The same flow claiming the working copy — what makes a rival commission QUEUE instead of run.
const ORDERED_EXCLUSIVE: &str =
    "name: coding\nmembers: [coder, finisher]\nflow: sequential\nworking_tree: exclusive\n";

/// A no-op timer, so nothing here depends on whatever `NXC_TIMER` happens to be.
static NO_TIMER: nexus_chat::timer::DisabledTimer = nexus_chat::timer::DisabledTimer;

fn team(tmp: &TempDir, defs: &Definitions) -> (Engine, Arc<RecordingWorker>) {
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    common::write_declarations(tmp.path(), defs);
    let worker = Arc::new(RecordingWorker::default());
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker.clone()),
            timer: TimerConfig::Dry,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    (engine, worker)
}

fn first_version() -> Definitions {
    Definitions::new(
        vec![
            role_saying("coder", "VERSION-ONE"),
            role_saying("finisher", "VERSION-ONE"),
        ],
        vec![channel(ORDERED)],
    )
    .expect("catalogue")
}

fn second_version() -> Definitions {
    Definitions::new(
        vec![
            role_saying("coder", "VERSION-TWO"),
            role_saying("finisher", "VERSION-TWO"),
        ],
        vec![channel(ORDERED)],
    )
    .expect("catalogue")
}

fn caller(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

fn as_session(session: &str) -> Caller<'_> {
    Caller {
        session: Some(session),
        actor: None,
        now: Some(NOW),
    }
}

fn open(engine: &Engine) -> String {
    engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "coding",
                body: "build T5",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the channel opens")
        .thread_id
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

// ---- the rule, in both directions -------------------------------------------------------------

#[test]
fn a_later_step_runs_under_the_declarations_the_operation_opened_with() {
    // The whole of the owner's decision, at the seam: the flow's SECOND step is commissioned after
    // the edit, and it is composed from the version the operation opened under.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, &first_version());
    open(&engine);
    let coder = worker.trigger_for("coder");
    assert!(worker.prompt_for("coder").contains("VERSION-ONE"));

    // The author edits both personas MID-OPERATION.
    common::replace_declarations(tmp.path(), &second_version());

    engine
        .reply_thread(
            as_session(&coder.internal_session),
            ReplyThreadRequest {
                machine: None,
                thread: &coder.reply_thread.clone().unwrap(),
                body: "done",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply is posted");

    assert_eq!(
        worker.handles(),
        vec!["coder".to_string(), "finisher".to_string()],
        "the flow advanced"
    );
    assert!(
        worker.prompt_for("finisher").contains("VERSION-ONE"),
        "the second step ran under the version its operation opened with, not under the edit: {}",
        worker.prompt_for("finisher")
    );
}

#[test]
fn the_next_operation_takes_the_new_version() {
    // The other half, and without it the first would be a lock rather than a freeze: the edit is
    // not ignored, it simply waits for the next operation.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, &first_version());
    open(&engine);
    assert!(worker.prompt_for("coder").contains("VERSION-ONE"));

    common::replace_declarations(tmp.path(), &second_version());
    open(&engine);

    let started: Vec<String> = worker
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.role.handle == "coder")
        .map(|r| r.role.system_prompt.clone())
        .collect();
    assert_eq!(started.len(), 2, "two operations, two step-ones");
    assert!(started[0].contains("VERSION-ONE"), "{started:?}");
    assert!(
        started[1].contains("VERSION-TWO"),
        "the NEXT operation takes the new version — otherwise this would be a lock on the folder, \
         which is exactly what the owner ruled out: {started:?}"
    );
}

#[test]
fn a_persona_deleted_mid_operation_still_resolves_for_the_operation_it_stands_in() {
    // nby8's design question 2, and the reason a bare content HASH was not enough: the version has
    // to CARRY the declarations, not merely identify them. Deleting the finisher mid-flight used to
    // break the step that named it; now the step runs, out of the copy.
    let tmp = TempDir::new().unwrap();
    let (engine, worker) = team(&tmp, &first_version());
    open(&engine);
    let coder = worker.trigger_for("coder");

    std::fs::remove_file(tmp.path().join(".nxs-personas/finisher.yaml")).expect("delete a persona");

    let receipt = engine
        .reply_thread(
            as_session(&coder.internal_session),
            ReplyThreadRequest {
                machine: None,
                thread: &coder.reply_thread.clone().unwrap(),
                body: "done",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply is posted");
    assert!(
        receipt.warnings.is_empty(),
        "nothing failed: {:?}",
        receipt.warnings
    );
    assert!(
        worker.handles().contains(&"finisher".to_string()),
        "the deleted persona still resolves for the operation it stands in: {:?}",
        worker.handles()
    );
}

#[test]
fn a_thread_no_operation_opened_reads_the_folder_as_it_always_did() {
    // The `None` arm, asserted rather than assumed: binding every thread anybody ever replied to
    // would store a catalogue per conversation for a guarantee nothing there needs. A thread that
    // no commission and no channel open ever created has no binding at all.
    let tmp = TempDir::new().unwrap();
    let (_engine, _worker) = team(&tmp, &first_version());
    let store = open_store(&tmp);
    assert_eq!(
        store
            .declaration_freeze_of("m-never-opened-by-anything")
            .expect("readable"),
        None
    );
    assert_eq!(
        store
            .frozen_declarations("m-never-opened-by-anything", &first_version())
            .expect("readable")
            .map(|d| d.len()),
        None,
        "and reading it back answers `None`, which means: read the folder"
    );
}

// ---- what is READABLE about it afterwards -----------------------------------------------------

#[test]
fn the_version_an_operation_ran_under_is_readable_after_the_fact() {
    // nby8's acceptance: it must be reconstructable which rules a step ran under. The identifying
    // half is the content hash and when it was taken; the content itself comes back through
    // `frozen_declarations`, which resolves the same row.
    let tmp = TempDir::new().unwrap();
    let (engine, _worker) = team(&tmp, &first_version());
    let board = open(&engine);

    common::replace_declarations(tmp.path(), &second_version());

    let store = open_store(&tmp);
    let root = store.thread_root(&board).expect("a root");
    let (hash, taken) = store
        .declaration_freeze_of(&root)
        .expect("readable")
        .expect("this operation bound a version");
    assert_eq!(hash.len(), 64, "a sha256, as hex: {hash}");
    assert_eq!(taken, NOW);

    // The LIVE catalogue, as the engine resolves it right now — the folder already carries version
    // two, and it is what the reader below re-attaches its resolution from.
    let live = engine.definitions().expect("the folder resolves");
    assert!(
        live.role("coder")
            .unwrap()
            .system_prompt
            .contains("VERSION-TWO"),
        "the premise: the folder has moved on"
    );

    let frozen = store
        .frozen_declarations(&root, &live)
        .expect("readable")
        .expect("bound");
    assert!(
        frozen
            .role("coder")
            .unwrap()
            .system_prompt
            .contains("VERSION-ONE"),
        "what was in force is still readable after the folder moved on: {:?}",
        frozen.role("coder").unwrap().system_prompt
    );
    assert_eq!(
        frozen.source(),
        live.source(),
        "and it reports where this workspace reads declarations from — a fact about the filesystem \
         NOW, carried across from the live catalogue rather than frozen with the rest"
    );
}

#[test]
fn two_operations_under_one_unchanged_folder_share_a_single_stored_copy() {
    // Why the copy is deduplicated by content hash (nby8 design question 1): a copy per operation
    // would pay for the catalogue every time a chain begins, and the catalogue changes far more
    // rarely than that. Two operations, one blob; an edit between them, two.
    let tmp = TempDir::new().unwrap();
    let (engine, _worker) = team(&tmp, &first_version());
    let a = open(&engine);
    let b = open(&engine);

    let store = open_store(&tmp);
    let hash_of = |t: &str| {
        store
            .declaration_freeze_of(&store.thread_root(t).unwrap())
            .unwrap()
            .unwrap()
            .0
    };
    assert_eq!(
        hash_of(&a),
        hash_of(&b),
        "an unchanged folder is one version, however many operations run under it"
    );

    common::replace_declarations(tmp.path(), &second_version());
    let c = open(&engine);
    let store = open_store(&tmp);
    let hash_of = |t: &str| {
        store
            .declaration_freeze_of(&store.thread_root(t).unwrap())
            .unwrap()
            .unwrap()
            .0
    };
    assert_ne!(
        hash_of(&a),
        hash_of(&c),
        "and a changed one is a different version — which is what makes drift visible at all"
    );
}

#[test]
fn a_binding_is_taken_once_and_never_re_taken() {
    // The guarantee, in the one SQL keyword that carries it: first write wins. Written as a direct
    // store test because the race it decides — two `nxc` processes reaching the start of one chain
    // at once — cannot be staged through the seam, and what must be pinned is that a SECOND call
    // for the same root changes nothing whatever the folder says by then.
    let tmp = TempDir::new().unwrap();
    let (engine, _worker) = team(&tmp, &first_version());
    let board = open(&engine);
    let mut store = open_store(&tmp);
    let root = store.thread_root(&board).unwrap();
    let before = store.declaration_freeze_of(&root).unwrap().unwrap();

    store
        .freeze_declarations(&root, "2026-08-29T23:00:00Z", &second_version())
        .expect("a second binding attempt is accepted and does nothing");

    assert_eq!(
        store.declaration_freeze_of(&root).unwrap().unwrap(),
        before,
        "neither the version nor the instant moved"
    );
}

// ---- the substitution is wired at FOUR call sites, and each one is driven here ----------------
//
// PR #391 review, Test Quality #1 (High): `operation_declarations`/`with_declarations` is wired
// independently into `reply`, `tick_the_thread`, `session_ended` and `fire_queued_trigger`, and
// every test above drives the flow through `reply` alone. A dropped substitution — or one handed
// the wrong thread id — in any of the other three would have passed the whole suite. Each test
// below makes the same live-vs-frozen divergence visible through ONE of them.

/// Everything a direct `orchestration::*` call needs, owned — `tick` has no `Engine` verb of its
/// own, so the one path that reaches `tick_the_thread` is the function itself.
struct DirectCtx {
    defs: Definitions,
    worker: Arc<RecordingWorker>,
    origin: String,
    db_path: String,
}

impl DirectCtx {
    fn resolve(tmp: &TempDir, engine: &Engine, worker: Arc<RecordingWorker>) -> DirectCtx {
        let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
        DirectCtx {
            // The LIVE catalogue, read from the folder as it is right now — which is exactly what
            // the substitution has to override.
            defs: Definitions::from_dir(&tmp.path().join(".nxs-personas")).expect("definitions"),
            worker,
            // The workspace's OWN minted origin, not a literal: the supervisor's reserved identity
            // is `<origin>/__channel__`, so a made-up origin would make this call fail to recognise
            // the board at all — and it would fail as a plausible-looking no-op.
            origin: engine.origin(),
            db_path: ws.db_path_str().expect("db path"),
        }
    }

    fn ctx<'a>(&'a self, now: &'a str) -> Ctx<'a> {
        Ctx {
            now,
            origin: &self.origin,
            actor: "carsten",
            session: None,
            hop: 0,
            defs: &self.defs,
            worker: self.worker.as_ref(),
            timer: &NO_TIMER,
            namer: &nexus_chat::naming::DryNamer,
            db_path: &self.db_path,
            project_claude_md: None,
            module_primes: None,
            machines: None,
        }
    }
}

#[test]
fn a_tick_that_advances_a_flow_composes_the_next_step_from_the_frozen_version() {
    // The PULL path — the one nobody is present on. A step whose window lapses advances the flow
    // from a scheduled job, and resolving that step against a freshly edited folder would be the
    // stability hole reopened by the path with no witness.
    let tmp = TempDir::new().unwrap();
    let mut first = first_version();
    first = Definitions::new(first.roles().to_vec(), vec![channel(ORDERED_WINDOWED)])
        .expect("catalogue");
    let (engine, worker) = team(&tmp, &first);
    let board = open(&engine);
    assert!(worker.prompt_for("coder").contains("VERSION-ONE"));

    // The author edits both personas mid-operation…
    common::replace_declarations(
        tmp.path(),
        &Definitions::new(
            second_version().roles().to_vec(),
            vec![channel(ORDERED_WINDOWED)],
        )
        .expect("catalogue"),
    );

    // …and the coder's window lapses with no reply at all, so a tick is what moves the flow.
    let later = "2026-08-29T11:00:00Z";
    let direct = DirectCtx::resolve(&tmp, &engine, worker.clone());
    assert!(
        direct
            .defs
            .role("coder")
            .unwrap()
            .system_prompt
            .contains("VERSION-TWO"),
        "the premise: the folder this tick would otherwise read has moved on"
    );
    let mut store = open_store(&tmp);
    let receipt = nexus_chat::orchestration::tick(
        &direct.ctx(later),
        &mut store,
        TickRequest { thread_id: &board },
    )
    .expect("the tick runs");
    assert_eq!(receipt.reason, "advanced", "{receipt:?}");
    assert!(
        worker.prompt_for("finisher").contains("VERSION-ONE"),
        "the step the TICK started was composed from the version its operation opened with: {}",
        worker.prompt_for("finisher")
    );
}

#[test]
fn a_session_ending_advances_the_flow_on_the_frozen_version() {
    // The third entrance into the supervisor: a member answered while its process was still
    // writing, so the reply DECLINED to advance (nxf 6j6v.10yb) and the announcement of the
    // session's end is what settles it. That path resolves the next step too.
    let tmp = TempDir::new().unwrap();
    let first = Definitions::new(
        first_version().roles().to_vec(),
        vec![channel(ORDERED_EXCLUSIVE)],
    )
    .expect("catalogue");
    let (engine, worker) = team(&tmp, &first);
    open(&engine);
    let coder = worker.trigger_for("coder");
    worker
        .1
        .lock()
        .unwrap()
        .push(coder.internal_session.clone());

    engine
        .reply_thread(
            as_session(&coder.internal_session),
            ReplyThreadRequest {
                machine: None,
                thread: &coder.reply_thread.clone().unwrap(),
                body: "done, but still writing",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply is posted");
    assert_eq!(
        worker.handles(),
        vec!["coder".to_string()],
        "the premise: the liveness gate held the flow, so nothing advanced on the reply"
    );

    // The edit lands in the gap, and the session then announces its end.
    common::replace_declarations(
        tmp.path(),
        &Definitions::new(
            second_version().roles().to_vec(),
            vec![channel(ORDERED_EXCLUSIVE)],
        )
        .expect("catalogue"),
    );
    worker.1.lock().unwrap().clear();
    engine
        .session_ended(caller("carsten"), &coder.internal_session)
        .expect("the session announces its end");

    assert!(
        worker.handles().contains(&"finisher".to_string()),
        "the flow advanced on the announcement: {:?}",
        worker.handles()
    );
    assert!(
        worker.prompt_for("finisher").contains("VERSION-ONE"),
        "…and on the version its operation opened with: {}",
        worker.prompt_for("finisher")
    );
}

#[test]
fn a_trigger_released_from_the_queue_is_composed_from_its_own_operations_version() {
    // The fourth entrance, and the one where an edit had the longest window to slip in: a
    // commission that lost the working-tree race waits in the queue, and its prompt is composed
    // when the lease comes free — which can be much later.
    //
    // **THREE versions, not two, and that is what makes this test discriminate.** A first cut used
    // two and passed even with `fire_queued_trigger`'s own substitution removed — the release runs
    // inside the holder's `reply`, which has already swapped the context to the HOLDER's version, so
    // with both operations bound to the same version nothing could tell the two apart. The mutation
    // check is what found that; the fix is to bind each operation to a version of its own:
    //
    //   v1  the holder opens          → the holder's operation is bound to v1
    //   v2  the parked one opens       → the PARKED operation is bound to v2
    //   v3  the folder while it waits  → what a substitution-less release would read
    //
    // The parked trigger must come out v2: not v1 (the holder's, which is what dropping this
    // substitution gives) and not v3 (the folder, which is what dropping the whole freeze gives).
    let tmp = TempDir::new().unwrap();
    let solo =
        |prompt: &str| format!("handle: solo\nsystem_prompt: {prompt}\nworking_tree: exclusive\n");
    let catalogue = |prompt: &str| {
        Definitions::new(
            vec![serde_yaml::from_str(&solo(prompt)).expect("role parses")],
            Vec::new(),
        )
        .expect("catalogue")
    };
    let (engine, worker) = team(&tmp, &catalogue("VERSION-ONE"));

    let held = engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "solo",
                body: "first",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the first commission runs");

    common::replace_declarations(tmp.path(), &catalogue("VERSION-TWO"));
    let parked = engine
        .send_to(
            caller("pm"),
            SendToRequest {
                machine: None,
                to: "solo",
                body: "second",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the second commission is accepted");
    assert_eq!(
        parked.queue_position,
        Some(1),
        "the premise: the second one is WAITING, not running: {parked:?}"
    );
    assert_eq!(
        worker.handles(),
        vec!["solo".to_string()],
        "only one started"
    );

    // The folder moves on again while the second one waits — the longest window there is.
    common::replace_declarations(tmp.path(), &catalogue("VERSION-THREE"));

    // The holder answers, which releases the lease and fires the queue.
    let first_session = worker.trigger_for("solo");
    engine
        .reply_thread(
            as_session(&first_session.internal_session),
            ReplyThreadRequest {
                machine: None,
                thread: &held.thread_id,
                body: "done",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply is posted");

    let started: Vec<String> = worker
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.role.system_prompt.clone())
        .collect();
    assert_eq!(started.len(), 2, "the parked commission fired: {started:?}");
    assert!(started[0].contains("VERSION-ONE"), "{started:?}");
    assert!(
        started[1].contains("VERSION-TWO"),
        "the released trigger was composed from ITS OWN operation's version — not the holder's \
         (VERSION-ONE, what dropping this one substitution gives) and not the folder's \
         (VERSION-THREE, what dropping the freeze gives): {started:?}"
    );
}
