//! **A channel declares a CYCLE, and the run goes back over it** (nxf 6j6v.553s, questions (d) and
//! (a)) — `steps:` beside `members:`, the fixed verdict bit `--needs-rework`, and the ceiling on the
//! back edge.
//!
//! Until this item `coder -> review -> coder` could not be declared at all. A `flow: sequential`
//! channel IS its `members:` list, and `channel::validate_channels`' check 6 refused a repeated
//! target for a load-bearing reason: the engine matched a slot thread to its step by that name, so a
//! repeat made two steps indistinguishable. The consequence was measured rather than theoretical —
//! with no way back, the review step in the proving ground became a flat member of the chain and
//! NOBODY mitigated inside a round; every finding cost a whole second round with a fresh coder.
//!
//! **Three harnesses, and each is here because the one above it cannot answer its question.** The
//! routing tests run the real `nxc` binary against the dry worker — nothing builds a thread, a
//! parent edge or an expectation by hand. The `offer` module goes through the library seam with a
//! worker that KEEPS what it was handed, because the offer lives in the system prompt and the dry
//! log carries only the trigger message; that worker also answers "is this session still running",
//! which the dry one always denies. And the `#[ignore]`d live test at the bottom runs real SDK
//! sessions, because whether a real model reaches for the third ending is not a thing any of the
//! above can show.

#[path = "common/mod.rs"]
mod common;

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-08-31T10:00:00Z";

/// The owner's own example from the 2026-08-29 note, as a workspace declares it.
const CYCLE: &str = "- name: coding\n  members: [coder, review, finisher]\n  steps:\n    \
                     - id: build\n      target: coder\n      next: check\n    \
                     - id: check\n      target: review\n      on_needs_rework: build\n      \
                     max_passes: 3\n      next: ship\n    \
                     - id: ship\n      target: finisher\n";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

/// The human at the keyboard: no spawned-context stamps, so it stands in no thread. Deliberately
/// NOT `NXF_DETERMINISTIC_IDS` — that counter is shared by messages, threads and sessions, and in a
/// chain this long a thread id would collide with an unrelated message id (nxf 6j6v.k8bw).
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

/// **The NEWEST slot under `parent` that expects `handle`** — and the "newest" is the whole point
/// here, because a cycle opens a second slot for the same role. `supervised_children` sorts by
/// thread id, ids are ULIDs, and an ordered flow mints one slot per `nxc` process milliseconds
/// apart, so the last one really is the one just opened. (Do not copy this onto a `parallel`
/// channel, whose whole set is minted inside one millisecond.)
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

// ---- (d): the cycle is declarable, and the run really goes round it ---------------------------

#[test]
fn a_verdict_sends_the_run_back_to_the_step_the_channel_names_and_the_flow_finishes() {
    // The whole item, end to end: build -> check -> (needs rework) -> build -> check -> ship.
    // The second `build` is the SAME declared step running a second time, which is exactly what a
    // flat `members:` list could not express.
    let tmp = workspace();
    let (pm, board) = up_to_the_first_review(&tmp, CYCLE);
    assert_eq!(
        started_roles(&tmp),
        vec!["pm", "coder", "review"],
        "step one settled, so the channel opened the step its declaration names as `next`"
    );

    // The verdict. This is the bit that did not exist: not "I cannot" (that ends the chain), but
    // "what you handed me does not meet the standard".
    let check = newest_slot_for(&tmp, &board, "review");
    let verdict = json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--needs-rework",
        "the lock order is wrong in two places",
    ]));
    assert_eq!(
        started_roles(&tmp),
        vec!["pm", "coder", "review", "coder"],
        "the back edge ran: `check` sent the work to `build`, not on to `ship` — {verdict}"
    );
    assert!(
        !open_store(&tmp)
            .thread_quorum(&board, NOW)
            .unwrap()
            .unwrap()
            .complete,
        "and the requester has NOT been answered: the round is still going"
    );

    // What the coder was handed: the notice first, then the findings, which ARE the task.
    let log = dry_log(&tmp);
    assert!(
        log.contains("NEEDS REWORK — this is NOT an approval"),
        "the second coder turn must open with the notice: {log}"
    );
    assert!(
        log.contains("This is pass 2 of 3."),
        "…with the counter, so it can tell whether another attempt exists: {log}"
    );
    assert!(
        log.contains("the lock order is wrong in two places"),
        "…and with the verdict itself, which is the work: {log}"
    );

    // Round two: the coder answers, the SAME `check` step runs again, and this time it approves.
    let rework = newest_slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &rework,
        "lock order fixed",
    ]));
    assert_eq!(
        started_roles(&tmp),
        vec!["pm", "coder", "review", "coder", "review"],
        "a step may run twice in one run — `id` is what tells the two occurrences apart"
    );
    let check2 = newest_slot_for(&tmp, &board, "review");
    json_of(
        persona(&tmp, "review", &session_of(&tmp, "review"))
            .args(["--json", "reply", "--thread", &check2, "good now"]),
    );
    assert_eq!(
        started_roles(&tmp),
        vec!["pm", "coder", "review", "coder", "review", "finisher"],
        "a plain answer takes `next`, so the run reaches the last step"
    );

    let ship = newest_slot_for(&tmp, &board, "finisher");
    let last = json_of(
        persona(&tmp, "finisher", &session_of(&tmp, "finisher"))
            .args(["--json", "reply", "--thread", &ship, "shipped"]),
    );
    assert!(
        last["completed"].is_object(),
        "the step with no `next` ends the run and consolidates: {last}"
    );
    assert_eq!(
        last["woke"], pm,
        "…and the requester hears about it: {last}"
    );
}

