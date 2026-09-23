//! **Somebody can finally say "it is good enough, go on"** (nxf 6j6v.am8j, owner 2026-09-12) — the
//! one transition a declared cycle had no door for, and the reason the first real run of this system
//! produced nothing.
//!
//! `a_declared_cycle_sends_the_work_back.rs` covers the cycle itself: the verdict, the back edge,
//! the ceiling. What it also shows, read from the other side, is that the ONLY way out of such a
//! cycle was for the reviewing step to become satisfied of its own accord. Measured in
//! `watch-bundestag` on 2026-09-12: thirty-one threads, six draft→review passes, six verdicts of
//! `needs_rework` out of six, an epic cut and usable — and not one ticket handed to the coding
//! chain. `max_passes` bounded one RUN and every answer from the owner started a new one, so the
//! human above the loop was its rewinder rather than its exit.
//!
//! So this suite asserts the door and, just as hard, its lock: an acceptance is the COMMISSIONER's
//! to give, never the producer's (*"der Erzeuger darf seinen eigenen Pruefer NICHT fuer zufrieden
//! erklaeren"*), and one that would move nothing is refused before it is written rather than posted
//! into a thread where it reads, afterwards, exactly like one that worked.
//!
//! The harness is the cycle suite's, deliberately: the real `nxc` binary against the dry worker, so
//! nothing here builds a thread, a parent edge or an expectation by hand.

#[path = "common/mod.rs"]
mod common;

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-09-13T10:00:00Z";

/// The owner's own example from the 2026-08-29 note, with the ceiling low enough to reach in a test.
const CYCLE: &str = "- name: coding\n  members: [coder, review, finisher]\n  steps:\n    \
                     - id: build\n      target: coder\n      next: check\n    \
                     - id: check\n      target: review\n      on_needs_rework: build\n      \
                     max_passes: 2\n      next: ship\n    \
                     - id: ship\n      target: finisher\n";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

/// The human at the keyboard: no spawned-context stamps, so it stands in no thread.
fn human(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"))
        .env("NXC_TIMER", "dry")
        .env("NXC_TIMER_LOG", tmp.path().join("timer.log"));
    c
}

/// A spawned role session: the stamps `SidecarWorker` puts into a triggered role's environment.
fn persona(tmp: &TempDir, handle: &str, session: &str) -> Command {
    let mut c = human(tmp);
    c.env("NXC_ACTOR", handle).env("NXC_SESSION", session);
    c
}

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim())
        .expect("valid json")
}

/// The text a REFUSED `nxc … --json` call reports. Under `--json` a typed refusal is the envelope
/// `{"error":{"kind":…,"msg":…}}` on STDOUT, not a line on stderr — so a test that read stderr alone
/// would assert against the empty string and pass for every message, including no message at all.
fn refusal(cmd: &mut Command) -> String {
    let out = cmd.assert().failure();
    let o = out.get_output();
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

fn write_roles(tmp: &TempDir, handles: &[&str], channels: &str) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in handles {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
        )
        .unwrap();
    }
    std::fs::write(roles.join("channels.yaml"), channels).unwrap();
}

