//! The new `nxc` surface from the COMMAND LINE (nxf 6j6v.p6m1) — the ticket's own acceptance.
//!
//! The five steps, end to end, as a human types them:
//!
//! 1. `nxc prime` with no persona, and the caller sees who is there and what for;
//! 2. `nxc send --to pm "…"` opens a thread with a workspace-unique id and starts the persona;
//! 3. that persona's own `nxc prime` tells it who it is and that it answers with `reply --thread`;
//! 4. it answers, the human answers back, several turns;
//! 5. `--stream` lets the human watch — and is refused to a persona.
//!
//! The worker is the dry one: what is under test is the surface and the routing, not the sidecar.
//! Where a real spawn matters (that a trigger happened at all, and against which session) the dry
//! worker's log is the evidence, exactly as the older suites use it.

use assert_cmd::Command;
use nexus_chat::workspace::ChatWorkspaceExt;
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-08-05T10:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config())
        .expect("seed chat workspace");
    tmp
}

/// The human at the keyboard: no spawned-context stamps at all.
fn human(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env_remove("NXC_WORKER")
        .env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
    c
}

/// A spawned persona session: the stamps `SidecarWorker` puts into a triggered role's environment.
fn persona(tmp: &TempDir, handle: &str, session: &str) -> Command {
    let mut c = human(tmp);
    c.env("NXC_ACTOR", handle).env("NXC_SESSION", session);
    c
}

fn write_roles(tmp: &TempDir) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\njob_title: Product manager\njob_description: turns intent into work orders\n\
         stage: senior\nsystem_prompt: You are the PM.\n\
         address_book:\n  - to: coder\n    why: hand over an implementable work order\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("coder.yaml"),
        "handle: coder\njob_title: Implementer\nsystem_prompt: You are the coder.\n",
    )
    .unwrap();
}

fn stdout_of(cmd: &mut Command) -> String {
    let out = cmd.assert().success();
    String::from_utf8_lossy(&out.get_output().stdout).into_owned()
}

fn json_of(cmd: &mut Command) -> Value {
    serde_json::from_str(stdout_of(cmd).trim()).expect("valid json")
}

/// The receipt on stdout of an invocation that DOES fail (non-zero exit) — nxf 6j6v.hpv8's shape:
/// the JSON body still carries the receipt (`message_id`, `warnings`, …), the exit code is what a
/// script watching only that still notices. `stdout_of`/`json_of` assert `.success()` and so cannot
/// read this case's stdout at all.
fn json_of_failure(cmd: &mut Command) -> Value {
    let out = cmd.assert().failure();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    serde_json::from_str(stdout.trim()).expect("a receipt, not just an error envelope")
}

/// A failing invocation's stderr — `assert_cmd`'s own predicates crate is not a dev-dependency
/// here, and the suites next door read the stream directly for the same reason.
fn failure_stderr(cmd: &mut Command) -> String {
    let out = cmd.assert().failure();
    String::from_utf8_lossy(&out.get_output().stderr).into_owned()
}

fn dry_log(tmp: &TempDir) -> String {
    std::fs::read_to_string(tmp.path().join("dry.log")).unwrap_or_default()
}

/// Re-declare a thread's expected set through the SEAM (`facade::set_expects`), against the same
/// workspace db the `nxc` subprocesses in this suite use — the supervisor's own instrument, and
/// since 6j6v.dvyq removed `nxc threads expect` the only way to make this write at all. Opener-only,
/// so `actor` must be the handle that opened the thread. The store handle is dropped before
/// returning, so the next `nxc` invocation opens the file cleanly.
fn set_expects(
    tmp: &TempDir,
    actor: &str,
    thread: &str,
    expects: &[String],
) -> nexus_chat::store::ThreadQuorum {
    let mut store = nexus_chat::workspace::Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store");
    nexus_chat::facade::set_expects(&mut store, NOW, actor, thread, expects)
        .expect("the opener may re-declare")
}

// ---- step 1: a human primes and sees who is there ---------------------------------------------

#[test]
fn a_human_primes_and_is_shown_who_can_be_addressed_and_what_for() {
    let tmp = workspace();
    write_roles(&tmp);
    let out = stdout_of(human(&tmp).args(["prime", "--consumer", "local/carsten"]));
    assert!(out.contains("## Who you can address"), "{out}");
    assert!(
        out.contains(
            "**Product manager** (handle: `pm`, level: senior) — turns intent into work orders"
        ),
        "who they are, the handle to type, and what they are for — the step's own words \
         (re-cut in 6j6v.frek: the invocation is stated once in the header, not per line):\n{out}"
    );
    assert!(
        out.contains("Address any of them the same way: `nxc send --to <handle>"),
        "and the way to address any of them, said once:\n{out}"
    );
    assert!(
        !out.contains("## You are"),
        "a human is not told it is a persona:\n{out}"
    );
}

#[test]
fn list_renders_the_same_directory_a_prime_does() {
    // frek acceptance 1–3 are about `nxc list`, and acceptance 5 says `prime` must render the
    // IDENTICAL view. Both go through `Directory::render_markdown`, but nothing asserted that the
    // `list` COMMAND prints it — every other `list` test in this crate passes `--json`, so the
    // human output was argued structurally and tested nowhere. Substring, not equality: `prime`
    // wraps the same block in a session-start document.
    let tmp = workspace();
    write_roles(&tmp);
    let listed = stdout_of(human(&tmp).args(["list"]));
    let primed = stdout_of(human(&tmp).args(["prime", "--consumer", "local/carsten"]));

    assert!(listed.starts_with("## Who you can address\n"), "{listed}");
    assert!(
        listed.contains("Address any of them the same way: `nxc send --to <handle>"),
        "{listed}"
    );
    assert!(
        listed.contains(
            "**Product manager** (handle: `pm`, level: senior) — turns intent into work orders"
        ),
        "{listed}"
    );
    assert!(
        primed.contains(listed.trim_end()),
        "prime must carry the very block `list` prints, or the human's view and the persona's \
         drift:\nlist:\n{listed}\nprime:\n{primed}"
    );
}

#[test]
fn list_says_so_when_nothing_at_all_is_declared() {
    // The one path `render_markdown` never reaches: an empty workspace has no directory to render,
    // so `cli.rs` answers with its own line — which must point at what to create rather than at
    // nothing. (Its sibling, "things are declared but none is addressable", is `render_markdown`'s
    // own branch and covered in `persona.rs`.)
    let tmp = workspace();
    let out = stdout_of(human(&tmp).args(["list"]));
    assert!(out.contains("nobody is declared here yet"), "{out}");
    // With the path (nxf 6j6v.dvyq step 4): "there is nothing" and "you have not declared anything
    // yet" are different states, and only the second one can be acted on.
    assert!(out.contains(".nxs-personas/channels.yaml"), "{out}");
}

