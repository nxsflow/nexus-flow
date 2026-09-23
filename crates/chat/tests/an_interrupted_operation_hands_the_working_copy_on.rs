//! **A week-long hold does not hold this workspace's working copy for a week** (nxf 6j6v.npy3,
//! acceptance point 7).
//!
//! The owner's decision of 2026-09-14, in one sentence: an operation that stopped at an availability
//! boundary has its uncommitted work **committed onto a park branch** and the checkout handed on,
//! rather than kept until the window lifts. A quota window is up to a week; a workspace with one
//! working copy and one operation sitting on it for a week is a workspace where nothing else can
//! happen.
//!
//! **Everything here is real.** A real git repository, the real lease, the real park, and the real
//! verbs. The only stand-in is the WORKER, and only for what a worker does — it starts no operating
//! system session and answers [`Worker::working_copy`] with the repository, which is the one fact
//! the park needs from it. Modelled on `an_unanswered_escalation_parks_and_comes_back.rs`, which is
//! the acceptance run of the OTHER occasion for a park; the mechanism below it is now literally the
//! same function (`park_and_hand_on`), and these two files are what keep both of its callers honest.
//!
//! ## Why the securing happens on the RESUME and not on the announcement
//!
//! The announcement is made BY the stopping session, whose process is necessarily still alive while
//! it speaks — so a park attempted there would commit a tree underneath a process still standing in
//! it. What the announcement does is ARM a job for the process grace; what that job runs is the
//! resume, which finds the window still closed, secures the tree, and re-arms itself for the instant
//! the runtime named. That is one job kind doing both halves, and it is `nxc tick`'s shape.

use std::path::{Path, PathBuf};

use nexus_chat::orchestration::{
    self, Ctx, ResumeOutcome, ResumeRequest, ResumeScope, SessionScope,
};
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{self, SendToRefs, SendToRequest};
use nexus_chat::worker::{DryWorker, TriggerRequest, TriggerResult, Worker};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

/// The coder is commissioned here and takes the working copy.
const T0: &str = "2026-09-12T09:00:00Z";
/// …and runs into the weekly window here.
const INTERRUPTED: &str = "2026-09-12T09:53:52Z";
/// The near firing: the process grace has passed, the window has not.
const GRACE: &str = "2026-09-12T09:54:02Z";
/// When the runtime says the window lifts — days away, which is the whole point.
const UNTIL: &str = "2026-09-19T12:00:00Z";

/// A worker that records like [`DryWorker`] and, unlike it, knows where the sessions would run —
/// the one fact the park asks a worker for. It also continues its own conversations, which is what
/// the shipped sidecar answers and what keeps the resume from refusing before it gets here.
struct ParkingWorker {
    log: PathBuf,
    root: PathBuf,
    /// Whether this runtime continues a conversation it began. `true` is the shipped sidecar's
    /// answer and what every case here runs on but one — the one that checks what a host whose
    /// runtime CANNOT is told.
    resumes: bool,
}

impl Worker for ParkingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        DryWorker {
            log: Some(self.log.clone()),
        }
        .trigger(req)
    }
    fn working_copy(&self) -> Option<PathBuf> {
        Some(self.root.clone())
    }
    fn resumes_sessions(&self) -> bool {
        self.resumes
    }
}

/// Every git call here runs with a **pinned `$HOME`**, for
/// `an_unanswered_escalation_parks_and_comes_back.rs`'s two reasons: the source gate that flags an
/// unpinned `init`, and determinism — unpinned, these commands read the developer's own
/// `~/.gitconfig`, so an `init.defaultBranch` or a `commit.gpgsign` decides what this test measures.
fn git(root: &Path, args: &[&str]) -> String {
    use nxs_test_support::PinHome;
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .pin_home(nxs_test_support::pinned_home())
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(root: &Path, message: &str) {
    git(
        root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "commit",
            "-q",
            "-m",
            message,
        ],
    );
}

