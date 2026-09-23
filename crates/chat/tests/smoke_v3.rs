//! Live SDK smokes for role-runtime-v3 (nxf epic 6j6v.8e45, ticket 6j6v.8e45's own final task):
//! every other ticket in this epic ran with `NXC_WORKER=dry` (zero real API calls, fully
//! deterministic). These three tests are the ONE place in the epic where real domain content
//! actually EXECUTES: real Claude Agent SDK sessions (`NXC_WORKER=sidecar`), real subscription
//! usage, real wall-clock time (seconds to a few minutes per session). `#[ignore]`d so `cargo test
//! --all` never runs them (the ticket's own explicit design, "run with subscription auth, not
//! CI") — run individually, one at a time, with:
//!
//!   cargo test -p nexus-chat --test smoke_v3 -- --ignored --nocapture --test-threads=1 <name>
//!
//! Environment mirrors `agent-sidecar/scripts/skeleton.sh` exactly (this epic's own proven v1
//! precedent — re-derived here, not reinvented): `target/debug` prepended onto `PATH` so both this
//! test's own `nxc`/`nxs` calls AND every NESTED in-session `nxc` call a spawned role makes resolve
//! the fresh binary this epic built (`worker.rs::forwarded_real_env` forwards `PATH` verbatim from
//! the `nxc` process's OWN env when it spawns the sidecar, so setting it on the `Command` that
//! spawns `nxc` — see [`cmd`] — is sufficient; no process-global `std::env::set_var` needed, so
//! nothing here fights `cargo test`'s own parallelism); `NXC_WORKER=sidecar` +
//! `NXC_SIDECAR=<repo>/agent-sidecar/src/main.mjs`; deliberately NO `ANTHROPIC_API_KEY` (this
//! machine authenticates via an existing `claude` CLI session under `~/.claude/`, confirmed live —
//! `apiKeySource: "none"` — setting a placeholder key would make the SDK prefer it over that
//! working ambient auth and break everything). Every scratch workspace is a fresh
//! `tempfile::TempDir` — never this repo's own working directory; a spawned role's own Bash/Write
//! tool calls are `cwd`-scoped to that same temp dir end to end.
//!
//! ## Two real engine bugs, found live by this file, now FIXED (nxf tickets 6j6v.04es / 6j6v.81cq)
//!
//! The first two runs of these smokes surfaced two real, independently reproduced engine bugs —
//! both now fixed (fix-round commit, after the initial Task 15 commit) and RE-VERIFIED LIVE:
//!
//! 1. **6j6v.04es** — the ephemeral `on_complete: summarize` synthesizer (`cli.rs`'s
//!    `on_channel_complete`, `OnComplete::Summarize`) used to be spawned with
//!    `RoleSpec { tools: Some(vec![]), .. }` — an explicit ZERO-tools declaration that disabled
//!    ALL built-in tools including Bash, even though the channel's own `summary_prompt` instructs
//!    the model to post its synthesis via `nxc reply <thread> "<...>"`. It had no way to run that
//!    command. Separately, its internal session was never registered via
//!    `store.create_pending_session(...)` before being triggered, so `main.mjs`'s post-query `nxc
//!    session bind` callback crashed with `not_found`. Fixed: grant exactly `Bash`, and register
//!    the pending session (mirroring every other spawn site in `cli.rs`). Neither half was ever
//!    caught by this epic's prior (dry-worker) tests, because every one of them SIMULATES the
//!    synthesizer's own reply by hand (`tests/channel_complete.rs`'s own comment: "simulated here
//!    exactly as a real synthesizer session eventually would") — this file is the first to ever
//!    run a real synthesizer session. Regression coverage:
//!    `channel_complete.rs::synthesizer_spawn_grants_bash_and_registers_a_pending_session_6j6v_04es`.
//!    **Re-verified live**: smoke (a) below now genuinely PASSES (the requester is woken with real
//!    synthesized content) — see the Task 15 report's fix-round addendum for the observed run.
//! 2. **6j6v.81cq** — `timer.rs`'s `AtTimer::schedule` blocked on `child.wait_with_output()` with
//!    no bound at all. On a machine with no `atd`/`atrun` registered (confirmed directly: absent
//!    from `launchctl list`, and a bare `at` invocation hangs indefinitely with zero output), a
//!    declared channel `timeout` therefore wedged the ENTIRE calling role's live SDK session
//!    forever — an orphaned session was directly observed still running 8+ minutes past this
//!    file's own timeout before being found and killed by hand. Fixed: `wait_bounded` polls
//!    `try_wait()` against an 8-second bound, killing (and reaping) the child and returning a
//!    `TimedOut` error if it's exceeded — `open_declared_channel_and_fan_out`'s own existing
//!    best-effort handling (`if let Err(e) = schedule_tick(...) { eprintln!(...) }`) already
//!    degrades gracefully from there. Regression coverage: `timer.rs`'s
//!    `wait_bounded_returns_a_timeout_err_instead_of_hanging_forever` (simulates a hanging child
//!    via `sleep`, portable/deterministic — not dependent on any real machine's `atd` state).
//!    **Re-verified live** on the REAL `/usr/bin/at` on this real machine (still confirmed hanging
//!    on its own): a real `send --to` against a `timeout:`-bearing channel now returns in ~8s with a
//!    graceful warning instead of hanging — see the report for the exact observed output. Smokes
//!    (b)/(c) below no longer need to strip `timeout: 20m` from the copied example content either
//!    (see [`copy_v3_example_roles`]'s own updated doc).
//!
//! Each smoke below still polls with a generous bound and, if a run doesn't reach its expected
//! terminal state, FAILS with a diagnostic dump of the live state at the point of timeout — that
//! discipline stays, even though all three known causes of such a failure below are now fixed. It
//! is doing its job: re-verifying the first two fixes live (smoke (a) end-to-end; smoke (b)
//! re-run without the old `timeout:`-stripping workaround) surfaced a THIRD real, live bug — a
//! synthesizer's mis-formatted first `outcome:` line permanently forecloses recognizing a later,
//! correctly-formatted retry as the completing reply — reported at the time as a recommended
//! follow-up ticket, not silently patched; that ticket (nxf 6j6v.mj8x) is now ALSO fixed and
//! RE-VERIFIED LIVE (see stage 3's own doc comment in [`run_v3_build_and_ship`] for the precise
//! mechanism and fix). Re-verifying THAT fix live then surfaced a FOURTH real, live bug — the run
//! now correctly reaches `merge` but stalls indefinitely at `report` — again not fixed here,
//! reported as nxf ticket 6j6v.pavb (see the same stage-3 doc comment), same discipline: a new
//! finding is tracked, never silently patched or silently ignored.

