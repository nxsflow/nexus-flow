//! **Parking the work of a stranded escalation** (nxf 6j6v.de9s) — the git half: how an operation's
//! work is put somewhere safe so the working copy can be handed on, and how it is found again.
//!
//! WHO calls this and WHEN is [`crate::orchestration`]'s (`park_the_stranded_holder`, on the tick);
//! this module owns the mechanics, following the pattern [`crate::working_tree`] and
//! [`crate::consolidation_claim`] already set — the decision lives at the seam that makes it, the
//! machinery lives in a module of its own.
//!
//! ## The state this exists for, measured
//!
//! `6j6v.fabb`: an unanswered escalation held the working copy. Ten threads of a FINISHED operation
//! held it, each with `complete: true` and `outstanding: []`, the operation itself `awaiting_human`.
//! A new round stood at `working tree: waiting (#1)` with no session, no process and no bound. The
//! documented way out (answer the escalation) demonstrably did not happen.
//!
//! [`crate::working_tree`]'s release path is why, and it is not a defect there: question (3) of
//! `release_working_tree_if_scope_is_done` refuses to let a handed-back area go, because an
//! escalating reply discharges the sender's turn exactly like a finished one while the task is
//! still mid-flight. That refusal is correct and stays. What was missing is the other end — nothing
//! ever decided that the wait had gone on long enough.
//!
//! ## The rule (owner, 2026-09-04)
//!
//! The holder escalates and waits for a human. If — AND ONLY IF — another operation is standing in
//! the queue, a deadline of [`PARK_AFTER`] runs. If it passes without an answer, the work is
//! committed onto a branch, the branch is recorded against the OPERATION, the tree goes back to its
//! base and the working copy is handed on. **The timer starts at CONTENTION, not at the
//! escalation** — with nothing waiting, nothing is parked, and that is what keeps this off the
//! ordinary path.
//!
//! ## No model call between a standing queue and the release
//!
//! The whole point of this path is to unstick a queue, so nothing on it may depend on a thing that
//! could itself be stuck. [`park_branch_name`] and [`park_commit_message`] are therefore pure
//! functions of the operation key and the instant: deterministic, unit-tested, and computable with
//! no network, no model and no second process. An agent may prettify the name afterwards, once the
//! copy is already free — the name is cosmetic, the commit is the safety.
//!
//! ## Why the engine composes the git commands rather than a worker doing it
//!
//! [`crate::worker::Worker::working_copy`] answers WHERE, because that is a fact about the machine
//! sessions run on and the worker is the only thing that knows it. Everything after that is policy —
//! which commands, in which order, and what a non-zero exit means — and policy belongs here for the
//! reason the sidecar's own design gives: the executor is the dumb performer of a spec it does not
//! author. A host that cannot name a working copy is simply never parked in, and says so.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use rusqlite::{params, OptionalExtension};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::error::Result;
use crate::store::ChatStore;

/// **How long an escalation may hold the working copy once ANOTHER operation is waiting for it**
/// (owner, 2026-09-04: thirty minutes).
///
/// Read by `orchestration::park_deadline` and resolved through `facade::resolve_instant`, which is
/// the same duration vocabulary [`crate::working_tree::WORKING_TREE_LEASE_BOUND`] and every declared
/// `timeout:` are written in.
///
/// **It is a CONTENTION bound, not a patience bound**, and that is the whole reason it can be this
/// short. Nothing measures it while the queue is empty: an escalation that nobody is waiting behind
/// runs to the lease's own backstop exactly as it always did, and a human who answers in an hour
/// loses nothing. The clock only starts once a second operation is standing still because of this
/// one, and thirty minutes is then the answer to "how long may one waiting operation pay for
/// another's unanswered question" — long enough that a human at a desk normally answers first, short
/// enough that a night is not lost to it.
pub const PARK_AFTER: &str = "30m";

/// The prefix of every branch this module CREATES — the shape `nxs/park/<operation>/<instant>`.
///
/// A namespace of its own so a parked branch is recognisable at a glance and by a glob (`git branch
/// --list 'nxs/park/*'`), and so nothing this engine creates can collide with a branch a person
/// named. Note what it does NOT cover: a park that finds the operation already working on a branch
/// of its own uses THAT branch, so "parked" is not the same set as "matches this prefix". The
/// authority on what is parked is the `parked_work` table, never the branch name.
pub const PARK_BRANCH_PREFIX: &str = "nxs/park";

/// **Where an operation stood when it first took the working copy** (correction 2 of nxf 6j6v.de9s:
/// *"DIE BASIS IST NICHT `main`"*).
///
/// Recorded once per operation, on its FIRST acquire of the lease, and never again — the point is
/// the branch the interrupted work STARTED from, which is exactly the thing a later acquire can no
/// longer see. The interrupted work may have begun on a feature branch, and the next holder may
/// need a third base; without this, resuming would not know where back is, and the park would have
/// to guess `main`.
///
/// `branch` is empty when the operation started on a detached HEAD. That is not an error and is not
/// normalized away: a detached HEAD is a real state a working copy can be in, and `commit` still
/// names the point exactly. Everything that reads this treats an empty `branch` as "go back to the
/// commit, detached", which is what the tree was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    /// The branch the operation started on, or empty for a detached HEAD.
    pub branch: String,
    /// The commit that branch (or that detached HEAD) pointed at.
    pub commit: String,
}

/// What a completed park produced — everything the resume needs and everything the receipt says.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Parked {
    /// The branch the work now sits on. Either one this park created (see [`PARK_BRANCH_PREFIX`]) or
    /// the operation's own, when it already had one.
    pub branch: String,
    /// The commit the branch pointed at when the working copy was handed on. **This is the anchor
    /// the movement check compares against** — see [`WorkingCopy::movement`].
    pub commit: String,
    /// [`Base::branch`] as it was recorded for this operation, carried onto the row so a resume does
    /// not have to re-derive a base the tree has since left.
    pub base_branch: String,
    /// [`Base::commit`] — the OTHER half of the movement check, and the half the first draft of this
    /// design left out (correction 4).
    pub base_commit: String,
    /// Whether this park had to create the branch. `false` means the operation already had one and
    /// the work was committed onto it, which is the case the owner's rule names ("einen Zweig
    /// anlegen nur, falls der Vorgang noch keinen hat").
    pub created_branch: bool,
    /// Whether anything was actually committed. `false` is a clean tree — the park still happened
    /// (the copy was handed on, and `commit` still names where the operation stood), there was
    /// simply nothing uncommitted to save.
    pub committed: bool,
}

/// **Why a park did NOT happen** — and, through [`Self::is_permanent`], what happens to the working
/// copy instead.
///
/// Every refusal here declines to act rather than acting approximately: committing a half-finished
/// merge or checking out over somebody's uncommitted work is silent and destructive, so nothing on
/// this path does either. What a refusal means for the CLAIM is decided by one rule, applied in one
/// place (`orchestration::park_and_hand_on`, owner decision of 2026-09-17, nxf 6j6v.8bv9):
///
/// - **A refusal that can pass holds the claim.** A tree mid-merge is finished by somebody, a git
///   command that failed may succeed on the next attempt. The claim stays, `nxc status` shows the
///   refusal on the holding operation, and the background service retries on every tick — holding
///   the copy a little longer is benign and visible, and the park goes through the moment it can.
/// - **A refusal that cannot pass hands the copy on UNPARKED.** A host that names no working copy,
///   a directory that is not a repository, an operation that never had a base recorded — no tick
///   will ever answer differently, so a claim held for them is a queue held for ever. The copy goes
///   on with a loud, named finding, and whatever was uncommitted stays in the tree for the next
///   holder to find.
///
/// The string every variant carries is the refusal's own account of WHY — and, where there is
/// something to fix, what. It deliberately says nothing about the claim: that is the rule's to
/// decide, and a sentence here that promised either outcome would be wrong for half the callers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "refusal", content = "detail")]
pub enum ParkRefusal {
    /// The worker cannot name a working copy ([`crate::worker::Worker::working_copy`] answered
    /// `None`) — a host that does not run sessions in a directory, or one that has not said so.
    /// Permanent: it is a property of the host.
    NoWorkingCopy(String),
    /// There is a directory and it is not a git repository. Nothing here can park work that git does
    /// not track. Permanent: nothing on this path makes a repository.
    NotARepository(String),
    /// **The tree is mid-rebase, mid-merge, mid-cherry-pick, mid-revert or mid-bisect** — decision
    /// (3) of nxf 6j6v.de9s. See [`WorkingCopy::mid_sequence`] for the whole argument. Transient: a
    /// person finishes or aborts it, and the next tick parks.
    MidSequence(String),
    /// A git command failed. The string is the command and what git said, verbatim and capped — or,
    /// for a park that got as far as its commit, what is safe and what is left. Transient: a lock
    /// goes away, a disk gets space, and a retry from where this stopped is idempotent.
    Git(String),
    /// **No base branch was ever recorded for the operation** (nxf 6j6v.8bv9 made this a variant; it
    /// was an inline sentence before). The base is recorded on the operation's first acquire of the
    /// lease, while a working copy could be read; without it there is nowhere to put the tree back
    /// to after a park. Permanent: the first acquire is long past, and a base recorded NOW would name
    /// the tree the operation has already moved, not the one it started from.
    NoBaseRecorded(String),
}

impl ParkRefusal {
    /// The one-line prose every caller prints and puts on a receipt — the refusal's own words, with
    /// no caller left to invent its own.
    pub fn detail(&self) -> &str {
        match self {
            ParkRefusal::NoWorkingCopy(s)
            | ParkRefusal::NotARepository(s)
            | ParkRefusal::MidSequence(s)
            | ParkRefusal::Git(s)
            | ParkRefusal::NoBaseRecorded(s) => s,
        }
    }

    /// **Can waiting change this answer?** `true` means no — the working copy is handed on without
    /// parking; `false` means yes — the claim is held and the park retried. See the type's own doc
    /// for the rule and why it splits where it does.
    ///
    /// An EXHAUSTIVE match with no `_` arm, on purpose: a new refusal is a compile error here, so its
    /// author has to decide which half it belongs to rather than inherit whichever a wildcard picked.
    pub fn is_permanent(&self) -> bool {
        match self {
            ParkRefusal::NoWorkingCopy(_)
            | ParkRefusal::NotARepository(_)
            | ParkRefusal::NoBaseRecorded(_) => true,
            ParkRefusal::MidSequence(_) | ParkRefusal::Git(_) => false,
        }
    }

    /// The refusal's name as `--json` spells it — the serde tag, `snake_case` (`mid_sequence`,
    /// `no_base_recorded`, …). What [`ChatStore::note_park_refusal`] stores and what `nxc status`
    /// prints in its `park refused (<name>)` line, so the terminal and the JSON never disagree about
    /// which refusal this is. `tests::every_refusal_names_itself_the_way_json_does` holds the two
    /// spellings together.
    pub fn kind(&self) -> &'static str {
        match self {
            ParkRefusal::NoWorkingCopy(_) => "no_working_copy",
            ParkRefusal::NotARepository(_) => "not_a_repository",
            ParkRefusal::MidSequence(_) => "mid_sequence",
            ParkRefusal::Git(_) => "git",
            ParkRefusal::NoBaseRecorded(_) => "no_base_recorded",
        }
    }
}

/// **A park that was refused for a reason that can pass, and is being retried** (nxf 6j6v.8bv9) —
/// one row of `park_refusals`, as `nxc status` shows it on the holding operation.
///
/// Only the TRANSIENT half of [`ParkRefusal`] ever produces one: a permanent refusal hands the
/// working copy on, and a copy that has moved on has nothing left to retry. The row lives no longer
/// than what it is about — the claim, and the occasion that wanted the copy — see
/// [`ChatStore::note_park_refusal`] for every place it goes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ParkRefusalNote {
    /// Which refusal — [`ParkRefusal::kind`]'s spelling (`mid_sequence`, `git`).
    pub refusal: String,
    /// The refusal's own words, which name what to fix ([`ParkRefusal::detail`]) — the most recent
    /// attempt's, so a reader sees the state as it is now rather than as it first was.
    pub detail: String,
    /// **WHICH TROUBLE is trying to park** — [`crate::orchestration::ParkOccasion::as_str`]'s
    /// spelling (`stranded_escalation`, `availability_boundary`). It is what makes "retrying" true:
    /// the tick's park step clears this row when that occasion stops standing, so the mark on
    /// `nxc status` is only ever there for a park somebody is actually attempting again.
    pub occasion: String,
    /// When the park was FIRST refused — what "retrying since" means. A retry that is refused again
    /// keeps this instant, whichever occasion made it; only the row going takes it with it — a park
    /// that finally goes through, a claim that ends, or the occasion above ceasing to stand.
    pub since: String,
    /// When it was last tried — the most recent refusal, so a reader can tell a retry that is still
    /// running from one that stopped being attempted.
    pub last_tried: String,
}

/// **What moved while the work was parked** (correction 4 of nxf 6j6v.de9s).
///
/// The draft this corrects wanted one check: is our park commit still the newest on its branch? That
/// catches somebody committing onto OUR branch and misses the commoner case — our branch untouched
/// while the BASE ran on. The park commit is then still the tip of its own branch, the check reports
/// calm, and the work is resumed on a stale foundation. So the comparison names both, and both
/// halves are reported separately because they call for different things: a moved branch is somebody
/// else's commit in your history, a moved base is a rebase you have not done yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Movement {
    /// The parked branch's tip now, when it is not the commit that was parked. `None` — it is
    /// unchanged, or the branch is gone (which is [`Self::branch_gone`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_now: Option<String>,
    /// The base branch's tip now, when it is not the commit recorded at acquire time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_now: Option<String>,
    /// The parked branch cannot be resolved at all any more — deleted, or renamed. Reported rather
    /// than folded into "moved": nothing can be resumed onto it, and saying "it moved" would send a
    /// reader looking for commits that are not there.
    pub branch_gone: bool,
    /// The base branch cannot be resolved any more.
    pub base_gone: bool,
}

impl Movement {
    /// Did anything move at all? The one question the deterministic text branches on.
    pub fn any(&self) -> bool {
        self.branch_now.is_some() || self.base_now.is_some() || self.branch_gone || self.base_gone
    }
}

