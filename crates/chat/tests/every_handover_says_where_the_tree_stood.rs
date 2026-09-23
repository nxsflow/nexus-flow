//! ACCEPTANCE of nxf 6j6v.2af2 — **every handover records where the working copy stood**: a `send`
//! and a `reply` each stamp the commit, the branch and a comparable fingerprint onto their own
//! message, the reads give it back, and a dirty tree is said out loud instead of being committed
//! behind somebody's back.
//!
//! The item in one sentence: *a message says WHO said WHAT and WHEN, and did not say on which state
//! of the working copy that held.* So the three things this file has to show are the three uses the
//! item names, in its order:
//!
//! 1. **Continuing** — the anchor of the handover a fresh session would take up (nxf 6j6v.npy3 is
//!    the resume; this is the fact it needs, and the DRIFT check it runs).
//! 2. **Reconstructing** — what did this reviewer actually have in front of it (`nxc threads show`).
//! 3. **Rewinding** — the commit is named, per handover.
//!
//! **Everything here is real.** A real git repository in a real temporary directory, the real seam
//! verbs (`surface::send_to`, `surface::reply_in_thread`, `facade::status`), the real store. The only
//! stand-in is the WORKER, and only for what a worker does: [`AnchoringWorker`] starts no
//! operating-system session — it records like [`DryWorker`] — and answers
//! [`Worker::working_copy`] with the repository, which is the one fact this path needs from it. That
//! is the seam the item uses; faking anything below it would be the test asserting its own model of
//! git instead of git.
//!
//! **The counter-tests are half of it**, and each one is a way the feature could be wrong while
//! every assertion above it passed:
//!
//! * a host that names no working copy records NOTHING, and the report says why rather than leaving
//!   a reader with a column of nulls;
//! * an untracked file — which is in neither `HEAD` nor `git diff` — moves the fingerprint;
//! * a handover does not COMMIT: HEAD, the branch and the dirty files are exactly as they were
//!   afterwards;
//! * `.nxs/` is written by every one of these calls and never reaches the anchor.

use std::path::{Path, PathBuf};

use nexus_chat::anchor::Anchor;
use nexus_chat::facade;
use nexus_chat::model::{Disposition, MessageKind, Priority, Refs};
use nexus_chat::orchestration::{self, Ctx, RoleResumeRequest};
use nexus_chat::park::WorkingCopy;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{self, ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::worker::{DryWorker, TriggerRequest, TriggerResult, Worker};
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const T0: &str = "2026-09-13T09:00:00Z";
const T1: &str = "2026-09-13T09:05:00Z";
const T2: &str = "2026-09-13T09:10:00Z";

// ---- harness ------------------------------------------------------------------------------------

/// A worker that records like [`DryWorker`] and, unlike it, knows where the sessions would run.
///
/// The second half is the whole of what this path asks a worker for
/// ([`Worker::working_copy`]); a `DryWorker` answers `None`, which is correct for it and makes it
/// unable to exercise this at all. Modelled on `an_unanswered_escalation_parks_and_comes_back.rs`'s
/// `ParkingWorker`, deliberately: the two features ask the worker the same one question, and a
/// second fixture shape would invite the two to drift.
struct AnchoringWorker {
    log: PathBuf,
    root: PathBuf,
    /// Whether [`Worker::working_copy`] answers. `false` is the shape every host that does not run
    /// its sessions in a directory this process can see presents — the DEFAULT for every
    /// implementation except `SidecarWorker`, and therefore the case an embedder actually meets.
    names_a_working_copy: bool,
}

impl Worker for AnchoringWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        DryWorker {
            log: Some(self.log.clone()),
        }
        .trigger(req)
    }
    fn working_copy(&self) -> Option<PathBuf> {
        self.names_a_working_copy.then(|| self.root.clone())
    }
}

