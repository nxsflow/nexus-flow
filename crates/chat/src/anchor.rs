//! **Where the working copy stood when a turn changed hands** (nxf 6j6v.2af2) — the commit anchor
//! every `send` and `reply` writes onto its message, and the fingerprint that makes it COMPARABLE
//! later.
//!
//! A message has always said WHO said WHAT and WHEN. It did not say **on which state of the working
//! copy that held**, so afterwards nobody could reconstruct what a reviewer actually had in front of
//! it, and an interrupted operation could not be continued where it stopped — because nothing knew
//! where that was. This module is the fact; [`crate::orchestration`] is the seam that stamps it, and
//! [`crate::facade`] is where it comes back.
//!
//! ## Two jobs, two artefacts (owner, 2026-09-12, precision 1)
//!
//! > *Erkennen und Wiederherstellen sind zwei Aufgaben mit zwei Artefakten.*
//!
//! * **Comparing** — "has the tree moved while this operation waited?" — wants a FINGERPRINT: cheap,
//!   fixed-width, and answerable without touching a file. That is what [`Anchor::fingerprint`] is,
//!   and [`Anchor::drift_from`] is the comparison.
//! * **Restoring** — putting uncommitted work back — wants a COMMIT, and a `git diff` is a poor
//!   substitute for one (it carries no untracked files and can fail to apply). The commit that
//!   secures held work is [`crate::park`]'s, and it stays there; see "What this does NOT do" below.
//!
//! ## The fingerprint covers three things, because any two of them miss the commonest case
//!
//! `HEAD` + the tracked diff + the porcelain status (precision 2): a newly created, UNTRACKED file
//! appears in neither `HEAD` nor `git diff`, and a coding agent produces those constantly. A
//! fingerprint over the first two alone would call that tree unchanged.
//!
//! **What no part of it sees is ignored files** (precision 3) — `.env`, a local database,
//! `node_modules`. That belongs in the promise and not in the later disappointment, and it is the
//! same boundary [`crate::park`] draws for what it commits. It is also why `.nxs/` — this
//! workspace's own board and channel — is outside the anchor in every direction: git does not track
//! it, so nothing here records it and nothing built on this can roll it back. **The repository
//! rewinds; the record does not.**
//!
//! ## Bytes, not text (independent review of PR #474, Integrity #1)
//!
//! `git status --porcelain` and `git diff HEAD` emit REPOSITORY CONTENT, and repository content is
//! bytes: a tracked file in Windows-1252 or Shift-JIS produces a diff that is not valid UTF-8, and
//! git does not call it binary. Both reads therefore go through
//! [`crate::park::WorkingCopy::git_bytes`] and are hashed as bytes — the fingerprint never needed
//! text in the first place, and requiring it was an invitation for the one failure this module must
//! not have: a decode error swallowed into an empty string, on a command that exited 0, turning a
//! dirty tree into `dirty: false` with a fingerprint of nothing.
//!
//! ## What this does NOT do: it never writes
//!
//! Every read here is a read. Five `git` invocations over four verbs (`rev-parse` twice, then
//! `symbolic-ref`, `status`, `diff`), no ref moved, no file touched, no index written.
//!
//! That is a decision and not an omission. A handover point is exactly where a live session is
//! sitting in the working copy: `nxc reply` is what an agent runs from INSIDE its own session, and
//! committing its tree onto a branch underneath it — which is what [`crate::park`] does — would
//! move HEAD out from under the process still working there. [`crate::park`]'s own text says the
//! window it accepts is narrow *"because a park only ever runs against an area with no live session
//! in it"*, and the handover is the one moment where that is false. Securing uncommitted work when
//! the working copy CHANGES HANDS is a decision of its own, is known, and is nxf 6j6v.8bv9's — which
//! says in its own words that committing there is *"eine Entscheidung, keine Vervollstaendigung"*.
//! So a dirty tree at a handover is recorded HONESTLY ([`Anchor::dirty`]) rather than quietly
//! committed.
//!
//! ## What is NOT reported per message, and what stands in its place
//!
//! A handover that takes no anchor posts its message unchanged and says nothing on its own receipt.
//! Two of the three reasons were never a failure at all ([`AnchorRefusal::NoWorkingCopy`],
//! [`AnchorRefusal::NotARepository`]): nothing was owed, and a warning on every message of every
//! host that runs no git is precisely the breadcrumb that teaches a reader to ignore the channel it
//! arrives on. What a reader needs there is to be told ONCE that this workspace records no anchors,
//! and that is [`crate::facade::StatusReport::worker_names_a_working_copy`] — the shape
//! `worker_answers_liveness` beside it already has.
//!
//! **The residual is [`AnchorRefusal::Git`]**, and it is named here rather than left to be
//! discovered: git exists, the repository exists, and a read failed — nothing on the receipt says
//! so, and the absent anchor is indistinguishable from a workspace that takes none. It is accepted
//! because a working copy whose `git rev-parse` fails is already refusing every park, every declared
//! hurdle and every liveness read in far louder ways, so a fourth quiet symptom buys little. If it
//! ever turns out to be the one that surfaces first, the answer is a
//! [`crate::orchestration::FailedConsequence`] on the two receipts that carry one, and the place to
//! produce it is the stamping seam that already holds this refusal.
//!
//! ## No model call, and nothing that can hang
//!
//! [`crate::park`]'s rule, verbatim, because this path is wider than that one: it runs on every
//! handover, inside the engine's held store mutex. Every command is local, deterministic and bounded
//! by [`crate::park`]'s own `GIT_BOUND` (this reuses that runner rather than starting a second one),
//! and a failure is a named refusal that costs the message nothing — the anchor is a fact ABOUT the
//! handover, never a precondition FOR it.

