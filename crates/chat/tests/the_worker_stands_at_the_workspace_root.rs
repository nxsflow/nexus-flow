//! **`nxc` resolves its worker at the WORKSPACE ROOT, not at the process working directory** (nxf
//! 6j6v.npn0).
//!
//! `Workspace::resolve` walks UPWARDS from the working directory until it finds a `.nxs/`, so
//! running `nxc` from a subdirectory of a workspace is entirely ordinary. The worker did not follow:
//! `CliCtx` built its `LazyWorker` from the process cwd, and `SidecarWorker` uses that one path for
//! three different things — where `.nxs/agent-logs/` is written, what the spec tells the session its
//! working directory is, and what the spawned process is `chdir`'d into.
//!
//! Two classes of damage followed, and the second one is why this is a bug rather than untidiness:
//!
//! 1. A `nxc send` from `crates/chat/` created `crates/chat/.nxs/agent-logs/` and rooted the agent
//!    in the subdirectory instead of at the workspace it was commissioned by.
//! 2. `Worker::session_is_running` reads `<cwd>/.nxs/agent-logs/<session>.pid`, so a LATER call from
//!    a different directory found no pid file and answered `false` — and `false` is the destructive
//!    direction at every site that asks. The channel-advance gate of nxf 6j6v.10yb hangs on exactly
//!    this read: it would let the next step into a checkout the previous step is still writing to,
//!    which is the failure that item was built to end.
//!
//! `nxc release` had this fixed NARROWLY for itself in 6j6v.fabb, because a wrong `false` there
//! hands the working copy on while the holding chain is still alive. That override is gone with the
//! verb itself (nxf 6j6v.b9nf); the rule that replaced it is general, and is covered below by
//! `session state` and `reply`, which the misread bites just as surely.
//!
//! Everything here drives the REAL `nxc` against the REAL `SidecarWorker` and a real live `node`,
//! for the reason the file next door records: this defect lives in the CLI adapter, and a test that
//! supplies its own worker cannot see it.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

use nexus_chat::workspace::{chat_config, setup};

/// A workspace with the measured `coding` channel and a `node` script that simply stays alive.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["coder", "finisher"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
        )
        .unwrap();
    }
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: coding\n  members: [coder, finisher]\n  flow: sequential\n  \
         working_tree: exclusive\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("stub-sidecar.mjs"),
        "setTimeout(() => {}, 30_000);\n",
    )
    .unwrap();
    std::fs::create_dir_all(tmp.path().join("sub").join("deeper")).unwrap();
    tmp
}

/// An `nxc` invocation standing in `dir` — either the workspace root or a subdirectory of it.
fn nxc_in(tmp: &TempDir, dir: &std::path::Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(dir)
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_WORKER", "sidecar")
        .env("NXC_SIDECAR", tmp.path().join("stub-sidecar.mjs"))
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", tmp.path().join("timer.log"));
    c
}

fn nxc(tmp: &TempDir) -> Command {
    nxc_in(tmp, tmp.path())
}

fn sub(tmp: &TempDir) -> std::path::PathBuf {
    tmp.path().join("sub").join("deeper")
}

fn persona<'a>(cmd: &'a mut Command, handle: &str, session: &str) -> &'a mut Command {
    cmd.env("NXC_ACTOR", handle).env("NXC_SESSION", session)
}

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim())
        .expect("valid json")
}

