//! **Every `Worker` in this crate has decided about each DEFAULTED method** — a source gate,
//! because nothing else can see these (review of PR #361, Test Quality #5).
//!
//! The incident it generalises: [`Worker::session_is_running`] was added with a DEFAULT (`false`),
//! which is what makes it safe to put on a trait a host implements — and it is exactly what let
//! `cli.rs`'s `LazyWorker`, a pure delegator, answer for a worker it never asked. Nothing failed to
//! compile, the seam suite stayed green, and the shipped command line was inert until a live run
//! with two real Claude sessions caught it: the next step of an exclusive channel started eight
//! milliseconds after the previous member's reply, into the same checkout.
//!
//! That got a point fix and a live-process test (`the_flow_waits_for_a_live_session.rs`). This is
//! the STRUCTURAL half: the next `Worker` impl — or the next defaulted method — cannot slip through
//! by being invisible, because adding one means adding a row here and saying which it is.
//!
//! It reads the SOURCE rather than the types, through the same reasoning `read_surface.rs` states
//! for its own gate: a defaulted trait method that an implementor declines is not observable at
//! runtime, in the type system, or by any test that only drives one worker.
//!
//! **It covers every defaulted method, not the first one** (nxf 6j6v.n92p). `run_precondition`
//! arrived with a default of its own — a refusal, the safe direction for ITS question — and a
//! delegator that declined it would make every declared hurdle on that path report "could not be
//! checked" while the seam suite, which drives its own workers, stayed green. That is the same
//! invisibility, one method over, so [`DEFAULTED`] names them all and every implementor answers
//! for each.
//!
//! **The third is the one that says whether the first can be trusted** (nxf 6j6v.t41e).
//! `answers_liveness` defaults to `false` — *this worker does not answer* — and a delegator that
//! declined it would report the worker it delegates to as unable to answer a question that worker
//! answers perfectly well. That is the LazyWorker incident again with the sign reversed: not a
//! silent `false` acted on as a fact, but a silent `false` about a fact that was available.
//!
//! Its two `OnTheDefault` rows carry an argument the other two methods do not need, so read them
//! rather than skimming: a worker that starts no process is NOT thereby a worker that can answer
//! whether a session is running. The store's sessions belong to the WORKSPACE, and one of them may
//! have been started by a real sidecar in another invocation and still be alive.
//!
//! **The fourth arrived with nxf 6j6v.de9s and was not added here — which is how the class shipped a
//! THIRD time** (found by the independent review of PR #474, Code Quality #2). `working_copy`
//! defaults to `None`, *this worker runs its sessions nowhere you can see*, and `LazyWorker` — the
//! wrapper every single command-line call passes through — rested on it. Everything downstream
//! believed the shipped `nxc` had no working copy at all: `park_the_stranded_holder` refused with
//! `NoWorkingCopy` on every tick, so the whole of 6j6v.de9s was inert on the shipped path while its
//! own tests, which drive their own workers at the engine seam, stayed green (nxf 6j6v.x5sr). This
//! table's doc already claimed to cover "every defaulted method, not the first one"; between de9s
//! and x5sr that claim was false in the source, and the gate that exists for exactly this could not
//! have caught it. It is true again now.
//!
//! **The sixth and seventh arrived together and are the first WRITE on the trait** (nxf 6j6v.b9nf).
//! `stops_sessions` defaults to `false` — *this worker cannot stop a session* — and `stop_session`
//! to a named refusal; a delegator that declined either would report a host that owns its processes
//! as unable to end one, and `withdraw` on a running round would then refuse by name for a runtime
//! that can. Same class, same wrapper — and unlike when this paragraph was written, this pair no
//! longer needs this table to be the only thing that can see a missing forward: `nxc withdraw` now
//! reaches both through the binary (`withdraw_a_running_round.rs`'s `nxc_withdraw_*` tests, against
//! a real process). This table stays anyway, for the reason its own doc gives above: a unit-level
//! pin is cheaper and fails closer to the cause than a withdrawal that silently refused a runtime
//! that can stop its own sessions.

use std::path::PathBuf;