use sha2::{Digest, Sha256};

use crate::park::WorkingCopy;

/// **Where the working copy stood at one handover** — a `send` or a `reply`, stamped onto that
/// message's [`crate::model::Refs`].
///
/// On the MESSAGE and not in a side record, because it is a fact about the world at the moment of
/// the handover and the message is what carries that moment. Mechanically it rides
/// `messages.refs`, which is a JSON text column, so it costs no schema change and no refold — the
/// same route [`crate::model::Refs::substituted`] took and for the same reasons.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Anchor {
    /// The commit `HEAD` pointed at. Full 40 hex digits, never abbreviated: an abbreviation is
    /// ambiguous in a repository that has grown since, and a reader that wants a short form can
    /// always take one (see [`Anchor::short`]).
    pub commit: String,
    /// The branch `HEAD` was on, or **empty for a detached HEAD** — kept rather than normalised
    /// away, exactly as [`crate::park::Base::branch`] keeps it: a detached HEAD is a real state a
    /// working copy can be in, and [`commit`](Anchor::commit) names the point exactly either way.
    pub branch: String,
    /// **Something was uncommitted when this handover happened** — tracked changes, or untracked
    /// files, or both.
    ///
    /// It is the honest half of the record and the reason this type does not pretend a commit
    /// describes the world: a coder hands over MID-WORK, which is the normal case and not the
    /// exception, and [`commit`](Anchor::commit) alone would describe that tree wrongly. `true`
    /// means *the commit is where this work started from, not what it looks like* — the work itself
    /// was not saved anywhere by this anchor (see the module docs on why this path does not commit).
    pub dirty: bool,
    /// **The comparable summary** — `sha256(commit + porcelain + diff)`, hex.
    ///
    /// One value, so a comparison is an equality test rather than three readings that can each be
    /// half-right, and fixed-width, so it can be carried on every message without regard to how
    /// large the change is. [`fingerprint_of`] is the derivation and is pure.
    pub fingerprint: String,
}

impl Anchor {
    /// The first twelve digits of [`commit`](Anchor::commit) — what a human line prints.
    ///
    /// Twelve rather than seven: git's own default abbreviation grows with the repository, and a
    /// terminal line that says one thing today and another in a year is not a thing a reader can
    /// learn. It is a DISPLAY form only; nothing compares or looks anything up by it.
    pub fn short(&self) -> &str {
        let n = self.commit.len().min(12);
        &self.commit[..n]
    }

