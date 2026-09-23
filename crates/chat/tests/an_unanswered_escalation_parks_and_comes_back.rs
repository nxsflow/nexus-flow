//! ACCEPTANCE of nxf 6j6v.de9s — **an unanswered escalation no longer blocks the queue**: the
//! coordinator parks the work on a branch, hands the working copy on, and brings the operation back
//! to its own files when the answer finally arrives.
//!
//! This file is the run the item's Definition of Done asks for, in one sequence and in order:
//!
//! > the holder escalates, a second operation queues, after the deadline the work is committed, the
//! > branch hangs off the OPERATION, the copy is free and the second operation is running. Then the
//! > answer comes, the first operation is resumed on its branch, and the resuming agent is given the
//! > branch name and the commit hash in a DETERMINISTIC text. Counter-test: with nobody waiting,
//! > NOTHING is parked. And: the movement check fires when the BASE has run on, not just the branch.
//!
//! **Everything here is real.** A real git repository in a real temporary directory, the real
//! engine verbs (`surface::send_to`, `orchestration::reply`, `orchestration::tick`), the real
//! lease and the real queue. The only stand-in is the WORKER, and only for what a worker does:
//! [`ParkingWorker`] starts no operating-system session — it records what it was handed, exactly as
//! [`DryWorker`] does — and answers [`Worker::working_copy`] with the repository, which is the one
//! fact the park needs from it. That is the seam the item added; faking anything below it would be
//! the test asserting its own model of git instead of git.
//!
//! **The clock is pinned and moved by hand.** Every verb is called with an explicit `now`, so
//! "thirty minutes pass" is a value in this file rather than a sleep — and the deadline being
//! measured from CONTENTION rather than from the escalation is a thing this file can actually
//! check, because the two instants are different here on purpose.

use std::path::{Path, PathBuf};

use nexus_chat::orchestration::{
    self, ConsequenceClass, Ctx, ReplyReceipt, TickRequest, WakeSkipReason,
};
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{self, ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::worker::{DryWorker, TriggerRequest, TriggerResult, Worker};
use nexus_chat::working_tree::WorkScope;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

/// The coder is commissioned here.
const T0: &str = "2026-09-05T09:00:00Z";
/// …escalates here. Nothing is waiting yet, so NO clock starts — this is the instant the owner's
/// rule deliberately does not measure from.
const ESCALATED: &str = "2026-09-05T09:05:00Z";
/// …and only here does a second operation arrive. THIS is where the thirty minutes begin.
const CONTENDED: &str = "2026-09-05T09:30:00Z";
/// Twenty-nine minutes after the contention began, and fifty-four after the escalation. If the
/// clock ran from the escalation this would already be past the deadline; it is not.
const NOT_YET: &str = "2026-09-05T09:59:00Z";
/// One minute past the contention's own deadline.
const PAST_THE_DEADLINE: &str = "2026-09-05T10:01:00Z";
/// The human finally answers.
const ANSWERED: &str = "2026-09-05T10:10:00Z";

// ---- harness ------------------------------------------------------------------------------------

/// A worker that records like [`DryWorker`] and, unlike it, knows where the sessions would run.
///
/// The second half is the whole of what the park asks a worker for
/// ([`Worker::working_copy`]), and a `DryWorker` answers `None` — which is correct for it and makes
/// it unable to exercise this path.
struct ParkingWorker {
    log: PathBuf,
    root: PathBuf,
    /// Whether [`Worker::working_copy`] answers. `false` is the shape every host that does not run
    /// its sessions in a directory this process can see presents to the park.
    names_a_working_copy: bool,
}

impl Worker for ParkingWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        DryWorker {
            log: Some(self.log.clone()),
        }
        .trigger(req)
    }
    fn working_copy(&self) -> Option<PathBuf> {
        // `None` when this fixture is asked to be a worker that names no directory — the DEFAULT
        // for every implementation except `SidecarWorker`, and therefore the refusal an embedder
        // actually meets. Modelled here rather than with a second struct so the two cases differ by
        // exactly the one answer under test.
        self.names_a_working_copy.then(|| self.root.clone())
    }
}