fn trigger_lines(tmp: &TempDir) -> Vec<String> {
    std::fs::read_to_string(tmp.path().join("dry.log"))
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

/// Which roles the engine has started, in start order — the observable SEQUENCE a flow produces.
fn started_roles(tmp: &TempDir) -> Vec<String> {
    trigger_lines(tmp)
        .iter()
        .map(|l| field(l, "role"))
        .collect()
}

/// The newest session the engine started for `role`.
fn session_of(tmp: &TempDir, role: &str) -> String {
    trigger_lines(tmp)
        .iter()
        .filter(|l| field(l, "role") == role)
        .map(|l| field(l, "session"))
        .next_back()
        .unwrap_or_else(|| panic!("no trigger for {role} in {:?}", trigger_lines(tmp)))
}

/// The whole dry-log text — the trigger MESSAGES embed real newlines, so a body is matched against
/// this rather than against one physical line.
fn dry_log(tmp: &TempDir) -> String {
    std::fs::read_to_string(tmp.path().join("dry.log")).unwrap_or_default()
}

/// **The NEWEST slot under `parent` that expects `handle`** — the "newest" matters because a cycle
/// opens a second slot for the same role. See the cycle suite's twin for the ULID argument.
fn newest_slot_for(tmp: &TempDir, parent: &str, handle: &str) -> String {
    let store = open_store(tmp);
    let want = format!("local/{handle}");
    store
        .supervised_children(parent)
        .unwrap()
        .into_iter()
        .rfind(|t| {
            store
                .thread_quorum(t, NOW)
                .unwrap()
                .is_some_and(|q| q.expects == [want.clone()])
        })
        .unwrap_or_else(|| panic!("no slot for {handle} under {parent}"))
}

/// Open the board and get to the point where the review step is waiting for its verdict.
fn up_to_the_first_review(tmp: &TempDir, channels: &str) -> (String, String) {
    write_roles(tmp, &["pm", "coder", "review", "finisher"], channels);
    let pm = json_of(human(tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board = json_of(persona(tmp, "pm", &pm).args([
        "--json",
        "send",
        "--to",
        "coding",
        "build the thing",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let build = newest_slot_for(tmp, &board, "coder");
    json_of(persona(tmp, "coder", &session_of(tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &build,
        "first cut is in",
    ]));
    (pm, board)
}

/// Drive `CYCLE` (`max_passes: 2`) to the state the ticket was written about: the ceiling reached,
/// the round handed back up to its requester, standing on a verdict nobody can discharge.
fn up_to_the_exhausted_ceiling(tmp: &TempDir) -> (String, String) {
    let (pm, board) = up_to_the_first_review(tmp, CYCLE);
    let check = newest_slot_for(tmp, &board, "review");
    json_of(persona(tmp, "review", &session_of(tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--needs-rework",
        "the lock order is wrong in two places",
    ]));
    let rework = newest_slot_for(tmp, &board, "coder");
    json_of(persona(tmp, "coder", &session_of(tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &rework,
        "second attempt",
    ]));
    let check2 = newest_slot_for(tmp, &board, "review");
    json_of(persona(tmp, "review", &session_of(tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check2,
        "--needs-rework",
        "still not right: the lock order",
    ]));
    assert_eq!(
        started_roles(tmp),
        vec!["pm", "coder", "review", "coder", "review", "pm"],
        "precondition: the ceiling escalated the round back to its requester and started no third \
         attempt"
    );
    (pm, board)
}

// ---- the door ---------------------------------------------------------------------------------

#[test]
fn accepting_continues_the_same_run_over_next_instead_of_starting_it_over() {
    // The whole item in one sequence. The requester has exactly one settled round standing on a
    // verdict, and until this verb its only two moves were "again" (a NEW run, step one, the same
    // ceiling re-armed) and "throw it away". This is the third, and it is the one that reaches the
    // step the round was cut off before.
    let tmp = workspace();
    let (pm, board) = up_to_the_exhausted_ceiling(&tmp);

    let accepted = json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "--accept",
        "the lock order is a follow-up, not a blocker. Ship it.",
    ]));
    assert_eq!(
        started_roles(&tmp),
        vec!["pm", "coder", "review", "coder", "review", "pm", "finisher"],
        "the step AFTER the one that rejected was started — not `coder` again, which is what an \
         ordinary answer would have started, and which is what re-armed the loop six times in the \
         run this item was opened on: {accepted}"
    );
    assert!(
        accepted["completed"].is_null(),
        "and the round did NOT end here — it is running again, one step further on: {accepted}"
    );

    // …and the run really is the same run: the step it opened carries the ORIGINAL request's mark,
    // so `max_passes` is not re-armed and the answers of the round are still the round's.
    let store = open_store(&tmp);
    let ship = newest_slot_for(&tmp, &board, "finisher");
    let check = store
        .supervised_children(&board)
        .unwrap()
        .into_iter()
        .find(|t| {
            store
                .flow_mark(t)
                .unwrap()
                .is_some_and(|m| m.step == "check")
        })
        .expect("the round ran a `check` step");
    assert_eq!(
        store.flow_mark(&ship).unwrap().map(|m| m.run),
        store.flow_mark(&check).unwrap().map(|m| m.run),
        "the accepted step belongs to the run that was accepted, not to a new one"
    );

    // And the round can now finish, which is the thing that had never once happened.
    let last = json_of(
        persona(&tmp, "finisher", &session_of(&tmp, "finisher"))
            .args(["--json", "reply", "--thread", &ship, "shipped"]),
    );
    assert!(
        last["completed"].is_object(),
        "the last step ends the run and consolidates: {last}"
    );
    assert_eq!(
        last["woke"], pm,
        "…and the requester hears about it: {last}"
    );
}

#[test]
fn the_accepted_step_is_handed_the_acceptance_and_the_verdict_it_overrode() {
    // The item's first open question — "Was bekommt der Folgeschritt gereicht?" — answered by the
    // measurement it was asked with: a successor given the reviewer's findings ALONE would work on
    // against a judgement that had just been overruled and have no way to tell.
    let tmp = workspace();
    let (pm, board) = up_to_the_exhausted_ceiling(&tmp);
    json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "--accept",
        "the lock order is a follow-up, not a blocker. Ship it.",
    ]));

    let ship = newest_slot_for(&tmp, &board, "finisher");
    let task = open_store(&tmp)
        .messages_in_thread(&ship)
        .unwrap()
        .first()
        .map(|m| m.body.clone())
        .expect("the accepted commission is the first message on the new slot");
    assert!(
        task.contains("ACCEPTED BY THE PARTY THAT COMMISSIONED THIS ROUND"),
        "the notice comes first, like every other hand-over in this engine: {task}"
    );
    assert!(
        task.contains("this is NOT the review passing"),
        "…and it says what it is NOT, because a reader that took this for a clean review would \
         report downstream that the work was reviewed clean: {task}"
    );
    assert!(
        task.contains("the lock order is a follow-up, not a blocker"),
        "…then the acceptance itself, which is the decision this step is running on: {task}"
    );
    assert!(
        task.contains("still not right: the lock order"),
        "…and the verdict that was set aside, so the next step knows what was overruled: {task}"
    );
    assert!(
        task.contains("\"check\""),
        "…named by the step whose judgement it was, so it can be found in the record: {task}"
    );
}

