//! **The `-v2` directory guard, for the verbs that reach a session only through the host's capabilities**
//! (6j6v.pn9c; epic 6j6v.zjyv §6.6; round 6 CQ #13, TQ #12, IR #1): `pick_up` starts a persona for a chat the
//! machine register hands to this machine, `handover` admits a commission another workspace sent across the
//! border, and `confirm` answers a coordinator's nudge. Each runs here through `Engine::*` opened in
//! `<root>-v2`, with the host's own `Machines` or `Peers`, and is checked against the same call from the
//! registered root. `v2_directory_guard.rs` has the other verbs and the directory recognition.

mod common;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nexus_chat::border::{PeerWorkspace, Peers};
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::ladder::SILENCE_SECS;
use nexus_chat::machine::{FixedMachines, MachineRef, MachineSeen, Machines};
use nexus_chat::orchestration::{self, Caller, TickRequest};
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{SendToRefs, SendToRequest};
use nexus_chat::timer::{DisabledTimer, TimerConfig};
use nexus_chat::transcript::TranscriptEntry;
use nexus_chat::worker::{
    AskOutcome, SessionAsk, SessionStop, TriggerOutcome, TriggerRequest, TriggerResult, Worker,
    WorkerConfig,
};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-11-13T10:00:00Z";

/// Every session the old engine hands its worker, from any workspace of the test; running while it is told so.
#[derive(Default)]
struct Hand {
    seen: Mutex<Vec<TriggerRequest>>,
    running: Mutex<HashSet<String>>,
    asked: Mutex<Vec<SessionAsk>>,
}

impl std::fmt::Debug for Hand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hand")
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
        true
    }
    fn asks_sessions(&self) -> bool {
        true
    }
    fn stops_sessions(&self) -> bool {
        true
    }
    fn stop_session(&self, _internal_session: &str) -> Result<SessionStop, String> {
        Ok(SessionStop::Requested)
    }
    fn ask_session(&self, ask: &SessionAsk) -> AskOutcome {
        if !self.session_is_running(&ask.session) {
            return AskOutcome::NoProcess;
        }
        self.asked.lock().unwrap().push(ask.clone());
        AskOutcome::Asked
    }
    fn withdraw_asks(&self, _internal_session: &str) -> Result<(), String> {
        Ok(())
    }
}

impl Hand {
    fn handles(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.role.handle.clone())
            .collect()
    }
}

fn role(yaml: &str) -> RoleDecl {
    serde_yaml::from_str(yaml).expect("test role parses")
}

/// A workspace at `root` declaring `roles`, optionally named for the border.
fn workspace(root: &Path, name: Option<&str>, roles: &[&str]) {
    std::fs::create_dir_all(root).unwrap();
    setup(root, &chat_config()).expect("seed chat workspace");
    if let Some(name) = name {
        nxs_foundation::workspace_name::set_name(&root.join(".nxs"), name).expect("name it");
    }
    declare(root, roles);
}

fn declare(dir: &Path, roles: &[&str]) {
    let defs = Definitions::new(roles.iter().map(|y| role(y)).collect(), vec![]).unwrap();
    common::write_declarations(dir, &defs);
}

/// `<root>-v2` beside `root`: its `.nxs` resolves where the root's does, and it carries the declarations a
/// clone would.
fn v2_dir_of(root: &Path, roles: &[&str]) -> PathBuf {
    let mut name = root.as_os_str().to_owned();
    name.push("-v2");
    let v2 = PathBuf::from(name);
    std::fs::create_dir_all(&v2).unwrap();
    std::os::unix::fs::symlink(root.join(".nxs"), v2.join(".nxs")).unwrap();
    declare(&v2, roles);
    v2
}

fn base() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    (tmp, base)
}