    /// **Has the working copy moved since this anchor was taken?** — the comparison the owner's
    /// decision of 2026-09-12 asks for, in the words it asks for it in: *"bei der Wiederaufnahme
    /// vergleichen. Ist der Zustand gewandert, geht eine Frage zurueck an den anfaenglichen
    /// Faden"*.
    ///
    /// `self` is NOW (a fresh [`WorkingCopy::anchor`]), `then` is what the handover recorded. Pure,
    /// so the whole of the comparison is testable without a repository, and so the one place that
    /// decides what "moved" means cannot be spread over its callers.
    ///
    /// **Its caller is the resume** (nxf 6j6v.npy3, which is blocked on exactly this ticket): a
    /// fresh session started at an interrupted step compares the tree it finds against the anchor
    /// of the handover it is continuing, and asks the root thread when the two disagree. It is
    /// public rather than crate-private because that consumer is named, the fact it compares is
    /// public, and an embedding host resuming its own runs needs the same answer.
    pub fn drift_from(&self, then: &Anchor) -> Drift {
        Drift {
            commit_now: (self.commit != then.commit).then(|| self.commit.clone()),
            branch_now: (self.branch != then.branch).then(|| self.branch.clone()),
            // **Only meaningful while the commit has NOT moved**, and that is a property of the
            // fingerprint rather than a simplification: the diff half of it is measured AGAINST
            // HEAD, so two fingerprints taken at different commits are not comparable at all. A
            // moved commit therefore subsumes this question instead of answering it wrongly —
            // whoever reads `commit_now` has to look at the history anyway.
            uncommitted_now: self.commit == then.commit && self.fingerprint != then.fingerprint,
        }
    }
}

/// **What moved between two anchors** ([`Anchor::drift_from`]) — three separate answers, because
/// they call for three different things.
///
/// Modelled on [`crate::park::Movement`] next door and reported the same way: each half on its own,
/// never folded into one "it moved" that sends a reader looking in the wrong place. A moved commit
/// is history to re-read, a moved branch is a tree standing somewhere else, and moved uncommitted
/// work is somebody's unsaved changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Drift {
    /// `HEAD` is a different commit now — the new one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_now: Option<String>,
    /// `HEAD` is on a different branch now — the new one, or **empty for a detached HEAD**, which
    /// is a real answer and not a missing one (see [`Anchor::branch`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_now: Option<String>,
    /// The commit and the branch are where they were and **the uncommitted work is not** — somebody
    /// edited, added or removed something in the tree since. See [`Anchor::drift_from`] for why this
    /// is only ever answered while the commit stands still.
    pub uncommitted_now: bool,
}

impl Drift {
    /// Did anything move at all? The one question a resume branches on.
    pub fn any(&self) -> bool {
        self.commit_now.is_some() || self.branch_now.is_some() || self.uncommitted_now
    }
}

/// **Why no anchor was taken** — every one of these leaves the handover exactly as it was.
///
/// The direction is [`crate::park::ParkRefusal`]'s: the message is worth more posted than lost, so
/// nothing here can fail a `send` or a `reply`. What each variant buys is that the ABSENCE of an
/// anchor is a named absence rather than an empty field a reader has to guess about.
///
/// Its own enum and not [`crate::park::ParkRefusal`], although the shapes rhyme: that type's prose
/// is written into it at the point of refusal and says *"there is nowhere to park this operation's
/// work"*, which is a true sentence about a park and a false one about a message. A refusal that
/// describes the wrong mechanism is worse than one that describes none.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "refusal", content = "detail")]
pub enum AnchorRefusal {
    /// [`crate::worker::Worker::working_copy`] answered `None` — a host that does not run its
    /// sessions in a directory this process can see, which is the DEFAULT for every worker except
    /// the shipped sidecar.
    ///
    /// **Not a failure and never reported as one.** There is no tree to describe, so nothing was
    /// owed; what a reader needs is to be told once that this workspace records no anchors at all,
    /// which is [`crate::facade::StatusReport::worker_names_a_working_copy`].
    NoWorkingCopy(String),
    /// There is a directory and git does not know it. Nothing here can describe a state git cannot
    /// see, and — like the variant above — nothing was owed: a project that is not a repository is
    /// a legitimate thing to run sessions in.
    NotARepository(String),
    /// **A git command failed where one should have worked** — the one shape of this enum that IS a
    /// failed consequence: there is a working copy, it is a repository, and the read did not
    /// happen. An unborn HEAD (a repository with no commit yet) lands here too, which is right: it
    /// is a state with genuinely nothing to anchor to.
    ///
    /// The string is the command and what git said, verbatim and capped by
    /// [`crate::park::WorkingCopy::git`].
    Git(String),
}

