//! **A worker can stop a session it started** (nxf 6j6v.b9nf) — the capability the next item needs
//! before `withdraw` can take back a RUNNING round, stated on the seam that owns the processes.
//!
//! The engine has never been able to END a session. [`Worker`]'s own doc calls the absence
//! deliberate, and it was: every question the engine asked a worker was a READ (`session_is_running`,
//! `working_copy`, `resumes_sessions`), and the one thing that ends a session was the session
//! itself, announcing its end through `nxc session ended`. That holds until somebody withdraws a
//! commission whose session is still writing into the checkout: the withdrawal parks the work, the
//! session keeps going, and the park is stale before its receipt is printed.
//!
//! So this adds the one WRITE, in the shape the other capabilities took: a question first
//! ([`Worker::stops_sessions`], `false` by default, so a host that cannot stop anything is refused by
//! name rather than believed), and then the act ([`Worker::stop_session`]), which delivers a request
//! and returns — it does not wait for the process to be gone, because the caller already has
//! [`Worker::session_is_running`] for that and a wait with the store mutex held is the one thing a
//! `Worker` method may never do.
//!
//! **SIGTERM, to the one pid, and nothing harder.** The sidecar is a process with a teardown: it
//! binds the runtime session, flushes the transcript and announces its end, and every one of those
//! is lost to a SIGKILL. The pid file [`SidecarWorker::session_is_running`] reads is the pid file
//! this sends to — same file, same parse, same identity check — so the two cannot name different
//! processes.
//!
//! **And the pid alone does not earn the signal** (fix round 3 of this item's review, Integrity
//! #1). Claims are deliberately never removed, so a session that crashed leaves a file naming a
//! number the operating system may since have handed to an editor, a dev server or another `nxc` of
//! the same user — and `kill(pid, 0)` says `true` about that stranger. The claim therefore records
//! the instant its process started, and the stop compares it against what the operating system
//! reports for the pid it finds. A claim that disagrees names a session that ENDED; a claim that
//! records nothing (the old shape) cannot be told apart from a recycled pid and is refused by name
//! rather than signalled. Both are here, each driven against a real live child that must survive.

use std::path::PathBuf;

use nexus_chat::worker::{SidecarWorker, TriggerOutcome, TriggerRequest, TriggerResult, Worker};

/// The least a host can implement: `trigger` and nothing else. Every defaulted method answers for
/// it, and the two new ones have to answer in the SAFE direction — a host that never said it can
/// stop a session must not be believed to.
struct MinimalWorker;

impl Worker for MinimalWorker {
    fn trigger(&self, _req: TriggerRequest) -> TriggerResult {
        Ok(TriggerOutcome::Accepted)
    }
}

#[test]
fn a_default_worker_cannot_stop_a_session() {
    let worker = MinimalWorker;
    assert!(
        !worker.stops_sessions(),
        "a worker that has not said it can stop a session cannot — a caller refuses by name on this"
    );
    let refused = worker
        .stop_session("s-1")
        .expect_err("the default is a refusal, never a silent no-op that reads as delivered");
    assert!(
        refused.contains("cannot stop"),
        "and the refusal says what it is: {refused}"
    );
}

/// A `SidecarWorker` standing at `root`. The sidecar path is never run here — `stop_session` and
/// `session_is_running` only read the pid file — so it may name a file that does not exist.
fn sidecar_at(root: &std::path::Path) -> SidecarWorker {
    SidecarWorker {
        sidecar: root.join("no-such-sidecar.mjs"),
        cwd: root.to_path_buf(),
    }
}

/// Where `SidecarWorker` keeps its claims — the layout `working_tree_two_process_e2e.rs` writes to
/// and `SidecarWorker::session_is_running` reads from.
fn pid_file(root: &std::path::Path, session: &str) -> PathBuf {
    let logs = root.join(".nxs/agent-logs");
    std::fs::create_dir_all(&logs).unwrap();
    logs.join(format!("{session}.pid"))
}