#[test]
fn an_acceptance_is_recorded_as_its_own_kind_so_the_override_stays_findable() {
    // The item asks for exactly this and says why: the state is "accepted by the owner", not
    // "skipped" and not "passed" — "das muss im Protokoll unterscheidbar bleiben, denn es ist eine
    // Uebersteuerung und die will man spaeter wiederfinden". A plain reply would be prose.
    let tmp = workspace();
    let (pm, board) = up_to_the_exhausted_ceiling(&tmp);
    json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "--accept",
        "good enough",
    ]));

    let overrides: Vec<String> = open_store(&tmp)
        .messages_in_thread(&board)
        .unwrap()
        .into_iter()
        .filter(|m| m.kind == nexus_chat::model::KIND_ACCEPTED)
        .map(|m| format!("{}: {}", m.sender, m.body))
        .collect();
    assert_eq!(
        overrides,
        vec!["local/pm: good enough".to_string()],
        "one label to ask the column for, and it names who overruled and why"
    );
}

#[test]
fn a_round_that_stopped_short_of_the_ceiling_can_be_accepted_too() {
    // The item's second open question — "Gilt das nur bei erschoepftem Deckel oder in jeder Runde?"
    // — answered as it recommended: the verb reads NO counter. What it needs is a verdict on a round
    // that has stopped, and the ceiling is only one of the ways a round stops.
    //
    // Here it stops the other way: the coder, sent back by the reviewer, escalates instead of
    // reworking — "I cannot do what the review asks without a decision". That is pass 2 of 3, well
    // inside the ceiling, and it is the shape the item's own second half describes as the producer's
    // ONLY legitimate move. The decision it asks for is this verb.
    let tmp = workspace();
    let roomy = CYCLE.replace("max_passes: 2", "max_passes: 3");
    let (pm, board) = up_to_the_first_review(&tmp, &roomy);
    let check = newest_slot_for(&tmp, &board, "review");
    json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--needs-rework",
        "rewrite the parser",
    ]));
    let rework = newest_slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &rework,
        "--escalate",
        "rewriting the parser is a week; I need a decision before I start",
    ]));
    assert_eq!(
        started_roles(&tmp),
        vec!["pm", "coder", "review", "coder", "pm"],
        "precondition: the escalation ended the chain and handed the round up — the ceiling was \
         never reached, so `max_passes: 3` still has an attempt left in it"
    );

    let accepted = json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "--accept",
        "no rewrite. The parser is good enough for this release.",
    ]));
    assert_eq!(
        started_roles(&tmp),
        vec!["pm", "coder", "review", "coder", "pm", "finisher"],
        "accepted inside the ceiling: nothing about this verb is a function of the counter — \
         {accepted}"
    );
}

// ---- the lock ---------------------------------------------------------------------------------

#[test]
fn a_party_serving_a_step_may_not_accept_the_verdict_on_its_own_work() {
    // The item's embedded ruling, and the half it decided AGAINST building: "Der Erzeuger darf
    // seinen eigenen Pruefer nicht fuer zufrieden erklaeren — das hebt genau die Trennung auf, um
    // derer willen der Kanal existiert." A producer that could discharge its own reviewer turns a
    // two-party round into a one-party one, and the whole apparatus into theatre.
    //
    // It is refused BY NAME and sent to `--escalate`, because a refusal that names no alternative
    // teaches the next session nothing and it tries again.
    let tmp = workspace();
    let (_pm, board) = up_to_the_first_review(&tmp, CYCLE);
    let check = newest_slot_for(&tmp, &board, "review");
    json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--needs-rework",
        "not yet",
    ]));
    let rework = newest_slot_for(&tmp, &board, "coder");

    let err = refusal(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &rework,
        "--accept",
        "I say it is fine",
    ]));
    assert!(
        err.contains("may not accept the verdict on its own work"),
        "the refusal has to say what the rule IS, not merely that there is one: {err}"
    );
    assert!(
        err.contains("--escalate"),
        "…and where the producer's own way up is: {err}"
    );
    assert_eq!(
        open_store(&tmp)
            .messages_in_thread(&rework)
            .unwrap()
            .iter()
            .filter(|m| m.body.contains("I say it is fine"))
            .count(),
        0,
        "and nothing was written: a refused acceptance must not leave a message that reads, \
         afterwards, exactly like one that worked"
    );
}

#[test]
fn an_acceptance_with_no_verdict_to_accept_is_refused_rather_than_posted() {
    // `--accept` overrules a JUDGEMENT. A round nobody sent back has none, and the thing the caller
    // probably wants there is the ordinary answer — so the refusal says so instead of quietly
    // posting a message that moves nothing.
    let tmp = workspace();
    let (pm, board) = up_to_the_first_review(&tmp, CYCLE);
    let before = open_store(&tmp).messages_in_thread(&board).unwrap().len();

    let err = refusal(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "--accept",
        "looks fine to me",
    ]));
    assert!(
        err.contains("nothing in this round was sent back"),
        "the refusal names the missing precondition: {err}"
    );
    assert_eq!(
        open_store(&tmp).messages_in_thread(&board).unwrap().len(),
        before,
        "…and it is a PREflight: nothing was written"
    );
}

