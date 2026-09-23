//! **A role that commissioned its own sub-round can say so by saying nothing** (nxf 6j6v.hw2t).
//!
//! Measured on 2026-09-08 in the owner's own workspace, on v0.88.0. `head-of-marketing` was
//! commissioned with a positioning question, decided to consult four specialists ad hoc, opened four
//! sub-threads under the same operation — and then had to end its turn with no way to say "I am
//! waiting on work I commissioned myself". Of the three things it could say, all three were false:
//!
//! ```text
//! reply              -> "done, here is your answer"   — nothing had been answered
//! silence            -> a reminder, then a SUBSTITUTE escalation posted in its name
//! reply --escalate   -> "I cannot carry this out"     — untrue, and it is the one message that
//!                                                       stops a chain
//! ```
//!
//! It chose the third, and the human's operation view read:
//!
//! ```text
//! 5 thread(s), 4 open  · NEEDS DECISION
//!   awaiting you — HANDED BACK by 47jy/head-of-marketing, not answered
//! ```
//!
//! A false alarm on the one view that exists to separate *hanging* from *running*: nothing needed
//! deciding, four sessions were working.
//!
//! What is held here is the whole of the answer, end to end over the real `nxc` binary:
//!
//! 1. the state is DERIVED — no verb, no token, nothing anybody can forget to say;
//! 2. the operation view says *waiting on its own sub-round* and does not say NEEDS DECISION;
//! 3. a plain `nxc reply` in that state is refused at the write point, with a reason to act on;
//! 4. `--escalate` still works there, because "I cannot" can be true while a sub-round runs;
//! 5. the ceiling is the sub-round's own — the waiter is woken with whatever it produced, an
//!    escalation included, and no second clock is started anywhere;
//! 6. and once the sub-round is back, the ordinary reply goes through.
//!
//! The harness is `a_reply_into_a_finished_round_is_refused.rs`'s — the real binary against the dry
//! worker, with no thread, parent edge or expectation built by hand — because the whole finding is
//! about what a caller at a terminal and a session at its teardown actually read.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};

const NOW: &str = "2026-09-08T10:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    write_roles(
        &tmp,
        &["head-of-marketing", "mkt-positionierung", "mkt-preis"],
    );
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

fn write_roles(tmp: &TempDir, handles: &[&str]) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in handles {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
        )
        .unwrap();
    }
}

/// The workspace's own clock moved on — what the scheduled job a declared `timeout:` arms runs
/// with once that window has passed. It stands in no session, exactly as that job does.
fn at(tmp: &TempDir, now: &str) -> Command {
    let mut c = human(tmp);
    c.env("NXC_NOW", now);
    c
}

/// A declared channel with a WINDOW — the only place a sub-round's own deadline can be declared
/// since `send --deadline` went (nxf 6j6v.ckeq). It is what makes the ceiling question askable at
/// all: a persona commission carries no window, so "the sub-round's own deadline" is a channel's.
const PANEL: &str = "- name: panel\n  members: [mkt-positionierung, mkt-preis]\n  timeout: 20m\n";

/// A stepped channel whose CHECK step declares a back edge — the only shape in which
/// `--needs-rework` is offered at all (`nxc guide channels`), and therefore the only one in which
/// "it is still possible while waiting" is a claim that can be tested.
const REVIEW_LOOP: &str = "- name: review-loop\n  members: [mkt-positionierung, mkt-preis]\n  \
                           timeout: 20m\n  steps:\n    \
                           - id: build\n      target: mkt-positionierung\n      next: check\n    \
                           - id: check\n      target: mkt-preis\n      on_needs_rework: build\n      \
                           max_passes: 2\n";

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim())
        .expect("valid json")
}

fn text_of(cmd: &mut Command) -> String {
    let out = cmd.assert().success();
    String::from_utf8_lossy(&out.get_output().stdout).to_string()
}

fn str_at(v: &Value, key: &str) -> String {
    v[key]
        .as_str()
        .unwrap_or_else(|| panic!("no string {key} in {v}"))
        .to_string()
}

