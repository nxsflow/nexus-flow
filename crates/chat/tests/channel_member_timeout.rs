//! The timeout is PER MEMBER and every transcript write resets it (nxf 6j6v.nf38).
//!
//! Until this item a declared channel's `timeout` produced ONE absolute instant that was stamped on
//! each member thread and then ran stubbornly: a member could be working perfectly — a shell command
//! half an hour into an hour of work — and the deadline struck anyway, because nothing anywhere ever
//! moved it. The owner's own words (6j6v.hq71, 2026-08-12) are that the timeout "gilt pro Mitglied
//! und wird bei jeder Transkript-Ausgabe zurueckgesetzt", and that is what this suite pins:
//!
//! * a member thread carries its OWN deadline, and one member's clock running out settles that
//!   member and nobody else;
//! * every transcript write by the ONE session standing on that thread pushes that member's deadline
//!   out by the declared window again;
//! * the design's §4.5 case — an agent whose shell command runs for an hour against a thirty-minute
//!   window — survives, where a stubborn deadline would have killed something that works.
//!
//! **How time is controlled: `NXC_NOW`, one pinned value per `nxc` invocation.** Nothing sleeps,
//! nothing polls, and no window here is short enough to be raced by a loaded machine — every window
//! is thirty minutes of *declared* time and every observation is a store read at an explicitly
//! chosen instant. `stale` is read straight off [`ChatStore::thread_quorum`] with that instant, which
//! is the same derivation the supervisor's own settle decision uses.
//!
//! **No workflow run appears anywhere in this file.** The only pre-existing thing that reset
//! anything was a RUN's step liveness, and it hangs on the run; this one hangs on the member thread
//! and its one session. `a_members_clock_is_its_own_sessions_and_no_run_is_involved` asserts the
//! absence rather than leaving it to be read off the imports.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

/// When every channel in this suite is opened. Thirty minutes later is `10:30`, which is the
/// deadline a `timeout: 30m` declaration produces and the instant everything here is measured
/// against.
const OPEN: &str = "2026-08-16T10:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

/// An `nxc` invocation whose wall clock is pinned to `now` — the only clock control this suite uses.
///
/// `NXC_TIMER=dry` on EVERY call, not only the ones that assert on scheduling: a channel opens a
/// one-shot job and `workflow tick` re-arms it, and an unset `NXC_TIMER` means the real `at`
/// subprocess (`TimerConfig::from_ambient`). A test suite must not shell out to the host's scheduler
/// to find out what this engine decided.
fn nxc(tmp: &TempDir, now: &str) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", now)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"))
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", tmp.path().join("timer.log"));
    c
}

/// Every one-shot job this workspace has scheduled, in order — `DryTimer`'s record of what a real
/// `at` would have been asked to run.
fn timer_lines(tmp: &TempDir) -> Vec<String> {
    std::fs::read_to_string(tmp.path().join("timer.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim())
        .expect("valid json")
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// The declared roles (always including the requester `pm`) plus the channel file, and the
/// requester's own bound session — without a return address a consolidation has nobody to wake, and
/// half of what this suite asserts is whether the requester was woken or left alone.
fn write_declarations(tmp: &TempDir, handles: &[&str], channels: &str) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in handles.iter().chain(std::iter::once(&"pm")) {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
        )
        .unwrap();
    }
    std::fs::write(roles.join("channels.yaml"), channels).unwrap();

    let mut store = open_store(tmp);
    store.create_pending_session("s-pm", "pm").unwrap();
    drop(store);
    nxc(tmp, OPEN)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();
}

/// Every dry-log line that opens a trigger entry (the message field embeds real newlines, so only
/// the first physical line of an entry starts with `trigger role=`).
fn trigger_lines(tmp: &TempDir) -> Vec<String> {
    std::fs::read_to_string(tmp.path().join("dry.log"))
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("trigger role="))
        .map(str::to_string)
        .collect()
}

