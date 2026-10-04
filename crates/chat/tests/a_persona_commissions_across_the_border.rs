//! **A persona commissions a persona of ANOTHER workspace on the same machine** (nxf 6j6v.70dy:
//! 6j6v.q32p, 6j6v.t5xb, 6j6v.4gp2, 6j6v.szc5) — at the engine seam, between two real workspaces.
//!
//! Each workspace is a real directory with its own `.nxs/` log, its own replica key and its own
//! declarations; the only thing they share is a recording worker (so the test can see who was
//! started where) and a [`Peers`] that knows both by name and runs the other side's handover
//! in-process — what the `nxs` binary does with a child process in the other directory.
//!
//! The acceptance points of the slice's specification (section 9) this file holds:
//!
//! - 1 — the scenario: the PM of `test/alpha` asks the PM of `test/beta`, the answer wakes it;
//! - 2 — the coder is refused, and the refusal names "role not admitted";
//! - 3 — an untrusted workspace is refused, and a foreign `pm` does not pass `personas: [pm]`;
//! - 4 — a question from the foreign PM wakes the commissioner, nobody else;
//! - 5 — a back-and-forth ends at the depth cap;
//! - 6 — only the border thread crosses;
//! - 8 — without `external` and without peers, nothing changes.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nexus_chat::border::{BorderState, PeerWorkspace, Peers};
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::orchestration::Caller;
use nexus_chat::role::RoleDecl;
use nexus_chat::surface::{ReplyThreadRequest, SendToReceipt, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{
    Coordinator, TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig,
};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt};
use tempfile::TempDir;

const NOW: &str = "2026-10-03T10:00:00Z";

/// Every trigger, with the workspace it was handed to — told apart by the database the spawned
/// session would write to (`NXC_DB` in the request's env).
#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<TriggerRequest>>,
    stopped: Mutex<Vec<String>>,
    /// Sessions that are mid-turn: a resume of one is refused as already running, which is what
    /// makes an answer for it HELD.
    busy: Mutex<Vec<String>>,
}

impl Recorder {
    fn all(&self) -> Vec<TriggerRequest> {
        self.seen.lock().unwrap().clone()
    }
    /// The triggers handed to `workspace` for persona `handle`, oldest first.
    fn of(&self, workspace: &Side, handle: &str) -> Vec<TriggerRequest> {
        self.all()
            .into_iter()
            .filter(|r| r.role.handle == handle && workspace.owns(r))
            .collect()
    }
}

impl Worker for Recorder {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        if self.busy.lock().unwrap().contains(&req.internal_session) {
            return Err(nexus_chat::worker::TriggerError::AlreadyRunning {
                session: req.internal_session,
                pid: 4242,
            });
        }
        self.seen.lock().unwrap().push(req);
        Ok(TriggerOutcome::Accepted)
    }
    /// Every session here has done its turn and returned — the recorder runs nothing — and it says
    /// so, which is what lets a withdrawal go ahead.
    fn answers_liveness(&self) -> bool {
        true
    }
    fn stop_session(
        &self,
        internal_session: &str,
    ) -> Result<nexus_chat::worker::SessionStop, String> {
        self.stopped
            .lock()
            .unwrap()
            .push(internal_session.to_string());
        Ok(nexus_chat::worker::SessionStop::Requested)
    }
}

/// What the two workspaces share: the worker every spawn of either reaches.
struct TwoWorkspaces {
    worker: Arc<Recorder>,
    failing_kicks: Arc<std::sync::atomic::AtomicUsize>,
}

/// The test's [`Peers`]: both workspaces by name, and the other side's handover run in-process
/// through its own [`Engine`].
#[derive(Debug, Clone)]
struct InProcessPeers {
    by_name: Vec<(String, PathBuf)>,
    worker: Arc<Recorder>,
    /// How many of the next peer handovers fail before running — a peer database that was busy,
    /// a binary that was gone.
    failing_kicks: Arc<std::sync::atomic::AtomicUsize>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Recorder")
    }
}

