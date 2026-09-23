//! **The deterministic hurdles a step must clear before it is opened** (nxf 6j6v.n92p) — the two
//! classes, the verdict rule between them, and the refusal a requester reads.
//!
//! The owner's words at the source (2026-08-24), after the five incidents recorded on `4jgn.ajea`:
//!
//! > "Ich denke, wir muessen bei den Kanaelen klarere deterministische Huerden bauen, die solche
//! > Situationen abfangen. Einen Teil davon muessen wir mitbringen, aber ich denke, wir muessen
//! > Nutzern auch ermoeglichen, eigene deterministische Regeln zu bauen. Der Koordinator eines
//! > Kanals ist dafuer die richtige Stelle."
//!
//! # The two classes, and the split IS the design
//!
//! **BROUGHT ALONG** — what this building block can know without knowing the project. A previous
//! step's process still writing into the checkout is a fact about THIS machine and about nothing
//! else, so it holds whether or not anybody declared anything, and it is deliberately not
//! expressible in a declaration: a property you have to remember to ask for is a property an
//! embedding host does not get. [`BUILT_IN_HURDLES`] is the inventory, written down rather than
//! scattered, and [`crate::orchestration::preconditions_hold`] is where it runs.
//!
//! **DECLARED** — what only the project knows. Both of the owner's own examples are of this kind
//! and have no business inside a general chat building block:
//!
//! * "the remote is not ahead of the local working copy" — `git rev-list --count HEAD..@{u}` is
//!   `0`. That is a git fact, and `nxc` knows nothing about git.
//! * "the working area is not blocked" — what *blocked* means is the project's own business: a
//!   clean tree? no running build? no foreign hunks?
//!
//! So the user declares them and the supervisor runs them:
//!
//! ```yaml
//! - name: coding
//!   members: [coder, finisher]
//!   flow: sequential
//!   working_tree: exclusive
//!   preconditions:
//!     - name: remote-not-ahead
//!       run: git rev-list --count HEAD..@{u}
//!       expect: "0"
//!     - name: tree-clean
//!       run: git status --porcelain
//!       expect: ""
//! ```
//!
//! # The four decisions this module implements
//!
//! 1. **FAIL-CLOSED, never fail-open.** A hurdle whose command crashes, does not exist, or runs
//!    into its bound STOPS the step. A hurdle that lets a doubt through is not a hurdle. That is
//!    what makes [`Worker::run_precondition`](crate::worker::Worker::run_precondition)'s default
//!    a refusal rather than a pass, which is the opposite direction from
//!    [`Worker::session_is_running`](crate::worker::Worker::session_is_running)'s default — see
//!    that method for why the two safe directions point opposite ways.
//! 2. **BEFORE the trigger, not after it.** Once the session is started the damage is started with
//!    it, so the check runs in `open_flow_step` ahead of `coordinator_commission_in`, in the
//!    channel's working directory.
//! 3. **A refusal comes back as a DECLARED CLASS, not as prose** — [`Refusal`], carried on
//!    [`FailedConsequence`](crate::orchestration::FailedConsequence) under
//!    [`ConsequenceClass::PreconditionRefused`](crate::orchestration::ConsequenceClass::PreconditionRefused).
//!    Otherwise nxf 6j6v.gk9j repeats itself: an escalation that arrives as ordinary text loses its
//!    force on the way. A requester must be able to tell "hurdle X refused, here is its output"
//!    from "the step failed" by branching, not by reading.
//! 4. **THE COST IS NAMED, not hidden.** One process start per hurdle per step. A quorum of four
//!    checkers with two declared hurdles is eight extra processes per round — acceptable, and
//!    written down here so nobody has to measure it twice.
//!
//! # The fifth decision: a hurdle is executable code in a file the hurdled may edit
//!
//! Declarations live in `.nxs-personas/`, in the same working copy the declared agents are told to
//! work in. A hurdle its subject can switch off is theatre. The owner's answer (2026-08-29, on the
//! ticket): **the declaration is FROZEN when an operation is opened** — see
//! [`crate::declaration_freeze`], which does it for the catalogue as a WHOLE rather than for
//! `preconditions:` alone, because two notions of "the version in force" beside each other would be
//! worse than none.