/// **The deterministic branch name for a park** — pure, so it can be computed on the path whose
/// whole purpose is to unstick a queue (correction 1).
///
/// `nxs/park/<operation>/<instant>`: the operation makes it findable, the instant makes it unique
/// when one operation is parked more than once, and the [`PARK_BRANCH_PREFIX`] namespace makes it
/// recognisable. The instant is compacted to `YYYYMMDDTHHMMSSZ` because a git ref may not contain
/// `:`, which every RFC3339 instant does.
///
/// **Everything outside `[A-Za-z0-9._-]` becomes `-`.** A scope key is `run:<id>` / `thread:<id>` /
/// `session:<id>` ([`crate::working_tree::WorkScope::key`]), so in practice this replaces the one
/// colon — but the ids themselves are minted elsewhere and a ref name that git refuses would turn a
/// park into a `Git` refusal at the worst possible moment. Sanitizing is cheaper than trusting.
/// A leading `-` or `.` would also be refused by git, so the sanitized segment is prefixed when it
/// starts with either.
pub fn park_branch_name(scope_key: &str, now: &str) -> String {
    format!(
        "{PARK_BRANCH_PREFIX}/{}/{}",
        ref_segment(scope_key),
        compact_instant(now)
    )
}

/// **The deterministic commit message for a park** — pure, for [`park_branch_name`]'s reason.
///
/// Three lines and every one of them is what a person reading `git log` a week later needs: that
/// nxs made this commit and not a person, which operation it belongs to, and that nothing in it was
/// reviewed or finished. It deliberately does not try to describe the WORK — that is what a model
/// call would be for, and a model call is exactly what may not stand on this path.
///
/// **The middle line is parameterized by `occasion`** (fix round 3 of nxf 6j6v.b9nf's review, Code
/// Quality #7 (c)). It used to always say "waiting for an answer to an escalation", which was
/// simply wrong for every occasion added after the first — an availability boundary, a lease past
/// its bound, a holder found dead inside it, and now a round that was WITHDRAWN — and this commit
/// lands in the user's own git history, where a wrong reason is not a bug report anybody files, it
/// is a fact a reader believes a week later. The `match` is exhaustive on purpose: a new occasion
/// that forgets to extend it fails to compile rather than shipping a park commit that lies about
/// itself. [`crate::orchestration::ParkOccasion::as_str`] is not reused here — that spelling is for
/// `--json`, this is a sentence for a person, and the two may say the same fact in different words
/// without drifting, because both are derived from the one enum this function matches on.
pub fn park_commit_message(
    scope_key: &str,
    now: &str,
    occasion: crate::orchestration::ParkOccasion,
) -> String {
    use crate::orchestration::ParkOccasion;
    let why = match occasion {
        ParkOccasion::StrandedEscalation => {
            "The working copy was handed on while this operation was waiting for an answer to an \
             escalation."
        }
        ParkOccasion::AvailabilityBoundary => {
            "The working copy was handed on while this operation was stopped at an availability \
             boundary, waiting for the model or provider to come back."
        }
        ParkOccasion::PastItsBound => {
            "The working copy was handed on because this operation's lease ran out with nothing \
             left running in its claim area."
        }
        ParkOccasion::DiedInsideItsBound => {
            "The working copy was handed on because this operation's process died before its \
             lease ran out."
        }
        ParkOccasion::Withdrawn => {
            "The working copy was handed on because this operation's running round was withdrawn \
             and its session was asked to stop."
        }
    };
    format!(
        "nxs: parked {scope_key} at {now}\n\n\
         {why}\n\
         Nothing here was finished or reviewed — it is the tree exactly as it stood.\n"
    )
}

/// One path segment of a git ref, from arbitrary text.
fn ref_segment(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if out.is_empty() {
        out.push('x');
    }
    if out.starts_with('-') || out.starts_with('.') {
        out.insert(0, 'x');
    }
    out
}

/// `2026-09-05T09:01:51Z` → `20260905T090151Z`, the colon-free form a git ref admits.
///
/// An instant this cannot parse is passed through [`ref_segment`] instead of failing: the name is
/// cosmetic, and a park that refused because a timestamp had an unexpected shape would sacrifice the
/// commit — the thing that actually matters — for the label on it.
fn compact_instant(now: &str) -> String {
    match OffsetDateTime::parse(now, &Rfc3339) {
        Ok(t) => {
            let t = t.to_offset(time::UtcOffset::UTC);
            format!(
                "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
                t.year(),
                u8::from(t.month()),
                t.day(),
                t.hour(),
                t.minute(),
                t.second()
            )
        }
        Err(_) => ref_segment(now),
    }
}

/// **How long any one git command of a park gets before it is killed** — a breach detector, not a
/// budget, exactly as [`crate::worker::PRECONDITION_BOUND`] is at the seam next door and for a
/// sharper reason: this runs inside the engine's held store mutex, on the unattended tick, and a git
/// call that never returns would wedge the whole workspace handle — permanently, and while trying to
/// end a wedge.
///
/// Every command here is local and deterministic (`rev-parse`, `status`, `add`, `commit`,
/// `checkout`) and answers in milliseconds to a second or two on any tree small enough to work in.
/// Sixty seconds is far past that and far short of forever; a `commit` of a very large tree on a
/// slow disk is the one case that could approach it.
///
/// **What a breach COSTS, corrected** (independent review of PR #435, Integrity #2). This doc used
/// to say the park is then "a named refusal with the claim left standing, not a corruption". The
/// refusal is named and the claim does stand — but a `git add`/`commit`/`checkout` killed mid-flight
/// can leave `index.lock` behind, and that file fails every subsequent git command in the working
/// copy, for everyone, until it is removed. Calling that "not a corruption" was true of the
/// repository and false of the working copy. [`WorkingCopy::git`] now clears the lock it can prove
/// it created and names one it cannot.
const GIT_BOUND: Duration = Duration::from_secs(60);

/// How long the pipe readers get to hand over what they collected once the child is reaped —
/// `run_precondition_within`'s `OUTPUT_GRACE`, for its reason: a healthy command's pipes are already
/// closed, and this is only for the pathological case where a grandchild still holds one open.
const OUTPUT_GRACE: Duration = Duration::from_millis(200);

/// How much of a git command's output travels with a refusal — [`crate::precondition::OUTPUT_CAP`]'s
/// reasoning, applied to the same kind of output at a different seam: a refusal rides a receipt an
/// agent reads, and `git checkout` over a thousand conflicting paths would bury it.
const GIT_OUTPUT_CAP: usize = 2000;

/// **The one working copy this device's sessions run in**, and every git operation the park needs on
/// it.
///
/// A struct rather than free functions taking a path, so that a caller cannot half-park: one value
/// names one directory and every command below runs in it. Constructed from
/// [`crate::worker::Worker::working_copy`] and from tests, and from nothing else.
pub struct WorkingCopy {
    root: PathBuf,
}

impl WorkingCopy {
    /// The working copy rooted at `root`. Nothing is checked here — [`Self::assert_repository`] is
    /// the check, and it is a step of the park rather than of construction so its refusal can be
    /// reported like every other.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Where it is — for the prose of a refusal, and for a test to assert against.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// **Where `index.lock` would be for this working copy**, resolved WITHOUT running git.
    ///
    /// It has to be filesystem-only, because its one caller is [`Self::git`] and a resolution that
    /// ran git would recurse. `.git` is a directory in an ordinary clone and a FILE holding
    /// `gitdir: <path>` in a linked worktree — both are handled, because a worktree is exactly the
    /// shape this module's own env-stripping is careful about, and a lock resolved to the wrong
    /// directory would be worse than none.
    ///
    /// `None` for anything else (no `.git` at all, an unreadable pointer file): the caller then
    /// simply says nothing about locks, which is what it should do when it cannot tell.
    fn index_lock(&self) -> Option<PathBuf> {
        let dot_git = self.root.join(".git");
        let meta = std::fs::metadata(&dot_git).ok()?;
        let git_dir = if meta.is_dir() {
            dot_git
        } else {
            let pointer = std::fs::read_to_string(&dot_git).ok()?;
            let named = Path::new(pointer.lines().next()?.strip_prefix("gitdir:")?.trim());
            if named.is_absolute() {
                named.to_path_buf()
            } else {
                self.root.join(named)
            }
        };
        Some(git_dir.join("index.lock"))
    }

    /// Run one git command in this working copy and hand back its trimmed stdout.
    ///
    /// **`-c` settings that make the run deterministic and unattended**, and each is here because
    /// the alternative is a park that hangs or refuses at the worst moment:
    ///
    /// * `core.hooksPath=` (empty) — a project hook is arbitrary code that may itself want the
    ///   working copy this park is trying to free, and `pre-commit` refusing would lose the work
    ///   entirely. The park is not a contribution being offered for review; it is a snapshot.
    /// * `commit.gpgsign=false` — a signing key with a passphrase turns a commit into a prompt with
    ///   nobody in front of it.
    /// * `GIT_TERMINAL_PROMPT=0`, `GIT_OPTIONAL_LOCKS=0` and an empty `GIT_ASKPASS`/`SSH_ASKPASS` —
    ///   nothing on this path may wait for a human, which is the state it exists to end.
    ///
    /// Everything else is the repository's own configuration, deliberately: the identity a commit is
    /// authored under is the project's business, and [`Self::commit_identity`] only supplies one
    /// when the repository has none at all.
    ///
    /// **`pub(crate)` for a second reader, and for no other** (nxf 6j6v.2af2):
    /// [`crate::anchor::WorkingCopy::anchor`] reads where the tree stands on every handover, and it
    /// must do so through THIS runner rather than its own — the bound above, the neutralised hooks
    /// and the stripped discovery variables are exactly as load-bearing for a read on the ordinary
    /// path as for a commit on the unsticking one. A second spawn helper beside it is how one of the
    /// two would come to lack a protection the other has.
    pub(crate) fn git(&self, args: &[&str]) -> std::result::Result<String, String> {
        self.git_bytes(args)
            .map(|out| String::from_utf8_lossy(&out).trim().to_string())
    }

