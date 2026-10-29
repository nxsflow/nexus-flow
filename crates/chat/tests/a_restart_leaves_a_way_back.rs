//! **A planned service restart leaves every running session a way back, in place** (nxf 6j6v.wxz0,
//! the field case 6j6v.8n9a).
//!
//! `nxs self-update` and `nxs sync daemon install` restart the background service with `bootout` +
//! `bootstrap`, and launchd's SIGTERM reaches every session the service started, because they share
//! its process group. Before this item that killed them mid-turn with nothing on record: the thread
//! said "nothing is coming", `nxc resume` found nothing on hold, and the uncommitted work sat in the
//! tree with no way back but a person.
//!
//! The restart now holds each session of the instance under the `service_restart` limit, stops it
//! with a control signal, waits a grace, and the new service resumes it IN PLACE at `until`: the
//! same session, the same checkout, the same transcript. These tests drive that across REAL
//! processes where the claim is about a process — every `nxc` is a real child against one
//! workspace, a session is a real child holding its claim the way the sidecar writes it, and
//! `$HOME` is pinned by `nxs_test_support::cargo_bin`. Nothing sleeps for a state: each step reads
//! a named signal (a table row, a log line, a file).
//!
//! **This repository's own board is never touched**: every `nxc` here strips the `NXC_*` variables
//! it would otherwise inherit from the agent running the suite.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use nexus_chat::interruption::{
    SERVICE_RESTART_GRACE_SECS, SERVICE_RESTART_LIMIT, SERVICE_RESTART_MARGIN_SECS,
    SERVICE_RESTART_POLL_SECS,
};
use nexus_chat::restart::{secure_the_instance, SecureTheInstance};
use nexus_chat::timer::TimerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use nxs_service::restart::{PreparedSession, RestartPreparation};
use serde_json::Value;
use tempfile::TempDir;

const T0: &str = "2026-10-27T10:00:00Z";
const DETAIL: &str = "nxs self-update 0.221.0 -> 0.222.0 restarted the background service";

fn at(secs: i64) -> String {
    use time::format_description::well_known::Rfc3339;
    let t = time::OffsetDateTime::parse(T0, &Rfc3339).unwrap() + time::Duration::seconds(secs);
    t.format(&Rfc3339).unwrap()
}

fn parse(instant: &str) -> time::OffsetDateTime {
    time::OffsetDateTime::parse(instant, &time::format_description::well_known::Rfc3339).unwrap()
}

/// A workspace: a coder that needs the copy alone, a rival that does too, two personas that need
/// none, a coding channel whose step has a two-minute window, and a stub sidecar for the ticks that
/// run under the sidecar worker (it writes the spec it was handed and exits).
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for (handle, tree) in [
        ("coder", "working_tree: exclusive\n"),
        ("rival", "working_tree: exclusive\n"),
        ("alpha", ""),
        ("beta", ""),
    ] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!(
                "handle: {handle}\njob_title: {handle}\nsystem_prompt: You are {handle}.\n{tree}"
            ),
        )
        .unwrap();
    }
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: guarded\n  members: [coder]\n  flow: sequential\n  timeout: 2m\n  \
         working_tree: exclusive\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("stub-sidecar.mjs"),
        format!(
            "import {{ appendFileSync, readFileSync }} from \"node:fs\";\n\
             const spec = JSON.parse(readFileSync(process.argv[process.argv.indexOf(\"--spec\") + 1], \"utf8\"));\n\
             appendFileSync({log:?}, JSON.stringify(spec) + \"\\n\");\n",
            log = tmp.path().join("stub.log").display().to_string()
        ),
    )
    .unwrap();
    tmp
}