use serde::{Deserialize, Serialize};

/// One hurdle a channel declares — a command, and what its passing looks like.
///
/// Stored verbatim by the loader and interpreted at the point of USE, exactly as
/// [`ChannelDecl::timeout`](crate::channel::ChannelDecl::timeout) is: this type parses the YAML and
/// says nothing about what a command means.
///
/// **Not `#[non_exhaustive]`**, unlike the enums on this surface: it is a serde DECLARATION type, so
/// a caller builds one from YAML rather than by literal, and a later field arrives with
/// `#[serde(default)]` the way every field but `name`/`run` already would.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Precondition {
    /// What this hurdle is called. It is the handle a refusal is reported under, so it is the one
    /// string a requester matches on — and [`crate::channel::validate_channels`] refuses an empty
    /// one and a duplicate within a channel for exactly that reason.
    pub name: String,
    /// The command line, run by the platform's shell in the channel's working directory. A shell
    /// line rather than an argv vector on purpose: every hurdle anybody has asked for so far is a
    /// pipeline or a git plumbing call, and an author writing one in a YAML file writes the line
    /// they would type.
    pub run: String,
    /// What the command must print for the hurdle to pass — compared against its TRIMMED standard
    /// output, so `expect: ""` means "printed nothing" and a trailing newline never decides a step.
    ///
    /// `None` — omitted — means the EXIT STATUS alone decides. That is the shorter form for a
    /// command that is already a predicate (`test -z "$(git status --porcelain)"`), and it is not a
    /// weaker one: a zero exit is as declared an answer as a matching line.
    ///
    /// A declared `expect:` never REPLACES the status check, it adds to it: a command that fails
    /// and happens to print the expected string is refused. See [`verdict`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<String>,
}

/// **The inventory of BROUGHT-ALONG hurdles** — the ones that hold with nothing declared.
///
/// A list rather than a scattering, because the class is defined by "the building block knows this
/// by itself" and a reader has to be able to see all of it at once. Each entry is the name a
/// [`Refusal`] is reported under.
///
/// The ticket names three. Two of them already had a home when this frame was built, and moving
/// them here would have made them WORSE rather than merely tidier — so they are named with their
/// site instead of duplicated, which is the honest inventory:
///
/// * **The previous step's process is no longer alive** — [`PREVIOUS_STEP_STILL_WRITING`], the one
///   that runs in this frame. It was nxf 6j6v.10yb's subject and was enforced at
///   `supervisor_consider_set` alone; the frame asks it at the step-opening seam, which is where
///   ALL four entrances meet, and that closes the one entrance 10yb could not see (a requester's
///   follow-up starting a fresh pass while the previous pass is still writing).
/// * **No other claim area holds the working copy** — the working-tree lease (nxf 6j6v.bqe0) knows
///   it, and what it does about it is better than a refusal: the trigger is ENQUEUED and runs when
///   the holder is done. Refusing here would turn a wait into a loss.
/// * **The thread still owes what it claims to owe** — `supervisor_consider_set`'s own
///   `channel_q.expects != [supervisor]` gate, which returns before any step is considered. Asking
///   it a second time at the seam would answer for a different moment and could only disagree.
pub const BUILT_IN_HURDLES: &[&str] = &[PREVIOUS_STEP_STILL_WRITING];

/// The brought-along hurdle this frame runs: no session of an earlier step of this very channel
/// thread is still writing (nxf 6j6v.10yb, at the seam nxf 6j6v.n92p put the frame on).
pub const PREVIOUS_STEP_STILL_WRITING: &str = "previous-step-still-writing";