    /// **The same run, with the output as BYTES** — what [`Self::git`] is a lossy view of, and what
    /// a caller whose payload is arbitrary repository content has to read instead (nxf 6j6v.2af2,
    /// independent review of PR #474, Integrity #1).
    ///
    /// Git output is bytes, not text: a tracked file in Windows-1252 or Shift-JIS makes
    /// `git diff HEAD` emit a stream that is not valid UTF-8, and `core.quotePath=false` does the
    /// same for `git status --porcelain`. `read_to_string` REFUSES such a stream and — this is the
    /// part that bites — leaves its buffer untouched, so the old string-only reader handed back
    /// `Ok("")` for a command that had exited 0 with a page of output. Every caller before this item
    /// read a short SHA, a branch name or a config value, where that could not happen; the anchor's
    /// two reads are the first whose successful CONTENT is load-bearing, and there an empty answer
    /// is not a harmless blank — it is a dirty tree reported as clean.
    ///
    /// So the bytes travel unexamined and whoever wants text says so. Nothing decodes anything here.
    pub(crate) fn git_bytes(&self, args: &[&str]) -> std::result::Result<Vec<u8>, String> {
        let named = || format!("`git {}`", args.join(" "));
        let mut cmd = Command::new("git");
        cmd.args([
            "-c",
            "core.hooksPath=",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "gc.auto=0",
        ])
        .args(args)
        .current_dir(&self.root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("GIT_PAGER", "cat")
        // **Anything that could point git somewhere other than `current_dir` is removed.** These
        // six are exactly the variables that override the repository discovery a working directory
        // would otherwise decide, and a session — or a git hook, or a test harness — that exported
        // one of them would make this commit an operation's work into a DIFFERENT repository.
        // `GIT_COMMON_DIR` and `GIT_CEILING_DIRECTORIES` joined the list on the independent review
        // of PR #435 (Integrity #4): the first is precisely the linked-worktree knob this module's
        // own `index_lock` is careful about, so leaving it out while claiming worktree-correctness
        // was an inconsistency rather than a judgement.
        //
        // **`GIT_CONFIG`/`GIT_CONFIG_GLOBAL` are deliberately NOT removed**, and the same review
        // listed them. They redirect CONFIGURATION, not discovery, and configuration is the
        // project's business: this park is a commit in somebody's repository and should be authored
        // under whatever identity that repository resolves. The two settings that would actually be
        // dangerous — hooks and signing — are neutralised by name with `-c` above, which is
        // stronger than stripping an environment variable that may not have been set anyway.
        //
        // The rest of the environment is left alone deliberately: git needs `HOME` for the project's
        // own identity and `PATH` for its helpers, and `env_clear()` would take both.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_CEILING_DIRECTORIES")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        // **Whether a lock was ALREADY there before this call** (independent review of PR #435,
        // Integrity #2). It is read here, before the spawn, because it is the only thing that makes
        // the cleanup below provable rather than presumptuous: a lock that appears while our own
        // child is running and is still there after we killed it is OURS. One `stat` per git call,
        // and it buys the difference between removing our own litter and removing a lock somebody
        // else's git — or a person's — is legitimately holding in this shared working copy.
        let lock = self.index_lock();
        let lock_was_there = lock.as_ref().is_some_and(|p| p.exists());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not run {} in {}: {e}", named(), self.root.display()))?;
        // Both pipes drained OFF-THREAD, before the wait — `crate::worker::SidecarWorker::
        // run_precondition_within`'s reasoning, at the same kind of seam. `git checkout` over a
        // thousand conflicting paths prints more than a pipe buffer holds, and a wait that reads
        // only after exit would be waiting for an exit that cannot happen until it reads. Over a
        // channel rather than a `JoinHandle` for that function's other reason: a reader parked on a
        // descriptor a grandchild still holds open never returns, and this runs inside the engine's
        // held store mutex.
        //
        // **`read_to_end` and not `read_to_string`** (independent review of PR #474, Integrity #1):
        // see [`Self::git_bytes`] for the whole argument. The short version is that the string form
        // discards a decode failure into an EMPTY BUFFER on a command that succeeded, which is the
        // one shape of wrong answer this runner must not produce.
        let drain = |handle: Option<Box<dyn std::io::Read + Send>>| {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut buf: Vec<u8> = Vec::new();
                if let Some(mut h) = handle {
                    let _ = h.read_to_end(&mut buf);
                }
                let _ = tx.send(buf);
            });
            rx
        };
        let out = drain(
            child
                .stdout
                .take()
                .map(|h| Box::new(h) as Box<dyn std::io::Read + Send>),
        );
        let err = drain(
            child
                .stderr
                .take()
                .map(|h| Box::new(h) as Box<dyn std::io::Read + Send>),
        );
        let deadline = std::time::Instant::now() + GIT_BOUND;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Err(e) => return Err(format!("waiting for {} failed: {e}", named())),
                Ok(None) => {}
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                // Reaped before the lock is looked at, deliberately: a child that is still dying can
                // still be holding the file, and a check that raced it would either miss the lock or
                // remove one that was about to be released properly.
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        // **The success arm keeps its `Result`, because a grace that ran out is not an empty
        // answer** (independent review of PR #474, Integrity #2). This used to be
        // `unwrap_or_default()` on both, so a reader thread that had not handed over within
        // [`OUTPUT_GRACE`] turned a command that exited 0 into `Ok("")` — indistinguishable from a
        // command that genuinely printed nothing. For a `rev-parse` that is an empty commit id; for
        // the anchor's `status --porcelain` it is a dirty tree read as clean. Rare by construction
        // (the child cannot exit before its pipe has been drained past the buffer, so what is left
        // at exit is at most a pipe's worth), which is exactly why it must not be silent: nobody
        // would ever see it happen.
        let stdout = out.recv_timeout(OUTPUT_GRACE);
        let stderr = String::from_utf8_lossy(&err.recv_timeout(OUTPUT_GRACE).unwrap_or_default())
            .trim()
            .to_string();
        match status {
            Some(s) if s.success() => stdout.map_err(|_| {
                format!(
                    "{} exited successfully in {} and its output could not be collected within \
                     {OUTPUT_GRACE:?} — an empty answer is not reported for a command that \
                     produced one",
                    named(),
                    self.root.display()
                )
            }),
            Some(_) => {
                let mut said = stderr;
                if said.is_empty() {
                    said = String::from_utf8_lossy(&stdout.unwrap_or_default())
                        .trim()
                        .to_string();
                }
                said.truncate(GIT_OUTPUT_CAP);
                Err(format!("{} failed: {said}", named()))
            }
            None => {
                // **A killed git can leave `index.lock` behind, and that wedges EVERY later git
                // command in this working copy** — ours, another session's, and the person's own —
                // until somebody removes it (independent review of PR #435, Integrity #2, which
                // rightly called the earlier "a named refusal, not a corruption" an understatement:
                // the refusal is named, and the tree is left unusable behind it).
                //
                // It is reachable without an attacker: a large `commit` on a slow disk is all it
                // takes. So the lock is cleared — but ONLY the one this call can prove it created,
                // which is a lock that was not there before the spawn. One that predates us belongs
                // to somebody else and is NAMED instead of deleted out from under them.
                let mut said = format!(
                    "{} did not finish within {GIT_BOUND:?} and was killed",
                    named()
                );
                match &lock {
                    Some(path) if path.exists() && lock_was_there => said.push_str(&format!(
                        "; {} was already there before this call and has been LEFT ALONE — if \
                         nothing else in this working copy is running git, remove it by hand, \
                         because every git command here fails until it is gone",
                        path.display()
                    )),
                    Some(path) if path.exists() && std::fs::remove_file(path).is_ok() => said
                        .push_str(&format!(
                            "; the {} it left behind has been removed, so later git commands in \
                             this working copy are not blocked by it",
                            path.display()
                        )),
                    Some(path) if path.exists() => said.push_str(&format!(
                        "; it left {} behind and that file could not be removed — every git command \
                         in this working copy fails until it is gone",
                        path.display()
                    )),
                    _ => {}
                }
                Err(said)
            }
        }
    }

    /// Is there a git repository here at all?
    pub fn assert_repository(&self) -> std::result::Result<(), ParkRefusal> {
        self.git(&["rev-parse", "--git-dir"])
            .map(|_| ())
            .map_err(|_| {
                ParkRefusal::NotARepository(format!(
                    "the working copy at {} is not a git repository, so there is nowhere to park \
                     this operation's work",
                    self.root.display()
                ))
            })
    }

    /// **Is the tree in the middle of a sequenced git operation?** — the rare, silently destructive
    /// case correction 3 names, and DECISION (3) of this item: it is not parked.
    ///
    /// A tree mid-rebase, mid-merge, mid-cherry-pick or mid-bisect cannot be committed as if it were
    /// work. `git add -A` over a conflicted index stages the conflict markers and `git commit`
    /// records a half-merge as a finished one — a commit that looks like the operation's work and is
    /// not. Checking out the base from there either fails or abandons the sequencer state, which is
    /// the one thing in a git repository that no later command can reconstruct.
    ///
    /// So this refuses, loudly, and the queue keeps waiting — the refusal can pass, so the rule on
    /// [`ParkRefusal`] holds the claim for it. That is the benign direction: a held checkout is
    /// visible on `nxc status` (`PARK REFUSED`) and retried on every tick, so it moves on by itself
    /// the moment the sequence is finished, while a bogus commit is silent and reaches history. The
    /// refusal names the state AND what finishes it, so whoever reads it knows what to do in the
    /// working copy.
    ///
    /// Asked through `git rev-parse --git-path`, not by joining `.git` onto the root: a worktree or
    /// a submodule keeps its git directory somewhere else entirely, and this must be right there
    /// rather than merely usually right.
    ///
    /// **It is a CHECK, not a lock, and the window is inherent** (independent review of PR #435,
    /// Integrity #6). Somebody can start a rebase in this working copy between this answer and the
    /// commands that follow it. Nothing in a shared checkout can close that window — the working
    /// copy is the shared thing, which is the whole reason this epic serialises access to it rather
    /// than isolating it — so it is written down rather than defended against. What narrows it in
    /// practice is that a park only ever runs against an area with no live session in it.
    pub fn mid_sequence(&self) -> std::result::Result<(), ParkRefusal> {
        // (marker, what the marker means, what finishing it is called) — the third column is the
        // "what to fix" half of the refusal, named per state because "finish the operation" is not
        // an instruction anybody can carry out without knowing which one.
        for (marker, what, which) in [
            ("rebase-merge", "a rebase is in progress", "the rebase"),
            (
                "rebase-apply",
                "a rebase or `git am` is in progress",
                "the rebase or `git am`",
            ),
            ("MERGE_HEAD", "a merge is in progress", "the merge"),
            (
                "CHERRY_PICK_HEAD",
                "a cherry-pick is in progress",
                "the cherry-pick",
            ),
            ("REVERT_HEAD", "a revert is in progress", "the revert"),
            ("BISECT_LOG", "a bisect is in progress", "the bisect"),
            // Both bisect markers, because which one a given git writes is a version detail and
            // this check must not depend on one (independent review of PR #435, Integrity #5).
            ("BISECT_START", "a bisect is in progress", "the bisect"),
        ] {
            let Ok(path) = self.git(&["rev-parse", "--git-path", marker]) else {
                continue;
            };
            if self.root.join(&path).exists() || Path::new(&path).exists() {
                let root = self.root.display();
                return Err(ParkRefusal::MidSequence(format!(
                    "the working copy at {root} is mid-flight — {what} ({marker} is present) — so \
                     this operation's work cannot be parked yet: committing a conflicted index \
                     would record a half-finished merge as if it were the work, and leaving the \
                     base would abandon state git cannot rebuild. Finish or abort {which} in {root}"
                )));
            }
        }
        Ok(())
    }

    /// **Where the tree stands right now** — the value recorded as an operation's [`Base`] on its
    /// first acquire of the lease.
    ///
    /// An empty `branch` is a detached HEAD, kept rather than resolved to something prettier; see
    /// [`Base`].
    pub fn here(&self) -> std::result::Result<Base, ParkRefusal> {
        self.assert_repository()?;
        let commit = self.git(&["rev-parse", "HEAD"]).map_err(ParkRefusal::Git)?;
        // `--quiet` so a detached HEAD is an empty answer rather than an error on stderr; the
        // command still exits non-zero there, which is why the error arm is a `String::new()` and
        // not a refusal.
        let branch = self
            .git(&["symbolic-ref", "--quiet", "--short", "HEAD"])
            .unwrap_or_default();
        Ok(Base { branch, commit })
    }

    /// Whether `rev` resolves to a commit here.
    ///
    /// **NOT named `resolve`, and that is not taste** (nxf 6j6v.12nn). `read_surface.rs` decides
    /// which calls can start a session by walking `chat/src` as a call graph joined on BARE NAMES —
    /// it cannot tell one `resolve` from another. A private helper called `resolve` in this module
    /// therefore merges with `Workspace::resolve` and `Definitions::resolve`, which every read verb
    /// on the engine handle calls; and because [`Self::git`] hands its pipe readers their result
    /// over an mpsc channel, whose method is `send`, the merged name reaches `facade::send` and from
    /// there the spawn funnel. The gate then reports — correctly, on the evidence it has — that
    /// `status`, `search`, `directory` and eight more can start a session.
    ///
    /// Measured, not supposed: naming it `resolve` turned eleven read verbs red with the path
    /// `status -> open -> resolve -> git -> send -> send_to -> … -> trigger_and_bind`. The rule this
    /// leaves behind for anything else added here: **a helper in a module that shells out must not
    /// share a name with something the read seam calls.**
    fn resolve_commit(&self, rev: &str) -> Option<String> {
        self.git(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ])
        .ok()
        .filter(|s| !s.is_empty())
    }

    /// Is anything uncommitted — tracked changes or untracked files?
    ///
    /// **`--untracked-files=normal` and NOT `all`**: the question is "is there something to commit",
    /// and one entry for an untracked directory answers it as well as a thousand for its contents.
    /// Ignored files are not listed and that is [`Self::park`]'s decision (2), not an accident here.
    fn is_dirty(&self) -> std::result::Result<bool, String> {
        Ok(!self
            .git(&["status", "--porcelain", "--untracked-files=normal"])?
            .is_empty())
    }

    /// A `-c user.*` pair when the repository has no identity configured, and nothing otherwise.
    ///
    /// The park must work in a repository nobody has configured — a fresh clone on a build machine,
    /// a temp repository in a test — because a commit that cannot be made is work that is lost. But
    /// where the project HAS an identity, the commit is authored under it: this is the project's
    /// history, and stamping a synthetic author over a configured one would be this engine deciding
    /// something that is not its to decide.
    fn commit_identity(&self) -> Vec<String> {
        let has = |key: &str| {
            self.git(&["config", "--get", key])
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
        };
        if has("user.email") && has("user.name") {
            return Vec::new();
        }
        vec![
            "-c".into(),
            "user.name=nxs".into(),
            "-c".into(),
            "user.email=nxs@localhost".into(),
        ]
    }

    /// **Commit this operation's work onto a branch — and LEAVE THE TREE ON IT.**
    ///
    /// The sequence, and every step of it is a decision this item was asked to make:
    ///
    /// 1. **Refuse a mid-flight tree** ([`Self::mid_sequence`], decision 3).
    /// 2. **Use the operation's own branch if it has one, else create [`park_branch_name`]'s — but
    ///    only when there is work to put on it.** "Has one" is decided against the recorded
    ///    [`Base`]: a HEAD on some branch OTHER than the base branch is the operation's own work
    ///    already living somewhere, and the owner's rule is to commit onto it rather than to invent
    ///    a second name for the same work. A HEAD still on the base branch AND CLEAN parks on the
    ///    base branch itself, with no branch created at all — the owner decided on 2026-09-17
    ///    (`nxf 6j6v.7hqc`) that a park creates a branch only when there is work on the operation's
    ///    base branch, because the engine never deletes park branches, and an empty one would sit
    ///    there forever as clutter nobody removes. A HEAD still on the base branch and DIRTY, or a
    ///    detached HEAD regardless of dirt (its position has to stay reachable somehow, and only a
    ///    branch does that here), gets a branch created — and `git checkout -b` carries any
    ///    uncommitted changes across, which is exactly what has to happen.
    /// 3. **Commit everything git tracks or would track** — `git add -A`, so **untracked files are
    ///    committed** (decision 1). They are the normal output of a coding agent and the least
    ///    reproducible thing in the tree; and leaving them behind would be worse than losing them,
    ///    because a checkout does not remove untracked files and they would follow the working copy
    ///    into the NEXT operation's hands. **Ignored files are not committed** (decision 2):
    ///    `.gitignore` is the project's own statement that they do not belong in history, and they
    ///    are mostly the build directory this epic exists to protect — they stay in place, exactly as
    ///    they do across every other hand-off this engine makes, and [`Parked`] says so.
    /// 4. **Read the commit** — the anchor the movement check compares against.
    ///
    /// **Going back to the base is [`Self::return_to_base`]'s, and the split is the whole point**
    /// (independent review of PR #435, Integrity #1 / Code Quality #1). This used to end with the
    /// checkout, so the tree left the branch before anything had recorded that the branch existed.
    /// If the caller's write then failed, the NEXT attempt met a clean tree standing on the base —
    /// indistinguishable from "never parked" — and minted a second, EMPTY branch; the row that
    /// eventually landed named the decoy while the real commit sat on an unreferenced ref, and the
    /// resume text went on to say there had been nothing to save.
    ///
    /// With the checkout moved out, a failed write leaves the tree ON the parked branch, which is
    /// the state that makes the retry idempotent rather than destructive: the next attempt sees a
    /// HEAD that is not the base, takes step 2's first arm, and reuses the very branch the first
    /// attempt made. Nothing is minted twice and nothing has to be cleaned up.
    ///
    /// A clean tree is not an error: there is nothing to commit, the park still happened, and
    /// [`Parked::committed`] is `false`. On the base branch that also means no branch was created
    /// (step 2, `nxf 6j6v.7hqc`) — [`Parked::created_branch`] is `false` and [`Parked::branch`]
    /// names the base branch itself, not a freshly minted one.
    ///
    /// **NO LOCK SPANS THIS SEQUENCE, AND THE WINDOW IS INHERENT** (independent review of PR #478,
    /// Integrity #2) — the same shape, and the same answer, as [`Self::mid_sequence`]'s own note.
    /// [`Self::here`], the `checkout -b`, the `add -A`, the `commit` and the closing `rev-parse` are
    /// five separate git processes against a working copy this engine deliberately SHARES rather
    /// than isolates, and the only serialisation anywhere on the park path is the SQLite
    /// compare-and-swap on the lease — which runs AFTERWARDS, in the `release_and_fire` that
    /// `orchestration::park_and_hand_on` reaches only once `attempt_the_park` has returned, because
    /// the work has to be saved before the copy can be taken. So a second process can be
    /// anywhere inside this sequence while the first is too, and that is written down here rather
    /// than defended against.
    ///
    /// What makes it safe to leave open is that **every interleaving lands on one of two outcomes,
    /// and neither is a wrong answer**:
    ///
    /// - **Convergence.** A racer whose [`Self::here`] lands after the other one's checkout and
    ///   commit sees a HEAD that is not the base, takes step 2's FIRST arm, finds the tree already
    ///   clean, and derives the identical `Parked { branch, commit }` from `rev-parse HEAD`. That is
    ///   the same arm the single-process retry takes, for the same reason, and
    ///   [`ChatStore::record_parked_work`] is one transaction precisely so that the two of them
    ///   leave ONE row.
    /// - **A loud, transient refusal.** Any other collision fails a git command instead of
    ///   succeeding wrongly: `checkout -b` onto a name that now exists, `.git/index.lock` held by
    ///   the other process, `commit` with nothing staged. All of them exit non-zero and become
    ///   [`ParkRefusal::Git`], which is TRANSIENT ([`ParkRefusal::is_permanent`]) — the claim is
    ///   held, `nxc status` says so, and the next tick tries again.
    ///
    /// What is NOT reachable is a recorded pair whose commit is not on its branch: `checkout -b`
    /// always branches from the current HEAD, and nothing on this path force-resets an existing
    /// ref, so a ref that exists cannot later stop containing what it claimed. What narrows the
    /// window in practice is the same thing that narrows `mid_sequence`'s — a park only ever runs
    /// against a claim area with no live session in it.
    pub fn commit_onto_a_branch(
        &self,
        scope_key: &str,
        now: &str,
        base: &Base,
        occasion: crate::orchestration::ParkOccasion,
    ) -> std::result::Result<Parked, ParkRefusal> {
        self.assert_repository()?;
        self.mid_sequence()?;
        let here = self.here()?;

        // **The 2026-09-17 decision (nxf 6j6v.7hqc), read BEFORE the `checkout -b` below so it can
        // pre-empt it.** HEAD is on the operation's own base branch and the tree is clean: there is
        // nothing uncommitted, so there is nothing for a park branch to hold, and the engine never
        // deletes one it creates — an empty branch would be clutter nobody ever removes. The park
        // still "happens": it reports the operation parked on the base branch itself, uncommitted.
        //
        // **Asked ONCE** (independent review of PR #478, Code Quality #3): `is_dirty` shells out to
        // `git status`, and `committed` further down is the same question about the same tree. What
        // sits between them is the `checkout -b`, and it cannot change the answer — it moves HEAD to
        // a new branch at the same commit and CARRIES the uncommitted changes across, which is the
        // very reason this path creates a branch at all. So the guard's answer is threaded through
        // rather than re-read; the arms that never reach the guard still read it for themselves,
        // exactly once each.
        let dirty_on_the_base = if !base.branch.is_empty() && here.branch == base.branch {
            Some(self.is_dirty().map_err(ParkRefusal::Git)?)
        } else {
            None
        };
        if dirty_on_the_base == Some(false) {
            let commit = self.git(&["rev-parse", "HEAD"]).map_err(ParkRefusal::Git)?;
            return Ok(Parked {
                branch: base.branch.clone(),
                commit,
                base_branch: base.branch.clone(),
                base_commit: base.commit.clone(),
                created_branch: false,
                committed: false,
            });
        }

        let (branch, created_branch) = if !here.branch.is_empty() && here.branch != base.branch {
            (here.branch.clone(), false)
        } else {
            let name = park_branch_name(scope_key, now);
            self.git(&["checkout", "-b", &name])
                .map_err(ParkRefusal::Git)?;
            (name, true)
        };

        let committed = match dirty_on_the_base {
            Some(dirty) => dirty,
            None => self.is_dirty().map_err(ParkRefusal::Git)?,
        };
        if committed {
            self.git(&["add", "-A"]).map_err(ParkRefusal::Git)?;
            let mut args: Vec<String> = self.commit_identity();
            args.push("commit".into());
            args.push("--no-verify".into());
            args.push("--no-gpg-sign".into());
            args.push("--message".into());
            args.push(park_commit_message(scope_key, now, occasion));
            let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
            self.git(&borrowed).map_err(ParkRefusal::Git)?;
        }
        let commit = self.git(&["rev-parse", "HEAD"]).map_err(ParkRefusal::Git)?;

        Ok(Parked {
            branch,
            commit,
            base_branch: base.branch.clone(),
            base_commit: base.commit.clone(),
            created_branch,
            committed,
        })
    }

    /// **Put the tree back where the operation started** — the second half of a park, run only once
    /// the first half has been RECORDED (see [`Self::commit_onto_a_branch`] for why that order).
    ///
    /// The base BRANCH if it still exists, the base COMMIT (detached) if it does not, because the
    /// operation's starting point is a fact and a deleted branch does not unmake it.
    ///
    /// A failure here is not a lost park and the caller must not report it as one: the work is
    /// committed and recorded by the time this runs, and what has failed is only the tidying. What
    /// it DOES mean is that the working copy must not be handed on — it is standing on the parked
    /// branch, and the next operation would start in somebody else's files.
    pub fn return_to_base(&self, base: &Base) -> std::result::Result<(), ParkRefusal> {
        let back_to = if !base.branch.is_empty() && self.resolve_commit(&base.branch).is_some() {
            base.branch.clone()
        } else {
            base.commit.clone()
        };
        self.git(&["checkout", &back_to])
            .map(|_| ())
            .map_err(ParkRefusal::Git)
    }

    /// **Has anything moved since this work was parked?** — both halves, for [`Movement`]'s reason.
    ///
    /// A read and nothing else: it changes no ref and touches no file, so it is safe to ask before
    /// deciding anything, and it answers the same way whether or not the checkout below succeeds.
    ///
    /// **The base half is skipped when it names the SAME ref as the branch** (fix round 1, Finding
    /// 1 of the review of this task): a clean tree parked ON the base branch (`nxf 6j6v.7hqc`) has
    /// `parked.branch == parked.base_branch`, and resolving that one ref twice would — if it
    /// moved — report the identical new tip as BOTH `branch_now` and `base_now`. [`resume_notice`]
    /// would then print two bullets for one movement, the second of them claiming "your branch
    /// does not contain that work", which is false when the branch and the base are the same ref:
    /// [`Self::restore`] checks THAT branch back out, landing exactly on the tip `branch_now`
    /// already reported.
    pub fn movement(&self, parked: &Parked) -> Movement {
        let mut m = Movement::default();
        match self.resolve_commit(&parked.branch) {
            None => m.branch_gone = true,
            Some(tip) if tip != parked.commit => m.branch_now = Some(tip),
            Some(_) => {}
        }
        if !parked.base_branch.is_empty() && parked.base_branch != parked.branch {
            match self.resolve_commit(&parked.base_branch) {
                None => m.base_gone = true,
                Some(tip) if tip != parked.base_commit => m.base_now = Some(tip),
                Some(_) => {}
            }
        }
        m
    }

    /// **Put the tree back on a parked branch** so a resumed session finds its own files.
    ///
    /// **git decides, not this function.** A plain `git checkout <branch>` refuses when the switch
    /// would overwrite uncommitted changes and permits it when it would not — which is exactly the
    /// rule wanted here, written by the tool that owns it. Whatever it says is reported: this never
    /// forces, never stashes and never discards, because the changes it would be discarding belong
    /// to whichever operation held the copy last, and losing a stranger's work to recover your own
    /// is not a trade this engine makes on its own.
    ///
    /// The failure is not fatal to the resume. The session still starts, and the deterministic text
    /// tells it where its work is and that the tree is not on it — which is strictly better than a
    /// session that starts on the wrong branch believing it is on the right one.
    pub fn restore(&self, branch: &str) -> std::result::Result<(), String> {
        self.assert_repository()
            .map_err(|r| r.detail().to_string())?;
        self.git(&["checkout", branch]).map(|_| ())
    }
}

