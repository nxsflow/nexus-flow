//! **One internal session, one process** — nxf 6j6v.7qtf, driven through the REAL `SidecarWorker`.
//!
//! The finding, measured live on nxs 0.62.0 in the `nxc` proving ground (4jgn.g90w): a wake that
//! arrived while the session it named was still working was answered with a SECOND `node` process on
//! the same spec file, in the same working directory, under the same internal session id. It did not
//! stop at two — three sidecars on one spec, four twenty seconds later, ~20 `claude` processes on
//! the machine — and a coder watched its own edits disappear under a sibling it could not see. The
//! working-tree lease cannot catch it: it separates CHAINS, and two processes of one session are the
//! same chain.
//!
//! `worker.rs`'s own unit tests hold the DECISION (`SessionLock` against a real live pid, in both of
//! its answers). What is held HERE is the seam: that `SidecarWorker::trigger` asks before it spawns,
//! that the refusal is the item's own named error rather than a generic failure, and — the second
//! half of the DoD — that a refused trigger never reaches the spec write, so the truth the running
//! process is still reading is not changed under it.
//!
//! The stub sidecar is a real `node` script that stays alive, so the pid the guard probes is a
//! genuine live process. It is deliberately NOT named like the shipped bundle: `SidecarWorker`
//! demands a resolvable `claude` only for the bundled sidecar, and this test is about the spawn
//! guard, not about the SDK.

use nexus_chat::worker::{
    Coordinator, RoleSpec, TriggerError, TriggerRequest, TriggerResult, TurnTerms, Worker,
    WorkerConfig,
};

/// A workspace directory plus a `node` script that simply stays alive for a while — the stand-in
/// for a session that is still working when the next wake arrives.
struct Stub {
    dir: tempfile::TempDir,
    script: std::path::PathBuf,
}

fn stub() -> Stub {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("stub-sidecar.mjs");
    // Long enough that no assertion below can race it, short enough that a test run which somehow
    // leaks one is not leaving a process behind for a human to find.
    std::fs::write(&script, "setTimeout(() => {}, 30_000);\n").unwrap();
    Stub {
        dir,
        script: script.clone(),
    }
}

fn request(session: &str) -> TriggerRequest {
    TriggerRequest {
        internal_session: session.to_string(),
        resume_real: None,
        message: "carry on".to_string(),
        role: RoleSpec {
            handle: "coder".to_string(),
            system_prompt: "You are the coder.".to_string(),
            use_claude_code_preset: false,
            tools: None,
            granted_tools: Vec::new(),
            permissions: None,
            model: None,
            declaration_hash: None,
        },
        env: Vec::new(),
        reply_thread: None,
        coordinator: Coordinator::Persona,
        terms: TurnTerms::default(),
    }
}

/// Every `node` this test started, killed and reaped — a leaked 30-second sleeper per test run is
/// exactly the kind of debris this whole item is about.
fn reap(dir: &std::path::Path) {
    let out = std::process::Command::new("pgrep")
        .args(["-f", &format!("{}", dir.join("stub-sidecar.mjs").display())])
        .output();
    if let Ok(out) = out {
        for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
            let _ = std::process::Command::new("kill")
                .args(["-9", pid])
                .status();
        }
    }
}

#[test]
fn a_wake_on_a_session_that_is_already_running_is_refused_instead_of_spawning_a_second_process() {
    let s = stub();
    let worker = WorkerConfig::Sidecar {
        sidecar: s.script.clone(),
        cwd: s.dir.path().to_path_buf(),
    }
    .build()
    .expect("the sidecar worker builds");

    worker
        .trigger(request("m-01SESSION"))
        .expect("the first trigger starts the session");

    let err = worker
        .trigger(request("m-01SESSION"))
        .expect_err("the second must not start a second process for the same session");
    match err {
        TriggerError::AlreadyRunning { session, pid } => {
            assert_eq!(session, "m-01SESSION");
            assert!(
                pid > 0,
                "the refusal names the live process, so a human can look at it"
            );
        }
        other => panic!("expected the named refusal, got: {other}"),
    }
    reap(s.dir.path());
}

#[test]
fn a_refused_trigger_does_not_rewrite_the_spec_the_running_process_is_reading() {
    // The second half of the DoD, and the half that did the silent damage: there is exactly ONE spec
    // file per session and every trigger used to rewrite it. In the proving ground the coder read
    // `"resume": null` out of its own spec and, minutes later, the same file said
    // `"resume": "ea9ab5e9-…"` — the second spawn had changed the first one's truth under it.
    let s = stub();
    let worker = WorkerConfig::Sidecar {
        sidecar: s.script.clone(),
        cwd: s.dir.path().to_path_buf(),
    }
    .build()
    .expect("the sidecar worker builds");

    worker.trigger(request("m-01SESSION")).unwrap();
    let spec_path = s.dir.path().join(".nxs/agent-logs/m-01SESSION.spec.json");
    let first = std::fs::read_to_string(&spec_path).expect("the first trigger wrote a spec");

    let mut second = request("m-01SESSION");
    second.message = "a completely different task".to_string();
    second.resume_real = Some("ea9ab5e9-f0e4-4d2d-9113-e3fbf99c2ba0".to_string());
    worker.trigger(second).expect_err("refused");

    assert_eq!(
        std::fs::read_to_string(&spec_path).unwrap(),
        first,
        "a refused trigger writes nothing: the running process's spec is untouched"
    );
    reap(s.dir.path());
}