use nxs_test_support::Command;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;
use std::time::{Duration, Instant};
use tempfile::TempDir;

// ---- environment plumbing ----------------------------------------------------------------------

/// The workspace root, fixed at compile time from THIS crate's manifest dir
/// (`<root>/crates/chat`) — mirrors `nxs-test-support::workspace_root`'s identical derivation from
/// `<root>/crates/test-support` (same depth), so it doesn't depend on the test's cwd.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/chat sits at <root>/crates/chat")
        .to_path_buf()
}

/// Build a `Command` for `bin` (`"nxc"` or `"nxs"`) wired for a LIVE role-runtime run — see the
/// module doc for why each piece is set the way it is. Goes through `nxs_test_support::cargo_bin`
/// (not a bare `assert_cmd::Command::cargo_bin`) so the stale-multicall-binary gate still runs.
fn cmd(bin: &str, ws: &Path) -> Command {
    let path = format!(
        "{}:{}",
        repo_root().join("target/debug").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut c = nxs_test_support::cargo_bin(bin);
    c.current_dir(ws)
        .env("PATH", path)
        // **Pinned since nxf 6j6v.07me**, and this is the one file where a pin buys something no
        // other pin here does. The CLI's default `<origin>` is the workspace's own replica prefix
        // now — random per scratch workspace, and these scenarios spell the qualified handles they
        // expect (`local/alpha`, `local/requester`, …) in their PROSE as much as in their
        // assertions, because a human reads this output while a live run is in flight. They are
        // `#[ignore]`d, so nothing in CI would have told anyone the literals had gone stale.
        .env("NXC_ORIGIN", "local")
        .env("NXC_WORKER", "sidecar")
        .env(
            "NXC_SIDECAR",
            repo_root().join("agent-sidecar/src/main.mjs"),
        );
    c
}

fn nxc(ws: &Path) -> Command {
    cmd("nxc", ws)
}

fn nxs(ws: &Path) -> Command {
    cmd("nxs", ws)
}

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or_else(|e| {
        panic!(
            "expected JSON, got {:?}: {e}",
            String::from_utf8_lossy(bytes)
        )
    })
}