/// **What a hurdle's command actually did** — the answer the executing seam gives back
/// ([`Worker::run_precondition`](crate::worker::Worker::run_precondition)).
///
/// Two variants and not a `Result`, because the distinction is not "worked / errored" but "the
/// hurdle produced a verdict / the hurdle could not be asked". Both stop the step — that is
/// fail-closed — and they are still different things to a reader, which is decision 3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreconditionOutcome {
    /// The command ran to completion. `status` is its exit code (`None` for a process killed by a
    /// signal, which is not a zero exit and therefore never a pass), `stdout` its standard output
    /// verbatim, `stderr` its standard error verbatim — carried so a refusal can SHOW why rather
    /// than assert it.
    Ran {
        status: Option<i32>,
        stdout: String,
        stderr: String,
    },
    /// The command could not be asked at all: no shell, a spawn that failed, a bound that ran out,
    /// or a worker that does not execute commands. The string says which.
    Unavailable(String),
}

/// **Which kind of refusal this is** — the declared class decision 3 asks for, one axis finer than
/// [`ConsequenceClass`](crate::orchestration::ConsequenceClass).
///
/// `#[non_exhaustive]` for the reason every enum on this surface is: it goes irreversibly public
/// with the open-source launch, and a further kind must be an additive minor rather than a break
/// for every downstream `match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RefusalKind {
    /// A BROUGHT-ALONG hurdle said no — see [`BUILT_IN_HURDLES`]. Nothing was declared and it still
    /// applied, which is the whole point of the class.
    BuiltIn,
    /// A declared hurdle ran and its verdict was NO: a non-zero exit, or output that is not what the
    /// declaration expects. The project's own rule, answered by the project's own command.
    Verdict,
    /// A declared hurdle could not be RUN — fail-closed (decision 1). A reader branches on this to
    /// tell "your rule says no" from "your rule could not be asked", which call for opposite
    /// responses: the first is work to do, the second is plumbing to fix.
    Unrunnable,
}

/// **One hurdle's refusal, as data** — what stopped a step, reported so a requester branches
/// instead of parsing prose (decision 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    /// The hurdle's declared `name:`, or one of [`BUILT_IN_HURDLES`].
    pub hurdle: String,
    /// Which kind of refusal — see [`RefusalKind`].
    pub kind: RefusalKind,
    /// What the hurdle produced, trimmed and capped at [`OUTPUT_CAP`] bytes: the command's own
    /// output for a [`RefusalKind::Verdict`], the reason it could not run for
    /// [`RefusalKind::Unrunnable`], and the engine's own account for a
    /// [`RefusalKind::BuiltIn`].
    ///
    /// Present so the refusal SHOWS its ground. A refusal that only asserts is the free text this
    /// type exists to replace.
    pub output: String,
}

/// How much of a hurdle's output travels with its refusal.
///
/// A refusal is carried on a receipt an agent reads, and `git status --porcelain` in a repository
/// mid-rebase can print thousands of lines — enough to bury the receipt it rides on and everything
/// else in it. Four kilobytes is several screens of the thing that matters (the first offending
/// lines) and far short of "the whole tree". Truncation is SAID, never silent: [`cap`] appends the
/// marker.
pub const OUTPUT_CAP: usize = 4096;

/// The marker [`cap`] appends when it cut. Named once so a reader — and a test — has one spelling.
pub const TRUNCATION_MARKER: &str = "… [truncated]";

/// Trim and cap one hurdle's output, saying so when it cut.
///
/// Cuts on a CHARACTER boundary, not a byte one: the output is a `String` that came off a command's
/// stdout, and slicing a multi-byte character in half would panic on a hurdle whose refusal reason
/// happens to contain a non-ASCII path.
pub fn cap(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.len() <= OUTPUT_CAP {
        return trimmed.to_string();
    }
    let mut end = OUTPUT_CAP;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{TRUNCATION_MARKER}", &trimmed[..end])
}