/// A chat workspace that is ALSO a git repository, with two personas that each want the working
/// copy to themselves.
///
/// The repository starts on `release/1.2` and not on `main`, deliberately: correction 2 of the item
/// says the base is not `main`, and a fixture that starts on `main` cannot tell a correct
/// implementation from one that hardcoded it.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["coder", "solo"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!(
                "handle: {handle}\njob_title: Implementer\nsystem_prompt: You are {handle}.\n\
                 working_tree: exclusive\n"
            ),
        )
        .unwrap();
    }
    git(tmp.path(), &["init", "-q"]);
    git(tmp.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    // `.nxs/` is this workspace's own database and `dry.log` is this test's own recording; neither
    // has any business in the operation's commits. `build/` is what a park must leave behind.
    std::fs::write(tmp.path().join(".gitignore"), ".nxs/\ndry.log\nbuild/\n").unwrap();
    std::fs::write(tmp.path().join("src.txt"), "as it was\n").unwrap();
    git(tmp.path(), &["add", "-A"]);
    commit(tmp.path(), "first");
    git(tmp.path(), &["checkout", "-q", "-b", "release/1.2"]);
    tmp
}

/// Every git call this fixture makes, with a **pinned `$HOME`**.
///
/// Two reasons, and the second is the one that makes it more than a gate's box to tick.
///
/// `every_pinned_home_pins_the_instance.rs` reads source text and flags any suite that runs
/// something spelled `"init"` in an argument list without pinning a home anywhere — `git init`
/// included, which its own doc names as a match that "costs nothing, because that suite pins
/// anyway". This is that suite pinning anyway. (Nothing here reaches the service registry:
/// `foundation::workspace::setup` creates `.nxs/` and registers nothing; the verb that registers is
/// the CLI's `init`, which this file never runs.)
///
/// The second reason is determinism, and it is why this is the right fix rather than a way around
/// the gate: without it these commands read the DEVELOPER's `~/.gitconfig`, so an `init.defaultBranch`,
/// a `commit.gpgsign = true` or a `core.hooksPath` on one machine and not another decides what this
/// test measures. Pinned, the repository has no configured identity at all — which is also the state
/// `park.rs`'s own `commit_identity` fallback exists for, so the park exercises it here rather than
/// borrowing whoever happens to be logged in.
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

fn commit(root: &Path, msg: &str) {
    git(
        root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "commit",
            "-q",
            "--no-gpg-sign",
            "-m",
            msg,
        ],
    );
}

fn head_branch(root: &Path) -> String {
    git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
}

/// Everything a verb needs, rebuilt per call — the declarations are read off the folder each time,
/// exactly as the CLI reads them.
struct Bench {
    tmp: TempDir,
    /// Handed to every [`ParkingWorker`] this bench builds — see that field's own doc.
    names_a_working_copy: bool,
}

impl Bench {
    fn new() -> Self {
        Bench {
            tmp: workspace(),
            names_a_working_copy: true,
        }
    }