#[test]
fn an_escalation_holds_the_run_where_it_stands_instead_of_sending_it_back() {
    // The other two thirds of the trichotomy, told apart. `--escalate` is a statement about the
    // sender's OWN work and ends the chain (nxf 6j6v.ma7v); `--needs-rework` is about somebody
    // else's and continues it. If the two were one bit, this round would restart the coder.
    let tmp = workspace();
    let (pm, board) = up_to_the_first_review(&tmp, CYCLE);

    let check = newest_slot_for(&tmp, &board, "review");
    let out = json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--escalate",
        "I have no access to the repository",
    ]));
    assert_eq!(
        started_roles(&tmp),
        vec!["pm", "coder", "review", "pm"],
        "no STEP was started — not `ship` via `next`, not `build` via the back edge. The trailing \
         `pm` is the requester being woken with the round, which is what an ended chain does — {out}"
    );
    assert!(
        out["completed"].is_object(),
        "the round is handed up as it stands: {out}"
    );
    assert_eq!(out["woke"], pm, "…to whoever asked: {out}");
    assert_eq!(
        open_store(&tmp).last_reply_escalated(&board).unwrap(),
        Some(true),
        "and the channel's own answer of record carries the escalation one level up"
    );
}

#[test]
fn the_ceiling_on_the_back_edge_escalates_upward_instead_of_stopping_quietly() {
    // Owner, 2026-08-29: "Wenn der Wert erreicht wird, wuerde dann automatisch hoch eskaliert
    // werden; der Coder wuerde in diesem Fall keinen neuen Anlauf starten." A ceiling that merely
    // stopped would leave a requester waiting on a round that had silently given up.
    let tmp = workspace();
    let capped = CYCLE.replace("max_passes: 3", "max_passes: 2");
    let (pm, board) = up_to_the_first_review(&tmp, &capped);

    // Pass 1 -> 2: allowed.
    let check = newest_slot_for(&tmp, &board, "review");
    json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--needs-rework",
        "not yet",
    ]));
    assert_eq!(started_roles(&tmp).len(), 4, "the first back edge ran");

    // Pass 2 -> 3: over the ceiling.
    let rework = newest_slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &rework,
        "second attempt",
    ]));
    let check2 = newest_slot_for(&tmp, &board, "review");
    let capped_out = json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check2,
        "--needs-rework",
        "still not right",
    ]));
    assert_eq!(
        started_roles(&tmp),
        vec!["pm", "coder", "review", "coder", "review", "pm"],
        "no third attempt was started — the trailing `pm` is the requester being woken, not a \
         step: {capped_out}"
    );
    assert!(
        capped_out["completed"].is_object(),
        "the round ends here rather than hanging: {capped_out}"
    );
    assert_eq!(
        capped_out["woke"], pm,
        "…and it goes UP, to whoever asked: {capped_out}"
    );
    assert_eq!(
        open_store(&tmp).last_reply_escalated(&board).unwrap(),
        Some(true),
        "as an ESCALATION — the one declared form for `the wanted result was not reached`"
    );
    let delivered = capped_out["completed"]["delivered"].as_str().unwrap();
    assert!(
        delivered.contains("max_passes: 2"),
        "and the requester is told WHY the round stopped, not left to count rounds: {delivered}"
    );
    assert!(
        delivered.contains("still not right"),
        "…with every answer of the round below it: {delivered}"
    );
}

#[test]
fn commissioning_the_round_again_is_a_new_run_and_the_counter_starts_over() {
    // Owner, 2026-08-29: "Bekommt er aber die Erlaubnis von oben, wird max_passes wieder auf Null
    // gesetzt und weiter gemacht." NOTHING resets it: a run is identified by the request standing on
    // the channel thread, so a follow-up IS a new run and the count is derived from that run's own
    // slots. The reset is the absence of a mechanism, which is why it needs a test of its own.
    let tmp = workspace();
    let capped = CYCLE.replace("max_passes: 3", "max_passes: 2");
    let (pm, board) = up_to_the_first_review(&tmp, &capped);

    // Run one spends its whole budget: pass 2 is allowed, pass 3 is over the ceiling.
    let check = newest_slot_for(&tmp, &board, "review");
    json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--needs-rework",
        "no",
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
    json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check2,
        "--needs-rework",
        "still no",
    ]));
    assert_eq!(
        started_roles(&tmp),
        vec!["pm", "coder", "review", "coder", "review", "pm"],
        "run one is spent: the ceiling escalated to the requester instead of starting a third \
         attempt"
    );

    // The requester answers the escalation by commissioning the round again. That is a NEW run.
    json_of(persona(&tmp, "pm", &pm).args(["--json", "reply", "--thread", &board, "try again"]));
    let build2 = newest_slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &build2,
        "third cut",
    ]));
    let check3 = newest_slot_for(&tmp, &board, "review");
    let again = json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check3,
        "--needs-rework",
        "closer",
    ]));
    assert_eq!(
        started_roles(&tmp),
        vec!["pm", "coder", "review", "coder", "review", "pm", "coder", "review", "coder"],
        "the new run has its own budget — this verdict is ITS pass 2, not the previous run's \
         pass 4: {again}"
    );
    assert!(
        dry_log(&tmp).contains("This is pass 2 of 2."),
        "…and the counter says so: {}",
        dry_log(&tmp)
    );
}