/// **What one command's outcome MEANS against one declaration** — the whole rule, as a pure
/// function, so the decision is testable without a process and cannot be spelled twice.
///
/// `None` is a pass. `Some(refusal)` stops the step.
///
/// The rule, in the order it is applied:
///
/// 1. The command must have RUN. Anything else is [`RefusalKind::Unrunnable`] — decision 1, and the
///    reason this arm exists at all.
/// 2. It must have exited ZERO. A non-zero exit is the shell's own way of saying no, and every
///    hurdle anybody writes is a command that already knows how to say it. A process killed by a
///    signal has no exit code and is not a pass.
/// 3. If — and only if — the declaration names an `expect:`, the command's TRIMMED stdout must
///    equal it (trimmed too, so an author who indents a YAML scalar is not punished for it).
pub fn verdict(decl: &Precondition, outcome: &PreconditionOutcome) -> Option<Refusal> {
    let (status, stdout, stderr) = match outcome {
        PreconditionOutcome::Unavailable(why) => {
            return Some(Refusal {
                hurdle: decl.name.clone(),
                kind: RefusalKind::Unrunnable,
                output: cap(why),
            })
        }
        PreconditionOutcome::Ran {
            status,
            stdout,
            stderr,
        } => (status, stdout, stderr),
    };
    // The output a refusal shows: what the command printed, and — when it printed nothing on
    // stdout — what it complained about. A command that fails with a message on stderr alone is the
    // ordinary shape of a broken hurdle (`fatal: no upstream configured`), and a refusal that
    // showed an empty string for it would hide the only sentence that explains it.
    let shown = |extra: &str| -> String {
        let joined = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
            (true, true) => extra.to_string(),
            (true, false) => format!("{extra}{}", stderr.trim()),
            (false, true) => format!("{extra}{}", stdout.trim()),
            (false, false) => format!("{extra}{}\n{}", stdout.trim(), stderr.trim()),
        };
        cap(&joined)
    };
    if *status != Some(0) {
        let how = match status {
            Some(code) => format!("exited {code}"),
            None => "was killed by a signal".to_string(),
        };
        return Some(Refusal {
            hurdle: decl.name.clone(),
            kind: RefusalKind::Verdict,
            output: shown(&format!("`{}` {how}\n", decl.run)),
        });
    }
    match &decl.expect {
        Some(want) if stdout.trim() != want.trim() => Some(Refusal {
            hurdle: decl.name.clone(),
            kind: RefusalKind::Verdict,
            output: shown(&format!(
                "`{}` printed something other than the declared `expect: {:?}`\n",
                decl.run, want
            )),
        }),
        _ => None,
    }
}

impl Refusal {
    /// The brought-along refusal of [`PREVIOUS_STEP_STILL_WRITING`].
    pub(crate) fn previous_step_still_writing(channel: &str) -> Refusal {
        Refusal {
            hurdle: PREVIOUS_STEP_STILL_WRITING.to_string(),
            kind: RefusalKind::BuiltIn,
            output: format!(
                "a session of an earlier step of channel {channel:?} is still running, and this \
                 channel needs the working copy to itself — starting the next step now would put \
                 two sessions in one checkout"
            ),
        }
    }