/// What an implementor has decided about a defaulted method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answers {
    /// It implements the method itself — required of anything that DELEGATES to another worker, and
    /// of anything that genuinely knows.
    ForItself,
    /// It rests on the trait default, deliberately. Only legitimate when the default is the TRUE
    /// answer for that worker, not merely a convenient one.
    OnTheDefault,
}

/// Every DEFAULTED method on `Worker` — the ones an implementor can decline without anything
/// noticing. `trigger` is not among them: it has no default, so declining it does not compile.
const DEFAULTED: &[&str] = &[
    "session_is_running",
    "run_precondition",
    "answers_liveness",
    "working_copy",
    // **The fifth, and the one that proved this list is the gate's weak point** (nxf 6j6v.npy3).
    // `LazyWorker` shipped without forwarding it, which made `nxc resume` inert on the real binary
    // — the FOURTH occurrence of the class this file was written to stop after the third. It was
    // invisible here for one reason only: adding a defaulted method to the trait does not add a row
    // to this constant, so the gate stayed silent about a method it had never been told about.
    "resumes_sessions",
    // **The sixth and seventh, added to this list in the same change that added them to the trait**
    // (nxf 6j6v.b9nf) — which is the discipline the fifth's comment above asks for. The question and
    // the act travel together: a delegator that forwards one and not the other would say `true` to
    // "can you stop it" and then refuse to.
    "stops_sessions",
    "stop_session",
];

struct Implementor {
    name: &'static str,
    /// One decision per entry of [`DEFAULTED`], in that order.
    answers: &'static [Answers],
    /// Why — an entry without one is a finding, not a free pass.
    why: &'static str,
}

use Answers::{ForItself, OnTheDefault};