/// The whole dry log — a trigger's message embeds newlines, so a delivered answer lives on the
/// lines AFTER the `trigger role=` header, not in it.
fn dry_log(tmp: &TempDir) -> String {
    std::fs::read_to_string(tmp.path().join("dry.log")).unwrap_or_default()
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

/// The internal session the fan-out started `handle` on — read off the dry log rather than derived,
/// so the session this suite writes a transcript for is provably the one the engine put to work.
fn session_of(tmp: &TempDir, handle: &str) -> String {
    let want = format!("trigger role={handle} ");
    let line = trigger_lines(tmp)
        .into_iter()
        .find(|l| l.starts_with(&want))
        .unwrap_or_else(|| panic!("no trigger for {handle}: {:?}", trigger_lines(tmp)));
    field(&line, "session")
}

/// The member thread `handle` answers on, under `channel_thread` (nxf 6j6v.pf6j).
fn member_thread(tmp: &TempDir, channel_thread: &str, handle: &str) -> String {
    let store = open_store(tmp);
    let want = format!("local/{handle}");
    store
        .supervised_children(channel_thread)
        .unwrap()
        .into_iter()
        .find(|t| {
            store
                .thread_quorum(t, OPEN)
                .unwrap()
                .is_some_and(|q| q.expects == [want.clone()])
        })
        .unwrap_or_else(|| panic!("no member thread for {handle} under {channel_thread}"))
}

/// Is this thread past its deadline, read at `now`? The same derivation the supervisor's own
/// "is the set settled" decision reads.
fn stale_at(tmp: &TempDir, thread: &str, now: &str) -> bool {
    open_store(tmp)
        .thread_quorum(thread, now)
        .unwrap()
        .unwrap_or_else(|| panic!("no such thread: {thread}"))
        .stale
}

/// The deadline a thread currently carries, read at `now` (the flag above is derived from it).
fn deadline_at(tmp: &TempDir, thread: &str, now: &str) -> Option<String> {
    open_store(tmp)
        .thread_quorum(thread, now)
        .unwrap()
        .unwrap_or_else(|| panic!("no such thread: {thread}"))
        .deadline
}

/// One transcript flush by `session`, at `now` — exactly what the sidecar does every 32 entries
/// (`nxc transcript append --session <internal>`, JSON-lines on stdin).
fn flush_transcript(tmp: &TempDir, now: &str, session: &str, entries: &[Value]) {
    let body = entries
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    nxc(tmp, now)
        .args(["transcript", "append", "--session", session])
        .write_stdin(body)
        .assert()
        .success();
}

/// The one entry a session emits when it starts a shell command — the §4.5 case's own first move.
fn shell_entry(command: &str) -> Value {
    serde_json::json!({
        "kind": "tool_use",
        "data": { "name": "Bash", "input": { "command": command } }
    })
}

/// Open `channel` at [`OPEN`] as the requester `pm` and return the CHANNEL thread's id.
fn open_channel(tmp: &TempDir, channel: &str) -> String {
    json_of(
        nxc(tmp, OPEN)
            .env("NXC_ACTOR", "pm")
            .env("NXC_SESSION", "s-pm")
            .args(["--json", "send", "--to", channel, "please look at this"]),
    )["thread_id"]
        .as_str()
        .expect("thread_id")
        .to_string()
}

fn count(tmp: &TempDir, sql: &str) -> i64 {
    open_store(tmp)
        .connection()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

// ---- acceptance 1: the deadline is PER MEMBER --------------------------------------------------

#[test]
fn each_member_runs_its_own_clock_and_one_running_out_does_not_end_the_others() {
    // Two members, one declared window, one channel. `reviewer-a` is working (its session writes to
    // its transcript); `reviewer-b` never shows up at all. Past the declared window, b's clock has
    // run out and a's has not — and, the half that actually matters, b timing out does NOT settle
    // the set, so the channel does not hand the requester an answer while a is still working.
    let tmp = workspace();
    write_declarations(
        &tmp,
        &["reviewer-a", "reviewer-b"],
        "- name: review\n  members: [reviewer-a, reviewer-b]\n  timeout: 30m\n",
    );
    let channel_thread = open_channel(&tmp, "review");
    let a_thread = member_thread(&tmp, &channel_thread, "reviewer-a");
    let b_thread = member_thread(&tmp, &channel_thread, "reviewer-b");
    let a_session = session_of(&tmp, "reviewer-a");

    // a is alive at 10:20 — ten minutes before the declared window would have struck.
    flush_transcript(
        &tmp,
        "2026-08-16T10:20:00Z",
        &a_session,
        &[shell_entry("cargo test --all")],
    );

    // 10:35: past the ONE deadline the declaration produced, and the two members answer differently.
    let now = "2026-08-16T10:35:00Z";
    assert!(
        !stale_at(&tmp, &a_thread, now),
        "reviewer-a wrote to its transcript at 10:20, so its own clock restarted there: {:?}",
        deadline_at(&tmp, &a_thread, now)
    );
    assert!(
        stale_at(&tmp, &b_thread, now),
        "reviewer-b never showed up, so ITS clock ran out: {:?}",
        deadline_at(&tmp, &b_thread, now)
    );

    // …and b's clock running out did not end a's turn: nothing was consolidated.
    let tick = json_of(nxc(&tmp, now).args(["--json", "tick", "--thread", &channel_thread]));
    assert_eq!(
        tick["acted"], false,
        "one member timing out must not settle the SET while another is still working: {tick}"
    );
    assert_eq!(
        trigger_lines(&tmp).len(),
        2,
        "still just the fan-out — the requester was not woken: {:?}",
        trigger_lines(&tmp)
    );

    // a answers at 10:40; NOW the set is settled (a complete, b timed out) and the requester gets
    // a's answer — which is the whole point of not having killed a at 10:30.
    nxc(&tmp, "2026-08-16T10:40:00Z")
        .env("NXC_ACTOR", "reviewer-a")
        .env("NXC_SESSION", &a_session)
        .args(["reply", "--thread", &a_thread, "the build is green"])
        .assert()
        .success();
    let lines = trigger_lines(&tmp);
    assert_eq!(
        lines.len(),
        3,
        "the consolidation woke the requester exactly once: {lines:?}"
    );
    assert_eq!(field(&lines[2], "role"), "pm", "{lines:?}");
    let log = dry_log(&tmp);
    assert!(
        log.contains("<message from=\"local/reviewer-a\">\nthe build is green\n</message>"),
        "and it carries the answer of the member that was still working at 10:30: {log}"
    );
    assert!(
        !log.contains("from=\"local/reviewer-b\""),
        "while the member that really did run out of time is simply absent: {log}"
    );
}

// ---- acceptance 2: every transcript write RESETS that member's deadline ------------------------

#[test]
fn a_transcript_write_pushes_the_members_deadline_out_and_a_silent_transcript_does_not() {
    // The controlled comparison: two channels, the SAME declaration, the SAME clock, the same
    // window — and the only difference between them is that one member's session wrote to its
    // transcript. `noisy` is not timed out past the declared window; `silent` is.
    //
    // The second half is deliberate and is the honest bound of this item: a session that emits
    // NOTHING for a whole window IS timed out. Resetting the clock is one of the two mechanisms the
    // design's §4.5 asks for; the other — knocking (reading the transcript and reporting what it
    // shows) before striking — is nxf 6j6v.1mtn and is not built here.
    let tmp = workspace();
    write_declarations(
        &tmp,
        &["noisy", "silent"],
        "- name: busy\n  members: [noisy]\n  timeout: 30m\n\
         - name: quiet\n  members: [silent]\n  timeout: 30m\n",
    );
    let busy = open_channel(&tmp, "busy");
    let quiet = open_channel(&tmp, "quiet");
    let noisy_thread = member_thread(&tmp, &busy, "noisy");
    let silent_thread = member_thread(&tmp, &quiet, "silent");

    assert_eq!(
        deadline_at(&tmp, &noisy_thread, OPEN),
        deadline_at(&tmp, &silent_thread, OPEN),
        "both start from the same declared window"
    );
    assert_eq!(
        deadline_at(&tmp, &noisy_thread, OPEN).as_deref(),
        Some("2026-08-16T10:30:00Z"),
        "`timeout: 30m` opened at 10:00 falls due at 10:30"
    );

    flush_transcript(
        &tmp,
        "2026-08-16T10:25:00Z",
        &session_of(&tmp, "noisy"),
        &[serde_json::json!({"kind": "assistant", "data": {"text": "still reading the diff"}})],
    );

    assert_eq!(
        deadline_at(&tmp, &noisy_thread, OPEN).as_deref(),
        Some("2026-08-16T10:55:00Z"),
        "the write at 10:25 restarted the declared window from there"
    );
    let now = "2026-08-16T10:45:00Z";
    assert!(
        !stale_at(&tmp, &noisy_thread, now),
        "a member whose transcript keeps moving is not timed out past the declared window"
    );
    assert!(
        stale_at(&tmp, &silent_thread, now),
        "the same declaration, the same clock, a silent transcript — this one IS"
    );
}

// ---- acceptance 3: the design's §4.5 case ------------------------------------------------------

#[test]
fn an_hour_long_shell_command_outlives_a_thirty_minute_window_instead_of_being_killed_by_it() {
    // Entwurf §4.5, verbatim: "ein Agent startet einen Shell-Befehl, der eine Stunde laeuft. Die
    // Shell ist gesund, im Transkript passiert nichts. Eine stur laufende Frist von dreissig Minuten
    // wuerde hier etwas kaputtmachen, das funktioniert."
    //
    // The member starts `cargo test --all` at 10:05 and is still at it an hour after the channel
    // opened — TWICE the declared window. A stubbornly running thirty-minute deadline killed it at
    // 10:30 and handed the requester a consolidation without its answer. Here it is alive at 11:00,
    // nothing has been consolidated behind its back, and its result at 11:02 is what completes the
    // channel.
    let tmp = workspace();
    write_declarations(
        &tmp,
        &["builder"],
        "- name: build\n  members: [builder]\n  timeout: 30m\n",
    );
    let channel_thread = open_channel(&tmp, "build");
    let member = member_thread(&tmp, &channel_thread, "builder");
    let session = session_of(&tmp, "builder");

    flush_transcript(
        &tmp,
        "2026-08-16T10:05:00Z",
        &session,
        &[shell_entry("cargo test --all --release")],
    );
    flush_transcript(
        &tmp,
        "2026-08-16T10:28:00Z",
        &session,
        &[shell_entry("tail -n 5 build.log")],
    );
    flush_transcript(
        &tmp,
        "2026-08-16T10:50:00Z",
        &session,
        &[shell_entry("tail -n 5 build.log")],
    );

    let hour_in = "2026-08-16T11:00:00Z";
    assert!(
        !stale_at(&tmp, &member, hour_in),
        "an hour in — twice the declared window — the shell is healthy and so is the member: {:?}",
        deadline_at(&tmp, &member, hour_in)
    );
    let tick = json_of(nxc(&tmp, hour_in).args(["--json", "tick", "--thread", &channel_thread]));
    assert_eq!(
        tick["acted"], false,
        "and nothing was delivered to the requester behind its back: {tick}"
    );

    // The command finishes and the member answers — an hour and two minutes after the open.
    nxc(&tmp, "2026-08-16T11:02:00Z")
        .env("NXC_ACTOR", "builder")
        .env("NXC_SESSION", &session)
        .args(["reply", "--thread", &member, "3047 passed, 0 failed"])
        .assert()
        .success();
    let lines = trigger_lines(&tmp);
    assert_eq!(
        lines.len(),
        2,
        "fan-out, then the requester's wake: {lines:?}"
    );
    assert_eq!(field(&lines[1], "role"), "pm", "{lines:?}");
    assert!(
        dry_log(&tmp)
            .contains("<message from=\"local/builder\">\n3047 passed, 0 failed\n</message>"),
        "the hour of work reaches the requester instead of being timed out: {}",
        dry_log(&tmp)
    );
}

// ---- acceptance 4: the mechanism is the member thread's one session, never a run ---------------

#[test]
fn a_members_clock_is_its_own_sessions_and_hangs_on_nothing_above_it() {
    // Two claims in one shape. (1) The reset is keyed on the ONE session standing on the member
    // thread: a sibling member's transcript write moves the sibling's clock and NOT this one's, so
    // nothing here is per-channel or per-workspace. (2) No workflow run, and no armed step-liveness
    // window, exists anywhere in this workspace at any point — the only thing that reset anything
    // before this item was a RUN's step liveness, and it is provably not what is running here.
    let tmp = workspace();
    write_declarations(
        &tmp,
        &["reviewer-a", "reviewer-b"],
        "- name: review\n  members: [reviewer-a, reviewer-b]\n  timeout: 30m\n",
    );
    let channel_thread = open_channel(&tmp, "review");
    let a_thread = member_thread(&tmp, &channel_thread, "reviewer-a");
    let b_thread = member_thread(&tmp, &channel_thread, "reviewer-b");

    flush_transcript(
        &tmp,
        "2026-08-16T10:20:00Z",
        &session_of(&tmp, "reviewer-a"),
        &[serde_json::json!({"kind": "thinking", "data": {"text": "reading"}})],
    );

    assert_eq!(
        deadline_at(&tmp, &a_thread, OPEN).as_deref(),
        Some("2026-08-16T10:50:00Z"),
        "the session that wrote is the one whose clock moved"
    );
    assert_eq!(
        deadline_at(&tmp, &b_thread, OPEN).as_deref(),
        Some("2026-08-16T10:30:00Z"),
        "its sibling's clock is untouched — the deadline hangs on the member thread, not the channel"
    );

    // Two assertions stood here — that no `workflow_runs` row and no `workflow_step_liveness`
    // window existed, which is how this test used to say "this clock is the member's own and hangs
    // on nothing above it". 6j6v.dvyq §3 removed both tables, so the claim is now a fact about the
    // schema rather than about this fixture, and `channel_flow.rs` is where it is asserted.
}

// ---- acceptance 5: a declared window that cannot be parsed is refused --------------------------

#[test]
fn an_unparseable_declared_timeout_is_refused_by_name_and_nothing_is_persisted() {
    // `ChannelDecl.timeout` is stored opaquely by the loader. Somebody has to decide what it means,
    // and the answer must never be a silent "then there is no cap" — a typo'd window that quietly
    // means "wait forever" is the failure nobody notices.
    let tmp = workspace();
    write_declarations(
        &tmp,
        &["reviewer-a"],
        "- name: review\n  members: [reviewer-a]\n  timeout: twenty-minutes\n",
    );
    let out = nxc(&tmp, OPEN)
        .args(["--json", "send", "--to", "review", "please look at this"])
        .assert()
        .failure();
    let v: Value =
        serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim()).unwrap();
    assert_eq!(v["error"]["kind"], "validation", "{v}");

    // **What the PREFLIGHT actually saves, asserted FIRST because it is the discriminating half.**
    // `facade::ask` rejects the same unreadable string on its own, so `threads == 0` holds whether or
    // not the preflight exists — a check that cannot fail is not evidence, and the first version of
    // this test asserted only that. What ONLY the preflight prevents is `ensure_declared_channel`,
    // which runs before `ask` and emits `set_channel_field` + `add_member` ops: without it, a refused
    // `send` leaves a materialised `decl:review` channel behind. These two run before the message
    // assertions so that removing the preflight fails on the thing the preflight is for.
    assert_eq!(
        count(
            &tmp,
            "SELECT COUNT(*) FROM channels WHERE channel_id = 'decl:review'"
        ),
        0,
        "the channel was never materialised — the preflight ran before `ensure_declared_channel`"
    );
    assert_eq!(
        count(&tmp, "SELECT COUNT(*) FROM ops"),
        0,
        "and not one op was emitted by the refused call"
    );
    assert_eq!(count(&tmp, "SELECT COUNT(*) FROM threads"), 0);

    let msg = v["error"]["msg"].as_str().unwrap_or_default();
    assert!(
        msg.contains("twenty-minutes"),
        "the error names the value that could not be read: {v}"
    );
    assert!(
        msg.contains("declared `timeout`"),
        "…and the field the author actually wrote, not a `--deadline` flag nobody typed: {v}"
    );
}

// ---- the carried finding: a re-declared turn must not be born past its deadline ----------------

#[test]
fn a_second_turn_starts_every_members_clock_again_instead_of_inheriting_one_that_already_struck() {
    // Carried from nxf 6j6v.cg8g (Task 1) as a deferred minor, and it lands here because this item
    // is the one that rebuilds the deadline per member: a re-declared thread whose OLD deadline has
    // passed reads `stale` from the instant its new turn is declared, and `stale` is what the
    // supervisor's settle decision and `workflow tick` both treat as "due". Turn two would have been
    // born finished — consolidated with nobody's answer in it.
    let tmp = workspace();
    write_declarations(
        &tmp,
        &["reviewer-a"],
        "- name: review\n  members: [reviewer-a]\n  timeout: 30m\n",
    );
    let channel_thread = open_channel(&tmp, "review");
    let member = member_thread(&tmp, &channel_thread, "reviewer-a");
    let session = session_of(&tmp, "reviewer-a");

    nxc(&tmp, "2026-08-16T10:10:00Z")
        .env("NXC_ACTOR", "reviewer-a")
        .env("NXC_SESSION", &session)
        .args(["reply", "--thread", &member, "turn one done"])
        .assert()
        .success();

    // The requester asks again at 11:00 — long past turn one's 10:30 deadline.
    let second_turn = "2026-08-16T11:00:00Z";
    nxc(&tmp, second_turn)
        .env("NXC_ACTOR", "pm")
        .env("NXC_SESSION", "s-pm")
        .args([
            "reply",
            "--thread",
            &channel_thread,
            "and now the other half",
        ])
        .assert()
        .success();

    assert_eq!(
        deadline_at(&tmp, &member, second_turn).as_deref(),
        Some("2026-08-16T11:30:00Z"),
        "the new turn gets the declared window from ITS start, not turn one's spent one"
    );
    assert!(
        !stale_at(&tmp, &member, second_turn),
        "a member that was just asked has not run out of time yet"
    );
    let tick = json_of(nxc(&tmp, "2026-08-16T11:05:00Z").args([
        "--json",
        "tick",
        "--thread",
        &channel_thread,
    ]));
    assert_eq!(
        tick["acted"], false,
        "so nothing consolidates a turn that nobody has answered: {tick}"
    );
}

// ---- the rule the whole design rests on: a DURATION resets, an INSTANT does not -----------------

#[test]
fn a_deadline_given_as_an_absolute_instant_arms_no_clock_and_no_transcript_moves_it() {
    // The decided rule (`facade::DeadlineSpec`): the argument's FORM decides its meaning. A
    // `<n><unit>` duration declares a WINDOW, which is resettable because a reset has a length to
    // reset BY; an RFC3339 instant declares a point in time, which nothing should move — a requester
    // who named 15:00 meant 15:00. Four places in the source state that rule and until this test
    // nothing would have failed if `resolve_deadline_spec` had started handing back a window for an
    // instant.
    let tmp = workspace();
    // DECLARED, not passed. `send --deadline` was the per-call override until nxf 6j6v.ckeq removed
    // it (a board's window is the channel's own answer, not the caller's), and the rule under test
    // is untouched by that: `timeout:` is parsed by the very same `facade::resolve_deadline_spec`
    // the flag used, so an absolute instant is still expressible — at the declaration, which is
    // where a deadline lives now.
    write_declarations(
        &tmp,
        &["reviewer-a"],
        "- name: review\n  members: [reviewer-a]\n  timeout: 2026-08-16T10:30:00Z\n",
    );
    let channel_thread = json_of(
        nxc(&tmp, OPEN)
            .env("NXC_ACTOR", "pm")
            .env("NXC_SESSION", "s-pm")
            .args([
                "--json",
                "send",
                "--to",
                "review",
                "please look at this",
                "--no-ref",
            ]),
    )["thread_id"]
        .as_str()
        .expect("thread_id")
        .to_string();
    let member = member_thread(&tmp, &channel_thread, "reviewer-a");
    let session = session_of(&tmp, "reviewer-a");

    assert_eq!(
        deadline_at(&tmp, &member, OPEN).as_deref(),
        Some("2026-08-16T10:30:00Z"),
        "the instant the CHANNEL declared is the member's deadline"
    );

    // A sign of life at 10:25 — the write that WOULD restart a declared window.
    flush_transcript(
        &tmp,
        "2026-08-16T10:25:00Z",
        &session,
        &[shell_entry("cargo test --all")],
    );

    assert_eq!(
        deadline_at(&tmp, &member, OPEN).as_deref(),
        Some("2026-08-16T10:30:00Z"),
        "an absolute instant is not a window, so nothing restarts it"
    );
    assert!(
        stale_at(&tmp, &member, "2026-08-16T10:31:00Z"),
        "and it strikes at the instant that was named, working member or not"
    );
    assert_eq!(
        count(&tmp, "SELECT COUNT(*) FROM channel_member_deadline"),
        0,
        "no clock was armed at all — the absence is the mechanism, not a special case downstream"
    );
}

// ---- the automatic strike survives a member that showed a sign of life -------------------------

#[test]
fn a_tick_that_declines_re_arms_itself_so_a_one_member_channel_is_still_struck() {
    // An `at` job fires exactly ONCE. A member's window moves, so the instant the job was scheduled
    // for stops being the instant anything falls due — and a decline that scheduled nothing would
    // spend the whole automatic strike on the first sign of life a member gave. On a one-member
    // channel, the commonest shape there is, that means the requester waits forever for a member
    // that wrote one line and then hung.
    let tmp = workspace();
    write_declarations(
        &tmp,
        &["builder"],
        "- name: build\n  members: [builder]\n  timeout: 30m\n",
    );
    let channel_thread = open_channel(&tmp, "build");
    let member = member_thread(&tmp, &channel_thread, "builder");
    let session = session_of(&tmp, "builder");

    assert_eq!(
        timer_lines(&tmp).len(),
        1,
        "the fan-out scheduled one job, for 10:30: {:?}",
        timer_lines(&tmp)
    );
    assert!(
        timer_lines(&tmp)[0].contains("deadline=2026-08-16T10:30:00Z"),
        "{:?}",
        timer_lines(&tmp)
    );

    // The member writes one line at 10:25 and then hangs. Its window now runs to 10:55.
    flush_transcript(
        &tmp,
        "2026-08-16T10:25:00Z",
        &session,
        &[shell_entry("cargo test --all")],
    );

    // 10:30: the scheduled job fires. It correctly declines — and schedules its own successor.
    let tick = json_of(nxc(&tmp, "2026-08-16T10:30:00Z").args([
        "--json",
        "tick",
        "--thread",
        &channel_thread,
    ]));
    assert_eq!(tick["acted"], false, "the member is alive: {tick}");

    let lines = timer_lines(&tmp);
    assert_eq!(
        lines.len(),
        2,
        "the declining tick armed the next one: {lines:?}"
    );
    assert!(
        lines[1].contains("deadline=2026-08-16T10:55:00Z"),
        "…for the instant the member's own clock now falls due at: {lines:?}"
    );
    assert!(
        lines[1].contains(&format!("command=nxc tick --thread {channel_thread}")),
        "…and it is the same job, on the same thread: {lines:?}"
    );

    // 10:55: the successor fires. The member has been silent since 10:25, so NOW it strikes and the
    // requester is handed what there is — which is the promise `changes/channel-member-timeout.md`
    // makes and the thing that would have been lost without the re-arm.
    let tick = json_of(nxc(&tmp, "2026-08-16T10:55:01Z").args([
        "--json",
        "tick",
        "--thread",
        &channel_thread,
    ]));
    assert_eq!(
        tick["acted"], true,
        "silent for the whole window: the automatic strike still happens: {tick}"
    );
    assert_eq!(
        tick["outstanding"],
        serde_json::json!(["local/builder"]),
        "…naming who never responded: {tick}"
    );
    assert!(
        stale_at(&tmp, &member, "2026-08-16T10:55:01Z"),
        "and the member really was past its restarted window"
    );
}

// ---- a member parked behind the working tree ---------------------------------------------------

#[test]
fn a_member_that_waits_behind_the_working_tree_starts_its_window_when_it_actually_starts() {
    // The clock is armed at fan-out, when the requester starts waiting — but a trigger parked behind
    // the working-tree lease has run nothing and produces no transcript to restart itself with. If
    // the queue outlasts the window the member is `stale`, the SET reads settled, the channel
    // consolidates WITHOUT it, and only then does the trigger fire: the member works for an answer
    // nobody will collect.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    // `exclusive` is what makes a second chain WAIT rather than run beside the first.
    for handle in ["pm", "holder", "builder"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!(
                "handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n\
                 working_tree: exclusive\n"
            ),
        )
        .unwrap();
    }
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: build\n  members: [builder]\n  timeout: 30m\n  working_tree: exclusive\n",
    )
    .unwrap();
    let mut store = open_store(&tmp);
    store.create_pending_session("s-pm", "pm").unwrap();
    drop(store);
    nxc(&tmp, OPEN)
        .args(["session", "bind", "s-pm", "real-pm"])
        .assert()
        .success();

    // `holder` takes the working tree first and keeps it.
    nxc(&tmp, OPEN)
        .env("NXC_ACTOR", "pm")
        .env("NXC_SESSION", "s-pm")
        .args(["--json", "send", "--to", "holder", "hold the tree"])
        .assert()
        .success();

    // The channel opens while the tree is held, so its member is QUEUED, not started.
    let channel_thread = open_channel(&tmp, "build");
    let member = member_thread(&tmp, &channel_thread, "builder");
    assert_eq!(
        count(&tmp, "SELECT COUNT(*) FROM working_tree_queue"),
        1,
        "the member is parked behind the lease, not running"
    );
    assert_eq!(
        deadline_at(&tmp, &member, OPEN).as_deref(),
        Some("2026-08-16T10:30:00Z"),
        "its window is armed from the moment the requester started waiting"
    );

    // The holder finishes at 11:00 — an hour later, long past the member's window. Releasing the
    // lease fires the queued trigger, and the member's window starts THERE.
    let holder_thread = open_store(&tmp)
        .thread_assignee_sessions()
        .unwrap()
        .into_iter()
        .find(|(t, _)| t != &member && t != &channel_thread)
        .map(|(t, _)| t)
        .expect("the holder's own thread");
    let holder_session = session_of(&tmp, "holder");
    nxc(&tmp, "2026-08-16T11:00:00Z")
        .env("NXC_ACTOR", "holder")
        .env("NXC_SESSION", &holder_session)
        .args(["reply", "--thread", &holder_thread, "tree released"])
        .assert()
        .success();

    assert_eq!(
        count(&tmp, "SELECT COUNT(*) FROM working_tree_queue"),
        0,
        "the queued member fired"
    );
    assert_eq!(
        deadline_at(&tmp, &member, "2026-08-16T11:00:00Z").as_deref(),
        Some("2026-08-16T11:30:00Z"),
        "and its window restarted when it actually started, not when it was asked"
    );
    assert!(
        !stale_at(&tmp, &member, "2026-08-16T11:00:00Z"),
        "a member that has just been put to work has not run out of time"
    );
}