#[test]
fn an_acceptance_while_a_step_still_owes_an_answer_is_refused_and_names_the_thread() {
    // **The MESSAGE half of the stillness gate, and only that half** (corrected after the
    // independent review of PR #470, Test Quality #3, which caught the name claiming more than the
    // body reaches). `CYCLE` declares no `working_tree:`, so `a_member_session_is_still_writing`
    // returns `false` before it ever asks the worker — and the dry worker this harness runs would
    // answer "nothing is alive" anyway. What refuses here is `!member_set_is_settled`: the rework
    // slot is open and owes an answer.
    //
    // The SESSION half — a round where every slot has answered and a process is still writing in
    // the checkout — cannot be reached from this harness at all, and is pinned at the library seam
    // by `seam::an_acceptance_is_refused_while_the_session_behind_an_answered_step_is_still_alive`.
    // The two are named apart deliberately: a test that says "still working" while proving "has not
    // answered" is the shape `nxm recall a-test-name-is-a-claim` was written about.
    let tmp = workspace();
    let roomy = CYCLE.replace("max_passes: 2", "max_passes: 3");
    let (pm, board) = up_to_the_first_review(&tmp, &roomy);
    let check = newest_slot_for(&tmp, &board, "review");
    json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--needs-rework",
        "not yet",
    ]));
    let rework = newest_slot_for(&tmp, &board, "coder");

    let err = refusal(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "--accept",
        "never mind, ship it",
    ]));
    assert!(
        err.contains("this round is still moving"),
        "the refusal says what state the round is in: {err}"
    );
    assert!(
        err.contains("have not answered"),
        "…and it is the MESSAGE half that refused, not the liveness one — the wording is what \
         tells them apart, and this fixture can only reach this one: {err}"
    );
    assert!(
        err.contains(&rework),
        "…naming the thread that has not answered, so the caller can go and look: {err}"
    );
    assert!(
        !err.contains("max_passes"),
        "…and it is NOT the counter that refused — the gate is stillness, which is the item's own \
         answer to whether this verb belongs to the ceiling alone: {err}"
    );
}

#[test]
fn the_ceiling_tells_the_requester_about_both_doors_and_not_only_about_another_attempt() {
    // A gate nobody is told about is not a gate. Until this item the ceiling's own note offered one
    // move — "commission the round again if another attempt is what you want" — and that is
    // precisely the move that re-arms `max_passes`. The human in the measured run took it six times,
    // because it was the only one on the page.
    let tmp = workspace();
    let (_pm, board) = up_to_the_first_review(&tmp, CYCLE);
    let check = newest_slot_for(&tmp, &board, "review");
    json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--needs-rework",
        "not yet",
    ]));
    let rework = newest_slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &rework,
        "second attempt",
    ]));
    let check2 = newest_slot_for(&tmp, &board, "review");
    let capped = json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check2,
        "--needs-rework",
        "still not right",
    ]));

    let delivered = capped["completed"]["delivered"].as_str().unwrap();
    assert!(
        delivered.contains("max_passes: 2"),
        "still says why the round stopped: {delivered}"
    );
    assert!(
        delivered.contains("--accept"),
        "…and now also names the other move, with the verb that takes it: {delivered}"
    );
    assert!(
        delivered.contains(&board),
        "…addressed at the thread the requester is to answer on, so it is one copy-paste: \
         {delivered}"
    );
}

#[test]
fn accepting_and_the_other_two_bits_are_refused_together_rather_than_ranked() {
    // Three bits, three different statements about three different pieces of work. Setting two says
    // two of them at once, and picking a winner would be this engine deciding which the caller
    // meant.
    let tmp = workspace();
    let (pm, board) = up_to_the_exhausted_ceiling(&tmp);
    for other in ["--escalate", "--needs-rework"] {
        let err = refusal(persona(&tmp, "pm", &pm).args([
            "--json", "reply", "--thread", &board, "--accept", other, "x",
        ]));
        assert!(
            err.contains("cannot be used with"),
            "clap refuses the pair before anything reaches the engine: {err}"
        );
    }
}

#[test]
fn the_dry_log_shows_the_accepted_step_was_started_fresh_and_not_resumed_into_the_reviewer() {
    // The accepted edge IS `next:`, reached early — so it takes the forward edge's default and
    // starts a fresh session for its own target, exactly as the same edge taken by a satisfied
    // reviewer would. Nothing about an override changes what the successor is.
    let tmp = workspace();
    let (pm, board) = up_to_the_exhausted_ceiling(&tmp);
    json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "--accept",
        "good enough",
    ]));
    let log = dry_log(&tmp);
    assert!(
        log.contains("role=finisher"),
        "the declared target of `ship` is what was started: {log}"
    );
    let ship = newest_slot_for(&tmp, &board, "finisher");
    assert_eq!(
        open_store(&tmp)
            .thread_quorum(&ship, NOW)
            .unwrap()
            .unwrap()
            .expects,
        vec!["local/finisher".to_string()],
        "and the new slot owes its answer to the step's own target"
    );
}

