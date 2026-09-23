//! The timer seam (nxf ticket 6j6v.z06n): when a declared channel's `timeout` opens a thread, a
//! deadline is armed that later runs `nxc tick --thread <id>` — nothing ever waits synchronously
//! for it. Mirrors `worker.rs`'s exact `Worker`/`select_worker`/`DryWorker` testability split:
//! `DryTimer` records what would have been scheduled/canceled for deterministic tests; the real
//! ones reach outside the process, best-effort — see `worker.rs`'s `SidecarWorker` (6j6v.8e45) for
//! the same shape on the SDK-spawning side.
//!
//! **The default is now the SERVICE** ([`ServiceTimer`], nxf 6j6v.8see). The deadline is written
//! into the workspace's own book and the one nexus-flow background service — the same process that
//! syncs — honours it. That replaced two earlier answers within a week of each other, and the
//! reason each fell is worth keeping:
//!
//! - [`AtTimer`] was the only backend for a year and is still a defensible choice on a Linux
//!   server, where `atd` is an ordinary daemon. On macOS it was never a fallback but a trap:
//!   `atrun` ships disabled, so `at` accepts a job, exits 0, prints an id — and nothing runs it. A
//!   declared `timeout:` therefore never fired once on the platform this product is developed on.
//!   It stays reachable with `NXC_TIMER=at`.
//! - A one-shot `launchd` agent per deadline (6j6v.74c0) fired correctly and is GONE. The owner
//!   found what it produces in the macOS background items: "sh — an item from an unidentified
//!   developer", one entry per open board. The decision of 2026-08-25 was one service, so that
//!   backend was removed rather than kept beside this one — a second clock is exactly what the
//!   decision was against.
//!
//! **`AtTimer` is covered too** (nxf 6j6v.amy1), which it was not until then — `DryTimer` and the
//! pure helpers were the whole of it, so the two methods that actually ship went untested. The way
//! around "`at`/`atd` is not guaranteed on every runner" is not to leave them uncovered but to stop
//! depending on the runner: the tests put a STUB `at`/`atrm` first on `PATH` and drive the real
//! `AtTimer` against it (real spawn, real pipes, real bounded wait), plus one round-trip against the
//! genuine binary that skips only when a DIRECT probe shows this machine has no usable `at`.
//!
//! **No daemon** (a hard rule, verbatim from the epic's own design): the scheduled job is a genuine
//! ONE-SHOT — it fires once and is gone; this process never keeps it alive, polls it, or manages
//! its lifecycle in any way.
//!
//! **No cross-process handle persistence** (nxf 6j6v.z06n's own deliberate scope decision, no new
//! schema): [`cancel`] is a real, tested, standalone primitive — it genuinely cancels a given
//! [`TimerHandle`] when called in the SAME process that holds it — but nothing in this ticket's
//! LIVE wiring persists a `TimerHandle` durably so a LATER `reply` invocation (a different process
//! entirely) could look it up and call `cancel` on it. The epic's own design text explicitly offers
//! "cancels/**ignores**" as equally valid strategies; the actual "no double-fire" guarantee this
//! ticket relies on is `nxc tick`'s own IDEMPOTENCY (see `cli.rs`'s `tick`), not real
//! cross-process OS-job cancellation.
//!
//! **Keyed on the THREAD** (a deliberate deviation from the ticket's own literal sketch, which
//! wanted a run id — see `cli.rs`'s `Command::Tick` doc): a declared channel's `timeout` hangs on
//! the board it opened, so `schedule_tick` takes a `thread_id`.

// The service backend lives in its own file (`timer/service.rs`): the deadline book, the
// attendance check and the honest refusal when nothing is watching. Beside this module rather than
// inside it because it is a self-contained adapter onto `nxs-service`, the same split the removed
// launchd backend had.
mod service;

pub use service::ServiceTimer;

use crate::error::{NxfError, Result};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// An opaque timer-job identifier. For [`AtTimer`], wraps the real `at` job id (parsed from `at`'s
/// own stderr, e.g. `job 3 at ...`); for [`DryTimer`], a synthetic identifier derived from the
/// thread id. Callers never parse this — mirrors `worker.rs`'s own opaque-string philosophy
/// (`TriggerRequest`'s fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerHandle(pub String);

/// Schedule/cancel a one-shot OS-level job that later runs `command`. Mirrors
/// [`crate::worker::Worker`]'s exact shape/testability split.
///
/// `Send + Sync` for [`crate::worker::Worker`]'s reason too (PR #269 review, Code Quality #3): since
/// the timer became a value a caller SAYS rather than something resolved from process env,
/// [`crate::engine::Engine`] holds one for the app's lifetime and must itself stay `Send + Sync` to
/// live in a Tauri backend's managed state.
pub trait Timer: Send + Sync {
    /// Schedule `command` to run once, at `deadline` (an absolute RFC3339 instant). `thread_id` is
    /// carried for [`DryTimer`]'s log line (an `AtTimer` job identifies itself purely by `command`,
    /// which already embeds the thread id — see [`schedule_tick`]).
    fn schedule(&self, thread_id: &str, deadline: &str, command: &str) -> Result<TimerHandle>;

    /// **Arm a caller's held delivery** (nxf 6j6v.gn8b): at `deadline`, hand `session` the answers
    /// that arrived while it was working.
    ///
    /// A METHOD of its own rather than another `schedule` with a different command string, because
    /// [`ServiceTimer`] does not take a command string at all — it writes a JOB from a closed set
    /// and the service builds the argv itself, so a second kind of deadline is a second kind of
    /// job. See [`ServiceTimer::schedule`]'s own doc for why that closed set exists.
    ///
    /// **The default delegates to [`schedule`](Timer::schedule)**, which is the right answer for
    /// every backend that DOES run a command string: `at` and `dry` both take
    /// [`delivery_command`] and run it. Only the service backend overrides.
    ///
    /// **And it refuses a session id that is not a schedulable key** (review of this branch,
    /// Integrity & Robustness). The service backend's own path runs
    /// [`nxs_service::timers::check`], which rejects anything outside `[A-Za-z0-9_-]` precisely
    /// because the key becomes part of an argv; this default builds a SHELL COMMAND STRING and had
    /// no such gate. Nothing can reach it today — every internal session id is engine-minted — but
    /// "cannot happen" is the wrong reason for the one backend that hands its argument to `sh`.
    fn schedule_delivery(&self, session: &str, deadline: &str) -> Result<TimerHandle> {
        if !nxs_service::timers::valid_key(session) {
            return Err(NxfError::validation(format!(
                "`{session}` is not a schedulable id: a delivery's key becomes part of the command \
                 a timer backend runs, so an id outside [A-Za-z0-9_-] is refused rather than \
                 spliced into a shell"
            )));
        }
        self.schedule(session, deadline, &delivery_command(session))
    }