/// The measured run, up to the moment the caller has to end its turn: the owner commissions
/// `head-of-marketing`, and that role consults two specialists of its own.
///
/// Returns `(root thread, the caller's session, the two sub-threads)` — the sub-threads SORTED,
/// which is the order every surface reports them in (`ORDER BY thread_id`) and deliberately not the
/// order they were opened in: `NXC_NOW` is pinned in this harness, so two ULIDs minted in the same
/// frozen millisecond differ only in their random tail.
fn a_caller_mid_consultation(tmp: &TempDir) -> (String, String, Vec<String>) {
    let opened = json_of(human(tmp).args([
        "--json",
        "send",
        "--to",
        "head-of-marketing",
        "where do we stand against the field?",
    ]));
    let root = str_at(&opened, "thread_id");
    let session = str_at(&opened, "session");

    let mut subs = ["mkt-positionierung", "mkt-preis"]
        .iter()
        .map(|specialist| {
            let sent = json_of(persona(tmp, "head-of-marketing", &session).args([
                "--json",
                "send",
                "--to",
                specialist,
                "what do you see?",
            ]));
            str_at(&sent, "thread_id")
        })
        .collect::<Vec<_>>();
    subs.sort();
    (root, session, subs)
}

/// The whole thread record `nxc status --thread <root> --json` carries for `thread`.
fn status_thread(tmp: &TempDir, root: &str, thread: &str) -> Value {
    let report = json_of(human(tmp).args(["--json", "status", "--thread", root]));
    report["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .flat_map(|op| op["threads"].as_array().cloned().unwrap_or_default())
        .find(|t| t["thread_id"] == thread)
        .unwrap_or_else(|| panic!("no row for {thread} in {report}"))
}

#[test]
fn the_waiting_state_is_derived_from_the_record_and_nobody_declares_it() {
    // The engine opened those two threads itself, on this caller's behalf. "Turn ended owing an
    // answer AND an open commission of its own" is therefore a fact about the record — which is why
    // this item builds no fourth reply verb: there is no token to forget.
    let tmp = workspace();
    let (root, _session, subs) = a_caller_mid_consultation(&tmp);

    let row = status_thread(&tmp, &root, &root);
    assert_eq!(
        row["waiting_on_sub_round"],
        serde_json::json!(subs),
        "the root is waiting on exactly the two rounds its assignee commissioned: {row}"
    );
    assert_eq!(
        row["escalated"],
        serde_json::json!(false),
        "nothing was handed back — that is the whole point: {row}"
    );

    // …and a sub-thread, which commissioned nothing of its own, says nothing.
    let sub = status_thread(&tmp, &root, &subs[0]);
    assert!(
        sub["waiting_on_sub_round"].is_null(),
        "a specialist that consulted nobody carries no waiting state at all: {sub}"
    );
}

#[test]
fn the_operation_view_reports_waiting_and_not_needs_decision() {
    // The measured false alarm, in the one view that exists to separate hanging from running.
    let tmp = workspace();
    let (root, _session, _subs) = a_caller_mid_consultation(&tmp);

    let report = json_of(human(&tmp).args(["--json", "status", "--thread", &root]));
    assert_eq!(
        report["operations"][0]["needs_decision"],
        serde_json::json!(false),
        "four working sessions are not a stopped chain: {report}"
    );

    let mut listing = human(&tmp);
    listing.arg("status");
    let human_view = text_of(&mut listing);
    assert!(
        !human_view.contains("NEEDS DECISION"),
        "the marker means a human must act, and here nobody must: {human_view}"
    );
    assert!(
        human_view.contains("waiting on its own sub-round (2 open)"),
        "…and waiting is SAID, not merely not-alarmed-about: {human_view}"
    );
    assert!(
        !human_view.contains("HANDED BACK"),
        "nothing was handed back: {human_view}"
    );
}

#[test]
fn a_plain_reply_while_your_own_round_runs_is_refused_at_the_write_point() {
    // A reply means DONE, and it is not true yet. Refused with a reason that can be acted on — the
    // same shape nxf 6j6v.0vd9 gave the reply into a finished round.
    let tmp = workspace();
    let (root, session, subs) = a_caller_mid_consultation(&tmp);

    let out = persona(&tmp, "head-of-marketing", &session)
        .args(["reply", "--thread", &root, "here is the positioning"])
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();

    assert!(
        err.contains("waiting on a round you commissioned yourself"),
        "the refusal names the state the caller is in: {err}"
    );
    assert!(
        err.contains(&subs[0]),
        "…names what is being waited on, so it can be read: {err}"
    );
    assert!(
        err.contains("end your turn"),
        "…and says the turn may simply end, which is the answer this item adds: {err}"
    );
    assert!(
        !err.contains("nxc withdraw"),
        "withdraw is deliberately not offered here even though it can now take back a running \
         round too (nxf 6j6v.b9nf) and stop the session that owns it: this refusal's whole job is \
         to teach the caller to WAIT, not to give the round up, and listing withdraw beside \"end \
         your turn\" would invite the reader to reach for the heavier verb in the one moment they \
         are being told patience costs nothing: {err}"
    );

    // Nothing was written: a refusal that posts is a claim of completion by another name.
    let row = status_thread(&tmp, &root, &root);
    assert_eq!(
        row["state"],
        serde_json::json!("open"),
        "the thread still owes its answer, and the caller still owes it: {row}"
    );
    assert_eq!(
        row["waiting_on_sub_round"],
        serde_json::json!(subs),
        "…and the waiting state is unchanged by the attempt: {row}"
    );
}

#[test]
fn the_refusal_reaches_an_app_through_the_same_door_it_reaches_the_terminal() {
    // `--json` is the agent's door, and an agent is the caller this rule is about. The REASON is
    // asserted, not merely the exit code: a non-zero exit is a claim about the process, and half a
    // dozen other rules can produce one on this verb — a test that reads only the code would go on
    // passing if this refusal disappeared and something else refused instead.
    //
    // BOTH STREAMS are read, because which one carries a refusal is `--json`'s business and not
    // this rule's: the flag renders the error as JSON on STDOUT, where an app parsing the call's
    // answer finds it, while the plain form writes prose to stderr. Asserting on stderr alone
    // reddened this test against correct code, which is how that was learnt. What is held here is
    // that the same words arrive; WHERE they arrive is the flag's contract and has its own tests.
    let tmp = workspace();
    let (root, session, subs) = a_caller_mid_consultation(&tmp);
    let out = persona(&tmp, "head-of-marketing", &session)
        .args(["--json", "reply", "--thread", &root, "done"])
        .assert()
        .failure();
    let err = format!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.get_output().stdout),
        String::from_utf8_lossy(&out.get_output().stderr)
    );
    assert!(
        err.contains("waiting on a round you commissioned yourself") && err.contains(&subs[0]),
        "the app door carries the same refusal, word for word, as the terminal one: {err}"
    );
}