/// `nxs init --json --module flow --module chat` (gotcha #6 from the proven skeleton: no
/// `--module` flags at all defaults to `flow` ONLY — always name both explicitly).
fn nxs_init(ws: &Path) {
    let out = nxs(ws)
        .args(["init", "--json", "--module", "flow", "--module", "chat"])
        .output()
        .expect("run nxs init");
    assert!(
        out.status.success(),
        "nxs init failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git(ws: &Path, args: &[&str]) {
    let status = SysCommand::new("git")
        .args(args)
        .current_dir(ws)
        .status()
        .expect("git available on PATH");
    assert!(status.success(), "git {args:?} failed in {}", ws.display());
}

/// A genuine `git init`-ed scratch repo (real history, mirroring how a real user would actually use
/// this — both smokes b/c need it: the v3 coder role's own instructions assume a feature branch to
/// commit/merge on).
fn git_init_scratch_repo(ws: &Path) {
    git(ws, &["init", "-q", "-b", "main"]);
    git(ws, &["config", "user.email", "smoke-v3@nxsflow.local"]);
    git(ws, &["config", "user.name", "nxsflow smoke-v3"]);
}

fn git_commit_all(ws: &Path, message: &str) {
    git(ws, &["add", "-A"]);
    git(ws, &["commit", "-q", "-m", message]);
}

fn v3_example_personas_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/role-runtime-v3/.nxs-personas")
}

/// Copy the REAL, shipped `role-runtime-v3` example content (ticket 6j6v.3nha) into the scratch
/// workspace's own `.nxs-personas/` dir — proving the actual example works, not a synthetic
/// stand-in (brief's own requirement for smokes b/c). Byte-for-byte verbatim, `channels.yaml`'s
/// declared `timeout: 20m` on the `review` channel included: an earlier version of this helper
/// stripped that field as a workaround for a real, live-confirmed `AtTimer::schedule` hang (nxf
/// ticket 6j6v.81cq — no bound around its `/usr/bin/at` subprocess call, and `at` genuinely never
/// responds on this machine). That bug is now fixed (`timer.rs`'s `wait_bounded`) and re-verified
/// live (see the Task 15 report's fix-round addendum) — a channel step declaring a real `timeout`
/// now degrades gracefully (a stderr warning, ~8s) instead of wedging the calling role's session
/// forever, so the workaround is no longer needed and the example content is copied unmodified.
/// Recurses into subdirectories rather than making a single flat pass — a flat, non-recursive
/// `std::fs::copy` over every top-level entry errors the moment it hits a directory path. The
/// shipped example has no subdirectory today (6j6v.dvyq §3 removed the one it had), and the
/// recursion stays anyway: what is copied is somebody else's folder, and the next thing declared
/// in a subdirectory would otherwise fail the copy rather than be ignored.
fn copy_v3_example_roles(ws: &Path) {
    copy_dir_recursive(&v3_example_personas_dir(), &ws.join(".nxs-personas"));
}

/// Copy every entry under `src` into `dst` (created if missing), recursing into subdirectories.
/// Shared by [`copy_v3_example_roles`] and covered directly by
/// [`copy_dir_recursive_copies_nested_subdirectories`] below (a fixture-only unit test — this
/// whole file's own scenarios are `#[ignore]`d, requiring a real Claude Agent SDK session).
fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create destination dir");
    for entry in std::fs::read_dir(src).expect("read source dir") {
        let entry = entry.expect("dir entry");
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            std::fs::copy(&src_path, &dst_path).expect("copy example file");
        }
    }
}

/// Not `#[ignore]`d (unlike every scenario below, which requires a real Claude Agent SDK session):
/// a small, fast fixture-only regression proving `copy_dir_recursive` handles a nested
/// subdirectory, without spinning up the whole smoke harness.
#[test]
fn copy_dir_recursive_copies_nested_subdirectories() {
    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("top.yaml"), "top: 1\n").unwrap();
    let nested = src.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(nested.join("deeper.yaml"), "handle: deeper\n").unwrap();

    let dst = TempDir::new().unwrap();
    copy_dir_recursive(src.path(), dst.path());

    assert_eq!(
        std::fs::read_to_string(dst.path().join("top.yaml")).unwrap(),
        "top: 1\n"
    );
    assert_eq!(
        std::fs::read_to_string(dst.path().join("nested/deeper.yaml")).unwrap(),
        "handle: deeper\n"
    );
}

/// Every file under `ws`, excluding `.git`/`.nxs`/`roles` (workspace plumbing, not the coder's own
/// work product), relative to `ws` — used to detect what a live coder session actually created.
fn list_workspace_files(ws: &Path) -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    let mut stack = vec![ws.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            let name = entry.file_name();
            if name == ".git" || name == ".nxs" || name == ".nxs-personas" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(path.strip_prefix(ws).unwrap().to_path_buf());
            }
        }
    }
    out
}