    /// A bench whose worker answers `working_copy() -> None`.
    fn without_a_working_copy() -> Self {
        Bench {
            tmp: workspace(),
            names_a_working_copy: false,
        }
    }
    fn root(&self) -> &Path {
        self.tmp.path()
    }
    fn store(&self) -> ChatStore {
        Workspace::resolve(None, self.root())
            .expect("resolve workspace")
            .open_chat_store()
            .expect("open chat store")
    }
    /// Run `f` with a fully built [`Ctx`] and an open store, then drop both so the next call opens
    /// the database file cleanly.
    fn with<T>(
        &self,
        now: &str,
        actor: &str,
        session: Option<&str>,
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
            names_a_working_copy: self.names_a_working_copy,
        };
        let ctx = Ctx {
            now,
            origin: "local",
            actor,
            session,
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

    /// The human commissions a persona. Returns the thread it opened.
    fn commission(&self, now: &str, to: &str, body: &str) -> String {
        self.with(now, "carsten", None, |ctx, store| {
            surface::send_to(
                ctx,
                store,
                SendToRequest {
                    machine: None,
                    to,
                    body,
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("commission")
            .thread_id
        })
    }

    /// Somebody answers into a thread — **the CLI's own door** (`nxc reply --thread`), not
    /// `orchestration::reply` underneath it.
    ///
    /// The difference is the whole reason this test can exist: `surface::reply_in_thread` carries
    /// the 1:1 fallback that resumes the other side of a direct conversation when the reply hands
    /// the turn back to it, and the layer below has only the message-target resume. A human
    /// answering an escalation types `nxc reply --thread`, so that is what is driven here.
    fn reply(
        &self,
        now: &str,
        actor: &str,
        session: Option<&str>,
        thread: &str,
        escalate: bool,
        body: &str,
    ) -> ReplyReceipt {
        self.with(now, actor, session, |ctx, store| {
            surface::reply_in_thread(
                ctx,
                store,
                ReplyThreadRequest {
                    machine: None,
                    thread,
                    body,
                    escalate,
                    needs_rework: false,
                    accept: false,
                },
            )
            .expect("reply is posted and routed")
        })
    }

    fn tick(&self, now: &str, thread: &str) -> orchestration::TickReceipt {
        self.with(now, "carsten", None, |ctx, store| {
            orchestration::tick(ctx, store, TickRequest { thread_id: thread }).expect("tick")
        })
    }

    /// Every trigger the worker was ever handed, whole entries — the `msg=` field embeds newlines,
    /// so an entry runs to the next `trigger role=`.
    fn triggers(&self) -> Vec<String> {
        let raw = std::fs::read_to_string(self.root().join("dry.log")).unwrap_or_default();
        let mut out: Vec<String> = Vec::new();
        for line in raw.lines() {
            if line.starts_with("trigger role=") {
                out.push(line.to_string());
            } else if let Some(last) = out.last_mut() {
                last.push('\n');
                last.push_str(line);
            }
        }
        out
    }

    fn started(&self, handle: &str) -> bool {
        self.triggers()
            .iter()
            .any(|l| l.starts_with(&format!("trigger role={handle} ")))
    }

    /// The session a persona was started under, read back off the trigger the worker was handed.
    fn session_of(&self, handle: &str) -> String {
        let line = self
            .triggers()
            .into_iter()
            .find(|l| l.starts_with(&format!("trigger role={handle} ")))
            .unwrap_or_else(|| panic!("no trigger for {handle} in {:?}", self.triggers()));
        line.split(" session=")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .expect("session field")
            .to_string()
    }

    fn holder(&self, now: &str) -> Option<String> {
        self.store()
            .working_tree_lease_row()
            .expect("lease row")
            .map(|(k, _)| k)
            .filter(|_| {
                self.store()
                    .working_tree_holder(now)
                    .expect("holder")
                    .is_some()
            })
    }

    fn scope_key_of(&self, thread: &str) -> String {
        WorkScope::Thread(thread.to_string()).key()
    }
}

/// The coder is commissioned, dirties the tree, and escalates — the state the whole item is about.
///
/// Returns `(thread, coder session)`.
fn a_holder_that_escalated(b: &Bench) -> (String, String) {
    let thread = b.commission(T0, "coder", "build the thing");
    let session = b.session_of("coder");
    // What a coding agent leaves behind: a tracked change, an untracked file, and a build artifact
    // its `.gitignore` excludes.
    std::fs::write(b.root().join("src.txt"), "as it was\nhalf finished\n").unwrap();
    std::fs::write(b.root().join("notes.md"), "my working notes\n").unwrap();
    std::fs::create_dir_all(b.root().join("build")).unwrap();
    std::fs::write(b.root().join("build/out.bin"), "artifact\n").unwrap();
    b.reply(
        ESCALATED,
        "coder",
        Some(&session),
        &thread,
        true,
        "I cannot go on: which of the two APIs did you mean?",
    );
    (thread, session)
}

// ---- the run ------------------------------------------------------------------------------------

/// **The Definition of Done, in one run.**
#[test]
fn the_queue_moves_the_work_is_on_a_branch_and_the_answer_finds_it_again() {
    let b = Bench::new();
    let (thread, coder_session) = a_holder_that_escalated(&b);
    let scope = b.scope_key_of(&thread);
    let base_commit = git(b.root(), &["rev-parse", "HEAD"]);

    // The escalation did NOT release the working copy — that is the state 6j6v.fabb measured and the
    // reason this item exists at all.
    assert_eq!(
        b.holder(ESCALATED).as_deref(),
        Some(scope.as_str()),
        "an escalation hands the task back, so the claim is still held"
    );

    // ---- a second operation arrives, and only NOW does a clock start ---------------------------
    let solo_thread = b.commission(CONTENDED, "solo", "the other job");
    assert!(!b.started("solo"), "it cannot start: the copy is taken");
    assert_eq!(
        b.store()
            .working_tree_contended_since()
            .expect("contention memo")
            .as_deref(),
        Some(CONTENDED),
        "the clock starts at CONTENTION, not at the escalation — the owner's own decision"
    );

    // ---- twenty-nine minutes later, nothing has happened ---------------------------------------
    b.tick(NOT_YET, &thread);
    assert_eq!(
        b.holder(NOT_YET).as_deref(),
        Some(scope.as_str()),
        "fifty-four minutes after the escalation and twenty-nine after the contention: still held, \
         which is what makes the deadline the contention's and not the escalation's"
    );
    assert!(!b.started("solo"));

    // ---- past the deadline: the park ------------------------------------------------------------
    let receipt = b.tick(PAST_THE_DEADLINE, &thread);
    let parked = receipt
        .parked
        .expect("the tick parked the stranded operation");
    assert_eq!(parked.scope, scope);
    assert_eq!(parked.contended_since, CONTENDED);
    assert!(parked.parked.committed);
    assert!(parked.parked.created_branch);
    assert!(
        parked.parked.branch.starts_with("nxs/park/"),
        "{}",
        parked.parked.branch
    );

    // …the work is committed, and it is ALL of it: the tracked change and the untracked file.
    let files = git(
        b.root(),
        &["show", "--name-only", "--format=", &parked.parked.commit],
    );
    assert!(files.contains("src.txt"), "{files}");
    assert!(files.contains("notes.md"), "{files}");
    assert!(
        !files.contains("build/"),
        "an ignored file is not committed: {files}"
    );

    // …the branch hangs off the OPERATION, as a list.
    let rows = b.store().parked_work(&scope).expect("parked work");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].parked.branch, parked.parked.branch);
    assert_eq!(rows[0].parked.base_branch, "release/1.2");
    assert_eq!(rows[0].parked.base_commit, base_commit);

    // …the tree is back on its base — `release/1.2`, never `main` (correction 2).
    assert_eq!(head_branch(b.root()), "release/1.2");
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        "as it was\n"
    );
    assert!(!b.root().join("notes.md").exists());
    assert!(
        b.root().join("build/out.bin").exists(),
        "an ignored file is left where it is — decision (2)"
    );