impl Peers for InProcessPeers {
    fn here(&self, db_path: &str) -> Result<Option<String>, String> {
        let db = std::fs::canonicalize(db_path).map_err(|e| e.to_string())?;
        Ok(self
            .by_name
            .iter()
            .find(|(_, root)| {
                std::fs::canonicalize(root.join(".nxs/db.sqlite"))
                    .ok()
                    .as_ref()
                    == Some(&db)
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
        use std::sync::atomic::Ordering;
        let left = self.failing_kicks.load(Ordering::SeqCst);
        if left > 0
            && self
                .failing_kicks
                .compare_exchange(left, left - 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            return Err(format!("{} is busy", peer.name));
        }
        let engine = open_engine(&peer.root, self.worker.clone(), Arc::new(self.clone()));
        engine
            .handover(machine_caller())
            .map(|_| ())
            .map_err(|e| e.msg)
    }
}

fn open_engine(root: &Path, worker: Arc<Recorder>, peers: Arc<dyn Peers>) -> Engine {
    Engine::open_with(
        None,
        root,
        EngineConfig {
            worker: WorkerConfig::Custom(worker),
            timer: TimerConfig::Dry,
            peers: Some(peers),
            ..EngineConfig::default()
        },
    )
    .expect("open engine")
}

/// The coordinator running the handover, which acts in nobody's name of its own.
fn machine_caller() -> Caller<'static> {
    Caller {
        session: None,
        actor: Some("handover"),
        now: Some(NOW),
    }
}

/// One side, opened.
struct Side {
    root: PathBuf,
    engine: Engine,
}

impl Side {
    fn owns(&self, req: &TriggerRequest) -> bool {
        let db = std::fs::canonicalize(self.root.join(".nxs/db.sqlite")).unwrap();
        req.env
            .iter()
            .find(|(k, _)| k == "NXC_DB")
            .and_then(|(_, v)| std::fs::canonicalize(v).ok())
            .is_some_and(|v| v == db)
    }

    fn as_person(&self, to: &str, body: &str) -> SendToReceipt {
        self.engine
            .send_to(
                Caller {
                    session: None,
                    actor: Some("carsten"),
                    now: Some(NOW),
                },
                send(to, body),
            )
            .unwrap_or_else(|e| panic!("send to {to}: {}", e.msg))
    }

    fn as_session(&self, session: &str, to: &str, body: &str) -> Result<SendToReceipt, String> {
        self.engine
            .send_to(
                Caller {
                    session: Some(session),
                    actor: None,
                    now: Some(NOW),
                },
                send(to, body),
            )
            .map_err(|e| e.msg)
    }

    fn reply(&self, session: &str, thread: &str, body: &str, escalate: bool) {
        self.engine
            .reply_thread(
                Caller {
                    session: Some(session),
                    actor: None,
                    now: Some(NOW),
                },
                ReplyThreadRequest {
                    machine: None,
                    thread,
                    body,
                    escalate,
                    needs_rework: false,
                    accept: false,
                },
            )
            .unwrap_or_else(|e| panic!("reply on {thread}: {}", e.msg));
    }

    fn border_rows(&self) -> Vec<nexus_chat::border::BorderRow> {
        let ws = nxs_foundation::workspace::discover(&self.root).unwrap();
        let store = ws.open_chat_store().unwrap();
        nexus_chat::border::rows(&store).unwrap()
    }

    fn thread_ids(&self) -> Vec<String> {
        let ws = nxs_foundation::workspace::discover(&self.root).unwrap();
        let store = ws.open_chat_store().unwrap();
        store
            .thread_edges()
            .unwrap()
            .into_iter()
            .map(|(id, _, _)| id)
            .collect()
    }
}

fn send<'a>(to: &'a str, body: &'a str) -> SendToRequest<'a> {
    SendToRequest {
        to,
        body,
        refs: SendToRefs::ExplicitlyNone,
        machine: None,
    }
}

fn role(yaml: &str) -> RoleDecl {
    serde_yaml::from_str(yaml).expect("test role parses")
}

/// A workspace at `root` named `name`, declaring `roles`.
fn workspace(root: &Path, name: &str, roles: Vec<RoleDecl>) {
    std::fs::create_dir_all(root).unwrap();
    setup(root, &chat_config()).expect("seed chat workspace");
    nxs_foundation::workspace_name::set_name(&root.join(".nxs"), name).expect("name it");
    let defs = Definitions::new(roles, vec![]).expect("catalogue");
    common::write_declarations(root, &defs);
}

fn key_of(root: &Path) -> String {
    let ws = nxs_foundation::workspace::discover(root).unwrap();
    ws.open_chat_store().unwrap().key_id().to_string()
}

fn trust(root: &Path, other: &Path, name: &str) {
    let ws = nxs_foundation::workspace::discover(root).unwrap();
    let mut store = ws.open_chat_store().unwrap();
    store.trust_key(&key_of(other), name, NOW).unwrap();
}

const ALPHA_PM: &str = "handle: pm\nsystem_prompt: You are the PM of test/alpha.\n";
const ALPHA_CODER: &str = "handle: coder\nsystem_prompt: You are a coder.\n";
const BETA_PM: &str = "handle: pm\nsystem_prompt: You are the PM of test/beta.\naddressable:\n  humans: true\n  external:\n    - \"*/pm\"\n";

/// Both workspaces, trusting each other unless told otherwise.
fn two(
    tmp: &TempDir,
    alpha_roles: &[&str],
    beta_roles: &[&str],
    alpha_trusts_beta: bool,
    beta_trusts_alpha: bool,
) -> (Side, Side, TwoWorkspaces) {
    let alpha = tmp.path().join("alpha");
    let beta = tmp.path().join("beta");
    workspace(
        &alpha,
        "test/alpha",
        alpha_roles.iter().map(|y| role(y)).collect(),
    );
    workspace(
        &beta,
        "test/beta",
        beta_roles.iter().map(|y| role(y)).collect(),
    );
    if alpha_trusts_beta {
        trust(&alpha, &beta, "test/beta");
    }
    if beta_trusts_alpha {
        trust(&beta, &alpha, "test/alpha");
    }
    let worker = Arc::new(Recorder::default());
    let failing_kicks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let peers = Arc::new(InProcessPeers {
        by_name: vec![
            ("test/alpha".to_string(), alpha.clone()),
            ("test/beta".to_string(), beta.clone()),
        ],
        worker: worker.clone(),
        failing_kicks: failing_kicks.clone(),
    });
    let a = Side {
        root: alpha.clone(),
        engine: open_engine(&alpha, worker.clone(), peers.clone()),
    };
    let b = Side {
        root: beta.clone(),
        engine: open_engine(&beta, worker.clone(), peers),
    };
    (
        a,
        b,
        TwoWorkspaces {
            worker,
            failing_kicks,
        },
    )
}

/// The person commissions alpha's `handle`, and the session started for it is returned.
fn alpha_session(a: &Side, w: &TwoWorkspaces, handle: &str) -> (String, String) {
    let receipt = a.as_person(handle, "find it out");
    let session = w
        .worker
        .of(a, handle)
        .last()
        .expect("the persona was started")
        .internal_session
        .clone();
    (session, receipt.thread_id)
}

// ---- 1: the scenario -------------------------------------------------------------------------

#[test]
fn the_pm_of_one_workspace_asks_the_pm_of_another_and_is_woken_with_the_answer() {
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let (alpha_pm, owners_thread) = alpha_session(&a, &w, "pm");

    let receipt = a
        .as_session(&alpha_pm, "test/beta/pm", "When does the access rule ship?")
        .expect("the commission is recorded and handed over");
    let border = receipt.border.clone().expect("a border receipt");
    assert_eq!(border.peer, "test/beta");
    assert_eq!(border.state, BorderState::Working, "{receipt:?}");

    // The receiver's coordinator admitted it and started ITS pm, owing the border thread a reply.
    let started = w.worker.of(&b, "pm");
    let [beta_pm] = started.as_slice() else {
        panic!("exactly one pm started in beta: {started:?}")
    };
    assert_eq!(beta_pm.coordinator, Coordinator::Border);
    assert_eq!(
        beta_pm.reply_thread.as_deref(),
        Some(receipt.thread_id.as_str())
    );
    assert!(
        beta_pm.message.contains("From test/alpha/pm"),
        "the persona is told who asks: {}",
        beta_pm.message
    );

    b.reply(
        &beta_pm.internal_session,
        &receipt.thread_id,
        "With version 0.300.",
        false,
    );

    // The answer crossed back and woke the commissioner — the same session, with the answer.
    let woken = w.worker.of(&a, "pm");
    let resumed = woken.last().expect("alpha's pm was woken");
    assert_eq!(resumed.internal_session, alpha_pm);
    assert_eq!(resumed.coordinator, Coordinator::Return);
    assert!(resumed.message.contains("0.300"), "{}", resumed.message);
    // …and it can pass it on: the wake carries the obligation it still has to its owner, so a pm
    // declaring no tools is granted the means to answer.
    assert_eq!(
        resumed.reply_thread.as_deref(),
        Some(owners_thread.as_str())
    );
    assert!(!resumed.role.granted_tools.is_empty(), "{:?}", resumed.role);

    let out = a.border_rows();
    assert_eq!(out[0].state, BorderState::Completed, "{out:?}");
    let inbound = b.border_rows();
    assert_eq!(inbound[0].state, BorderState::Completed, "{inbound:?}");
    assert_eq!(inbound[0].peer, "test/alpha");
}

// ---- 2 and 3: who is refused, and why ----------------------------------------------------------

#[test]
fn the_coder_is_refused_with_role_not_admitted_and_nothing_starts_there() {
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM, ALPHA_CODER], &[BETA_PM], true, true);
    let (coder, _) = alpha_session(&a, &w, "coder");