// ---- polling ------------------------------------------------------------------------------------

/// Poll `try_once` every `interval` until it returns `Some`, or give up (returning `None`) once
/// `bound` has elapsed. Every trigger in this codebase (`send`/`ask`/`reply`) is fire-and-forget —
/// there is no synchronous "wait for the real session to finish" call anywhere — so this is the
/// ONE mechanism every stage below waits on real model progress through.
fn poll_until<T>(
    bound: Duration,
    interval: Duration,
    mut try_once: impl FnMut() -> Option<T>,
) -> Option<T> {
    let start = Instant::now();
    loop {
        if let Some(v) = try_once() {
            return Some(v);
        }
        if start.elapsed() >= bound {
            return None;
        }
        std::thread::sleep(interval);
    }
}

/// Where the whole operation stands, as `nxc status --thread` renders it — the read that replaced
/// `workflow status` when 6j6v.dvyq §3 removed the run record.
fn operation_status(ws: &Path, thread_id: &str) -> Option<Value> {
    let out = nxc(ws)
        .args(["--json", "status", "--thread", thread_id])
        .output()
        .ok()?;
    out.status.success().then(|| json_of(&out.stdout))
}

fn latest_thread_for_channel(ws: &Path, channel_id: &str, consumer: &str) -> Option<String> {
    let out = nxc(ws)
        .args([
            "--json",
            "threads",
            "list",
            "--channel",
            channel_id,
            "--consumer",
            consumer,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let list = json_of(&out.stdout);
    // Thread ids are ULID-based (time-sortable, per `threads list`'s own doc: "channel + thread
    // ULID" ordering) — the LAST entry is the most recently opened.
    list.as_array()?
        .last()?
        .get("thread_id")?
        .as_str()
        .map(str::to_string)
}

fn threads_show(ws: &Path, thread_id: &str, consumer: &str) -> Option<Value> {
    let out = nxc(ws)
        .args([
            "--json",
            "threads",
            "show",
            thread_id,
            "--consumer",
            consumer,
        ])
        .output()
        .ok()?;
    out.status.success().then(|| json_of(&out.stdout))
}

/// A diagnostic dump of every session this run spawned (role, declared tools, resume target, and
/// its sidecar log) — read directly off `.nxs/agent-logs/`, mirroring `agent-sidecar/README.md`'s
/// own "observed run" documentation convention. Used ONLY inside a `panic!` message on a timeout,
/// so a failing smoke shows its actual history, never a bare "timed out" with no context.
fn dump_agent_logs(ws: &Path) -> String {
    let dir = ws.join(".nxs/agent-logs");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return format!("(no {} dir)", dir.display());
    };
    let mut sessions: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_string_lossy()
                .strip_suffix(".spec.json")
                .map(str::to_string)
        })
        .collect();
    sessions.sort();
    let mut out = String::new();
    for session in sessions {
        let spec_raw =
            std::fs::read_to_string(dir.join(format!("{session}.spec.json"))).unwrap_or_default();
        let spec: Value = serde_json::from_str(&spec_raw).unwrap_or_default();
        let log = std::fs::read_to_string(dir.join(format!("{session}.log"))).unwrap_or_default();
        out.push_str(&format!(
            "  session {session}: role={:?} tools={:?} resume={:?}\n    log: {}\n",
            spec.get("role"),
            spec.get("tools"),
            spec.get("resume"),
            log.lines().collect::<Vec<_>>().join(" | ")
        ));
    }
    out
}

// ==================================================================================================
// (a) Channel fan-out + synthesize — the cheapest, fastest smoke.
// ==================================================================================================
//
// A minimal, SYNTHETIC channel (not the full v3 example): two trivial reviewer roles (`alpha`/
// `beta`) whose whole job is a one-sentence opinion, and a `requester` role whose job is to open
// the ask (so there is a live session for the completion PUSH-wake to resume — `ask`'s own
// `refs.session_id` return-address stamping only fires when the caller has a real ambient
// `NXC_SESSION`, confirmed in this file's own pre-flight investigation: a bare-terminal `ask` never
// gets woken at all, by design, not by bug).

const OPINION_ROLE_SYSTEM_PROMPT: &str = "You are commissioned via an nxc thread. Reply with a \
one-sentence opinion on whatever you are asked, using the exact `nxc reply` command given to you \
in the message body. Nothing else.";