#[test]
fn list_shows_the_team_to_a_human_and_the_address_book_to_a_persona() {
    let tmp = workspace();
    write_roles(&tmp);
    let all = json_of(human(&tmp).args(["--json", "list"]));
    assert_eq!(all["personas"].as_array().unwrap().len(), 2);

    // A persona is recognised from the session it runs under — it passes no `--persona` at all.
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan it"]));
    let session = opened["session"].as_str().unwrap().to_string();
    let mine = json_of(persona(&tmp, "pm", &session).args(["--json", "list"]));
    assert_eq!(mine["persona"], "pm");
    let names: Vec<&str> = mine["personas"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["handle"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["coder"], "its own address book, not the team");
    assert_eq!(
        mine["personas"][0]["why"], "hand over an implementable work order",
        "and what for"
    );

    // An explicit `--persona` projects that view for anyone — the app's path.
    let asked = json_of(human(&tmp).args(["--json", "list", "--persona", "pm"]));
    assert_eq!(asked["persona"], "pm");
}

// ---- step 2: send --to opens a thread and starts the persona ----------------------------------

#[test]
fn sending_to_a_persona_opens_a_thread_and_triggers_it() {
    let tmp = workspace();
    write_roles(&tmp);
    let receipt = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    assert_eq!(receipt["target"], "persona");
    let thread = receipt["thread_id"].as_str().expect("a thread id");
    let session = receipt["session"].as_str().expect("a started session");
    // nxf 6j6v.hpv8, PR review Important #1: a clean spawn's receipt must show `spawned: true` and
    // an EMPTY-but-PRESENT `warnings` array — not merely omit `warnings` — so a reader checking for
    // the key's presence (rather than its content) cannot mistake a working `--json` for one built
    // against a version that never learned to render it. `skip_serializing_if` on this field would
    // still pass every OTHER assertion in this file; only this one would catch it.
    assert_eq!(receipt["spawned"], true, "{receipt}");
    assert_eq!(receipt["warnings"], serde_json::json!([]), "{receipt}");

    let log = dry_log(&tmp);
    assert!(
        log.contains(&format!("role=pm session={session}")),
        "the persona was actually triggered:\n{log}"
    );
    assert!(
        log.contains(&format!("nxc reply --thread {thread}")),
        "and it was told the thread it must answer into:\n{log}"
    );
    // The POSTED message is the human's own words; the plumbing went only to the persona.
    let board = stdout_of(human(&tmp).args(["threads", "show", thread]));
    assert!(board.contains("plan the release"), "{board}");
    assert!(
        !board.contains("nxc reply --thread"),
        "the thread is the conversation, not the wiring:\n{board}"
    );
}

#[test]
fn send_to_a_persona_reports_a_warning_and_a_nonzero_exit_when_the_trigger_fails() {
    // nxf 6j6v.hpv8: the persona branch posts the message and mints the thread + session BEFORE it
    // triggers the role (`coordinator_commission`), so a spawn failure must not discard all three
    // behind
    // an opaque error the way it used to. `NXC_WORKER` set to a value `select_worker` does not
    // recognise is a deterministic, synchronous trigger failure needing no sidecar/`node` at all —
    // the CLI-side analogue of the Engine seam's `HostWorker::new(|_| Err(...))` fixture (see
    // `WorkerConfig::from_ambient_or_installed`'s own `Err` arm).
    let tmp = workspace();
    write_roles(&tmp);

    let receipt = json_of_failure(human(&tmp).env("NXC_WORKER", "not-a-real-worker").args([
        "--json",
        "send",
        "--to",
        "pm",
        "please help",
    ]));
    assert!(
        receipt["message_id"].as_str().unwrap().starts_with("m-"),
        "{receipt}"
    );
    assert_eq!(receipt["target"], "persona");
    assert!(
        receipt["session"].as_str().is_some(),
        "a session was minted even though it never started: {receipt}"
    );
    assert_eq!(
        receipt["spawned"], false,
        "the typed discriminator, not just non-empty warnings: {receipt}"
    );
    let warnings = receipt["warnings"]
        .as_array()
        .expect("always present in --json, even when empty");
    assert_eq!(warnings.len(), 1, "{receipt}");
    assert!(
        warnings[0]["detail"]
            .as_str()
            .unwrap()
            .contains("unknown NXC_WORKER"),
        "{receipt}"
    );
    // nxf 6j6v.6m6x, at the CLI seam: the entry is a RECORD, so the reason a script would branch on
    // is a value rather than a phrase inside a sentence (nxf 6j6v.93zd gave it the shape).
    assert_eq!(warnings[0]["class"], "step_skipped", "{receipt}");
    assert_eq!(warnings[0]["reason"], "trigger_failed", "{receipt}");

    // The message really landed in the channel — the whole point of the ticket.
    let thread = receipt["thread_id"].as_str().expect("a thread id");
    let board = stdout_of(human(&tmp).args(["threads", "show", thread]));
    assert!(board.contains("please help"), "{board}");
}

// `send_role_reports_a_warning_and_a_nonzero_exit_when_the_trigger_fails` stood here — the same
// assertions as the case above, entered through `send --role` instead of `send --to <persona>`.
// REMOVED with the flag by 6j6v.dvyq §3 (block b), and NOT migrated: the two entrances ran the same
// `coordinator_commission` funnel, which is exactly why this existed as a second case and exactly
// why it
// has nothing left to add once there is one entrance. Migrating it would have produced a byte-wise
// duplicate of `send_to_a_persona_reports_a_warning_and_a_nonzero_exit_when_the_trigger_fails`.

#[test]
fn an_unknown_target_is_refused_before_anything_is_written() {
    let tmp = workspace();
    write_roles(&tmp);
    let err = failure_stderr(human(&tmp).args(["send", "--to", "ghost", "hello?"]));
    assert!(err.contains("no such target"), "{err}");
    assert!(dry_log(&tmp).is_empty(), "nothing was triggered");
}

// ---- step 3: the persona's own prime -----------------------------------------------------------

#[test]
fn the_persona_primes_and_is_told_who_it_is_and_how_to_answer() {
    let tmp = workspace();
    write_roles(&tmp);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let session = opened["session"].as_str().unwrap().to_string();

    // No `--persona`: the session it was started under is what says who it is.
    let out = stdout_of(persona(&tmp, "pm", &session).args(["prime"]));
    assert!(out.contains("## You are `pm`"), "{out}");
    assert!(out.contains("Product manager"), "{out}");
    assert!(out.contains("senior"), "the stage is part of the identity");
    assert!(
        out.contains("nxc reply --thread"),
        "the answering rule — the step the whole flow hangs on:\n{out}"
    );
    assert!(
        out.contains("**Implementer** (handle: `coder`)"),
        "and its own address book — the same rendering the human gets (6j6v.frek):\n{out}"
    );
}

// ---- step 4: several turns, in both directions -------------------------------------------------

#[test]
fn the_conversation_runs_several_turns_and_each_one_reaches_the_other_side() {
    let tmp = workspace();
    write_roles(&tmp);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let thread = opened["thread_id"].as_str().unwrap().to_string();
    let pm_session = opened["session"].as_str().unwrap().to_string();

    // The persona answers.
    let answered = json_of(persona(&tmp, "pm", &pm_session).args([
        "--json",
        "reply",
        "--thread",
        &thread,
        "here is the plan",
    ]));
    assert_eq!(answered["thread_id"], thread);
    assert_eq!(
        answered["resumed"], false,
        "the human it answered has no session to resume"
    );

    // The human answers back — and THIS is the turn that has to reach the persona again.
    let follow_up =
        json_of(human(&tmp).args(["--json", "reply", "--thread", &thread, "and the risks?"]));
    assert_eq!(follow_up["resumed"], true);
    assert_eq!(
        follow_up["woke"], pm_session,
        "the thread knows which session to continue"
    );

    // Which really did reach the worker, as a resume of that same session.
    let log = dry_log(&tmp);
    assert!(
        log.contains(&format!("session={pm_session} resume=")),
        "the second turn resumed the persona rather than starting a new one:\n{log}"
    );
    assert_eq!(
        log.matches(&format!("nxc reply --thread {thread}")).count(),
        2,
        "EVERY turn tells the persona where to answer, the resume included — a live run showed \
         that without it the second answer lands in the session's transcript and the thread stays \
         silent:\n{log}"
    );
    assert_eq!(
        log.matches("role=pm").count(),
        2,
        "exactly two triggers: the summon and the resume\n{log}"
    );

    // And the thread reads back as the conversation it is.
    let board = stdout_of(human(&tmp).args(["threads", "show", &thread]));
    for line in ["plan the release", "here is the plan", "and the risks?"] {
        assert!(board.contains(line), "missing {line:?} in:\n{board}");
    }
}

/// The INVERSE of the test that stood here. `reply_still_takes_a_target_positionally_exactly_as_
/// before` checked 6j6v.p6m1's additive claim — "`--thread` is a new door, not a moved one" — and
/// that claim was true for exactly as long as both doors existed. 6j6v.dvyq §3 closes the old one,
/// so the property worth holding is the opposite: there is ONE address for a conversation.
#[test]
fn reply_no_longer_takes_a_target_positionally() {
    let tmp = workspace();
    write_roles(&tmp);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "go"]));
    let thread = opened["thread_id"].as_str().unwrap().to_string();

    // The old shape: target and body as two positionals. clap now sees a second positional it has
    // no argument for, so it refuses before the store is touched at all.
    let err = failure_stderr(human(&tmp).args(["reply", &thread, "old style"]));
    assert!(
        err.contains("unexpected argument") || err.contains("--thread"),
        "the refusal points at the one address a reply has: {err}"
    );

    // And the surviving shape still works.
    let replied = json_of(human(&tmp).args(["--json", "reply", "--thread", &thread, "new style"]));
    assert_eq!(replied["thread_id"], thread);
}