    /// The one sentence a human reads, and what an app surfaces when it renders the finding rather
    /// than branching on it — so the stderr breadcrumb, the `detail` on the record and this say the
    /// same thing by construction.
    pub(crate) fn detail(&self, step: &str, channel: &str) -> String {
        let what = match self.kind {
            RefusalKind::BuiltIn => "a brought-along precondition",
            RefusalKind::Verdict => "the declared precondition",
            RefusalKind::Unrunnable => "the declared precondition",
        };
        let verb = match self.kind {
            RefusalKind::Unrunnable => "could not be checked",
            _ => "refused",
        };
        format!(
            "step {step:?} of channel {channel:?} was not started: {what} {:?} {verb} — {}",
            self.hurdle, self.output
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(expect: Option<&str>) -> Precondition {
        Precondition {
            name: "tree-clean".to_string(),
            run: "git status --porcelain".to_string(),
            expect: expect.map(str::to_string),
        }
    }

    fn ran(status: Option<i32>, stdout: &str, stderr: &str) -> PreconditionOutcome {
        PreconditionOutcome::Ran {
            status,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn a_zero_exit_with_the_declared_output_passes() {
        assert_eq!(verdict(&decl(Some("")), &ran(Some(0), "\n", "")), None);
        assert_eq!(verdict(&decl(None), &ran(Some(0), "anything", "")), None);
    }

    #[test]
    fn the_expected_output_is_compared_trimmed_on_both_sides() {
        // An author who writes `expect: "0\n"` or indents a block scalar means the same thing as
        // one who writes `expect: "0"`, and a trailing newline must never decide a step.
        assert_eq!(verdict(&decl(Some(" 0 ")), &ran(Some(0), "0\n", "")), None);
    }

    #[test]
    fn a_non_zero_exit_refuses_even_when_the_output_matches() {
        // The status check is not replaced by `expect:`, it is added to: `git rev-list HEAD..@{u}`
        // with no upstream fails AND prints nothing, and reading that as "zero commits ahead" is
        // exactly the fail-open this must not do.
        let r =
            verdict(&decl(Some("")), &ran(Some(128), "", "fatal: no upstream")).expect("refused");
        assert_eq!(r.kind, RefusalKind::Verdict);
        assert!(r.output.contains("exited 128"), "{}", r.output);
        assert!(
            r.output.contains("fatal: no upstream"),
            "the only sentence that explains it comes from stderr: {}",
            r.output
        );
    }

    #[test]
    fn a_signal_death_is_never_a_pass() {
        let r = verdict(&decl(None), &ran(None, "", "")).expect("refused");
        assert_eq!(r.kind, RefusalKind::Verdict);
        assert!(r.output.contains("killed by a signal"), "{}", r.output);
    }

    #[test]
    fn output_that_is_not_what_was_declared_refuses_and_shows_what_came_instead() {
        let r = verdict(&decl(Some("")), &ran(Some(0), " M src/lib.rs", "")).expect("refused");
        assert_eq!(r.kind, RefusalKind::Verdict);
        assert_eq!(r.hurdle, "tree-clean");
        assert!(r.output.contains("M src/lib.rs"), "{}", r.output);
    }

    #[test]
    fn a_hurdle_that_could_not_be_run_refuses_as_its_own_kind() {
        // Decision 1 in one assertion: doubt does not pass, and it is REPORTED as doubt rather than
        // as the project's rule having said no.
        let r = verdict(
            &decl(None),
            &PreconditionOutcome::Unavailable("no shell on this platform".to_string()),
        )
        .expect("refused");
        assert_eq!(r.kind, RefusalKind::Unrunnable);
        assert!(r.output.contains("no shell"), "{}", r.output);
    }

    #[test]
    fn long_output_is_capped_and_says_that_it_was() {
        let flood = "M src/lib.rs\n".repeat(2000);
        let r = verdict(&decl(Some("")), &ran(Some(0), &flood, "")).expect("refused");
        assert!(
            r.output.len() <= OUTPUT_CAP + TRUNCATION_MARKER.len() + 200,
            "a refusal must not bury the receipt it rides on: {} bytes",
            r.output.len()
        );
        assert!(r.output.ends_with(TRUNCATION_MARKER), "truncation is said");
    }

    #[test]
    fn capping_never_splits_a_character() {
        // The output is a command's stdout and a path can be non-ASCII; slicing on a byte index
        // would panic exactly where a refusal is being reported, which is the worst possible place.
        let flood = "ä".repeat(OUTPUT_CAP);
        let out = cap(&flood);
        assert!(out.ends_with(TRUNCATION_MARKER));
        assert!(out.len() <= OUTPUT_CAP + TRUNCATION_MARKER.len());
    }

    #[test]
    fn the_built_in_inventory_is_what_the_frame_runs() {
        // The list is the class's whole content, so it is asserted rather than assumed: a hurdle
        // added to the frame without a line here would leave the inventory lying.
        assert_eq!(BUILT_IN_HURDLES, &[PREVIOUS_STEP_STILL_WRITING]);
        let r = Refusal::previous_step_still_writing("coding");
        assert_eq!(r.kind, RefusalKind::BuiltIn);
        assert!(BUILT_IN_HURDLES.contains(&r.hurdle.as_str()));
    }

    #[test]
    fn the_detail_names_the_step_the_channel_and_the_hurdle() {
        let r = verdict(&decl(Some("")), &ran(Some(0), " M x", "")).expect("refused");
        let d = r.detail("coder", "coding");
        assert!(d.contains("\"coder\""), "{d}");
        assert!(d.contains("\"coding\""), "{d}");
        assert!(d.contains("tree-clean"), "{d}");
    }
}