    let receipt = a
        .as_session(&coder, "test/beta/pm", "When does the access rule ship?")
        .expect("the commission is recorded; the refusal comes back as its answer");
    let border = receipt.border.expect("a border receipt");
    assert_eq!(border.state, BorderState::Rejected);
    assert_eq!(border.reason.as_deref(), Some("role not admitted"));
    assert!(
        w.worker.of(&b, "pm").is_empty(),
        "nothing was started in beta"
    );

    let woken = w.worker.of(&a, "coder");
    assert!(
        woken.last().unwrap().message.contains("role not admitted"),
        "the coder is woken with the reason: {}",
        woken.last().unwrap().message
    );
}

#[test]
fn a_workspace_the_receiver_does_not_trust_is_refused_as_not_trusted() {
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, false);
    let (alpha_pm, _) = alpha_session(&a, &w, "pm");

    let receipt = a
        .as_session(&alpha_pm, "test/beta/pm", "When?")
        .expect("recorded");
    let border = receipt.border.expect("a border receipt");
    assert_eq!(border.state, BorderState::Rejected);
    assert_eq!(border.reason.as_deref(), Some("not trusted"));
    assert!(w.worker.of(&b, "pm").is_empty());
}

#[test]
fn the_caller_refuses_up_front_when_it_does_not_trust_the_receiver_itself() {
    // The answer would arrive signed by a key the caller's workspace does not believe, and an
    // unvouched message wakes nobody — so the caller is told before anything is written.
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], false, true);
    let (alpha_pm, _) = alpha_session(&a, &w, "pm");

    let err = a
        .as_session(&alpha_pm, "test/beta/pm", "When?")
        .expect_err("refused");
    assert!(err.contains("not trusted"), "{err}");
    assert!(
        err.contains("nxs sync trust add --workspace test/beta"),
        "{err}"
    );
    assert!(a.border_rows().is_empty() && b.border_rows().is_empty());
}