// ---- turn two's strike is automatic too --------------------------------------------------------

#[test]
fn a_second_turn_that_goes_silent_is_struck_by_a_job_the_hand_out_scheduled() {
    // Turn ONE's automatic strike is the fan-out's one-shot job, moved along by every declining
    // tick. By the time a SECOND turn is handed out that chain is over: the set settled, so the job
    // either fired and acted or was overtaken by the synchronous `reply` path, and an `at` job fires
    // exactly once. So the hand-out that restarts every member's clock has to schedule the job that
    // watches it — otherwise turn two falls back to the state before this item, where the strike
    // happens only if somebody else happens to ask.
    let tmp = workspace();
    write_declarations(
        &tmp,
        &["builder"],
        "- name: build\n  members: [builder]\n  timeout: 30m\n",
    );
    let channel_thread = open_channel(&tmp, "build");
    let member = member_thread(&tmp, &channel_thread, "builder");
    let session = session_of(&tmp, "builder");
    assert_eq!(
        timer_lines(&tmp).len(),
        1,
        "turn one's own job, for 10:30: {:?}",
        timer_lines(&tmp)
    );

    // Turn one is answered at 10:10. The set settles, the channel consolidates synchronously, and
    // turn one's chain has nothing left to do.
    nxc(&tmp, "2026-08-16T10:10:00Z")
        .env("NXC_ACTOR", "builder")
        .env("NXC_SESSION", &session)
        .args(["reply", "--thread", &member, "turn one done"])
        .assert()
        .success();

    // 11:00: the requester asks again — the turn-two hand-out.
    let second_turn = "2026-08-16T11:00:00Z";
    nxc(&tmp, second_turn)
        .env("NXC_ACTOR", "pm")
        .env("NXC_SESSION", "s-pm")
        .args([
            "reply",
            "--thread",
            &channel_thread,
            "and now the other half",
        ])
        .assert()
        .success();
    assert_eq!(
        deadline_at(&tmp, &member, second_turn).as_deref(),
        Some("2026-08-16T11:30:00Z"),
        "the restart itself is turn one's fix and is asserted elsewhere; it is the premise here"
    );

    let lines = timer_lines(&tmp);
    assert_eq!(
        lines.len(),
        2,
        "the hand-out scheduled turn two's own strike: {lines:?}"
    );
    assert!(
        lines[1].contains("deadline=2026-08-16T11:30:00Z"),
        "…for the instant turn two's restarted window falls due at: {lines:?}"
    );
    assert!(
        lines[1].contains(&format!("command=nxc tick --thread {channel_thread}")),
        "…and it is the same job, on the same thread: {lines:?}"
    );

    // The member never writes a line and nobody else asks. The scheduled job is the only thing that
    // runs, and it is what strikes.
    let tick = json_of(nxc(&tmp, "2026-08-16T11:30:01Z").args([
        "--json",
        "tick",
        "--thread",
        &channel_thread,
    ]));
    assert_eq!(
        tick["acted"], true,
        "silent for the whole second window: the automatic strike still happens: {tick}"
    );
    assert_eq!(
        tick["outstanding"],
        serde_json::json!(["local/builder"]),
        "…naming who never answered turn two: {tick}"
    );
}