#[cfg(unix)]
mod on_a_real_process {
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use super::{pid_file, sidecar_at};
    use nexus_chat::worker::{SessionStop, Worker};

    /// A child that is killed and reaped when the test ends, whichever way it ends — a `sleep 30`
    /// left behind by a failed assertion would outlive the run.
    struct Reaped(Child);

    impl Drop for Reaped {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn a_live_process() -> Reaped {
        Reaped(
            Command::new("sleep")
                .arg("30")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn a real process to be alive"),
        )
    }

    #[test]
    fn the_sidecar_worker_stops_the_process_named_by_the_pid_file() {
        let tmp = tempfile::tempdir().unwrap();
        let worker = sidecar_at(tmp.path());
        let mut alive = a_live_process();
        // The claim exactly as `SessionLock` writes it: the pid AND the instant that process
        // started, which is what lets the stop prove the process is this session's.
        std::fs::write(
            pid_file(tmp.path(), "s-1"),
            nexus_chat::worker::session_claim_for(alive.0.id()),
        )
        .unwrap();
        assert!(
            worker.session_is_running("s-1"),
            "the premise: the pid file names a process that is alive"
        );

        assert!(
            worker.stops_sessions(),
            "the sidecar owns real processes, so it is the one worker that can stop one"
        );
        worker
            .stop_session("s-1")
            .expect("the stop is delivered to a live pid");

        // The request is DELIVERED, not waited out: `stop_session` returns at once and the caller
        // watches `session_is_running`. Here the test is the child's parent, so it also has to reap
        // — a signalled child is a zombie until somebody does, and `kill(pid, 0)` on a zombie still
        // says "exists", exactly as a real sidecar's parent (`init`, after `nxc` detached it) would
        // have reaped it by the time anything looks.
        let bound = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = alive.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < bound,
                "the process named by the pid file was still alive 5 s after the stop"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(
            status.signal(),
            Some(libc::SIGTERM),
            "SIGTERM and nothing harder — the sidecar's teardown needs the chance to run: {status:?}"
        );
        assert!(
            !worker.session_is_running("s-1"),
            "and the liveness read that a caller waits on now says so"
        );
    }

    /// **A live process that is not this session's is never signalled** (nxf 6j6v.b9nf, fix round
    /// 3 of this item's review, Integrity #1).
    ///
    /// Claims are deliberately never removed, so a session that crashed or a machine that rebooted
    /// leaves a file naming a number the operating system is free to hand to something else. This
    /// is that file: a live, unrelated child of this test, named by a claim that records nothing
    /// about WHICH process it means — the shape every claim had before this fix. `kill(pid, 0)`
    /// says `true` about it, which is exactly the answer that used to be enough to send a SIGTERM.
    ///
    /// It is refused BY NAME rather than treated as gone, because nothing here says the session
    /// ended — only that this file cannot prove which process is behind it.
    #[test]
    fn a_live_process_named_by_a_claim_with_no_recorded_identity_is_not_signalled() {
        let tmp = tempfile::tempdir().unwrap();
        let worker = sidecar_at(tmp.path());
        let mut bystander = a_live_process();
        std::fs::write(
            pid_file(tmp.path(), "s-stale"),
            bystander.0.id().to_string(),
        )
        .unwrap();

        let refused = worker.stop_session("s-stale").expect_err(
            "a claim that cannot prove which process it names is refused, never signalled",
        );
        assert!(
            refused.contains("s-stale") && refused.contains("recycled pid"),
            "refused by name, saying what could not be established: {refused}"
        );
        // The process is still there. A SIGTERM is delivered long before this, and `sleep` dies of
        // it — so a regression shows up as a child that has exited, not as a flake.
        std::thread::sleep(Duration::from_millis(250));
        assert!(
            bystander.0.try_wait().unwrap().is_none(),
            "an unrelated live process was signalled by a stop for a session it has nothing to do \
             with"
        );
    }