impl AnchorRefusal {
    /// The one-line prose every caller prints, with no caller left to invent its own —
    /// [`crate::park::ParkRefusal::detail`]'s discipline at the second seam that needs it.
    pub fn detail(&self) -> &str {
        match self {
            AnchorRefusal::NoWorkingCopy(s)
            | AnchorRefusal::NotARepository(s)
            | AnchorRefusal::Git(s) => s,
        }
    }
}

/// **The fingerprint, as a pure function of its three inputs** — so the one thing a comparison
/// stands on is computable, and testable, without a repository.
///
/// `sha256` over the three, each length-prefixed. The prefixes are not decoration: a bare
/// concatenation lets one input end where the next begins, so a diff whose last line looks like a
/// porcelain entry could produce the fingerprint of a genuinely different tree. It costs eight bytes
/// and removes the whole class.
///
/// **The two repository reads are BYTES** and the commit is text, which is the honest typing of what
/// they are: a commit id is 40 hex digits by construction, while a diff is whatever is in the files
/// (see the module docs on why that distinction is load-bearing rather than tidy).
pub fn fingerprint_of(commit: &str, porcelain: &[u8], diff: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update((commit.len() as u64).to_be_bytes());
    h.update(commit.as_bytes());
    for part in [porcelain, diff] {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part);
    }
    format!("{:x}", h.finalize())
}

/// ASCII whitespace off both ends of a byte slice — [`str::trim`] for the reads that never became
/// text. Used for the DIRTY question, where a lone newline must not read as work.
fn trim_ascii(b: &[u8]) -> &[u8] {
    let start = b.iter().position(|c| !c.is_ascii_whitespace());
    let Some(start) = start else { return &[] };
    let end = b
        .iter()
        .rposition(|c| !c.is_ascii_whitespace())
        .unwrap_or(start);
    &b[start..=end]
}

impl WorkingCopy {
    /// **Read where this working copy stands right now** — the anchor a handover stamps, and the
    /// `now` half of [`Anchor::drift_from`].
    ///
    /// Four reads and nothing else, in this order and each for its own reason:
    ///
    /// 1. `rev-parse --git-dir` — is this a repository at all ([`AnchorRefusal::NotARepository`])?
    ///    Asked first, so a non-repository gets the refusal that names its state rather than
    ///    whatever the next command happens to say about it.
    /// 2. `rev-parse HEAD` — the commit. An unborn HEAD fails here and is
    ///    [`AnchorRefusal::Git`]: a repository with no commit has nothing to anchor to, and saying
    ///    so is better than inventing a zero sha.
    /// 3. `status --porcelain --untracked-files=all` — the tracked states AND every untracked file.
    ///    `all` rather than [`crate::park`]'s `normal`, and the difference is the point: `normal`
    ///    collapses an untracked directory to one line, so a second file appearing inside it would
    ///    drift through unseen. This read answers "is it the same tree", not "is there something to
    ///    commit".
    /// 4. `diff HEAD --no-textconv` — staged AND unstaged changes to tracked files in one command,
    ///    which is exactly the content `status` cannot see.
    ///
    ///    **`--no-textconv` because a `textconv` driver is a PROGRAM** (independent review of PR
    ///    #474, Integrity #3): a repository may configure one per path, and `git diff` then runs it
    ///    over every matching file to produce the text it shows. That is an arbitrary process on the
    ///    ordinary handover path — the one thing this module's own rule forbids — and it would make
    ///    the fingerprint a function of that program's output rather than of the tree. The raw diff
    ///    is also the better comparison: a converter that normalises its input hides exactly the
    ///    drift this exists to see.
    ///
    /// Steps 3 and 4 read BYTES ([`crate::park::WorkingCopy::git_bytes`]); see the module docs.
    ///
    /// The branch comes off the same `symbolic-ref` [`crate::park::WorkingCopy::here`] uses, where a
    /// detached HEAD is an empty answer rather than an error.
    ///
    /// **One call per handover MESSAGE, and a fan-out pays it per member.** A channel opening for
    /// five members takes five anchor reads — which is what makes each member's own commission
    /// carry the state IT was handed, and which is nothing beside the five sessions that opening
    /// starts.
    pub fn anchor(&self) -> std::result::Result<Anchor, AnchorRefusal> {
        self.git(&["rev-parse", "--git-dir"]).map_err(|_| {
            AnchorRefusal::NotARepository(format!(
                "the working copy at {} is not a git repository, so this handover cannot record \
                 where the tree stood — the message is posted either way",
                self.root().display()
            ))
        })?;
        let commit = self
            .git(&["rev-parse", "HEAD"])
            .map_err(AnchorRefusal::Git)?;
        // `--quiet` so a detached HEAD is an empty answer instead of a message on stderr; the
        // command still exits non-zero there, which is why this is `unwrap_or_default` and not a
        // refusal. [`crate::park::WorkingCopy::here`] reads it the same way.
        let branch = self
            .git(&["symbolic-ref", "--quiet", "--short", "HEAD"])
            .unwrap_or_default();
        let porcelain = self
            .git_bytes(&["status", "--porcelain", "--untracked-files=all"])
            .map_err(AnchorRefusal::Git)?;
        let diff = self
            .git_bytes(&["diff", "HEAD", "--no-textconv"])
            .map_err(AnchorRefusal::Git)?;
        Ok(Anchor {
            fingerprint: fingerprint_of(&commit, &porcelain, &diff),
            // **Derived from the porcelain read already taken**, not from a second `status` call:
            // two reads either side of a write would let a message say "clean" about a tree whose
            // fingerprint it took while dirty.
            dirty: !trim_ascii(&porcelain).is_empty(),
            commit,
            branch,
        })
    }
}