/// A chat workspace that is ALSO a git repository, with one persona that wants the copy to itself.
///
/// It starts on `release/1.2` rather than `main`, for the sibling file's reason: a fixture that
/// starts on `main` cannot tell a correct implementation from one that hardcoded the name.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("coder.yaml"),
        "handle: coder\njob_title: Implementer\nsystem_prompt: You are coder.\n\
         working_tree: exclusive\n",
    )
    .unwrap();
    git(tmp.path(), &["init", "-q"]);
    git(tmp.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    std::fs::write(tmp.path().join(".gitignore"), ".nxs/\ndry.log\n").unwrap();
    std::fs::write(tmp.path().join("src.txt"), "as it was\n").unwrap();
    git(tmp.path(), &["add", "-A"]);
    commit(tmp.path(), "first");
    git(tmp.path(), &["checkout", "-q", "-b", "release/1.2"]);
    tmp
}

trait Workspaced {
    fn root(&self) -> &Path;

    fn store(&self) -> ChatStore {
        Workspace::resolve(None, self.root())
            .expect("resolve workspace")
            .open_chat_store()
            .expect("open chat store")
    }

    /// Run `f` with a fully built [`Ctx`] and an open store, then drop both so the next call opens
    /// the database file cleanly.
    fn with<T>(&self, now: &str, actor: &str, f: impl FnOnce(&Ctx, &mut ChatStore) -> T) -> T {
        self.with_worker(now, actor, true, f)
    }

    fn with_worker<T>(
        &self,
        now: &str,
        actor: &str,
        resumes: bool,
        f: impl FnOnce(&Ctx, &mut ChatStore) -> T,
    ) -> T {
        let ws = Workspace::resolve(None, self.root()).expect("resolve workspace");
        let db_path = ws.db_path_str().expect("db path");
        let mut store = ws.open_chat_store().expect("open chat store");
        let defs =
            nexus_chat::definitions::Definitions::resolve(self.root()).expect("declarations");
        let worker = ParkingWorker {
            log: self.root().join("dry.log"),
            root: self.root().to_path_buf(),
            resumes,
        };
        let ctx = Ctx {
            now,
            origin: "local",
            actor,
            session: None,
            hop: 0,
            defs: &defs,
            worker: &worker,
            timer: &nexus_chat::timer::DisabledTimer,
            namer: &nexus_chat::naming::DryNamer,
            db_path: &db_path,
            project_claude_md: None,
            module_primes: None,
            machines: None,
        };
        f(&ctx, &mut store)
    }
}

impl Workspaced for TempDir {
    fn root(&self) -> &Path {
        self.path()
    }
}

