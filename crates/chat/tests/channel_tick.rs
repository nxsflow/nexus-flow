//! Black-box CLI tests for the timeout timer + `nxc workflow tick` (nxf ticket 6j6v.z06n): opening
//! a declared channel with a `timeout` schedules a one-shot timer (`NXC_TIMER=dry`/`NXC_TIMER_LOG`,
//! mirroring `NXC_WORKER=dry`/`NXC_DRY_LOG`'s exact convention) whose scheduled command is `nxc
//! workflow tick --thread <id>` — what a real `at` job would eventually run. `tick` re-derives the
//! thread's completion/staleness and, if due, routes it through the SAME completion logic `reply`'s
//! PUSH-wake block uses (Task 9), now shared via `cli.rs`'s `route_declared_channel_completion`.
//!
//! Two deliberate scope deviations from the ticket's own literal sketch (see `cli.rs`'s
//! `WorkflowAction::Tick`/`timer.rs`'s own doc for the full reasoning, not repeated here):
//! `schedule_tick`/`tick` are keyed on the THREAD, never a `run_id`; and `tick`'s own IDEMPOTENCY —
//! not a real cross-process `cancel` — is what makes "no double-fire" true, since a scheduled OS
//! job fires in a different process invocation than the one that scheduled it, with no shared
//! memory between them.
//!
//! Determinism mirrors `channel_fanout.rs`/`channel_complete.rs`: `NXC_ACTOR=alice`/
//! `NXC_ORIGIN=local`/`NXC_NOW` pinned by the `nxc()` helper, overridden per-call where a test needs
//! to simulate the clock advancing past a deadline.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
use tempfile::TempDir;

// ---- direct (non-subprocess) test env guard (mirrors channel_complete.rs's identical precedent)

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard<'a>(#[allow(dead_code)] std::sync::MutexGuard<'a, ()>);

impl Drop for EnvGuard<'_> {
    fn drop(&mut self) {
        std::env::remove_var("NXC_TIMER");
        std::env::remove_var("NXC_TIMER_LOG");
    }
}

/// Acquire the whole-body serialization guard for the one direct (non-subprocess) test below that
/// sets process-global env vars `timer::schedule_tick`/`cancel` read directly. A poisoned mutex (a
/// PRIOR test panicked while holding it) is recovered rather than propagated.
fn lock_env() -> EnvGuard<'static> {
    EnvGuard(ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner()))
}

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

fn nxc(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env("NXC_ACTOR", "alice")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", "2026-07-22T00:00:00Z");
    c
}

fn json_of(out: &[u8]) -> Value {
    serde_json::from_str(String::from_utf8_lossy(out).trim()).expect("valid json")
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

fn personas_dir(tmp: &TempDir) -> PathBuf {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    roles
}

fn write_role(roles: &std::path::Path, handle: &str) {
    std::fs::write(
        roles.join(format!("{handle}.yaml")),
        format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
    )
    .unwrap();
}

fn write_channels_yaml(roles: &std::path::Path, yaml: &str) {
    std::fs::write(roles.join("channels.yaml"), yaml).unwrap();
}

/// Every dry-log line that opens a trigger entry (mirrors `channel_fanout.rs`'s/`channel_complete.
/// rs`'s identical helper — the message field itself may embed real newlines).
fn trigger_header_lines(dry: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(dry)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("trigger role="))
        .map(str::to_string)
        .collect()
}

