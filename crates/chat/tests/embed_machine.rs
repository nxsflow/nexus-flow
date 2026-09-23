//! **The executing machine at the seam the products speak** (nxf 6j6v.1c6k): `Engine::send_to`,
//! `Engine::reply_thread`, `Engine::machine` and `Engine::pick_up`, with the host's half passed as
//! [`EngineConfig::machines`]. `the_executing_machine.rs` drives the same behaviour one layer down;
//! this is the path manufakt.io and every other embedding host takes.
//!
//! Two workspaces stand for two machines. Ops travel between them the way a relay carries them —
//! the log exported from one and applied to the other — and each trusts the other's key, the way
//! `nxs sync trust add` does.

mod common;

use std::path::Path;
use std::sync::{Arc, Mutex};

use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::machine::{DesignationSource, FixedMachines, MachineRef, MachineSeen, Machines};
use nexus_chat::orchestration::Caller;
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{MachineQuery, ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-09-22T10:00:00Z";

#[derive(Default)]
struct RecordingWorker {
    seen: Mutex<Vec<TriggerRequest>>,
}

impl Worker for RecordingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }
}

fn machine(id: &str, name: &str) -> MachineRef {
    MachineRef {
        machine_id: id.into(),
        name: name.into(),
    }
}

fn online(m: &MachineRef, online: bool) -> MachineSeen {
    MachineSeen {
        machine_id: m.machine_id.clone(),
        name: m.name.clone(),
        online,
        age_secs: if online { 20 } else { 7200 },
    }
}

fn laptop() -> MachineRef {
    machine("01jlaptop", "laptop")
}

fn studio() -> MachineRef {
    machine("01jstudio", "studio")
}

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let coder: RoleDecl =
        serde_yaml::from_str("handle: coder\nsystem_prompt: You are coder.\n").unwrap();
    common::write_declarations(tmp.path(), &Definitions::new(vec![coder], vec![]).unwrap());
    tmp
}

fn engine(dir: &Path, worker: Arc<RecordingWorker>, machines: Arc<dyn Machines>) -> Engine {
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

fn caller(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

/// `to` takes in everything `from` holds, and trusts `from`'s key.
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
fn a_chat_sent_from_one_machine_runs_on_the_other_through_the_engine() {
    let (lap_dir, stu_dir) = (workspace(), workspace());
    let (lw, sw) = (
        Arc::new(RecordingWorker::default()),
        Arc::new(RecordingWorker::default()),
    );
    let both = vec![online(&laptop(), true), online(&studio(), true)];
    let lap = engine(
        lap_dir.path(),
        lw.clone(),
        Arc::new(FixedMachines::new(Some(laptop()), Ok(both.clone()))),
    );
    let stu = engine(
        stu_dir.path(),
        sw.clone(),
        Arc::new(FixedMachines::new(Some(studio()), Ok(both))),
    );

    // The question first, as an app renders it: nothing named, so this machine — no asking.
    let answer = lap
        .machine(
            caller("alice"),
            MachineQuery::Persona {
                to: "coder",
                machine: None,
            },
        )
        .unwrap();
    let here = answer.machine.clone().unwrap();
    assert_eq!(
        (
            here.machine_id.as_str(),
            here.source,
            here.here,
            answer.must_ask
        ),
        ("01jlaptop", DesignationSource::StartedHere, true, false)
    );
    assert_eq!(
        answer.online.len(),
        0,
        "this machine answers for itself without the relay"
    );

    let r = lap
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
    assert!(!r.spawned);
    assert_eq!(r.handed_to.unwrap().machine_id, "01jstudio");
    assert!(lw.seen.lock().unwrap().is_empty());

    carry(lap_dir.path(), stu_dir.path());
    let report = stu.pick_up(caller("service")).unwrap();
    assert_eq!(report.served.len(), 1, "{report:?}");
    assert_eq!(report.machine.unwrap().machine_id, "01jstudio");
    assert_eq!(sw.seen.lock().unwrap().len(), 1);
    assert!(stu.pick_up(caller("service")).unwrap().served.is_empty());

    // And the chat, read back on the laptop: where it runs, from the chat itself.
    let on = lap
        .machine(
            caller("alice"),
            MachineQuery::Thread {
                thread: &r.thread_id,
                machine: None,
            },
        )
        .unwrap();
    let m = on.machine.unwrap();
    assert_eq!(
        (m.machine_id.as_str(), m.source, m.here),
        ("01jstudio", DesignationSource::Chat, false)
    );
}

#[test]
fn an_absent_machine_is_asked_about_at_the_seam_and_nothing_is_written() {
    let dir = workspace();
    let w = Arc::new(RecordingWorker::default());
    let eng = engine(
        dir.path(),
        w.clone(),
        Arc::new(FixedMachines::new(
            Some(laptop()),
            Ok(vec![online(&laptop(), true), online(&studio(), false)]),
        )),
    );
    let first = eng
        .send_to(
            caller("alice"),
            SendToRequest {
                to: "coder",
                body: "go",
                refs: SendToRefs::ExplicitlyNone,
                machine: Some("laptop"),
            },
        )
        .unwrap();
    // Hand it to the studio while it is online…
    let studio_up = engine(
        dir.path(),
        w.clone(),
        Arc::new(FixedMachines::new(
            Some(laptop()),
            Ok(vec![online(&laptop(), true), online(&studio(), true)]),
        )),
    );
    studio_up
        .reply_thread(
            caller("alice"),
            ReplyThreadRequest {
                thread: &first.thread_id,
                body: "over to the studio",
                escalate: false,
                needs_rework: false,
                accept: false,
                machine: Some("studio"),
            },
        )
        .unwrap();
    // …and now it is gone: a reply asks, and posts nothing.
    let before = eng
        .thread(&first.thread_id, &format!("{}/alice", eng.origin()))
        .unwrap()
        .messages
        .len();
    let err = eng
        .reply_thread(
            caller("alice"),
            ReplyThreadRequest {
                thread: &first.thread_id,
                body: "still there?",
                escalate: false,
                needs_rework: false,
                accept: false,
                machine: None,
            },
        )
        .unwrap_err();
    assert!(err.msg.contains("not online"), "{}", err.msg);
    assert!(err.msg.contains("laptop (01jlaptop"), "{}", err.msg);
    assert_eq!(
        eng.thread(&first.thread_id, &format!("{}/alice", eng.origin()))
            .unwrap()
            .messages
            .len(),
        before
    );
    let asked = eng
        .machine(
            caller("alice"),
            MachineQuery::Thread {
                thread: &first.thread_id,
                machine: None,
            },
        )
        .unwrap();
    assert!(asked.must_ask);
    assert_eq!(asked.online.len(), 1);
}
