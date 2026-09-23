//! ACCEPTANCE of nxf 6j6v.s9ex — **`nxc status` names unresumed parked work**.
//!
//! Every trouble hand-off of this device's working copy parks the holder's uncommitted work onto a
//! branch (`crates/chat/src/park.rs`) — but until this item the row it recorded was invisible from
//! `nxc status`: a reader had to already know an operation had been parked (and under which key) to
//! find `ChatStore::parked_work` at all. This item puts the fact on the view itself, on the
//! operation the row belongs to.
//!
//! **On reproducing a real park.** `an_unanswered_escalation_parks_and_comes_back.rs` drives the
//! real park path end to end — a real git repository, two personas racing for the working copy, a
//! tick past the thirty-minute contention deadline — and that coverage is not duplicated here. This
//! file is about the STATUS VIEW's rendering of a parked row, not about how a row comes to exist, so
//! [`record_park`] writes the row directly with [`nexus_chat::store::ChatStore::record_parked_work`],
//! under exactly the key the real park path uses — see that function's own doc for the exact chain
//! of calls that proves it.

mod common;

use std::sync::{Arc, Mutex};

use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::facade::StatusScope;
use nexus_chat::orchestration::Caller;
use nexus_chat::park::Parked;
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
use nexus_chat::working_tree::WorkScope;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-09-17T10:00:00Z";
const PARKED_AT: &str = "2026-09-17T09:30:00Z";

/// A worker that only records — nothing here exercises a real session, so nothing needs to answer
/// for one.
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

fn role(handle: &str) -> RoleDecl {
    serde_yaml::from_str(&format!(
        "handle: {handle}\nsystem_prompt: You are {handle}.\n"
    ))
    .expect("test role parses")
}

/// A workspace with one persona, `coder`, and an `Engine` open on it.
fn workspace(worker: Arc<dyn Worker>) -> (TempDir, Engine) {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let defs = Definitions::new(vec![role("coder")], vec![]).expect("catalogue");
    common::write_declarations(tmp.path(), &defs);
    let engine = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(worker),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    (tmp, engine)
}

fn store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

fn human() -> Caller<'static> {
    Caller {
        session: None,
        actor: Some("carsten"),
        now: Some(NOW),
    }
}