fn field(line: &str, key: &str) -> String {
    let marker = format!("{key}=");
    let start = line
        .find(&marker)
        .unwrap_or_else(|| panic!("no {marker} in {line}"))
        + marker.len();
    line[start..]
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

fn timer_log_lines(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// Bind a fresh pending session `s-pm` to role `pm` and its real SDK id `real-pm` — the return
/// address a declared channel's opener needs for the PUSH-wake / `workflow tick` routing to have
/// anyone to wake (mirrors `channel_complete.rs`'s `live_reply_path_...` setup exactly).
/// The member thread `handle` was given below `channel_thread` (nxf 6j6v.pf6j) — where a channel
/// member actually answers now that every thread has exactly two ends.
fn member_thread(tmp: &TempDir, channel_thread: &str, handle: &str) -> String {
    let store = open_store(tmp);
    let want = format!("local/{handle}");
    store
        .supervised_children(channel_thread)
        .unwrap()
        .into_iter()
        .find(|t| {
            store
                .thread_quorum(t, "2026-07-22T00:00:00Z")
                .unwrap()
                .is_some_and(|q| q.expects == [want.clone()])
        })
        .unwrap_or_else(|| panic!("no member thread for {handle} under {channel_thread}"))
}

fn seed_pm_session(tmp: &TempDir) {
    let mut store = open_store(tmp);
    store.create_pending_session("s-pm", "pm").unwrap();
    drop(store);
    nxc(tmp)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();
}

// ---- channel.timeout -> the effective deadline -----------------------------------------------

#[test]
fn opening_a_declared_channel_with_a_timeout_schedules_a_tick() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "bob");
    write_channels_yaml(&roles, "- name: review\n  members: [bob]\n  timeout: 20m\n");
    let timer_log = tmp.path().join("timer.log");

    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"))
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", &timer_log)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    let tid = v["thread_id"].as_str().unwrap().to_string();
    assert_eq!(
        v["deadline"], "2026-07-22T00:20:00Z",
        "channel.timeout becomes the effective deadline when no --deadline is given: {v}"
    );

    let lines = timer_log_lines(&timer_log);
    assert_eq!(lines.len(), 1, "exactly one schedule call: {lines:?}");
    assert!(lines[0].starts_with("schedule "), "{lines:?}");
    assert!(lines[0].contains(&format!("thread={tid}")), "{lines:?}");
    assert!(
        lines[0].contains("deadline=2026-07-22T00:20:00Z"),
        "{lines:?}"
    );
    assert!(
        lines[0].contains(&format!("command=nxc tick --thread {tid}")),
        "{lines:?}"
    );
}

// `an_explicit_deadline_on_ask_overrides_the_channels_own_declared_timeout` stood here. REMOVED
// with `send --deadline` (nxf 6j6v.ckeq, decision 2): a board's window is the CHANNEL's own
// `timeout:`, and a per-call override beside a declared value is two answers to one question —
// owner, 2026-08-21: "sonst wird ein Agent vielleicht uebereifrig Optionen aendern". With nothing
// able to override, there is no precedence left to assert.
//
// What the test also covered incidentally — that the TIMER is scheduled against the effective
// deadline — is the subject of `the_channels_declared_timeout_schedules_a_one_shot_tick` above it,
// which drives the declared value; and the ABSOLUTE-instant form of a deadline, which this was the
// only `--deadline` test to reach, moved to the declaration in
// `channel_member_timeout.rs::a_deadline_given_as_an_absolute_instant_arms_no_clock_and_no_
// transcript_moves_it` (`timeout:` is parsed by the same `facade::resolve_deadline_spec` grammar,
// so an RFC3339 instant is still expressible — only not per call).

#[test]
fn a_channel_with_no_timeout_and_no_explicit_deadline_schedules_no_tick_at_all() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "bob");
    write_channels_yaml(&roles, "- name: review\n  members: [bob]\n");
    let timer_log = tmp.path().join("timer.log");

    nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"))
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", &timer_log)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();

    assert!(
        !timer_log.exists(),
        "no deadline at all means nothing to schedule"
    );
}

// ---- tick: rejection + clean no-ops -------------------------------------------------------------

#[test]
fn tick_on_an_unknown_thread_id_is_not_found() {
    let tmp = workspace();
    let out = nxc(&tmp)
        .args(["--json", "tick", "--thread", "m-does-not-exist"])
        .assert()
        .failure();
    assert_eq!(
        json_of(&out.get_output().stdout)["error"]["kind"],
        "not_found"
    );
}