/// **The anchor for a host that may not have one** — the whole of what a stamping seam does.
///
/// One function rather than the three steps at each call site, because there are four such sites
/// ([`crate::orchestration`]'s handover points) and the answer to "what does a host with no working
/// copy get" must be the same at all of them.
pub fn anchor_for(
    worker: &dyn crate::worker::Worker,
) -> std::result::Result<Anchor, AnchorRefusal> {
    let Some(root) = worker.working_copy() else {
        return Err(AnchorRefusal::NoWorkingCopy(
            "this workspace's runtime does not run its sessions in a directory this process can \
             see, so a handover cannot record where the working copy stood"
                .to_string(),
        ));
    };
    WorkingCopy::at(root).anchor()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fingerprint_is_a_pure_function_of_its_three_inputs() {
        // The property the whole comparison stands on: same inputs, same value, every time and with
        // no repository, no network and no second process.
        let a = fingerprint_of("abc", b" M src.txt\n", b"@@ -1 +1 @@\n");
        let b = fingerprint_of("abc", b" M src.txt\n", b"@@ -1 +1 @@\n");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64, "sha256, hex");
    }

    #[test]
    fn each_of_the_three_inputs_moves_the_fingerprint_on_its_own() {
        // Precision 2 of the owner's note, as a test: an untracked file shows up ONLY in the
        // porcelain output, so a fingerprint that folded that input away would call the commonest
        // change of all — an agent creating a file — no change at all.
        let base = fingerprint_of("abc", b"", b"");
        assert_ne!(base, fingerprint_of("abd", b"", b""), "the commit");
        assert_ne!(
            base,
            fingerprint_of("abc", b"?? new.txt\n", b""),
            "untracked"
        );
        assert_ne!(
            base,
            fingerprint_of("abc", b"", b"@@ -1 +1 @@\n"),
            "the diff"
        );
    }

    #[test]
    fn the_three_inputs_cannot_bleed_into_each_other() {
        // Why the lengths are prefixed. Without it these two trees — one with a porcelain entry and
        // an empty diff, one with neither and that text in the diff — hash identically, and two
        // genuinely different states would compare equal.
        assert_ne!(
            fingerprint_of("abc", b"?? a\n", b""),
            fingerprint_of("abc", b"", b"?? a\n")
        );
    }

    #[test]
    fn a_diff_that_is_not_valid_utf8_still_fingerprints_and_still_differs() {
        // Integrity #1 of the review of PR #474, at the pure level: the fingerprint takes BYTES, so
        // a tracked file in Windows-1252 (0xE4 is `ä` there and is not valid UTF-8 on its own)
        // produces a value like any other — and two different such diffs produce two different
        // values. A text-only input type could not express this case at all, which is precisely how
        // it went unnoticed: the failure was upstream, in a reader that turned it into "".
        let one = fingerprint_of("abc", b" M latin.txt\n", b"+Gr\xe4\xdfe\n");
        let two = fingerprint_of("abc", b" M latin.txt\n", b"+Gr\xe4\xdfer\n");
        assert_ne!(one, two);
        assert_eq!(one.len(), 64);
    }

    #[test]
    fn trimming_is_ascii_only_and_survives_bytes_that_are_not_text() {
        // The DIRTY question is asked of the trimmed porcelain read, and that read may carry raw
        // path bytes (`core.quotePath=false`). A trim that assumed UTF-8 would panic or refuse here.
        assert_eq!(trim_ascii(b"  \n"), b"");
        assert_eq!(trim_ascii(b""), b"");
        assert_eq!(trim_ascii(b"\n M a\n"), b"M a");
        assert_eq!(trim_ascii(b"\xff\xfe"), b"\xff\xfe");
    }

    /// The anchor of a clean tree on `main`, as a fixture for the drift tests below.
    fn at(commit: &str, branch: &str, dirty: bool, print: &str) -> Anchor {
        Anchor {
            commit: commit.to_string(),
            branch: branch.to_string(),
            dirty,
            fingerprint: print.to_string(),
        }
    }

    #[test]
    fn an_unmoved_tree_has_no_drift() {
        let then = at("aaa", "main", false, "f1");
        assert!(!then.drift_from(&then).any());
    }

    #[test]
    fn a_moved_commit_is_reported_and_the_uncommitted_question_is_not_asked() {
        // The commit moved, so the fingerprints are incomparable by construction — the diff half is
        // measured against HEAD. Reporting `uncommitted_now` here would be an answer derived from
        // two values that do not describe the same baseline.
        let then = at("aaa", "main", false, "f1");
        let now = at("bbb", "main", false, "f2");
        let d = now.drift_from(&then);
        assert_eq!(d.commit_now.as_deref(), Some("bbb"));
        assert_eq!(d.branch_now, None);
        assert!(!d.uncommitted_now);
        assert!(d.any());
    }

    #[test]
    fn uncommitted_work_that_moved_under_a_standing_commit_is_the_whole_finding() {
        // The case a `git log` cannot show and the one an interrupted operation meets most often:
        // nobody committed anything, and the tree is not what it was.
        let then = at("aaa", "main", true, "f1");
        let now = at("aaa", "main", true, "f2");
        let d = now.drift_from(&then);
        assert_eq!(d.commit_now, None);
        assert!(d.uncommitted_now);
        assert!(d.any());
    }

    #[test]
    fn a_tree_that_switched_branch_at_the_same_commit_still_reports_a_drift() {
        // Two branches can point at one commit, so the fingerprint is identical and the equality
        // test alone says "calm". The tree is somewhere else, and a resume that wrote into it would
        // write onto the wrong branch — which is why `branch_now` is its own term of `any()`.
        let then = at("aaa", "main", false, "f1");
        let now = at("aaa", "release/1.2", false, "f1");
        let d = now.drift_from(&then);
        assert_eq!(d.branch_now.as_deref(), Some("release/1.2"));
        assert!(!d.uncommitted_now);
        assert!(d.any());
    }

    #[test]
    fn a_tree_that_went_detached_says_so_with_an_empty_branch() {
        // An empty `branch_now` is an ANSWER — the tree is on no branch now — and not a missing
        // one. `Some("")` is what carries that, which is why the field is not `Option<NonEmpty>`.
        let then = at("aaa", "main", false, "f1");
        let now = at("aaa", "", false, "f1");
        assert_eq!(now.drift_from(&then).branch_now.as_deref(), Some(""));
    }

    #[test]
    fn the_short_form_is_twelve_digits_and_never_panics_on_a_shorter_one() {
        assert_eq!(
            at("0123456789abcdef", "main", false, "f").short(),
            "0123456789ab"
        );
        // A sha this short cannot come from git, and a display helper that panicked on one would
        // take a whole `nxc status` down for a cosmetic.
        assert_eq!(at("abc", "main", false, "f").short(), "abc");
    }
}