#[test]
fn a_foreign_pm_does_not_pass_a_bare_personas_entry() {
    // `personas: [pm]` means THIS workspace's pm. A handle reduced to its last segment would have let
    // `test/alpha/pm` through (nxf 6j6v.q32p).
    let tmp = TempDir::new().unwrap();
    let only_own_pm = "handle: lead\nsystem_prompt: You lead.\naddressable:\n  personas: [pm]\n";
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM, only_own_pm], true, true);
    let (alpha_pm, _) = alpha_session(&a, &w, "pm");

    let receipt = a
        .as_session(&alpha_pm, "test/beta/lead", "Hello")
        .expect("recorded");
    let border = receipt.border.expect("a border receipt");
    assert_eq!(border.state, BorderState::Rejected);
    assert_eq!(border.reason.as_deref(), Some("role not admitted"));
    assert!(w.worker.of(&b, "lead").is_empty());
}

#[test]
fn a_person_cannot_commission_a_persona_of_another_workspace() {
    let tmp = TempDir::new().unwrap();
    let (a, _b, _w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let err = a
        .engine
        .send_to(
            Caller {
                session: None,
                actor: Some("carsten"),
                now: Some(NOW),
            },
            send("test/beta/pm", "Hello"),
        )
        .expect_err("a person goes to that workspace");
    assert!(
        err.msg.contains("only a persona can commission it"),
        "{}",
        err.msg
    );
}

#[test]
fn an_address_nobody_on_this_machine_carries_is_not_on_this_machine() {
    let tmp = TempDir::new().unwrap();
    let (a, _b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let (alpha_pm, _) = alpha_session(&a, &w, "pm");
    let err = a
        .as_session(&alpha_pm, "test/gamma/pm", "Hello")
        .expect_err("refused");
    assert!(err.contains("not on this machine"), "{err}");
}

// ---- 4: a question goes to the commissioner ----------------------------------------------------

#[test]
fn a_question_from_the_foreign_pm_wakes_the_commissioner_and_its_answer_goes_back() {
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let (alpha_pm, _) = alpha_session(&a, &w, "pm");
    let receipt = a.as_session(&alpha_pm, "test/beta/pm", "Plan it.").unwrap();
    let beta_pm = w.worker.of(&b, "pm")[0].internal_session.clone();

    b.reply(
        &beta_pm,
        &receipt.thread_id,
        "Which release do you mean?",
        true,
    );

    let woken = w.worker.of(&a, "pm");
    let wake = woken.last().unwrap();
    assert_eq!(
        wake.internal_session, alpha_pm,
        "the commissioner, nobody else"
    );
    assert!(wake.message.contains("Which release"), "{}", wake.message);
    assert_eq!(a.border_rows()[0].state, BorderState::InputRequired);
    assert_eq!(b.border_rows()[0].state, BorderState::InputRequired);
    // Nothing in beta was woken for it: the question is not beta's owner's to answer — and beta's
    // view does not tell its owner it is waiting on them either.
    assert_eq!(w.worker.of(&b, "pm").len(), 1);
    let in_beta = status_row(&b, &receipt.thread_id);
    assert!(in_beta.escalated && !in_beta.awaiting_human, "{in_beta:?}");
    let beta_ops = b
        .engine
        .status(NOW, nexus_chat::facade::StatusScope::Workspace)
        .unwrap()
        .operations;
    let op = beta_ops
        .iter()
        .find(|op| op.root == receipt.thread_id)
        .expect("the operation is live while the commissioner has the question");
    assert!(!op.needs_decision, "{op:?}");

    // The commissioner answers on the border thread, and the foreign pm is resumed with it.
    a.reply(&alpha_pm, &receipt.thread_id, "The next one, 0.300.", false);
    let beta_triggers = w.worker.of(&b, "pm");
    let resumed = beta_triggers.last().unwrap();
    assert_eq!(beta_triggers.len(), 2, "{beta_triggers:?}");
    assert_eq!(resumed.internal_session, beta_pm);
    assert!(resumed.message.contains("0.300"), "{}", resumed.message);
    assert_eq!(a.border_rows()[0].state, BorderState::Working);
}

// ---- 5: the depth cap holds across both workspaces ---------------------------------------------

#[test]
fn a_back_and_forth_ends_at_the_depth_cap_instead_of_circling() {
    let tmp = TempDir::new().unwrap();
    let open_pm = |ws: &str| {
        format!(
            "handle: pm\nsystem_prompt: You are the PM of {ws}.\naddressable:\n  humans: true\n  external:\n    - \"*/pm\"\n"
        )
    };
    let (alpha_yaml, beta_yaml) = (open_pm("test/alpha"), open_pm("test/beta"));
    let (a, b, w) = two(&tmp, &[&alpha_yaml], &[&beta_yaml], true, true);
    let (mut session, _) = alpha_session(&a, &w, "pm");
    let sides = [(&a, "test/beta/pm"), (&b, "test/alpha/pm")];
    let mut ended = None;
    for hop in 0..64 {
        let (from, to) = sides[hop % 2];
        match from.as_session(&session, to, "And you?") {
            Ok(r)
                if r.border
                    .as_ref()
                    .is_some_and(|b| b.state == BorderState::Rejected) =>
            {
                ended = Some((hop, r.border.unwrap().reason.unwrap()));
                break;
            }
            Ok(_) => {}
            Err(e) => {
                ended = Some((hop, e));
                break;
            }
        }
        let other = if hop % 2 == 0 { &b } else { &a };
        session = w
            .worker
            .of(other, "pm")
            .last()
            .unwrap()
            .internal_session
            .clone();
    }
    let (hop, why) = ended.expect("the chain ended");
    assert!(
        why.contains("depth"),
        "it ended at the depth cap, after {hop} crossings: {why}"
    );
    assert!(
        hop >= 30,
        "the cap is the whole chain's, not a per-side one: {hop}"
    );
}

// ---- 6: only the border thread crosses ---------------------------------------------------------

#[test]
fn only_the_border_thread_crosses_and_what_the_receiver_commissions_behind_it_stays_there() {
    let tmp = TempDir::new().unwrap();
    let helper = "handle: helper\nsystem_prompt: You help.\n";
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM, helper], true, true);
    let (alpha_pm, _) = alpha_session(&a, &w, "pm");
    let receipt = a
        .as_session(&alpha_pm, "test/beta/pm", "Find out.")
        .unwrap();
    let beta_pm = w.worker.of(&b, "pm")[0].internal_session.clone();

    let behind = b
        .as_session(&beta_pm, "helper", "Look it up for me.")
        .expect("the receiver commissions in its own workspace");
    let helper_session = w.worker.of(&b, "helper")[0].internal_session.clone();
    b.reply(&helper_session, &behind.thread_id, "It is 0.300.", false);
    b.reply(&beta_pm, &receipt.thread_id, "Done: 0.300.", false);

    let alpha_threads = a.thread_ids();
    assert!(alpha_threads.contains(&receipt.thread_id));
    assert!(
        !alpha_threads.contains(&behind.thread_id),
        "the thread behind the border is not in the caller's log: {alpha_threads:?}"
    );
    let beta_threads = b.thread_ids();
    let beta_from_alpha: Vec<&String> = beta_threads
        .iter()
        .filter(|t| a.thread_ids().contains(t))
        .collect();
    assert_eq!(
        beta_from_alpha,
        vec![&receipt.thread_id],
        "of the caller's threads only the border thread is in the receiver's log"
    );
}