#[test]
fn tick_before_the_deadline_with_replies_still_pending_is_a_clean_no_op() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "bob");
    write_role(&roles, "carol");
    write_channels_yaml(
        &roles,
        "- name: review\n  members: [bob, carol]\n  timeout: 20m\n",
    );

    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"))
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", tmp.path().join("timer.log"))
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let tid = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Nobody has replied; NXC_NOW is unchanged (still well before the 20-minute deadline).
    //
    // `NXC_TIMER=dry` on the TICK as well as on the open (nxf 6j6v.nf38): a declining tick now
    // re-arms the one-shot job, so this call schedules — and an unset `NXC_TIMER` would mean the
    // real `at` subprocess, which no test may reach for.
    let out = nxc(&tmp)
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", tmp.path().join("timer.log"))
        .args(["--json", "tick", "--thread", &tid])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["acted"], false, "{v}");
    assert_eq!(v["reason"], "not_due", "{v}");
    assert_eq!(
        v["outstanding"],
        serde_json::json!(["local/bob", "local/carol"]),
        "{v}"
    );
    // …and it did not just decline: it moved the job to the next instant a member falls due. A
    // one-shot `at` job fires once, so a decline that scheduled nothing would spend the automatic
    // strike on the first tick that ever fired early (nxf 6j6v.nf38).
    let lines = timer_log_lines(&tmp.path().join("timer.log"));
    assert_eq!(
        lines.len(),
        2,
        "the open scheduled, the tick re-armed: {lines:?}"
    );
    assert!(
        lines[1].contains(&format!("thread={tid}"))
            && lines[1].contains("deadline=2026-07-22T00:20:00Z"),
        "…for the earliest member deadline still ahead: {lines:?}"
    );
}

#[test]
fn tick_on_a_non_declared_channels_thread_is_a_deliberate_no_op() {
    let tmp = workspace();
    // A board on a channel NO DECLARATION names. `channels create` + `ask` built it until 6j6v.dvyq
    // §3 removed both; the shape they produced is still reachable through the write they delegated
    // to, and it is the shape this test is about — a thread with no `on_complete` policy behind it.
    let tid = {
        let mut store = open_store(&tmp);
        store.set_channel_field("c-raw", "kind", "group", "local/alice");
        store.add_member("c-raw", "local/alice", "local/alice");
        nexus_chat::facade::ask(
            &mut store,
            nexus_chat::facade::AskRequest {
                now: "2026-07-22T00:00:00Z",
                origin: "local",
                actor: "alice",
                channel: "c-raw",
                body: "please review",
                expect: &["local/bob".to_string()],
                deadline: None,
                kind: nexus_chat::model::MessageKind::Question,
                priority: nexus_chat::model::Priority::Normal,
                refs: nexus_chat::model::Refs::default(),
            },
        )
        .expect("the board opens")
        .thread_id
    };

    // Even fully complete, a non-declared channel's thread has no `on_complete` policy behind it at
    // all — nothing for tick to do (the pre-existing M2 quorum mechanism, untouched).
    nxc(&tmp)
        .env("NXC_ACTOR", "bob")
        .args(["reply", "--thread", &tid, "lgtm"])
        .assert()
        .success();

    let out = nxc(&tmp)
        .args(["--json", "tick", "--thread", &tid])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["acted"], false, "{v}");
    assert_eq!(v["reason"], "not_a_declared_channel", "{v}");
}

// ---- past deadline, partial replies: completes-on-timeout ---------------------------------------

#[test]
fn tick_past_deadline_with_partial_replies_completes_on_timeout_pass_through() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    write_role(&roles, "bob");
    write_role(&roles, "carol");
    write_channels_yaml(
        &roles,
        "- name: review\n  members: [bob, carol]\n  timeout: 1m\n",
    );
    seed_pm_session(&tmp);

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-pm")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", tmp.path().join("timer.log"))
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let tid = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    // bob replies in time, in HIS OWN thread; carol never does.
    let bob_thread = member_thread(&tmp, &tid, "bob");
    nxc(&tmp)
        .env("NXC_ACTOR", "bob")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["reply", "--thread", &bob_thread, "looks fine so far"])
        .assert()
        .success();
    assert_eq!(
        trigger_header_lines(&dry).len(),
        2,
        "fan-out only so far, no completion yet"
    );

    // Simulate the timer firing well past the 1-minute deadline.
    let out = nxc(&tmp)
        .env("NXC_NOW", "2026-07-22T00:05:00Z")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "tick", "--thread", &tid])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["acted"], true, "{v}");
    assert_eq!(
        v["outstanding"],
        serde_json::json!(["local/carol"]),
        "noting who didn't respond: {v}"
    );

    let lines = trigger_header_lines(&dry);
    assert_eq!(lines.len(), 3, "exactly one new wake trigger: {lines:?}");
    assert_eq!(field(&lines[2], "role"), "pm");
    assert_eq!(field(&lines[2], "session"), "s-pm");
    let log = std::fs::read_to_string(&dry).unwrap();
    // The DEFINED pass-through shape (nxf 6j6v.e9qj): one delimited block per collected message,
    // so a partial delivery is still unambiguous about whose answer is whose.
    assert!(
        log.contains("<message from=\"local/bob\">\nlooks fine so far\n</message>"),
        "the partial reply that DID land must be delivered: {log}"
    );
    assert!(
        !log.contains("from=\"local/carol\""),
        "carol never replied — must not appear as a block at all: {log}"
    );
}