#[test]
fn a_verdict_a_step_declares_no_edge_for_falls_away_and_is_reported() {
    // Owner, 2026-08-29: the bit is not offered where no edge is declared, and one set anyway is
    // dropped — "REINE DIAGNOSE, keine Entscheidung und nicht in der Nachricht". Dropping it
    // SILENTLY is what would leave an under-declared channel looking like a broken one.
    let tmp = workspace();
    let no_edge = "- name: coding\n  members: [coder, review]\n  steps:\n    \
                   - id: build\n      target: coder\n      next: check\n    \
                   - id: check\n      target: review\n";
    write_roles(&tmp, &["pm", "coder", "review"], no_edge);
    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board = json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "go"]))
        ["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let build = newest_slot_for(&tmp, &board, "coder");
    json_of(
        persona(&tmp, "coder", &session_of(&tmp, "coder"))
            .args(["--json", "reply", "--thread", &build, "done"]),
    );

    let check = newest_slot_for(&tmp, &board, "review");
    let out = json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--needs-rework",
        "meh",
    ]));
    let warnings = out["warnings"].as_array().expect("a warnings list");
    assert!(
        warnings
            .iter()
            .any(|w| w["class"] == "verdict_dropped" && w["thread"] == check.as_str()),
        "the drop is on the receipt of the very reply that carried it: {out}"
    );
    assert!(
        out["completed"].is_object(),
        "…and the run went on exactly as a plain answer would have: it was the last step, so the \
         channel consolidated — {out}"
    );
}

#[test]
fn the_working_copy_is_free_again_once_a_cycle_has_run_its_course() {
    // **The mutation guard for `working_tree::hands_the_task_back`'s `NeedsRework` arm.** A verdict
    // LOOKS like a hand-back and is the opposite of one for a working copy: the run continues, and
    // the next step's own obligation is what holds the lease.
    //
    // Sorting it into "hands the task back" instead would be an over-hold with NO self-clearing
    // door. A settled slot is never re-declared — each pass gets a fresh one — so the reviewer's
    // thread would keep reading `needs_rework` for the rest of this workspace's life and the copy
    // would stay claimed to `WORKING_TREE_LEASE_BOUND` past a round that finished cleanly. Flip that
    // arm and this test is what goes red.
    let tmp = workspace();
    let exclusive = CYCLE.replace(
        "- name: coding\n  members: [coder, review, finisher]\n",
        "- name: coding\n  members: [coder, review, finisher]\n  working_tree: exclusive\n",
    );
    write_roles(&tmp, &["pm", "coder", "review", "finisher"], &exclusive);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]));
    let (pm, pm_thread) = (
        opened["session"].as_str().unwrap().to_string(),
        opened["thread_id"].as_str().unwrap().to_string(),
    );
    let board = json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "send",
        "--to",
        "coding",
        "build the thing",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let build = newest_slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &build,
        "first cut is in",
    ]));

    let check = newest_slot_for(&tmp, &board, "review");
    json_of(persona(&tmp, "review", &session_of(&tmp, "review")).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--needs-rework",
        "not yet",
    ]));
    let holding = json_of(
        persona(&tmp, "coder", &session_of(&tmp, "coder"))
            .args(["--json", "threads", "show", &board]),
    )["working_tree"]
        .clone();
    assert_eq!(
        holding, "holding",
        "mid-cycle the claim is still the run's — which is what the notice tells the party being \
         sent back, so the two must agree"
    );

    // Round two, to the end of the declared run.
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
    let ship = newest_slot_for(&tmp, &board, "finisher");
    json_of(
        persona(&tmp, "finisher", &session_of(&tmp, "finisher"))
            .args(["--json", "reply", "--thread", &ship, "shipped"]),
    );
    // …and the requester, woken with the round, closes the chain it opened. Until it does, the
    // claim area still has an obligation outstanding one level up and the lease is correctly held —
    // which is the quorum rule, not this arm.
    json_of(persona(&tmp, "pm", &pm).args(["--json", "reply", "--thread", &pm_thread, "done"]));

    let after = json_of(
        persona(&tmp, "coder", &session_of(&tmp, "coder"))
            .args(["--json", "threads", "show", &board]),
    )["working_tree"]
        .clone();
    assert_eq!(
        after,
        Value::Null,
        "the run finished cleanly, so the working copy goes back — a verdict two steps ago must \
         not hold it forever"
    );
}

#[test]
fn setting_both_declared_bits_is_refused_before_anything_is_written() {
    // They are statements about DIFFERENT work with opposite consequences. Picking a winner would
    // be the engine deciding which of two things the caller meant.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "review", "finisher"], CYCLE);
    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    persona(&tmp, "pm", &pm)
        .args([
            "reply",
            "--thread",
            "th-nonexistent",
            "--escalate",
            "--needs-rework",
            "both",
        ])
        .assert()
        .failure();
}