// ---- reply --thread --if-unanswered (nxf 6j6v.ww0a, review finding 1) -------------------------
//
// The epic's own canonical shape: ticket 6's prime block teaches a role exactly `nxc reply --thread
// <id> "<result>"`, so `--if-unanswered` must be reachable there, not only on the positional-target
// form. Two personas, chained (pm triggers coder), so a real, resolvable return address (pm's own
// session) sits on the other end of coder's thread — the exact shape that would expose a guard bug:
// without gating the return-address resume on `receipt.posted`, a skipped (already-answered) repeat
// call would still find pm's session and erroneously re-trigger it with a reply that was never
// written.

#[test]
fn reply_thread_if_unanswered_posts_and_resumes_the_return_address_when_owed() {
    let tmp = workspace();
    write_roles(&tmp);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let pm_session = opened["session"].as_str().unwrap().to_string();

    // pm (nested trigger, from ITS OWN session) hands the work to coder — coder's own thread now
    // expects coder, and its root message carries pm's session as the return address.
    let handed_off = json_of(persona(&tmp, "pm", &pm_session).args([
        "--json",
        "send",
        "--to",
        "coder",
        "implement it",
    ]));
    let coder_thread = handed_off["thread_id"].as_str().unwrap().to_string();
    let coder_session = handed_off["session"].as_str().unwrap().to_string();

    // coder owes a reply on its own thread (named in `expects_reply_from`, has not posted) →
    // `--if-unanswered` posts, and the return-address resume behaves exactly as an ordinary
    // `reply --thread` would: it wakes pm. `woke` is the mechanism-independent proof of that
    // (coder's reply also completes its own 1-person board, so this can land through either the
    // completion routing `orchestration::reply` runs internally or `reply_in_thread`'s own
    // return-address fallback — see the Engine-seam counterpart in `embed_surface.rs` for why
    // `resumed` alone does not distinguish the two).
    let first = json_of(persona(&tmp, "coder", &coder_session).args([
        "--json",
        "reply",
        "--thread",
        &coder_thread,
        "--if-unanswered",
        "done implementing",
    ]));
    assert_eq!(first["posted"], true);
    assert!(first["message_id"].as_str().unwrap().starts_with("m-"));
    assert_eq!(first["woke"], pm_session, "{first}");
    let log_after_first = dry_log(&tmp);
    assert_eq!(
        log_after_first.matches("msg=done implementing").count(),
        1,
        "pm was woken with coder's actual reply text:\n{log_after_first}"
    );
}

#[test]
fn reply_thread_if_unanswered_is_a_silent_no_op_and_wakes_nobody_when_not_owed() {
    let tmp = workspace();
    write_roles(&tmp);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let pm_session = opened["session"].as_str().unwrap().to_string();
    let handed_off = json_of(persona(&tmp, "pm", &pm_session).args([
        "--json",
        "send",
        "--to",
        "coder",
        "implement it",
    ]));
    let coder_thread = handed_off["thread_id"].as_str().unwrap().to_string();
    let coder_session = handed_off["session"].as_str().unwrap().to_string();

    // 1st call: coder owes → posts and wakes pm (pinned above; repeated here as the fixture this
    // test's repeat call depends on).
    json_of(persona(&tmp, "coder", &coder_session).args([
        "--json",
        "reply",
        "--thread",
        &coder_thread,
        "--if-unanswered",
        "done implementing",
    ]));
    let log_before_repeat = dry_log(&tmp);

    // 2nd call: coder already replied — `expects_reply_from` no longer names anyone outstanding —
    // so this is a deliberate no-op: exit 0, nothing posted, and CRITICALLY nobody woken. Without
    // gating the return-address resume on `posted`, this would still find pm's real session via
    // `thread_return_address` and re-trigger it with "done implementing again" even though nothing
    // was written — the exact regression this pair of tests exists to catch.
    let second = json_of(persona(&tmp, "coder", &coder_session).args([
        "--json",
        "reply",
        "--thread",
        &coder_thread,
        "--if-unanswered",
        "done implementing again",
    ]));
    assert_eq!(second["posted"], false);
    assert!(
        second.get("message_id").is_none(),
        "nothing was posted, so there is no message id: {second}"
    );
    assert_eq!(
        second["resumed"], false,
        "a skip must wake nobody: {second}"
    );
    assert!(second.get("woke").is_none(), "{second}");

    let log_after_repeat = dry_log(&tmp);
    assert_eq!(
        log_after_repeat, log_before_repeat,
        "the repeat call must not have triggered anything at all"
    );

    // Exactly one coder reply landed — read from coder's own side, the DM's actual membership.
    let board =
        stdout_of(persona(&tmp, "coder", &coder_session).args(["threads", "show", &coder_thread]));
    assert_eq!(
        board.matches("done implementing").count(),
        1,
        "the repeat call wrote nothing: {board}"
    );
}