#[test]
fn a_different_session_is_never_blocked_by_a_running_one() {
    // The guard is per SESSION, not a global one: a fresh `send --to <persona>` mints a new internal
    // session and must start immediately, whatever else is running.
    let s = stub();
    let worker = WorkerConfig::Sidecar {
        sidecar: s.script.clone(),
        cwd: s.dir.path().to_path_buf(),
    }
    .build()
    .expect("the sidecar worker builds");

    worker.trigger(request("m-01FIRST")).expect("first session");
    worker
        .trigger(request("m-01SECOND"))
        .expect("a second, different session is a second, different process");
    reap(s.dir.path());
}

// ---- what the CALLER sees when the guard fires (nxf 6j6v.7qtf, DoD point 2) --------------------
//
// The guard's whole value is that the refusal is VISIBLE: a wake that was answered by silence is
// the failure class this repo builds gates against. `nxc guide limits-and-safety` tells an app that
// `warnings` is the array to watch, so the refusal has to be IN it — and the live run of this item
// found it was not: the surface's thread-return-address resume recorded `wake_skipped` and left
// `warnings` empty, which is the one place `orchestration::reply`'s own resume site has always
// filled both of.
//
// Driven with a worker that refuses every trigger the way the guard does, so what is under test is
// the REPORTING and not the guard's own decision (that is held above, against a real live pid).

/// A worker that answers every trigger the way [`SessionLock`] does when the session is running.
#[derive(Debug)]
struct AlwaysRunning;

impl Worker for AlwaysRunning {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        Err(TriggerError::AlreadyRunning {
            session: req.internal_session,
            pid: 4242,
        })
    }
}

/// **What a refused wake becomes since nxf 6j6v.gn8b: a HELD delivery, not a reported failure.**
///
/// This test asserted the opposite until then, and the flip is the whole point of that item. The
/// refusal below is unchanged — the guard still says "this session has a process" and still refuses
/// — but the answer is no longer dropped on the floor with a finding beside it. It goes into the
/// coordinator's durable queue and reaches the caller when its session settles, which is why
/// `wake_skipped` and `warnings` are now EMPTY here: nothing failed.
///
/// What it used to assert, kept so nobody restores it by accident: `wake_skipped.reason ==
/// AlreadyRunning` plus a `RequesterNotWoken` finding in `warnings`. That was the right shape while
/// the answer really was lost — an app watching `warnings` was the only one who could notice — and
/// it is the wrong shape now, because reporting a delivery that WILL happen as a consequence that
/// did NOT is how a caller learns to ignore the field that matters.
#[test]
fn a_refused_wake_holds_the_answer_instead_of_reporting_a_failure() {
    use nexus_chat::engine::{Engine, EngineConfig};
    use nexus_chat::orchestration::Caller;
    use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
    use nexus_chat::timer::TimerConfig;
    use std::sync::Arc;

    use nexus_chat::workspace::{chat_config, setup};

    const NOW: &str = "2026-08-22T10:00:00Z";
    let tmp = tempfile::tempdir().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    std::fs::create_dir_all(tmp.path().join(".nxs-personas")).unwrap();
    std::fs::write(
        tmp.path().join(".nxs-personas/coder.yaml"),
        "handle: coder\nsystem_prompt: You are the coder.\n",
    )
    .unwrap();

    // The FIRST trigger has to succeed, or there is no session to wake later. `Dry` starts it; the
    // refusing worker is installed for the wake.
    let started = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    let opened = started
        .send_to(
            Caller {
                session: None,
                actor: Some("ckoch"),
                now: Some(NOW),
            },
            SendToRequest {
                machine: None,
                to: "coder",
                body: "do the thing",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .expect("the coder is summoned");
    let thread = opened.thread_id.clone();
    let session = opened.session.clone().expect("a session was minted");

    // The coder answers from inside that session, which is what leaves a RETURN ADDRESS in the
    // thread — the address the human's next reply resumes through.
    started
        .reply_thread(
            Caller {
                session: Some(&session),
                actor: Some("coder"),
                now: Some(NOW),
            },
            ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "a partial answer",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the coder posts");
    drop(started);

    let refusing = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Custom(Arc::new(AlwaysRunning)),
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");
    let receipt = refusing
        .reply_thread(
            Caller {
                session: None,
                actor: Some("ckoch"),
                now: Some(NOW),
            },
            ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "and one more thing",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("the reply is posted whatever the wake does");

    assert!(
        receipt.posted,
        "the message is durable either way: {receipt:?}"
    );
    assert!(receipt.woke.is_none(), "nothing was woken: {receipt:?}");

    // THE assertion of this test now: the answer is HELD for the session that is working, and the
    // receipt says so in a field of its own.
    assert_eq!(
        receipt.held.as_deref(),
        Some(session.as_str()),
        "the refusal became a held delivery, named on the receipt: {receipt:?}"
    );
    assert!(
        receipt.wake_skipped.is_none(),
        "a delivery that WILL happen is not a wake that did not: {receipt:?}"
    );
    assert!(
        receipt.warnings.is_empty(),
        "…and nothing failed, so nothing is warned about: {receipt:?}"
    );

    // And it is really in the queue, readable back through the seam that filled it — not merely
    // announced on a receipt.
    let waiting = refusing
        .held_deliveries(&session)
        .expect("the queue is readable");
    assert_eq!(waiting.len(), 1, "{waiting:?}");
    assert_eq!(waiting[0].held.thread_id, thread);
    assert_eq!(waiting[0].held.body, "and one more thing");
    assert_eq!(
        waiting[0].held.sender,
        format!("{}/ckoch", refusing.origin())
    );
}