    /// **The goal state is not a failure** (nxf 6j6v.b9nf, fix round 3, Code Quality #6). A session
    /// that ended between the liveness read and the stop — the ordinary race for a round that was
    /// about to finish — reaches exactly the state the caller asked for. Reported as an `Err` it
    /// became a `session_not_stopped` warning saying "the process is still there; stop it by hand"
    /// about a process that had gone, and `nxc withdraw` exited 1 on success.
    ///
    /// The claim here is also an OLD one (a bare pid, no identity), which pins the order of the two
    /// questions: liveness is asked first, so a claim whose process is GONE is answered as gone
    /// rather than as unverifiable.
    #[test]
    fn stopping_a_session_whose_process_is_already_gone_is_the_state_the_stop_was_for() {
        let tmp = tempfile::tempdir().unwrap();
        let worker = sidecar_at(tmp.path());
        let gone = a_live_process();
        let pid = gone.0.id();
        drop(gone);
        std::fs::write(pid_file(tmp.path(), "s-gone"), pid.to_string()).unwrap();

        let outcome = worker.stop_session("s-gone").expect(
            "a session that is already over is the state the stop was for, not a failure of it",
        );
        match outcome {
            SessionStop::NothingToStop(note) => assert!(
                note.contains("s-gone") && note.contains(&pid.to_string()),
                "the note names the session and the pid it found: {note}"
            ),
            other => panic!("nothing was signalled, so nothing was requested: {other:?}"),
        }
    }