    // …and the second operation is running.
    assert!(b.started("solo"), "the queue moved");
    assert_eq!(
        b.holder(PAST_THE_DEADLINE).as_deref(),
        Some(b.scope_key_of(&solo_thread).as_str()),
        "the copy did not merely become free, it was handed on"
    );

    // ---- the second operation finishes, and the answer comes ------------------------------------
    let solo_session = b.session_of("solo");
    b.reply(
        ANSWERED,
        "solo",
        Some(&solo_session),
        &solo_thread,
        false,
        "done",
    );
    assert!(
        b.holder(ANSWERED).is_none(),
        "a finished operation gives the copy back"
    );

    let before = b.triggers().len();
    b.reply(
        ANSWERED,
        "carsten",
        None,
        &thread,
        false,
        "the second one. Carry on.",
    );

    // The coder is resumed, on its own branch, and TOLD.
    let resumed = b.triggers()[before..]
        .iter()
        .find(|l| l.starts_with("trigger role=coder "))
        .cloned()
        .expect("the answer resumed the coder");
    assert!(resumed.contains(&format!("session={coder_session}")));
    assert_eq!(
        head_branch(b.root()),
        parked.parked.branch,
        "the tree is back on the parked branch"
    );
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        "as it was\nhalf finished\n",
        "and the work is in it"
    );
    // The deterministic text: branch name and commit hash, in the message the session reads.
    assert!(resumed.contains("PARKED WORK"), "{resumed}");
    assert!(resumed.contains(&parked.parked.branch), "{resumed}");
    assert!(resumed.contains(&parked.parked.commit), "{resumed}");
    assert!(
        resumed.contains("the second one. Carry on."),
        "the human's own words are still there, after the notice: {resumed}"
    );

    // And it is not told twice: the row is stamped as come back for.
    assert!(
        b.store()
            .parked_work(&scope)
            .expect("parked work")
            .is_empty(),
        "the park has been resumed"
    );
}