#[test]
fn a_round_that_has_already_gone_past_the_verdict_cannot_be_accepted_backwards() {
    // A run keeps every slot it ever opened, so "some step of this run once said no" stays true for
    // the rest of the round's life — including after the work was fixed, passed and shipped. An
    // acceptance read off that history would re-open a step the round is already past. What is
    // asked instead is the STATE: has the successor run?
    let tmp = workspace();
    let (_pm, board) = up_to_the_first_review(&tmp, CYCLE);
    let pm = session_of(&tmp, "pm");
    let check = newest_slot_for(&tmp, &board, "review");
    json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--needs-rework",
        "not yet",
    ]));
    let rework = newest_slot_for(&tmp, &board, "coder");
    json_of(
        persona(&tmp, "coder", &session_of(&tmp, "coder"))
            .args(["--json", "reply", "--thread", &rework, "fixed"]),
    );
    let check2 = newest_slot_for(&tmp, &board, "review");
    json_of(
        persona(&tmp, "review", &session_of(&tmp, "review"))
            .args(["--json", "reply", "--thread", &check2, "good now"]),
    );
    assert_eq!(
        started_roles(&tmp).last().map(String::as_str),
        Some("finisher"),
        "precondition: the reviewer became satisfied on its own and the round went on over `next`"
    );

    let err = refusal(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "--accept",
        "about that first review",
    ]));
    assert!(
        err.contains("already gone on past"),
        "the refusal names the state, not the history: {err}"
    );
}

#[test]
fn a_round_that_stopped_on_a_verdict_is_told_about_the_exit_however_it_stopped() {
    // A gate nobody is told about is not a gate — and the ceiling is only ONE of the ways a round
    // comes to rest on a verdict. Here the coder escalates during the rework, well inside the
    // ceiling: the round is handed up with the reviewer's judgement still standing, and the
    // delivery has to name the same exit the ceiling names, or the verb exists only for readers who
    // happened to hit `max_passes`.
    let tmp = workspace();
    let roomy = CYCLE.replace("max_passes: 2", "max_passes: 3");
    let (_pm, board) = up_to_the_first_review(&tmp, &roomy);
    let check = newest_slot_for(&tmp, &board, "review");
    json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--needs-rework",
        "rewrite the parser",
    ]));
    let rework = newest_slot_for(&tmp, &board, "coder");
    let stopped = json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &rework,
        "--escalate",
        "that is a week of work; I need a decision",
    ]));

    let delivered = stopped["completed"]["delivered"].as_str().unwrap();
    assert!(
        delivered.contains("--accept"),
        "a round that stopped short of its ceiling names the exit too: {delivered}"
    );
    assert!(
        delivered.contains("\"check\""),
        "…and says which judgement it is standing on: {delivered}"
    );
}

#[test]
fn a_round_that_finished_cleanly_is_offered_nothing() {
    // The offer and the refusal are the same question asked from two sides, which is what keeps a
    // delivery from advertising a move the engine would then decline. A round that reached its last
    // step has nothing to accept, so it says nothing extra — every healthy delivery reads exactly as
    // it did before this verb existed.
    let tmp = workspace();
    let (pm, board) = up_to_the_first_review(&tmp, CYCLE);
    let check = newest_slot_for(&tmp, &board, "review");
    json_of(
        persona(&tmp, "review", &session_of(&tmp, "review"))
            .args(["--json", "reply", "--thread", &check, "good"]),
    );
    let ship = newest_slot_for(&tmp, &board, "finisher");
    let done = json_of(
        persona(&tmp, "finisher", &session_of(&tmp, "finisher"))
            .args(["--json", "reply", "--thread", &ship, "shipped"]),
    );
    assert_eq!(done["woke"], pm, "precondition: the round finished: {done}");
    let delivered = done["completed"]["delivered"].as_str().unwrap();
    assert!(
        !delivered.contains("--accept"),
        "nothing was sent back, so nothing is offered: {delivered}"
    );
}

#[test]
fn the_round_a_ceiling_stopped_finishes_without_the_escalation_mark() {
    // The ceiling hands a round up AS AN ESCALATION — that is how "the wanted result was not
    // reached" travels, and it is unchanged. What must not survive the acceptance is the MARK: a
    // verdict is not an escalation, so once the accepted round reaches its last step it consolidates
    // clean, and the requester is not handed a finished round labelled as one still needing a
    // decision.
    //
    // (`supervisor_accept_verdict`'s own doc records the one shape where that label DOES survive —
    // a round stopped by a member's escalation rather than by the ceiling — and why it is written
    // down instead of patched. This test is the other half of that statement: the path the item was
    // measured on.)
    let tmp = workspace();
    let (pm, board) = up_to_the_exhausted_ceiling(&tmp);
    assert_eq!(
        open_store(&tmp).last_reply_escalated(&board).unwrap(),
        Some(true),
        "precondition: the ceiling handed the round up as an escalation"
    );

    json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "--accept",
        "good enough",
    ]));
    let ship = newest_slot_for(&tmp, &board, "finisher");
    let done = json_of(
        persona(&tmp, "finisher", &session_of(&tmp, "finisher"))
            .args(["--json", "reply", "--thread", &ship, "shipped"]),
    );
    assert!(done["completed"].is_object(), "the round finished: {done}");
    assert_eq!(
        open_store(&tmp).last_reply_escalated(&board).unwrap(),
        Some(false),
        "and it is delivered as a result, not as a round still standing on `I cannot`"
    );
}

// ---- the same verb at the library seam ---------------------------------------------------------
//
// Two things live here that the CLI harness above cannot reach. An embedding app sets
// `ReplyThreadRequest::accept` rather than typing a flag, and the rule about who may accept has to
// hold for it exactly as it does for `nxc` — it is enforced in the engine, not in the argument
// parser. And `DryWorker::session_is_running` answers `false` for everything, so the half of the
// stillness gate that asks about a LIVE SESSION never engages under it: every CLI test above sees a
// world in which no session is ever alive.