#[test]
fn one_checker_of_a_quorum_is_enough_to_send_the_round_back_and_the_fold_still_runs() {
    // **ANY-MEMBER-WINS** (owner, 2026-08-29: "In unserem angedachten Review Kanal duerfte jeder
    // Reviewer dieses Bit setzen und der Kanal-Koordinator muss ... auch das Bit in Richtung Coder
    // setzen"). A checker that found something must not be outvoted by one that did not.
    //
    // And the channel it happens in is the SHIPPED shape: `on_complete: summarize`, three answers
    // folded to one judgement. That is why the read walks the inner MEMBERS rather than the inner
    // channel's discharge — a fold's discharge is the synthesizer's own reply, whose kind the engine
    // does not author, so a verdict would have nothing to ride on. Carrying it by refusing to fold
    // (which is what an ESCALATION does, nxf 6j6v.e9qj) would send the coder three separate reviews
    // instead of the one judgement the channel was declared to produce.
    let tmp = workspace();
    let quorum = "- name: coding\n  members: [coder, review, finisher]\n  steps:\n    \
                  - id: build\n      target: coder\n      next: check\n    \
                  - id: check\n      target: review\n      on_needs_rework: build\n      \
                  max_passes: 3\n      next: ship\n    \
                  - id: ship\n      target: finisher\n\
                  - name: review\n  members: [checkera, checkerb]\n  on_complete: summarize\n  \
                  summary_prompt: Fold the verdicts into one judgement.\n";
    write_roles(
        &tmp,
        &["pm", "coder", "checkera", "checkerb", "finisher"],
        quorum,
    );
    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board = json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "send",
        "--to",
        "coding",
        "build the thing",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let build = newest_slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &build,
        "first cut is in",
    ]));

    // Step `check` is a whole CHANNEL, so its slot is that channel's own thread and its members
    // stand one level further down.
    let store = open_store(&tmp);
    let review_thread = store
        .supervised_children(&board)
        .unwrap()
        .into_iter()
        .next_back()
        .expect("the review channel's own thread");
    drop(store);
    let a = newest_slot_for(&tmp, &review_thread, "checkera");
    let b = newest_slot_for(&tmp, &review_thread, "checkerb");

    json_of(
        persona(&tmp, "checkera", &session_of(&tmp, "checkera")).args([
            "--json",
            "reply",
            "--thread",
            &a,
            "--needs-rework",
            "the lock order is wrong",
        ]),
    );
    json_of(
        persona(&tmp, "checkerb", &session_of(&tmp, "checkerb")).args([
            "--json",
            "reply",
            "--thread",
            &b,
            "reads fine to me",
        ]),
    );

    // The set settled without an escalation, so the declared FOLD runs — one synthesizer, exactly as
    // it would for a clean round.
    assert!(
        started_roles(&tmp).contains(&"__synth__".to_string()),
        "the verdict must not turn the declared fold off: {:?}",
        started_roles(&tmp)
    );
    json_of(
        persona(&tmp, "__synth__", &session_of(&tmp, "__synth__")).args([
            "--json",
            "reply",
            "--thread",
            &review_thread,
            "One checker found a real problem; the round is not clean.",
        ]),
    );

    assert_eq!(
        started_roles(&tmp).iter().filter(|r| *r == "coder").count(),
        2,
        "one dissenting checker sent the whole round back: {:?}",
        started_roles(&tmp)
    );
    let log = dry_log(&tmp);
    assert!(
        log.contains("One checker found a real problem"),
        "…and what the coder was handed is the channel's FOLDED judgement, not three raw reviews: \
         {log}"
    );
    assert!(
        log.contains("This is pass 2 of 3."),
        "…with the run's counter, which the quorum hop does not reset: {log}"
    );
}

// ---- (a): the bit is OFFERED per step, at the forced ending ------------------------------------
//
// The dry log carries the trigger MESSAGE and not the system prompt, and the offer is in the
// prompt — so these two go through the engine seam with a worker that keeps what it was handed.

mod offer {
    use super::CYCLE;
    use nexus_chat::definitions::Definitions;
    use nexus_chat::engine::{Engine, EngineConfig};
    use nexus_chat::orchestration::Caller;
    use nexus_chat::role::RoleDecl;
    use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
    use nexus_chat::timer::TimerConfig;
    use nexus_chat::worker::{TriggerOutcome, TriggerRequest, TriggerResult, Worker, WorkerConfig};
    use nexus_chat::workspace::{chat_config, setup};
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    const NOW: &str = "2026-08-31T10:00:00Z";