/// **Point 4 of the owner's rule**: *"Kommt die Antwort spaeter, stellt sie sich in die
/// Warteschlange."*
///
/// This is the case the park CREATES and nothing before it had to handle. An answer to an escalation
/// arrives as a `ChainMove::Unwind` resume, and such a resume has never asked for the working-tree
/// lease — correctly, because the operation still held its own claim. A park gives that claim away.
/// Without the gate widening, this answer would walk straight into a checkout that now belongs to
/// somebody else, which is the collision the whole epic exists to prevent, arriving through the one
/// door it had left open.
#[test]
fn an_answer_that_arrives_while_somebody_else_has_the_copy_waits_its_turn() {
    let b = Bench::new();
    let (thread, _) = a_holder_that_escalated(&b);
    let solo_thread = b.commission(CONTENDED, "solo", "the other job");
    let parked = b
        .tick(PAST_THE_DEADLINE, &thread)
        .parked
        .expect("parked")
        .parked;
    assert!(b.started("solo"));

    // The human answers while `solo` is still working.
    let before = b.triggers().len();
    let receipt = b.reply(
        ANSWERED,
        "carsten",
        None,
        &thread,
        false,
        "the second one. Carry on.",
    );
    // **Asserted ON THE RECEIPT, not inferred from the trigger log** (independent review of PR
    // #435, Test Quality #2). The absence of a trigger is a side effect that several unrelated
    // failures produce; what the changelog PROMISES an embedder is this exact pair, and until this
    // assertion nothing in the suite ever looked at `WakeSkipReason::Queued` at all.
    assert!(receipt.posted, "the answer is durably in the thread");
    assert!(!receipt.resumed);
    assert_eq!(receipt.woke, None, "no session is running for this answer");
    let skipped = receipt
        .wake_skipped
        .as_ref()
        .expect("a resume that did not land says so");
    assert_eq!(skipped.reason, WakeSkipReason::Queued);
    assert!(
        skipped
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("parked") && d.contains("queued")),
        "the detail has to say WHY, not just that something was skipped: {:?}",
        skipped.detail
    );
    assert!(
        !b.triggers()[before..]
            .iter()
            .any(|l| l.starts_with("trigger role=coder ")),
        "the coder must NOT start: it no longer holds the copy, and `solo` does"
    );
    assert_eq!(
        head_branch(b.root()),
        "release/1.2",
        "and nothing touched the tree solo is working in"
    );

    // `solo` finishes and gives the copy back; the queue starts the coder — on its own branch, with
    // the deterministic text.
    let solo_session = b.session_of("solo");
    b.reply(
        "2026-09-05T10:20:00Z",
        "solo",
        Some(&solo_session),
        &solo_thread,
        false,
        "done",
    );
    let resumed = b.triggers()[before..]
        .iter()
        .find(|l| l.starts_with("trigger role=coder "))
        .cloned()
        .expect("the queue started the coder once the copy came free");
    assert!(resumed.contains("PARKED WORK"), "{resumed}");
    assert!(resumed.contains(&parked.branch), "{resumed}");
    assert!(resumed.contains(&parked.commit), "{resumed}");
    assert!(
        resumed.contains("the second one. Carry on."),
        "the human's own answer travelled with it: {resumed}"
    );
    assert_eq!(head_branch(b.root()), parked.branch);
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        "as it was\nhalf finished\n"
    );
}