    /// **Arm the way back from an availability boundary** (nxf 6j6v.npy3): at `deadline`, take
    /// `session`'s interrupted operation up again.
    ///
    /// A method of its own for [`schedule_delivery`](Timer::schedule_delivery)'s reason — the
    /// service backend writes a JOB from a closed set, so a third kind of deadline is a third kind
    /// of job — and it carries the same key gate for the same reason: the default builds a SHELL
    /// COMMAND STRING, and "no engine-minted id can reach it" is the wrong argument for the one
    /// backend that hands its argument to `sh`.
    ///
    /// **`deadline` is an EARLIEST, never a promise.** The job fires twice over one hold — once at
    /// the process grace, to secure the working copy, and once when the window actually lifts — and
    /// the verb it runs re-checks the world either way. See [`nxs_service::timers::Job::ChatResume`].
    fn schedule_resume(&self, session: &str, deadline: &str) -> Result<TimerHandle> {
        if !nxs_service::timers::valid_key(session) {
            return Err(NxfError::validation(format!(
                "`{session}` is not a schedulable id: a resume's key becomes part of the \
                 command a timer backend runs, so an id outside [A-Za-z0-9_-] is refused rather \
                 than spliced into a shell"
            )));
        }
        self.schedule(session, deadline, &resume_command(session))
    }
    /// Cancel a previously scheduled job. Only meaningful/tested within the SAME process that
    /// received the `TimerHandle` — see this module's own doc on why the live wiring never
    /// attempts this across a process boundary.
    fn cancel(&self, handle: &TimerHandle) -> Result<()>;

    /// **Is anything actually going to run what this timer schedules?** (nxf 6j6v.0j12)
    ///
    /// `schedule` answers "was this deadline accepted", which since [`ServiceTimer`] is a different
    /// question: the book is durable, so a deadline is accepted whether or not a service exists to
    /// honour it. This is the other question, asked without scheduling anything — so a verb that
    /// arms no window at all can still report that nothing is attending this workspace.
    ///
    /// **Defaulted to `None`, and that is the correct answer for every other backend.** `at` and
    /// `dry` do not depend on the nexus-flow service, so "the service is down" is not a fact about
    /// them; only [`ServiceTimer`] can raise it, and only it does.
    fn service_fault(&self) -> Option<nxs_service::ServiceFault> {
        None
    }
}

/// `NXC_TIMER=dry`: writes one line per schedule/cancel to `NXC_TIMER_LOG` — append-only, silently
/// a no-op when the env var isn't set — and never touches the real OS scheduler.
///
/// This used to mirror `DryWorker`'s `NXC_DRY_LOG` convention exactly. It no longer does: nxf
/// 6j6v.570x moved the worker's log path onto [`crate::worker::WorkerConfig::Dry`] as a field,
/// because reading it from process env in a multi-threaded test binary let a test that wanted no
/// log inherit a neighbour's deleted `TempDir` path. THIS type still reads process env, so it still
/// carries that latent race — the tests that touch `NXC_TIMER_LOG` all hold a lock today, which is
/// what keeps it latent. Filed as its own round (6j6v.5s63) rather than folded in here, so the
/// worker fix stays reviewable.
pub struct DryTimer;

impl Timer for DryTimer {
    fn schedule(&self, thread_id: &str, deadline: &str, command: &str) -> Result<TimerHandle> {
        let handle = TimerHandle(format!("dry:{thread_id}"));
        if let Ok(path) = std::env::var("NXC_TIMER_LOG") {
            append_log(
                &path,
                &format!(
                    "schedule thread={thread_id} deadline={deadline} handle={} command={command}\n",
                    handle.0
                ),
            )?;
        }
        Ok(handle)
    }

    fn cancel(&self, handle: &TimerHandle) -> Result<()> {
        if let Ok(path) = std::env::var("NXC_TIMER_LOG") {
            append_log(&path, &format!("cancel handle={}\n", handle.0))?;
        }
        Ok(())
    }
}

fn append_log(path: &str, line: &str) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| NxfError::io(format!("timer dry log: {e}")))?;
    f.write_all(line.as_bytes())
        .map_err(|e| NxfError::io(format!("timer dry log: {e}")))
}

/// `NXC_TIMER=at` (or default/unset, mirroring `select_worker`'s own `Some("sidecar")|None`
/// idiom): shells out to the real `at` command. Best-effort at RUNTIME — a machine with no
/// `atd`/`atrun` simply gets an error back (bounded by [`AT_SUBPROCESS_BOUND`], never a hang) —
/// but no longer untested: both methods are exercised against a stub `at`/`atrm` on `PATH`, and
/// against the real binary where the runner has one (nxf 6j6v.amy1, see this module's own doc).
///
/// # It asks whether the platform will RUN a job before it submits one (nxf 6j6v.7qtf's sibling,
/// 6j6v.p3sm)
///
/// "`at` took the job" and "the job will fire" are two different facts, and on macOS they come
/// apart completely: `atrun` ships DISABLED, so `at` accepts submissions, exits 0, prints a job id
/// — and nothing ever runs them. Measured on a stock Mac while the proving ground ran: `atq` listed
/// jobs from July whose due date was a month gone, and `launchctl print system/com.apple.atrun` did
/// not find the service at all. A declared channel's `timeout:` therefore never fired once, and the
/// caller could not see it anywhere: a successful `schedule()` returned a `TimerHandle` like any
/// other.
///
/// So this type probes the DAEMON, once, lazily, on its first `schedule` — and a platform whose
/// daemon will not run jobs is a scheduling FAILURE stated up front rather than a handle that means
/// nothing. That failure is what the caller reports as
/// [`ConsequenceClass::TickUnscheduled`](crate::orchestration::ConsequenceClass::TickUnscheduled).
///
/// It also removes a cost that was pure waste: without the probe, every `send`/`reply` on a thread
/// with a declared `timeout:` paid up to [`AT_SUBPROCESS_BOUND`] waiting on an `at` that could not
/// help it. The probe is one bounded `launchctl` call per process that schedules at all, and only
/// on the platform where the question has an answer.
pub struct AtTimer {
    /// The probe's verdict: `None` — the daemon runs jobs, or this platform cannot be asked;
    /// `Some(why)` — it does not, and `why` is what the caller is told. Lazy, so a process that
    /// never schedules pays nothing.
    daemon_refusal: std::sync::OnceLock<Option<String>>,
}

impl Default for AtTimer {
    fn default() -> Self {
        AtTimer::new()
    }
}

impl AtTimer {
    /// The real one, which probes the platform's daemon before its first submission.
    pub fn new() -> AtTimer {
        AtTimer {
            daemon_refusal: std::sync::OnceLock::new(),
        }
    }

    /// An `AtTimer` that SKIPS the probe and submits to whatever `at` is on `PATH`.
    ///
    /// For the tests that drive the shipped `schedule`/`cancel` against a stub `at` (nxf 6j6v.amy1):
    /// what they assert is the submission — the argv, the stdin handover, the job-id parse — and on
    /// a stock Mac the probe would correctly refuse before any of it ran, turning real coverage of
    /// the shipped path into a skip. Not a way around the guard: the probing constructor is what
    /// [`TimerConfig::At`] builds, so nothing that ships takes this door.
    pub fn assuming_the_daemon_runs_jobs() -> AtTimer {
        let daemon_refusal = std::sync::OnceLock::new();
        let _ = daemon_refusal.set(None);
        AtTimer { daemon_refusal }
    }