mod seam {
    use nexus_chat::definitions::Definitions;
    use nexus_chat::engine::{Engine, EngineConfig};
    use nexus_chat::orchestration::Caller;
    use nexus_chat::precondition::PreconditionOutcome;
    use nexus_chat::role::RoleDecl;
    use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
    use nexus_chat::timer::TimerConfig;
    use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
    use nexus_chat::workspace::{chat_config, setup};
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    const NOW: &str = "2026-09-13T10:00:00Z";

    /// The owner's cycle, declared `working_tree: exclusive` — which is the condition under which
    /// the engine asks about live sessions at all (a `shared` channel's two live writers are not a
    /// defect, and nothing about them is this verb's business).
    ///
    /// It also declares ONE hurdle, which is how a step is made to decline on demand: the worker
    /// below answers it, and a test that flips that answer gets a deterministic refused advance —
    /// the state `an_acceptance_that_started_nothing_says_so_in_the_record` needs and that nothing
    /// else in this suite can reach.
    const EXCLUSIVE_CYCLE: &str = concat!(
        "- name: coding\n  members: [coder, review, finisher]\n",
        "  working_tree: exclusive\n",
        "  preconditions:\n",
        "    - name: tree-clean\n      run: git status --porcelain\n      expect: \"\"\n",
        "  steps:\n",
        "    - id: build\n      target: coder\n      next: check\n",
        "    - id: check\n      target: review\n      on_needs_rework: build\n",
        "      max_passes: 2\n      next: ship\n",
        "    - id: ship\n      target: finisher\n",
    );

    const ROLES: &str = "handle: pm\nsystem_prompt: p\n;handle: coder\nsystem_prompt: c\n;\
                         handle: review\nsystem_prompt: r\n;handle: finisher\nsystem_prompt: f\n";