#[test]
fn a_second_handover_moves_and_wakes_nothing() {
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let (alpha_pm, _) = alpha_session(&a, &w, "pm");
    let receipt = a.as_session(&alpha_pm, "test/beta/pm", "When?").unwrap();
    let beta_pm = w.worker.of(&b, "pm")[0].internal_session.clone();
    b.reply(&beta_pm, &receipt.thread_id, "0.300", false);
    let before = w.worker.all().len();

    for side in [&a, &b, &a] {
        let report = side.engine.handover(machine_caller()).unwrap();
        assert!(report.exchanged.is_empty(), "{report:?}");
        assert!(
            report.woke.is_empty() && report.admitted.is_empty(),
            "{report:?}"
        );
    }
    assert_eq!(w.worker.all().len(), before, "no echo, no second wake");
}

// ---- 8: a workspace that opens nothing behaves as before ---------------------------------------

#[test]
fn without_peers_an_address_is_refused_and_a_bare_handle_is_untouched() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("solo");
    workspace(&root, "test/solo", vec![role(ALPHA_PM)]);
    let worker = Arc::new(Recorder::default());
    let engine = Engine::open_with(
        None,
        &root,
        EngineConfig {
            worker: WorkerConfig::Custom(worker.clone()),
            timer: TimerConfig::Dry,
            ..EngineConfig::default()
        },
    )
    .unwrap();
    let person = || Caller {
        session: None,
        actor: Some("carsten"),
        now: Some(NOW),
    };
    let err = engine
        .send_to(person(), send("test/beta/pm", "Hello"))
        .expect_err("no peers");
    assert!(
        err.msg.contains("reaches no other workspace"),
        "{}",
        err.msg
    );
    let receipt = engine.send_to(person(), send("pm", "Hello")).unwrap();
    assert!(receipt.border.is_none());
    assert_eq!(worker.all().len(), 1);
}