/// **A park that CANNOT happen says so on the tick receipt** (independent review of PR #435, Test
/// Quality #1).
///
/// `ParkRefusal::NoWorkingCopy` is the default answer of every `Worker` implementation except
/// `SidecarWorker` — so for an embedder bringing its own runtime it is not an edge case, it is THE
/// case. Nothing exercised it anywhere, and nothing anywhere confirmed that a refusal reaches
/// `TickReceipt.warnings` rather than being swallowed. The refusal is the whole visible half of
/// this mechanism when it declines; a silent decline would be the exact failure class the receipt
/// exists to close.
///
/// **What the decline DOES changed with nxf 6j6v.8bv9.** This test used to pin that the claim stays,
/// the queue waits, and the finding sends a human to `nxc release`. The owner's refusal rule of
/// 2026-09-17 makes a host that names no working copy a PERMANENT refusal — no tick will ever
/// answer it differently — so the copy is now handed on UNPARKED, the finding is
/// `work_handed_on_unparked`, and the escalating operation's uncommitted work stays in the tree for
/// the next holder. `a_refused_park_follows_the_rule.rs` holds the rule for every refusal; this keeps
/// the case where it was first pinned.
#[test]
fn a_park_that_has_no_working_copy_to_park_in_says_so_on_the_receipt() {
    let b = Bench::without_a_working_copy();
    let (thread, _) = a_holder_that_escalated(&b);
    let scope = b.scope_key_of(&thread);
    let solo = b.commission(CONTENDED, "solo", "the other job");

    let receipt = b.tick(PAST_THE_DEADLINE, &thread);

    assert!(receipt.parked.is_none(), "nothing was parked");
    let finding = receipt
        .warnings
        .iter()
        .find(|w| w.class == ConsequenceClass::WorkHandedOnUnparked)
        .expect("the refusal is reported as a finding, not swallowed");
    assert_eq!(
        finding.thread.as_deref(),
        Some(thread.as_str()),
        "anchored at the claim ROOT, so a reader knows whose work was left in the tree"
    );
    assert!(
        finding
            .detail
            .contains("does not run sessions in a directory"),
        "the finding says WHY: {}",
        finding.detail
    );
    assert!(
        finding.detail.contains("WITHOUT parking"),
        "…and what happened instead: {}",
        finding.detail
    );
    assert!(
        matches!(
            receipt.handed_on.as_ref().and_then(|h| h.unparked.as_ref()),
            Some(nexus_chat::park::ParkRefusal::NoWorkingCopy(_))
        ),
        "the receipt names the hand-off and its refusal: {:?}",
        receipt.handed_on
    );
    // The copy moved on — a refusal no tick can change must not hold a queue for ever — and the
    // escalating operation's work is exactly where it was: uncommitted, in the tree.
    assert_eq!(
        b.holder(PAST_THE_DEADLINE).as_deref(),
        Some(b.scope_key_of(&solo).as_str())
    );
    assert!(b.started("solo"), "the queue moved");
    assert!(b.store().parked_work(&scope).unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        "as it was\nhalf finished\n",
        "the escalating operation's work is untouched, and stays for the next holder"
    );
}

/// **A tree mid-merge refuses at the tick seam too**, not only in `park.rs`'s own unit test.
///
/// Same argument as the test above, for the refusal that is about the WORKING COPY rather than
/// about the host: `park.rs` proves `WorkingCopy::park` declines, and this proves the decline
/// travels — as a finding a reader sees, with the claim and the queue left standing.
#[test]
fn a_tree_mid_merge_refuses_the_park_and_leaves_everything_standing() {
    let b = Bench::new();
    let (thread, _) = a_holder_that_escalated(&b);
    let scope = b.scope_key_of(&thread);
    let _solo = b.commission(CONTENDED, "solo", "the other job");

    // A conflicting merge, stopped: `MERGE_HEAD` is present and the index carries conflicts.
    git(b.root(), &["stash", "--include-untracked", "-q"]);
    git(b.root(), &["checkout", "-q", "-b", "other"]);
    std::fs::write(b.root().join("src.txt"), "other side\n").unwrap();
    git(b.root(), &["add", "-A"]);
    commit(b.root(), "other");
    git(b.root(), &["checkout", "-q", "release/1.2"]);
    std::fs::write(b.root().join("src.txt"), "this side\n").unwrap();
    git(b.root(), &["add", "-A"]);
    commit(b.root(), "this");
    let merge = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "merge",
            "other",
        ])
        .current_dir(b.root())
        .output()
        .unwrap();
    assert!(!merge.status.success(), "the merge is supposed to conflict");

    let receipt = b.tick(PAST_THE_DEADLINE, &thread);

    assert!(receipt.parked.is_none());
    let finding = receipt
        .warnings
        .iter()
        .find(|w| w.class == ConsequenceClass::WorkNotParked)
        .expect("the refusal is reported");
    assert!(
        finding.detail.contains("a merge is in progress"),
        "the finding names the state, so a reader knows which git command finishes it: {}",
        finding.detail
    );
    assert_eq!(b.holder(PAST_THE_DEADLINE).as_deref(), Some(scope.as_str()));
    assert!(!b.started("solo"));
    assert!(b.root().join(".git/MERGE_HEAD").exists(), "untouched");
    assert_eq!(git(b.root(), &["branch", "--list", "nxs/park/*"]), "");
}