/// **What a resumed session is told, word for word** (the DoD of nxf 6j6v.de9s: *"der fortsetzende
/// Agent bekommt Zweigname und Commit-Hash in einem DETERMINISTISCHEN Text"*).
///
/// A fixed template with the facts substituted in — no model, no free prose, and the same input
/// always produces the same output, which is what makes it testable and what stops the one message
/// an interrupted session actually reads from being invented afresh each time.
///
/// **It says four things, and each is here because omitting it would mislead:**
///
/// 1. **Where the work is** — branch and commit. Without this the session's own files look lost.
/// 2. **Whether the tree is on it.** [`WorkingCopy::restore`] lets git decide, and git refuses when
///    the switch would overwrite somebody's uncommitted changes. A session told "you are on your
///    branch" when it is not would write its next change into a stranger's tree, which is worse than
///    the park was.
/// 3. **What was NOT saved** — decision (2) of this item: files `.gitignore` excludes were left in
///    the working copy rather than committed. Said out loud, because "everything was committed" is
///    what a reader assumes, and the difference is exactly where a `.env` or a build output lives.
/// 4. **What moved** — both halves ([`Movement`]). A base that ran on while the branch stood still
///    is the case correction 4 exists for, and a session that resumes without being told will build
///    on a foundation that is no longer there.
///
/// `restored` is [`WorkingCopy::restore`]'s own answer, passed in rather than re-asked, so the text
/// cannot claim a checkout that did not happen.
pub fn resume_notice(
    parked: &Parked,
    restored: &std::result::Result<(), String>,
    movement: &Movement,
) -> String {
    let mut out = String::from(
        "PARKED WORK — while you were waiting for an answer, this operation's work was committed to \
         a branch and the working copy was handed to another operation. Here is where everything \
         is.\n\n",
    );
    out.push_str(&format!("  branch: {}\n", parked.branch));
    out.push_str(&format!("  commit: {}\n", parked.commit));
    if parked.base_branch.is_empty() {
        out.push_str(&format!("  base:   {} (detached)\n", parked.base_commit));
    } else {
        out.push_str(&format!(
            "  base:   {} at {}\n",
            parked.base_branch, parked.base_commit
        ));
    }
    out.push('\n');
    match restored {
        Ok(()) => out.push_str("The working copy has been put back on that branch.\n"),
        Err(said) => out.push_str(&format!(
            "The working copy could NOT be put back on that branch: {said}\nCheck it out \
             yourself before you change anything — what is in the tree now is not your work.\n"
        )),
    }
    // **"Nothing to save" is only ever said about a branch this park MINTED** (independent review
    // of PR #435, Integrity #1). On a branch the operation already had, a clean tree means the work
    // is already committed on it — saying "there was nothing to save" there would tell a session its
    // own branch is empty when it is not, which is the one sentence in this text that could send
    // somebody looking for work that is sitting right in front of them.
    //
    // **A clean park ON THE BASE BRANCH gets a THIRD sentence, not the second one** (fix round 1,
    // Finding 2 of the review of this task). `!parked.created_branch` used to mean exactly one
    // thing — "the operation already had a branch of its own, and it already carried the work" —
    // until `nxf 6j6v.7hqc` gave it a second, unrelated cause: HEAD on the base branch with
    // nothing uncommitted at all, where `parked.branch` names the base and there is no branch of
    // the operation's own that could have "already carried" anything. Reusing that sentence there
    // would claim a branch exists that this park deliberately did not create.
    if !parked.committed {
        if !parked.base_branch.is_empty() && parked.branch == parked.base_branch {
            out.push_str(&format!(
                "Nothing was uncommitted when the copy was handed on, so no park branch was made \
                 — you are resuming right on the base branch, {}.\n",
                parked.branch
            ));
        } else if parked.created_branch {
            out.push_str(
                "There was nothing uncommitted to save: the tree was clean when it was handed \
                 on.\n",
            );
        } else {
            out.push_str(
                "Nothing new had to be committed: this branch already carried your work when \
                 the copy was handed on.\n",
            );
        }
    }
    out.push_str(
        "Files your `.gitignore` excludes were NOT committed. They were left in the working copy \
         exactly as they were, which is also where another operation has been working since.\n",
    );
    if !movement.any() {
        out.push_str("Nothing has moved since: the branch and its base are where you left them.\n");
        return out;
    }
    out.push_str("\nWhat moved while you were away:\n");
    // **The branch and the base are the SAME ref when a clean tree parked ON the base branch**
    // (fix round 1, Finding 1 of the review of this task, `nxf 6j6v.7hqc`). `movement()` resolves
    // that ref once, so exactly one of `branch_gone`/`branch_now` fires here — and this reports it
    // as what it is, the base branch itself, rather than falling into the generic branch bullet
    // below and then ALSO into the base bullets further down for the same move.
    if !parked.base_branch.is_empty() && parked.branch == parked.base_branch {
        if movement.branch_gone {
            out.push_str(&format!(
                "  - {} — the base branch you parked on — no longer resolves. Commit {} is the \
                 only handle left on where you stood.\n",
                parked.branch, parked.commit
            ));
        }
        if let Some(tip) = &movement.branch_now {
            out.push_str(&format!(
                "  - {} — the base branch you parked on — has moved on: it is at {} now, and it \
                 was at {} when you parked. You are resuming on it exactly as it stands now.\n",
                parked.branch, tip, parked.commit
            ));
        }
        return out;
    }
    if movement.branch_gone {
        out.push_str(&format!(
            "  - the branch {} no longer resolves — it was deleted or renamed, and commit {} is the \
             only handle left on your work.\n",
            parked.branch, parked.commit
        ));
    }
    if let Some(tip) = &movement.branch_now {
        out.push_str(&format!(
            "  - {} has moved on: it is at {} now, and it was at {} when your work was parked.\n",
            parked.branch, tip, parked.commit
        ));
    }
    if movement.base_gone {
        out.push_str(&format!(
            "  - the base {} no longer resolves.\n",
            parked.base_branch
        ));
    }
    if let Some(tip) = &movement.base_now {
        out.push_str(&format!(
            "  - the base {} has moved on: it is at {} now, and it was at {} when this operation \
             started. Your branch does not contain that work.\n",
            parked.base_branch, tip, parked.base_commit
        ));
    }
    out
}

/// **One row of [`ChatStore::parked_work`]** — a park that happened, as the store kept it.
///
/// [`Self::parked`] is the same [`Parked`] the park itself returned, so the movement check reads one
/// shape whether it is asking about a park that just happened or one from three days ago.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ParkedWork {
    /// The row id — what [`ChatStore::mark_parked_work_resumed`] names.
    pub id: i64,
    /// When the working copy was handed on.
    pub parked_at: String,
    /// Everything the park produced.
    #[serde(flatten)]
    pub parked: Parked,
}

impl ChatStore {
    /// **Record where an operation started, once and once only** (correction 2).
    ///
    /// `INSERT ... ON CONFLICT DO NOTHING`, in one statement, for the reason every write in
    /// [`crate::working_tree`] is one statement: two `nxc` processes acquiring for the same area
    /// must not be able to interleave between reading "is a base recorded" and writing one. The
    /// FIRST acquire wins, which is exactly the semantics wanted — a later acquire looks at a tree
    /// the operation may already have moved.
    ///
    /// Returns whether this call is the one that wrote it, which is what lets the caller pay for the
    /// git read only when there is nothing recorded yet.
    pub fn record_operation_base(
        &mut self,
        scope_key: &str,
        base: &Base,
        now: &str,
    ) -> Result<bool> {
        let n = self.connection().execute(
            "INSERT INTO operation_base(scope_key, branch, commit_id, recorded_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(scope_key) DO NOTHING",
            params![scope_key, base.branch, base.commit, now],
        )?;
        Ok(n > 0)
    }