// ---- szc5: the state and the other side, in both workspaces ------------------------------------

/// The status row of `thread` in `side`'s own report.
fn status_row(side: &Side, thread: &str) -> nexus_chat::facade::StatusThread {
    let report = side
        .engine
        .status(NOW, nexus_chat::facade::StatusScope::All(None))
        .expect("status");
    report
        .operations
        .into_iter()
        .flat_map(|op| op.threads)
        .find(|t| t.thread_id == thread)
        .unwrap_or_else(|| panic!("{thread} is in the report"))
}

#[test]
fn the_border_thread_shows_its_state_and_the_other_side_in_both_workspaces() {
    use nexus_chat::border::Direction;
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let (alpha_pm, _) = alpha_session(&a, &w, "pm");
    let receipt = a.as_session(&alpha_pm, "test/beta/pm", "When?").unwrap();

    let out = status_row(&a, &receipt.thread_id)
        .border
        .expect("a border row");
    let inbound = status_row(&b, &receipt.thread_id)
        .border
        .expect("a border row");
    assert_eq!(
        (out.peer.as_str(), out.direction, out.state),
        ("test/beta", Direction::Outbound, BorderState::Working)
    );
    assert_eq!(
        (inbound.peer.as_str(), inbound.direction, inbound.state),
        ("test/alpha", Direction::Inbound, BorderState::Working)
    );
    // In the receiver the border thread is the ROOT of what it does about the commission: the
    // caller's thread above it is not there, and the operation does not hang under a phantom.
    assert_eq!(status_row(&b, &receipt.thread_id).parent, None);

    let beta_pm = w.worker.of(&b, "pm")[0].internal_session.clone();
    b.reply(&beta_pm, &receipt.thread_id, "0.300", false);
    assert_eq!(
        status_row(&a, &receipt.thread_id).border.unwrap().state,
        BorderState::Completed
    );
    assert_eq!(
        status_row(&b, &receipt.thread_id).border.unwrap().state,
        BorderState::Completed
    );
    // A thread that stays in its workspace carries no border.
    let owners = a.thread_ids();
    let local = owners.iter().find(|t| **t != receipt.thread_id).unwrap();
    assert!(status_row(&a, local).border.is_none());
}

// ---- the deadline, and a withdrawal, end it on both sides --------------------------------------

#[test]
fn two_hours_without_a_sign_of_life_cancel_it_and_wake_the_commissioner() {
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let (alpha_pm, _) = alpha_session(&a, &w, "pm");
    let receipt = a.as_session(&alpha_pm, "test/beta/pm", "When?").unwrap();
    let beta_pm = w.worker.of(&b, "pm")[0].internal_session.clone();
    let at = |now: &'static str| Caller {
        session: None,
        actor: Some("handover"),
        now: Some(now),
    };

    // Quiet for an hour is not quiet for two: a persona reading its board is working.
    a.engine.handover(at("2026-10-03T11:00:00Z")).unwrap();
    assert_eq!(a.border_rows()[0].state, BorderState::Working);

    a.engine.handover(at("2026-10-03T12:00:01Z")).unwrap();
    let out = a.border_rows();
    assert_eq!(out[0].state, BorderState::Canceled, "{out:?}");
    assert!(
        out[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("no sign of life"),
        "{out:?}"
    );
    let wake = w.worker.of(&a, "pm").last().unwrap().clone();
    assert_eq!(wake.internal_session, alpha_pm);
    assert!(wake.message.contains("no sign of life"), "{}", wake.message);

    // The receiver was told on the thread and stopped its persona.
    assert_eq!(b.border_rows()[0].state, BorderState::Canceled);
    assert!(w.worker.stopped.lock().unwrap().contains(&beta_pm));
    // …and the commissioner is no longer waiting on it.
    let row = status_row(&a, &receipt.thread_id);
    assert!(row.outstanding.is_empty(), "{row:?}");
}