// ---- the SECOND turn: the discharge belongs to the turn, not to the thread (nxf 6j6v.cg8g) -----
//
// The chain of 6j6v.a71h §2, whose second turn is the normal case and not the exception: the coder
// answers, its supervisor comes back onto the SAME thread with the review, and the coder answers
// again. Before 6j6v.cg8g, `replied` was "has this sender EVER written in this thread", so the
// coder's first answer discharged the obligation for good — and `--if-unanswered`, the teardown net
// a sidecar fires for a session that ends without answering (`worker.rs`'s `replyThread` →
// `nxc reply --thread <id> --if-unanswered`), became a permanent no-op from the second turn on,
// precisely where it is the only thing left.
//
// The supervisor's RE-DECLARATION is what opens the new turn, and the fix is that the discharge is
// measured from that declaration rather than from the thread's whole history. It is made HERE
// through `facade::set_expects` — the seam write, opener-only — against the same workspace db the
// `nxc` subprocesses use. Not through a CLI verb: `nxc threads expect` was removed with 6j6v.dvyq
// (this same fix round), because a human-typed re-declare is not part of the agent-to-agent surface.
// The channel supervisor of 6j6v.hq71/6j6v.pf6j will make exactly this call from inside its own flow.
//
// WHAT THIS TEST PROVES, AND WHAT IT DOES NOT. It proves the PREDICATE end to end through the real
// CLI: given a re-declared obligation, `nxc reply --thread <id> --if-unanswered` posts in the second
// turn where it used to no-op, and the answer routes back. It does NOT prove the second turn's
// teardown net end to end from a spawned session, and cannot yet: the sidecar only fires
// `--if-unanswered` when its trigger carried a `replyThread` (`worker.rs`'s `TriggerRequest`), and
// the direct trigger path still leaves that `None` on a resume — `coordinator_commission` writes
// `expects_reply_from` (and sets `reply_thread`) only when the thread carries no expectation yet
// (`orchestration.rs`). Closing that is the channel supervisor's job (6j6v.pf6j), deliberately not
// this item's: this item owns the predicate the supervisor computes on.

#[test]
fn reply_if_unanswered_owes_again_on_the_second_turn_of_the_same_thread() {
    let tmp = workspace();
    write_roles(&tmp);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let pm_session = opened["session"].as_str().unwrap().to_string();

    // Turn 1. pm hands the work to coder: the thread expects coder, and nobody has answered.
    let handed_off = json_of(persona(&tmp, "pm", &pm_session).args([
        "--json",
        "send",
        "--to",
        "coder",
        "implement it",
    ]));
    let coder_thread = handed_off["thread_id"].as_str().unwrap().to_string();
    let coder_session = handed_off["session"].as_str().unwrap().to_string();

    let first = json_of(persona(&tmp, "coder", &coder_session).args([
        "--json",
        "reply",
        "--thread",
        &coder_thread,
        "--if-unanswered",
        "done implementing",
    ]));
    assert_eq!(first["posted"], true, "turn 1: coder owed a reply: {first}");

    // Still inside turn 1: the debt is settled, so the net stays shut. This half must NOT regress —
    // it is the whole reason `--if-unanswered` exists (nxf 6j6v.ww0a).
    let inside_turn_one = json_of(persona(&tmp, "coder", &coder_session).args([
        "--json",
        "reply",
        "--thread",
        &coder_thread,
        "--if-unanswered",
        "done implementing again",
    ]));
    assert_eq!(
        inside_turn_one["posted"], false,
        "the same turn is answered once: {inside_turn_one}"
    );

    // Turn 2. The supervisor comes back with the review and re-declares the obligation — the same
    // role, the same thread, a new turn.
    json_of(persona(&tmp, "pm", &pm_session).args([
        "--json",
        "reply",
        "--thread",
        &coder_thread,
        "the review says: fix the retry path",
    ]));
    let redeclared = set_expects(
        &tmp,
        "local/pm",
        &coder_thread,
        &["local/coder".to_string()],
    );
    assert_eq!(
        redeclared.outstanding,
        ["local/coder"],
        "the re-declaration re-opens the obligation: {redeclared:?}"
    );

    // …and the teardown net is live again: the session that is working on the final report owes an
    // answer, so a session ending without one still leaves the thread answered.
    let second_turn = json_of(persona(&tmp, "coder", &coder_session).args([
        "--json",
        "reply",
        "--thread",
        &coder_thread,
        "--if-unanswered",
        "final report: retry path fixed",
    ]));
    assert_eq!(
        second_turn["posted"], true,
        "turn 2: the coder owes again, so the teardown net posts: {second_turn}"
    );
    assert_eq!(
        second_turn["woke"], pm_session,
        "and the second turn's answer hands the turn back to the supervisor: {second_turn}"
    );

    // Both of the coder's answers are in the thread, and the second one is not a duplicate of the
    // first: two turns, two answers.
    let board =
        stdout_of(persona(&tmp, "coder", &coder_session).args(["threads", "show", &coder_thread]));
    assert_eq!(
        board.matches("done implementing").count(),
        1,
        "turn 1 answered exactly once: {board}"
    );
    assert_eq!(
        board.matches("final report: retry path fixed").count(),
        1,
        "turn 2 answered exactly once: {board}"
    );
}

// ---- step 5: --stream, and who may use it ------------------------------------------------------

#[test]
fn stream_follows_the_conversation_until_the_answer_lands() {
    let tmp = workspace();
    write_roles(&tmp);
    // A background answer, arriving while the follow is running: the point of `--stream` is that
    // the caller does not have to come back and look.
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let thread = opened["thread_id"].as_str().unwrap().to_string();
    let pm_session = opened["session"].as_str().unwrap().to_string();

    let answering = {
        let (dir, thread, session) = (tmp.path().to_path_buf(), thread.clone(), pm_session.clone());
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            nxs_test_support::cargo_bin("nxc")
                .current_dir(&dir)
                .env("NXF_DETERMINISTIC_IDS", "1")
                .env("NXC_ACTOR", "pm")
                .env("NXC_ORIGIN", "local")
                .env("NXC_NOW", NOW)
                .env("NXC_WORKER", "dry")
                .env("NXC_SESSION", &session)
                .args(["reply", "--thread", &thread, "here is the plan"])
                .assert()
                .success();
        })
    };

    let watched = stdout_of(
        human(&tmp)
            .env("NXC_STREAM_IDLE", "20")
            .env("NXC_STREAM_POLL", "0.05")
            .args(["reply", "--thread", &thread, "any update?", "--stream"]),
    );
    answering.join().unwrap();
    assert!(
        watched.contains("local/pm: here is the plan"),
        "the answer arrives on the stream:\n{watched}"
    );
}