    /// Why this platform will not run a scheduled job, or `None` if it will (or cannot be asked).
    fn daemon_refusal(&self) -> Option<&str> {
        self.daemon_refusal.get_or_init(probe_at_daemon).as_deref()
    }
}

/// What a caller is told when the platform's `at` daemon is not going to run anything. It names the
/// developer's own way out and says plainly that it is not the product's.
///
/// **Gated with its one producer.** The text is about macOS specifically, and on every other
/// platform nothing produces it — an ungated constant is dead code there, which `-D warnings` is
/// right to reject. What is NOT platform-specific is the refusal MECHANISM, and the tests below
/// split along the same line: one drives the mechanism with an injected reason on any unix, one
/// pins this wording where it can actually be produced.
#[cfg(target_os = "macos")]
const ATRUN_NOT_LOADED: &str = "macOS ships `atrun` disabled, so `at` accepts a job, exits 0 and \
     prints a job id while nothing ever runs it — a declared channel `timeout:` cannot fire on \
     this machine. Enable it for a developer with `sudo launchctl load -w \
     /System/Library/LaunchDaemons/com.apple.atrun.plist`; that is a workaround, not a fix for a \
     shipped product.";

/// Ask launchd whether `com.apple.atrun` is loaded — the one question that separates "the queue
/// accepted it" from "something will run it" on macOS.
///
/// **Only a NEGATIVE answer refuses.** A probe that cannot be spawned, or that does not come back
/// inside [`AT_SUBPROCESS_BOUND`], leaves the timer exactly as it was before this existed: it
/// submits, and `at`'s own error is the signal. Refusing on a probe that merely failed to answer
/// would turn an unknown into a certainty in the wrong direction.
#[cfg(target_os = "macos")]
fn probe_at_daemon() -> Option<String> {
    let child = Command::new("launchctl")
        .args(["print", "system/com.apple.atrun"])
        .stdin(Stdio::null())
        // Discarded: `launchctl print` is verbose on success and this asks only for its EXIT CODE.
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    match wait_bounded(child, AT_SUBPROCESS_BOUND) {
        Ok(out) if out.status.success() => None,
        Ok(_) => Some(ATRUN_NOT_LOADED.to_string()),
        Err(_) => None,
    }
}

/// No probe here. On Linux `atd` is a long-lived daemon and `at`'s own failure is the signal the
/// caller already gets; there is no platform-wide default that silently swallows every job the way
/// macOS's disabled `atrun` does. A machine whose `atd` is stopped keeps behaving exactly as it did
/// before nxf 6j6v.p3sm — which is a real gap, named here rather than papered over with a `pgrep`
/// that would answer differently on every distribution.
#[cfg(not(target_os = "macos"))]
fn probe_at_daemon() -> Option<String> {
    None
}

/// The bound [`AtTimer::schedule`] gives the spawned `at` process to accept the job and exit (nxf
/// ticket 6j6v.81cq). This is NOT waiting for the scheduled deadline to arrive — only for `at`
/// itself to acknowledge the submission, which (per the guidance this fix was filed against) is
/// near-instant if it is going to respond at all. 8 seconds is generous slack over that, not an
/// attempt to guess how long a real `atd` round-trip might take.
const AT_SUBPROCESS_BOUND: Duration = Duration::from_secs(8);