/// The ladder's own record: a confirmed rest is a row here.
fn recovery_rows(root: &Path) -> i64 {
    rusqlite::Connection::open(root.join(".nxs").join("db.sqlite"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM recovery_log", [], |row| row.get(0))
        .unwrap()
}

fn caller(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

fn names_the_root(text: &str, root: &Path, v2: &Path) -> bool {
    text.contains(&root.display().to_string()) && text.contains(&v2.display().to_string())
}

// ---- pick_up -------------------------------------------------------------------------------------------------

const CODER: &str = "handle: coder\nsystem_prompt: You are coder.\n";

fn machine(id: &str, name: &str) -> MachineRef {
    MachineRef {
        machine_id: id.into(),
        name: name.into(),
    }
}

fn seen(m: &MachineRef) -> MachineSeen {
    MachineSeen {
        machine_id: m.machine_id.clone(),
        name: m.name.clone(),
        online: true,
        age_secs: 20,
    }
}

fn machine_engine(
    dir: &Path,
    worker: Arc<Hand>,
    here: MachineRef,
    all: Vec<MachineSeen>,
) -> Engine {
    let machines: Arc<dyn Machines> = Arc::new(FixedMachines::new(Some(here), Ok(all)));
    Engine::open_with(
        None,
        dir,
        EngineConfig {
            worker: WorkerConfig::Custom(worker),
            timer: TimerConfig::Disabled,
            machines: Some(machines),
            ..EngineConfig::default()
        },
    )
    .expect("open engine")
}

/// `to` takes in everything `from` holds, and trusts `from`'s key, as a relay pass would leave it.
fn carry(from: &Path, to: &Path) {
    let open = |dir: &Path| {
        Workspace::resolve(None, dir)
            .unwrap()
            .open_chat_store()
            .unwrap()
    };
    let source = open(from);
    let mut target = open(to);
    target.trust_key(source.key_id(), "peer", NOW).unwrap();
    target.apply(&source.export());
}

#[test]
fn a_pick_up_in_the_v2_directory_starts_no_persona_there_and_names_the_root() {
    let (_tmp, base) = base();
    let (laptop, studio) = (
        machine("01jlaptop", "laptop"),
        machine("01jstudio", "studio"),
    );
    let both = vec![seen(&laptop), seen(&studio)];
    // The chat is sent on the laptop and handed to the studio, whose workspace has a v2 directory.
    let lap_root = base.join("laptop");
    workspace(&lap_root, None, &[CODER]);
    let root = base.join("studio");
    workspace(&root, None, &[CODER]);
    let v2 = v2_dir_of(&root, &[CODER]);
    let lap_worker = Arc::new(Hand::default());
    let sent = machine_engine(&lap_root, lap_worker.clone(), laptop, both.clone())
        .send_to(
            caller("alice"),
            SendToRequest {
                to: "coder",
                body: "Add a --since flag.",
                refs: SendToRefs::ExplicitlyNone,
                machine: Some("studio"),
            },
        )
        .unwrap();
    assert!(!sent.spawned, "handed to the studio, started nowhere");
    carry(&lap_root, &root);

    let worker = Arc::new(Hand::default());
    let report = machine_engine(&v2, worker.clone(), studio.clone(), both.clone())
        .pick_up(caller("service"))
        .expect("the pickup reports");
    assert!(
        worker.handles().is_empty(),
        "no persona was started in the v2 directory: {:?}",
        worker.handles()
    );
    assert!(report.served.is_empty(), "{report:?}");
    let said = format!("{:?}", report.failed);
    assert!(
        names_the_root(&said, &root, &v2),
        "the refusal names the root: {said}"
    );

    // From the root the same chat is picked up: the refused pickup gave its claim back.
    let report = machine_engine(&root, worker.clone(), studio, both)
        .pick_up(caller("service"))
        .unwrap();
    assert_eq!(report.served.len(), 1, "{report:?}");
    assert_eq!(worker.handles(), ["coder"]);
}

// ---- handover ------------------------------------------------------------------------------------------------

/// Both workspaces by name; the other side's handover never runs from a send, so the test decides where it runs.
#[derive(Debug)]
struct NamedPeers {
    by_name: Vec<(String, PathBuf)>,
}

impl Peers for NamedPeers {
    fn here(&self, db_path: &str) -> Result<Option<String>, String> {
        let db = std::fs::canonicalize(db_path).map_err(|e| e.to_string())?;
        Ok(self
            .by_name
            .iter()
            .find(|(_, root)| {
                std::fs::canonicalize(root.join(".nxs/db.sqlite")).ok() == Some(db.clone())
            })
            .map(|(name, _)| name.clone()))
    }

    fn find(&self, name: &str) -> Result<Vec<PeerWorkspace>, String> {
        Ok(self
            .by_name
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(n, root)| PeerWorkspace {
                name: n.clone(),
                root: root.clone(),
            })
            .collect())
    }

    fn hand_over_in(&self, peer: &PeerWorkspace) -> Result<(), String> {
        Err(format!("{} is busy", peer.name))
    }
}

fn peer_engine(dir: &Path, worker: Arc<Hand>, peers: Arc<NamedPeers>) -> Engine {
    Engine::open_with(
        None,
        dir,
        EngineConfig {
            worker: WorkerConfig::Custom(worker),
            timer: TimerConfig::Dry,
            peers: Some(peers),
            ..EngineConfig::default()
        },
    )
    .expect("open engine")
}

const ALPHA_PM: &str = "handle: pm\nsystem_prompt: You are the PM of test/alpha.\n";
const BETA_PM: &str = "handle: pm\nsystem_prompt: You are the PM of test/beta.\naddressable:\n  humans: true\n  external:\n    - \"*/pm\"\n";

fn trust(root: &Path, other: &Path, name: &str) {
    let key = Workspace::resolve(None, other)
        .unwrap()
        .open_chat_store()
        .unwrap()
        .key_id()
        .to_string();
    let mut store = Workspace::resolve(None, root)
        .unwrap()
        .open_chat_store()
        .unwrap();
    store.trust_key(&key, name, NOW).unwrap();
}

#[test]
fn a_handover_in_the_v2_directory_admits_no_commission_there_and_names_the_root() {
    let (_tmp, base) = base();
    let alpha = base.join("alpha");
    workspace(&alpha, Some("test/alpha"), &[ALPHA_PM]);
    let root = base.join("beta");
    workspace(&root, Some("test/beta"), &[BETA_PM]);
    let v2 = v2_dir_of(&root, &[BETA_PM]);
    trust(&alpha, &root, "test/beta");
    trust(&root, &alpha, "test/alpha");
    let peers = Arc::new(NamedPeers {
        by_name: vec![
            ("test/alpha".to_owned(), alpha.clone()),
            ("test/beta".to_owned(), root.clone()),
        ],
    });
    let worker = Arc::new(Hand::default());
    // The alpha PM commissions beta's PM across the border; beta's own handover has not run yet.
    let alpha_engine = peer_engine(&alpha, worker.clone(), peers.clone());
    alpha_engine
        .send_to(caller("carsten"), send("pm", "find it out"))
        .unwrap();
    let pm = worker.seen.lock().unwrap()[0].internal_session.clone();
    alpha_engine
        .send_to(
            Caller {
                session: Some(&pm),
                actor: None,
                now: Some(NOW),
            },
            send("test/beta/pm", "When does the access rule ship?"),
        )
        .expect("the commission is recorded");
    assert_eq!(worker.handles(), ["pm"], "only alpha's pm runs");

    // Beta's handover, run in its v2 directory: the commission would start beta's pm there.
    let handover = peer_engine(&v2, worker.clone(), peers.clone()).handover(caller("handover"));
    let said = match &handover {
        Ok(report) => format!("{report:?}"),
        Err(e) => e.to_string(),
    };
    assert_eq!(
        worker.handles(),
        ["pm"],
        "nothing was started in the v2 directory"
    );
    assert!(
        names_the_root(&said, &root, &v2),
        "the refusal names the root: {said}"
    );

    // From beta's root, beta's pm is started.
    peer_engine(&root, worker.clone(), peers)
        .handover(caller("handover"))
        .expect("the handover runs");
    assert_eq!(
        worker.handles(),
        ["pm", "pm"],
        "beta's pm was started from the root"
    );
}

fn send<'a>(to: &'a str, body: &'a str) -> SendToRequest<'a> {
    SendToRequest {
        to,
        body,
        refs: SendToRefs::ExplicitlyNone,
        machine: None,
    }
}

// ---- confirm -------------------------------------------------------------------------------------------------

fn later(secs: i64) -> String {
    use time::format_description::well_known::Rfc3339;
    let t = time::OffsetDateTime::parse(NOW, &Rfc3339).unwrap() + time::Duration::seconds(secs);
    t.format(&Rfc3339).unwrap()
}

fn plain_engine(dir: &Path, worker: Arc<Hand>) -> Engine {
    Engine::open_with(
        None,
        dir,
        EngineConfig {
            worker: WorkerConfig::Custom(worker),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine")
}

/// The coordinator's look at `thread` from the root, `secs` after `NOW`: what nudges a silent session.
fn look(root: &Path, engine: &Engine, worker: &Arc<Hand>, thread: &str, secs: i64) {
    let ws = Workspace::resolve(None, root).unwrap();
    let db_path = ws.db_path_str().unwrap();
    let mut store = ws.open_chat_store().unwrap();
    let defs = engine.definitions().unwrap();
    let origin = engine.origin();
    let now = later(secs);
    let ctx = orchestration::Ctx {
        now: &now,
        origin: &origin,
        actor: "service",
        session: None,
        hop: 0,
        defs: &defs,
        worker: worker.as_ref(),
        timer: &DisabledTimer,
        namer: &nexus_chat::naming::DryNamer,
        db_path: &db_path,
        project_claude_md: None,
        module_primes: None,
        machines: None,
        peers: None,
        liveness: Default::default(),
    };
    orchestration::tick(&ctx, &mut store, TickRequest { thread_id: thread }).expect("tick");
}

#[test]
fn a_confirm_typed_in_the_v2_directory_records_its_rest_and_starts_nothing() {
    let (_tmp, base) = base();
    let root = base.join("ws");
    workspace(&root, None, &[CODER]);
    let v2 = v2_dir_of(&root, &[CODER]);
    let worker = Arc::new(Hand::default());
    let at_root = plain_engine(&root, worker.clone());
    let thread = at_root
        .send_to(caller("carsten"), send("coder", "Add a --since flag."))
        .unwrap()
        .thread_id;
    let session = worker.seen.lock().unwrap()[0].internal_session.clone();
    worker.running.lock().unwrap().insert(session.clone());
    let entry: TranscriptEntry = serde_json::from_value(serde_json::json!({
        "kind": "turn_start",
        "at": later(1),
        "toolUseId": null,
        "data": { "thread": thread },
    }))
    .unwrap();
    at_root.transcript_append(&session, &[entry]).unwrap();
    look(&root, &at_root, &worker, &thread, 1 + SILENCE_SECS);
    assert_eq!(
        worker.asked.lock().unwrap().len(),
        1,
        "the session was nudged"
    );
    let started = worker.handles().len();
    let before = recovery_rows(&root);

    // `confirm` answers the nudge and starts no session: it is not refused in the v2 directory, and it
    // leaves no spawn record there.
    let receipt = plain_engine(&v2, worker.clone())
        .confirm_rest(
            Caller {
                session: Some(&session),
                actor: None,
                now: Some(&later(1 + SILENCE_SECS + 30)),
            },
            &thread,
            20,
        )
        .expect("the rest is granted");
    assert_eq!(receipt.granted_minutes, 20);
    assert!(recovery_rows(&root) > before, "the rest was recorded");
    assert_eq!(worker.handles().len(), started, "no session was started");
    let logs = v2.join(".nxs").join("agent-logs");
    let spawned: Vec<_> = std::fs::read_dir(&logs)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "pid"))
                .collect()
        })
        .unwrap_or_default();
    assert!(spawned.is_empty(), "no spawn record: {spawned:?}");
}