#[test]
fn stream_is_refused_to_a_persona_and_says_what_to_do_instead() {
    let tmp = workspace();
    write_roles(&tmp);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "go"]));
    let session = opened["session"].as_str().unwrap().to_string();

    let err = failure_stderr(
        persona(&tmp, "pm", &session).args(["send", "--to", "coder", "do this", "--stream"]),
    );
    assert!(err.contains("for a human at the keyboard"), "{err}");
    // "…and says what to do instead" was in this test's NAME and in nothing it asserted, which is
    // how the string kept teaching `nxc inbox` for a commit after 6j6v.dvyq §3 took that verb off
    // the agent surface. This arm fires ONLY for a registered persona, so it is the one runtime
    // message in the binary that no human can ever see: it must name what the agent surface
    // carries, and nothing else.
    assert!(
        err.contains("nxc status --thread"),
        "the refusal names the read that replaces waiting: {err}"
    );
    assert!(
        !err.contains("nxc inbox") && !err.contains("nxc read"),
        "and teaches neither verb that left the agent surface: {err}"
    );

    // And nothing was sent: the gate is checked before the write, not after it. Probed with
    // `search` since 6j6v.1gm9 removed `nxc inbox` — and as `local/pm`, which is a member of BOTH
    // conversations, so the negative below is not vacuous: the same read finds the message that
    // WAS posted.
    let seen =
        |q: &str| stdout_of(human(&tmp).args(["--json", "search", q, "--consumer", "local/pm"]));
    assert!(
        seen("go").contains("\"body\":\"go\""),
        "the read is live — it finds the message that was posted: {}",
        seen("go")
    );
    assert_eq!(
        seen("do this").trim(),
        "[]",
        "and the refused one never landed"
    );
}

#[test]
fn stream_knocks_with_what_it_last_saw_and_then_stops_watching() {
    // The two ends of the deadline, through the real loop: nobody answers (the worker is dry), so
    // the follow knocks with the evidence it has and then gives up — cleanly, exit 0, because the
    // message it followed was still sent. Only the state machine behind this was tested before
    // (PR #295 review, Test Quality #4); the rendering of both outcomes was not.
    let tmp = workspace();
    write_roles(&tmp);

    // A TWO-SECOND window, deliberately not shorter. The knock fires in the `idle/6` stretch before
    // the window closes — 333ms here, about six turns of the 50ms poll loop — and the follow would
    // skip straight to giving up if a single turn ever spanned the whole stretch. Under a full
    // parallel suite that is a real stall length, so the margin is the point: halving `idle` to save
    // a second buys a load-dependent flake in the release profile.
    //
    // `send --to` rather than `reply --thread`, because the send SEEDS the persona's session — which
    // is what gives the knock a log path to point a human at.
    let watched = |json: bool| {
        let mut c = human(&tmp);
        c.env("NXC_STREAM_IDLE", "2").env("NXC_STREAM_POLL", "0.05");
        let mut args = vec!["send", "--to", "pm", "silence please", "--stream"];
        if json {
            args.insert(0, "--json");
        }
        stdout_of(c.args(args))
    };

    let machine = watched(true);
    let lines: Vec<&str> = machine.lines().collect();
    let opened: Value = serde_json::from_str(lines[0]).expect("the receipt comes first");
    let (thread, session) = (
        opened["thread_id"].as_str().unwrap(),
        opened["session"].as_str().unwrap(),
    );
    assert!(
        machine.contains(r#""stream":"knock""#),
        "the knock reaches an agent as its own event:\n{machine}"
    );
    assert!(
        machine.contains(&format!(".nxs/agent-logs/{session}.log")),
        "and says where to go and look:\n{machine}"
    );
    assert!(
        machine.contains(r#""stream":"gave_up""#) && machine.contains(thread),
        "the give-up names the thread the answer will still land in:\n{machine}"
    );

    let human_form = watched(false);
    assert!(
        human_form.contains("quiet for") && human_form.contains("nothing in the transcript"),
        "the human knock says what the transcript last showed:\n{human_form}"
    );
    assert!(
        human_form.contains("no longer watching") && human_form.contains("nxc threads show "),
        "and the give-up hands the reader the way back in:\n{human_form}"
    );
}

#[test]
fn a_stream_timing_that_is_not_a_duration_is_refused_before_anything_is_written() {
    // `NXC_STREAM_IDLE=-1` used to parse as an f64 and then PANIC inside `Duration::from_secs_f64`
    // (exit 101), after the message had already been posted — the confirmed bug in the PR #295
    // review, found independently by two reviews. Both halves are pinned here: it is a validation
    // error, and it happens before the write, exactly as the persona gate beside it does.
    let tmp = workspace();
    write_roles(&tmp);
    let err = failure_stderr(human(&tmp).env("NXC_STREAM_IDLE", "-1").args([
        "send",
        "--to",
        "pm",
        "must not be sent",
        "--stream",
    ]));
    assert!(
        err.contains("NXC_STREAM_IDLE") && err.contains("not negative"),
        "the message names the variable and what is wrong with it: {err}"
    );

    let hits = stdout_of(human(&tmp).args([
        "--json",
        "search",
        "must not be sent",
        "--consumer",
        "local/pm",
    ]));
    assert_eq!(
        hits.trim(),
        "[]",
        "nothing was posted on the way to the rejection: {hits}"
    );
    assert_eq!(dry_log(&tmp), "", "and no persona was started");
}

#[test]
fn stream_without_a_conversation_to_follow_is_refused() {
    let tmp = workspace();
    write_roles(&tmp);
    // The positional channel is gone (6j6v.dvyq §3), so clap refuses the second positional before
    // `--stream` is ever looked at — which is the earlier and better refusal: nothing is resolved,
    // nothing is written.
    let err = failure_stderr(human(&tmp).args(["send", "some-channel", "hi", "--stream"]));
    assert!(err.contains("unexpected argument"), "{err}");
    // And `--role`, the flag that once named a target `--stream` could not follow, is gone too
    // (6j6v.dvyq §3, block b) — so this is refused one step earlier still, by clap, for the same
    // reason the positional is: there is nothing to resolve.
    let err = failure_stderr(human(&tmp).args(["send", "hi", "--role", "pm", "--stream"]));
    assert!(err.contains("unexpected argument"), "{err}");
}

// ---- what the consolidation took away, and what says the same thing now ------------------------

/// The CONSTRUCTION PROOF acceptance point 1 asks for: "every removed entrance is expressible
/// through a remaining one, proven by construction (no paper mapping)."
///
/// This test used to be its mirror image. `every_older_entry_point_still_works_unchanged` was
/// 6j6v.p6m1's own executable check of the rule that item set itself — "take nothing away" — and it
/// was honest for as long as that held. 6j6v.dvyq is the other half of p6m1, the WEGNEHMEN, so the
/// same fixture now drives each removed entrance and its successor side by side: the old shape is
/// refused, and the new one does what the old one did.
#[test]
fn every_removed_entry_point_is_refused_and_its_successor_does_the_same_thing() {
    let tmp = workspace();
    write_roles(&tmp);
    std::fs::write(
        tmp.path().join(".nxs-personas/channels.yaml"),
        "- name: standup\n  members: [pm, coder]\n",
    )
    .unwrap();

    // `send --role <handle>` → `send --to <handle>`. A COLLAPSE, and the construction proof is that
    // the successor does strictly more: the same session is minted and the same worker started, and
    // a THREAD comes back that the old form never produced — which is what makes the answer
    // addressable by `reply --thread`, the one reply form §3 leaves standing.
    let err = failure_stderr(human(&tmp).args(["send", "hello pm", "--role", "pm"]));
    assert!(err.contains("unexpected argument"), "{err}");
    let summoned = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "hello pm"]));
    assert!(
        summoned["session"].as_str().is_some(),
        "still mints a session: {summoned}"
    );
    assert!(
        summoned["thread_id"].as_str().is_some(),
        "and hands back the address the old form had none of: {summoned}"
    );
    assert_eq!(summoned["spawned"], true, "{summoned}");
    assert_eq!(summoned["warnings"], serde_json::json!([]), "{summoned}");

    // `channels create <name>` → GONE. A channel is a declaration; there is no verb that mints one.
    let err = failure_stderr(human(&tmp).args(["channels", "create", "side"]));
    assert!(
        err.contains("unrecognized subcommand") || err.contains("channels"),
        "{err}"
    );

    // `send --session <id>` → NOTHING. The one entrance on §3's list with no successor, and the one
    // line of this test that is not a construction proof, deliberately (owner, 2026-08-19): a
    // resume COMMISSIONS a live session, `reply --thread` ANSWERS a conversation through its return
    // address, and a caller holding a session id and no thread has no path left. Recorded as a
    // named LOSS in `seam_disposition.rs`; the residual — delivering into a session mid-turn — is
    // nxf 6j6v.xr3z.
    let err = failure_stderr(human(&tmp).args(["send", "more", "--session", "m-whatever"]));
    assert!(err.contains("unexpected argument"), "{err}");

    // `send <declared-channel> <body>` → `send --to <declared-channel> <body>`.
    let err = failure_stderr(human(&tmp).args(["send", "standup", "status?"]));
    assert!(
        err.contains("unexpected argument"),
        "the positional channel is gone: {err}"
    );
    let board = json_of(human(&tmp).args(["--json", "send", "--to", "standup", "status?"]));
    assert!(board["thread_id"].is_string(), "{board}");

    // `ask <channel> <body> --expect <h>` → `send --to <channel> <body>`, with the DECLARATION
    // supplying who must answer. That is the substitution, and the expected set proves it: the same
    // board comes out, and nobody had to name the reviewers on the command line.
    let err = failure_stderr(human(&tmp).args(["ask", "standup", "and now?", "--expect", "coder"]));
    assert!(
        err.contains("unrecognized subcommand") || err.contains("ask"),
        "{err}"
    );
    let asked = json_of(human(&tmp).args(["--json", "send", "--to", "standup", "and now?"]));
    assert!(asked["thread_id"].is_string(), "{asked}");
    assert!(
        !asked["expects"]
            .as_array()
            .expect("expects is a list")
            .is_empty(),
        "the channel's own declaration says who must answer: {asked}"
    );

    // `reply <target> <body>` → `reply --thread <id> <body>`; covered on its own above, driven here
    // too so the whole walk ends on a real answer into the board this test opened.
    json_of(human(&tmp).args([
        "--json",
        "reply",
        "--thread",
        asked["thread_id"].as_str().unwrap(),
        "ok",
    ]));
}