const IMPLEMENTORS: &[Implementor] = &[
    Implementor {
        name: "BoundedWorker",
        answers: &[ForItself, ForItself, ForItself, ForItself, ForItself, ForItself, ForItself],
        why: "A DELEGATOR: it wraps a host-supplied worker to bound `trigger`. Anything it does not \
              forward is answered for a third party that may well know better, so it forwards. \
              Neither READ is bounded on its own thread the way `trigger` is — a conforming \
              implementation answers from local state, and a host that blocks in one breaks the \
              same rule it would break by blocking in `trigger`. The third is the sharpest case of \
              the same rule: answering it for a host worker would report a worker that DOES read \
              its processes as unable to say so. The fourth is the same rule about a PLACE: a host \
              that runs its sessions in a directory would be reported as running them nowhere, \
              which silently disables both the park and the handover anchor. The fifth is the fourth's argument about a \
              CAPABILITY rather than a place: a host whose runtime continues its own conversations \
              would be reported as unable to, and the resume would then refuse by name for a \
              runtime that can. The sixth and seventh are the fifth's argument about the one WRITE: \
              a host that can stop its own sessions would be reported as unable to, and a \
              withdrawal of a running round would refuse by name for a runtime that can. Neither \
              is bounded on its own thread — a conforming `stop_session` DELIVERS a signal and \
              returns, which is the same local-state promise the reads make.",
    },
    Implementor {
        name: "DryWorker",
        answers: &[
            OnTheDefault,
            OnTheDefault,
            OnTheDefault,
            OnTheDefault,
            OnTheDefault,
            OnTheDefault,
            OnTheDefault,
        ],
        why: "It spawns NOTHING, so no session it ever triggered is running and `false` is the \
              truthful answer rather than a convenient one. Making it consult pid files would be a \
              lie told for the sake of a test. The same word decides the hurdle: a worker whose \
              whole contract is that it starts no process must not be the one that runs a \
              project's shell command, and `Unavailable` says exactly that. And it must not claim \
              to ANSWER the liveness question on the strength of that: `false` is true of the \
              sessions IT triggered, and the sessions in the store belong to the workspace — one \
              of them may have been started by a real sidecar in another invocation and still be \
              alive. It looks at no process, so it cannot tell. And it names no working copy for \
              the same reason it runs no hurdle: it starts nothing anywhere, so there is no \
              directory its sessions run in — `None` is the truth rather than a decline. And it continues nothing: it never began a conversation, so \
              there is none to carry on and `false` is the truth rather than a decline. And it \
              can stop nothing, for the reason it starts nothing: there is no process of its own to \
              signal, and a `true` here would have a withdrawal believe a stop was delivered to a \
              session that goes on running.",
    },
    Implementor {
        name: "SidecarWorker",
        answers: &[ForItself, ForItself, ForItself, ForItself, ForItself, ForItself, ForItself],
        why: "The only worker in this crate that owns real processes. Liveness comes off the pid \
              file its own `SessionLock` already writes at every trigger — same file, same parse, \
              same identity check, and the residual that type documents: content that does not \
              name one process reads as gone, and a claim whose recorded start instant cannot be \
              compared at all is believed by the READS and refused by the SIGNAL (nxf 6j6v.b9nf). \
              A pid the operating system has handed on is no longer one of them: the claim records \
              when its process started, so a stranger holding the number reads as a session that \
              ENDED. It is also the only one that HAS the \
              working directory a declared hurdle must run in, which is why the hurdle seam is on \
              this trait at all. It is therefore also the only one entitled to say `true` to the \
              third: it LOOKS, so its `false` is a fact about the session rather than about who was \
              asked. The fourth is the one method it is the sole SOURCE of: `cwd`, the workspace \
              root it chdirs every session into, which is what a park commits in and what a \
              handover anchor describes. The fifth is the one it answers `true` to, alone in this \
              list: a `resume` reaches the SDK\'s `options.resume`, and Claude Code keeps its \
              transcript at the project path, so a conversation it began is still there days \
              later — which is the case the whole availability-boundary feature is about. The \
              sixth and seventh are the pid file once more, this time WRITTEN TO rather than read: \
              `stop_session` sends SIGTERM to the pid its own lock recorded — and only once that \
              pid has PROVED it is this session's process, because a claim is never removed and \
              the number in a stale one may since belong to a stranger — and `stops_sessions` says \
              so exactly where the liveness read can be asked, on the one platform condition both \
              share.",
    },
    Implementor {
        name: "LazyWorker",
        answers: &[ForItself, ForItself, ForItself, ForItself, ForItself, ForItself, ForItself],
        why: "A DELEGATOR, and the one this gate exists for: `cli.rs` resolves the worker per call \
              from the environment, so EVERY command-line call passes through it. It forwarded only \
              `trigger`, the default answered for the selected worker, and the gate that keeps two \
              sessions out of one checkout was inert on the shipped path until a live run found it. \
              The third would fail the same way and be harder to see, since its default is also \
              what a wrong implementation says: `nxc session state` would report the shipped \
              sidecar as unable to answer. And the fourth is the one that actually happened: it \
              rested on `None`, so every `nxc tick` reported a host with no working copy and the \
              park never ran on the shipped path (nxf 6j6v.x5sr, the third instance of this exact \
              class at this exact wrapper). The fifth happened next, and identically (nxf 6j6v.npy3): it \
              rested on `false`, so every `nxc resume` on the shipped binary answered \
              'this runtime cannot continue a conversation it began' about a sidecar that can — \
              the FOURTH instance of this class at this wrapper, caught by review rather than by \
              this gate, because a new defaulted method does not add its own row above. The sixth \
              and seventh were added here in the same change that added them to the trait, so \
              this row is the gate for them rather than the apology.",
    },
    Implementor {
        name: "DisabledWorker",
        answers: &[
            OnTheDefault,
            OnTheDefault,
            OnTheDefault,
            OnTheDefault,
            OnTheDefault,
            OnTheDefault,
            OnTheDefault,
        ],
        why: "`WorkerConfig::Disabled` means the caller never opted into orchestration at all — it \
              refuses every `trigger` with a named error. There are no sessions of its own to be \
              running, so `false` is what is true here, not a fallback — and nothing it could ever \
              start has a hurdle to clear. It answers no liveness question for the same reason \
              `DryWorker` does not: it looks at no process, and the sessions it would be asked \
              about are the workspace's, not its own. It names no working copy on the same ground: \
              a handle that refuses to start anything has no directory it starts things in. And it continues nothing for the reason it starts nothing. \
              And it stops nothing on the same ground: a session it never started is not its to \
              signal, and the named refusal is the truth rather than a decline.",
    },
];