#[test]
fn tick_whose_wake_cannot_spawn_still_succeeds_and_reports_the_skip_in_json() {
    // Independent review of PR #265, Integrity & Robustness #1, on the CLI seam. `tick` shares
    // `reply`'s completion routing and its "skip, don't fail" contract — but it used to discard the
    // routing's `wake_skipped`, so `--json` said `acted: true` and nothing else while the requester
    // was never woken. For the agent consumer `--json` exists for, that is indistinguishable from a
    // delivered hand-off; the stderr breadcrumb below is for the human at the terminal.
    //
    // The trigger is made to fail hermetically exactly as `verbs.rs`'s reply twin does: `NXC_DRY_LOG`
    // pointed at a DIRECTORY, so `DryWorker`'s append open gets `EISDIR` — a real `Worker::trigger`
    // failure with no sidecar, no network and no timing involved.
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    write_role(&roles, "bob");
    write_channels_yaml(&roles, "- name: review\n  members: [bob]\n  timeout: 1m\n");
    seed_pm_session(&tmp);

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-pm")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", tmp.path().join("timer.log"))
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let tid = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    let unwritable = tmp.path().join("dry-log-dir");
    std::fs::create_dir(&unwritable).unwrap();

    // The timer fires past the 1-minute deadline; bob never replied, so the board times out and the
    // routing tries to hand the (empty) result back to pm — and cannot.
    let out = nxc(&tmp)
        .env("NXC_NOW", "2026-07-22T00:05:00Z")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &unwritable)
        .args(["--json", "tick", "--thread", &tid])
        .assert()
        .success();

    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["acted"], true, "{v}");
    assert_eq!(v["wake_skipped"]["session"], "s-pm", "{v}");
    assert_eq!(v["wake_skipped"]["reason"], "trigger_failed", "{v}");
    assert!(
        v["wake_skipped"]["detail"]
            .as_str()
            .is_some_and(|d| d.contains("dry log")),
        "the underlying trigger failure travels with the finding: {v}"
    );
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("channel completion wake failed") && stderr.contains("s-pm"),
        "and the breadcrumb stays, for the human reading the terminal: {stderr}"
    );
}

#[test]
fn a_tick_that_wakes_successfully_omits_the_skip_key_entirely() {
    // The companion guard: `wake_skipped` must stay absent on the ordinary path, or its presence
    // says nothing. Same script as the pass_through timeout test above, only with a writable log.
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    write_role(&roles, "bob");
    write_channels_yaml(&roles, "- name: review\n  members: [bob]\n  timeout: 1m\n");
    seed_pm_session(&tmp);

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-pm")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", tmp.path().join("timer.log"))
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let tid = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    let out = nxc(&tmp)
        .env("NXC_NOW", "2026-07-22T00:05:00Z")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "tick", "--thread", &tid])
        .assert()
        .success();

    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["acted"], true, "{v}");
    assert_eq!(v["woke"], "s-pm", "the tick names the session it woke: {v}");
    assert_eq!(
        v["wake_skipped"],
        Value::Null,
        "the key is left out entirely when the hand-off landed: {v}"
    );
    assert_eq!(
        v["advance_failed"],
        Value::Null,
        "and so is the advance finding — no workflow run is involved here at all: {v}"
    );
}