/// **The counter-test the Definition of Done names**: with nobody waiting, NOTHING is parked.
///
/// This is what keeps the mechanism off the ordinary path. An escalation that costs nobody anything
/// is not a problem to be solved, and a park that fired here would be committing half-finished work
/// and moving a developer's checkout out from under them for no reason at all.
#[test]
fn with_nobody_waiting_an_escalation_is_never_parked() {
    let b = Bench::new();
    let (thread, _) = a_holder_that_escalated(&b);
    let scope = b.scope_key_of(&thread);

    // Long past any deadline the contention would have had.
    b.tick(PAST_THE_DEADLINE, &thread);
    b.tick("2026-09-05T12:00:00Z", &thread);

    assert!(
        b.store()
            .working_tree_contended_since()
            .expect("contention memo")
            .is_none(),
        "nothing is contended, so nothing is measured"
    );
    assert!(
        b.store()
            .parked_work(&scope)
            .expect("parked work")
            .is_empty(),
        "and nothing is parked"
    );
    assert_eq!(head_branch(b.root()), "release/1.2");
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        "as it was\nhalf finished\n",
        "the escalating operation's work is untouched, in the tree, exactly where it left it"
    );
    assert_eq!(git(b.root(), &["branch", "--list", "nxs/park/*"]), "");
}

/// **The contention memo is a memo, not a state machine**: an answered escalation ends it, and the
/// next one is measured from its own beginning rather than from a stale instant.
#[test]
fn an_answered_escalation_ends_the_contention_it_started() {
    let b = Bench::new();
    let (thread, coder_session) = a_holder_that_escalated(&b);

    let _solo = b.commission(CONTENDED, "solo", "the other job");
    assert_eq!(
        b.store().working_tree_contended_since().unwrap().as_deref(),
        Some(CONTENDED)
    );

    // The human answers INSIDE the thirty minutes. The coder owes a reply again, so nobody is
    // stranded any more — and the clock must not keep running towards a park.
    b.reply(
        NOT_YET,
        "carsten",
        None,
        &thread,
        false,
        "the second one. Carry on.",
    );
    assert!(
        b.store().working_tree_contended_since().unwrap().is_none(),
        "the escalation was answered: there is nothing to park any more"
    );

    // …and a tick past what WOULD have been the deadline parks nothing.
    b.tick(PAST_THE_DEADLINE, &thread);
    assert!(b
        .store()
        .parked_work(&b.scope_key_of(&thread))
        .unwrap()
        .is_empty());
    assert_eq!(
        head_branch(b.root()),
        "release/1.2",
        "the coder is working again, in the tree, on its base"
    );
    let _ = coder_session;
}

/// **The movement check fires when the BASE has run on** — the last line of the Definition of Done,
/// end to end rather than at the git seam alone (`park.rs`'s own tests cover that half).
///
/// The parked branch is untouched here: its tip is still exactly the commit that was parked. The
/// draft this corrects would have reported calm and the coder would have carried on believing its
/// foundation had not moved.
#[test]
fn the_resumed_session_is_told_when_the_base_ran_on_under_its_untouched_branch() {
    let b = Bench::new();
    let (thread, _) = a_holder_that_escalated(&b);
    let solo_thread = b.commission(CONTENDED, "solo", "the other job");
    let parked = b
        .tick(PAST_THE_DEADLINE, &thread)
        .parked
        .expect("parked")
        .parked;

    // The operation that got the copy lands its own work on the BASE branch and finishes.
    std::fs::write(b.root().join("theirs.txt"), "shipped\n").unwrap();
    git(b.root(), &["add", "-A"]);
    commit(b.root(), "the other job, landed");
    let base_now = git(b.root(), &["rev-parse", "release/1.2"]);
    let solo_session = b.session_of("solo");
    b.reply(
        ANSWERED,
        "solo",
        Some(&solo_session),
        &solo_thread,
        false,
        "done",
    );

    let before = b.triggers().len();
    b.reply(
        ANSWERED,
        "carsten",
        None,
        &thread,
        false,
        "the second one. Carry on.",
    );
    let resumed = b.triggers()[before..]
        .iter()
        .find(|l| l.starts_with("trigger role=coder "))
        .cloned()
        .expect("the answer resumed the coder");

    assert!(
        resumed.contains("the base release/1.2 has moved on"),
        "the base moved and the session has to be told: {resumed}"
    );
    assert!(resumed.contains(&base_now), "{resumed}");
    assert!(
        !resumed.contains(&format!("{} has moved on", parked.branch)),
        "our own branch did not move, and saying it did would send the reader looking for commits \
         that are not there: {resumed}"
    );
}