fn opinion_role_yaml(handle: &str) -> String {
    // `tools:` MUST sit at column 0, strictly after the `system_prompt: |` block scalar's own
    // (2-space-indented) body -- an earlier version of this helper indented it along with the
    // prompt text, which silently swallowed `tools: [Bash]` into the block scalar itself (valid
    // YAML, but no top-level `tools` key at all -- confirmed live: the spec.json this produced
    // carried `"tools":null`, not `["Bash"]`). `OPINION_ROLE_SYSTEM_PROMPT` is a single physical
    // line, so no further re-indentation is needed here.
    format!("handle: {handle}\nsystem_prompt: |\n  {OPINION_ROLE_SYSTEM_PROMPT}\ntools: [Bash]\n")
}

const OPINIONS_CHANNEL_YAML: &str = "\
- name: opinions
  members: [alpha, beta]
  expects: all
  on_complete: summarize
  visibility: requester_only
  summary_prompt: |
    You are the synthesizer for the opinions channel. You have been handed the full thread,
    including both members' one-sentence opinions. Reply with exactly ONE aggregated sentence
    combining both viewpoints into the SAME thread you were handed, with the body on STDIN:
    `nxc reply --thread <the thread id you were handed> - <<'EOF'`, your sentence, then `EOF`
    on its own line.
";

const REQUESTER_ROLE_YAML: &str = "\
handle: requester
system_prompt: |
  You are triggered once to kick off a channel review, and possibly again later when it completes.
  Tell which turn you are on from your OWN conversation history (a truly fresh session has no
  earlier turn of yours in it at all).

  CASE A -- FIRST TURN (no earlier turn of yours exists): the text you were given is a trivial
  question. Open it: `nxc send --to opinions \"<the question, in your own words>\"`. Do nothing else this
  turn.

  CASE B -- RESUMED TURN (you already have an earlier turn): the text you were just given is the
  opinions channel's combined synthesis. Write it VERBATIM to a new file `result.txt` in the
  current directory using the Write tool. Do nothing else.
tools: [Bash, Write]
";