    /// Keeps every request verbatim, and answers the one read the engine makes of a worker — which
    /// of the sessions it started are still running.
    ///
    /// **The second half is why this is not the dry worker.** `DryWorker::session_is_running` is
    /// `false` for everything, which is exactly the mode nxf 6j6v.10yb's defect hides in: every test
    /// that drives the CLI sees a world where no session is ever alive, so the liveness gate never
    /// engages and a test cannot tell whether the path under it respects the gate at all.
    ///
    /// ONE lock per lookup — a second one taken inside `unwrap_or_else` while the first is alive
    /// deadlocks a `std::sync::Mutex`, and it would do so only on a run that is already failing (the
    /// shape `a_substituted_reply_says_so.rs` records).
    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<TriggerRequest>>,
        running: Mutex<std::collections::HashSet<String>>,
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
    }

    impl Recorder {
        /// The roles it has been asked to start, in start order.
        fn started(&self) -> Vec<String> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.role.handle.clone())
                .collect()
        }

        /// The process behind `session` has exited — what a host worker learns from its own runtime.
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
    }

    fn team(tmp: &TempDir, roles: &str) -> (Engine, Arc<Recorder>) {
        team_with(tmp, roles, CYCLE)
    }

    fn team_with(tmp: &TempDir, roles: &str, channels: &str) -> (Engine, Arc<Recorder>) {
        setup(tmp.path(), &chat_config()).expect("seed chat workspace");
        let roles: Vec<RoleDecl> = roles
            .split(';')
            .map(|r| serde_yaml::from_str(r).expect("role parses"))
            .collect();
        let defs = Definitions::new(
            roles,
            serde_yaml::from_str(channels).expect("channels parse"),
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

    /// Open the board and settle step one, so BOTH steps have been started once.
    fn to_the_review(engine: &Engine, worker: &Recorder) {
        engine
            .send_to(
                caller("pm"),
                SendToRequest {
                    machine: None,
                    to: "coding",
                    body: "build the thing",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("the channel is addressed");
        let build = worker.newest_for("coder");
        engine
            .reply_thread(
                caller("coder"),
                ReplyThreadRequest {
                    machine: None,
                    thread: build
                        .reply_thread
                        .as_deref()
                        .expect("step one owes an answer"),
                    body: "first cut is in",
                    escalate: false,
                    needs_rework: false,
                    accept: false,
                },
            )
            .expect("the reply settles step one");
        // The step is over when its SESSION is over (nxf 6j6v.10yb). This channel declares no
        // `working_tree:`, so the gate does not engage — the call is here because a host worker
        // makes it, and because the one test below that DOES declare the working copy needs the
        // same shape to be visible beside it.
        engine
            .session_ended(caller("coder"), &build.internal_session)
            .expect("the runtime reports the process gone");
        worker.exited(&build.internal_session);
    }

    const PLAIN: &str = "handle: pm\nsystem_prompt: You are pm.\ntools: [Bash]\n;\
                         handle: coder\nsystem_prompt: You are coder.\ntools: [Bash]\n;\
                         handle: review\nsystem_prompt: You are review.\ntools: [Bash]\n;\
                         handle: finisher\nsystem_prompt: You are finisher.\ntools: [Bash]\n";

    #[test]
    fn only_the_step_that_declares_a_back_edge_is_told_the_bit_exists() {
        // Owner, 2026-08-29: "dem Pruefenden wird `--needs-rework` in diesem Fall GAR NICHT ERST ALS
        // OPTION ANGEBOTEN". The existence of the bit and its effect are ONE declaration, so "set but
        // inert" cannot arise instead of merely being logged afterwards.
        let tmp = TempDir::new().unwrap();
        let (engine, worker) = team(&tmp, PLAIN);
        to_the_review(&engine, &worker);

        let build = worker.newest_for("coder").role.system_prompt;
        assert!(
            build.contains("exactly two ways to end the turn"),
            "`build` declares no back edge, so its session is offered the two endings it has: \
             {build}"
        );
        assert!(
            !build.contains("--needs-rework"),
            "…and is not told about a bit that would go nowhere: {build}"
        );

        let check = worker.newest_for("review").role.system_prompt;
        assert!(
            check.contains("exactly three ways to end the turn"),
            "`check` declares `on_needs_rework:`, so its session gets the third ending: {check}"
        );
        assert!(
            check.contains("- `--needs-rework` — what you were asked to assess"),
            "…spelled out as its own ending with its own literal flag, not hinted at — the command it \
             hangs off is written once, above (nxf 6j6v.s46h): {check}"
        );
        assert!(
            check.contains("--escalate"),
            "…and the other two are untouched: {check}"
        );
    }

    /// The five roles the nested shape needs, and deliberately NO role called `review` — so the
    /// `check` step's target resolves to the declared CHANNEL of that name, and the parties that
    /// serve it are its MEMBERS, one level below the slot.
    const QUORUM: &str = "handle: pm\nsystem_prompt: You are pm.\ntools: [Bash]\n;\
                          handle: coder\nsystem_prompt: You are coder.\ntools: [Bash]\n;\
                          handle: checkera\nsystem_prompt: You are checkera.\ntools: [Bash]\n;\
                          handle: checkerb\nsystem_prompt: You are checkerb.\ntools: [Bash]\n;\
                          handle: finisher\nsystem_prompt: You are finisher.\ntools: [Bash]\n";

    /// Appended to [`CYCLE`] — the same declared cycle, with `check`'s target now a channel of two.
    /// The shipped review round is exactly this, and `one_checker_of_a_quorum_is_enough_…` above
    /// drives the VERDICT through it; these two drive the OFFER.
    const REVIEW_CHANNEL: &str = "- name: review\n  members: [checkera, checkerb]\n";

    #[test]
    fn a_member_of_a_channel_the_step_commissioned_is_told_about_the_back_edge_too() {
        // **nxf 6j6v.0dr0, DoD 1.** The offer used to ask only whether the thread being commissioned
        // carried the flow mark ITSELF. A member of a nested channel does not — its thread hangs
        // under the channel thread, and that is where the mark sits — so the three parties the whole
        // back edge exists for were the ones never told about it, while the same role addressed
        // directly as a step's target was.
        let tmp = TempDir::new().unwrap();
        let (engine, worker) = team_with(&tmp, QUORUM, &format!("{CYCLE}{REVIEW_CHANNEL}"));
        to_the_review(&engine, &worker);

        for member in ["checkera", "checkerb"] {
            let prompt = worker.newest_for(member).role.system_prompt;
            assert!(
                prompt.contains("exactly three ways to end the turn"),
                "{member} serves the step that declares `on_needs_rework:`, so its session gets \
                 the third ending: {prompt}"
            );
            assert!(
                prompt.contains("- `--needs-rework` — what you were asked to assess"),
                "…in the same words a role target of such a step is told, spelled out as its \
                 own ending with its own literal flag: {prompt}"
            );
        }
    }

    #[test]
    fn a_member_of_a_channel_whose_step_declares_no_back_edge_is_still_not_offered_the_bit() {
        // **nxf 6j6v.0dr0, DoD 2.** Widening WHERE the question is asked must not widen WHAT it
        // answers: the bit is offered exactly where it has somewhere to go, and a step that
        // commissions a whole channel without declaring an edge is still such a nowhere. Without
        // this the test above would be satisfied by offering the bit to every member of every
        // channel a step ever opens.
        let tmp = TempDir::new().unwrap();
        let forward_only = CYCLE.replace("      on_needs_rework: build\n      max_passes: 3\n", "");
        assert!(
            !forward_only.contains("on_needs_rework"),
            "the edge is really gone from the declaration under test: {forward_only}"
        );
        let (engine, worker) = team_with(&tmp, QUORUM, &format!("{forward_only}{REVIEW_CHANNEL}"));
        to_the_review(&engine, &worker);

        for member in ["checkera", "checkerb"] {
            let prompt = worker.newest_for(member).role.system_prompt;
            assert!(
                prompt.contains("exactly two ways to end the turn"),
                "{member}'s step declares no back edge, so its session is offered the two endings \
                 it has: {prompt}"
            );
            assert!(
                !prompt.contains("--needs-rework"),
                "…and is not told about a bit that would go nowhere: {prompt}"
            );
        }
    }

    #[test]
    fn a_persona_a_channel_member_commissions_itself_is_offered_no_verdict() {
        // **The one level up is for the MEMBERS of a commissioned channel and for nobody else** —
        // the boundary `commissioning_step` already holds for the task layer and the band (nxf
        // 6j6v.6kam / 6j6v.4c3e), asserted here for the third thing that now rides on it.
        //
        // A member's own session may commission somebody, and that thread hangs under the member's
        // thread exactly as the member's hangs under the channel's. Reading a step's edge there
        // would hand a verdict — routing SOMEBODY ELSE's work back — to a party the step never
        // named and whose answer the run does not read.
        let tmp = TempDir::new().unwrap();
        let roles =
            format!("{QUORUM};handle: helper\nsystem_prompt: You are helper.\ntools: [Bash]\n");
        let (engine, worker) = team_with(&tmp, &roles, &format!("{CYCLE}{REVIEW_CHANNEL}"));
        to_the_review(&engine, &worker);
        let checker = worker.newest_for("checkera");
        assert!(
            checker
                .role
                .system_prompt
                .contains("exactly three ways to end the turn"),
            "the member itself has the offer — without that this test proves nothing: {}",
            checker.role.system_prompt
        );

        engine
            .send_to(
                Caller {
                    session: Some(&checker.internal_session),
                    actor: None,
                    now: Some(NOW),
                },
                SendToRequest {
                    machine: None,
                    to: "helper",
                    body: "look something up for me",
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .expect("a member's session may commission somebody itself");

        let helper = worker.newest_for("helper").role.system_prompt;
        assert!(
            helper.contains("exactly two ways to end the turn"),
            "the helper serves no step and is offered no verdict: {helper}"
        );
        assert!(
            !helper.contains("--needs-rework"),
            "…not even the word: {helper}"
        );
    }

    #[test]
    fn a_back_edge_opens_no_second_session_into_a_checkout_the_previous_step_is_still_in() {
        // **The mode every CLI-driven test in this file is blind to** (nxf 6j6v.10yb): the dry
        // worker answers "not running" for every session, so nothing that reads liveness ever
        // engages there and none of those tests can say whether the stepped path respects it. This
        // one supplies the answer a host runtime supplies.
        //
        // **What it does and does not pin, established by mutation rather than assumed.** TWO
        // mechanisms hold this, and EITHER ALONE is enough: `supervisor_consider_set`'s liveness
        // gate (which sits above the fork between the flat order and the state machine, so a back
        // edge meets it exactly as a `next` does) and the brought-along hurdle
        // `previous-step-still-writing` inside `preconditions_hold`. Disabling one leaves this test
        // GREEN; disabling both turns it red. So its name is the PROPERTY — no second session in
        // that checkout — and deliberately not either mechanism's name, because a test named for
        // one mechanism while two enforce it is a claim its body cannot make (the project's
        // `a-test-name-is-a-claim` rule; the over-claiming version of this name was caught here by
        // exactly that mutation).
        let tmp = TempDir::new().unwrap();
        let exclusive = CYCLE.replace(
            "- name: coding\n  members: [coder, review, finisher]\n",
            "- name: coding\n  members: [coder, review, finisher]\n  working_tree: exclusive\n",
        );
        let (engine, worker) = team_with(&tmp, PLAIN, &exclusive);
        to_the_review(&engine, &worker);

        let check = worker.newest_for("review");
        engine
            .reply_thread(
                caller("review"),
                ReplyThreadRequest {
                    machine: None,
                    thread: check
                        .reply_thread
                        .as_deref()
                        .expect("the step owes an answer"),
                    body: "the lock order is wrong",
                    escalate: false,
                    needs_rework: true,
                    accept: false,
                },
            )
            .expect("the verdict is posted");
        assert_eq!(
            worker.started(),
            vec!["coder", "review"],
            "the reviewer's own process is still in the checkout, so the back edge has NOT opened \
             a second coder into it"
        );

        // The session ends. THAT is what releases the flow — and the round then goes back.
        engine
            .session_ended(caller("review"), &check.internal_session)
            .expect("the runtime reports the process gone");
        worker.exited(&check.internal_session);
        assert_eq!(
            worker.started(),
            vec!["coder", "review", "coder"],
            "with the checkout free, the verdict routes over the declared back edge"
        );
    }

    #[test]
    fn the_seam_refuses_both_declared_bits_too_and_not_only_the_command_line() {
        // The CLI refuses the pair through clap's `conflicts_with`, which an EMBEDDING HOST never
        // goes through — it fills in a request struct. Without a guard at the seam the two booleans
        // would be a state the engine has to pick a winner for, and picking one is the engine
        // deciding which of two different things about two different pieces of work the caller
        // meant. This is the test the CLI-level one cannot be.
        let tmp = TempDir::new().unwrap();
        let (engine, worker) = team(&tmp, PLAIN);
        to_the_review(&engine, &worker);
        let check = worker.newest_for("review");
        let err = engine
            .reply_thread(
                caller("review"),
                ReplyThreadRequest {
                    machine: None,
                    thread: check
                        .reply_thread
                        .as_deref()
                        .expect("the step owes an answer"),
                    body: "both at once",
                    escalate: true,
                    needs_rework: true,
                    accept: false,
                },
            )
            .expect_err("a reply carries at most one declared bit");
        assert!(err.msg.contains("at most one declared bit"), "{err:?}");
    }

    #[test]
    fn a_role_that_opted_out_of_priming_is_still_offered_the_bit_it_is_obliged_to_use() {
        // **The 6j6v.kffm shape, one axis over.** `prime: false` opts out of the usage block and is
        // never opted out of the forced ending, which is the engine's. Announce the bit in the PRIME
        // block instead and such a checker gets a duty (answer, and your verdict routes) with no
        // means (nothing ever told it the word) — an obligation whose instrument nobody supplies.
        //
        // This is the mutation guard for exactly that: move the offer onto the prime block and this
        // test goes red while every other test in this file stays green.
        let tmp = TempDir::new().unwrap();
        let unprimed = PLAIN.replace(
            "handle: review\nsystem_prompt: You are review.\ntools: [Bash]\n",
            "handle: review\nsystem_prompt: You are review.\ntools: [Bash]\nprime: false\n",
        );
        let (engine, worker) = team(&tmp, &unprimed);
        to_the_review(&engine, &worker);

        let check = worker.newest_for("review").role.system_prompt;
        assert!(
            !check.contains("nexus-chat"),
            "the premise: this role opted out of the usage block — {check}"
        );
        assert!(
            check.contains("--needs-rework"),
            "…and is still told the one thing the ENGINE depends on: {check}"
        );
    }
}

// ---- the live acceptance: real sessions, a real model, a real cycle ---------------------------

/// **The one thing no deterministic test above can show: a real model actually uses the third
/// ending, and the engine really turns the round around on it** (nxf 6j6v.553s (a)/(d)).
///
/// Everything above drives the dry worker or a recording one: the routing is proved, and what is
/// NOT proved is that a session which was told about `--needs-rework` in its system prompt reaches
/// for it, that the party sent back is started a second time with the verdict as its task, and that
/// the ceiling really escalates a live round instead of leaving it standing. Those three are the
/// claim this item makes to its users.
///
/// **The declared round deliberately does not converge**, and that is what makes it one run rather
/// than a script: `check` has no `next`, so a plain approval would simply end it. The reviewer is
/// told the work is not good enough, so the round goes `build -> check -> build -> check -> ceiling`
/// and comes back up as an escalation. Every hop is a real session.
///
/// `#[ignore]`d for the reasons `an_obligation_comes_with_its_means.rs`'s live test states: real SDK
/// sessions, real subscription auth, real wall-clock minutes. Run it by hand from a checkout where
/// `claude` is authenticated:
///
/// ```console
/// $ cargo build -p nxs
/// $ cargo test -p nexus-chat --test a_declared_cycle_sends_the_work_back -- --ignored --nocapture
/// ```
#[test]
#[ignore = "live: spawns real Claude Agent SDK sessions through the real sidecar"]
fn live_a_real_reviewer_sends_a_real_coder_back_and_the_ceiling_escalates() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("this crate sits at <root>/crates/chat")
        .to_path_buf();
    let sidecar = repo.join("agent-sidecar/src/main.mjs");
    assert!(sidecar.is_file(), "live run: no sidecar at {sidecar:?}");
    assert!(
        std::process::Command::new("sh")
            .args(["-c", "command -v claude"])
            .output()
            .is_ok_and(|o| o.status.success()),
        "live run: `claude` is not on PATH — the SDK needs the CLI, authenticated"
    );
    assert!(
        std::env::var_os("ANTHROPIC_API_KEY").is_none(),
        "live run: ANTHROPIC_API_KEY is set; the SDK would prefer it over this machine's own \
         authenticated `claude` session"
    );
    nxs_test_support::assert_multicall_binary_fresh();
    let nxc = std::env::current_exe()
        .expect("the test binary has a path")
        .ancestors()
        .nth(2)
        .expect("the test binary lives in target/<profile>/deps")
        .join(format!("nxc{}", std::env::consts::EXE_SUFFIX));

    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    // NEITHER role is told the word `--needs-rework`. That is the premise: the reviewer learns it
    // from the FORCED ENDING the engine composes into its system prompt, because its step declares
    // an `on_needs_rework:` edge — and if the offer never reaches a real session, this run cannot
    // get past its first review.
    std::fs::write(
        roles.join("coder.yaml"),
        "handle: coder\njob_title: Live acceptance coder\nsystem_prompt: |\n  \
         You are a coder in a live acceptance run. Read, write or change NO files and run no\n  \
         command except the one reply you owe. Do not inspect anything. Answer the thread you\n  \
         were given with a one-line report of what you would have done, and stop.\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("review.yaml"),
        "handle: review\njob_title: Live acceptance reviewer\nsystem_prompt: |\n  \
         You are reviewing somebody else's work in a live acceptance run. Read, write or change\n  \
         NO files and run no command except the one reply you owe. Do not inspect anything.\n  \
         Your verdict is fixed for this run: the work you were handed does NOT meet the\n  \
         standard, because the lock order is wrong in two places. End your turn in whichever of\n  \
         the ways you were told about carries that verdict back to whoever produced the work,\n  \
         and put the reason in the body.\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: coding\n  members: [coder, review]\n  steps:\n    \
         - id: build\n      target: coder\n      next: check\n    \
         - id: check\n      target: review\n      on_needs_rework: build\n      max_passes: 2\n",
    )
    .unwrap();

    let live = |args: &[&str]| -> Value {
        let mut c = std::process::Command::new(&nxc);
        c.current_dir(tmp.path())
            .env_remove("NXC_SESSION")
            .env_remove("NXC_NOW")
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    repo.join("target/debug").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("NXC_ACTOR", "carsten")
            .env("NXC_ORIGIN", "local")
            .env("NXC_WORKER", "sidecar")
            // A real one-shot job would be left in the runner's own login session, due against a
            // `TempDir` that is gone by then. The clock is not what this proves.
            .env("NXC_TIMER", "dry")
            .env("NXC_SIDECAR", &sidecar)
            .args(args);
        let out = c.output().expect("nxc runs");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        serde_json::from_str::<Value>(stdout.trim()).unwrap_or_else(|e| {
            panic!(
                "no --json receipt from `nxc {}`: {e}\nstdout: {stdout}\nstderr: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr)
            )
        })
    };

    let board = live(&["--json", "send", "--to", "coding", "--no-ref", "Build it."])["thread_id"]
        .as_str()
        .expect("the send opened the channel thread")
        .to_string();

    // Four real sessions have to run and answer, one after the other. Generously bounded: the
    // failure this guards is a round that stalls, which without a bound is a hang rather than a
    // test.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(900);
    loop {
        // `complete` on the CHANNEL THREAD, which is what "the round came back" means: the
        // supervisor's own discharge has landed and nothing is outstanding up here any more.
        //
        // Deliberately not `escalated` — `nxc threads show` does not carry that key (it is
        // `nxc status`'s, per thread), and a poll on a key that is never present is a loop that
        // cannot end. That mistake cost this test a 15-minute spin on its first run.
        let view = live(&["--json", "threads", "show", &board]);
        if view["complete"] == Value::Bool(true) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the live round never came back up; the board was:\n{view:#}"
        );
        std::thread::sleep(std::time::Duration::from_secs(5));
    }

    // What the round actually did, read off the live board rather than remembered.
    let store = open_store(&tmp);
    let slots = store.supervised_children(&board).unwrap();
    let coder_slots: Vec<&String> = slots
        .iter()
        .filter(|t| {
            store
                .thread_quorum(t, NOW)
                .unwrap()
                .is_some_and(|q| q.expects == ["local/coder".to_string()])
        })
        .collect();
    assert_eq!(
        coder_slots.len(),
        2,
        "a real reviewer sent a real coder back: two `build` slots, not one — {slots:?}"
    );

    // The second one was commissioned with the NOTICE and the verdict, which is its whole task.
    let second = store.messages_in_thread(coder_slots[1]).unwrap();
    let task = second
        .first()
        .map(|m| m.body.clone())
        .expect("the rework commission is the first message on the second slot");
    assert!(
        task.contains("NEEDS REWORK — this is NOT an approval"),
        "the party sent back must be told what happened: {task}"
    );
    assert!(
        task.contains("This is pass 2 of 2."),
        "…and which attempt this is, so it can decide whether to escalate instead: {task}"
    );
    assert!(
        task.contains("lock order"),
        "…and the reviewer's own reasons, which ARE the work: {task}"
    );

    // …and the second verdict hit the ceiling: no THIRD attempt was started, and the round came back
    // up as an ESCALATION rather than stopping quietly.
    assert_eq!(
        slots.len(),
        4,
        "`max_passes: 2` allows two attempts and no more — build, check, build, check: {slots:?}"
    );
    assert_eq!(
        store.last_reply_escalated(&board).unwrap(),
        Some(true),
        "the ceiling hands the round UP in the one declared form there is for `the wanted result \
         could not be reached`"
    );

    // **What this deliberately does NOT assert, and it is a pre-existing shape rather than a gap
    // this item opened**: the ceiling's own sentence ("`max_passes: 2` … no further attempt was
    // started") rides the WAKE that carries a pass-through to its requester, and this round's
    // requester is a HUMAN, who has no session to wake. So the board's discharge here is the
    // `[pass_through delivered]` marker, exactly as it is for every human-requested declared channel
    // — the human reads `nxc status`, which is where `escalated` and `needs_decision` are. The
    // sentence itself is asserted where a requester actually receives it: the deterministic
    // `the_ceiling_on_the_back_edge_escalates_upward_instead_of_stopping_quietly` reads it off the
    // receipt of the reply that hit the ceiling.
}