/// A chat workspace that is ALSO a git repository, with a human and two personas.
///
/// It starts on `release/1.2` rather than on `main`, deliberately: a fixture on the default branch
/// cannot tell an implementation that READS the branch from one that assumes it.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["coder", "reviewer"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\njob_title: Implementer\nsystem_prompt: You are {handle}.\n"),
        )
        .unwrap();
    }
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: coding\n  members: [coder, reviewer]\n",
    )
    .unwrap();
    git(tmp.path(), &["init", "-q"]);
    git(tmp.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    // **`.nxs/` and the dry log are ignored, and that is the promise under test, not fixture
    // hygiene**: the board and the channel live in `.nxs/`, every call below writes to them, and the
    // anchor must never see it — "the repository rewinds, the record does not".
    std::fs::write(tmp.path().join(".gitignore"), ".nxs/\ndry.log\n").unwrap();
    std::fs::write(tmp.path().join("src.txt"), "as it was\n").unwrap();
    git(tmp.path(), &["add", "-A"]);
    commit(tmp.path(), "first");
    git(tmp.path(), &["checkout", "-q", "-b", "release/1.2"]);
    tmp
}

/// Every git call this fixture makes, with a **pinned `$HOME`** — for
/// `an_unanswered_escalation_parks_and_comes_back.rs`'s two reasons, which hold here unchanged: the
/// `every_pinned_home_pins_the_instance.rs` gate reads source text and flags a suite that runs
/// anything spelled `"init"` without pinning a home, and — the reason that matters — without it these
/// commands read the DEVELOPER's `~/.gitconfig`, so an `init.defaultBranch` or a `core.hooksPath` on
/// one machine and not another would decide what this test measures.
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

struct Bench {
    tmp: TempDir,
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