#[test]
#[ignore = "spawns real Claude Agent SDK sessions (NXC_WORKER=sidecar) -- real cost/time, run manually"]
fn channel_fanout_and_synthesize_wakes_the_requester_with_real_content() {
    let ws = TempDir::new().expect("scratch workspace");
    let root = ws.path();

    nxs_init(root);
    let roles = root.join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(roles.join("alpha.yaml"), opinion_role_yaml("alpha")).unwrap();
    std::fs::write(roles.join("beta.yaml"), opinion_role_yaml("beta")).unwrap();
    std::fs::write(roles.join("channels.yaml"), OPINIONS_CHANNEL_YAML).unwrap();
    std::fs::write(roles.join("requester.yaml"), REQUESTER_ROLE_YAML).unwrap();

    let question = "In one word, which is better for a small team: monorepo or polyrepo? Give a \
                     one-sentence take.";
    let out = nxc(root)
        .args(["--json", "send", "--to", "requester", question])
        .output()
        .expect("send --to requester");
    assert!(
        out.status.success(),
        "send --to requester failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Stage 1: both trivial reviewers must reply for real.
    //
    // `latest_thread_for_channel`'s own `--consumer` is the BARE role handle ("alpha"), not the
    // qualified `local/alpha` chat identity: `ensure_declared_channel` joins a declared channel's
    // `members:` in the SAME bare role-handle namespace `resolve_role`/`fan_out_targets` use — only
    // the channel's own OPENER (here, `requester`) is joined under its qualified identity (see that
    // function's own doc). `threads list --channel` is membership-gated only (bare "alpha" is a
    // genuine member), unaffected by `visibility`.
    //
    // `threads_show`'s own consumer is different, and MUST be the OPENER's QUALIFIED identity
    // (`local/requester` — the opener alone is joined qualified, see the paragraph above), not a
    // plain member's bare handle ("alpha"): `opinions` declares `visibility: requester_only`
    // (independent review, Code Quality #1/Integrity #3, High — now genuinely enforced, see
    // `channel::filter_board_messages`), so a non-opener member's own view is filtered down to the
    // opening message plus ONLY that member's own reply — reading as "alpha" would never see
    // beta's reply at all, and this poll would never observe both. Only the requester sees every
    // reply, by design.
    //
    // Checked against the `messages` array, NOT `replied`/`outstanding` (confirmed live: the
    // moment the second reviewer's reply completes the quorum, `on_channel_complete` retargets
    // `expects_reply_from` to the synthesizer's own identity as PART OF that SAME `reply` call —
    // so `replied` flips back to empty against the NEW expects set essentially synchronously.
    // There is no observable window where `replied.len() == 2` — polling on it never sees it turn
    // true, even though both replies genuinely landed. `messages` never gets retargeted or
    // pruned, so it is the only reliable place to look for "did both reviewers actually reply".
    let stage1_bound = Duration::from_secs(150);
    let board = poll_until(stage1_bound, Duration::from_secs(8), || {
        let tid = latest_thread_for_channel(root, "decl:opinions", "alpha")?;
        let board = threads_show(root, &tid, "local/requester")?;
        let messages = board["messages"].as_array()?;
        let both_replied = ["local/alpha", "local/beta"]
            .iter()
            .all(|h| messages.iter().any(|m| m["sender"] == *h));
        both_replied.then_some(board)
    });
    let Some(board) = board else {
        panic!(
            "timed out after {stage1_bound:?} waiting for both alpha and beta to reply for real.\n\
             agent-logs:\n{}",
            dump_agent_logs(root)
        );
    };

    let messages = board["messages"].as_array().expect("messages array");
    for handle in ["local/alpha", "local/beta"] {
        let reply = messages
            .iter()
            .find(|m| m["sender"] == handle)
            .unwrap_or_else(|| panic!("no reply from {handle} in board: {board:#?}"));
        let body = reply["body"].as_str().unwrap_or_default();
        assert!(
            body.len() > 10 && body != question,
            "{handle}'s reply must be real, non-echo model output, got: {body:?}"
        );
    }

    // Stage 2: the requester must be woken with the synthesizer's real content, written verbatim
    // to result.txt via its own Write tool. Now that nxf ticket 6j6v.04es is fixed (see this
    // file's module doc), this is expected to genuinely PASS -- the bound below (200s) is generous
    // relative to how fast the mechanism actually runs (both reviewers above replied within
    // seconds, and the synthesizer itself is a short, tool-light turn), not a hopeful maximum. A
    // timeout here would now be a real regression, not the documented, expected outcome it used to
    // be before the fix.
    let result_path = root.join("result.txt");
    let stage2_bound = Duration::from_secs(200);
    let delivered = poll_until(stage2_bound, Duration::from_secs(10), || {
        std::fs::read_to_string(&result_path).ok()
    });

    let Some(content) = delivered else {
        let tid = latest_thread_for_channel(root, "decl:opinions", "alpha");
        let final_board = tid
            .as_deref()
            .and_then(|t| threads_show(root, t, "local/requester"));
        panic!(
            "requester was never woken with the synthesizer's real content within \
             {stage2_bound:?} (result.txt never appeared) -- this is now a REGRESSION: nxf ticket \
             6j6v.04es (the synthesizer's zero-tools grant + missing create_pending_session) was \
             fixed in cli.rs's on_channel_complete; if this fires again, check that fix hasn't been \
             reverted or that a NEW failure mode has appeared.\n\
             Final thread state: {final_board:#?}\n\
             agent-logs:\n{}",
            dump_agent_logs(root)
        );
    };

    assert!(!content.trim().is_empty(), "result.txt exists but is empty");
    assert!(
        content.len() > 20 && content != question,
        "synthesis content must be real, non-echo model output, got: {content:?}"
    );
}

// ==================================================================================================
// (b)/(c) A full `build-and-ship` operation, end-to-end, over the REAL v3 example content.
// ==================================================================================================
//
// Both smokes drive the identical mechanism (`nxc send --to build-and-ship` on the real
// `examples/role-runtime-v3/.nxs-personas/` content, copied byte-for-byte verbatim into a fresh
// git-init'd scratch repo) -- they differ only in the work order's size/shape. The genuinely new
// evidence smoke (c) adds over (b) is qualitative -- does a real coder handle a marginally bigger,
// two-part work order (implementation AND a test) sensibly in a genuine git repo?
//
// **RE-CUT BY 6j6v.dvyq §3 AND NOT RE-RUN LIVE SINCE, which is said here rather than left to be
// found.** These smokes used to drive `nxc workflow start --name build-and-ship` and poll the run
// record for `current_step`/`status: done`. That surface is gone: the example declares
// `build-and-ship` as a `flow: sequential` CHANNEL now, so the operation is a thread and its
// progress is read with `nxc status --thread`. The stages below are the same three stages against
// the same real content; what changed is what they poll. Anyone with subscription auth to hand
// should run them and record the result -- until then the shape is reasoned, not observed.
//
// Two live findings this file carried are worth keeping straight through that re-cut:
//
// * nxf 6j6v.04es (synthesizer) and 6j6v.81cq (AtTimer hang) were real engine bugs found here and
//   fixed; both are in code the re-cut did not touch, so they stay fixed.
// * nxf 6j6v.pavb -- a run that reached `merge` and then stalled at its `report` step forever --
//   was a finding ABOUT the run engine's own step-firing, and that engine is gone. The stall shape
//   it describes cannot arise on a channel flow, where the last member's answer is what the
//   supervisor hands back. It is not "fixed"; its subject was removed.

/// Shared machinery for smokes (b)/(c): hand the real `build-and-ship` channel a work order on a
/// fresh, git-init'd scratch repo seeded with the real v3 example declarations, and drive it
/// through the coder -> review round for real, then wait (bounded) for the operation to come back
/// answered.
fn run_v3_build_and_ship(work_order: &str, stage3_bound: Duration) {
    let ws = TempDir::new().expect("scratch workspace");
    let root = ws.path();

    git_init_scratch_repo(root);
    copy_v3_example_roles(root);
    std::fs::write(
        root.join("README.md"),
        "# smoke-v3 toy project\n\nA disposable scratch repo for a live role-runtime-v3 smoke.\n",
    )
    .unwrap();
    git_commit_all(root, "initial scaffold");
    nxs_init(root);
    git_commit_all(root, "nxs init");

    let before = list_workspace_files(root);

    let out = nxc(root)
        .args(["--json", "send", "--to", "build-and-ship", work_order])
        .output()
        .expect("run nxc send --to build-and-ship");
    assert!(
        out.status.success(),
        "send --to build-and-ship failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let operation = json_of(&out.stdout)["thread_id"]
        .as_str()
        .expect("send --to a channel always returns the thread it opened")
        .to_string();

    // Stage 1: the coder implements the work order for real (a live SDK session with Bash/Read/
    // Write, scoped to this scratch repo only) and answers on its own thread, which is what starts
    // the second step. The observable is the WORKSPACE, not a run record: the coder's answer is
    // what makes the review channel's own thread appear below the board, and stage 2 waits on that.
    let stage1_bound = Duration::from_secs(360);
    let implemented = poll_until(stage1_bound, Duration::from_secs(10), || {
        let after = list_workspace_files(root);
        let new_files: Vec<PathBuf> = after.difference(&before).cloned().collect();
        (!new_files.is_empty()).then_some(new_files)
    });
    let Some(new_files) = implemented else {
        panic!(
            "coder never produced a file within {stage1_bound:?}. Operation: {:?}\n\
             agent-logs:\n{}",
            operation_status(root, &operation),
            dump_agent_logs(root)
        );
    };
    for f in &new_files {
        let content = std::fs::read_to_string(root.join(f)).unwrap_or_default();
        assert!(
            !content.trim().is_empty(),
            "coder's new file {f:?} is empty"
        );
    }

    // Stage 2: the review channel's 4 real reviewer roles (general/code-quality/test-quality/
    // integrity) each reply for real -- proving the actual shipped example content, not a
    // synthetic stand-in.
    //
    // `latest_thread_for_channel`'s own `--consumer` is the BARE role handle ("general"), not the
    // qualified `local/general` chat identity -- see smoke (a)'s identical comment:
    // `ensure_declared_channel` joins a declared channel's `members:` in the bare role-handle
    // namespace; only the channel's own OPENER (here, the coder, via `fire_channel_step`) is
    // joined qualified. `threads list --channel` is membership-gated only, unaffected by
    // `visibility`.
    let review_tid = poll_until(Duration::from_secs(60), Duration::from_secs(5), || {
        latest_thread_for_channel(root, "decl:review", "general")
    })
    .unwrap_or_else(|| {
        panic!(
            "review channel thread never appeared; agent-logs:\n{}",
            dump_agent_logs(root)
        )
    });

    // Checked against `messages`, NOT `replied`/`outstanding` -- same race as smoke (a)'s identical
    // comment: the completing reviewer's OWN `reply` call retargets `expects_reply_from` to the
    // synthesizer's identity synchronously, so `replied` never observably reaches 4 even when all
    // 4 real replies genuinely landed. `messages` is append-only and never retargeted.
    //
    // `threads_show`'s own consumer here MUST be the OPENER's QUALIFIED identity
    // (`local/coder` -- the opener alone is joined qualified, see the paragraph above), not a
    // plain reviewer's bare handle ("general"): the shipped `review` channel declares
    // `visibility: requester_only` (independent review, Code Quality #1/Integrity #3, High -- now
    // genuinely enforced, see `channel::filter_board_messages`), so a non-opener reviewer's own
    // view is filtered down to the opening message plus ONLY that reviewer's own reply -- reading
    // as "general" would never see the other three reviewers' replies at all, and this poll would
    // never observe all four.
    let expected_reviewers = [
        "local/general",
        "local/code-quality",
        "local/test-quality",
        "local/integrity",
    ];
    let stage2_bound = Duration::from_secs(360);
    let quorum = poll_until(stage2_bound, Duration::from_secs(10), || {
        let board = threads_show(root, &review_tid, "local/coder")?;
        let messages = board["messages"].as_array()?;
        let all_replied = expected_reviewers
            .iter()
            .all(|h| messages.iter().any(|m| m["sender"] == *h));
        all_replied.then_some(board)
    });
    let Some(board) = quorum else {
        panic!(
            "not all 4 reviewers replied within {stage2_bound:?}. Last board: {:?}\n\
             agent-logs:\n{}",
            threads_show(root, &review_tid, "local/coder"),
            dump_agent_logs(root)
        );
    };
    // At least one reviewer's real reply text is inspectable, and demonstrably real (the declared
    // Output Format's own literal marker, not an echo of the trigger body).
    let messages = board["messages"].as_array().unwrap();
    let a_review = messages
        .iter()
        .find(|m| m["sender"] == "local/general")
        .expect("general's reply is in the thread");
    let review_body = a_review["body"].as_str().unwrap_or_default();
    assert!(
        review_body.contains("Grade:"),
        "expected the role's own declared Output Format ('Grade: ...'), got: {review_body}"
    );

    // Stage 3: the operation comes back ANSWERED. The `review` round folds its four verdicts, its
    // supervisor hands the fold up to the `build-and-ship` supervisor, and that one discharges the
    // board the work order was sent to — which is what "the operation is done" means once there is
    // no run record to ask (6j6v.dvyq §3).
    //
    // A real bug found live here and FIXED (nxf 6j6v.mj8x), kept because the code it fixed is
    // untouched by that re-cut: phase-2 "completing reply" recognition used to key on the sender's
    // FIRST message in the thread, so a synthesizer that self-corrected with a second, better reply
    // was never recognized as completing anything and the round hung with the right answer sitting
    // in the thread. `is_completing_reply` now recognizes ANY reply from the thread's sole expected
    // handle when that handle is the qualified synthesis marker.
    let done = poll_until(stage3_bound, Duration::from_secs(15), || {
        let view = threads_show(root, &operation, "local/carsten")?;
        view["complete"].as_bool().unwrap_or(false).then_some(view)
    });
    let Some(final_board) = done else {
        let stuck_review = threads_show(root, &review_tid, "local/coder");
        panic!(
            "the operation never came back answered within {stage3_bound:?}. Read the whole tree \
             below before assuming a regression: a stage that is still WORKING looks the same from \
             here as one that is stuck, and `nxc status --thread` is what tells them apart.\n\
             operation: {:?}\n\
             review thread (last known state): {stuck_review:#?}\n\
             agent-logs:\n{}",
            operation_status(root, &operation),
            dump_agent_logs(root)
        );
    };
    assert!(
        final_board["complete"].as_bool().unwrap_or(false),
        "the board the work order was sent to is discharged: {final_board:#?}"
    );
}

#[test]
#[ignore = "spawns real Claude Agent SDK sessions (NXC_WORKER=sidecar) -- real cost/time, run manually"]
fn full_build_and_ship_run_reaches_a_terminal_state_on_a_scratch_workspace() {
    // 480s, which was enough for a full coder + four-reviewer + synthesizer chain in the runs this
    // file recorded. It is a bound on a REAL model run, so read a timeout as "look at the tree"
    // rather than as a verdict.
    run_v3_build_and_ship(
        "Add a function `add(a, b)` that returns their sum, in a new file `add.js`. No tests needed.",
        Duration::from_secs(480),
    );
}

#[test]
#[ignore = "spawns real Claude Agent SDK sessions (NXC_WORKER=sidecar) -- real cost/time, run manually"]
fn toy_repo_dogfood_build_and_ship_on_a_real_git_repo() {
    run_v3_build_and_ship(
        "Add a function `add(a, b)` that returns their sum in a new file `add.js`, AND a small test \
         for it using whatever test convention fits a plain JS file (no test framework or dependency \
         installs needed -- a simple assert-based script is fine).",
        Duration::from_secs(480),
    );
}