#[test]
fn tick_past_deadline_with_partial_replies_completes_on_timeout_summarize() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    write_role(&roles, "bob");
    write_role(&roles, "carol");
    write_channels_yaml(
        &roles,
        "- name: review\n  members: [bob, carol]\n  timeout: 1m\n  on_complete: summarize\n  \
         summary_prompt: \"Summarize the discussion.\"\n",
    );
    seed_pm_session(&tmp);

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-pm")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", tmp.path().join("timer.log"))
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let tid = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    nxc(&tmp)
        .env("NXC_ACTOR", "bob")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args([
            "reply",
            "--thread",
            &member_thread(&tmp, &tid, "bob"),
            "looks fine",
        ])
        .assert()
        .success();
    assert_eq!(trigger_header_lines(&dry).len(), 2, "fan-out only so far");

    // Timer fires past the 1-minute deadline; carol never replied.
    let out = nxc(&tmp)
        .env("NXC_NOW", "2026-07-22T00:05:00Z")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "tick", "--thread", &tid])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["acted"], true, "{v}");
    assert_eq!(v["outstanding"], serde_json::json!(["local/carol"]), "{v}");

    let lines = trigger_header_lines(&dry);
    assert_eq!(
        lines.len(),
        3,
        "exactly one synthesizer spawned over the partial replies: {lines:?}"
    );
    assert_eq!(field(&lines[2], "role"), "__synth__");
    assert!(
        !lines.iter().any(|l| l.contains("role=pm")),
        "pm must not be woken yet — only phase 1 has run: {lines:?}"
    );

    // A duplicate tick (simulating the timer firing twice, or a leftover job outliving phase 1)
    // must not spawn a second synthesizer — the idempotency gate (§4) catches the phase-1 marker.
    let out = nxc(&tmp)
        .env("NXC_NOW", "2026-07-22T00:05:00Z")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "tick", "--thread", &tid])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["acted"], false, "{v}");
    assert_eq!(v["reason"], "already_handled", "{v}");

    let lines_after = trigger_header_lines(&dry);
    assert_eq!(
        lines_after.len(),
        3,
        "no second synthesizer spawned: {lines_after:?}"
    );
}

// ---- no double-fire: tick after a normal (non-timeout) completion is a no-op --------------------

#[test]
fn tick_after_a_normal_pass_through_completion_is_a_no_op_no_double_fire() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    write_role(&roles, "bob");
    write_role(&roles, "carol");
    // Deliberately no `timeout:` at all — "no timeout involved" (the brief's own wording): this
    // proves `tick`'s idempotency holds even when no timer was ever scheduled for this thread.
    write_channels_yaml(&roles, "- name: review\n  members: [bob, carol]\n");
    seed_pm_session(&tmp);

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-pm")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let tid = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Both members reply before any deadline pressure — the ordinary reply-driven completion path
    // (Task 9, unmodified) fires exactly as it always has.
    nxc(&tmp)
        .env("NXC_ACTOR", "bob")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args([
            "reply",
            "--thread",
            &member_thread(&tmp, &tid, "bob"),
            "lgtm",
        ])
        .assert()
        .success();
    nxc(&tmp)
        .env("NXC_ACTOR", "carol")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args([
            "reply",
            "--thread",
            &member_thread(&tmp, &tid, "carol"),
            "approved",
        ])
        .assert()
        .success();

    let lines = trigger_header_lines(&dry);
    assert_eq!(
        lines.len(),
        3,
        "fan-out (2) + exactly one wake, unmodified Task 9 behavior: {lines:?}"
    );
    assert_eq!(field(&lines[2], "role"), "pm");

    // The (simulated) timer fires anyway, despite the completion having already happened normally.
    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "tick", "--thread", &tid])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["acted"], false, "{v}");
    // The thread is genuinely `complete: true` now (the fix-round redesign: pass_through
    // self-satisfies its own delivered marker instead of emptying `expects` to `[]`), so step 2's
    // ordinary due-check does NOT catch this — the explicit marker check (step 4) does, exactly
    // mirroring `summarize`'s own idempotency mechanism.
    assert_eq!(v["reason"], "already_handled", "{v}");

    let lines_after = trigger_header_lines(&dry);
    assert_eq!(
        lines_after.len(),
        3,
        "no second wake — pass_through's own delivered marker (`expects` retargeted to the \
         qualified DELIVERED_HANDLE identity and immediately self-satisfied) is recognized by \
         tick's idempotency check before any routing is attempted: {lines_after:?}"
    );

    // The regression this fix round exists to prevent: the completed board must stay correctly
    // discoverable via the PULL-based surfaces (opener_wake/inbox/threads show), not vanish from
    // them the moment delivery happens — unlike the earlier ("empty expects") design, which made
    // `complete` read false forever and erased it from exactly these surfaces.
    let show = json_of(
        &nxc(&tmp)
            .args(["--json", "threads", "show", &tid])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(show["complete"], true, "{show}");
    assert_eq!(show["outstanding"], serde_json::json!([]), "{show}");
}