    /// Run `f` with a fully built [`Ctx`] and an open store, then drop both so the next call opens
    /// the database file cleanly.
    fn with<T>(&self, now: &str, actor: &str, f: impl FnOnce(&Ctx, &mut ChatStore) -> T) -> T {
        let ws = Workspace::resolve(None, self.root()).expect("resolve workspace");
        let db_path = ws.db_path_str().expect("db path");
        let mut store = ws.open_chat_store().expect("open chat store");
        let defs =
            nexus_chat::definitions::Definitions::resolve(self.root()).expect("declarations");
        let worker = AnchoringWorker {
            log: self.root().join("dry.log"),
            root: self.root().to_path_buf(),
            names_a_working_copy: self.names_a_working_copy,
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

    /// The human commissions somebody. Returns the thread it opened.
    fn commission(&self, now: &str, to: &str, body: &str) -> String {
        self.with(now, "carsten", |ctx, store| {
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

    /// Somebody answers into a thread, through the CLI's own door (`nxc reply --thread`).
    fn reply(&self, now: &str, actor: &str, thread: &str, body: &str) {
        self.with(now, actor, |ctx, store| {
            surface::reply_in_thread(
                ctx,
                store,
                ReplyThreadRequest {
                    machine: None,
                    thread,
                    body,
                    escalate: false,
                    needs_rework: false,
                    accept: false,
                },
            )
            .expect("reply");
        });
    }

    /// The messages of one thread, oldest first — the read `nxc threads show` serves.
    fn messages(&self, thread: &str) -> Vec<facade::MessageView> {
        self.with(T2, "carsten", |_ctx, store| {
            // `undeclared()` is the honest policy for a direct conversation — no declaration says
            // anything about who may see what in a `dm:` channel.
            facade::thread(
                store,
                thread,
                "local/carsten",
                nexus_chat::channel::ChannelPolicy::undeclared(),
            )
            .expect("thread")
            .messages
        })
    }

    /// The whole status report — the read `nxc status` serves.
    fn status(&self) -> facade::StatusReport {
        self.with(T2, "carsten", |ctx, store| {
            facade::status(store, ctx.worker, T2, facade::StatusScope::All(None)).expect("status")
        })
    }

    /// The one row of the status report for `thread`.
    fn status_thread(&self, thread: &str) -> facade::StatusThread {
        self.status()
            .operations
            .into_iter()
            .flat_map(|o| o.threads)
            .find(|t| t.thread_id == thread)
            .unwrap_or_else(|| panic!("no status row for {thread}"))
    }

    /// Where the tree stands RIGHT NOW — the `now` half of a drift check, read the way a resume
    /// would read it.
    fn anchor_now(&self) -> Anchor {
        WorkingCopy::at(self.root()).anchor().expect("anchor")
    }

    /// The commission receipt of a persona summon — for the cases that need the session it minted.
    fn commission_receipt(&self, now: &str, to: &str, body: &str) -> surface::SendToReceipt {
        self.with(now, "carsten", |ctx, store| {
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
        })
    }

    /// Every thread hanging directly under `parent`, as status rows.
    fn children_of(&self, parent: &str) -> Vec<facade::StatusThread> {
        self.status()
            .operations
            .into_iter()
            .flat_map(|o| o.threads)
            .filter(|t| t.parent.as_deref() == Some(parent))
            .collect()
    }
}

/// The anchor a message carries, or a panic naming the message that should have had one.
fn anchor_of(m: &facade::MessageView) -> &Anchor {
    m.refs
        .working_copy
        .as_ref()
        .unwrap_or_else(|| panic!("message {} carries no anchor", m.message_id))
}

// ---- the acceptance ----------------------------------------------------------------------------

#[test]
fn a_commission_records_the_commit_the_branch_and_a_fingerprint() {
    // Use (3) of the item, and the floor everything else stands on: `send --to <persona>` writes
    // where the tree stood onto the message it posts.
    let b = Bench::new();
    let head = git(b.root(), &["rev-parse", "HEAD"]);
    let thread = b.commission(T0, "coder", "build it");

    let messages = b.messages(&thread);
    let a = anchor_of(&messages[0]);
    assert_eq!(a.commit, head, "the commit HEAD pointed at, in full");
    assert_eq!(a.branch, "release/1.2", "the branch, READ and not assumed");
    assert_eq!(a.fingerprint.len(), 64, "sha256, hex");
    // **The tree is clean although the engine has just written to `.nxs/`** — the board, the
    // channel, the thread and this very message. That is the promise: git does not track `.nxs/`,
    // so the record is outside the anchor in both directions.
    assert!(
        !a.dirty,
        "`.nxs/` is written by this call and must not reach the anchor"
    );
}

#[test]
fn a_reply_records_the_tree_the_answering_side_had_in_front_of_it() {
    // Use (2) of the item: what did this reviewer actually have? The commission and the reply are
    // two different states of the tree, and each message carries its own.
    let b = Bench::new();
    let thread = b.commission(T0, "coder", "build it");
    let at_commission = anchor_of(&b.messages(&thread)[0]).clone();

    // The coder works: one tracked edit, one file git has never seen.
    std::fs::write(b.root().join("src.txt"), "as it is now\n").unwrap();
    std::fs::write(b.root().join("fresh.txt"), "brand new\n").unwrap();
    b.reply(T1, "coder", &thread, "done");

    let messages = b.messages(&thread);
    let a = anchor_of(messages.last().unwrap());
    assert_eq!(
        a.commit, at_commission.commit,
        "nothing was committed, so the commit is the one the work started from"
    );
    assert!(a.dirty, "there IS uncommitted work, and the record says so");
    assert_ne!(
        a.fingerprint, at_commission.fingerprint,
        "the two handovers saw two different trees"
    );
}

#[test]
fn a_handover_never_writes_to_the_repository() {
    // The decision this item had to make, as a test: a dirty tree is RECORDED honestly, never
    // committed. A handover is exactly the moment a live session is sitting in the working copy —
    // `nxc reply` is what an agent runs from inside its own session — so committing here would move
    // HEAD out from under the process still working. Securing that work when the copy CHANGES HANDS
    // is nxf 6j6v.8bv9's, and it is a decision rather than a completion.
    let b = Bench::new();
    let head_before = git(b.root(), &["rev-parse", "HEAD"]);
    let branches_before = git(b.root(), &["branch", "--list", "--format=%(refname)"]);

    std::fs::write(b.root().join("src.txt"), "mid-work\n").unwrap();
    std::fs::write(b.root().join("fresh.txt"), "brand new\n").unwrap();
    let thread = b.commission(T0, "coder", "build it");
    b.reply(T1, "coder", &thread, "done");

    assert_eq!(git(b.root(), &["rev-parse", "HEAD"]), head_before, "HEAD");
    assert_eq!(
        git(b.root(), &["branch", "--list", "--format=%(refname)"]),
        branches_before,
        "no branch was created"
    );
    assert_eq!(
        std::fs::read_to_string(b.root().join("src.txt")).unwrap(),
        "mid-work\n",
        "the working file is untouched"
    );
    assert!(
        b.root().join("fresh.txt").exists(),
        "the untracked file is still where the agent left it"
    );
    assert_eq!(
        git(
            b.root(),
            &["status", "--porcelain", "--untracked-files=all"]
        )
        .lines()
        .count(),
        2,
        "still exactly the two changes the agent made, staged by nobody"
    );
}

#[test]
fn an_untracked_file_alone_moves_the_fingerprint() {
    // Precision 2 of the owner's note, end to end through real git rather than over the pure
    // function: a newly created file appears in NEITHER `HEAD` nor `git diff`, so a fingerprint over
    // those two alone would call the commonest change an agent makes no change at all.
    let b = Bench::new();
    let before = b.anchor_now();
    std::fs::write(b.root().join("brand-new.txt"), "hello\n").unwrap();
    let after = b.anchor_now();

    assert_eq!(after.commit, before.commit, "nothing was committed");
    assert_eq!(
        git(b.root(), &["diff", "HEAD"]),
        "",
        "and `git diff` sees nothing at all — which is the point"
    );
    assert_ne!(after.fingerprint, before.fingerprint);
    assert!(after.dirty);
}

#[test]
fn the_drift_at_a_recorded_handover_is_what_a_resume_asks() {
    // Use (1), and the owner's decision of 2026-09-12: *"bei der Wiederaufnahme vergleichen. Ist der
    // Zustand gewandert, geht eine Frage zurueck an den anfaenglichen Faden."* The comparison is what
    // this item owes nxf 6j6v.npy3; the question back is that item's.
    let b = Bench::new();
    let thread = b.commission(T0, "coder", "build it");
    let handed_over = anchor_of(&b.messages(&thread)[0]).clone();

    // Nothing has happened yet: a resume starting here would find its own world.
    assert!(
        !b.anchor_now().drift_from(&handed_over).any(),
        "an untouched tree has not drifted"
    );

    // Somebody edits the tree while the operation waits — and commits nothing.
    std::fs::write(b.root().join("src.txt"), "somebody else was here\n").unwrap();
    let d = b.anchor_now().drift_from(&handed_over);
    assert!(d.any(), "the tree moved");
    assert_eq!(d.commit_now, None, "and not by a commit");
    assert!(d.uncommitted_now, "the uncommitted work is not what it was");

    // …and now a commit, which is the other half.
    git(b.root(), &["add", "-A"]);
    commit(b.root(), "second");
    let d = b.anchor_now().drift_from(&handed_over);
    assert_eq!(
        d.commit_now.as_deref(),
        Some(git(b.root(), &["rev-parse", "HEAD"]).as_str()),
        "the new HEAD is named, so a reader can go and read the history"
    );
}

#[test]
fn the_status_view_shows_the_anchor_of_the_newest_handover() {
    // "Sichtbar machen, sonst sind es Daten ohne Leser" — the same finding nxf 6j6v.s9ex names for
    // the parked branch next door. And the rule that decides WHICH anchor: the newest one in the
    // thread, which before any answer is the COMMISSION — the tree the still-working session was
    // started in, and exactly what a resume needs.
    let b = Bench::new();
    let thread = b.commission(T0, "coder", "build it");
    let at_commission = anchor_of(&b.messages(&thread)[0]).clone();
    assert_eq!(
        b.status_thread(&thread).working_copy.as_ref(),
        Some(&at_commission),
        "a thread nobody has answered shows the commission's anchor"
    );

    std::fs::write(b.root().join("src.txt"), "done\n").unwrap();
    b.reply(T1, "coder", &thread, "done");
    let at_reply = anchor_of(b.messages(&thread).last().unwrap()).clone();
    assert_ne!(at_reply, at_commission, "the tree moved between the two");
    assert_eq!(
        b.status_thread(&thread).working_copy.as_ref(),
        Some(&at_reply),
        "and afterwards the NEWEST handover's"
    );
    assert!(
        b.status().worker_names_a_working_copy,
        "this runtime does name one"
    );
}

#[test]
fn every_member_of_a_fan_out_carries_the_state_it_was_handed() {
    // The measured shape of nxf 6j6v.npy3 is a CHANNEL step — the `__synth__` session of a review
    // round, interrupted 0.6s after its answer. A member's commission is written by the supervisor
    // and not by the requester, so it reaches the stamp through a different door
    // (`open_flow_step` -> `coordinator_commission_in`); if that door were missed, every 1:1
    // assertion above would still pass and the one case the resume exists for would carry nothing.
    let b = Bench::new();
    let board = b.commission(T0, "coding", "review this");
    let head = git(b.root(), &["rev-parse", "HEAD"]);

    let rows: Vec<facade::StatusThread> = b
        .status()
        .operations
        .into_iter()
        .flat_map(|o| o.threads)
        .filter(|t| t.parent.as_deref() == Some(board.as_str()))
        .collect();
    assert_eq!(rows.len(), 2, "two members, two slot threads");
    for row in rows {
        let a = row
            .working_copy
            .unwrap_or_else(|| panic!("member thread {} carries no anchor", row.thread_id));
        assert_eq!(a.commit, head);
        assert_eq!(a.branch, "release/1.2");
    }
}

#[test]
fn a_host_that_names_no_working_copy_records_nothing_and_the_report_says_why() {
    // The refusal an EMBEDDER actually meets: `Worker::working_copy` answers `None` for every
    // implementation except the shipped sidecar. Nothing is recorded, nothing fails, and the reader
    // is told ONCE — not left to conclude from a column of nulls that the feature is broken.
    let b = Bench::without_a_working_copy();
    let thread = b.commission(T0, "coder", "build it");
    b.reply(T1, "coder", &thread, "done");

    for m in b.messages(&thread) {
        assert!(
            m.refs.working_copy.is_none(),
            "no anchor was invented for a host that names no tree"
        );
    }
    let report = b.status();
    assert!(
        !report.worker_names_a_working_copy,
        "and the report says so"
    );
    assert!(
        b.status_thread(&thread).working_copy.is_none(),
        "the row carries nothing either"
    );
}

#[test]
fn a_working_copy_that_is_not_a_repository_is_a_named_refusal_and_not_a_panic() {
    // A project that is not a git repository is a legitimate thing to run sessions in, so this is
    // not a failure — it is an anchor that cannot exist, said in words that name the state rather
    // than whatever the next git command happened to print.
    let tmp = TempDir::new().unwrap();
    let refusal = WorkingCopy::at(tmp.path())
        .anchor()
        .expect_err("a bare directory is no repository");
    assert!(
        matches!(
            refusal,
            nexus_chat::anchor::AnchorRefusal::NotARepository(_)
        ),
        "{refusal:?}"
    );
    assert!(
        refusal.detail().contains("not a git repository"),
        "the refusal carries its own words: {}",
        refusal.detail()
    );
}

#[test]
fn a_repository_with_no_commit_yet_is_a_refusal_rather_than_an_invented_sha() {
    // An unborn HEAD is a real state, and there is genuinely nothing to anchor to. A zero sha or an
    // empty string would be a value that reads like an answer.
    let tmp = TempDir::new().unwrap();
    git(tmp.path(), &["init", "-q"]);
    let refusal = WorkingCopy::at(tmp.path())
        .anchor()
        .expect_err("no commit, no anchor");
    assert!(
        matches!(refusal, nexus_chat::anchor::AnchorRefusal::Git(_)),
        "{refusal:?}"
    );
}

#[test]
fn a_detached_head_is_recorded_as_the_state_it_is() {
    // `branch` is empty and the commit still names the point exactly — `crate::park::Base` keeps a
    // detached HEAD the same way, and normalising it to something prettier would make a resume
    // check out a branch the tree was never on.
    let b = Bench::new();
    let head = git(b.root(), &["rev-parse", "HEAD"]);
    git(b.root(), &["checkout", "-q", "--detach"]);
    let thread = b.commission(T0, "coder", "build it");

    let a = anchor_of(&b.messages(&thread)[0]).clone();
    assert_eq!(a.commit, head);
    assert_eq!(a.branch, "", "no branch, and that is the answer");
    // Back onto the branch it started on: the same commit, and a drift a resume must be told about.
    git(b.root(), &["checkout", "-q", "release/1.2"]);
    let d = b.anchor_now().drift_from(&a);
    assert_eq!(d.branch_now.as_deref(), Some("release/1.2"));
    assert_eq!(d.commit_now, None, "the commit never moved");
}

// ---- the findings of the independent review of PR #474 ---------------------------------------

#[test]
fn a_tracked_file_that_is_not_valid_utf8_does_not_make_a_dirty_tree_read_clean() {
    // Integrity #1 of the review, as a regression test at the level it actually failed. `git diff`
    // emits repository CONTENT, and content is bytes: a file in Windows-1252 produces a diff that is
    // not valid UTF-8. The string-only reader behind this path answered `Ok("")` for it — the
    // command had exited 0, and `read_to_string` leaves its buffer untouched on a decode error — so
    // the fingerprint was taken over nothing and the tree could report `dirty: false`.
    //
    // `0xE4 0xDF` is `äß` in Windows-1252 and is not a valid UTF-8 sequence. git does not call this
    // file binary: it contains no NUL.
    let b = Bench::new();
    std::fs::write(b.root().join("latin.txt"), b"Gr\xe4\xdfe\n").unwrap();
    git(b.root(), &["add", "-A"]);
    commit(b.root(), "a file in Windows-1252");
    let clean = b.anchor_now();
    assert!(!clean.dirty, "committed, so nothing is outstanding");

    // Now EDIT it, still in Windows-1252. The diff git prints for this is not text.
    std::fs::write(b.root().join("latin.txt"), b"Gr\xe4\xdfer\n").unwrap();
    let dirty = b.anchor_now();
    assert!(
        dirty.dirty,
        "the tree IS dirty and must say so — the field the design stakes its honesty on"
    );
    assert_ne!(
        dirty.fingerprint, clean.fingerprint,
        "and the change must move the fingerprint, or drift is blind to every non-UTF-8 file"
    );

    // The sharper half: two DIFFERENT edits of one file produce the same porcelain line, so only the
    // diff tells them apart. With the diff read collapsed to nothing these would compare equal and
    // `drift_from` would report calm over a changed tree.
    std::fs::write(b.root().join("latin.txt"), b"Gr\xe4\xdfeste\n").unwrap();
    let other = b.anchor_now();
    assert_ne!(other.fingerprint, dirty.fingerprint);
    assert!(other.drift_from(&dirty).uncommitted_now);
}

#[test]
fn a_member_gets_a_fresh_anchor_on_its_second_turn() {
    // Code Quality #1 of the review: `supervisor_hand_out_next_turn` is a FIFTH handover site — the
    // supervisor forwards a member its next turn and resumes that member's live session — and it was
    // the one site the anchor missed, because the stamp had been keyed on "wherever a return address
    // is stamped" and the supervisor is not a session. The miss was worse than an absence: the
    // status read reports the newest ANCHORED message, so a member on turn two kept showing turn
    // one's tree as though it were the state it had just been handed.
    let b = Bench::new();
    let board = b.commission(T0, "coding", "review this");
    let turn_one: Vec<Anchor> = b
        .children_of(&board)
        .into_iter()
        .map(|t| t.working_copy.expect("turn one is anchored"))
        .collect();
    assert_eq!(turn_one.len(), 2, "two members");

    // The world moves between the two turns — otherwise this test cannot tell a fresh anchor from a
    // stale one, which is exactly the bug.
    std::fs::write(b.root().join("src.txt"), "after the first round\n").unwrap();
    git(b.root(), &["add", "-A"]);
    commit(b.root(), "second");
    let now = b.anchor_now();
    assert_ne!(now.commit, turn_one[0].commit, "the fixture really moved");

    // The requester follows up on the BOARD thread, which is what makes the supervisor hand out the
    // next turn to every member.
    b.reply(T1, "carsten", &board, "one more pass please");

    for row in b.children_of(&board) {
        let a = row
            .working_copy
            .unwrap_or_else(|| panic!("member thread {} lost its anchor", row.thread_id));
        assert_eq!(
            a.commit, now.commit,
            "turn two must carry the tree it was handed, not the one turn one was"
        );
    }
}

#[test]
fn a_resume_records_the_tree_the_continued_session_is_handed() {
    // Test Quality #1 of the review: `role_resume` is one of the five stamped sites and had no test
    // of its own, so dropping that one call would have left the whole suite green. No surface
    // reaches this verb any more (see its own doc), which is why the test drives it directly.
    let b = Bench::new();
    let receipt = b.commission_receipt(T0, "coder", "build it");
    let session = receipt.session.expect("a persona summon mints a session");

    std::fs::write(b.root().join("src.txt"), "work in progress\n").unwrap();
    let expected = b.anchor_now();

    b.with(T1, "carsten", |ctx, store| {
        orchestration::role_resume(
            ctx,
            store,
            RoleResumeRequest {
                session: &session,
                body: "and one more thing",
                channel: None,
                kind: MessageKind::Task,
                priority: Priority::Normal,
                disposition: Disposition::InTurn,
                thread: Some(&receipt.thread_id),
                refs: Refs::default(),
                model: None,
            },
        )
        .expect("resume");
    });

    let messages = b.messages(&receipt.thread_id);
    let a = anchor_of(messages.last().unwrap());
    assert_eq!(a.commit, expected.commit);
    assert!(
        a.dirty,
        "the tree the continued session is handed is dirty, and says so"
    );
    assert_eq!(a.fingerprint, expected.fingerprint);
}

#[test]
fn the_json_a_reader_branches_on_carries_exactly_the_four_fields() {
    // Test Quality #5 of the review. Every other assertion here compares Rust values, so the WIRE
    // shape — what an embedding app and `nxc --json` actually read — was never pinned. The golden
    // corpus structurally cannot pin it either: its worker names no working copy, so no `nxc`
    // invocation in it can produce an anchor at all.
    let b = Bench::new();
    let thread = b.commission(T0, "coder", "build it");
    let value = serde_json::to_value(&b.messages(&thread)[0].refs).expect("refs serialize");
    let anchor = value
        .get("working_copy")
        .unwrap_or_else(|| panic!("no working_copy key in {value}"));
    let obj = anchor.as_object().expect("an object, not a flat string");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["branch", "commit", "dirty", "fingerprint"]);
    assert!(obj["commit"].is_string() && obj["dirty"].is_boolean());
}

#[test]
fn an_anchor_this_build_cannot_parse_is_skipped_and_does_not_fail_the_read() {
    // Test Quality #4 of the review: the forward-compatibility branch in
    // `ChatStore::working_copy_anchors_sql`. A newer peer may write a shape this build does not
    // know, and a read that failed the whole report over one unfamiliar row would make `nxc status`
    // unusable on a workspace that syncs with a newer device. It cannot be reached through the API —
    // `Refs::working_copy` is typed — so the row is written the way a newer peer's bytes arrive.
    let b = Bench::new();
    let first = b.commission(T0, "coder", "build it");
    let second = b.commission(T1, "reviewer", "check it");
    b.with(T2, "carsten", |_ctx, store| {
        let n = store
            .connection()
            .execute(
                "UPDATE messages SET refs = json_set(refs, '$.working_copy', \
                 json('{\"shape\":\"from a newer peer\"}')) WHERE thread_id = ?1",
                rusqlite::params![&first],
            )
            .expect("rewrite the stored anchor");
        assert_eq!(n, 1, "exactly the one message of the first thread");
    });

    let report = b.status();
    let rows: Vec<&facade::StatusThread> = report
        .operations
        .iter()
        .flat_map(|o| &o.threads)
        .filter(|t| t.thread_id == first || t.thread_id == second)
        .collect();
    assert_eq!(rows.len(), 2, "both threads are still reported");
    let unreadable = rows.iter().find(|t| t.thread_id == first).unwrap();
    let readable = rows.iter().find(|t| t.thread_id == second).unwrap();
    assert!(
        unreadable.working_copy.is_none(),
        "the row this reader cannot parse reads as no anchor"
    );
    assert!(
        readable.working_copy.is_some(),
        "and it takes nothing else down with it"
    );
}

#[test]
fn the_command_line_records_an_anchor_too_and_that_is_the_seam_that_actually_failed() {
    // Test Quality #2 of the review, and the most important test in this file for the bug it
    // guards. Everything above drives the ENGINE seam with its own worker — which is precisely the
    // shape of coverage that let nxf 6j6v.x5sr ship: `cli.rs`'s `LazyWorker` wraps the worker every
    // command-line call resolves, it did not forward `working_copy`, and so the shipped `nxc`
    // reported a host with no working copy at all. Every engine-seam test stayed green throughout.
    //
    // So this one goes through the real binary, with the real `SidecarWorker` behind the real
    // wrapper. `NXC_SIDECAR` names a file that does not exist: the worker still builds and still
    // answers WHERE (which is the question under test), the spawn then fails and is reported on the
    // receipt's `warnings` — the message is durably posted either way, which is the contract
    // `send --to` documents.
    nxs_test_support::assert_multicall_binary_fresh();

    let b = Bench::new();
    let head = git(b.root(), &["rev-parse", "HEAD"]);
    let nxc = || {
        let mut c = nxs_test_support::cargo_bin("nxc");
        c.current_dir(b.root())
            .env_remove("NXC_SESSION")
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", "local")
            .env("NXC_WORKER", "sidecar")
            .env("NXC_SIDECAR", b.root().join("no-such-sidecar.mjs"))
            .env("NXC_TIMER", "dry");
        for (key, value) in nxs_test_support::pinned_home_env() {
            c.env(key, value);
        }
        c
    };
    let json = |cmd: &mut nxs_test_support::Command| -> serde_json::Value {
        let out = cmd.output().expect("run nxc");
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "not json ({e}): {}\nstderr: {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        })
    };

    let receipt = json(nxc().args(["--json", "send", "--to", "coder", "--no-ref", "build it"]));
    let thread = receipt["thread_id"].as_str().expect("a thread").to_string();

    let board = json(nxc().args(["--json", "threads", "show", &thread]));
    let anchor = &board["messages"][0]["refs"]["working_copy"];
    assert_eq!(
        anchor["commit"].as_str(),
        Some(head.as_str()),
        "the command line must record where the tree stood; without the wrapper forwarding \
         `working_copy` this key is absent entirely — which is what shipped. Receipt: {receipt}"
    );
    assert_eq!(anchor["branch"].as_str(), Some("release/1.2"));

    // …and the host-level answer the same wrapper decides, on the read that explains an empty
    // column (`StatusReport::worker_names_a_working_copy`).
    let status = json(nxc().args(["--json", "status", "--all"]));
    assert_eq!(status["worker_names_a_working_copy"].as_bool(), Some(true));
}