/// Every crate source file that could hold an `impl Worker for …`.
fn sources() -> Vec<PathBuf> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&src)
        .unwrap_or_else(|e| panic!("reading {}: {e}", src.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    out.sort();
    out
}

/// Each `impl Worker for <Name> { … }` in `text`, as (name, body). The block ends at the first
/// line that is exactly `}` — the crate's own formatting, enforced by `cargo fmt`, so this needs no
/// brace counting and cannot be fooled by a `}` inside a string in the body.
fn worker_impls(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for marker in ["impl Worker for ", "impl crate::worker::Worker for "] {
        let mut from = 0;
        while let Some(at) = text[from..].find(marker) {
            let start = from + at;
            let rest = &text[start + marker.len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let body_start = start + marker.len() + name.len();
            let end = text[body_start..]
                .find("\n}\n")
                .map(|e| body_start + e)
                .unwrap_or(text.len());
            out.push((name, text[body_start..end].to_string()));
            from = end;
        }
    }
    out
}

#[test]
fn every_worker_implementor_has_decided_about_every_defaulted_method() {
    let mut found: Vec<(String, String)> = Vec::new();
    for path in sources() {
        let text = std::fs::read_to_string(&path).expect("readable source");
        found.extend(worker_impls(&text));
    }
    assert!(
        found.len() >= IMPLEMENTORS.len(),
        "the gate found fewer `impl Worker for` blocks ({}) than the table names ({}) — the parser \
         is broken, or an implementor was removed without its row: {:?}",
        found.len(),
        IMPLEMENTORS.len(),
        found.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );

    for (name, body) in &found {
        let declared = IMPLEMENTORS
            .iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "\n\n`impl Worker for {name}` is not named in this file's table.\n\n  \
                 Every method in {DEFAULTED:?} has a DEFAULT, so declining one compiles, runs, and \
                 breaks nothing a test can see — that is how `LazyWorker` came to answer `false` \
                 for every command-line call while the seam suite stayed green. Add a row with one \
                 decision per defaulted method: `ForItself` (it implements the method — required \
                 of anything that DELEGATES to another worker) or `OnTheDefault` (the default is \
                 the TRUE answer here, with the reason why).\n"
                )
            });
        assert_eq!(
            declared.answers.len(),
            DEFAULTED.len(),
            "\n\n`{name}`'s row carries {} decisions and `Worker` has {} defaulted methods \
             ({DEFAULTED:?}) — a row that is short by one is a method nobody decided about.\n",
            declared.answers.len(),
            DEFAULTED.len()
        );
        for (method, answer) in DEFAULTED.iter().zip(declared.answers) {
            let implements = body.contains(&format!("fn {method}"));
            match answer {
                ForItself => assert!(
                    implements,
                    "\n\n`{name}` is declared as answering for itself and does not implement \
                     `{method}`.\n\n  Its reason on record: {}\n",
                    declared.why
                ),
                OnTheDefault => assert!(
                    !implements,
                    "\n\n`{name}` is declared as resting on the trait default and implements \
                     `{method}` after all — update its row, so the table keeps saying what the \
                     code does.\n\n  Its reason on record: {}\n",
                    declared.why
                ),
            }
        }
    }

    for declared in IMPLEMENTORS {
        assert!(
            found.iter().any(|(n, _)| n == declared.name),
            "\n\n`{}` has a row here and no `impl Worker for` in the crate — drop the row rather \
             than leaving an excuse for something that no longer exists.\n",
            declared.name
        );
    }
}

#[test]
fn the_gate_reads_the_delegators_and_would_notice_one_that_stopped_forwarding() {
    // The gate's own premise, asserted rather than assumed: it really does see the two delegators
    // and really does read their bodies. Without this the test above passes just as well against a
    // parser that finds nothing — which is the failure mode a source gate has and a type-level one
    // does not.
    let cli = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/cli.rs"))
        .unwrap();
    let impls = worker_impls(&cli);
    let lazy = impls
        .iter()
        .find(|(n, _)| n == "LazyWorker")
        .expect("the gate finds the delegator every CLI call goes through");
    assert!(
        lazy.1.contains("fn trigger") && DEFAULTED.iter().all(|m| lazy.1.contains(m)),
        "and reads its whole body, not just its header: {:?}",
        lazy.1
    );
}