/// Commission `coder`, returning the root thread id — the operation the parked row below is
/// attributed to.
fn commission(engine: &Engine) -> String {
    engine
        .send_to(
            human(),
            SendToRequest {
                machine: None,
                to: "coder",
                body: "build the thing",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("commission")
        .thread_id
}

/// A [`Parked`] value with fields a test can tell apart from any other — not the product of a real
/// `commit_onto_a_branch`, just data of the same shape.
fn a_park() -> Parked {
    Parked {
        branch: "nxs/park/th-example/2026-09-17T09-00-00Z".to_string(),
        commit: "abc1234def5678901234567890123456789012".to_string(),
        base_branch: "main".to_string(),
        base_commit: "0ff1ce00000000000000000000000000000000".to_string(),
        created_branch: true,
        committed: true,
    }
}

/// Record a park for `thread`, under exactly the key the real park path would use.
///
/// `orchestration::park_and_hand_on` (called by both `park_the_stranded_holder` and
/// `secure_the_interrupted_tree`) does `store.record_parked_work(holder, &parked, ctx.now)`, where
/// `holder` is read off the working-tree lease row — the string `WorkScope::key()` produced when
/// the lease was acquired. For the one scope that can ever hold this workspace's working copy that
/// is `WorkScope::Thread(<id>).key()` == `"thread:<id>"`, which is what this writes.
fn record_park(tmp: &TempDir, thread: &str, parked: &Parked, at: &str) -> i64 {
    let mut store = store(tmp);
    let key = WorkScope::Thread(thread.to_string()).key();
    store
        .record_parked_work(&key, parked, at)
        .expect("record parked work")
}

#[test]
fn an_operation_with_parked_work_lists_it_on_the_status_view() {
    let worker = Arc::new(RecordingWorker::default());
    let (tmp, engine) = workspace(worker);
    let thread = commission(&engine);
    let parked = a_park();
    record_park(&tmp, &thread, &parked, PARKED_AT);

    let ids = [thread.as_str()];
    let report = engine
        .status(NOW, StatusScope::Threads(&ids))
        .expect("status reads");
    let op = &report.operations[0];
    assert_eq!(op.parked.len(), 1, "{:?}", op.parked);
    assert_eq!(op.parked[0].parked.branch, parked.branch);
    assert_eq!(op.parked[0].parked.commit, parked.commit);
    assert_eq!(op.parked[0].parked.committed, parked.committed);
    assert_eq!(op.parked[0].parked_at, PARKED_AT);
}

/// **Parked work keeps its operation on the DEFAULT listing** (nxf 6j6v.8bv9, requirement 7).
///
/// An operation whose threads are all discharged — a withdrawn round, a swept holder, or, as here,
/// one that simply answered — has no other live signal left, and its work sits on a branch nobody
/// has come back for. Without this rule it would drop out of `nxc status` at exactly the moment the
/// branch is the one thing a reader still has to act on.
#[test]
fn a_parked_operation_with_nothing_else_open_is_still_listed_by_default() {
    let worker = Arc::new(RecordingWorker::default());
    let (tmp, engine) = workspace(worker.clone());
    let thread = commission(&engine);
    let session = worker.seen.lock().unwrap()[0].internal_session.clone();
    engine
        .reply_thread(
            Caller {
                session: Some(&session),
                actor: Some("coder"),
                now: Some(NOW),
            },
            nexus_chat::surface::ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "done",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the coder answers");
    assert!(
        !engine
            .status(NOW, StatusScope::Workspace)
            .expect("status reads")
            .operations
            .iter()
            .any(|op| op.root == thread),
        "the premise: an answered operation with nothing parked is not on the default listing"
    );

    record_park(&tmp, &thread, &a_park(), PARKED_AT);

    let report = engine
        .status(NOW, StatusScope::Workspace)
        .expect("status reads");
    let op = report
        .operations
        .iter()
        .find(|op| op.root == thread)
        .expect("an operation with unresumed parked work stays on the default listing");
    assert!(op.live, "parked work counts as live");
    assert_eq!(op.open, 0, "and nothing else about it is");
    assert!(!op.needs_decision && !op.holds_working_tree && !op.interrupted);
    assert_eq!(op.parked.len(), 1);
}

#[test]
fn resumed_parked_work_is_not_listed() {
    let worker = Arc::new(RecordingWorker::default());
    let (tmp, engine) = workspace(worker);
    let thread = commission(&engine);
    let parked = a_park();
    let id = record_park(&tmp, &thread, &parked, PARKED_AT);
    {
        let mut s = store(&tmp);
        s.mark_parked_work_resumed(id, NOW)
            .expect("mark parked work resumed");
    }

    let ids = [thread.as_str()];
    let report = engine
        .status(NOW, StatusScope::Threads(&ids))
        .expect("status reads");
    assert!(
        report.operations[0].parked.is_empty(),
        "{:?}",
        report.operations[0].parked
    );
}

#[test]
fn a_workspace_that_never_parked_serialises_no_parked_key() {
    let worker = Arc::new(RecordingWorker::default());
    let (_tmp, engine) = workspace(worker);
    let thread = commission(&engine);

    let ids = [thread.as_str()];
    let report = engine
        .status(NOW, StatusScope::Threads(&ids))
        .expect("status reads");
    let value = report.to_value();
    let op = &value["operations"][0];
    assert!(
        op.as_object()
            .expect("operation is an object")
            .get("parked")
            .is_none(),
        "an operation with nothing parked must not carry a \"parked\" key at all: {op}"
    );
}

// ---- the CLI half: `nxc status --json` and `nxc status`, once each ----------------------------

mod cli {
    use assert_cmd::Command;
    use serde_json::Value;
    use tempfile::TempDir;

    use nexus_chat::store::ChatStore;
    use nexus_chat::working_tree::WorkScope;
    use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};

    use super::{a_park, PARKED_AT};

    fn workspace() -> TempDir {
        let tmp = TempDir::new().unwrap();
        setup(tmp.path(), &chat_config()).expect("seed chat workspace");
        let roles = tmp.path().join(".nxs-personas");
        std::fs::create_dir_all(&roles).unwrap();
        std::fs::write(
            roles.join("coder.yaml"),
            "handle: coder\nsystem_prompt: You are coder.\n",
        )
        .unwrap();
        tmp
    }

    fn nxc(tmp: &TempDir, now: &str) -> Command {
        let mut c = nxs_test_support::cargo_bin("nxc");
        c.current_dir(tmp.path())
            .env_remove("NXC_SESSION")
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", "local")
            .env("NXC_NOW", now)
            .env("NXC_WORKER", "dry")
            .env("NXC_TIMER", "dry")
            .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
        c
    }

    fn json_of(cmd: &mut Command) -> Value {
        let out = cmd.assert().success();
        serde_json::from_slice(&out.get_output().stdout).expect("valid json")
    }

    fn open_store(tmp: &TempDir) -> ChatStore {
        Workspace::resolve(None, tmp.path())
            .expect("resolve workspace")
            .open_chat_store()
            .expect("open chat store")
    }

    #[test]
    fn the_human_status_line_names_the_branch_and_short_commit() {
        let tmp = workspace();
        let thread = json_of(nxc(&tmp, super::NOW).args([
            "--json",
            "send",
            "--to",
            "coder",
            "--no-ref",
            "build the thing",
        ]))["thread_id"]
            .as_str()
            .unwrap()
            .to_string();

        let parked = a_park();
        let key = WorkScope::Thread(thread.clone()).key();
        open_store(&tmp)
            .record_parked_work(&key, &parked, PARKED_AT)
            .expect("record parked work");

        let out = nxc(&tmp, super::NOW)
            .args(["status", "--thread", &thread])
            .assert()
            .success();
        let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
        let short_commit = &parked.commit[..7];
        // The WHOLE line the fragment promises: branch, seven characters of the commit, and since
        // when the copy has been handed on. `since` was unasserted (independent review of PR #478,
        // Test Quality #4) although it is a third of what this item added.
        assert!(
            stdout.contains(&format!(
                "parked on {} at {short_commit} since {PARKED_AT}",
                parked.branch
            )),
            "{stdout}"
        );
        assert!(
            !stdout.contains("nothing was uncommitted"),
            "this park committed work, so the note for the other case must not be here: {stdout}"
        );

        // `--json` carries the same field.
        let report = json_of(nxc(&tmp, super::NOW).args(["--json", "status", "--thread", &thread]));
        let json_parked = &report["operations"][0]["parked"][0];
        assert_eq!(json_parked["branch"], parked.branch);
        assert_eq!(json_parked["commit"], parked.commit);
        assert_eq!(
            json_parked["parked_at"], PARKED_AT,
            "the instant the human line reads `since` from: {report}"
        );
        assert_eq!(json_parked["committed"], true, "{report}");
    }

    /// **A park that found nothing to commit says so, in the line's own words** (independent review
    /// of PR #478, Test Quality #4).
    ///
    /// The other branch of `cli::parked_line`, and the other half of the fragment's "plus a note
    /// when the park found nothing to commit". It is the shape `nxf 6j6v.7hqc` made ordinary: a
    /// clean tree standing on the operation's own base branch parks onto that branch, creates
    /// nothing and commits nothing — so "parked" alone would read as "there was uncommitted work"
    /// on precisely the row where there was none.
    #[test]
    fn the_human_status_line_says_when_the_park_found_nothing_to_commit() {
        let tmp = workspace();
        let thread = json_of(nxc(&tmp, super::NOW).args([
            "--json",
            "send",
            "--to",
            "coder",
            "--no-ref",
            "build the thing",
        ]))["thread_id"]
            .as_str()
            .unwrap()
            .to_string();

        // What `commit_onto_a_branch` returns for a clean tree on the recorded base branch: the
        // base branch itself, no branch created, nothing committed.
        let parked = nexus_chat::park::Parked {
            branch: "main".to_string(),
            created_branch: false,
            committed: false,
            ..a_park()
        };
        let key = WorkScope::Thread(thread.clone()).key();
        open_store(&tmp)
            .record_parked_work(&key, &parked, PARKED_AT)
            .expect("record parked work");

        let out = nxc(&tmp, super::NOW)
            .args(["status", "--thread", &thread])
            .assert()
            .success();
        let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
        let short_commit = &parked.commit[..7];
        assert!(
            stdout.contains(&format!(
                "parked on main at {short_commit} since {PARKED_AT}, nothing was uncommitted"
            )),
            "{stdout}"
        );

        // …and `--json` carries the fact the note is rendered from, rather than the note.
        let report = json_of(nxc(&tmp, super::NOW).args(["--json", "status", "--thread", &thread]));
        let json_parked = &report["operations"][0]["parked"][0];
        assert_eq!(json_parked["committed"], false, "{report}");
        assert_eq!(json_parked["created_branch"], false, "{report}");
        assert_eq!(json_parked["branch"], "main", "{report}");
    }
}