#[test]
fn a_completed_pass_through_board_stays_visible_via_opener_wake_and_threads_show() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    write_role(&roles, "bob");
    // `alice` (the `nxc()` helper's own default `NXC_ACTOR`) is the CLI caller that runs `ask`
    // below — she is the substrate thread OPENER (`ensure_declared_channel` always joins the
    // sender's own qualified identity as a member, alongside the declared `members:`), distinct
    // from `pm`, whose ROLE SESSION is merely the PUSH-wake's return-address target. The PULL-based
    // surfaces below (`threads show`/`threads list`/`prime`'s `threads_you_opened`) are queried as
    // `local/alice`, the actual opener — not `local/pm`, who is never a channel member here at all.
    write_channels_yaml(&roles, "- name: review\n  members: [bob]\n");
    seed_pm_session(&tmp);

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-pm")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let tid = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    nxc(&tmp)
        .env("NXC_ACTOR", "bob")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args([
            "reply",
            "--thread",
            &member_thread(&tmp, &tid, "bob"),
            "lgtm",
        ])
        .assert()
        .success();

    // `threads show` (default consumer = the caller's own ambient identity, `local/alice`, a real
    // member): the CHANNEL thread reads complete, with nothing outstanding, and the self-satisfying
    // delivered-marker message is visible in its raw history (distinctively worded, not confusable
    // with a real reply). Bob's own answer is NOT in this thread any more (nxf 6j6v.pf6j gives him
    // his own), but it reaches the requester all the same: `pass_through` collects the member
    // threads' answers and delivers them, and the marker records that it happened.
    let show = json_of(
        &nxc(&tmp)
            .args(["--json", "threads", "show", &tid])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(show["complete"], true, "{show}");
    assert_eq!(show["outstanding"], serde_json::json!([]), "{show}");
    let messages = show["messages"]
        .as_array()
        .expect("threads show carries a messages array");
    assert!(
        messages
            .iter()
            .any(|m| m["body"] == "[pass_through delivered]"),
        "the self-satisfying marker message must be visible in raw thread history: {show}"
    );
    assert!(
        !messages.iter().any(|m| m["body"] == "lgtm"),
        "bob answers in his OWN thread now — the channel thread has two ends: {show}"
    );
    let bob_board = json_of(
        &nxc(&tmp)
            .args([
                "--json",
                "threads",
                "show",
                &member_thread(&tmp, &tid, "bob"),
            ])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert!(
        bob_board["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .any(|m| m["body"] == "lgtm"),
        "…and it IS in his own thread: {bob_board}"
    );

    // `threads list` (as the opener, `local/alice`): the board is still listed at all, and reports
    // complete — this is the actual regression this fix round exists to prevent (the earlier
    // "empty expects" design made the board vanish from every membership/PULL-based read the
    // instant delivery happened).
    let boards = json_of(
        &nxc(&tmp)
            .args(["--json", "threads", "list"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let board_list = boards.as_array().expect("threads list is a JSON array");
    let board = board_list
        .iter()
        .find(|b| b["thread_id"] == tid)
        .unwrap_or_else(|| panic!("the opener's board must still be listed: {boards}"));
    assert_eq!(board["complete"], true, "{board}");

    // `nxc prime`'s `threads_you_opened` (the real `opener_wake` consumer surface, PULL-based): the
    // opener's board still surfaces as one that FINISHED while it was away — the actual safety net
    // this fix round restores (a PUSH wake that silently failed — `wake_role_requester` never
    // propagates its own failure — would otherwise have left the requester with no way to discover
    // the board completed at all, since the earlier "empty expects" design erased it from this exact
    // surface too).
    //
    // The notice is a WINDOW since nxf 6j6v.2hx9 — what finished since this caller's previous
    // session ended — so the caller needs a previous end for there to be a window at all. Alice is
    // the `nxc()` helper's own actor, and this is the same fixture a real persona gets from its own
    // teardown.
    {
        let mut store = Workspace::resolve(None, tmp.path())
            .unwrap()
            .open_chat_store()
            .unwrap();
        store.create_pending_session("s-alice", "alice").unwrap();
        store
            .mark_session_ended("s-alice", "2026-07-01T00:00:00Z")
            .unwrap();
    }
    let prime = json_of(
        &nxc(&tmp)
            .args(["--json", "prime"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let opened = prime["threads_you_opened"]["complete"]
        .as_array()
        .unwrap_or_else(|| panic!("threads_you_opened.complete must be present: {prime}"));
    assert!(
        opened.iter().any(|t| t["thread_id"] == tid),
        "the completed board must surface in the opener's own prime wake: {prime}"
    );
}

#[test]
fn tick_after_a_normal_summarize_completion_is_a_no_op_no_double_fire() {
    let tmp = workspace();
    let roles = personas_dir(&tmp);
    write_role(&roles, "pm");
    write_role(&roles, "bob");
    write_role(&roles, "carol");
    write_channels_yaml(
        &roles,
        "- name: review\n  members: [bob, carol]\n  on_complete: summarize\n  \
         summary_prompt: \"Summarize the discussion.\"\n",
    );
    seed_pm_session(&tmp);

    let dry = tmp.path().join("dry.log");
    let out = nxc(&tmp)
        .env("NXC_SESSION", "s-pm")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "send", "--to", "review", "please review"])
        .assert()
        .success();
    let tid = json_of(&out.get_output().stdout)["thread_id"]
        .as_str()
        .unwrap()
        .to_string();

    nxc(&tmp)
        .env("NXC_ACTOR", "bob")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args([
            "reply",
            "--thread",
            &member_thread(&tmp, &tid, "bob"),
            "lgtm",
        ])
        .assert()
        .success();
    nxc(&tmp)
        .env("NXC_ACTOR", "carol")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args([
            "reply",
            "--thread",
            &member_thread(&tmp, &tid, "carol"),
            "approved",
        ])
        .assert()
        .success(); // phase 1: spawns the synthesizer
    nxc(&tmp)
        .env("NXC_ACTOR", "__synth__")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["reply", "--thread", &tid, "final synthesized summary"])
        .assert()
        .success(); // phase 2: wakes pm

    let lines = trigger_header_lines(&dry);
    assert_eq!(
        lines.len(),
        4,
        "fan-out (2) + synthesizer (1) + wake (1), unmodified Task 9 behavior: {lines:?}"
    );
    assert_eq!(field(&lines[3], "role"), "pm");

    // The (simulated) timer fires anyway, despite both phases having already completed normally.
    let out = nxc(&tmp)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", &dry)
        .args(["--json", "tick", "--thread", &tid])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["acted"], false, "{v}");
    assert_eq!(v["reason"], "already_handled", "{v}");

    let lines_after = trigger_header_lines(&dry);
    assert_eq!(
        lines_after.len(),
        4,
        "no extra synthesizer spawn, no second wake: {lines_after:?}"
    );
}

// ---- `cancel` primitive: a direct, same-process round trip --------------------------------------

#[test]
fn cancel_succeeds_on_a_handle_schedule_tick_returned_in_the_same_process() {
    let _guard = lock_env();
    let tmp = TempDir::new().unwrap();
    let log = tmp.path().join("timer.log");
    // A direct (non-subprocess) call to the library primitives themselves (nxf ticket 6j6v.z06n
    // §3): `cancel` is only claimed to work "in the SAME process/test" — this proves exactly that,
    // never a cross-process lookup.
    std::env::set_var("NXC_TIMER", "dry");
    std::env::set_var("NXC_TIMER_LOG", &log);

    let handle = nexus_chat::timer::schedule_tick("m-th-direct", "2026-07-22T00:20:00Z")
        .expect("schedule_tick succeeds");
    nexus_chat::timer::cancel(&handle).expect("cancel succeeds on the handle just returned");

    let contents = std::fs::read_to_string(&log).unwrap();
    assert!(
        contents.contains("schedule thread=m-th-direct"),
        "{contents}"
    );
    assert!(
        contents.contains(&format!("cancel handle={}", handle.0)),
        "{contents}"
    );
}