/// The `nxc` environment every process here gets: nothing inherited from the agent running this.
fn nxc_env(root: &Path, now: &str, worker: &str) -> Vec<(String, String)> {
    let mut env = vec![
        ("NXC_ACTOR", "carsten".to_string()),
        ("NXC_ORIGIN", "local".to_string()),
        ("NXC_NOW", now.to_string()),
        ("NXC_WORKER", worker.to_string()),
        ("NXC_DRY_LOG", root.join("dry.log").display().to_string()),
        ("NXC_TIMER", "dry".to_string()),
        (
            "NXC_TIMER_LOG",
            root.join("timer.log").display().to_string(),
        ),
        ("NXC_NAMER", "dry".to_string()),
        (
            "NXC_SIDECAR",
            root.join("stub-sidecar.mjs").display().to_string(),
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect::<Vec<_>>();
    env.extend(
        nxs_test_support::pinned_home_env()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v)),
    );
    env
}

/// One `nxc` process at `now`, under `worker`.
fn nxc_as(root: &Path, now: &str, worker: &str) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    for (var, _) in std::env::vars() {
        if var.starts_with("NXC_") {
            c.env_remove(var);
        }
    }
    c.current_dir(root);
    for (k, v) in nxc_env(root, now, worker) {
        c.env(k, v);
    }
    c
}

/// One `nxc` process at `now`, under the recording worker.
fn nxc(root: &Path, now: &str) -> Command {
    nxc_as(root, now, "dry")
}

fn json_of(out: &[u8]) -> Value {
    serde_json::from_str(String::from_utf8_lossy(out).trim()).unwrap_or_else(|e| {
        panic!(
            "not a --json receipt ({e}): {}",
            String::from_utf8_lossy(out)
        )
    })
}