// ---- the working-tree lease from the command line (nxf 6j6v.303b) ------------------------------
//
// The CLI half of the ticket's "one test on each seam". A human at the terminal must not be told
// "opened thread …" and left to assume somebody is working on it — the epic's §6 calls that class of
// silent failure out by name. The engine half of the same behaviour is in `embed_surface.rs`.

/// Two personas that each declare they need the working copy to themselves.
fn write_exclusive_roles(tmp: &TempDir) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("coder.yaml"),
        "handle: coder\njob_title: Implementer\nsystem_prompt: You are the coder.\n\
         working_tree: exclusive\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("fixer.yaml"),
        "handle: fixer\njob_title: Fixer\nsystem_prompt: You are the fixer.\n\
         working_tree: exclusive\n",
    )
    .unwrap();
    // Untouched by any of it: the default every declaration written before this epic reads as.
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\njob_title: Product manager\nsystem_prompt: You are the PM.\n",
    )
    .unwrap();
}

#[test]
fn a_second_send_to_an_exclusive_persona_is_queued_and_the_receipt_says_so() {
    let tmp = workspace();
    write_exclusive_roles(&tmp);

    let first = json_of(human(&tmp).args(["--json", "send", "--to", "coder", "build the website"]));
    assert!(
        first.get("queue_position").is_none(),
        "the first chain takes the working copy, so its receipt is byte-identical to before this \
         ticket: {first}"
    );
    assert!(first.get("queued_behind").is_none(), "{first}");

    let second =
        json_of(human(&tmp).args(["--json", "send", "--to", "fixer", "change the button label"]));
    assert_eq!(
        second["queued_behind"].as_str(),
        Some(format!("thread:{}", first["thread_id"].as_str().unwrap()).as_str()),
        "the receipt names the chain holding the working copy: {second}"
    );
    assert_eq!(second["queue_position"].as_i64(), Some(1), "{second}");

    // The claim the receipt cannot make on its own: the dry worker was handed exactly one trigger.
    let log = dry_log(&tmp);
    assert_eq!(
        log.lines().filter(|l| l.starts_with("trigger ")).count(),
        1,
        "only the lease holder was started:\n{log}"
    );
    assert!(log.contains("role=coder"), "{log}");
    assert!(
        !log.contains("role=fixer"),
        "the queued persona was never started:\n{log}"
    );

    // And a `shared` persona is not affected by any of it.
    let shared = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "status?"]));
    assert!(shared.get("queue_position").is_none(), "{shared}");
    let log = dry_log(&tmp);
    assert!(
        log.contains("role=pm"),
        "a shared persona still starts:\n{log}"
    );
}