    /// **A claim on a RECYCLED pid names a session that ended** (nxf 6j6v.b9nf, fix round 3,
    /// Integrity #1) — the case the recorded identity exists to catch, and the one the old-format
    /// refusal above cannot distinguish.
    ///
    /// Here the claim records an instant no live process could have: the pid is alive, and it
    /// started long after the claim says. That is proof the session ended and its number was handed
    /// on — so the session reads as GONE, not as running, and nothing is signalled. Reporting it as
    /// running instead is what would leave a withdrawn holder's claim area waiting for a process
    /// that is nobody's.
    #[test]
    fn a_live_process_whose_identity_disproves_the_claim_is_a_session_that_ended() {
        let tmp = tempfile::tempdir().unwrap();
        let worker = sidecar_at(tmp.path());
        let mut bystander = a_live_process();
        let pid = bystander.0.id();
        std::fs::write(
            pid_file(tmp.path(), "s-recycled"),
            format!("{pid}\n2020-01-01T00:00:00Z\n"),
        )
        .unwrap();

        assert!(
            !worker.session_is_running("s-recycled"),
            "the liveness read says the same thing: this claim names a session that is over"
        );
        let outcome = worker
            .stop_session("s-recycled")
            .expect("a session that is over is the state the stop was for, not a failure");
        match outcome {
            SessionStop::NothingToStop(note) => assert!(
                note.contains("s-recycled") && note.contains("different time"),
                "the note says what it found: {note}"
            ),
            other => panic!("a stranger's pid must never be signalled: {other:?}"),
        }
        std::thread::sleep(Duration::from_millis(250));
        assert!(
            bystander.0.try_wait().unwrap().is_none(),
            "the unrelated live process holding the recycled pid was signalled"
        );
    }
}

#[test]
fn stopping_a_session_without_a_pid_file_is_refused_by_name() {
    let tmp = tempfile::tempdir().unwrap();
    let worker = sidecar_at(tmp.path());

    let refused = worker
        .stop_session("never-started")
        .expect_err("no pid file is a refusal — there is nothing to signal");
    assert!(
        refused.contains("never-started") && refused.contains("pid file"),
        "the refusal names the session and the file it looked for: {refused}"
    );
}

// **The two cases that used to be pinned here are pinned in `worker.rs`'s own unit tests now**
// (fix round 3 of this item's review, Test Quality #4).
//
// They are the group-signal guard: a claim holding `0`, which `kill` reads as the CALLER's whole
// process group, and one holding `u32::MAX`, which wraps negative in `pid_t` (`-1` is every
// process this user may signal). Both were driven through the real `stop_session` from here — so
// if the guard ever regressed, the pin itself would run `kill(0, SIGTERM)` against the test
// runner, which is not hypothetical: a pid file holding `0` took down the run that first exercised
// this stop.
//
// `SidecarWorker::stop_session_by` takes the signal as a parameter, and the unit tests hand it a
// recording one. That pins strictly more than this file could — the refusal by name AND that
// nothing was sent — with no signal able to leave the test. What stays here is everything that
// can be driven safely: a live child that must be stopped, and live children that must NOT be.
//
// The READ side of the same guard is safe from here and stays — `session_is_running` and
// [`live_sessions_in`] never signal anything.

/// **And the READS answer the same way, because they share the one parse** (fix round 2 of this
/// item's review, Important #2). `stop_session` refused `0` by name while `session_is_running` and
/// [`live_sessions_in`] parsed a bare `u32` and handed it to `kill(pid, 0)` — which for `0` is a
/// question about the CALLER's whole process group and answers `true` for any live process at all.
///
/// Before this branch that only stalled a liveness gate. Now a pid file holding `0` in a withdrawn
/// holder's claim area would mean the park never happens: the tick's withdrawn occasion waits for
/// nothing to be running, the marker never clears, and every release path declines — a working copy
/// wedged until somebody deletes the file by hand. One parse, one answer, at all three sites.
#[test]
fn a_pid_file_holding_zero_is_not_a_running_session() {
    let tmp = tempfile::tempdir().unwrap();
    let worker = sidecar_at(tmp.path());
    std::fs::write(pid_file(tmp.path(), "s-0"), "0").unwrap();

    assert!(
        !worker.session_is_running("s-0"),
        "0 names no process — `kill(0, 0)` asks about this caller's own process group, and a \
         `true` there would wedge the claim area of a withdrawn holder for good"
    );
    assert!(
        nexus_chat::worker::live_sessions_in(tmp.path()).is_empty(),
        "and the whole-workspace read agrees: {:?}",
        nexus_chat::worker::live_sessions_in(tmp.path())
    );
}

/// **The wrapper every command-line call passes through forwards both** — the `6j6v.x5sr` class,
/// read at the source. `cli.rs`'s `LazyWorker` is private, and — since `withdraw` learned to stop a
/// running round (nxf 6j6v.b9nf) — `nxc withdraw` now DOES reach both through the binary:
/// `withdraw_a_running_round.rs`'s `nxc_withdraw_*` tests drive it against a real pid file and a
/// real process, so there is a receipt to read the answer off after all. This test pins the same
/// forward more cheaply and closer to the cause: no process to spawn, no signal to wait out, and a
/// failure here names the missing forward directly instead of showing up as a withdrawal that
/// silently refused a runtime that can stop its own sessions. `every_worker_answers_for_itself.rs`
/// checks that the methods are implemented at all; this checks that each one reaches the selected
/// worker, which is the half a wrapper that implements the method with the default's body would
/// still get wrong.
#[test]
fn the_lazy_cli_worker_forwards_stop() {
    let cli = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/cli.rs"))
        .unwrap();
    let marker = "impl crate::worker::Worker for LazyWorker";
    let start = cli.find(marker).expect("the delegator is where it was");
    let body = &cli[start..];
    let body = &body[..body.find("\n}\n").expect("the impl block ends")];

    for method in ["fn stops_sessions", "fn stop_session"] {
        let at = body
            .find(method)
            .unwrap_or_else(|| panic!("`LazyWorker` does not implement `{method}`"));
        let rest = &body[at..];
        let method_body = &rest[..rest.find("\n    }\n").expect("the method ends")];
        assert!(
            method_body.contains("select_worker"),
            "`LazyWorker::{method}` answers for itself instead of asking the worker it resolves:\n\
             {method_body}"
        );
    }
}