#[test]
fn escalate_is_still_possible_while_a_sub_round_runs() {
    // "I cannot carry this out" can be TRUE while a round of yours runs — an agent that discovers it
    // lacks a credential learns nothing by waiting. Only the finished claim is impossible.
    let tmp = workspace();
    let (root, session, _subs) = a_caller_mid_consultation(&tmp);

    let receipt = json_of(persona(&tmp, "head-of-marketing", &session).args([
        "--json",
        "reply",
        "--thread",
        &root,
        "--escalate",
        "I have no access to the analyst subscription and cannot answer this at all",
    ]));
    assert_eq!(receipt["posted"], serde_json::json!(true), "{receipt}");

    let row = status_thread(&tmp, &root, &root);
    assert_eq!(
        row["escalated"],
        serde_json::json!(true),
        "a real escalation still marks the thread: {row}"
    );
}

#[test]
fn a_waiting_caller_produces_no_escalation_which_is_what_gives_the_marker_its_meaning_back() {
    // The other half of the item: an escalation in an operation must mean a chain has STOPPED. It
    // can only mean that if the ordinary case does not produce one — so nothing on this whole run,
    // from the commission to the end of the caller's turn, may carry the escalation marker.
    let tmp = workspace();
    let (root, _session, _subs) = a_caller_mid_consultation(&tmp);

    let report = json_of(human(&tmp).args(["--json", "status", "--thread", &root]));
    let escalated: Vec<&Value> = report["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .flat_map(|op| op["threads"].as_array().expect("threads"))
        .filter(|t| t["escalated"] == serde_json::json!(true))
        .collect();
    assert!(
        escalated.is_empty(),
        "waiting produced an escalation, which is exactly the alarm this item spends nothing on: \
         {report}"
    );
}

#[test]
fn the_ceiling_is_the_sub_rounds_own_outcome_and_the_waiter_is_woken_with_it() {
    // "No second clock" (the item's own words): nothing here starts a timer for the waiting caller.
    // The bound is the commissioned round — and when it ends, however it ends, the waiter is woken
    // with THAT outcome. The hardest form of "however it ends" is an escalation, so it is the one
    // pinned: a specialist that cannot answer must reach the caller, not stop at it.
    let tmp = workspace();
    let (root, session, subs) = a_caller_mid_consultation(&tmp);

    let before = status_thread(&tmp, &root, &root);
    assert!(
        before["deadline"].is_null(),
        "the waiting caller's own thread carries no window of its own: {before}"
    );

    // The specialist whose thread sorts first hands its round back.
    let first_role = specialist_of(&tmp, &root, &subs[0]);
    let receipt = json_of(
        persona(&tmp, &first_role, &session_of(&tmp, &first_role)).args([
            "--json",
            "reply",
            "--thread",
            &subs[0],
            "--escalate",
            "the panel data is behind a licence I do not have",
        ]),
    );
    assert!(
        !receipt["woke"].is_null() || !receipt["wake_skipped"].is_null(),
        "the answer travels back to the caller that commissioned it, one way or another: {receipt}"
    );

    // …and the caller is no longer waiting on that one. The second is still open, so it still is.
    let row = status_thread(&tmp, &root, &root);
    assert_eq!(
        row["waiting_on_sub_round"],
        serde_json::json!([subs[1]]),
        "a round that came back is not being waited on any more: {row}"
    );
    assert_eq!(
        row["deadline"], before["deadline"],
        "and nothing about the wait gave the caller's thread a window it did not have: {row}"
    );

    // The caller is still the party that owes the root an answer, and still may not claim to be
    // finished — one round of two is back.
    persona(&tmp, "head-of-marketing", &session)
        .args(["reply", "--thread", &root, "here is the positioning"])
        .assert()
        .failure();
}

#[test]
fn once_the_last_round_is_back_the_ordinary_reply_goes_through() {
    // The gate opens on its own. Nothing is reset, no state is cleared, nobody says "I am done
    // waiting" — the last sub-thread discharges and the predicate stops holding.
    let tmp = workspace();
    let (root, session, subs) = a_caller_mid_consultation(&tmp);
    for sub in &subs {
        let specialist = specialist_of(&tmp, &root, sub);
        let answered = json_of(
            persona(&tmp, &specialist, &session_of(&tmp, &specialist)).args([
                "--json",
                "reply",
                "--thread",
                sub,
                "here is what I see",
            ]),
        );
        // **The wake still arrives**, which is the half of this item that was never broken and had
        // to stay that way: the caller ended its turn owing an answer, and what brings it back is
        // its own sub-round returning. `woke` names the session resumed; `wake_skipped` is the
        // engine saying it tried and found the caller mid-turn, which is a delivery HELD rather
        // than an answer lost (nxf 6j6v.gn8b). Either is the hand-off happening — neither being
        // there would mean the answer reached nobody, which is the state this item's waiting
        // caller would then sit in forever.
        assert!(
            !answered["woke"].is_null() || !answered["wake_skipped"].is_null(),
            "the specialist's answer travels back to the caller that commissioned it: {answered}"
        );
    }

    let row = status_thread(&tmp, &root, &root);
    assert!(
        row["waiting_on_sub_round"].is_null(),
        "both rounds are back, so nothing is being waited on: {row}"
    );

    let receipt = json_of(persona(&tmp, "head-of-marketing", &session).args([
        "--json",
        "reply",
        "--thread",
        &root,
        "here is the positioning, built on both consultations",
    ]));
    assert_eq!(receipt["posted"], serde_json::json!(true), "{receipt}");
}

#[test]
fn a_requester_taking_another_turn_in_its_own_conversation_is_not_refused() {
    // The term that keeps this rule off everybody else: only the party the thread is WAITING FOR can
    // claim to be finished with it. The human that opened the operation owns every thread under it
    // and is waiting on all of them — refusing its own follow-up would take the whole verb away
    // from the one caller this item is not about.
    let tmp = workspace();
    let (root, _session, _subs) = a_caller_mid_consultation(&tmp);
    json_of(human(&tmp).args([
        "--json",
        "reply",
        "--thread",
        &root,
        "one more thing while you are at it",
    ]));
}

/// The newest session the dry worker was asked to start for `role` — newest, because a role may be
/// commissioned more than once in one run and the last trigger is the live one.
fn session_of(tmp: &TempDir, role: &str) -> String {
    let log = std::fs::read_to_string(tmp.path().join("dry.log")).unwrap_or_default();
    log.lines()
        .filter(|l| l.starts_with("trigger role="))
        .filter(|l| field(l, "role") == role)
        .map(|l| field(l, "session"))
        .next_back()
        .unwrap_or_else(|| panic!("no trigger for {role} in {log}"))
}

#[test]
fn a_sub_round_that_runs_out_of_time_wakes_the_waiter_with_that_outcome() {
    // "No second clock" (the item's own words), proven from the other end: the caller commissions a
    // declared channel, which is the one thing in this engine that carries a window of its own, and
    // then nothing happens. What ends the wait is THAT window — the scheduled `nxc tick` the channel
    // armed for itself — not a timer anybody started for the waiting caller. If a second clock had
    // been built, this is the test that would have had to name it.
    let tmp = workspace();
    std::fs::write(
        tmp.path().join(".nxs-personas").join("channels.yaml"),
        PANEL,
    )
    .unwrap();

    let opened = json_of(human(&tmp).args([
        "--json",
        "send",
        "--to",
        "head-of-marketing",
        "where do we stand against the field?",
    ]));
    let root = str_at(&opened, "thread_id");
    let session = str_at(&opened, "session");
    let board = str_at(
        &json_of(persona(&tmp, "head-of-marketing", &session).args([
            "--json",
            "send",
            "--to",
            "panel",
            "what do you all see?",
        ])),
        "thread_id",
    );

    // The caller is waiting, and the WINDOW is the board's rather than its own.
    let waiting = status_thread(&tmp, &root, &root);
    assert_eq!(
        waiting["waiting_on_sub_round"],
        serde_json::json!([board]),
        "the panel round is what the caller is waiting on: {waiting}"
    );
    assert!(
        waiting["deadline"].is_null(),
        "nothing gave the waiting caller's own thread an instant to be judged against: {waiting}"
    );
    let sub = status_thread(&tmp, &root, &board);
    assert!(
        !sub["deadline"].is_null(),
        "…while the round it is waiting on has the window its channel declared: {sub}"
    );

    // Nobody answers, and the channel's own scheduled job fires past that window.
    let ticked =
        json_of(at(&tmp, "2026-09-08T10:30:00Z").args(["--json", "tick", "--thread", &board]));
    assert!(
        ticked["delivered"].is_string() || !ticked["woke"].is_null(),
        "the round ended on its own window and its outcome went back to the caller: {ticked}"
    );

    // And the wait is over — because the round is, not because anything timed the waiter out.
    let after = status_thread(&tmp, &root, &root);
    assert!(
        after["waiting_on_sub_round"].is_null(),
        "the caller is waiting on nothing now: {after}"
    );
    assert_eq!(
        after["state"],
        serde_json::json!("open"),
        "…and still owes ITS caller the answer, which is the obligation it kept throughout: {after}"
    );
}

#[test]
fn the_view_never_says_nothing_is_coming_about_a_caller_whose_round_is_running() {
    // Review of PR #460, Integrity & Robustness #1 — and the defect it uncovered is the very one
    // this item exists to remove, reintroduced on the very line it exists to correct. A waiting
    // turn ALWAYS ends its session (the sidecar announces it on every teardown), so before this was
    // fixed the ORDINARY waiting caller rendered both clauses at once:
    //
    //   waiting on local/head-of-marketing — waiting on its own sub-round (2 open) — ITS SESSION
    //   HAS ENDED, nothing is coming
    //
    // "Nothing is coming" is a claim about the future and it is false there: two rounds are running
    // and the coordinator will start this session again with their answers.
    let tmp = workspace();
    let (_root, session, _subs) = a_caller_mid_consultation(&tmp);
    human(&tmp)
        .args(["session", "ended", &session])
        .assert()
        .success();

    let mut listing = human(&tmp);
    listing.arg("status");
    let view = text_of(&mut listing);
    assert!(
        view.contains("waiting on its own sub-round (2 open)"),
        "the waiting state is still said: {view}"
    );
    assert!(
        !view.contains("nothing is coming"),
        "…and the line does not contradict itself in the same breath: {view}"
    );
}

#[test]
fn a_dead_end_still_says_nothing_is_coming_where_that_is_true() {
    // The other side of the gate, so the fix above cannot have been a blanket removal: a thread
    // whose assignee ended WITHOUT commissioning anything is genuinely finished with nothing coming,
    // and that is the finding `nxc status` exists to surface. Only the waiting case is exempt.
    let tmp = workspace();
    let opened = json_of(human(&tmp).args([
        "--json",
        "send",
        "--to",
        "mkt-preis",
        "what is the going rate?",
    ]));
    human(&tmp)
        .args(["session", "ended", &str_at(&opened, "session")])
        .assert()
        .success();

    let mut listing = human(&tmp);
    listing.arg("status");
    let view = text_of(&mut listing);
    assert!(
        view.contains("ITS SESSION HAS ENDED, nothing is coming"),
        "a session that ended owing an answer and waiting on nothing IS a dead end: {view}"
    );
}

#[test]
fn needs_rework_is_still_possible_while_a_sub_round_runs() {
    // Review of PR #460, Test Quality #1: the refusal's first line is `req.escalate ||
    // req.needs_rework`, and only the escalate half was exercised — a mutation dropping the other
    // would have passed the whole suite. `--needs-rework` is a verdict on SOMEBODY ELSE's work, so
    // it is no more a claim about this caller's own result than `--escalate` is.
    let tmp = workspace();
    std::fs::write(
        tmp.path().join(".nxs-personas").join("channels.yaml"),
        REVIEW_LOOP,
    )
    .unwrap();
    let opened = json_of(human(&tmp).args([
        "--json",
        "send",
        "--to",
        "review-loop",
        "build and check it",
    ]));
    let board = str_at(&opened, "thread_id");

    // The builder answers; the checker's step opens, and the checker consults somebody of its own.
    let build = newest_slot_for(&tmp, &board, "mkt-positionierung");
    json_of(
        persona(
            &tmp,
            "mkt-positionierung",
            &session_of(&tmp, "mkt-positionierung"),
        )
        .args(["--json", "reply", "--thread", &build, "first cut is in"]),
    );
    let check = newest_slot_for(&tmp, &board, "mkt-preis");
    let checker = session_of(&tmp, "mkt-preis");
    json_of(persona(&tmp, "mkt-preis", &checker).args([
        "--json",
        "send",
        "--to",
        "head-of-marketing",
        "is this positioning defensible?",
    ]));

    // A plain reply is refused — the checker is waiting on its own consultation…
    persona(&tmp, "mkt-preis", &checker)
        .args(["reply", "--thread", &check, "looks good"])
        .assert()
        .failure();

    // …and the VERDICT is not. It says nothing about whether this caller reached its own result.
    let verdict = json_of(persona(&tmp, "mkt-preis", &checker).args([
        "--json",
        "reply",
        "--thread",
        &check,
        "--needs-rework",
        "the claim in paragraph two is not supported",
    ]));
    assert_eq!(verdict["posted"], serde_json::json!(true), "{verdict}");
}

/// The NEWEST slot under `parent` that expects `handle` — a stepped channel opens one per step, and
/// a run may open a second for the same role.
fn newest_slot_for(tmp: &TempDir, parent: &str, handle: &str) -> String {
    let store = Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store");
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

/// Which persona `thread` is waiting for, as a BARE handle — the way back from a thread id to the
/// role behind it, read off the thread's own expectation rather than guessed from the order the
/// consultations were opened in.
fn specialist_of(tmp: &TempDir, root: &str, thread: &str) -> String {
    let row = status_thread(tmp, root, thread);
    let qualified = row["expects"][0]
        .as_str()
        .unwrap_or_else(|| panic!("no expectation on {thread}: {row}"));
    qualified
        .rsplit_once('/')
        .map(|(_, bare)| bare)
        .unwrap_or(qualified)
        .to_string()
}

/// One whitespace-delimited `key=value` off a dry-worker log line.
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