#[test]
fn withdrawing_the_operation_takes_the_border_thread_back_in_both_workspaces() {
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let (alpha_pm, owners_thread) = alpha_session(&a, &w, "pm");
    a.as_session(&alpha_pm, "test/beta/pm", "When?").unwrap();
    let beta_pm = w.worker.of(&b, "pm")[0].internal_session.clone();

    a.engine
        .withdraw(
            Caller {
                session: None,
                actor: Some("carsten"),
                now: Some(NOW),
            },
            &owners_thread,
        )
        .expect("the owner takes the operation back");

    assert_eq!(a.border_rows()[0].state, BorderState::Canceled);
    assert_eq!(b.border_rows()[0].state, BorderState::Canceled);
    assert!(w.worker.stopped.lock().unwrap().contains(&beta_pm));
}

/// Trust revoked AFTER the commission (nxf 6j6v.dcpd, item 1): the answer arrives signed by a key
/// this workspace no longer vouches for, so it steers nothing — and the commissioner must hear that
/// reason, now, rather than "no sign of life" two hours later.
#[test]
fn an_answer_from_a_workspace_no_longer_trusted_cancels_it_with_that_reason() {
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let (alpha_pm, _) = alpha_session(&a, &w, "pm");
    let receipt = a.as_session(&alpha_pm, "test/beta/pm", "When?").unwrap();
    let beta_pm = w.worker.of(&b, "pm")[0].internal_session.clone();

    // The owner of alpha stops trusting beta while the thread is open.
    {
        let ws = nxs_foundation::workspace::discover(&a.root).unwrap();
        let mut store = ws.open_chat_store().unwrap();
        assert!(store.distrust_key(&key_of(&b.root)).unwrap());
    }
    b.reply(&beta_pm, &receipt.thread_id, "With version 0.300.", false);

    let out = a.border_rows();
    assert_eq!(out[0].state, BorderState::Canceled, "{out:?}");
    assert!(
        out[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("test/beta, a workspace no longer trusted here"),
        "{out:?}"
    );
    let wake = w.worker.of(&a, "pm").last().unwrap().clone();
    assert_eq!(wake.internal_session, alpha_pm);
    assert!(
        wake.message.contains("no longer trusted"),
        "{}",
        wake.message
    );
    assert!(
        !wake.message.contains("0.300"),
        "the unvouched answer itself is not handed on: {}",
        wake.message
    );
    // The commissioner is no longer waiting on it.
    let row = status_row(&a, &receipt.thread_id);
    assert!(row.outstanding.is_empty(), "{row:?}");
}

// ---- an answer for a commissioner still in its turn ------------------------------------------

#[test]
fn an_answer_held_for_a_commissioner_still_in_its_turn_keeps_it_waiting_rather_than_owing() {
    // The race the live run met: the other workspace's pm answers within seconds, while the
    // commissioner is still ending its turn. The answer is HELD for it — and until it is handed
    // over, the commissioner's own row says it is waiting on its round, so its sidecar does not
    // remind it into a substituted reply.
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let (alpha_pm, owners_thread) = alpha_session(&a, &w, "pm");
    let receipt = a.as_session(&alpha_pm, "test/beta/pm", "When?").unwrap();
    let beta_pm = w.worker.of(&b, "pm")[0].internal_session.clone();

    w.worker.busy.lock().unwrap().push(alpha_pm.clone());
    b.reply(&beta_pm, &receipt.thread_id, "0.300", false);

    let owners = status_row(&a, &owners_thread);
    assert_eq!(
        owners.waiting_on_sub_round,
        vec![receipt.thread_id.clone()],
        "the commissioner is waiting for the answer held for it: {owners:?}"
    );
    assert_eq!(
        w.worker.of(&a, "pm").len(),
        1,
        "nothing was resumed while it was busy"
    );
}

// ---- review of PR #14: the paths a live run met and the gaps it left -------------------------

#[test]
fn the_held_answer_reaches_the_commissioner_once_its_turn_has_ended() {
    // The end of the race `an_answer_held_…` starts: the commissioner's turn ends, the held answer
    // is handed over — once, with the answer in it.
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let (alpha_pm, _) = alpha_session(&a, &w, "pm");
    let receipt = a.as_session(&alpha_pm, "test/beta/pm", "When?").unwrap();
    let beta_pm = w.worker.of(&b, "pm")[0].internal_session.clone();
    w.worker.busy.lock().unwrap().push(alpha_pm.clone());
    b.reply(&beta_pm, &receipt.thread_id, "0.300", false);
    assert_eq!(w.worker.of(&a, "pm").len(), 1, "held, not woken");

    w.worker.busy.lock().unwrap().clear();
    a.engine
        .deliver_held(machine_caller(), &alpha_pm)
        .expect("the delivery runs");
    a.engine.handover(machine_caller()).unwrap();
    let woken = w.worker.of(&a, "pm");
    assert_eq!(woken.len(), 2, "woken exactly once: {woken:?}");
    assert_eq!(woken[1].internal_session, alpha_pm);
    assert!(woken[1].message.contains("0.300"), "{}", woken[1].message);
}

#[test]
fn a_sign_of_life_in_the_receivers_transcript_restarts_the_deadline() {
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let (alpha_pm, _) = alpha_session(&a, &w, "pm");
    a.as_session(&alpha_pm, "test/beta/pm", "When?").unwrap();
    let beta_pm = w.worker.of(&b, "pm")[0].internal_session.clone();
    // The persona is reading its board at 11:30 — working, though it has said nothing yet.
    b.engine
        .transcript_append(
            &beta_pm,
            &[nexus_chat::transcript::TranscriptEntry {
                kind: "tool_use".into(),
                at: Some("2026-10-03T11:30:00Z".into()),
                tool_use_id: None,
                parent_tool_use_id: None,
                subagent_type: None,
                data: serde_json::json!({"name": "Bash"}),
            }],
        )
        .unwrap();
    let at = |now: &'static str| Caller {
        session: None,
        actor: Some("handover"),
        now: Some(now),
    };
    a.engine.handover(at("2026-10-03T12:00:01Z")).unwrap();
    assert_eq!(
        a.border_rows()[0].state,
        BorderState::Working,
        "two hours from the start, but half an hour from the last sign of life"
    );
    a.engine.handover(at("2026-10-03T13:30:01Z")).unwrap();
    assert_eq!(a.border_rows()[0].state, BorderState::Canceled);
}