    /// Keeps every request, and answers the one read the engine makes of a worker: which of the
    /// sessions it started are still running. That second half is the whole reason this is not the
    /// dry worker — see the module header.
    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<TriggerRequest>>,
        running: Mutex<std::collections::HashSet<String>>,
        /// Whether the channel's declared hurdle answers NO from now on — the switch that makes an
        /// advance decline deterministically, with no process and no clock in it.
        hurdle_refuses: Mutex<bool>,
    }

    impl Recorder {
        fn newest_for(&self, handle: &str) -> TriggerRequest {
            let seen = self.seen.lock().unwrap();
            match seen.iter().rev().find(|r| r.role.handle == handle) {
                Some(req) => req.clone(),
                None => {
                    let handles: Vec<&str> = seen.iter().map(|r| r.role.handle.as_str()).collect();
                    panic!("no trigger for {handle}, only: {handles:?}")
                }
            }
        }

        fn started(&self) -> Vec<String> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.role.handle.clone())
                .collect()
        }

        /// The process behind `session` has exited — what a host worker learns from its runtime.
        fn exited(&self, session: &str) {
            self.running.lock().unwrap().remove(session);
        }
    }

    impl Worker for Recorder {
        fn trigger(&self, req: TriggerRequest) -> TriggerResult {
            self.running
                .lock()
                .unwrap()
                .insert(req.internal_session.clone());
            self.seen.lock().unwrap().push(req);
            Ok(TriggerOutcome::Accepted)
        }

        fn session_is_running(&self, internal_session: &str) -> bool {
            self.running.lock().unwrap().contains(internal_session)
        }

        /// The channel's one declared hurdle, answered from the switch above — a clean exit 0 until
        /// a test says otherwise, and then a verdict of NO.
        fn run_precondition(&self, _command: &str) -> PreconditionOutcome {
            let refuses = *self.hurdle_refuses.lock().unwrap();
            PreconditionOutcome::Ran {
                status: Some(if refuses { 1 } else { 0 }),
                stdout: if refuses {
                    " M crates/chat/src/lib.rs".to_string()
                } else {
                    String::new()
                },
                stderr: String::new(),
            }
        }
    }

    fn team(tmp: &TempDir) -> (Engine, Arc<Recorder>) {
        setup(tmp.path(), &chat_config()).expect("seed chat workspace");
        let roles: Vec<RoleDecl> = ROLES
            .split(';')
            .map(|r| serde_yaml::from_str(r).expect("role parses"))
            .collect();
        let defs = Definitions::new(
            roles,
            serde_yaml::from_str(EXCLUSIVE_CYCLE).expect("channels parse"),
        )
        .expect("catalogue");
        super::common::write_declarations(tmp.path(), &defs);
        let worker = Arc::new(Recorder::default());
        let engine = Engine::open_with(
            None,
            tmp.path(),
            EngineConfig {
                worker: WorkerConfig::Custom(worker.clone()),
                timer: TimerConfig::Dry,
                ..EngineConfig::default()
            },
        )
        .expect("open engine");
        (engine, worker)
    }

    fn caller(actor: &str) -> Caller<'_> {
        Caller {
            session: None,
            actor: Some(actor),
            now: Some(NOW),
        }
    }

    fn answer(engine: &Engine, who: &str, req: &TriggerRequest, body: &str, rework: bool) {
        engine
            .reply_thread(
                caller(who),
                ReplyThreadRequest {
                    machine: None,
                    thread: req
                        .reply_thread
                        .as_deref()
                        .expect("the step owes an answer"),
                    body,
                    escalate: false,
                    needs_rework: rework,
                    accept: false,
                },
            )
            .expect("the reply settles the step");
    }

    /// The answer AND the end of the session behind it — a step is over when its SESSION is over
    /// (nxf 6j6v.10yb), and on an `exclusive` channel nothing advances until it is.
    fn answer_and_finish(engine: &Engine, who: &str, req: &TriggerRequest, body: &str, rw: bool) {
        answer(engine, who, req, body, rw);
        engine
            .session_ended(caller(who), &req.internal_session)
            .expect("the runtime reports the process gone");
    }

    #[test]
    fn an_acceptance_is_refused_while_the_session_behind_an_answered_step_is_still_alive() {
        // The half of the gate the CLI suite cannot see. Every slot of the run has ANSWERED — the
        // board says settled — and the coder's process is still writing in the checkout. Opening the
        // successor here is nxf 6j6v.10yb's measured damage, caused on purpose, so the verb declines
        // and says what it is waiting for.
        let tmp = TempDir::new().unwrap();
        let (engine, worker) = team(&tmp);
        let board = engine
            .send_to(
                caller("pm"),
                SendToRequest {
                    machine: None,
                    to: "coding",
                    body: "build the thing",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("the channel is addressed")
            .thread_id;

        answer_and_finish(
            &engine,
            "coder",
            &worker.newest_for("coder"),
            "first cut",
            false,
        );
        answer_and_finish(
            &engine,
            "review",
            &worker.newest_for("review"),
            "the lock order is wrong",
            true,
        );
        // The rework answers and does NOT announce its end: the message is in, the process is not.
        answer(
            &engine,
            "coder",
            &worker.newest_for("coder"),
            "second cut",
            false,
        );

        let err = engine
            .reply_thread(
                caller("pm"),
                ReplyThreadRequest {
                    machine: None,
                    thread: &board,
                    body: "never mind the lock order, ship it",
                    escalate: false,
                    needs_rework: false,
                    accept: true,
                },
            )
            .expect_err("a live session in the checkout must refuse the acceptance");
        assert!(
            err.to_string().contains("still writing"),
            "the refusal names what is being waited for, and it is not a message: {err}"
        );
        assert_eq!(
            worker.started(),
            vec!["coder", "review", "coder"],
            "and nothing was opened beside the session that is still going"
        );
    }

    #[test]
    fn the_seam_refuses_accept_paired_with_either_other_bit_and_not_only_the_command_line() {
        // **The gate the CLI-level test cannot be** (independent review of PR #470, Test Quality #1,
        // High). `--accept` is refused beside the other two bits by clap's `conflicts_with_all`, and
        // an EMBEDDING HOST never goes through clap — it fills in a request struct. nxf 6j6v.am8j
        // rewrote `refuse_two_verdicts` from a pair check into a count of the set bits precisely
        // because a third bit had arrived, and left that rewrite covered only at the parser. The
        // sibling test for the original two-bit combination
        // (`a_declared_cycle_sends_the_work_back.rs`'s
        // `the_seam_refuses_both_declared_bits_too_and_not_only_the_command_line`) argues this at
        // length; this is its `accept`-bearing analogue, and it asks BOTH new pairs, because a
        // count that got one of them wrong would pass a test that asked only the other.
        let tmp = TempDir::new().unwrap();
        let (engine, worker) = team(&tmp);
        let board = engine
            .send_to(
                caller("pm"),
                SendToRequest {
                    machine: None,
                    to: "coding",
                    body: "build the thing",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("the channel is addressed")
            .thread_id;
        answer_and_finish(
            &engine,
            "coder",
            &worker.newest_for("coder"),
            "first cut",
            false,
        );
        answer_and_finish(&engine, "review", &worker.newest_for("review"), "no", true);
        answer_and_finish(
            &engine,
            "coder",
            &worker.newest_for("coder"),
            "second cut",
            false,
        );
        let check2 = worker.newest_for("review");
        answer_and_finish(&engine, "review", &check2, "still no", true);
        worker.exited(&check2.internal_session);

        for (escalate, needs_rework) in [(true, false), (false, true)] {
            let err = engine
                .reply_thread(
                    caller("pm"),
                    ReplyThreadRequest {
                        machine: None,
                        thread: &board,
                        body: "two things at once",
                        escalate,
                        needs_rework,
                        accept: true,
                    },
                )
                .expect_err("a reply carries at most one declared bit");
            assert!(
                err.msg.contains("at most one declared bit"),
                "escalate={escalate} needs_rework={needs_rework}: {err:?}"
            );
        }

        // …and the refusal is a PREflight on this path too: nothing of either attempt was written,
        // so the round is still standing exactly where the reviewer left it and a proper acceptance
        // still works.
        engine
            .reply_thread(
                caller("pm"),
                ReplyThreadRequest {
                    machine: None,
                    thread: &board,
                    body: "one thing: ship it",
                    escalate: false,
                    needs_rework: false,
                    accept: true,
                },
            )
            .expect("one bit is a state");
        assert_eq!(
            worker.started(),
            vec!["coder", "review", "coder", "review", "finisher"],
            "the two refused calls left the round untouched"
        );
    }

    #[test]
    fn an_acceptance_that_started_nothing_says_so_in_the_record() {
        // **The audit trail may not diverge from what happened** (independent review of PR #470,
        // Integrity & Robustness #2). An advance can decline — here the channel's declared hurdle
        // answers NO at the moment the acceptance is given — and the `accepted` message is already
        // durable by then. The receipt tells the caller, and the receipt is not stored; the message
        // is. Left alone the thread would carry an acceptance no later reader could tell from one
        // that worked, which is this verb's own justification undone.
        let tmp = TempDir::new().unwrap();
        let (engine, worker) = team(&tmp);
        let board = engine
            .send_to(
                caller("pm"),
                SendToRequest {
                    machine: None,
                    to: "coding",
                    body: "build the thing",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("the channel is addressed")
            .thread_id;
        answer_and_finish(
            &engine,
            "coder",
            &worker.newest_for("coder"),
            "first cut",
            false,
        );
        answer_and_finish(&engine, "review", &worker.newest_for("review"), "no", true);
        answer_and_finish(
            &engine,
            "coder",
            &worker.newest_for("coder"),
            "second cut",
            false,
        );
        let check2 = worker.newest_for("review");
        answer_and_finish(&engine, "review", &check2, "still no", true);
        worker.exited(&check2.internal_session);

        // The world changes under the round: the checkout is dirty now, and the channel says a step
        // may not start into that.
        *worker.hurdle_refuses.lock().unwrap() = true;

        let receipt = engine
            .reply_thread(
                caller("pm"),
                ReplyThreadRequest {
                    machine: None,
                    thread: &board,
                    body: "the lock order is a follow-up. Ship it.",
                    escalate: false,
                    needs_rework: false,
                    accept: true,
                },
            )
            .expect("the acceptance itself is posted — nothing after the write may fail the call");
        assert_eq!(
            worker.started(),
            vec!["coder", "review", "coder", "review"],
            "no step was started: the hurdle refused it"
        );
        assert!(
            !receipt.warnings.is_empty(),
            "…and the caller is told on the receipt of its own call: {receipt:?}"
        );

        // The durable half, which is the finding: the round's own thread says the acceptance did not
        // take effect, right beside the acceptance.
        let bodies: Vec<String> = super::open_store(&tmp)
            .messages_in_thread(&board)
            .unwrap()
            .into_iter()
            .map(|m| m.body)
            .collect();
        assert!(
            bodies.iter().any(|b| b.contains("did NOT take effect")),
            "the record has to say it, not only the receipt: {bodies:?}"
        );
        assert_eq!(
            bodies
                .iter()
                .filter(|b| b.contains("the lock order is a follow-up"))
                .count(),
            1,
            "…beside the acceptance, which is still there — this is a note, not a retraction"
        );
    }

    #[test]
    fn the_whole_verb_runs_at_the_library_seam() {
        // `accept: true` on the request an app builds, doing exactly what the flag does — the rule
        // about who may accept is the engine's, not the argument parser's, so an app gets it too.
        let tmp = TempDir::new().unwrap();
        let (engine, worker) = team(&tmp);
        let board = engine
            .send_to(
                caller("pm"),
                SendToRequest {
                    machine: None,
                    to: "coding",
                    body: "build the thing",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("the channel is addressed")
            .thread_id;

        answer_and_finish(
            &engine,
            "coder",
            &worker.newest_for("coder"),
            "first cut",
            false,
        );
        answer_and_finish(&engine, "review", &worker.newest_for("review"), "no", true);

        // The party serving a step may not do this, at this seam either — asked here, while its own
        // rework slot is live, because that is the moment the rule is ABOUT. (On a slot of a round
        // that is already over, `reply`'s older refusal answers first and says so, which is the
        // accurate statement for that state.)
        let refused = engine
            .reply_thread(
                caller("coder"),
                ReplyThreadRequest {
                    machine: None,
                    thread: worker
                        .newest_for("coder")
                        .reply_thread
                        .as_deref()
                        .expect("the coder's rework slot"),
                    body: "I say it is fine",
                    escalate: false,
                    needs_rework: false,
                    accept: true,
                },
            )
            .expect_err("a producer may not accept its own reviewer");
        assert!(
            refused
                .to_string()
                .contains("may not accept the verdict on its own work"),
            "{refused}"
        );

        answer_and_finish(
            &engine,
            "coder",
            &worker.newest_for("coder"),
            "second cut",
            false,
        );
        let check2 = worker.newest_for("review");
        answer_and_finish(&engine, "review", &check2, "still no", true);
        worker.exited(&check2.internal_session);
        assert_eq!(
            worker.started(),
            vec!["coder", "review", "coder", "review"],
            "precondition: `max_passes: 2` is spent and no third attempt was started"
        );

        engine
            .reply_thread(
                caller("pm"),
                ReplyThreadRequest {
                    machine: None,
                    thread: &board,
                    body: "the lock order is a follow-up. Ship it.",
                    escalate: false,
                    needs_rework: false,
                    accept: true,
                },
            )
            .expect("the party that commissioned the round may");
        assert_eq!(
            worker.started(),
            vec!["coder", "review", "coder", "review", "finisher"],
            "the step after the one that rejected was started, through the handle"
        );
        let ship = worker.newest_for("finisher");
        assert!(
            ship.message
                .contains("ACCEPTED BY THE PARTY THAT COMMISSIONED THIS ROUND")
                && ship.message.contains("the lock order is a follow-up")
                && ship.message.contains("still no"),
            "and it was handed the notice, the acceptance and the verdict that was set aside: {}",
            ship.message
        );
    }
}