#[test]
fn the_holders_reply_frees_the_working_copy_and_starts_the_persona_that_was_waiting() {
    // The CLI half of nxf 6j6v.fe0f, as a human and a persona actually type it: the coder is
    // working, the fixer is queued behind it, and the coder's own `reply --thread` — the exact
    // command its prime block tells it to answer with — is what hands the working copy over. No
    // daemon runs, nothing polls: the release rides the reply.
    let tmp = workspace();
    write_exclusive_roles(&tmp);

    let first = json_of(human(&tmp).args(["--json", "send", "--to", "coder", "build the website"]));
    let second =
        json_of(human(&tmp).args(["--json", "send", "--to", "fixer", "change the button label"]));
    assert_eq!(
        second["queue_position"].as_i64(),
        Some(1),
        "the fixer is waiting: {second}"
    );
    let waiting_session = second["session"].as_str().unwrap().to_string();
    let log = dry_log(&tmp);
    assert_eq!(
        log.lines().filter(|l| l.starts_with("trigger ")).count(),
        1,
        "only the coder is running so far:\n{log}"
    );

    // The coder answers, exactly as it was told to.
    let thread = first["thread_id"].as_str().unwrap();
    let answered = json_of(
        human(&tmp)
            .env("NXC_ACTOR", "coder")
            .args(["--json", "reply", "--thread", thread, "shipped"]),
    );
    assert_eq!(answered["posted"].as_bool(), Some(true), "{answered}");

    let log = dry_log(&tmp);
    assert_eq!(
        log.lines().filter(|l| l.starts_with("trigger ")).count(),
        2,
        "the reply that discharged the coder's obligation started the waiting fixer:\n{log}"
    );
    assert!(
        log.contains(&format!("role=fixer session={waiting_session}")),
        "and it is the SAME session the queued receipt already named — not a second one:\n{log}"
    );
    assert!(
        log.contains("msg=change the button label"),
        "re-composed from the entry's own raw inputs:\n{log}"
    );
}

#[test]
fn the_human_readable_send_to_says_plainly_that_nothing_has_started_yet() {
    // `--json` is what an app reads; this is what the person at the keyboard reads, and it is the
    // half that would otherwise quietly say "opened thread …" and stop.
    let tmp = workspace();
    write_exclusive_roles(&tmp);

    let first = stdout_of(human(&tmp).args(["send", "--to", "coder", "build the website"]));
    assert!(first.starts_with("-> coder · thread m-"), "{first}");
    assert!(
        !first.contains("QUEUED"),
        "the holder is not queued behind anything: {first}"
    );

    let second = stdout_of(human(&tmp).args(["send", "--to", "fixer", "change the button label"]));
    assert!(
        second.contains("QUEUED at position 1 behind thread:"),
        "the human must be told where in the queue this landed: {second}"
    );
    assert!(
        second.contains("has not started yet"),
        "and that nothing is running on it: {second}"
    );
}

#[test]
fn a_persona_send_waits_on_a_lease_a_declared_channel_is_holding() {
    // The epic's "beide Eingaenge streiten um dieselbe Sperre" from the command line. The CHANNEL
    // declares `working_tree: exclusive`; its member declares nothing — the shape of the epic's own
    // §4 example, and the shape the first cut of this ticket let through unguarded.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("reviewer.yaml"),
        "handle: reviewer\njob_title: Reviewer\nsystem_prompt: You are the reviewer.\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("coder.yaml"),
        "handle: coder\njob_title: Implementer\nsystem_prompt: You are the coder.\n\
         working_tree: exclusive\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: coding\n  members: [reviewer]\n  working_tree: exclusive\n",
    )
    .unwrap();

    let board =
        json_of(human(&tmp).args(["--json", "send", "--to", "coding", "build the website"]));
    assert_eq!(board["target"].as_str(), Some("channel"), "{board}");

    let persona =
        json_of(human(&tmp).args(["--json", "send", "--to", "coder", "change the button label"]));
    assert_eq!(
        persona["queued_behind"].as_str(),
        Some(format!("thread:{}", board["thread_id"].as_str().unwrap()).as_str()),
        "the persona entrance waits on the lease the CHANNEL entrance took: {persona}"
    );
    assert_eq!(persona["queue_position"].as_i64(), Some(1), "{persona}");

    let log = dry_log(&tmp);
    assert_eq!(
        log.lines().filter(|l| l.starts_with("trigger ")).count(),
        1,
        "one working copy, one running session:\n{log}"
    );
    assert!(log.contains("role=reviewer"), "{log}");
    assert!(!log.contains("role=coder"), "{log}");
}

// ---- the writing surface after nxf 6j6v.ckeq: three statements, no levers ----------------------
//
// The seam half of the same item is `writing_seam.rs`. What is only visible from the command line
// is here: the working-tree queue's FIFO order (whose sort key lost its first component when
// `--priority` went), the mandatory `refs` with its `--no-ref` escape, and the sidecar teardown,
// which is the one caller `--if-unanswered` was ever built for.

/// Three personas that each declare they need the working copy to themselves — the queue this
/// section is about, at its shortest useful length (a holder and two waiting behind it).
fn write_three_exclusive_roles(tmp: &TempDir) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["first", "second", "third"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!(
                "handle: {handle}\nsystem_prompt: You are {handle}.\nworking_tree: exclusive\n"
            ),
        )
        .unwrap();
    }
}