fn store(root: &Path) -> nexus_chat::store::ChatStore {
    Workspace::resolve(None, root)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = StdCommand::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn a_repository(root: &Path) {
    git(root, &["init", "-q", "-b", "main"]);
    std::fs::write(
        root.join(".gitignore"),
        ".nxs/\n*.log\n*.mjs\ntorn-down-*\nsignalled-*\nflush-*\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "base"]);
}

/// Commission `to` at `now` and return (its root thread, its newest session).
fn commission(root: &Path, now: &str, to: &str) -> (String, String) {
    let out = nxc(root, now)
        .args(["send", "--to", to, "--no-ref", "--json", "do the work"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let thread = json_of(&out.stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let session: String = store(root)
        .connection()
        .query_row(
            "SELECT internal_id FROM session_map WHERE role LIKE ?1 ORDER BY rowid DESC LIMIT 1",
            [format!("%{to}")],
            |r| r.get(0),
        )
        .expect("a session was minted");
    (thread, session)
}

/// The owner's channel commission of `guarded`: (root thread, the coder's slot, its session).
fn commission_the_channel(root: &Path, now: &str) -> (String, String, String) {
    let out = nxc(root, now)
        .args(["send", "--to", "guarded", "--no-ref", "--json", "build it"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let thread = json_of(&out.stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (slot, session): (String, String) = store(root)
        .connection()
        .query_row(
            "SELECT thread, session FROM owed_starts WHERE role LIKE '%coder'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the coder's obligation is recorded");
    (thread, slot, session)
}

/// The session's own callbacks before the restart: it bound and wrote one transcript entry.
fn at_work(root: &Path, now: &str, session: &str) {
    nxc(root, now)
        .env("NXC_SESSION", session)
        .args(["session", "bind", session, &format!("real-{session}")])
        .assert()
        .success();
    nxc(root, now)
        .env("NXC_SESSION", session)
        .args(["transcript", "append", "--session", session])
        .write_stdin(r#"{"kind":"assistant","data":{"text":"building"}}"#)
        .assert()
        .success();
}

/// The coder's own work in the tree: a branch, one commit, one uncommitted file.
fn the_coders_work(root: &Path) {
    git(root, &["switch", "-q", "-c", "fix/the-coders-branch"]);
    std::fs::write(root.join("done.rs"), "fn done() {}\n").unwrap();
    git(root, &["add", "done.rs"]);
    git(root, &["commit", "-q", "-m", "the coder's commit"]);
    std::fs::write(root.join("half-done.rs"), "fn half() {}\n").unwrap();
}

fn the_coders_work_is_in_place(root: &Path) {
    assert_eq!(
        git(root, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "fix/the-coders-branch"
    );
    assert_eq!(
        git(root, &["log", "-1", "--format=%s"]),
        "the coder's commit"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("half-done.rs")).unwrap(),
        "fn half() {}\n"
    );
    assert_eq!(
        git(root, &["branch", "--list", "nxs/park/*"]),
        "",
        "no park branch"
    );
}

/// A process standing in for a session, with its claim written the way the sidecar writes it.
/// It runs until it is killed; `on_term` is the shell its SIGTERM trap runs (`""` ignores it, and
/// `None` keeps the default — the process dies at once).
///
/// **Reaped by a thread of its own, the way a parent reaps** (CI on Linux, round 4): this test
/// process is the session's parent, and a child that exited at its stop and was never waited for is
/// a zombie — which still answers `kill(pid, 0)` and, on Linux, still shows its start time in
/// `/proc`, so it would read as a live session for the whole grace. The service that really starts
/// sessions is a different process from the restart's hook.
struct Session {
    pid: u32,
    exited: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Session {
    fn start(root: &Path, session: &str, on_term: Option<&str>, own_group: bool) -> Session {
        use std::os::unix::process::CommandExt;
        let script = match on_term {
            Some(trap) => format!("trap '{trap}' TERM; while :; do sleep 0.1; done"),
            None => "exec sleep 600".to_string(),
        };
        let mut cmd = StdCommand::new("sh");
        cmd.arg("-c")
            .arg(script)
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        for (var, _) in std::env::vars() {
            if var.starts_with("NXC_") {
                cmd.env_remove(var);
            }
        }
        if own_group {
            cmd.process_group(0);
        }
        let mut child = cmd.spawn().expect("start the stand-in session");
        let pid = child.id();
        let logs = nexus_chat::worker::agent_logs_dir(root);
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(
            logs.join(format!("{session}.pid")),
            nexus_chat::worker::session_claim_for(pid),
        )
        .unwrap();
        let exited = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = exited.clone();
        std::thread::spawn(move || {
            let _ = child.wait();
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        Session { pid, exited }
    }

    /// A sidecar-shaped session: on SIGTERM it runs the sidecar's teardown — bind, a transcript
    /// flush, `session ended --cause` — through real `nxc` processes at `now`, then marks it done
    /// and exits.
    fn sidecar_shaped(root: &Path, session: &str, now: &str) -> Session {
        let nxc = assert_cmd::cargo::cargo_bin("nxc");
        let env: String = nxc_env(root, now, "dry")
            .into_iter()
            .map(|(k, v)| format!("{k}={} ", shell_quote(&v)))
            .collect();
        let nxc = shell_quote(&nxc.display().to_string());
        let marker = root.join(format!("torn-down-{session}"));
        let flush = root.join(format!("flush-{session}.json"));
        std::fs::write(&flush, r#"{"kind":"assistant","data":{"text":"stopped"}}"#).unwrap();
        let trap = format!(
            "env {env}NXC_SESSION={session} {nxc} session bind {session} real-{session}; \
             env {env}NXC_SESSION={session} {nxc} transcript append --session {session} \
             < {flush}; \
             env {env}NXC_SESSION={session} {nxc} session ended {session} --cause \"told to stop (SIGTERM)\"; \
             touch {marker}; exit 0",
            marker = marker.display(),
            flush = flush.display()
        );
        // The trap is single-quoted by `start`, so it must carry no single quote of its own.
        assert!(!trap.contains('\''), "{trap}");
        Session::start(root, session, Some(&trap), false)
    }

    fn pid(&self) -> u32 {
        self.pid
    }

    fn is_alive(&mut self) -> bool {
        !self.exited.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether the process exits within `bound` — its reaper's word, not a single look.
    fn has_exited_within(&mut self, bound: Duration) -> bool {
        let deadline = Instant::now() + bound;
        while self.is_alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        !self.is_alive()
    }

    /// SIGKILL, and wait for the reaper to see it go.
    fn kill_hard(&mut self) {
        if self.is_alive() {
            // SAFETY: a signal to the child this test started; it touches no memory here.
            unsafe { libc::kill(self.pid as libc::pid_t, libc::SIGKILL) };
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.is_alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill_hard();
    }
}

fn shell_quote(s: &str) -> String {
    assert!(!s.contains('"') && !s.contains('\''), "{s}");
    format!("\"{s}\"")
}

/// `NXC_TIMER_LOG` is process-global env the in-process hook's dry timer reads: every test that
/// runs the hook serializes on it.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// This test process's own process group — the "service" whose sessions the hook is asked about.
fn own_group() -> i32 {
    // SAFETY: `getpgid(0)` reads this process's own group and touches no memory.
    unsafe { libc::getpgid(0) }
}

/// Run the restart's hook over `root` with `grace_secs`, the real sleep and poll interval, and a
/// clock that starts at `base` and runs at wall speed.
fn secure(root: &Path, base: &str, grace_secs: i64, on_poll: &dyn Fn(u32)) -> RestartPreparation {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("NXC_TIMER_LOG", root.join("timer.log"));
    let started = Instant::now();
    let base = parse(base);
    let clock = move || base + started.elapsed();
    let workspaces = [root.to_path_buf()];
    let report = secure_the_instance(SecureTheInstance {
        workspaces: &workspaces,
        pgid: own_group(),
        detail: DETAIL,
        grace_secs,
        margin_secs: SERVICE_RESTART_MARGIN_SECS,
        poll: Duration::from_millis(100),
        timer: TimerConfig::Dry,
        clock: &clock,
        sleep: &std::thread::sleep,
        on_poll,
    });
    std::env::remove_var("NXC_TIMER_LOG");
    report
}

fn prepared(root: &Path, session: &str) -> PreparedSession {
    PreparedSession {
        workspace: root.display().to_string(),
        session: session.to_string(),
    }
}

fn lease_row(root: &Path) -> Option<(String, String)> {
    store(root).working_tree_lease_row().unwrap()
}

fn resume(root: &Path, now: &str, session: &str) -> Value {
    let out = nxc(root, now)
        .args(["resume", "--session", session, "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    json_of(&out.stdout)
}

fn tick(root: &Path, now: &str, thread: &str) -> Value {
    let out = nxc_as(root, now, "sidecar")
        .args(["tick", "--thread", thread, "--json"])
        .output()
        .unwrap();
    json_of(&out.stdout)
}

/// Every spec the stub sidecar was handed — a tick that started something.
fn stub_starts(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("stub.log"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).expect("a spec"))
        .collect()
}

/// [`stub_starts`] once `n` of them are there — the stub is started detached, so its line lands
/// after the tick returns — bounded at ten seconds.
fn stub_starts_n(root: &Path, n: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let got = stub_starts(root);
        if got.len() >= n || Instant::now() > deadline {
            return got;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The thread entry of `thread` in a `status --all --json` report.
fn status_thread(root: &Path, now: &str, thread: &str) -> Value {
    let out = nxc(root, now)
        .args(["status", "--all", "--json"])
        .output()
        .unwrap();
    let report = json_of(&out.stdout);
    report["operations"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|op| op["threads"].as_array().cloned().unwrap_or_default())
        .find(|t| t["thread_id"] == thread)
        .unwrap_or_else(|| panic!("thread {thread} is not in {report}"))
}

/// Wait for `path` to appear — a teardown's last act — bounded at thirty seconds.
fn appears(path: &Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// **A restart hold cannot be forged from the command line** (round-2 IR #4). A hold under
/// `service_restart` pins the working copy until `until` plus the margin, so one written by hand
/// with a far-future `until` would pin it for good. The restart's hook records through the Engine
/// seam; the runtime's callback refuses the name, and records nothing.
#[test]
fn the_runtime_callback_refuses_the_service_restart_limit() {
    let tmp = workspace();
    let root = tmp.path();
    let out = nxc(root, T0)
        .args(["send", "--to", "coder", "--no-ref", "--json", "build it"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let session: String = nexus_chat::workspace::Workspace::resolve(None, root)
        .and_then(|ws| {
            use nexus_chat::workspace::ChatWorkspaceExt;
            ws.open_chat_store()
        })
        .unwrap()
        .connection()
        .query_row(
            "SELECT internal_id FROM session_map ORDER BY rowid DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let refused = nxc(root, T0)
        .env("NXC_SESSION", &session)
        .args([
            "session",
            "interrupted",
            &session,
            "--limit",
            "service_restart",
            "--until",
            "2099-01-01T00:00:00Z",
        ])
        .output()
        .unwrap();
    assert!(!refused.status.success(), "the forged hold was accepted");
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(
        said.contains("service_restart"),
        "the refusal names the limit: {said}"
    );
    let resume = nxc(root, T0)
        .args(["resume", "--session", &session, "--json"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&resume.stdout).contains("nothing_on_hold"),
        "nothing was recorded: {}",
        String::from_utf8_lossy(&resume.stdout)
    );
}

// ---- DoD 1 and 3: the whole restart, across real processes -------------------------------------

/// **DoD 1** — the field case 8n9a, the other way round. A sidecar-shaped session binds, writes,
/// commits on its own branch and leaves an uncommitted file; it holds the working copy and a rival
/// waits behind it. The restart's hook holds and stops it, its teardown runs inside the grace, and
/// afterwards the board says it is coming back, nothing parks or hands on through the gap, and the
/// take-up at `until` continues the same session in the same tree without asking about drift.
/// **DoD 3** across processes: the early resume and the gap tick secure nothing.
#[test]
fn a_restart_holds_the_session_and_it_comes_back_in_its_own_tree() {
    let tmp = workspace();
    let root = tmp.path();
    a_repository(root);
    let (thread, session) = commission(root, T0, "coder");
    let (holder, _) = lease_row(root).expect("the premise: the coder holds the copy");
    assert!(
        holder.ends_with(&thread),
        "{holder} names the coder's operation"
    );
    commission(root, &at(5), "rival");
    at_work(root, &at(10), &session);
    the_coders_work(root);
    let mut running = Session::sidecar_shaped(root, &session, &at(61));
    let lease = lease_row(root);

    let report = secure(root, &at(60), 20, &|_| {});

    assert_eq!(report.held, [prepared(root, &session)], "{report:?}");
    assert!(
        report.stopped.is_empty() && report.unrecorded.is_empty(),
        "{report:?}"
    );
    assert!(
        appears(&root.join(format!("torn-down-{session}"))),
        "the teardown ran inside the grace"
    );
    assert!(
        running.has_exited_within(Duration::from_secs(10)),
        "it exited after its teardown"
    );
    let until = report.until.clone().expect("the receipt names until");
    assert_eq!(until, at(60 + 20 + SERVICE_RESTART_MARGIN_SECS));
    // The teardown's three acts landed: bound, flushed, ended.
    assert_eq!(
        store(root).resolve_real(&session).unwrap().as_deref(),
        Some(format!("real-{session}").as_str())
    );
    assert!(store(root).session_ended_at(&session).unwrap().is_some());
    let entries: i64 = store(root)
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM agent_transcript WHERE internal_session = ?1",
            [&session],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        entries, 2,
        "the work before the stop and the teardown's flush"
    );
    // The board: interrupted under the restart's limit — not "nothing is coming".
    let row = status_thread(root, &at(65), &thread);
    assert_eq!(row["interrupted"]["limit"], SERVICE_RESTART_LIMIT, "{row}");

    // DoD 3: an early resume answers not-yet and secures nothing.
    let early = resume(root, &at(70), &session);
    assert_eq!(early["outcome"], "not_yet", "{early}");
    the_coders_work_is_in_place(root);
    assert_eq!(
        lease_row(root),
        lease,
        "the lease row is unchanged by the early resume"
    );

    // DoD 3 / DoD 1: a tick in the gap — the rival is queued behind the lease — parks nothing,
    // hands nothing on, and starts no second session on the slot.
    tick(root, &at(90), &thread);
    the_coders_work_is_in_place(root);
    assert_eq!(
        lease_row(root),
        lease,
        "the lease row is unchanged by the gap tick"
    );
    assert!(
        stub_starts(root).is_empty(),
        "nothing started: {:?}",
        stub_starts(root)
    );
    let status = json_of(
        &nxc(root, &at(90))
            .args(["status", "--all", "--json"])
            .output()
            .unwrap()
            .stdout,
    );
    assert!(
        !status.to_string().contains("park_refused"),
        "no PARK REFUSED: {status}"
    );

    // The take-up at until — as the NEW service runs it, under the sidecar worker: the same
    // session, continued with its own runtime conversation, and no drift question although the tree
    // differs from the commission's anchor.
    let out = nxc_as(root, &until, "sidecar")
        .args(["resume", "--session", &session, "--json"])
        .output()
        .unwrap();
    let taken = json_of(&out.stdout);
    assert_eq!(taken["outcome"], "resumed", "{taken}");
    let specs = stub_starts_n(root, 1);
    assert_eq!(specs.len(), 1, "{specs:?}");
    assert_eq!(specs[0]["session"], session.as_str());
    assert_eq!(
        specs[0]["resume"],
        format!("real-{session}").as_str(),
        "{}",
        specs[0]
    );
    the_coders_work_is_in_place(root);
}

// ---- DoD 2, 10, 11: the grace, the kill, newcomers and the bound on until ------------------------

/// **DoD 2** — one session goes at its stop, one ignores it. The first is `held`; the second is a
/// straggler still alive at the grace and left to the bootout (the hook sends no kill of its own).
#[test]
fn a_session_gone_in_the_grace_is_held_and_one_still_alive_is_left_to_the_bootout() {
    let tmp = workspace();
    let root = tmp.path();
    let (_, quick) = commission(root, T0, "alpha");
    let (_, stubborn) = commission(root, T0, "beta");
    let _q = Session::start(root, &quick, None, false);
    let mut s = Session::start(root, &stubborn, Some(""), false);

    let report = secure(root, &at(60), 2, &|_| {});

    assert_eq!(report.held, [prepared(root, &quick)], "{report:?}");
    assert_eq!(report.stopped, [prepared(root, &stubborn)], "{report:?}");
    assert!(s.is_alive(), "the hook does not kill: the bootout does");
    for session in [&quick, &stubborn] {
        let hold = store(root).current_interruption(session).unwrap();
        assert_eq!(
            hold.map(|h| h.limit).as_deref(),
            Some(SERVICE_RESTART_LIMIT),
            "{session} is on the restart's hold"
        );
    }
}

/// **DoD 10** — a session of the instance that starts after the first look (a resume the old
/// service started during the grace) is held and stopped at a later look.
#[test]
fn a_session_that_starts_during_the_grace_is_held_at_a_later_look() {
    let tmp = workspace();
    let root = tmp.path();
    let (_, first) = commission(root, T0, "alpha");
    let (_, newcomer) = commission(root, T0, "beta");
    let _a = Session::start(root, &first, Some(""), false);
    let started: Mutex<Option<Session>> = Mutex::new(None);
    let root_buf: PathBuf = root.to_path_buf();
    let on_poll = |n: u32| {
        if n == 2 {
            *started.lock().unwrap() = Some(Session::start(&root_buf, &newcomer, None, false));
        }
    };

    let report = secure(root, &at(60), 2, &on_poll);

    let seen: Vec<String> = report
        .held
        .iter()
        .chain(&report.stopped)
        .map(|p| p.session.clone())
        .collect();
    assert!(seen.contains(&newcomer), "{report:?}");
    assert_eq!(
        store(root)
            .current_interruption(&newcomer)
            .unwrap()
            .map(|h| h.limit)
            .as_deref(),
        Some(SERVICE_RESTART_LIMIT)
    );
}

/// **DoD 11** — `until` never falls while the hook runs. With an injected clock that advances one
/// poll per wait, and a straggler that keeps the hook busy for its whole grace: `until` is the
/// start plus the grace plus the margin, and it lies at least the margin less one poll after the
/// hook returned — at second resolution, the receipt's own.
#[test]
fn until_lies_beyond_the_hooks_return_by_the_margin_less_one_poll() {
    let tmp = workspace();
    let root = tmp.path();
    let (_, stubborn) = commission(root, T0, "beta");
    let _s = Session::start(root, &stubborn, Some(""), false);
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("NXC_TIMER_LOG", root.join("timer.log"));
    let offset = std::cell::Cell::new(time::Duration::ZERO);
    let base = parse(&at(60)) + time::Duration::milliseconds(700);
    let clock = || base + offset.get();
    let sleep = |d: Duration| offset.set(offset.get() + d);
    let workspaces = [root.to_path_buf()];

    let report = secure_the_instance(SecureTheInstance {
        workspaces: &workspaces,
        pgid: own_group(),
        detail: DETAIL,
        grace_secs: SERVICE_RESTART_GRACE_SECS,
        margin_secs: SERVICE_RESTART_MARGIN_SECS,
        poll: Duration::from_secs(SERVICE_RESTART_POLL_SECS as u64),
        timer: TimerConfig::Dry,
        clock: &clock,
        sleep: &sleep,
        on_poll: &|_| {},
    });
    std::env::remove_var("NXC_TIMER_LOG");

    let started = parse(report.started_at.as_deref().expect("started_at"));
    let returned = parse(report.returned_at.as_deref().expect("returned_at"));
    let until = parse(report.until.as_deref().expect("until"));
    assert_eq!(
        until,
        started
            + time::Duration::seconds(SERVICE_RESTART_GRACE_SECS)
            + time::Duration::seconds(SERVICE_RESTART_MARGIN_SECS)
    );
    assert!(
        until - returned
            >= time::Duration::seconds(SERVICE_RESTART_MARGIN_SECS - SERVICE_RESTART_POLL_SECS),
        "until {until} returned {returned}"
    );
    assert_eq!(report.stopped, [prepared(root, &stubborn)]);
}

// ---- DoD 6, 7, 8: nothing to do, a missed session, another group ---------------------------------

/// **DoD 6** — with no live session the hook records nothing, reports empty, and `status` is the
/// same before and after.
#[test]
fn with_no_live_session_the_hook_records_nothing_and_status_is_unchanged() {
    let tmp = workspace();
    let root = tmp.path();
    commission(root, T0, "alpha");
    let status = |now: &str| {
        json_of(
            &nxc(root, now)
                .args(["status", "--all", "--json"])
                .output()
                .unwrap()
                .stdout,
        )
    };
    let before = status(&at(100));

    let report = secure(root, &at(60), 2, &|_| {});

    assert!(report.is_empty(), "{report:?}");
    assert!(store(root).standing_interruptions().unwrap().is_empty());
    assert_eq!(status(&at(100)), before);
}

/// **DoD 7** — a session the hook never saw (its workspace not attended) is hard-killed by the
/// bootout before any teardown. It is left in exactly the state the stranded sweep reads: bound,
/// no end announced, no hold, and a claim that no live process holds.
#[test]
fn a_session_the_hook_missed_is_left_where_the_stranded_sweep_finds_it() {
    let tmp = workspace();
    let root = tmp.path();
    let (_, session) = commission(root, T0, "alpha");
    at_work(root, &at(10), &session);
    let mut missed = Session::start(root, &session, Some(""), false);
    // The hook ran over the workspaces the service attends; this one was not among them.
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let report = secure_the_instance(SecureTheInstance {
        workspaces: &[],
        pgid: own_group(),
        detail: DETAIL,
        grace_secs: 1,
        margin_secs: SERVICE_RESTART_MARGIN_SECS,
        poll: Duration::from_millis(100),
        timer: TimerConfig::Dry,
        clock: &time::OffsetDateTime::now_utc,
        sleep: &std::thread::sleep,
        on_poll: &|_| {},
    });
    assert!(report.is_empty());
    // The bootout's SIGKILL.
    missed.kill_hard();

    let s = store(root);
    assert!(s.resolve_real(&session).unwrap().is_some(), "bound");
    assert!(
        s.session_ended_at(&session).unwrap().is_none(),
        "no end announced"
    );
    assert!(
        s.current_interruption(&session).unwrap().is_none(),
        "no hold"
    );
    assert!(
        nexus_chat::worker::agent_logs_dir(root)
            .join(format!("{session}.pid"))
            .exists(),
        "its claim is left behind"
    );
    assert!(
        !nexus_chat::worker::live_sessions_in(root).contains(&session),
        "and no live process holds it"
    );
}

/// **DoD 8** — per instance, not per workspace: of two live sessions in one attended workspace, the
/// one in the service's process group is held and stopped; the one in another group is reported
/// `left_alone`, still running, un-held and un-signalled.
#[test]
fn a_session_in_another_process_group_is_left_alone() {
    let tmp = workspace();
    let root = tmp.path();
    let (_, ours) = commission(root, T0, "alpha");
    let (_, theirs) = commission(root, T0, "beta");
    let _o = Session::start(root, &ours, None, false);
    let signalled = root.join(format!("signalled-{theirs}"));
    let mut t = Session::start(
        root,
        &theirs,
        Some(&format!("touch {}", signalled.display())),
        true,
    );

    let report = secure(root, &at(60), 2, &|_| {});

    assert_eq!(report.held, [prepared(root, &ours)], "{report:?}");
    assert_eq!(report.left_alone, [prepared(root, &theirs)], "{report:?}");
    assert!(t.is_alive(), "still running");
    assert!(!signalled.exists(), "never signalled");
    assert!(
        store(root).current_interruption(&theirs).unwrap().is_none(),
        "never held"
    );
    assert!(t.pid() > 0);
}

// ---- DoD 9: the copy is pinned through the gap, across processes ---------------------------------

/// **DoD 9** — the holder is a channel step whose declared two-minute window ends INSIDE the gap; a
/// rival is queued behind it. The holder runs its real teardown and announces its end (a child that
/// merely exited would pass a predicate over unended sessions and prove nothing). After the window
/// has ended, a tick does not park the copy and does not hand the lease on — neither on the
/// expired-lease path nor on the tick's hold path — and the lease row is byte-identical after the
/// stop, after the gap tick, and before the take-up. Once `until + MARGIN` has passed with no
/// resume — the service never came back — the lease goes to the rival, and the hold still shows.
#[test]
fn the_copy_is_pinned_through_the_gap_and_released_past_the_bound() {
    let tmp = workspace();
    let root = tmp.path();
    a_repository(root);
    let (root_thread, slot, session) = commission_the_channel(root, T0);
    let (_, expires) = lease_row(root).expect("the coder's operation holds the copy");
    assert_eq!(
        expires,
        at(120),
        "the premise: the declared window ends at T0 + 2m"
    );
    commission(root, &at(5), "rival");
    at_work(root, &at(10), &session);
    the_coders_work(root);
    let _running = Session::sidecar_shaped(root, &session, &at(61));

    let report = secure(root, &at(60), 20, &|_| {});
    assert_eq!(report.held, [prepared(root, &session)], "{report:?}");
    assert!(appears(&root.join(format!("torn-down-{session}"))));
    let after_the_stop = lease_row(root);
    // until = at(110); the copy is kept to at(140).

    tick(root, &at(125), &slot);
    assert_eq!(lease_row(root), after_the_stop, "after the gap tick");
    the_coders_work_is_in_place(root);
    assert!(
        stub_starts(root).is_empty(),
        "the rival did not start: {:?}",
        stub_starts(root)
    );
    tick(root, &at(139), &root_thread);
    assert_eq!(lease_row(root), after_the_stop, "before the take-up");
    assert!(stub_starts(root).is_empty());

    // The service never came back: past the bound the ordinary reclaim applies.
    tick(root, &at(141), &slot);
    let starts = stub_starts_n(root, 1);
    assert!(
        starts
            .iter()
            .any(|s| s["role"]["handle"] == "rival" || s.to_string().contains("You are rival")),
        "the rival has the copy now: {starts:?}"
    );
    let row = status_thread(root, &at(141), &slot);
    assert_eq!(row["interrupted"]["limit"], SERVICE_RESTART_LIMIT, "{row}");
}