impl Timer for AtTimer {
    fn schedule(&self, thread_id: &str, deadline: &str, command: &str) -> Result<TimerHandle> {
        // BEFORE the submission, which is both halves of the win: a caller that would have got a
        // meaningless handle gets the reason instead, and a caller that would have paid
        // `AT_SUBPROCESS_BOUND` waiting on an `at` that cannot help it pays nothing.
        //
        // **The way on travels with the refusal, not with the platform's text.** It used to be a
        // sentence inside `ATRUN_NOT_LOADED`, which meant a refusal on any other platform said what
        // was wrong and not what to do about it — a gap the test below found the moment the constant
        // stopped being the only reason this arm can carry. This is also the one use `thread_id` has
        // in this method: an `at` job identifies itself by its command, but a REFUSAL is about a
        // board, and naming it is what makes the message actionable on its own.
        if let Some(why) = self.daemon_refusal() {
            return Err(NxfError::io(format!(
                "the timeout tick for thread {thread_id} was not scheduled: {why} — nothing will \
                 notice this board's window on its own; `nxc tick --thread {thread_id}` runs the \
                 same check by hand"
            )));
        }
        let at_time = to_at_time_spec(deadline)?;
        let mut child = Command::new("at")
            .args(at_command_args(&at_time))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| NxfError::io(format!("spawning `at`: {e}")))?;
        {
            // `.take()` (owned), not `.as_mut()` (borrowed): the ONLY way to actually close the
            // pipe (so `at` sees EOF and knows the job body is complete) without calling
            // `wait_with_output` — which this function no longer does, since it needs to bound
            // the wait itself (see `wait_bounded`'s own doc for why `wait_with_output` can't be
            // reused directly here). A bare borrow leaves the underlying stdin handle open even
            // after this block ends, since dropping a `&mut` reference drops nothing.
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| NxfError::io("`at`'s stdin was not piped".to_string()))?;
            stdin
                .write_all(command.as_bytes())
                .map_err(|e| NxfError::io(format!("writing `at`'s stdin: {e}")))?;
        }
        let output = wait_bounded(child, AT_SUBPROCESS_BOUND).map_err(|e| {
            NxfError::io(format!(
                "waiting on `at` (no response within {AT_SUBPROCESS_BOUND:?} — possibly no \
                 atd/atrun running): {e}"
            ))
        })?;
        if !output.status.success() {
            return Err(NxfError::io(format!(
                "`at` exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        // `at` reports the job id on stderr, conventionally "job <n> at <date>".
        let stderr = String::from_utf8_lossy(&output.stderr);
        let job_id = stderr
            .split_whitespace()
            .skip_while(|w| *w != "job")
            .nth(1)
            .unwrap_or("?");
        Ok(TimerHandle(format!("at:{job_id}")))
    }

    fn cancel(&self, handle: &TimerHandle) -> Result<()> {
        let job_id = handle.0.strip_prefix("at:").unwrap_or(&handle.0);
        let status = Command::new("atrm")
            .arg(job_id)
            .status()
            .map_err(|e| NxfError::io(format!("spawning `atrm`: {e}")))?;
        if !status.success() {
            return Err(NxfError::io(format!(
                "`atrm {job_id}` exited with {status}"
            )));
        }
        Ok(())
    }
}

/// Wait for `child` to exit, polling with [`Child::try_wait`] rather than blocking on
/// [`Child::wait_with_output`] — the ONLY way to actually bound the wait (nxf ticket 6j6v.81cq):
/// `wait_with_output` takes `self` by value and blocks unconditionally until the child exits,
/// with no way to give up early — there is no bounded variant in `std`, and reaching for one
/// would mean either spawning a watcher thread and losing the ability to `kill()` the child once
/// it has been moved into that thread, or pulling in a crate for what a short poll loop already
/// does directly. If `bound` elapses first, the child is killed (and reaped, to avoid leaving a
/// zombie) and this returns a `TimedOut` error — the exact case that used to hang forever when a
/// misbehaving/absent `at`/`atd` never responds at all (confirmed live: a real run on a machine
/// with no `atd` registered left the CALLING role's entire SDK session wedged for 8+ minutes
/// before being found and killed by hand).
///
/// Reads `stdout`/`stderr` to completion only AFTER the child has actually exited (whether
/// normally within the bound, or via the kill path) — never while polling. This mirrors
/// `wait_with_output`'s own contract closely enough for `AtTimer`'s actual use (a job id printed
/// in one short line on stderr): a child that produced enough output to fill its pipe buffer
/// before exiting could in principle stall this loop, but `at`'s own output is a single short
/// line, so that risk is accepted here rather than adding concurrent-drain complexity for a case
/// that cannot occur with the one real caller this function has.
fn wait_bounded(mut child: Child, bound: Duration) -> std::io::Result<std::process::Output> {
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= bound {
            let _ = child.kill();
            let _ = child.wait(); // reap — avoid leaving a zombie behind.
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("child process did not exit within {bound:?}"),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut stdout = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_end(&mut stdout);
    }
    let mut stderr = Vec::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_end(&mut stderr);
    }
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// The exact argument vector `at` is invoked with, given an already-formatted time spec
/// ([`to_at_time_spec`]'s output): `["-t", <spec>]`. `-t` is REQUIRED — `[[CC]YY]MMDDhhmm[.ss]` is
/// exclusively the `at -t <spec>` argument form; passing it as a bare positional (no `-t`) is a
/// garbled-time error on both GNU and BSD `at` (a real bug this ticket's own review caught: `at`
/// was invoked with the spec as a bare arg, so the real production timer never actually scheduled
/// anything — every real `AtTimer::schedule` call silently failed and was logged, never fired).
/// Factored out as a pure function so the argument SHAPE is unit-testable without spawning the
/// real `at` binary (this module's own doc: the real spawn itself is best-effort, not CI-gated).
fn at_command_args(at_time: &str) -> Vec<String> {
    vec!["-t".to_string(), at_time.to_string()]
}

/// Convert an absolute RFC3339 `deadline` into the one unambiguous, locale-independent time spec
/// `at -t` accepts: `[[CC]YY]MMDDhhmm[.ss]`.
fn to_at_time_spec(deadline: &str) -> Result<String> {
    use time::format_description::well_known::Rfc3339;
    let dt = time::OffsetDateTime::parse(deadline, &Rfc3339)
        .map_err(|e| NxfError::validation(format!("deadline `{deadline}` is not RFC3339: {e}")))?;
    Ok(format!(
        "{:04}{:02}{:02}{:02}{:02}.{:02}",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    ))
}

/// The timer an embedder gets by default: one that schedules **nothing** and says so by doing
/// nothing (PR #269 review, Code Quality #3).
///
/// Not an error, deliberately — unlike [`crate::worker::WorkerConfig::Disabled`], whose refusal is
/// loud because a verb that would spawn a session and silently doesn't is a lie. Nothing here is a
/// lie: a board with no scheduled tick still goes `stale` on its own and is still routable by an
/// explicit `nxc tick`. The scheduled job is an automatic CONVENIENCE over a verb that
/// remain idempotent and callable by hand, which is exactly why it can be declined without anything
/// breaking.
///
/// And declining is the right default for an app. [`AtTimer`] shells out to `at`, in the host's own
/// process, from inside the engine's locked section, for up to
/// [`AT_SUBPROCESS_BOUND`] — a real stall for a desktop app that has its own scheduler and never
/// asked for an `atd` dependency. An app that DOES want OS-level jobs asks for
/// [`TimerConfig::At`] outright.
pub struct DisabledTimer;

impl Timer for DisabledTimer {
    fn schedule(&self, thread_id: &str, _deadline: &str, _command: &str) -> Result<TimerHandle> {
        Ok(TimerHandle(format!("disabled:{thread_id}")))
    }
    fn cancel(&self, _handle: &TimerHandle) -> Result<()> {
        Ok(())
    }
}

/// Which timer to build, as a value rather than a read of process env — [`crate::worker::WorkerConfig`]'s
/// twin on the other external seam, and here for the same reason (PR #269 review, Code Quality #3).
///
/// The seam's own rule is that it reads no environment variable: the CLI keeps deriving its choice
/// from `NXC_TIMER` via [`TimerConfig::from_ambient`], at the CLI adapter, and an embedding app
/// states it outright. Before this, `orchestration` reached for `NXC_TIMER` directly from inside a
/// library call, which meant an app could neither choose nor decline the `at` subprocess it was
/// paying for.
///
/// `#[non_exhaustive]` from the start, so a later timer (a host-supplied one, a cron backend) is an
/// additive minor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TimerConfig {
    /// Schedule nothing — see [`DisabledTimer`]. The default, and what [`crate::engine::Engine`] uses.
    #[default]
    Disabled,
    /// Record what would have been scheduled to `NXC_TIMER_LOG` — see [`DryTimer`].
    Dry,
    /// The real one-shot OS job via `at` — see [`AtTimer`]. The `nxc` CLI's choice everywhere
    /// except macOS.
    At,
    /// The deadline goes into the workspace's own book and the ONE nexus-flow background service
    /// honours it — see [`ServiceTimer`]. The `nxc` CLI's choice on every platform (nxf 6j6v.8see).
    Service,
}

/// Which backend an unset `NXC_TIMER` means on `os` (`std::env::consts::OS`'s spelling).
///
/// A FUNCTION and not a `cfg!` sprinkled through `from_ambient`, for the reason 6j6v.74c0 names in
/// its own scope note: Windows is its own country (epic 6j6v.rf4b) and this clock does not have to
/// run there — but the platform choice must not be wired so that a third backend no longer fits.
/// It is one match arm, testable from any platform, and a Windows one goes next to these.
///
/// **Every platform gets the service** (nxf 6j6v.8see), and that is why this is still a function.
///
/// It used to answer `launchd` on macOS and `at` everywhere else, because the clock was an OS job
/// and each OS has its own. The clock is not an OS job any more: it is a line in the workspace's
/// deadline book, and the process that reads it is the same nexus-flow service on every platform.
/// There is nothing left for the platform to decide.
///
/// Kept as a function rather than collapsed into a constant for the reason 6j6v.74c0 gave when it
/// introduced it: the moment a platform needs its own answer again, this is the one arm to add, and
/// it is testable from any platform. Windows is its own country (6j6v.rf4b) and reaches the same
/// answer here — the service runs there as an ordinary foreground process under whatever supervisor
/// the host uses; what Windows does NOT have is the launchd agent that starts it, which is
/// `nxs sync daemon install`'s own platform gate and stated there.
pub(crate) fn platform_default(_os: &str) -> TimerConfig {
    TimerConfig::Service
}

impl TimerConfig {
    /// The CLI's selection rules, unchanged, over an INJECTED ambient lookup rather than
    /// `std::env::var` directly — [`crate::worker::WorkerConfig::from_ambient`]'s discipline, for
    /// its reason: the decision stays pure and unit-testable without mutating process-global env.
    ///
    /// Never yields [`TimerConfig::Disabled`]: "no timer" is a deliberate library-side choice, not
    /// something an unset variable should silently produce — again exactly the worker's rule.
    pub fn from_ambient(ambient: impl Fn(&str) -> Option<String>) -> Result<TimerConfig> {
        match ambient("NXC_TIMER").as_deref() {
            Some("dry") => Ok(TimerConfig::Dry),
            Some("at") => Ok(TimerConfig::At),
            Some("service") => Ok(TimerConfig::Service),
            // UNSET now asks the platform ([`platform_default`]) instead of always answering `at`.
            // That is the whole of 6j6v.74c0 at this seam: on macOS `at` took every job and ran
            // none of them, so the default was a clock that did not tick. Naming a backend
            // explicitly still overrides it in both directions.
            None => Ok(platform_default(std::env::consts::OS)),
            Some(other) => Err(NxfError::io(format!("unknown NXC_TIMER '{other}'"))),
        }
    }

    /// Build the timer for a caller that IS standing in its workspace — the CLI. `Arc` (not `Box`)
    /// because a long-lived owner shares it across clones of itself ([`crate::engine::Engine`] is
    /// `Clone` and every clone drives the same timer).
    pub fn build(&self) -> std::sync::Arc<dyn Timer> {
        self.build_for(None)
    }

    /// Build the timer for a caller whose working directory is NOT its workspace — an embedding app
    /// (nxf 6j6v.8see).
    ///
    /// The distinction is new and it is load-bearing. Every earlier backend resolved the workspace
    /// from `std::env::current_dir()` at schedule time, which is right for `nxc` (an invocation is
    /// standing in the project) and quietly wrong for an app, whose working directory is wherever it
    /// was launched from. It never bit, because an app's default is [`TimerConfig::Disabled`] — but
    /// the owner decision of 2026-08-25 makes an app arming real windows the ordinary case, so the
    /// workspace has to come from the handle rather than from the process.
    ///
    /// Additive on purpose: [`build`](Self::build) keeps its signature, so nothing that already
    /// calls it changes.
    pub fn build_in(&self, ws: &nxs_foundation::workspace::Workspace) -> std::sync::Arc<dyn Timer> {
        self.build_for(Some(&ws.dir))
    }

    fn build_for(&self, nxs_dir: Option<&std::path::Path>) -> std::sync::Arc<dyn Timer> {
        match self {
            TimerConfig::Disabled => std::sync::Arc::new(DisabledTimer),
            TimerConfig::Dry => std::sync::Arc::new(DryTimer),
            TimerConfig::At => std::sync::Arc::new(AtTimer::new()),
            TimerConfig::Service => std::sync::Arc::new(match nxs_dir {
                Some(dir) => ServiceTimer::in_workspace(dir),
                None => ServiceTimer::new(),
            }),
        }
    }
}

/// Which [`Timer`] to use, read from `NXC_TIMER` — the CLI's own resolution:
/// [`TimerConfig::from_ambient`] over the real process env, then built. Mirrors
/// [`crate::worker::select_worker`] exactly.
pub fn select_timer() -> Result<std::sync::Arc<dyn Timer>> {
    Ok(TimerConfig::from_ambient(|key| std::env::var(key).ok())?.build())
}

/// The command a scheduled one-shot job runs to re-check a declared channel board's deadline.
///
/// It named `nxc workflow tick` until 6j6v.dvyq §3 took the `workflow` group off the surface; the
/// verb itself is unchanged, hidden from `--help`, and this is its only caller that is not a human
/// forcing a check by hand.
pub fn tick_command(thread_id: &str) -> String {
    format!("nxc tick --thread {thread_id}")
}

/// The command a scheduled one-shot job runs to hand a settled caller what was held for it
/// (nxf 6j6v.gn8b) — [`tick_command`]'s sibling, and the shell form of
/// [`nxs_service::timers::Job::ChatDeliver`].
pub fn delivery_command(session: &str) -> String {
    format!("nxc session deliver {session}")
}

/// The command a scheduled one-shot job runs to take up an operation that stopped at an
/// availability boundary (nxf 6j6v.npy3) — [`delivery_command`]'s sibling, and the shell form of
/// [`nxs_service::timers::Job::ChatResume`].
///
/// `--session` rather than `--thread`: the hold is keyed on the session that stopped, and the
/// operation is resolved from it. A human types the thread form, because a thread id is what
/// `nxc status` puts in front of them.
pub fn resume_command(session: &str) -> String {
    format!("nxc resume --session {session}")
}

/// Schedule a one-shot tick for `thread_id`, due at `deadline` (an absolute RFC3339 instant — the
/// caller resolves any duration form BEFORE this, mirroring `facade::ask`'s own "resolve before
/// writing" discipline). The scheduled command is exactly what a real timer job runs: `nxc tick
/// --thread <thread_id>` — keyed on the THREAD (see this module's own doc).
pub fn schedule_tick(thread_id: &str, deadline: &str) -> Result<TimerHandle> {
    select_timer()?.schedule(thread_id, deadline, &tick_command(thread_id))
}

// `liveness_command`/`schedule_liveness` stood here — the other clock this module drove, watching
// a RUN's current step for a stall. REMOVED with the run record (6j6v.dvyq §3): a step-liveness
// window belonged to a run's current step, and there are no runs.

/// Cancel a previously scheduled tick. A real, tested, standalone primitive (see this module's own
/// doc) — but no live call site in this ticket persists a handle across a process boundary to call
/// this from a LATER invocation; the idempotent `nxc tick` is the actual "no double-fire"
/// guarantee (`cli.rs`'s `tick`).
pub fn cancel(handle: &TimerHandle) -> Result<()> {
    select_timer()?.cancel(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `NXC_TIMER`/`NXC_TIMER_LOG` are process-global env, so these unit tests serialize against
    // each other (mirrors `worker.rs`'s own tests needing no such guard only because they take env
    // as an explicit argument — `select_timer`/`DryTimer` read real process env directly, so a lock
    // is the honest fix here, matching `channel_complete.rs`'s established precedent for direct,
    // non-subprocess tests that touch process env).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard<'a> {
        #[allow(dead_code)]
        lock: std::sync::MutexGuard<'a, ()>,
        /// `PATH` as it was on acquisition. The `AtTimer` tests below PREPEND a stub directory to
        /// it, so this one is restored rather than removed — clearing it would leave every later
        /// test in this binary unable to find any external command at all.
        path: Option<String>,
    }
    impl Drop for EnvGuard<'_> {
        fn drop(&mut self) {
            std::env::remove_var("NXC_TIMER");
            std::env::remove_var("NXC_TIMER_LOG");
            match &self.path {
                Some(p) => std::env::set_var("PATH", p),
                None => std::env::remove_var("PATH"),
            }
        }
    }
    fn lock_env() -> EnvGuard<'static> {
        EnvGuard {
            lock: ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner()),
            path: std::env::var("PATH").ok(),
        }
    }

    #[test]
    fn dry_timer_schedule_then_cancel_round_trips_in_the_same_process() {
        let _guard = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("timer.log");
        std::env::set_var("NXC_TIMER", "dry");
        std::env::set_var("NXC_TIMER_LOG", &log);

        let handle = schedule_tick("m-th1", "2026-07-22T00:20:00Z").unwrap();
        assert!(!handle.0.is_empty());
        cancel(&handle).unwrap();

        let contents = std::fs::read_to_string(&log).unwrap();
        assert!(
            contents.contains("schedule thread=m-th1 deadline=2026-07-22T00:20:00Z"),
            "{contents}"
        );
        assert!(
            contents.contains(&format!("cancel handle={}", handle.0)),
            "{contents}"
        );
    }

    #[test]
    fn dry_timer_schedule_carries_the_tick_command() {
        let _guard = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("timer.log");
        std::env::set_var("NXC_TIMER", "dry");
        std::env::set_var("NXC_TIMER_LOG", &log);

        schedule_tick("m-th2", "2026-07-22T00:20:00Z").unwrap();

        let contents = std::fs::read_to_string(&log).unwrap();
        assert!(
            contents.contains("command=nxc tick --thread m-th2"),
            "{contents}"
        );
    }

    #[test]
    fn dry_timer_with_no_log_var_set_is_a_silent_no_op() {
        let _guard = lock_env();
        std::env::remove_var("NXC_TIMER_LOG");
        std::env::set_var("NXC_TIMER", "dry");
        let handle = schedule_tick("m-th3", "2026-07-22T00:20:00Z").unwrap();
        cancel(&handle).unwrap();
    }

    #[test]
    fn select_timer_rejects_an_unknown_choice() {
        let _guard = lock_env();
        std::env::set_var("NXC_TIMER", "bogus");
        // `Arc<dyn Timer>` isn't `Debug`, so `Result::unwrap_err` (which requires the `Ok` side to
        // be `Debug`) doesn't apply here — match directly instead.
        match select_timer() {
            Err(e) => assert_eq!(e.kind, nxs_foundation::error::ErrorKind::Io),
            Ok(_) => panic!("expected an error for an unknown NXC_TIMER value"),
        }
    }

    #[test]
    fn from_ambient_mirrors_select_timer_over_an_injected_lookup() {
        // The worker seam's own discipline (PR #269 review, Code Quality #3): the selection rules
        // are pure and testable without mutating process-global env, which parallel tests in one
        // binary would otherwise race on.
        let ambient = |pairs: Vec<(&'static str, &'static str)>| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(
            TimerConfig::from_ambient(ambient(vec![("NXC_TIMER", "dry")])).unwrap(),
            TimerConfig::Dry
        );
        assert_eq!(
            TimerConfig::from_ambient(ambient(vec![("NXC_TIMER", "at")])).unwrap(),
            TimerConfig::At
        );
        assert_eq!(
            TimerConfig::from_ambient(ambient(vec![("NXC_TIMER", "service")])).unwrap(),
            TimerConfig::Service
        );
        // `launchd` NAMED a backend that no longer exists (nxf 6j6v.8see). It must be an error and
        // not a silent fallback: somebody with it in a shell profile has to learn that the clock
        // moved, not quietly get a different one.
        let err = TimerConfig::from_ambient(ambient(vec![("NXC_TIMER", "launchd")])).unwrap_err();
        assert!(err.msg.contains("launchd"), "{}", err.msg);
        // Unset means the real timer, exactly as it always has for the CLI — never `Disabled`, which
        // is a library-side choice a caller makes deliberately. WHICH real timer is now the
        // platform's answer (nxf 6j6v.74c0), so this asserts against the same function the
        // selection uses rather than pinning one platform's answer into a test that runs on three.
        assert_eq!(
            TimerConfig::from_ambient(ambient(vec![])).unwrap(),
            platform_default(std::env::consts::OS)
        );
        assert_eq!(TimerConfig::default(), TimerConfig::Disabled);
        let err = TimerConfig::from_ambient(ambient(vec![("NXC_TIMER", "banana")])).unwrap_err();
        assert!(err.msg.contains("banana"), "{}", err.msg);
    }

    /// The platform choice itself, asserted from every platform — a `cfg!` here would make each
    /// runner check only its own arm.
    ///
    /// Every platform now answers `Service` (nxf 6j6v.8see): the clock stopped being an OS job, so
    /// there is nothing left for the platform to decide. The test stays, and stays exhaustive,
    /// because the FUNCTION stays — the moment a platform needs its own answer again this is where
    /// it goes, and a test that only checked the runner's own arm would not notice.
    #[test]
    fn every_platform_arms_its_deadlines_through_the_one_service() {
        for os in ["macos", "linux", "windows", "freebsd"] {
            assert_eq!(platform_default(os), TimerConfig::Service, "{os}");
        }
    }

    /// The whole selection, end to end, for the platform this actually runs on: an unset
    /// `NXC_TIMER` must build the SERVICE backend. Asserted through `build()`'s real output, not
    /// through the enum, because the enum is the easy half.
    #[test]
    fn an_unset_timer_variable_builds_the_service_backend() {
        let cfg = TimerConfig::from_ambient(|_| None).unwrap();
        assert_eq!(cfg, TimerConfig::Service);
        // …and `build()` really produces THAT backend, proved by an error only it can raise.
        //
        // **The probe is a thread id, not a deadline** (review of nxf 6j6v.fabb, Test Quality #8).
        // This is the one test in the tree that builds a REAL `ServiceTimer` — unpinned, so it
        // would resolve the developer's own workspace and `~/.nexusflow`. An unschedulable thread
        // id is refused by `timers::arm`'s charset gate, which runs before anything is written, so
        // this cannot reach a file even if the body below it is reordered.
        let err = cfg
            .build()
            .schedule("not a thread id", "2099-01-01T00:00:00Z", "true")
            .unwrap_err();
        assert!(err.msg.contains("not a schedulable id"), "{}", err.msg);
    }

    #[test]
    fn the_disabled_timer_schedules_nothing_and_is_not_an_error() {
        // Unlike a disabled WORKER, which refuses loudly: a verb that would spawn a session and
        // silently doesn't is a lie, whereas a board with no scheduled tick still goes stale on its
        // own and is still routable by an explicit `nxc tick`. The scheduled job is an
        // automatic convenience over verbs that stay idempotent and callable by hand.
        let _guard = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("timer.log");
        std::env::set_var("NXC_TIMER_LOG", &log);
        let handle = TimerConfig::Disabled
            .build()
            .schedule("m-th1", "2026-07-22T00:20:00Z", "nxc tick --thread m-th1")
            .expect("declining to schedule is not a failure");
        TimerConfig::Disabled.build().cancel(&handle).unwrap();
        assert!(!log.exists(), "nothing at all was scheduled");
    }

    #[test]
    fn to_at_time_spec_formats_a_canonical_rfc3339_instant() {
        assert_eq!(
            to_at_time_spec("2026-07-22T00:20:05Z").unwrap(),
            "202607220020.05"
        );
    }

    #[test]
    fn to_at_time_spec_rejects_a_non_rfc3339_string() {
        let err = to_at_time_spec("20m").unwrap_err();
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Validation);
    }

    #[test]
    fn at_command_args_uses_the_dash_t_time_spec_flag() {
        // The real bug this test guards against: `at` was invoked with the time spec as a bare
        // positional (no `-t`), which is a garbled-time error on both GNU and BSD `at` — the spec
        // format `[[CC]YY]MMDDhhmm[.ss]` is exclusively the `-t` argument form. `Command`'s built
        // argv isn't cheaply inspectable after `.spawn()`, so this is asserted on the pure argument-
        // vector builder `AtTimer::schedule` actually calls, rather than on a spawned process.
        assert_eq!(
            at_command_args("202607220020.05"),
            vec!["-t".to_string(), "202607220020.05".to_string()]
        );
    }

    // ---- regression: 6j6v.81cq -- `wait_bounded` must never hang forever --------------------------
    //
    // Confirmed live (`crates/chat/tests/smoke_v3.rs`, nxf ticket 6j6v.8e45): on a machine with no
    // `atd`/`atrun` registered, a bare `at` invocation never responds at all, and the OLD
    // `child.wait_with_output()` call (no bound) blocked the ENTIRE calling `nxc` process forever —
    // which, called from inside a live role's own Bash tool, wedged that role's whole SDK session
    // indefinitely (an orphaned session was directly observed still running 8+ minutes later). This
    // is deliberately NOT reproduced via the real `at` binary (that would make the test's pass/fail
    // depend on whether THIS machine happens to have a working atd, exactly the non-portability the
    // brief warned against) — a `sleep` far longer than the bound stands in for any subprocess that
    // never responds, real `at` included.
    #[test]
    fn wait_bounded_returns_a_timeout_err_instead_of_hanging_forever() {
        let child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawning `sleep` (assumed present, like `at`/`atrm` already are)");
        let pid = child.id();

        let start = Instant::now();
        let result = wait_bounded(child, Duration::from_millis(200));
        let elapsed = start.elapsed();

        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::TimedOut,
            "a hanging child must surface as a timeout, not success or a different error kind"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "must return promptly once the bound elapses, not wait out the full 30s sleep: {elapsed:?}"
        );

        // The child must actually be killed, not merely abandoned (which would leave a real
        // process running for the rest of its 30s sleep every time this test runs).
        std::thread::sleep(Duration::from_millis(100));
        let still_alive = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .expect("spawning `kill -0`")
            .success();
        assert!(
            !still_alive,
            "the timed-out child (pid {pid}) must be killed, not left running"
        );
    }

    // ---- AtTimer: the real one-shot timer (nxf 6j6v.amy1) -----------------------------------------
    //
    // Until now the suite exercised `DryTimer` and the pure helpers only, so the two `Timer` methods
    // that actually ship — `AtTimer::schedule`/`cancel` — had NO coverage: not the stdin handover
    // (the job body `at` is supposed to run), not the job-id parse out of `at`'s stderr, not the
    // non-zero-exit error path, not `cancel`'s `at:` prefix strip. That is the one component with no
    // real coverage at all, and the new `schedule_liveness` clock leans on it harder than before.
    //
    // These drive the SHIPPED code path — a real `Command::spawn`, real pipes, real `wait_bounded` —
    // against a STUB `at`/`atrm` first on `PATH`, so what is asserted is `AtTimer`'s own behavior
    // rather than a re-implementation of it, and the result does not depend on whether this machine
    // happens to run an `atd`. The gated test at the end covers the real binary where there is one.
    // `PATH` is process-global, hence `lock_env` (which restores it); the stubs shadow only
    // `at`/`atrm`, which nothing else in this binary invokes.
    //
    // `#[cfg(unix)]` on the block: `at`/`atrm` are a Unix mechanism (the module's whole reason for
    // `AtTimer`), and marking the executable bit needs a Unix-only trait. Without the gate these
    // would not COMPILE on Windows — the loud, confusing kind of failure to hand whoever picks up
    // 6j6v.mwn4 (adding `windows-latest` to CI); with it they simply do not run there, which is the
    // honest answer for a timer that cannot exist.

    /// Write `script` as an executable `name` in `dir` — a stub for a binary `AtTimer` shells out to.
    #[cfg(unix)]
    fn stub_bin(dir: &std::path::Path, name: &str, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Put `dir` first on `PATH` for the rest of the holding test.
    #[cfg(unix)]
    fn prepend_path(dir: &std::path::Path) {
        let orig = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{orig}", dir.display()));
    }

    #[test]
    #[cfg(unix)]
    fn at_timer_schedule_hands_at_the_job_body_on_stdin_and_parses_the_job_id_from_its_stderr() {
        let _guard = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        let args = tmp.path().join("argv");
        let body = tmp.path().join("stdin");
        stub_bin(
            tmp.path(),
            "at",
            // Records what it was called with and what it was fed, then answers the way the real
            // `at` does: the job id goes to STDERR, in the conventional "job <n> at <date>" line.
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {argv}\ncat > {stdin}\n\
                 echo 'job 42 at Wed Jul 22 00:20:00 2026' >&2\n",
                argv = args.display(),
                stdin = body.display()
            ),
        );
        prepend_path(tmp.path());

        let handle = AtTimer::assuming_the_daemon_runs_jobs()
            .schedule("m-th1", "2026-07-22T00:20:05Z", &tick_command("m-th1"))
            .expect("the stub `at` accepts the job");

        assert_eq!(
            handle,
            TimerHandle("at:42".to_string()),
            "the handle carries the job id `at` reported, so `cancel` can name it"
        );
        assert_eq!(
            std::fs::read_to_string(&args).unwrap(),
            "-t\n202607220020.05\n",
            "invoked as `at -t <spec>` — the spec form is exclusively the -t argument, a bare \
             positional is a garbled-time error on both GNU and BSD at"
        );
        assert_eq!(
            std::fs::read_to_string(&body).unwrap(),
            "nxc tick --thread m-th1",
            "the scheduled job body is the tick command, delivered on stdin and its pipe closed \
             (an unclosed stdin leaves the real `at` waiting for EOF forever)"
        );
    }

    #[test]
    #[cfg(unix)]
    fn at_timer_schedule_surfaces_a_refusing_at_as_an_io_error_carrying_its_stderr() {
        let _guard = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        stub_bin(
            tmp.path(),
            "at",
            "#!/bin/sh\ncat > /dev/null\necho 'at: garbled time' >&2\nexit 1\n",
        );
        prepend_path(tmp.path());

        let err = AtTimer::assuming_the_daemon_runs_jobs()
            .schedule("m-th1", "2026-07-22T00:20:05Z", "nxc tick --thread m-th1")
            .expect_err("a non-zero `at` exit is a failure, not a silently-lost job");
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Io);
        assert!(
            err.msg.contains("garbled time"),
            "the reason `at` gave has to reach the caller verbatim — this is the only place it \
             exists: {}",
            err.msg
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_platform_whose_daemon_will_not_run_jobs_refuses_before_it_submits_anything() {
        // nxf 6j6v.p3sm, both halves in one assertion. On macOS `at` ACCEPTS the job and exits 0
        // while nothing ever runs it, so a successful submission proved nothing and the caller could
        // not see the difference anywhere. And the submission it no longer makes is the one that
        // cost every `send`/`reply` on a timed thread up to `AT_SUBPROCESS_BOUND`.
        let _guard = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        let called = tmp.path().join("at-was-called");
        stub_bin(
            tmp.path(),
            "at",
            &format!(
                "#!/bin/sh\ntouch {called}\ncat > /dev/null\necho 'job 7 at Wed Jul 22 00:20:00 2026' >&2\n",
                called = called.display()
            ),
        );
        prepend_path(tmp.path());

        // The reason is INJECTED rather than taken from the platform's own constant: what this test
        // holds is the mechanism — a refusal carries its reason and submits nothing — and that is
        // true on every platform. The macOS wording has its own test below, where it can actually
        // be produced.
        let timer = AtTimer::new();
        let _ = timer.daemon_refusal.set(Some(
            "this platform's job daemon will not run the job".to_string(),
        ));

        let err = timer
            .schedule("m-th1", "2026-07-22T00:20:05Z", &tick_command("m-th1"))
            .expect_err("a daemon that will not run the job is not a schedule");
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Io);
        assert!(
            err.msg.contains("will not run the job"),
            "the caller is told WHAT is wrong, not merely that something is: {}",
            err.msg
        );
        assert!(
            err.msg.contains("nxc tick --thread"),
            "and what to do instead, since the board still goes stale on demand: {}",
            err.msg
        );
        assert!(
            !called.exists(),
            "and no `at` subprocess is spawned at all — that is the bound this stops paying"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn the_macos_refusal_names_the_condition_the_workaround_and_the_way_on() {
        // The wording half, where it can be produced. A refusal that says only "not scheduled"
        // sends a reader looking for a bug in `nxc`; this one names the platform fact, the
        // developer's own way out, and the verb that still works.
        assert!(ATRUN_NOT_LOADED.contains("atrun"), "{ATRUN_NOT_LOADED}");
        assert!(
            ATRUN_NOT_LOADED.contains("launchctl load -w"),
            "{ATRUN_NOT_LOADED}"
        );
        assert!(
            ATRUN_NOT_LOADED.contains("not a fix for a shipped product"),
            "the workaround must not read as the answer: {ATRUN_NOT_LOADED}"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn the_probe_agrees_with_launchctl_about_this_very_machine() {
        // The probe is the whole basis for refusing, so it is held against the SAME question asked
        // directly rather than against an assumption about how this runner is configured — a Mac
        // with `atrun` deliberately loaded must not redden this, and must not silently pass either.
        let _guard = lock_env();
        let loaded = Command::new("launchctl")
            .args(["print", "system/com.apple.atrun"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .expect("spawning `launchctl` (present on every macOS)");
        assert_eq!(
            probe_at_daemon().is_none(),
            loaded,
            "the probe must answer what launchd answers: loaded={loaded}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn at_timer_cancel_calls_atrm_with_the_bare_job_id() {
        let _guard = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        let args = tmp.path().join("argv");
        stub_bin(
            tmp.path(),
            "atrm",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {argv}\n",
                argv = args.display()
            ),
        );
        prepend_path(tmp.path());

        AtTimer::assuming_the_daemon_runs_jobs()
            .cancel(&TimerHandle("at:42".to_string()))
            .expect("the stub `atrm` accepts the job id");

        assert_eq!(
            std::fs::read_to_string(&args).unwrap(),
            "42\n",
            "`atrm` takes the bare job id — the `at:` prefix is this crate's own handle namespace \
             and must be stripped, or every cancel names a job that does not exist"
        );
    }

    #[test]
    #[cfg(unix)]
    fn at_timer_cancel_surfaces_a_failing_atrm_as_an_io_error() {
        let _guard = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        stub_bin(tmp.path(), "atrm", "#!/bin/sh\nexit 1\n");
        prepend_path(tmp.path());

        let err = AtTimer::assuming_the_daemon_runs_jobs()
            .cancel(&TimerHandle("at:42".to_string()))
            .expect_err("a non-zero `atrm` exit is a failure");
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Io);
        assert!(err.msg.contains("42"), "{}", err.msg);
    }

    /// The real thing, where the runner has one — schedule an actual `at` job through `AtTimer` and
    /// cancel it through `AtTimer`, then prove the queue no longer lists it.
    ///
    /// **The gate is a probe, not a guess, and it never turns a regression into a skip.** It shells
    /// out to `at` DIRECTLY (not through the code under test) with the same harmless job; only if
    /// that succeeds does the environment have a usable `at`, and from there any failure of
    /// `AtTimer` is a real defect rather than a missing dependency. Gating on the mere presence of
    /// the binary would not do: `at` exists on plenty of machines that refuse every job.
    ///
    /// The scheduled body is `/usr/bin/true` at a far-future date, never `nxc tick`: this
    /// test must not leave anything behind that a machine with a live `atrun` would eventually
    /// execute. Both the probe's job and the timer's job are removed before it returns.
    #[test]
    fn at_timer_round_trips_a_real_at_job_where_the_runner_has_a_usable_at() {
        // Takes the lock without setting anything: this is the one test that wants the REAL `at`,
        // so it must not run while a stub test above has `PATH` hijacked.
        let _guard = lock_env();
        const FAR_FUTURE: &str = "2027-12-25T00:00:00Z";
        let Some(probe_job) = probe_real_at(FAR_FUTURE) else {
            eprintln!("SKIP at_timer_round_trips_a_real_at_job: no usable `at` on this runner");
            return;
        };
        atrm(&probe_job);

        let handle = AtTimer::assuming_the_daemon_runs_jobs()
            .schedule("m-th1", FAR_FUTURE, "/usr/bin/true")
            .expect("a runner whose bare `at` just accepted a job must accept this one too");
        let job = handle
            .0
            .strip_prefix("at:")
            .expect("the handle names a real job")
            .to_string();
        assert!(
            at_queue_lists(&job),
            "the job `at` reported ({job}) is really in the queue"
        );

        AtTimer::assuming_the_daemon_runs_jobs()
            .cancel(&handle)
            .expect("cancel removes it");
        assert!(
            !at_queue_lists(&job),
            "after cancel the job is gone from the queue"
        );
    }

    /// Schedule a harmless far-future job with a bare `at` and return its job id, or `None` if this
    /// machine has no `at` that accepts jobs at all. The caller removes the job.
    fn probe_real_at(deadline: &str) -> Option<String> {
        let spec = to_at_time_spec(deadline).ok()?;
        let mut child = Command::new("at")
            .args(["-t", &spec])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        child
            .stdin
            .take()?
            .write_all(b"/usr/bin/true")
            .expect("writing the probe job body");
        let out = wait_bounded(child, Duration::from_secs(8)).ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stderr)
            .split_whitespace()
            .skip_while(|w| *w != "job")
            .nth(1)
            .map(str::to_string)
    }

    fn atrm(job: &str) {
        let _ = Command::new("atrm").arg(job).status();
    }

    /// Whether `at -l` still lists `job` — the queue's own answer, not this crate's.
    fn at_queue_lists(job: &str) -> bool {
        let Ok(out) = Command::new("at").arg("-l").output() else {
            return false;
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l.split_whitespace().next() == Some(job))
    }

    #[test]
    fn wait_bounded_returns_the_real_output_for_a_child_that_exits_within_the_bound() {
        let child = Command::new("sh")
            .args(["-c", "echo out-line; echo err-line >&2; exit 3"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawning `sh`");

        let output =
            wait_bounded(child, Duration::from_secs(5)).expect("child exits well within the bound");

        assert_eq!(output.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "out-line");
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "err-line");
    }
}