    /// The base recorded for `scope_key`, or `None` when this operation never took the lease while
    /// a working copy could be read — in which case nothing can be parked for it, and the park says
    /// so rather than guessing a base.
    pub fn operation_base(&self, scope_key: &str) -> Result<Option<Base>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT branch, commit_id FROM operation_base WHERE scope_key = ?1",
                params![scope_key],
                |r| {
                    Ok(Base {
                        branch: r.get(0)?,
                        commit: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Append one park to the operation's list — the LIST the owner's rule asks for, appended to
    /// rather than replaced, so a second park does not erase the first branch's existence.
    ///
    /// **Idempotent on (scope, branch, commit) while the row is unresumed** (independent review of
    /// PR #435, Integrity #1). An attempt that got as far as the commit and then failed — this
    /// write itself, or the checkout after it — is retried by the next tick, and that retry finds
    /// the tree still standing on the parked branch and re-derives the SAME branch and the SAME
    /// commit ([`WorkingCopy::commit_onto_a_branch`] has the whole shape). Appending a second row
    /// for it would leave the operation with two entries naming one park, and the resume would then
    /// report a park that never separately happened.
    ///
    /// A row that has been RESUMED is not reused: coming back and being parked again is exactly the
    /// case the list exists for, and the branch may legitimately be the operation's own both times.
    ///
    /// **The look and the write are ONE `BEGIN IMMEDIATE`, and that is what makes the claim above
    /// true of two PROCESSES and not only of one retrying itself** (independent review of PR #478,
    /// Integrity #1). As a plain SELECT-then-INSERT it was true of the retry and false of the two
    /// doors nxf 6j6v.xb24 opened onto the same dead holder: the tick sweep and a newcomer's own
    /// reclaim can both notice it, and they converge on the IDENTICAL `(branch, commit)` by design —
    /// [`WorkingCopy::commit_onto_a_branch`]'s reuse arm is what makes the git-level race
    /// self-healing, so converging is the normal case here rather than the exotic one. Unguarded,
    /// both SELECTs can run before either INSERT, neither finds anything, and one park event leaves
    /// two rows and a duplicated `parked on …` line on `nxc status`. The transaction serialises the
    /// loser's SELECT behind the winner's commit, so it finds the row and takes the early return
    /// below — the same early return the retry has always taken. Dropping the transaction on that
    /// path rolls back nothing, because nothing was written.
    pub fn record_parked_work(
        &mut self,
        scope_key: &str,
        parked: &Parked,
        now: &str,
    ) -> Result<i64> {
        let tx = rusqlite::Transaction::new_unchecked(
            self.connection(),
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM parked_work
                  WHERE scope_key = ?1 AND branch = ?2 AND commit_id = ?3 AND resumed_at IS NULL",
                params![scope_key, parked.branch, parked.commit],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        tx.execute(
            "INSERT INTO parked_work(
                 scope_key, branch, commit_id, base_branch, base_commit,
                 created_branch, committed, parked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                scope_key,
                parked.branch,
                parked.commit,
                parked.base_branch,
                parked.base_commit,
                parked.created_branch as i64,
                parked.committed as i64,
                now
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// **The parks of one operation that nobody has come back for**, newest first.
    ///
    /// Newest first because that is the one a resume wants: an operation parked twice resumes onto
    /// the branch its most recent park left, and the older rows are history rather than candidates.
    /// They are still returned — a reader looking for work that seems to have vanished needs the
    /// whole list, which is the reason this is a list at all.
    pub fn parked_work(&self, scope_key: &str) -> Result<Vec<ParkedWork>> {
        let conn = self.connection();
        let mut st = conn.prepare(
            "SELECT id, parked_at, branch, commit_id, base_branch, base_commit,
                    created_branch, committed
               FROM parked_work
              WHERE scope_key = ?1 AND resumed_at IS NULL
              ORDER BY id DESC",
        )?;
        let rows = st
            .query_map(params![scope_key], |r| {
                Ok(ParkedWork {
                    id: r.get(0)?,
                    parked_at: r.get(1)?,
                    parked: Parked {
                        branch: r.get(2)?,
                        commit: r.get(3)?,
                        base_branch: r.get(4)?,
                        base_commit: r.get(5)?,
                        created_branch: r.get::<_, i64>(6)? != 0,
                        committed: r.get::<_, i64>(7)? != 0,
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// **Is there any unresumed parked work in this workspace at all?** — the cheap guard that keeps
    /// the question off the ordinary trigger path.
    ///
    /// [`crate::orchestration::trigger_role`] has to ask "does THIS operation have work parked?"
    /// before it can decide whether a trigger must contend for the working copy, and that question
    /// costs a thread-tree walk to resolve the operation at all. This one is a single indexed read of
    /// a table that is empty in every workspace where nothing has ever been parked — which is all of
    /// them until an escalation goes unanswered under contention — so the walk is paid for only where
    /// there is something to find.
    pub fn any_parked_work(&self) -> Result<bool> {
        Ok(self.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM parked_work WHERE resumed_at IS NULL)",
            [],
            |r| r.get::<_, i64>(0),
        )? != 0)
    }

    /// **Every unresumed park in the workspace, read in ONE query and grouped by the key it was
    /// recorded under** (nxf 6j6v.s9ex) — [`Self::parked_work`]'s shape, widened from one scope to
    /// the whole workspace, for `facade::status` to attribute onto its operations without paying one
    /// query per operation. Each group is newest first, exactly like [`Self::parked_work`].
    ///
    /// Call it only after [`Self::any_parked_work`] answers `true` — that is the cheap guard, this
    /// is the read it guards, and the report that calls both is what keeps a workspace that has
    /// never parked down to the one indexed existence check.
    pub fn all_parked_work(&self) -> Result<HashMap<String, Vec<ParkedWork>>> {
        let conn = self.connection();
        let mut st = conn.prepare(
            "SELECT scope_key, id, parked_at, branch, commit_id, base_branch, base_commit,
                    created_branch, committed
               FROM parked_work
              WHERE resumed_at IS NULL
              ORDER BY scope_key, id DESC",
        )?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    ParkedWork {
                        id: r.get(1)?,
                        parked_at: r.get(2)?,
                        parked: Parked {
                            branch: r.get(3)?,
                            commit: r.get(4)?,
                            base_branch: r.get(5)?,
                            base_commit: r.get(6)?,
                            created_branch: r.get::<_, i64>(7)? != 0,
                            committed: r.get::<_, i64>(8)? != 0,
                        },
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut by_scope: HashMap<String, Vec<ParkedWork>> = HashMap::new();
        for (scope_key, row) in rows {
            by_scope.entry(scope_key).or_default().push(row);
        }
        Ok(by_scope)
    }

    /// Stamp one park as come back for. The row STAYS — see the table's own comment: "parked twice
    /// and came back twice" is exactly the history somebody debugging a lost branch needs, and a
    /// delete would take it.
    pub fn mark_parked_work_resumed(&mut self, id: i64, now: &str) -> Result<bool> {
        let n = self.connection().execute(
            "UPDATE parked_work SET resumed_at = ?2 WHERE id = ?1 AND resumed_at IS NULL",
            params![id, now],
        )?;
        Ok(n > 0)
    }

    /// **Record that `scope_key`'s park was refused for a reason that can pass** (nxf 6j6v.8bv9) —
    /// what `nxc status` shows as `PARK REFUSED` on the holding operation while the tick retries.
    ///
    /// One row per claim, in one statement: a retry that is refused again moves `last_at` and takes
    /// the newest words and the occasion that made this attempt, and KEEPS `first_at` — "retrying
    /// since" is since the first refusal, and a retry that reset it would make a park refused for a
    /// day look like one refused a minute ago. That holds for a retry from a DIFFERENT occasion
    /// too: what the instant answers is "how long has this claim been refusing to park", which is
    /// the same question whichever trouble asked it last.
    ///
    /// **Where the row goes again — every one of them, so a stale "retrying" cannot outlive what it
    /// is about:**
    ///
    /// - a park that goes through, and a copy handed on unparked (`orchestration::park_and_hand_on`,
    ///   through [`Self::clear_park_refusal`]);
    /// - the lease leaving this holder by ANY path — a release and an expired reclaim, inside the
    ///   very transaction that moves it ([`forget_park_refusal`], called from
    ///   `working_tree`'s two hand-off entrances), and a different scope taking an expired lease in
    ///   the acquire compare-and-swap ([`forget_other_park_refusals`]). The last is the one that
    ///   `orchestration::release_and_fire` never sees, which is why the forgetting sits in the store
    ///   rather than beside that function;
    /// - **the OCCASION named here ceasing to stand** while the claim is still held (fix round 1 of
    ///   nxf 6j6v.8bv9) — the escalation is answered and the operation works on, the hold is taken
    ///   up. Nothing is attempting that park any more, so `orchestration::park_step` forgets the row
    ///   on the tick where its occasion no longer stands. It clears ONLY rows stamped with that
    ///   occasion: another trouble's retry is still running, and dropping its row would reset the
    ///   one instant it is keeping.
    pub fn note_park_refusal(
        &mut self,
        scope_key: &str,
        refusal: &ParkRefusal,
        occasion: crate::orchestration::ParkOccasion,
        now: &str,
    ) -> Result<()> {
        self.connection().execute(
            "INSERT INTO park_refusals(scope_key, refusal, detail, occasion, first_at, last_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(scope_key) DO UPDATE SET
                 refusal  = excluded.refusal,
                 detail   = excluded.detail,
                 occasion = excluded.occasion,
                 last_at  = excluded.last_at",
            params![
                scope_key,
                refusal.kind(),
                refusal.detail(),
                occasion.as_str(),
                now
            ],
        )?;
        Ok(())
    }

    /// Forget `scope_key`'s refusal, reporting whether there was one — see
    /// [`Self::note_park_refusal`] for every caller.
    pub fn clear_park_refusal(&mut self, scope_key: &str) -> Result<bool> {
        forget_park_refusal(self.connection(), scope_key)
    }

    /// The refusal recorded for `scope_key`, or `None` when its park has not been refused (or is not
    /// being retried any more).
    pub fn park_refusal(&self, scope_key: &str) -> Result<Option<ParkRefusalNote>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT refusal, detail, occasion, first_at, last_at
                 FROM park_refusals WHERE scope_key = ?1",
                params![scope_key],
                park_refusal_note_from_row,
            )
            .optional()?)
    }

    /// **Every refusal on record, keyed by the claim it is about** — [`Self::all_parked_work`]'s
    /// shape, for `facade::status` to attribute onto its operations in one query.
    ///
    /// No existence guard in front of it, unlike its neighbour: the table holds at most one row per
    /// claim and a device has one claim at a time, so reading all of it IS the cheap check.
    pub fn all_park_refusals(&self) -> Result<HashMap<String, ParkRefusalNote>> {
        let conn = self.connection();
        let mut st = conn.prepare(
            "SELECT scope_key, refusal, detail, occasion, first_at, last_at FROM park_refusals",
        )?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    ParkRefusalNote {
                        refusal: r.get(1)?,
                        detail: r.get(2)?,
                        occasion: r.get(3)?,
                        since: r.get(4)?,
                        last_tried: r.get(5)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<HashMap<_, _>>>()?;
        Ok(rows)
    }
}

fn park_refusal_note_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ParkRefusalNote> {
    Ok(ParkRefusalNote {
        refusal: r.get(0)?,
        detail: r.get(1)?,
        occasion: r.get(2)?,
        since: r.get(3)?,
        last_tried: r.get(4)?,
    })
}

/// **Forget `scope_key`'s refusal on a connection the caller is already writing through** — so the
/// lease leaving a holder and its refusal going land in ONE transaction, and no reader can see a
/// claim that has moved on still "retrying". `working_tree`'s release and expired reclaim call it
/// inside their own `BEGIN IMMEDIATE`; [`ChatStore::clear_park_refusal`] is the same statement on
/// its own.
pub(crate) fn forget_park_refusal(conn: &rusqlite::Connection, scope_key: &str) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM park_refusals WHERE scope_key = ?1",
        params![scope_key],
    )? > 0)
}

/// **Forget every refusal but `holder`'s** — for the one lease movement that names only the NEW
/// holder: [`ChatStore::acquire_working_tree`]'s compare-and-swap, which can take an expired lease
/// from a scope it never names. Whoever holds the copy now is the only claim a refusal can be about.
pub(crate) fn forget_other_park_refusals(conn: &rusqlite::Connection, holder: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM park_refusals WHERE scope_key <> ?1",
        params![holder],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_branch_name_is_a_pure_function_of_the_operation_and_the_instant() {
        // The property correction 1 is about: computable with no model, no network, no second
        // process — so two calls with the same inputs are the same name, always.
        let a = park_branch_name("thread:01JAAA", "2026-09-05T09:01:51Z");
        let b = park_branch_name("thread:01JAAA", "2026-09-05T09:01:51Z");
        assert_eq!(a, b);
        assert_eq!(a, "nxs/park/thread-01JAAA/20260905T090151Z");
    }

    #[test]
    fn a_branch_name_carries_no_character_git_refuses() {
        // A ref may not contain a colon, and every scope key has one.
        let name = park_branch_name("run:01J~BB^CC?DD", "2026-09-05T09:01:51+02:00");
        assert!(!name.contains(':'), "{name}");
        for bad in ['~', '^', '?', '*', '[', '\\', ' '] {
            assert!(!name.contains(bad), "{bad} in {name}");
        }
        // …and the instant is normalized to UTC on the way in, so two names for one instant
        // expressed in two offsets are the same name.
        assert_eq!(
            name,
            park_branch_name("run:01J~BB^CC?DD", "2026-09-05T07:01:51Z")
        );
    }

    #[test]
    fn a_segment_that_would_start_with_a_dash_is_prefixed() {
        assert_eq!(ref_segment("-nope"), "x-nope");
        assert_eq!(ref_segment(".hidden"), "x.hidden");
        assert_eq!(ref_segment(""), "x");
    }

    #[test]
    fn an_unparseable_instant_costs_the_label_and_never_the_commit() {
        // The name is cosmetic; a park that refused over a timestamp shape would sacrifice the one
        // thing that matters for the one that does not.
        let name = park_branch_name("thread:x", "not an instant");
        assert!(name.starts_with("nxs/park/thread-x/"), "{name}");
    }

    /// **The refusal rule's split, by value** (nxf 6j6v.8bv9, owner decision of 2026-09-17): what
    /// waiting can change holds the claim, what it cannot hands the copy on.
    #[test]
    fn only_a_refusal_that_waiting_cannot_change_is_permanent() {
        for (refusal, permanent) in [
            (ParkRefusal::NoWorkingCopy(String::new()), true),
            (ParkRefusal::NotARepository(String::new()), true),
            (ParkRefusal::NoBaseRecorded(String::new()), true),
            (ParkRefusal::MidSequence(String::new()), false),
            (ParkRefusal::Git(String::new()), false),
        ] {
            assert_eq!(refusal.is_permanent(), permanent, "{refusal:?}");
        }
    }

    /// **The terminal and the JSON spell a refusal the same way.** `kind` is what the store keeps
    /// and `nxc status` prints; the serde tag is what `--json` carries; a variant renamed in one and
    /// not the other would have the two disagree about which refusal a reader is looking at.
    #[test]
    fn every_refusal_names_itself_the_way_json_does() {
        for refusal in [
            ParkRefusal::NoWorkingCopy("x".into()),
            ParkRefusal::NotARepository("x".into()),
            ParkRefusal::MidSequence("x".into()),
            ParkRefusal::Git("x".into()),
            ParkRefusal::NoBaseRecorded("x".into()),
        ] {
            let json = serde_json::to_value(&refusal).unwrap();
            assert_eq!(json["refusal"], refusal.kind(), "{json}");
            assert_eq!(json["detail"], "x", "{json}");
            assert_eq!(refusal.detail(), "x");
        }
    }

    /// **"Retrying since" is since the FIRST refusal** — a retry refused again takes the newest
    /// words, the occasion that made THIS attempt and a moved `last_tried`, and keeps `since`;
    /// clearing is idempotent and says whether it found anything.
    ///
    /// The second attempt here comes from the other occasion on purpose: what `since` answers is
    /// how long this claim has been failing to park, which does not restart because a different
    /// trouble asked last.
    #[test]
    fn a_refused_park_keeps_the_instant_it_was_first_refused_at() {
        use crate::orchestration::ParkOccasion;
        let mut store = ChatStore::open_in_memory(1);
        assert_eq!(store.park_refusal("thread:a").unwrap(), None);
        assert!(store.all_park_refusals().unwrap().is_empty());

        store
            .note_park_refusal(
                "thread:a",
                &ParkRefusal::MidSequence("a merge is in progress".into()),
                ParkOccasion::StrandedEscalation,
                "2026-09-05T10:00:00Z",
            )
            .unwrap();
        store
            .note_park_refusal(
                "thread:a",
                &ParkRefusal::Git("`git checkout` failed: index.lock".into()),
                ParkOccasion::AvailabilityBoundary,
                "2026-09-05T10:01:00Z",
            )
            .unwrap();

        let expected = ParkRefusalNote {
            refusal: "git".to_string(),
            detail: "`git checkout` failed: index.lock".to_string(),
            occasion: "availability_boundary".to_string(),
            since: "2026-09-05T10:00:00Z".to_string(),
            last_tried: "2026-09-05T10:01:00Z".to_string(),
        };
        assert_eq!(
            store.park_refusal("thread:a").unwrap(),
            Some(expected.clone())
        );
        let all = store.all_park_refusals().unwrap();
        assert_eq!(all.len(), 1, "one row per claim: {all:?}");
        assert_eq!(all.get("thread:a"), Some(&expected));

        assert!(store.clear_park_refusal("thread:a").unwrap());
        assert!(!store.clear_park_refusal("thread:a").unwrap(), "idempotent");
        assert_eq!(store.park_refusal("thread:a").unwrap(), None);
    }

    /// **Two processes parking the same work at once leave ONE row** (independent review of PR
    /// #478, Integrity #1).
    ///
    /// [`ChatStore::record_parked_work`]'s own doc claims idempotency on `(scope, branch, commit)`
    /// while the row is unresumed, and a plain SELECT-then-INSERT only delivers that for ONE
    /// process retrying ITSELF. This epic opened a second door onto the same dead holder — the tick
    /// sweep and a newcomer's own reclaim — and they converge on the identical `(branch, commit)`
    /// on purpose: [`WorkingCopy::commit_onto_a_branch`]'s reuse arm is what makes the git-level
    /// race self-healing. Converging is therefore the NORMAL case here, not the exotic one, and
    /// with the pair unguarded both callers SELECT nothing and both insert: one park event, two
    /// rows, and `nxc status` printing the same `parked on …` line twice.
    ///
    /// **The interleaving is FORCED, not raced.** A third connection holds the write lock open, so
    /// each caller is stopped at whichever of its statements needs that lock first — the INSERT
    /// when the pair is unguarded, the `BEGIN IMMEDIATE` when it is one transaction. Each says so
    /// through its own busy handler, this thread waits until BOTH are standing there and only then
    /// lets the lock go. No sleep and no scheduler luck in either direction: unguarded, both
    /// SELECTs have already run and both INSERTs land; guarded, the loser cannot even SELECT until
    /// the winner has committed, so it finds the row and takes the early return.
    #[test]
    fn two_concurrent_parks_of_the_same_work_leave_one_row() {
        use std::sync::atomic::{AtomicBool, Ordering};

        static FIRST_IS_WAITING: AtomicBool = AtomicBool::new(false);
        static SECOND_IS_WAITING: AtomicBool = AtomicBool::new(false);
        // A caller still refused after this many retries is deadlocked rather than contended:
        // answering `false` there fails its call loudly instead of hanging the suite.
        const GIVE_UP_AFTER: i32 = 5_000_000;
        fn first_is_waiting(retries: i32) -> bool {
            FIRST_IS_WAITING.store(true, Ordering::SeqCst);
            std::thread::yield_now();
            retries < GIVE_UP_AFTER
        }
        fn second_is_waiting(retries: i32) -> bool {
            SECOND_IS_WAITING.store(true, Ordering::SeqCst);
            std::thread::yield_now();
            retries < GIVE_UP_AFTER
        }

        const KEY: &str = "thread:01JRACE";
        let parked = Parked {
            branch: "nxs/park/thread-01JRACE/20260905T090000Z".into(),
            commit: "1111111111111111111111111111111111111111".into(),
            base_branch: "main".into(),
            base_commit: "0000000000000000000000000000000000000000".into(),
            created_branch: true,
            committed: true,
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("db.sqlite").to_str().unwrap().to_string();
        // The gate: a write transaction with nothing in it, so the only thing it does is hold both
        // callers where they stand until this thread is sure they are both there.
        let gate = ChatStore::open(&path, 1).unwrap();

        // `open` APPLIES THE SCHEMA, which is itself a write — so every connection is opened while
        // the gate is still free (`opened`), and only then is the lock taken (`ready`). Opening
        // under the gate would block a racer before it ever reached the call under test.
        let opened = std::sync::Arc::new(std::sync::Barrier::new(3));
        let ready = std::sync::Arc::new(std::sync::Barrier::new(3));
        let racer = |device: i64, waiting: fn(i32) -> bool| {
            let (path, parked) = (path.clone(), parked.clone());
            let (opened, ready) = (
                std::sync::Arc::clone(&opened),
                std::sync::Arc::clone(&ready),
            );
            std::thread::spawn(move || {
                let mut store = ChatStore::open(&path, device).unwrap();
                store.connection().busy_handler(Some(waiting)).unwrap();
                opened.wait();
                ready.wait();
                store.record_parked_work(KEY, &parked, NOW).unwrap()
            })
        };
        let first = racer(2, first_is_waiting);
        let second = racer(3, second_is_waiting);

        opened.wait();
        gate.connection().execute_batch("BEGIN IMMEDIATE;").unwrap();
        ready.wait();

        let since = std::time::Instant::now();
        while !(FIRST_IS_WAITING.load(Ordering::SeqCst) && SECOND_IS_WAITING.load(Ordering::SeqCst))
        {
            assert!(
                since.elapsed() < std::time::Duration::from_secs(30),
                "a caller never reached the write lock this test holds, so the interleaving under \
                 test never happened — first waiting: {}, second waiting: {}",
                FIRST_IS_WAITING.load(Ordering::SeqCst),
                SECOND_IS_WAITING.load(Ordering::SeqCst)
            );
            std::thread::yield_now();
        }
        gate.connection().execute_batch("COMMIT;").unwrap();

        let first = first.join().expect("the first caller does not panic");
        let second = second.join().expect("the second caller does not panic");

        let rows = ChatStore::open(&path, 4).unwrap().parked_work(KEY).unwrap();
        assert_eq!(
            rows.len(),
            1,
            "one park event is one row — a second would put the same `parked on …` line on `nxc \
             status` twice: {rows:?}"
        );
        assert_eq!(rows[0].parked, parked, "and it is the park that happened");
        assert_eq!(
            first, second,
            "both callers were told about the same row, which is what idempotent means here"
        );
        assert_eq!(first, rows[0].id);
    }

    #[test]
    fn a_commit_message_names_the_operation_and_says_it_is_not_a_result() {
        use crate::orchestration::ParkOccasion;
        let m = park_commit_message(
            "thread:01JAAA",
            "2026-09-05T09:01:51Z",
            ParkOccasion::StrandedEscalation,
        );
        assert!(m.starts_with("nxs: parked thread:01JAAA at 2026-09-05T09:01:51Z\n"));
        assert!(m.contains("was waiting for an answer to an escalation"));
        assert!(m.contains("Nothing here was finished or reviewed"));
    }

    /// **Every occasion states its OWN true reason** (fix round 3 of nxf 6j6v.b9nf's review, Code
    /// Quality #7 (c)) — this commit lands in the user's own git history, so a wrong reason there
    /// is a fact a reader believes, not a bug report anybody files. One assertion per occasion,
    /// each on the one word only that occasion's sentence carries, so a copy-paste that left two
    /// occasions saying the same thing would be caught here rather than by a reader a week later.
    #[test]
    fn every_occasion_states_its_own_reason_in_the_commit_message() {
        use crate::orchestration::ParkOccasion;
        let says = |occasion: ParkOccasion, needle: &str| {
            let m = park_commit_message("thread:01JAAA", "2026-09-05T09:01:51Z", occasion);
            assert!(
                m.contains(needle),
                "{occasion:?} should say {needle:?}: {m}"
            );
        };
        says(ParkOccasion::StrandedEscalation, "an escalation");
        says(ParkOccasion::AvailabilityBoundary, "availability boundary");
        says(ParkOccasion::PastItsBound, "lease ran out");
        says(ParkOccasion::DiedInsideItsBound, "process died");
        says(ParkOccasion::Withdrawn, "was withdrawn");

        // And every occasion still carries the two facts common to all of them.
        for occasion in [
            ParkOccasion::StrandedEscalation,
            ParkOccasion::AvailabilityBoundary,
            ParkOccasion::PastItsBound,
            ParkOccasion::DiedInsideItsBound,
            ParkOccasion::Withdrawn,
        ] {
            let m = park_commit_message("thread:01JAAA", "2026-09-05T09:01:51Z", occasion);
            assert!(m.starts_with("nxs: parked thread:01JAAA at 2026-09-05T09:01:51Z\n"));
            assert!(m.contains("Nothing here was finished or reviewed"));
        }
    }

    // ---- the git mechanics, against real repositories ------------------------------------------
    //
    // Every one of these drives real `git` in a real temporary repository, deliberately: what is
    // being asserted is what a checkout looks like AFTERWARDS — which files are in the tree, which
    // branch HEAD is on, what a commit contains — and none of that can be faked by a double without
    // the test asserting its own model of git instead of git.

    /// A repository on `main` with one commit, a `.gitignore`, and NO configured identity — so the
    /// park exercises [`WorkingCopy::commit_identity`]'s fallback rather than a dev machine's own
    /// name.
    fn repo() -> (tempfile::TempDir, WorkingCopy) {
        let tmp = tempfile::TempDir::new().unwrap();
        git(tmp.path(), &["init", "-q"]);
        // Never `init -b main`: `init.defaultBranch` is a machine setting, and a test that inherits
        // it asserts against whatever the developer happens to have configured.
        git(tmp.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
        std::fs::write(tmp.path().join(".gitignore"), "build/\n").unwrap();
        std::fs::write(tmp.path().join("kept.txt"), "one\n").unwrap();
        git(tmp.path(), &["add", "-A"]);
        commit(tmp.path(), "first");
        let copy = WorkingCopy::at(tmp.path());
        (tmp, copy)
    }

    fn git(root: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A setup commit made under an explicit identity, so the REPOSITORY still has none configured.
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

    /// The two halves of a park, in the order [`park_the_stranded_holder`] runs them — commit onto a
    /// branch, then return the tree to its base.
    ///
    /// The tests below assert what the working copy looks like AFTERWARDS, so they have to drive the
    /// same sequence the caller drives; a helper is what keeps that true when only one of the two
    /// halves changes. What sits BETWEEN them in production is the database write, and
    /// [`a_retry_after_a_failed_record_reuses_the_branch_instead_of_minting_an_empty_one`] is the
    /// test for the state a failure there leaves behind.
    ///
    /// [`park_the_stranded_holder`]: crate::orchestration
    ///
    /// `StrandedEscalation` throughout: these are the git mechanics, occasion-blind by design, and
    /// [`a_commit_message_names_the_operation_and_says_it_is_not_a_result`] is where each occasion's
    /// own wording is pinned instead.
    fn park(copy: &WorkingCopy, scope: &str, now: &str, base: &Base) -> Parked {
        let parked = copy
            .commit_onto_a_branch(
                scope,
                now,
                base,
                crate::orchestration::ParkOccasion::StrandedEscalation,
            )
            .unwrap();
        copy.return_to_base(base).unwrap();
        parked
    }

    const NOW: &str = "2026-09-05T09:00:00Z";

    /// **Decisions (1) and (2) of nxf 6j6v.de9s, in one tree**: an untracked file is SAVED, an
    /// ignored one is LEFT.
    ///
    /// The asymmetry is the whole of both decisions. An untracked file is the normal output of a
    /// coding agent and the least reproducible thing in the tree, and leaving it behind would be
    /// worse than losing it — a checkout does not remove untracked files, so it would follow the
    /// working copy into the next operation's hands. An ignored file is the project's own statement
    /// that it does not belong in history, and it is mostly the build directory this epic exists to
    /// protect; it stays where it is, exactly as it does across every other hand-off.
    #[test]
    fn a_park_commits_untracked_work_and_leaves_ignored_files_in_the_tree() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        let base = copy.here().unwrap();

        std::fs::write(root.join("kept.txt"), "one\ntwo\n").unwrap(); // a tracked change
        std::fs::write(root.join("new.txt"), "brand new\n").unwrap(); // untracked
        std::fs::create_dir_all(root.join("build")).unwrap();
        std::fs::write(root.join("build/out.bin"), "artifact\n").unwrap(); // ignored

        let parked = park(&copy, "thread:01JAAA", NOW, &base);
        assert!(parked.committed);
        assert!(parked.created_branch);

        // The tree is back on the base, so the SAVED files are gone from it…
        assert_eq!(head_branch(root), "main");
        assert!(!root.join("new.txt").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("kept.txt")).unwrap(),
            "one\n"
        );
        // …and the ignored one was never touched.
        assert_eq!(
            std::fs::read_to_string(root.join("build/out.bin")).unwrap(),
            "artifact\n",
            "an ignored file stays in the working copy — it is not in the commit and not deleted"
        );

        // What the commit actually holds: both the tracked change and the untracked file, and not
        // the ignored one.
        let files = git(root, &["show", "--name-only", "--format=", &parked.commit]);
        assert!(files.contains("new.txt"), "{files}");
        assert!(files.contains("kept.txt"), "{files}");
        assert!(!files.contains("build/"), "{files}");
    }

    /// **"Einen Zweig anlegen nur, falls der Vorgang noch keinen hat"** — the owner's rule, both
    /// halves.
    #[test]
    fn a_park_uses_the_operations_own_branch_and_invents_one_only_when_there_is_none() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        let base = copy.here().unwrap();
        assert_eq!(base.branch, "main");

        // (a) The operation went and made its own branch: the work is committed onto THAT.
        git(root, &["checkout", "-q", "-b", "feat/its-own"]);
        std::fs::write(root.join("work.txt"), "in progress\n").unwrap();
        let parked = park(&copy, "thread:01JAAA", NOW, &base);
        assert_eq!(parked.branch, "feat/its-own");
        assert!(!parked.created_branch);
        assert_eq!(
            head_branch(root),
            "main",
            "and the tree goes back to the base"
        );

        // (b) The operation is still standing on the base: a branch is created for it.
        std::fs::write(root.join("more.txt"), "later\n").unwrap();
        let parked = park(&copy, "thread:01JAAA", "2026-09-05T10:00:00Z", &base);
        assert!(parked.created_branch);
        assert_eq!(parked.branch, "nxs/park/thread-01JAAA/20260905T100000Z");
        assert_eq!(head_branch(root), "main");
    }

    /// **The 2026-09-17 decision (nxf 6j6v.7hqc): a park on a clean tree creates no branch.**
    ///
    /// HEAD sits on the recorded base branch and there is nothing uncommitted — so there is
    /// nothing for a park branch to hold, and the engine never deletes park branches (see the
    /// module doc). Minting one anyway would be clutter nobody ever removes. The park still
    /// "happens" — it reports the operation parked on the base branch itself, with `committed:
    /// false`, exactly as a clean tree on the operation's own (non-base) branch already does.
    #[test]
    fn a_clean_tree_on_the_base_branch_parks_without_creating_a_branch() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        let base = copy.here().unwrap();
        assert_eq!(base.branch, "main");

        let parked = copy
            .commit_onto_a_branch(
                "thread:01JIII",
                NOW,
                &base,
                crate::orchestration::ParkOccasion::StrandedEscalation,
            )
            .unwrap();

        assert!(
            !parked.created_branch,
            "nothing to commit, so no branch is minted"
        );
        assert_eq!(parked.branch, "main");
        assert!(!parked.committed);
        assert_eq!(parked.commit, base.commit);
        assert_eq!(head_branch(root), "main", "the tree never left the base");
        assert_eq!(
            git(root, &["branch", "--list", "nxs/park/*"]),
            "",
            "no empty branch is left behind"
        );
    }

    /// The counterpart to the test above: the new rule is scoped to a CLEAN tree, and a dirty one
    /// on the base branch still gets a park branch exactly as before.
    #[test]
    fn a_dirty_tree_on_the_base_branch_still_gets_a_park_branch() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        let base = copy.here().unwrap();
        std::fs::write(root.join("new.txt"), "uncommitted\n").unwrap();

        let parked = copy
            .commit_onto_a_branch(
                "thread:01JJJJ",
                NOW,
                &base,
                crate::orchestration::ParkOccasion::StrandedEscalation,
            )
            .unwrap();

        assert!(parked.created_branch);
        assert!(parked.branch.starts_with("nxs/park/"), "{}", parked.branch);
        assert!(parked.committed);
        assert_eq!(head_branch(root), parked.branch);
    }

    /// A clean DETACHED HEAD still gets a branch created for it — decision (2026-09-17) is scoped
    /// to HEAD being ON the base branch, and a detached HEAD's position has to stay reachable
    /// somehow, which only a branch (or a tag, which this module never makes) provides.
    #[test]
    fn a_clean_detached_head_still_gets_a_branch() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        let on_main = copy.here().unwrap();
        git(root, &["checkout", "-q", "--detach", &on_main.commit]);
        let base = copy.here().unwrap();
        assert!(base.branch.is_empty(), "HEAD is detached");

        let parked = copy
            .commit_onto_a_branch(
                "thread:01JKKK",
                NOW,
                &base,
                crate::orchestration::ParkOccasion::StrandedEscalation,
            )
            .unwrap();

        assert!(
            parked.created_branch,
            "a detached HEAD's position must stay reachable"
        );
        assert!(parked.branch.starts_with("nxs/park/"), "{}", parked.branch);
        assert!(!parked.committed, "the tree was clean");
    }

    /// **Fix round 1, Finding 1 (review of this task): a moved base under a park that reused ITS
    /// OWN branch must not be reported as two movements of two different things.**
    ///
    /// A clean park on the base branch (`nxf 6j6v.7hqc`) leaves `parked.branch` and
    /// `parked.base_branch` naming the SAME ref. Resolving that ref twice — once as "the branch",
    /// once as "the base" — would report one move as both `branch_now` and `base_now`, and
    /// `resume_notice` would then print a second bullet claiming "your branch does not contain
    /// that work", which is false: `restore(&parked.branch)` checks this very branch back out,
    /// landing exactly on the tip reported here.
    #[test]
    fn a_moved_base_under_a_park_that_reused_it_is_reported_once_and_truthfully() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        let base = copy.here().unwrap();
        assert_eq!(base.branch, "main");

        let parked = copy
            .commit_onto_a_branch(
                "thread:01JLLL",
                NOW,
                &base,
                crate::orchestration::ParkOccasion::StrandedEscalation,
            )
            .unwrap();
        assert_eq!(parked.branch, "main");
        assert_eq!(parked.branch, parked.base_branch);
        assert!(!parked.created_branch && !parked.committed);

        // Somebody else's operation advances `main` while this one sits parked.
        std::fs::write(root.join("theirs.txt"), "theirs\n").unwrap();
        git(root, &["add", "-A"]);
        commit(root, "somebody else's work");
        let new_tip = git(root, &["rev-parse", "main"]);

        let m = copy.movement(&parked);
        assert!(m.any());
        assert_eq!(
            m.branch_now.as_deref(),
            Some(new_tip.as_str()),
            "the one ref that moved is reported once"
        );
        assert!(
            m.base_now.is_none() && !m.base_gone,
            "the base half must not ALSO fire for the same ref: {m:?}"
        );

        let notice = resume_notice(&parked, &Ok(()), &m);
        assert_eq!(
            notice.matches("has moved on").count(),
            1,
            "one movement, one bullet: {notice}"
        );
        assert!(
            !notice.contains("does not contain that work"),
            "false after `restore` checks this very branch back out: {notice}"
        );
        assert!(notice.contains(&parked.commit), "{notice}");
        assert!(notice.contains(&new_tip), "{notice}");
        assert!(
            notice.contains("base branch"),
            "the reader is told this ref is also their base: {notice}"
        );
    }

    /// **Fix round 1, Finding 2 (review of this task): the "nothing new had to be committed"
    /// sentence was written for an operation reusing its OWN branch — wrong when reused verbatim
    /// for a clean park on the BASE branch, where there is no branch of the operation's own that
    /// could have "already carried" anything.**
    #[test]
    fn the_resume_text_for_a_clean_park_on_the_base_branch_says_no_branch_was_made() {
        let (_tmp, copy) = repo();
        let base = copy.here().unwrap();
        let parked = copy
            .commit_onto_a_branch(
                "thread:01JMMM",
                NOW,
                &base,
                crate::orchestration::ParkOccasion::StrandedEscalation,
            )
            .unwrap();
        assert!(!parked.created_branch && !parked.committed);

        let notice = resume_notice(&parked, &Ok(()), &Movement::default());
        assert!(notice.contains("no park branch was made"), "{notice}");
        assert!(
            notice.contains(&format!("base branch, {}", parked.branch)),
            "{notice}"
        );
        assert!(
            !notice.contains("already carried your work"),
            "that sentence is about an operation's OWN branch, not the base it parked on \
             untouched: {notice}"
        );
    }

    /// **Correction 2: THE BASE IS NOT `main`.** An operation that began on a feature branch is put
    /// back on that feature branch — anything else would silently move the work of whoever comes
    /// next onto a foundation nobody chose.
    #[test]
    fn a_park_goes_back_to_the_branch_the_operation_started_on() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        git(root, &["checkout", "-q", "-b", "release/1.2"]);
        std::fs::write(root.join("rel.txt"), "release work\n").unwrap();
        git(root, &["add", "-A"]);
        commit(root, "release work");

        // THIS is where the operation took the working copy.
        let base = copy.here().unwrap();
        assert_eq!(base.branch, "release/1.2");

        std::fs::write(root.join("scratch.txt"), "unfinished\n").unwrap();
        let parked = park(&copy, "thread:01JBBB", NOW, &base);

        assert_eq!(head_branch(root), "release/1.2");
        assert_eq!(parked.base_branch, "release/1.2");
        assert_eq!(parked.base_commit, base.commit);
        assert!(root.join("rel.txt").exists(), "the base's own work is here");
        assert!(!root.join("scratch.txt").exists(), "the parked work is not");
    }

    /// **Decision (3): a tree mid-flight is NOT parked.** Committing a conflicted index would record
    /// a half-finished merge as if it were the operation's work, and leaving the base would abandon
    /// sequencer state git cannot rebuild. So it refuses — and the refusal has to leave everything
    /// exactly as it found it, which is the second half of what is asserted here.
    #[test]
    fn a_tree_in_the_middle_of_a_merge_is_refused_and_left_untouched() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        let base = copy.here().unwrap();

        git(root, &["checkout", "-q", "-b", "other"]);
        std::fs::write(root.join("kept.txt"), "other side\n").unwrap();
        git(root, &["add", "-A"]);
        commit(root, "other");
        git(root, &["checkout", "-q", "main"]);
        std::fs::write(root.join("kept.txt"), "this side\n").unwrap();
        git(root, &["add", "-A"]);
        commit(root, "this");
        // A merge that conflicts and stops, leaving MERGE_HEAD behind.
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.invalid",
                "merge",
                "other",
            ])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(!out.status.success(), "the merge is supposed to conflict");

        let refusal = copy
            .commit_onto_a_branch(
                "thread:01JCCC",
                NOW,
                &base,
                crate::orchestration::ParkOccasion::StrandedEscalation,
            )
            .unwrap_err();
        assert!(
            matches!(refusal, ParkRefusal::MidSequence(_)),
            "{refusal:?}"
        );
        assert!(refusal.detail().contains("a merge is in progress"));
        // …and what finishes it (nxf 6j6v.8bv9) — and nothing about a verb at all. A park message
        // is a pure function of git state (this module's own doc, above), so it never gets to be
        // wrong about which CLI verb is the way out: `nxc release` was the risk this guarded
        // against when the assertion was written, and it left the surface whole (nxf 6j6v.b9nf)
        // without this refusal text having to change.
        assert!(
            refusal.detail().contains("Finish or abort the merge in"),
            "{}",
            refusal.detail()
        );
        assert!(!refusal.detail().contains("nxc "), "{}", refusal.detail());
        assert!(!refusal.is_permanent(), "a merge is finished by somebody");
        // Untouched: still on the branch, still mid-merge, no park branch invented.
        assert_eq!(head_branch(root), "main");
        assert!(root.join(".git/MERGE_HEAD").exists());
        assert_eq!(git(root, &["branch", "--list", "nxs/park/*"]), "");
    }

    /// Two branches whose tips change the SAME line — the cheapest way to make every sequencing
    /// command git has stop mid-flight FOR REAL, rather than by writing its marker file by hand.
    fn conflicting_history(root: &Path) {
        git(root, &["checkout", "-q", "-b", "other"]);
        std::fs::write(root.join("kept.txt"), "other side\n").unwrap();
        git(root, &["add", "-A"]);
        commit(root, "other");
        git(root, &["checkout", "-q", "main"]);
        std::fs::write(root.join("kept.txt"), "this side\n").unwrap();
        git(root, &["add", "-A"]);
        commit(root, "this");
    }

    /// A git command that is SUPPOSED to stop on a conflict, under an explicit identity (the
    /// repository has none) — and asserted to have stopped, so a case that quietly succeeded cannot
    /// go on to assert nothing.
    fn git_stops_mid_sequence(root: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
            .args(args)
            .current_dir(root)
            .output()
            .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
        assert!(
            !out.status.success(),
            "git {args:?} was supposed to stop on a conflict: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }

    /// **Every sequence a tree can stop in, driven through REAL git** (independent review of PR
    /// #478, Test Quality #3).
    ///
    /// [`WorkingCopy::mid_sequence`] names seven markers and nxf 6j6v.8bv9 reworded the "what to
    /// fix" half of every row; only the merge was ever produced for real, so a wrong word in any
    /// other row passed the whole suite green. Each case here runs the actual git command and lets
    /// it stop where it stops — a marker file written by the test would only ever prove that the
    /// test and the table agree with each other.
    ///
    /// The merge row is here for completeness of the table and keeps its own case next door
    /// ([`a_tree_in_the_middle_of_a_merge_is_refused_and_left_untouched`]), which asserts the
    /// things that are about the refusal rather than about the marker.
    #[test]
    fn every_sequence_git_can_stop_in_is_refused_by_the_name_that_finishes_it() {
        for (sequence, marker, what, which) in [
            (
                &["merge", "other"][..],
                "MERGE_HEAD",
                "a merge is in progress",
                "the merge",
            ),
            (
                &["cherry-pick", "other"],
                "CHERRY_PICK_HEAD",
                "a cherry-pick is in progress",
                "the cherry-pick",
            ),
            (
                &["revert", "other"],
                "REVERT_HEAD",
                "a revert is in progress",
                "the revert",
            ),
            (
                &["rebase", "other"],
                "rebase-merge",
                "a rebase is in progress",
                "the rebase",
            ),
            // The `am` backend, which is the other half of the row that names `git am`. It is the
            // one sequence whose marker a modern git writes only when it is asked for by name.
            (
                &["rebase", "--apply", "other"],
                "rebase-apply",
                "a rebase or `git am` is in progress",
                "the rebase or `git am`",
            ),
        ] {
            let (tmp, copy) = repo();
            let root = tmp.path();
            conflicting_history(root);
            let base = copy.here().unwrap();
            git_stops_mid_sequence(root, sequence);
            assert!(
                root.join(".git").join(marker).exists(),
                "git {sequence:?} was supposed to leave {marker} behind"
            );

            let refusal = copy
                .commit_onto_a_branch(
                    "thread:01JSEQ",
                    NOW,
                    &base,
                    crate::orchestration::ParkOccasion::StrandedEscalation,
                )
                .unwrap_err();
            let said = refusal.detail().to_string();
            assert!(
                matches!(refusal, ParkRefusal::MidSequence(_)),
                "git {sequence:?}: {refusal:?}"
            );
            assert!(said.contains(what), "git {sequence:?}: {said}");
            assert!(said.contains(marker), "git {sequence:?}: {said}");
            assert!(
                said.contains(&format!("Finish or abort {which} in")),
                "the reader is told which sequence to finish, by the name it has — git \
                 {sequence:?}: {said}"
            );
            assert!(
                !refusal.is_permanent(),
                "every sequence ends when somebody ends it — git {sequence:?}"
            );
            // The refusal leaves the one thing no later command can rebuild exactly as it found it,
            // and invents no branch on the way past.
            assert!(
                root.join(".git").join(marker).exists(),
                "git {sequence:?}: the sequencer state is still there"
            );
            assert_eq!(git(root, &["branch", "--list", "nxs/park/*"]), "");
        }
    }

    /// **A bisect stops a tree too, and BOTH markers say so** (independent review of PR #478, Test
    /// Quality #3).
    ///
    /// `git bisect start` is the one sequence that does not need a conflict to begin, and on this
    /// git it writes `BISECT_LOG` and `BISECT_START` together. The second row exists because which
    /// of the two a given git writes is a version detail (independent review of PR #435, Integrity
    /// #5), so the only way to reach it is to stand where a git that writes only `BISECT_START`
    /// would leave the repository: git made both files here and the log is then taken away again.
    /// Nothing is written by hand, and the state the second half asserts against is one a real git
    /// produces.
    #[test]
    fn a_tree_left_mid_bisect_is_refused_by_either_marker_git_left() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        let base = copy.here().unwrap();
        git(root, &["bisect", "start"]);

        let log = root.join(".git/BISECT_LOG");
        let start = root.join(".git/BISECT_START");
        assert!(log.exists() && start.exists(), "this git writes both");
        for marker in [&log, &start] {
            let refusal = copy
                .commit_onto_a_branch(
                    "thread:01JBIS",
                    NOW,
                    &base,
                    crate::orchestration::ParkOccasion::StrandedEscalation,
                )
                .unwrap_err();
            assert!(
                matches!(refusal, ParkRefusal::MidSequence(_)),
                "{refusal:?}"
            );
            let said = refusal.detail();
            assert!(said.contains("a bisect is in progress"), "{said}");
            assert!(said.contains("Finish or abort the bisect in"), "{said}");
            assert!(!refusal.is_permanent(), "a bisect is ended by somebody");
            // …and now stand where a git that writes only the OTHER file leaves things.
            std::fs::remove_file(marker).unwrap();
        }
        assert_eq!(git(root, &["branch", "--list", "nxs/park/*"]), "");
    }

    /// **A git command that genuinely fails is a TRANSIENT refusal, and the retry gets the work**
    /// (independent review of PR #478, Test Quality #3).
    ///
    /// [`ParkRefusal::Git`] was only ever exercised from a hand-injected row, so nothing proved
    /// that a real failure is classified as transient, names the command that failed, or leaves the
    /// state the retry needs. The failure here is the one the runner's own `index.lock` reasoning is
    /// about and the cheapest real one there is: a lock file in the way wedges every git command in
    /// a working copy until it is gone. It belongs to nobody here, so [`WorkingCopy::git`] leaves it
    /// alone — removing somebody else's lock is exactly what it refuses to do.
    #[test]
    fn a_git_command_that_really_fails_is_transient_and_the_retry_parks_the_work() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        let base = copy.here().unwrap();
        std::fs::write(root.join("mine.txt"), "half finished\n").unwrap();

        let lock = root.join(".git/index.lock");
        std::fs::write(&lock, "").unwrap();
        let refusal = copy
            .commit_onto_a_branch(
                "thread:01JLCK",
                NOW,
                &base,
                crate::orchestration::ParkOccasion::StrandedEscalation,
            )
            .unwrap_err();
        let said = refusal.detail().to_string();
        assert!(matches!(refusal, ParkRefusal::Git(_)), "{refusal:?}");
        assert!(
            said.contains("`git add -A` failed"),
            "the refusal names the command that failed, not just that one did: {said}"
        );
        assert!(
            said.contains("index.lock"),
            "…and what git said about it: {said}"
        );
        assert!(
            !refusal.is_permanent(),
            "a lock somebody is holding goes when they let go of it"
        );
        assert!(
            lock.exists(),
            "a lock this call did not create is left where it is"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("mine.txt")).unwrap(),
            "half finished\n",
            "and the work is still in the tree, unsaved but not lost"
        );

        // The retry, once the lock is gone: the branch the failed attempt made is REUSED rather
        // than a second one minted, which is the shape `commit_onto_a_branch`'s own doc promises a
        // failure leaves behind.
        std::fs::remove_file(&lock).unwrap();
        let parked = copy
            .commit_onto_a_branch(
                "thread:01JLCK",
                "2026-09-05T10:00:00Z",
                &base,
                crate::orchestration::ParkOccasion::StrandedEscalation,
            )
            .unwrap();
        assert!(parked.committed, "{parked:?}");
        assert!(
            !parked.created_branch,
            "the first attempt already made it: {parked:?}"
        );
        assert_eq!(parked.branch, "nxs/park/thread-01JLCK/20260905T090000Z");
        let files = git(root, &["show", "--name-only", "--format=", &parked.commit]);
        assert!(files.contains("mine.txt"), "{files}");
    }
    /// **The retry after a failed record does not mint an empty decoy branch** (independent review
    /// of PR #435, Integrity #1 / Code Quality #1 — found by two reviewers separately).
    ///
    /// The shape being pinned is the one the split exists for. A park that committed and then failed
    /// to record leaves the tree ON the parked branch, because the return to base is a separate step
    /// the caller only reaches after the write. The next attempt therefore takes the "the operation
    /// already has a branch" arm and reuses it — same branch, same commit, nothing minted.
    ///
    /// What the old shape did instead, and why the assertion below is the whole finding: it returned
    /// the tree to the base inside the same call, so the retry met a clean tree on the base,
    /// concluded that the operation had no branch, and created a second one — empty. The row that
    /// eventually landed named that decoy while the real commit sat on an unreferenced ref, and the
    /// resume text went on to say there had been nothing to save.
    #[test]
    fn a_retry_after_a_failed_record_reuses_the_branch_instead_of_minting_an_empty_one() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        let base = copy.here().unwrap();
        std::fs::write(root.join("mine.txt"), "half finished\n").unwrap();

        // Attempt one gets as far as the commit; the caller's write then fails, so `return_to_base`
        // is never reached.
        let first = copy
            .commit_onto_a_branch(
                "thread:01JHHH",
                NOW,
                &base,
                crate::orchestration::ParkOccasion::StrandedEscalation,
            )
            .unwrap();
        assert!(first.created_branch && first.committed);
        assert_eq!(
            head_branch(root),
            first.branch,
            "the tree stays on the branch"
        );

        // Attempt two, one minute later — a different instant, so a fresh name WOULD be available.
        let second = copy
            .commit_onto_a_branch(
                "thread:01JHHH",
                "2026-09-05T09:01:00Z",
                &base,
                crate::orchestration::ParkOccasion::StrandedEscalation,
            )
            .unwrap();
        assert_eq!(second.branch, first.branch, "the same branch, reused");
        assert_eq!(second.commit, first.commit, "and the same commit");
        assert!(!second.created_branch);
        assert_eq!(
            git(root, &["branch", "--list", "nxs/park/*"])
                .lines()
                .count(),
            1,
            "exactly one park branch exists — a second, empty one is the defect"
        );
        copy.return_to_base(&base).unwrap();
        assert_eq!(head_branch(root), "main");

        // …and the text that reuse produces does not claim the branch is empty.
        let notice = resume_notice(&second, &Ok(()), &Movement::default());
        assert!(
            notice.contains("this branch already carried your work"),
            "{notice}"
        );
        assert!(
            !notice.contains("nothing uncommitted to save"),
            "a reused branch must never be reported as an empty one: {notice}"
        );
    }

    /// **The ignored-files caveat is in the text, word for word** (independent review of PR #435,
    /// Test Quality #3).
    ///
    /// Decision (2) of the item is that ignored files are NOT parked; the DoD, both guide pages and
    /// `resume_notice`'s own doc all call the sentence load-bearing, and nothing asserted it. A
    /// caveat that quietly stops being printed is worse than one that was never promised — a session
    /// would then read a text that says everything was saved.
    #[test]
    fn the_resume_text_always_says_what_was_not_saved() {
        let parked = Parked {
            branch: "nxs/park/thread-x/20260905T090000Z".into(),
            commit: "abc123".into(),
            base_branch: "main".into(),
            base_commit: "def456".into(),
            created_branch: true,
            committed: true,
        };
        for restored in [Ok(()), Err("git checkout refused".to_string())] {
            let notice = resume_notice(&parked, &restored, &Movement::default());
            assert!(
                notice.contains("Files your `.gitignore` excludes were NOT committed."),
                "the caveat has to survive both outcomes of the checkout: {notice}"
            );
            assert!(
                notice.contains("left in the working copy"),
                "…and it has to say where they are: {notice}"
            );
        }
    }

    /// **Correction 4, against a real repository**: the case the draft's own check could not see —
    /// the parked branch untouched while the BASE ran on.
    #[test]
    fn movement_sees_a_base_that_ran_on_under_an_untouched_branch() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        let base = copy.here().unwrap();
        std::fs::write(root.join("mine.txt"), "mine\n").unwrap();
        let parked = park(&copy, "thread:01JDDD", NOW, &base);
        assert!(!copy.movement(&parked).any(), "nothing has happened yet");

        // Somebody else's operation had the copy and pushed the base on. OUR branch is untouched —
        // its tip is still exactly the commit we parked — so the draft's single check would report
        // calm here and the work would resume on a foundation that has moved.
        std::fs::write(root.join("theirs.txt"), "theirs\n").unwrap();
        git(root, &["add", "-A"]);
        commit(root, "somebody else's work");

        let m = copy.movement(&parked);
        assert!(m.any());
        assert!(m.branch_now.is_none(), "our branch really did not move");
        assert_eq!(
            m.base_now.as_deref(),
            Some(git(root, &["rev-parse", "main"]).as_str())
        );
        assert!(!m.base_gone && !m.branch_gone);

        // And the text a resumed session reads says so, by name and by hash.
        let notice = resume_notice(&parked, &Ok(()), &m);
        assert!(notice.contains(&parked.branch));
        assert!(notice.contains(&parked.commit));
        assert!(notice.contains("the base main has moved on"), "{notice}");
    }

    /// **A deleted branch is reported as GONE, not as moved** — "it moved" would send a reader
    /// looking for commits that are not there, when what they need is the commit hash.
    #[test]
    fn a_deleted_park_branch_is_reported_as_gone_with_the_commit_still_named() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        let base = copy.here().unwrap();
        std::fs::write(root.join("mine.txt"), "mine\n").unwrap();
        let parked = park(&copy, "thread:01JEEE", NOW, &base);
        git(root, &["branch", "-D", &parked.branch]);

        let m = copy.movement(&parked);
        assert!(m.branch_gone);
        let notice = resume_notice(&parked, &Err("no such branch".into()), &m);
        assert!(notice.contains("no longer resolves"), "{notice}");
        assert!(notice.contains(&parked.commit), "{notice}");
        assert!(
            notice.contains("could NOT be put back"),
            "a failed checkout must never read as a successful one: {notice}"
        );
    }

    /// **`restore` lets git decide, and git refuses to overwrite somebody else's uncommitted work.**
    ///
    /// The alternative — forcing, or stashing — would trade a stranger's work for our own, which is
    /// not a trade this engine makes on its own. The resume still happens; the session is told.
    #[test]
    fn restore_does_not_overwrite_the_next_operations_uncommitted_work() {
        let (tmp, copy) = repo();
        let root = tmp.path();
        let base = copy.here().unwrap();
        std::fs::write(root.join("kept.txt"), "my unfinished edit\n").unwrap();
        let parked = park(&copy, "thread:01JFFF", NOW, &base);

        // The operation that got the copy next is mid-edit on the same file.
        std::fs::write(root.join("kept.txt"), "somebody else is editing this\n").unwrap();
        let said = copy.restore(&parked.branch).unwrap_err();
        assert!(said.contains("git checkout"), "{said}");
        assert_eq!(
            std::fs::read_to_string(root.join("kept.txt")).unwrap(),
            "somebody else is editing this\n",
            "their work is untouched"
        );

        // With the tree clean, the same call succeeds and the parked work is back.
        git(root, &["checkout", "-q", "--", "kept.txt"]);
        copy.restore(&parked.branch).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("kept.txt")).unwrap(),
            "my unfinished edit\n"
        );
    }

    /// A working copy that is not a git repository refuses by name rather than failing obscurely —
    /// and so does an operation asked to park in one.
    #[test]
    fn a_directory_that_is_not_a_repository_says_so() {
        let tmp = tempfile::TempDir::new().unwrap();
        let copy = WorkingCopy::at(tmp.path());
        let refusal = copy
            .commit_onto_a_branch(
                "thread:01JGGG",
                NOW,
                &Base {
                    branch: "main".into(),
                    commit: "0".into(),
                },
                crate::orchestration::ParkOccasion::StrandedEscalation,
            )
            .unwrap_err();
        assert!(
            matches!(refusal, ParkRefusal::NotARepository(_)),
            "{refusal:?}"
        );
        assert!(refusal.detail().contains("not a git repository"));
    }

    #[test]
    fn movement_is_nothing_when_neither_end_has_moved() {
        let m = Movement::default();
        assert!(!m.any());
    }

    #[test]
    fn a_moved_base_is_movement_even_when_the_branch_is_untouched() {
        // Correction 4, as a value: the case the draft's own check could not see.
        let m = Movement {
            base_now: Some("deadbeef".into()),
            ..Movement::default()
        };
        assert!(m.any());
    }
}