fn store_of(tmp: &TempDir) -> nexus_chat::store::ChatStore {
    nexus_chat::workspace::Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

#[test]
fn the_working_tree_queue_is_fifo_and_nothing_on_the_surface_can_reorder_it() {
    // TWO halves, and only together do they say "decided" rather than "so far so good".
    //
    // (a) THE ORDER. Three exclusive personas summoned in a fixed order: the first takes the working
    //     copy, the other two queue behind it in arrival order. `working_tree.rs` still sorts on
    //     `(priority, enqueued_at, id)` — the column is untouched, and 6j6v.xr3z will want it — but
    //     with `priority` off both writing verbs every entry carries `Priority::Normal`, so the
    //     first component is constant and the key degenerates to arrival order.
    //
    // (b) THE ABSENCE OF A LEVER. Without this half the test is satisfied by an accident: three
    //     sends that all happen to use the default would drain in order under a priority queue too.
    //     `--priority` was the ONE way a caller could have jumped the queue, and it is refused by
    //     name now, so FIFO is what the surface can express rather than what it happened to do.
    //
    // Owner, 2026-08-21: "Reines FIFO ist erst einmal okay. Wir werden das nochmal anpacken muessen,
    // aber ich will erst die Probleme sammeln, die durch mangelnde Prio entstehen."
    let tmp = workspace();
    write_three_exclusive_roles(&tmp);

    let queued: Vec<Value> = ["first", "second", "third"]
        .iter()
        .map(|handle| {
            json_of(human(&tmp).args([
                "--json",
                "send",
                "--to",
                handle,
                "--no-ref",
                "do your part",
            ]))
        })
        .collect();

    assert!(
        queued[0].get("queue_position").is_none(),
        "the first summon takes the working copy: {}",
        queued[0]
    );
    assert_eq!(
        queued[1]["queue_position"].as_i64(),
        Some(1),
        "the second summon is first in line: {}",
        queued[1]
    );
    assert_eq!(
        queued[2]["queue_position"].as_i64(),
        Some(2),
        "the third is behind it — arrival order, nothing else: {}",
        queued[2]
    );
    let log = dry_log(&tmp);
    assert!(log.contains("role=first"), "{log}");
    assert!(
        !log.contains("role=second") && !log.contains("role=third"),
        "only the holder started:\n{log}"
    );

    // …and the order really is the order they drain in, not just the order they were reported in.
    let store = store_of(&tmp);
    let waiting: Vec<String> = store
        .list_working_tree_queue()
        .expect("read the queue")
        .into_iter()
        .map(|q| q.role)
        .collect();
    assert_eq!(
        waiting,
        vec!["second".to_string(), "third".to_string()],
        "the queue drains in arrival order"
    );
    drop(store);

    // (b) There is no lever left.
    let stderr = failure_stderr(human(&tmp).args([
        "send",
        "--to",
        "second",
        "--priority",
        "urgent",
        "--no-ref",
        "let me through",
    ]));
    assert!(
        stderr.contains("--priority"),
        "the queue is FIFO because nothing can ask for anything else — the flag is refused by name \
         so a caller learns why rather than being silently ignored: {stderr}"
    );
}

#[test]
fn omitting_refs_without_no_ref_warns_as_a_field_in_the_json_envelope() {
    // THE TRAP THIS TEST EXISTS FOR (6j6v.ckeq §5): a warning that only reaches stderr is a warning
    // an app never sees, which is exactly the breadcrumb class 6j6v.93zd closed for `warnings`. So
    // the field is asserted on the `--json` receipt an app reads, and the stderr line is asserted
    // beside it — one text in two places, so a terminal reader and a UI cannot be told different
    // things.
    let tmp = workspace();
    write_roles(&tmp);

    let out = human(&tmp)
        .args(["--json", "send", "--to", "coder", "build the website"])
        .assert()
        .success();
    let receipt: Value =
        serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim())
            .expect("valid json");
    let warning = receipt["refs_warning"]
        .as_str()
        .unwrap_or_else(|| panic!("`refs_warning` must be a FIELD an app can read: {receipt}"));
    assert!(
        warning.contains("--no-ref"),
        "the warning names the way to say `there really is nothing`: {warning}"
    );
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains(warning),
        "the human at the terminal is told the same thing, verbatim: {stderr}"
    );
}

#[test]
fn a_named_reference_and_an_explicit_no_ref_both_meet_the_obligation() {
    // The two ways to be quiet about a ticket, and the field is `null` for both — the discriminator
    // an app branches on is the field's VALUE, never a substring of its prose. The key itself is
    // always there, which is `warnings`' own form (6j6v.hpv8: "leer = nichts Auffaelliges; im
    // `--json` IMMER vorhanden") and what stops a reader having to tell "absent" from "nothing to
    // report".
    let tmp = workspace();
    write_roles(&tmp);

    let with_ref = json_of(human(&tmp).args([
        "--json",
        "send",
        "--to",
        "coder",
        "--ref",
        "nxf_ids=6j6v.ckeq",
        "build the website",
    ]));
    assert!(
        with_ref.get("refs_warning").is_some() && with_ref["refs_warning"].is_null(),
        "a named reference meets the obligation, and the key is present anyway: {with_ref}"
    );

    let no_ref = json_of(human(&tmp).args([
        "--json",
        "send",
        "--to",
        "coder",
        "--no-ref",
        "and this one is about nothing",
    ]));
    assert!(
        no_ref["refs_warning"].is_null(),
        "an explicit `--no-ref` is an answer, not an omission: {no_ref}"
    );
}

#[test]
fn a_list_of_references_is_expressible_and_all_of_it_lands_on_the_message() {
    // Owner: "Es muss eine LISTE angegeben werden koennen." `--ref nxf_ids=` is repeatable, and the
    // whole list reaches the posted message rather than the last one winning.
    let tmp = workspace();
    write_roles(&tmp);

    let receipt = json_of(human(&tmp).args([
        "--json",
        "send",
        "--to",
        "coder",
        "--ref",
        "nxf_ids=6j6v.ckeq",
        "--ref",
        "nxf_ids=6j6v.m4xe",
        "build the website",
    ]));
    assert!(receipt["refs_warning"].is_null(), "{receipt}");

    let store = store_of(&tmp);
    let thread = receipt["thread_id"].as_str().unwrap();
    let refs: nexus_chat::model::Refs = store
        .messages_in_thread(thread)
        .unwrap()
        .into_iter()
        .find_map(|m| m.refs)
        .map(|raw| serde_json::from_str(&raw).expect("refs parse"))
        .expect("the opening message carries refs");
    assert_eq!(
        refs.nxf_ids,
        vec!["6j6v.ckeq".to_string(), "6j6v.m4xe".to_string()],
        "every id the caller named is on the message, in order"
    );
}

#[test]
fn the_sidecar_teardown_still_writes_its_failure_reply_when_a_session_ends_unanswered() {
    // `agent-sidecar/src/main.mjs` runs EXACTLY this when an SDK session ends without ever having
    // answered the thread it owed: `nxc reply --thread <id> --if-unanswered <text>`. That call is
    // the last line of defence against a thread going quiet forever, and it is the reason
    // `if_unanswered` exists at all — never an agent option, and never an app one. 6j6v.ckeq takes
    // the field off `ReplyThreadRequest`; this is the test that the mechanism did not go with it.
    let tmp = workspace();
    write_roles(&tmp);
    let opened = json_of(human(&tmp).args([
        "--json",
        "send",
        "--to",
        "coder",
        "--no-ref",
        "please do the thing",
    ]));
    let thread = opened["thread_id"].as_str().unwrap().to_string();

    // The session ends without answering: the teardown settles the debt on its behalf.
    let settled = json_of(persona(&tmp, "coder", "s-coder").args([
        "--json",
        "reply",
        "--thread",
        &thread,
        "--if-unanswered",
        "the session ended without producing an answer",
    ]));
    assert_eq!(
        settled["posted"].as_bool(),
        Some(true),
        "the debt was owed, so the teardown wrote the failure reply: {settled}"
    );

    let store = store_of(&tmp);
    assert!(
        store
            .messages_in_thread(&thread)
            .unwrap()
            .iter()
            .any(|m| m.body.contains("ended without producing an answer")),
        "and it is on the thread, where whoever is waiting will see it"
    );
    assert!(
        store.thread_quorum(&thread, NOW).unwrap().unwrap().complete,
        "the thread is no longer waiting on a session that has gone"
    );
    drop(store);

    // IDEMPOTENT, which is the whole reason the flag exists: the teardown may be reached twice — the
    // session may have answered before it died — and must not answer a second time.
    let again = json_of(persona(&tmp, "coder", "s-coder").args([
        "--json",
        "reply",
        "--thread",
        &thread,
        "--if-unanswered",
        "the session ended without producing an answer",
    ]));
    assert_eq!(
        again["posted"].as_bool(),
        Some(false),
        "nothing owed, nothing written: {again}"
    );
}