#[test]
fn the_work_is_committed_and_the_checkout_handed_on_while_the_window_is_closed() {
    let tmp = workspace();

    // The coder is commissioned and takes the working copy.
    let receipt = tmp.with(T0, "carsten", |ctx, store| {
        surface::send_to(
            ctx,
            store,
            SendToRequest {
                machine: None,
                to: "coder",
                body: "rework the parser",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the coder is commissioned")
    });
    let thread = receipt.thread_id.clone();
    let session = receipt.session.clone().expect("a session was minted");
    assert!(
        tmp.store()
            .working_tree_holder(T0)
            .expect("read the lease")
            .is_some(),
        "an exclusive persona takes the working copy"
    );

    // It works — and what it has written is NOT committed. That is the normal state of a turn: an
    // agent hands over mid-work, which is exactly why nxf 6j6v.2af2 records a dirty tree honestly
    // rather than pretending a commit describes it.
    std::fs::write(tmp.path().join("src.txt"), "half rewritten\n").unwrap();
    std::fs::write(
        tmp.path().join("new.txt"),
        "and a file git has never seen\n",
    )
    .unwrap();

    // Then the weekly window closes on it.
    tmp.with(INTERRUPTED, "coder", |ctx, store| {
        orchestration::session_interrupted(
            ctx,
            store,
            &session,
            "seven_day",
            Some(UNTIL),
            "You've hit your weekly limit",
        )
        .expect("the hold is recorded");
        orchestration::session_ended(ctx, store, &session).expect("and the process is over");
    });

    // The near firing, ten seconds later: the window is days away, so nothing is started — and the
    // working copy is secured and handed on rather than held until it lifts.
    let out = tmp.with(GRACE, "carsten", |ctx, store| {
        orchestration::resume_interrupted(
            ctx,
            store,
            ResumeRequest {
                scope: ResumeScope::Session(&session),
                force: false,
            },
        )
        .expect("the resume decides something")
    });
    assert_eq!(
        out.outcome,
        ResumeOutcome::NotYet {
            until: Some(UNTIL.to_string())
        }
    );
    assert!(
        out.warnings.is_empty(),
        "securing the tree must not fail quietly: {:?}",
        out.warnings
    );

    // **The work is safe.** On a branch of the operation's own, with BOTH the tracked change and
    // the file git had never seen — an untracked file is what a coding agent produces constantly,
    // and a park that left them behind would lose exactly the new work.
    let parked = tmp
        .store()
        .parked_work(&format!("thread:{thread}"))
        .expect("read the parked work");
    assert_eq!(parked.len(), 1, "{parked:?}");
    let row = &parked[0];
    assert!(
        row.parked.branch.starts_with("nxs/park/"),
        "{}",
        row.parked.branch
    );
    assert!(row.parked.committed, "there WAS something to save");
    let files = git(
        tmp.path(),
        &["show", "--name-only", "--format=", &row.parked.commit],
    );
    assert!(files.contains("src.txt"), "{files}");
    assert!(files.contains("new.txt"), "the untracked file too: {files}");

    // **And the checkout is free and back where it was**, so the next operation starts in its own
    // files rather than in this one's.
    assert_eq!(
        git(tmp.path(), &["symbolic-ref", "--quiet", "--short", "HEAD"]),
        "release/1.2",
        "the tree is back on the operation's base, not left standing on the park branch"
    );
    assert_eq!(git(tmp.path(), &["status", "--porcelain"]), "", "and clean");
    assert!(
        tmp.store()
            .working_tree_holder(GRACE)
            .expect("read the lease")
            .is_none(),
        "the working copy is handed on rather than held for a week"
    );

    // The hold STANDS — nothing was taken up — so the operation keeps saying why nobody is working
    // in it.
    let state = tmp.with(GRACE, "carsten", |ctx, store| {
        orchestration::session_state(store, ctx.worker, SessionScope::Session(&session))
            .expect("readable")
    });
    assert_eq!(
        state.sessions[0]
            .interrupted
            .as_ref()
            .map(|h| h.limit.as_str()),
        Some("seven_day")
    );
}

#[test]
fn a_second_near_firing_does_not_mint_a_second_empty_park() {
    // The job can fire more than once, and a person can run the verb as often as they like. What
    // must not happen is a new, EMPTY park branch each time: the first firing hands the lease on,
    // and every one after it finds nothing of this operation's in the checkout and does nothing.
    let tmp = workspace();
    let receipt = tmp.with(T0, "carsten", |ctx, store| {
        surface::send_to(
            ctx,
            store,
            SendToRequest {
                machine: None,
                to: "coder",
                body: "rework the parser",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("commissioned")
    });
    let thread = receipt.thread_id.clone();
    let session = receipt.session.clone().expect("a session");
    std::fs::write(tmp.path().join("src.txt"), "half rewritten\n").unwrap();
    tmp.with(INTERRUPTED, "coder", |ctx, store| {
        orchestration::session_interrupted(ctx, store, &session, "seven_day", Some(UNTIL), "quota")
            .expect("recorded");
        orchestration::session_ended(ctx, store, &session).expect("ended");
    });

    for _ in 0..3 {
        let out = tmp.with(GRACE, "carsten", |ctx, store| {
            orchestration::resume_interrupted(
                ctx,
                store,
                ResumeRequest {
                    scope: ResumeScope::Session(&session),
                    force: false,
                },
            )
            .expect("decides")
        });
        assert!(matches!(out.outcome, ResumeOutcome::NotYet { .. }));
    }

    let parked = tmp
        .store()
        .parked_work(&format!("thread:{thread}"))
        .expect("read");
    assert_eq!(
        parked.len(),
        1,
        "one interruption, one park — not one per firing: {parked:?}"
    );
}

#[test]
fn resuming_brings_the_operation_back_to_its_own_files() {
    // The other half of the promise. The park is only defensible because the return is automatic:
    // `trigger_role` restores it at the lease gate, which is the one place both ways back converge,
    // and the session is told where its work is in a deterministic text.
    let tmp = workspace();
    let receipt = tmp.with(T0, "carsten", |ctx, store| {
        surface::send_to(
            ctx,
            store,
            SendToRequest {
                machine: None,
                to: "coder",
                body: "rework the parser",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("commissioned")
    });
    let session = receipt.session.clone().expect("a session");
    std::fs::write(tmp.path().join("src.txt"), "half rewritten\n").unwrap();
    tmp.with(INTERRUPTED, "coder", |ctx, store| {
        orchestration::session_interrupted(ctx, store, &session, "seven_day", Some(UNTIL), "quota")
            .expect("recorded");
        orchestration::session_ended(ctx, store, &session).expect("ended");
    });
    tmp.with(GRACE, "carsten", |ctx, store| {
        orchestration::resume_interrupted(
            ctx,
            store,
            ResumeRequest {
                scope: ResumeScope::Session(&session),
                force: false,
            },
        )
        .expect("secured")
    });

    // The window lifts, and the way back runs.
    let out = tmp.with("2026-09-19T12:00:01Z", "carsten", |ctx, store| {
        orchestration::resume_interrupted(
            ctx,
            store,
            ResumeRequest {
                scope: ResumeScope::Session(&session),
                force: false,
            },
        )
        .expect("decides")
    });
    assert_eq!(out.outcome, ResumeOutcome::Resumed, "{out:?}");

    // The operation is standing in its own files again.
    let branch = git(tmp.path(), &["symbolic-ref", "--quiet", "--short", "HEAD"]);
    assert!(
        branch.starts_with("nxs/park/"),
        "the resume checks the operation's parked branch back out, got {branch}"
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("src.txt")).unwrap(),
        "half rewritten\n",
        "…with the work that was secured"
    );

    // And the session was told where it is, in the deterministic text the park composes.
    let log = std::fs::read_to_string(tmp.path().join("dry.log")).unwrap_or_default();
    assert!(
        log.contains("nxs/park/"),
        "the resumed session is told its branch name: {log}"
    );
}

// ---- the tree moved, and the question that goes back for it ----------------------------------
//
// `ResumeOutcome::TreeMoved` had NO test anywhere when this file was first written (independent
// review of PR #476, Test Quality #1) — the one branch the item's definition of done names by name
// as needing coverage. The second case below is the one that would have caught Code Quality #2
// directly: the drift check was reading a WORKSPACE-GLOBAL parked-work flag, so an unrelated park
// anywhere switched it off for everybody.

/// Commission the coder, let it write, and interrupt it — with the window ALREADY PAST, so a resume
/// goes straight for the drift check instead of stopping at `NotYet` and parking.
fn interrupted_with_the_window_already_past(tmp: &TempDir) -> (String, String) {
    let receipt = tmp.with(T0, "carsten", |ctx, store| {
        surface::send_to(
            ctx,
            store,
            SendToRequest {
                machine: None,
                to: "coder",
                body: "rework the parser",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("commissioned")
    });
    let session = receipt.session.clone().expect("a session");
    tmp.with(INTERRUPTED, "coder", |ctx, store| {
        // A five-hour window that lifted while nobody was looking — so the resume below is past it.
        orchestration::session_interrupted(
            ctx,
            store,
            &session,
            "five_hour",
            Some("2026-09-12T10:00:00Z"),
            "You've hit your 5-hour limit",
        )
        .expect("recorded");
        orchestration::session_ended(ctx, store, &session).expect("ended");
    });
    (session, receipt.thread_id.clone())
}

/// Somebody else moves the checkout on while the operation waits.
fn the_tree_moves_on(tmp: &TempDir) {
    std::fs::write(tmp.path().join("src.txt"), "somebody else's work\n").unwrap();
    git(tmp.path(), &["add", "-A"]);
    commit(tmp.path(), "another operation came through here");
}

const AFTER_THE_WINDOW: &str = "2026-09-12T11:00:00Z";

#[test]
fn a_tree_that_moved_under_the_hold_sends_a_question_back_instead_of_resuming_blind() {
    let tmp = workspace();
    let (session, thread) = interrupted_with_the_window_already_past(&tmp);
    the_tree_moves_on(&tmp);

    let out = tmp.with(AFTER_THE_WINDOW, "carsten", |ctx, store| {
        orchestration::resume_interrupted(
            ctx,
            store,
            ResumeRequest {
                scope: ResumeScope::Session(&session),
                force: false,
            },
        )
        .expect("decides")
    });

    let ResumeOutcome::TreeMoved { drift } = &out.outcome else {
        panic!("expected TreeMoved, got {:?}", out.outcome);
    };
    assert!(
        drift.commit_now.is_some(),
        "HEAD moved, and the report says which commit it is at now: {drift:?}"
    );

    // **Nothing was started.** The dry log records every trigger; the commission is the only one.
    let log = std::fs::read_to_string(tmp.path().join("dry.log")).unwrap_or_default();
    assert_eq!(
        log.matches("trigger role=coder").count(),
        1,
        "the session must NOT have been resumed into a tree that moved:\n{log}"
    );

    // **The hold STANDS**, because nobody took the operation up — it is waiting for a person now,
    // and an operation that looked taken-up would stop saying why nothing is happening in it.
    let state = tmp.with(AFTER_THE_WINDOW, "carsten", |ctx, store| {
        orchestration::session_state(store, ctx.worker, SessionScope::Session(&session))
            .expect("readable")
    });
    assert!(
        state.sessions[0].interrupted.is_some(),
        "the hold must still stand: nothing has taken this operation up"
    );

    // **And the question is on the thread that STARTED the operation**, which is the root — the
    // same emergency exit nxf 6j6v.am8j takes, rather than a second mechanism beside it.
    let root = tmp.store().thread_root(&thread).expect("a root");
    let bodies: Vec<String> = tmp
        .store()
        .messages_in_thread(&root)
        .expect("read the root thread")
        .into_iter()
        .map(|m| m.body)
        .collect();
    let question = bodies
        .iter()
        .find(|b| b.contains("working copy has moved"))
        .unwrap_or_else(|| panic!("no question reached the root thread; bodies: {bodies:?}"));
    assert!(
        question.contains("nothing has been started"),
        "it has to say that, or a reader assumes the resume went ahead: {question}"
    );
    assert!(
        question.contains("HEAD is a different commit now"),
        "and say WHAT moved, so the person can answer without a git command: {question}"
    );
}

#[test]
fn an_unrelated_operations_parked_work_does_not_switch_off_this_ones_drift_check() {
    // **THE REGRESSION TEST FOR CODE QUALITY #2.** The check skipped itself on
    // `store.any_parked_work()` — a WORKSPACE-WIDE flag — so a park belonging to any other
    // operation silently suppressed drift detection here, and the resume walked into a moved tree.
    // The convention this now follows is stated in `trigger_role`'s own lease gate: `any_parked_work`
    // is a cheap pre-filter, and only `parked_work(&scope.key())` may decide anything.
    let tmp = workspace();
    let (session, _thread) = interrupted_with_the_window_already_past(&tmp);
    the_tree_moves_on(&tmp);

    // Somewhere else entirely in this workspace, an unrelated operation has work parked.
    tmp.with(INTERRUPTED, "carsten", |_ctx, store| {
        store
            .record_parked_work(
                "thread:m-some-other-operation-entirely",
                &nexus_chat::park::Parked {
                    branch: "nxs/park/thread-m-some-other-operation-entirely/20260912T090000Z"
                        .into(),
                    commit: "0000000000000000000000000000000000000000".into(),
                    base_branch: "release/1.2".into(),
                    base_commit: "1111111111111111111111111111111111111111".into(),
                    created_branch: true,
                    committed: true,
                },
                INTERRUPTED,
            )
            .expect("a park belonging to somebody else");
    });
    assert!(
        tmp.store().any_parked_work().expect("read"),
        "the workspace-wide flag is now true, which is the whole point of this case"
    );

    let out = tmp.with(AFTER_THE_WINDOW, "carsten", |ctx, store| {
        orchestration::resume_interrupted(
            ctx,
            store,
            ResumeRequest {
                scope: ResumeScope::Session(&session),
                force: false,
            },
        )
        .expect("decides")
    });

    assert!(
        matches!(out.outcome, ResumeOutcome::TreeMoved { .. }),
        "somebody else's park must not switch off THIS operation's drift check, got {:?}",
        out.outcome
    );
}

#[test]
fn a_runtime_that_cannot_continue_hands_the_hold_to_a_person_rather_than_leaving_it_standing() {
    // **The dead hold** (independent review of PR #476, Integrity & Robustness #2). Every other
    // outcome of a resume waits on something that CHANGES — a window lifts, a process exits, a
    // person answers. This one does not: whether a runtime continues its own conversations is a
    // fixed fact about the workspace, so a hold left open on it is refused in identical words by
    // every future resume, forever, while `nxc status` goes on saying "ON HOLD" as though something
    // were coming.
    let tmp = workspace();
    let (session, thread) = interrupted_with_the_window_already_past(&tmp);
    // It reached a real runtime session, so there IS a conversation whose loss would matter.
    tmp.with(INTERRUPTED, "coder", |_ctx, store| {
        store.bind_session(&session, "sdk-abc").expect("bound");
    });

    let out = tmp.with_worker(AFTER_THE_WINDOW, "carsten", false, |ctx, store| {
        orchestration::resume_interrupted(
            ctx,
            store,
            ResumeRequest {
                scope: ResumeScope::Session(&session),
                force: false,
            },
        )
        .expect("decides")
    });
    assert_eq!(out.outcome, ResumeOutcome::ProviderCannotResume);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);

    // The hold is CLOSED, so nothing keeps offering to resume what can never be resumed.
    assert!(tmp
        .store()
        .current_interruption(&session)
        .expect("readable")
        .is_none());

    // And the person who started the operation is told — what happened, why nothing was started,
    // and what is actually left for them to do.
    let root = tmp.store().thread_root(&thread).expect("a root");
    let bodies: Vec<String> = tmp
        .store()
        .messages_in_thread(&root)
        .expect("read")
        .into_iter()
        .map(|m| m.body)
        .collect();
    let told = bodies
        .iter()
        .find(|b| b.contains("cannot be taken up automatically"))
        .unwrap_or_else(|| panic!("nothing reached the root thread; bodies: {bodies:?}"));
    assert!(
        told.contains("Nothing has been started, and nothing will be"),
        "it must not read like something is coming: {told}"
    );
    assert!(
        told.contains("nxc transcript show"),
        "and it names where the interrupted session's own work can be read: {told}"
    );
}