#[test]
fn a_commission_whose_first_kick_failed_is_handed_over_by_the_next_handover() {
    // The other side's handover did not run (its database was busy). Nothing new is pushed the next
    // time — the ops are already there — so the kick has to come from the other side not holding
    // the thread yet, or the commission would be stranded.
    let tmp = TempDir::new().unwrap();
    let (a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let (alpha_pm, _) = alpha_session(&a, &w, "pm");
    // Two: the send's own kick, and the one its handover tries right after it.
    w.failing_kicks
        .store(2, std::sync::atomic::Ordering::SeqCst);
    let receipt = a.as_session(&alpha_pm, "test/beta/pm", "When?").unwrap();
    assert_eq!(receipt.border.unwrap().state, BorderState::Submitted);
    assert!(w.worker.of(&b, "pm").is_empty(), "the kick failed");

    a.engine.handover(machine_caller()).unwrap();
    assert_eq!(
        w.worker.of(&b, "pm").len(),
        1,
        "the next handover kicked again"
    );
    assert_eq!(a.border_rows()[0].state, BorderState::Working);
}

#[test]
fn a_border_stamp_on_a_thread_of_this_workspace_makes_no_border_thread() {
    // A message in a local thread that merely CARRIES a stamp — written here, by anybody who can
    // set refs — is not a commission from outside: no record, no persona, no refusal posted.
    let tmp = TempDir::new().unwrap();
    let (_a, b, w) = two(&tmp, &[ALPHA_PM], &[BETA_PM], true, true);
    let forged = nexus_chat::model::Refs {
        border: Some(nexus_chat::border::BorderRef {
            from: "test/alpha".into(),
            to: "test/beta/pm".into(),
            role: Some("pm".into()),
            hop: 0,
            refusal: None,
            canceled: false,
        }),
        ..Default::default()
    };
    b.engine
        .send_to(
            Caller {
                session: None,
                actor: Some("carsten"),
                now: Some(NOW),
            },
            SendToRequest {
                to: "pm",
                body: "pretend I came from alpha",
                refs: SendToRefs::Declared(forged),
                machine: None,
            },
        )
        .unwrap();
    let before = w.worker.all().len();
    b.engine.handover(machine_caller()).unwrap();
    assert!(b.border_rows().is_empty(), "{:?}", b.border_rows());
    assert_eq!(w.worker.all().len(), before);
}