/// Every session the worker has actually started, read off the spec files it wrote at `root`.
fn started_sessions_under(root: &std::path::Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(root.join(".nxs/agent-logs"))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    e.file_name()
                        .to_str()
                        .and_then(|n| n.strip_suffix(".spec.json"))
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Every session started ANYWHERE under the workspace — the whole tree, not just the root.
///
/// The distinction is the point of this file: the defect did not fail to start the next step, it
/// started it somewhere else. A count taken at the root alone reads a stray `sub/deeper/.nxs/
/// agent-logs/<session>.spec.json` as "nothing started", which is the exact opposite of what
/// happened.
fn every_started_session(root: &std::path::Path) -> Vec<String> {
    fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if let Some(session) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(".spec.json"))
            {
                out.push(format!("{}:{session}", path.parent().unwrap().display()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

fn spec_of(tmp: &TempDir, session: &str) -> Value {
    let path = tmp
        .path()
        .join(".nxs/agent-logs")
        .join(format!("{session}.spec.json"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("the spec was written"))
        .expect("valid json")
}

/// Kill and reap every `node` this test started.
fn reap(tmp: &TempDir) {
    if let Ok(out) = std::process::Command::new("pgrep")
        .args([
            "-f",
            &format!("{}", tmp.path().join("stub-sidecar.mjs").display()),
        ])
        .output()
    {
        for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
            let _ = std::process::Command::new("kill")
                .args(["-9", pid])
                .status();
        }
    }
}

#[test]
fn a_send_from_a_subdirectory_writes_its_agent_logs_at_the_workspace_root() {
    let tmp = workspace();

    let opened = json_of(
        nxc_in(&tmp, &sub(&tmp)).args(["--json", "send", "--no-ref", "--to", "coding", "T5"]),
    );
    assert_eq!(opened["spawned"], true, "the round starts: {opened}");

    let at_root = started_sessions_under(tmp.path());
    assert_eq!(
        at_root.len(),
        1,
        "the session's spec belongs to the workspace that commissioned it: {at_root:?}"
    );
    assert!(
        !sub(&tmp).join(".nxs").exists(),
        "a second `.nxs/` under the directory the operator happened to stand in is exactly the \
         litter this item is about"
    );
    assert_eq!(
        spec_of(&tmp, &at_root[0])["cwd"],
        Value::String(tmp.path().canonicalize().unwrap().display().to_string()),
        "and the session is told to work at the root, not in the subdirectory"
    );

    reap(&tmp);
}

#[test]
fn the_liveness_gate_still_holds_when_the_reply_comes_from_a_subdirectory() {
    // The damaging half. The round opens at the root, so the pid file is there; the coder then
    // answers from somewhere else in the tree — an entirely ordinary thing for an agent working in
    // a package directory to do. Before this item that call resolved its worker at the SUBDIRECTORY,
    // found no pid file, read `false` for "is the session still running", and let the finisher into
    // the checkout the coder was still writing to.
    let tmp = workspace();
    json_of(nxc(&tmp).args(["--json", "send", "--no-ref", "--to", "coding", "T5"]));
    let coder = started_sessions_under(tmp.path())
        .into_iter()
        .next()
        .expect("step one started");
    let slot = spec_of(&tmp, &coder)["replyThread"]
        .as_str()
        .expect("a commissioned step is told its thread")
        .to_string();
    assert!(
        tmp.path()
            .join(".nxs/agent-logs")
            .join(format!("{coder}.pid"))
            .exists(),
        "premise: a real process holds this session, and its pid file is at the root"
    );

    json_of(persona(
        nxc_in(&tmp, &sub(&tmp)).args(["--json", "reply", "--thread", &slot, "INTERIM"]),
        "coder",
        &coder,
    ));

    let anywhere = every_started_session(tmp.path());
    assert_eq!(
        anywhere.len(),
        1,
        "the finisher must NOT have started: the coder's process is still writing into the working \
         copy this channel declared it needs alone, and where the operator stood does not change \
         that. Started: {anywhere:?}"
    );
    assert_eq!(
        started_sessions_under(tmp.path()),
        vec![coder.clone()],
        "and the one session there is, is still the coder's, at the root"
    );

    reap(&tmp);
}

#[test]
fn status_reads_the_live_session_from_a_subdirectory_and_says_so_on_the_thread() {
    // The read this file's rule reached in nxf 6j6v.qmy6, and the string only a real worker can
    // produce (review of PR #444, Test Quality #2/#5). `nxc status` now asks the worker whether a
    // thread's assignee session still has a process, so it inherits the whole class this file is
    // about — a `LazyWorker` built from the process cwd would find no pid file one directory down
    // and quietly report every live session as unknown, which reads on screen as "no information"
    // about a thread that is in fact working.
    //
    // Both halves in one run: the answer is correct, AND it is correct from a subdirectory.
    let tmp = workspace();
    json_of(nxc(&tmp).args(["--json", "send", "--no-ref", "--to", "coding", "T5"]));
    let coder = started_sessions_under(tmp.path())
        .into_iter()
        .next()
        .expect("step one started");
    let slot = spec_of(&tmp, &coder)["replyThread"]
        .as_str()
        .expect("a commissioned step is told its thread")
        .to_string();

    for dir in [tmp.path().to_path_buf(), sub(&tmp)] {
        let out = nxc_in(&tmp, &dir).args(["status"]).assert().success();
        let text = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
        assert!(
            text.contains(&slot) && text.contains("(its session is running)"),
            "a live `node` holds this session and the thread must say so, whatever directory the \
             operator stands in — here {}: {text}",
            dir.display()
        );
        assert!(
            !text.contains("cannot say whether a session"),
            "…and a worker that CAN answer must not draw the advisory note: {text}"
        );
    }

    // The other half of the same sentence, and the one the field exists for: once the session says
    // it is over while the thread still owes an answer, the row stops reading as "be patient".
    nxc(&tmp)
        .args(["session", "ended", &coder])
        .assert()
        .success();
    let out = nxc_in(&tmp, &sub(&tmp)).args(["status"]).assert().success();
    let hung = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(
        hung.contains("ITS SESSION HAS ENDED, nothing is coming"),
        "an open thread whose session announced its end is HUNG, and the terminal says it \
         outright: {hung}"
    );

    reap(&tmp);
}

#[test]
fn a_session_whose_pid_file_still_sits_in_a_subdirectory_reads_as_not_running_and_the_gate_opens() {
    // **THE RESIDUAL, pinned as a fact rather than left as a sentence** (review of PR #375,
    // Integrity #1/#2). The comment on `LazyWorker` used to say "no answer gets worse than it was",
    // comparing only with calls from any OTHER directory. That is too kind: an operator who worked
    // CONSISTENTLY out of one subdirectory — starting the session there and asking there — had a
    // liveness check that worked. After this change every read moves to the workspace root, so a
    // session still alive at the moment of the upgrade reads `false` where it used to read `true`.
    //
    // And it composes with nxf 6j6v.858n: a `false` here is what lets the next step open, so the
    // two-agents-in-one-checkout failure 6j6v.10yb exists to prevent becomes reachable through a
    // MISREAD instead of through a genuinely finished session. Bounded to that one session, and
    // self-healing — every trigger from here on writes at the root — but real, and this is what
    // says exactly how far it goes.
    let tmp = workspace();
    json_of(nxc(&tmp).args(["--json", "send", "--no-ref", "--to", "coding", "T5"]));
    let coder = started_sessions_under(tmp.path())
        .into_iter()
        .next()
        .expect("step one started");
    let slot = spec_of(&tmp, &coder)["replyThread"]
        .as_str()
        .expect("a commissioned step is told its thread")
        .to_string();

    // The upgrade boundary, staged: this session's pid file is where the OLD code would have put it
    // for a caller standing in `sub/deeper`, and no longer where the new code looks. The process
    // itself is untouched and very much alive.
    let stray = sub(&tmp).join(".nxs/agent-logs");
    std::fs::create_dir_all(&stray).unwrap();
    let pid_file = tmp
        .path()
        .join(".nxs/agent-logs")
        .join(format!("{coder}.pid"));
    let claim = std::fs::read_to_string(&pid_file).expect("the worker took a claim");
    // The claim holds the pid AND the instant that process started (nxf 6j6v.b9nf, so a signal can
    // never reach a recycled pid), so the pid is its FIRST LINE — handing the whole file to `kill`
    // is what a reader assuming the old one-line format does, and it fails to parse.
    let pid = claim.lines().next().expect("the claim names a pid").trim();
    std::fs::write(stray.join(format!("{coder}.pid")), &claim).unwrap();
    std::fs::remove_file(&pid_file).unwrap();
    assert!(
        std::process::Command::new("kill")
            .args(["-0", pid])
            .status()
            .is_ok_and(|s| s.success()),
        "premise: the process is alive — otherwise this test proves nothing about a MISREAD"
    );

    let misread = json_of(nxc(&tmp).args(["--json", "session", "state", &coder]));
    assert_eq!(
        misread["sessions"][0]["state"], "unknown",
        "the honest report of the residual: a live process the root cannot see reads as unknown, \
         not as running: {misread}"
    );

    // …and the consequence, said out loud rather than discovered: the gate opens on that misread.
    json_of(persona(
        nxc(&tmp).args(["--json", "reply", "--thread", &slot, "INTERIM"]),
        "coder",
        &coder,
    ));
    let after = started_sessions_under(tmp.path());
    assert_eq!(
        after.len(),
        2,
        "the finisher started while the coder's process is still alive — this is the bound of the \
         residual, not a behaviour anybody wants: {after:?}"
    );

    // THE BOUND ITSELF: everything started after the change writes its claim at the root, so the
    // gate sees the finisher perfectly. The misread cannot outlive the one session that predates it.
    let finisher = after
        .iter()
        .find(|s| **s != coder)
        .expect("a second session");
    let live = json_of(nxc(&tmp).args(["--json", "session", "state", finisher]));
    assert_eq!(
        live["sessions"][0]["state"], "running",
        "the next session is seen, because its pid file is where the workspace roots it: {live}"
    );

    reap(&tmp);
}

// `release_refuses_from_a_subdirectory_while_the_session_is_alive` was here — `nxc release`'s own
// coverage of the general fix above, exercised through the one verb whose refusal used to be "the
// ONLY thing that keeps it from being the blunt override epic 6j6v.bqe0 declined" (this file's own
// words for it). REMOVED with the verb (nxf 6j6v.b9nf): the subject is gone, and no surviving verb
// reproduces the same shape of failure. `nxc withdraw`'s worst case on the identical misread is NOT
// this test's "a false hands the checkout on mid-flight" — a live session that reads as not-running
// leaves withdraw with nothing to stop and nothing queued either, so it fails SAFE with `not_found`
// rather than destructively. The general fix this whole file pins — every worker built at the
// workspace root, and the one-session residual bounded and self-healing — is unchanged and remains
// covered above by `session state` and `reply`, neither of which this removal touches.
