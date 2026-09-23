//! `reply --escalate` — the note "I cannot do this" (nxf 6j6v.wt37, component 4 of 6j6v.hq71).
//!
//! An agent uses `reply` for EXACTLY TWO things: "I am finished", and this. Everything else it
//! produces lands in the transcript and never in the agent-to-agent conversation. This suite pins
//! the second of those two, and the two effects that must not be conflated:
//!
//! - **THE TURN** (6j6v.cg8g): an escalating reply discharges the sender's turn exactly like a
//!   finished one. The session ends, the debt is gone, and the discharge predicate gives escalation
//!   no special case at all.
//! - **THE CLAIM** (6j6v.1xw1): the escalation stays READABLE on the thread it was written on, so
//!   the lease-scope rule built in that item can hold the working copy while the question travels
//!   upward. This suite proves the marker is readable; it does not touch the release rule.
//!
//! `MessageKind::Question` is a THIRD thing and stays one: the real follow-up question, from
//! somebody who can still do the work.
//!
//! **And the two above, COMBINED** (branch review of PR #337, Test Quality #1): an escalation on a
//! RE-DECLARED second turn, driven end to end. The claim and the turn meet there — turn one's "I
//! cannot" must not still be the answer to turn two, and turn two's must reach the channel thread
//! through the re-declaration — and until that test existed, every escalation test here stopped
//! after turn one. See
//! `an_escalation_belongs_to_its_own_turn_and_neither_survives_nor_precedes_a_re_declaration`, at
//! the end of this file.

mod common;

use assert_cmd::Command;
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::model::KIND_ESCALATION;
use nexus_chat::orchestration::Caller;
use nexus_chat::role::RoleDecl;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-08-14T10:00:00Z";
const ORIGIN: &str = "local";

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
        .env("NXC_ORIGIN", ORIGIN)
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
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
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    serde_json::from_str(stdout.trim()).expect("valid json")
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

fn write_roles(tmp: &TempDir, handles: &[&str], channels: Option<&str>) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in handles {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
        )
        .unwrap();
    }
    if let Some(c) = channels {
        std::fs::write(roles.join("channels.yaml"), c).unwrap();
    }
}

/// The `kind` column of every message in `thread`, in causal order — the CARRIER, read straight off
/// the view rather than through anything that could be re-deriving it.
fn kinds_in(store: &ChatStore, thread: &str) -> Vec<String> {
    store
        .messages_in_thread(thread)
        .unwrap()
        .into_iter()
        .map(|m| m.kind)
        .collect()
}

/// Open a thread with `coder` and return its id: the human summons the persona, which is what
/// declares the obligation the reply below discharges.
fn summon_coder(tmp: &TempDir) -> String {
    write_roles(tmp, &["coder"], None);
    json_of(
        human(tmp)
            .arg("--json")
            .args(["send", "--to", "coder", "please do the thing"]),
    )["thread_id"]
        .as_str()
        .expect("thread_id")
        .to_string()
}

// ---- acceptance point 1: the verb exists, and it carries exactly one meaning -------------------

#[test]
fn an_escalating_reply_marks_the_kind_column_and_is_readable_as_i_cannot() {
    let tmp = workspace();
    let thread = summon_coder(&tmp);

    let receipt = json_of(persona(&tmp, "coder", "s-coder").arg("--json").args([
        "reply",
        "--thread",
        &thread,
        "--escalate",
        "I cannot",
    ]));
    assert_eq!(receipt["posted"], true, "{receipt}");

    let store = open_store(&tmp);
    // THE CARRIER: `messages.kind`, the column that already existed and already syncs.
    assert_eq!(
        kinds_in(&store, &thread),
        vec!["info".to_string(), KIND_ESCALATION.to_string()],
        "the escalation rides the kind column of the reply itself"
    );
    // THE CLAIM (6j6v.1xw1's input): readable as an escalation.
    assert_eq!(store.last_reply_escalated(&thread).unwrap(), Some(true));
}

#[test]
fn an_escalating_reply_still_discharges_the_turn_exactly_like_a_finished_one() {
    // THE TURN (6j6v.cg8g), at the CLI rather than at the predicate: the session ends, the debt is
    // gone. This is the end-to-end half of the store-level pin that the discharge never looks at
    // `messages.kind` — proven here so "escalation gets no special case" is a fact about the shipped
    // verb and not only about one SQL expression.
    let tmp = workspace();
    let thread = summon_coder(&tmp);

    let before = open_store(&tmp)
        .thread_quorum(&thread, NOW)
        .unwrap()
        .unwrap();
    assert_eq!(before.outstanding, vec!["local/coder".to_string()]);
    assert!(!before.complete, "{before:?}");

    json_of(persona(&tmp, "coder", "s-coder").arg("--json").args([
        "reply",
        "--thread",
        &thread,
        "--escalate",
        "I cannot",
    ]));

    let after = open_store(&tmp)
        .thread_quorum(&thread, NOW)
        .unwrap()
        .unwrap();
    assert!(
        after.outstanding.is_empty(),
        "the escalating reply discharged the turn: {after:?}"
    );
    assert!(after.complete, "{after:?}");
    assert_eq!(after.replied, vec!["local/coder".to_string()]);

    // …and the teardown net therefore has nothing left to post, exactly as after a finished reply.
    let teardown = json_of(persona(&tmp, "coder", "s-coder").arg("--json").args([
        "reply",
        "--thread",
        &thread,
        "--if-unanswered",
        "late",
    ]));
    assert_eq!(
        teardown["posted"], false,
        "the debt was discharged by the escalation: {teardown}"
    );
}

// ---- acceptance point 3: `question` stays what it is -------------------------------------------

#[test]
fn the_escalation_marker_has_exactly_one_door_and_kind_is_not_a_second_one() {
    // Was TWO tests — `a_follow_up_question_is_not_an_escalation_and_the_two_doors_are_different`
    // and `escalate_and_kind_are_one_choice_so_the_two_cannot_be_asked_for_at_once` — and both were
    // about `--kind`: that `escalation` was deliberately absent from its value list, that
    // `conflicts_with` refused asking for both at once, and that `--kind question` was a THIRD thing
    // (a real follow-up question from somebody who can still do the work), kept apart from "I
    // cannot".
    //
    // `--kind` is gone (nxf 6j6v.ckeq, decision 3: `kind` falls as a CALLER option and stays as a
    // CARRIER, with exactly two of its five values in use). So the guarantee those two bought is now
    // structural — there is one door because there is only one — and what is left to hold is that
    // the door still works, that nothing else opens it, and that a plain reply is NOT an escalation.
    //
    // WHAT WENT WITH THE FLAG, and it is a named loss rather than a collapse: `--kind question` is
    // unreachable. `working_tree::hands_the_task_back` still treats an open QUESTION exactly like an
    // escalation for the working copy (the owner's ruling of 2026-08-16, nxf 6j6v.1xw1), and
    // `working_tree_claim_scope.rs` still covers that branch — but no caller can produce one. For
    // the CLAIM the difference was never a difference; for the RECORD it was, and that is what is
    // temporarily lost.
    let tmp = workspace();
    let thread = summon_coder(&tmp);

    // A plain reply is a plain reply.
    json_of(persona(&tmp, "coder", "s-coder").arg("--json").args([
        "reply",
        "--thread",
        &thread,
        "what do you mean by X?",
    ]));
    let store = open_store(&tmp);
    assert_eq!(
        kinds_in(&store, &thread),
        vec!["info".to_string(), "info".to_string()],
        "without the note, a reply carries the default kind"
    );
    assert_eq!(
        store.last_reply_escalated(&thread).unwrap(),
        Some(false),
        "and it does not read as \"I cannot\""
    );
    drop(store);

    // …and there is no second spelling of the marker. The parser refuses the flag that used to
    // carry the other four values, so `--escalate` is the ONE door by construction.
    //
    // WHAT IS ASSERTED, and what deliberately is not: the REFUSAL (a non-zero exit, and nothing
    // written) is the behaviour under test, and the message is only checked for the token a user
    // needs to see. Clap's sentence around it is clap's to change.
    for value in ["escalation", "question", "report"] {
        let out = persona(&tmp, "coder", "s-coder")
            .args([
                "reply",
                "--thread",
                &thread,
                "--kind",
                value,
                "by the back door",
            ])
            .assert()
            .failure();
        let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
        assert!(
            stderr.contains("--kind"),
            "`--kind {value}` is not a door at all any more: {stderr}"
        );
    }

    // Nothing was written by any of the refusals.
    assert_eq!(
        kinds_in(&open_store(&tmp), &thread),
        vec!["info".to_string(), "info".to_string()]
    );
}

// ---- the library seam: an escalate only the CLI can send would be a seam gap --------------------

fn engine_with_coder(tmp: &TempDir) -> Engine {
    let defs = Definitions::new(
        vec![
            serde_yaml::from_str::<RoleDecl>("handle: coder\nsystem_prompt: You are coder.\n")
                .unwrap(),
        ],
        vec![],
    )
    .unwrap();
    // Declared in the workspace's own `.nxs-personas/`, not injected: the injection path went with
    // nxf 6j6v.dvyq step 6, so this is what "the handle has a coder" now means.
    common::write_declarations(tmp.path(), &defs);
    Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine")
}

fn engine_caller(actor: &str) -> Caller<'_> {
    Caller {
        session: None,
        actor: Some(actor),
        now: Some(NOW),
    }
}

/// `Engine::send_to("coder", …)` — the obligation both seam tests below discharge.
fn engine_summons_coder(eng: &Engine) -> String {
    eng.send_to(
        engine_caller("carsten"),
        SendToRequest {
            machine: None,
            to: "coder",
            body: "please do the thing",
            refs: SendToRefs::ExplicitlyNone,
        },
    )
    .expect("send_to")
    .thread_id
}

#[test]
fn the_engine_can_say_i_cannot_through_the_very_same_field_the_cli_sets() {
    // Project rule: a thing only the CLI can express is exactly the seam gap `verb_seam.rs` targets —
    // and that gate is blind to FLAGS (`leaf_verbs` recurses on `get_subcommands()` only), so it
    // proves nothing here and this test is what the claim rests on.
    //
    // `--escalate` adds no parameter anywhere — it sets `ReplyThreadRequest::kind`, which
    // `Engine::reply_thread` has always taken — so the two surfaces cannot drift apart.
    let tmp = workspace();
    let eng = engine_with_coder(&tmp);
    let thread = engine_summons_coder(&eng);

    let receipt = eng
        .reply_thread(
            engine_caller("coder"),
            ReplyThreadRequest {
                machine: None,
                thread: &thread,
                body: "I cannot: the credentials are missing",
                escalate: true,
                needs_rework: false,
                accept: false,
            },
        )
        .expect("reply_thread");
    assert!(receipt.posted, "{receipt:?}");

    let store = open_store(&tmp);
    assert_eq!(
        store.last_reply_escalated(&thread).unwrap(),
        Some(true),
        "what the engine said reads back exactly as what the CLI says"
    );
    assert!(
        store.thread_quorum(&thread, NOW).unwrap().unwrap().complete,
        "and it discharges the turn at this seam too"
    );
}

// `the_positional_form_of_the_engines_reply_says_it_too` stood here. REMOVED with the shape it
// covered (nxf 6j6v.ckeq): the CLI's `reply` had TWO shapes reaching two seam methods —
// `Engine::reply_thread` over `surface::ReplyThreadRequest`, and `Engine::reply` over
// `orchestration::ReplyRequest` — and this test existed because `escalate_or_kind` fed both, so
// covering one would have left half the verb asserted from the CLI alone.
//
// There is one shape now. `Engine::reply` is gone, `--kind` is gone, and `escalate_or_kind` is gone
// with it: the escalation note is a BOOLEAN on `ReplyThreadRequest`, mapped once by
// `surface::reply_kind`, so there is no second surface left to drift. The test above
// (`the_engine_can_say_i_cannot_through_the_very_same_field_the_cli_sets`) covers the surviving one,
// and `writing_seam.rs` pins the boolean-to-carrier mapping itself.

// ---- the supervisor's branch: the question travels UP ------------------------------------------

const ONE_MEMBER: &str =
    "- name: coding\n  members: [coder]\n  on_complete: pass_through\n  visibility: all_members\n";

/// Drive a one-member declared channel to a settled set, with the member's reply given by `args`,
/// and return `(channel_thread, member_thread)`.
fn one_member_channel_turn(tmp: &TempDir, reply_args: &[&str]) -> (String, String) {
    write_roles(tmp, &["coder"], Some(ONE_MEMBER));
    let opened =
        json_of(
            human(tmp)
                .arg("--json")
                .args(["send", "--to", "coding", "please do the thing"]),
        );
    let channel_thread = opened["thread_id"].as_str().expect("thread_id").to_string();
    let store = open_store(tmp);
    let member_thread = store
        .supervised_children(&channel_thread)
        .unwrap()
        .into_iter()
        .next()
        .expect("one member thread under the channel thread");
    drop(store);

    let mut cmd = persona(tmp, "coder", "s-coder");
    cmd.arg("--json")
        .args(["reply", "--thread", &member_thread]);
    cmd.args(reply_args);
    json_of(&mut cmd);
    (channel_thread, member_thread)
}

#[test]
fn a_member_that_says_i_cannot_hands_an_escalation_up_the_channel_thread() {
    // hq71 §4: the supervisor decides what a settled set means. A member's "I cannot" settles its own
    // thread exactly like any other answer (that is 6j6v.cg8g's acceptance, and it is unchanged), so
    // the set IS settled and the consolidation runs. What changes is what the supervisor hands UP:
    // its own discharge of the channel thread carries the escalation, so the question keeps
    // travelling — possibly as far as the human — instead of dying at the supervisor.
    let tmp = workspace();
    let (channel_thread, member_thread) = one_member_channel_turn(
        &tmp,
        &[
            "--escalate",
            "I cannot: this repo has no fixture to run it against",
        ],
    );

    let store = open_store(&tmp);
    assert_eq!(
        store.last_reply_escalated(&member_thread).unwrap(),
        Some(true),
        "the member's own thread carries it"
    );
    assert_eq!(
        store.last_reply_escalated(&channel_thread).unwrap(),
        Some(true),
        "and so does the channel thread the supervisor discharged"
    );
    // The set still SETTLED — nothing about the turn changed.
    assert!(
        store
            .thread_quorum(&member_thread, NOW)
            .unwrap()
            .unwrap()
            .complete,
        "the escalation discharged the member's turn"
    );
    assert!(
        store
            .thread_quorum(&channel_thread, NOW)
            .unwrap()
            .unwrap()
            .complete,
        "and the consolidation ran"
    );
}

#[test]
fn a_member_that_finishes_hands_up_no_escalation() {
    // The control that makes the test above mean something: the same flow, the same consolidation,
    // and the channel thread reads as a plain result.
    let tmp = workspace();
    let (channel_thread, member_thread) = one_member_channel_turn(&tmp, &["all done"]);

    let store = open_store(&tmp);
    assert_eq!(
        store.last_reply_escalated(&member_thread).unwrap(),
        Some(false)
    );
    assert_eq!(
        store.last_reply_escalated(&channel_thread).unwrap(),
        Some(false),
        "a finished member hands up a result, not a question"
    );
}

const TWO_MEMBERS: &str = "- name: coding\n  members: [coder-a, coder-b]\n  \
                           on_complete: pass_through\n  visibility: all_members\n";

/// Two members in a `pass_through` channel; `replies` gives each member's `reply` arguments in the
/// order `supervised_children` returns them. Returns `(channel_thread, member_threads)`.
fn two_member_channel_turn(tmp: &TempDir, replies: [&[&str]; 2]) -> (String, Vec<String>) {
    write_roles(tmp, &["coder-a", "coder-b"], Some(TWO_MEMBERS));
    let opened =
        json_of(
            human(tmp)
                .arg("--json")
                .args(["send", "--to", "coding", "please do the thing"]),
        );
    let channel_thread = opened["thread_id"].as_str().expect("thread_id").to_string();
    let store = open_store(tmp);
    let member_threads = store.supervised_children(&channel_thread).unwrap();
    assert_eq!(member_threads.len(), 2, "two members, two threads");
    // Which handle owns which thread is the register's business, not this test's — read it back.
    let handles: Vec<String> = member_threads
        .iter()
        .map(|t| {
            store.thread_quorum(t, NOW).unwrap().unwrap().expects[0]
                .rsplit('/')
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    drop(store);

    for ((thread, handle), args) in member_threads.iter().zip(&handles).zip(replies) {
        let mut cmd = persona(tmp, handle, &format!("s-{handle}"));
        cmd.arg("--json").args(["reply", "--thread", thread]);
        cmd.args(args);
        json_of(&mut cmd);
    }
    (channel_thread, member_threads)
}

#[test]
fn one_escalating_member_is_enough_whichever_of_the_set_replied_last() {
    // `set_escalated` is a DISJUNCTION over the set, and a one-member channel cannot tell a
    // disjunction from an identity. A set is not a result while any part of it says it could not be
    // produced — and the error direction is the benign one: a claim held too long beats a working
    // copy released while the work is mid-flight.
    //
    // BOTH ORDERS, because only one of them is load-bearing and it is not the obvious one.
    // `[finish, escalate]` is satisfied by a "the LAST replier decides" rule just as well as by a
    // disjunction, so on its own it proves nothing about the set. `[escalate, finish]` is the case
    // that separates them: the escalation is not the last word, and the set must still refuse to
    // read as a result. Keeping the first as well is what makes the pair symmetric rather than
    // merely sufficient.
    const ESCALATE: &[&str] = &["--escalate", "I cannot: the credentials are missing"];
    const FINISH: &[&str] = &["all done"];

    for (label, replies, escalating_index) in [
        ("the escalation replied LAST", [FINISH, ESCALATE], 1usize),
        ("the escalation replied FIRST", [ESCALATE, FINISH], 0usize),
    ] {
        let tmp = workspace();
        let (channel_thread, members) = two_member_channel_turn(&tmp, replies);
        let finishing_index = 1 - escalating_index;

        let store = open_store(&tmp);
        assert_eq!(
            store
                .last_reply_escalated(&members[escalating_index])
                .unwrap(),
            Some(true),
            "{label}: the member that could not, said so"
        );
        assert_eq!(
            store
                .last_reply_escalated(&members[finishing_index])
                .unwrap(),
            Some(false),
            "{label}: the member that finished did finish"
        );
        assert_eq!(
            store.last_reply_escalated(&channel_thread).unwrap(),
            Some(true),
            "{label}: one member that cannot is enough to make the SET not a result"
        );
    }
}

#[test]
fn a_summarize_channel_hands_a_members_escalation_up_instead_of_folding_it_away() {
    // **This test used to be a CHARACTERISATION PIN and is now the claim.** Until nxf 6j6v.e9qj it
    // was called `a_summarize_channel_does_not_carry_a_members_escalation_up_to_its_channel_thread`
    // and asserted the gap wt37 disclosed: under `pass_through` the ENGINE writes the channel
    // thread's discharge (the `__delivered__` marker) and can mark it, while under `summarize` that
    // discharge is the SYNTHESIZER's own reply, whose kind the engine does not author — so a
    // member's "I cannot" died one level down. It was deliberately written to go RED the day that
    // changed. It did, with `left: Some(true)`, and this is that day.
    //
    // The declared rule that closes it is `ChannelDecl::consolidation`: **an escalating set is never
    // folded**. So a `summarize` channel whose member could not deliver does not run a model over
    // the answers at all — it passes them through, which puts the engine's own marker back in the
    // place that carries the escalation. Both output forms therefore answer the same way here, which
    // is what nxf 6j6v.1xw1 needs: `last_reply_escalated(claim_thread)` no longer depends on which
    // `on_complete` the channel happens to declare.
    let tmp = workspace();
    write_roles(
        &tmp,
        &["coder"],
        Some(
            "- name: coding\n  members: [coder]\n  on_complete: summarize\n  \
             summary_prompt: Fold the answers.\n  visibility: all_members\n",
        ),
    );
    let opened =
        json_of(
            human(&tmp)
                .arg("--json")
                .args(["send", "--to", "coding", "please do the thing"]),
        );
    let channel_thread = opened["thread_id"].as_str().expect("thread_id").to_string();
    let member_thread = open_store(&tmp)
        .supervised_children(&channel_thread)
        .unwrap()
        .into_iter()
        .next()
        .expect("one member thread");

    json_of(persona(&tmp, "coder", "s-coder").arg("--json").args([
        "reply",
        "--thread",
        &member_thread,
        "--escalate",
        "I cannot: the credentials are missing",
    ]));

    let store = open_store(&tmp);
    assert_eq!(
        store.last_reply_escalated(&member_thread).unwrap(),
        Some(true),
        "the member's own thread carries it, exactly as under pass_through"
    );
    assert_eq!(
        store.last_reply_escalated(&channel_thread).unwrap(),
        Some(true),
        "…and so does the channel thread now: the consolidator refused to fold an escalating set, \
         so the channel's answer of record is the engine's own discharge and it is marked"
    );
    assert!(
        store
            .thread_quorum(&member_thread, NOW)
            .unwrap()
            .unwrap()
            .complete,
        "the escalation discharged the member's turn here too"
    );
    drop(store);

    // And the fold really did not run — no synthesizer was ever spawned for this set. This is the
    // half that separates "the escalation was marked afterwards" from "the escalation changed which
    // output form ran", and only the second is what the declared rule says.
    let log = std::fs::read_to_string(tmp.path().join("dry.log")).unwrap_or_default();
    assert!(
        !log.contains("role=__synth__"),
        "an escalating set is never folded, so no synthesizer runs over it: {log}"
    );
}

// ---- the two headline features, COMBINED: an escalation on a re-declared turn -------------------

/// Open a one-member `pass_through` channel and return `(channel_thread, member_thread)` with
/// nothing answered yet — [`one_member_channel_turn`] without the reply, so a caller can drive more
/// than one turn through it.
fn one_member_channel(tmp: &TempDir) -> (String, String) {
    write_roles(tmp, &["coder"], Some(ONE_MEMBER));
    let opened = json_of(
        human(tmp)
            .arg("--json")
            .args(["send", "--to", "coding", "first pass"]),
    );
    let channel_thread = opened["thread_id"].as_str().expect("thread_id").to_string();
    let member_thread = open_store(tmp)
        .supervised_children(&channel_thread)
        .unwrap()
        .into_iter()
        .next()
        .expect("one member thread under the channel thread");
    (channel_thread, member_thread)
}

/// `coder` answers its member thread with `args`, returning the receipt.
fn coder_answers(tmp: &TempDir, member_thread: &str, args: &[&str]) -> Value {
    let mut cmd = persona(tmp, "coder", "s-coder");
    cmd.arg("--json").args(["reply", "--thread", member_thread]);
    cmd.args(args);
    json_of(&mut cmd)
}

const ESCALATE_TURN_ONE: &[&str] = &["--escalate", "I cannot: TURN-ONE-CANNOT"];
const ESCALATE_TURN_TWO: &[&str] = &["--escalate", "I cannot: TURN-TWO-CANNOT"];
const FINISH_TURN_ONE: &[&str] = &["TURN-ONE-DONE"];
const FINISH_TURN_TWO: &[&str] = &["TURN-TWO-DONE"];

/// **Turn-scoped discharge and `reply --escalate`, in one flow** — the intersection the branch
/// review of PR #337 named as the highest-value gap in this branch's new coverage (Test Quality #1,
/// High), and the reason it named it: everything that pins escalation drives a SINGLE turn, and the
/// one existing test that reuses a member thread twice never RE-DECLARES the expectation, so it is
/// not a second turn at all.
///
/// **What only a second turn can catch.** [`ChatStore::last_reply_escalated`] is the FIFTH reader of
/// `after_the_declaration`, and four defects of exactly its class — a predicate about WHAT WAS SAID,
/// written over a thread's LIFETIME inside machinery scoped to the TURN — shipped on this work order
/// before the rule that forbids them existed (`replied`, the assignee guard, `collected_replies`,
/// `workflow_tick`'s idempotency note). Every one of them looked right, kept every existing test
/// green, and was wrong from turn two on.
///
/// **Verified by mutation rather than asserted**: dropping `after_the_declaration` from
/// `last_reply_escalated` and running `cargo test -p nexus-chat --no-fail-fast` reddens exactly TWO
/// tests — `store::tests::the_answer_to_this_turn_is_escalating_or_not_and_the_question_is_neither`
/// (the predicate-level pin, which already existed) and this one. Not a single other integration
/// test noticed, which is precisely the end-to-end half the review named.
///
/// The assertion that separates the two is the one in the MIDDLE, after the re-declaration and
/// before the answer: turn-scoped says `None` — nobody has answered THIS turn — where a lifetime
/// read hands back turn one's verdict. Its doc spells out why that is not a nuance
/// (`None` is not `Some(false)`): nxf 6j6v.1xw1 derives the working-copy lease from this, so turn
/// one's "I cannot" would hold a working copy across a turn nobody has started answering, and turn
/// one's "I am finished" would release one while turn two is being worked on.
///
/// Driven in BOTH directions, because each catches a different half: escalate → finish proves the
/// escalation does not persist into the next turn, finish → escalate proves the new turn's
/// escalation reaches the channel thread through the re-declaration rather than only through a first
/// fan-out.
#[test]
fn an_escalation_belongs_to_its_own_turn_and_neither_survives_nor_precedes_a_re_declaration() {
    for (label, turn_one, turn_two, escalated_one, escalated_two) in [
        (
            "escalate, then finish",
            ESCALATE_TURN_ONE,
            FINISH_TURN_TWO,
            true,
            false,
        ),
        (
            "finish, then escalate",
            FINISH_TURN_ONE,
            ESCALATE_TURN_TWO,
            false,
            true,
        ),
    ] {
        let tmp = workspace();
        let (channel_thread, member_thread) = one_member_channel(&tmp);

        // ---- turn ONE ----------------------------------------------------------------------
        let first = coder_answers(&tmp, &member_thread, turn_one);
        let delivered_one = first["completed"]["delivered"]
            .as_str()
            .unwrap_or_else(|| panic!("{label}: turn one consolidated: {first}"))
            .to_string();
        {
            let store = open_store(&tmp);
            assert_eq!(
                store.last_reply_escalated(&member_thread).unwrap(),
                Some(escalated_one),
                "{label}: turn one's own thread"
            );
            assert_eq!(
                store.last_reply_escalated(&channel_thread).unwrap(),
                Some(escalated_one),
                "{label}: and the channel thread the supervisor discharged"
            );
        }
        // On the WORD, not on the punctuation that happened to follow it: nxf 6j6v.gk9j replaced the
        // one-line "ESCALATION: …" with the fuller notice (what it is, what it costs while it
        // stands, what is expected), and the claim here — turn one's handover says it in words and
        // not only in the column — is unchanged by that.
        assert_eq!(
            delivered_one.contains("ESCALATION"),
            escalated_one,
            "{label}: turn one's handover says it in words too: {delivered_one}"
        );

        // ---- the RE-DECLARATION: the requester asks again, in the channel thread -------------
        let again = json_of(human(&tmp).arg("--json").args([
            "reply",
            "--thread",
            &channel_thread,
            "second pass",
        ]));
        assert_eq!(again["posted"], true, "{label}: {again}");

        // **THE ASSERTION THIS TEST EXISTS FOR.** Turn two is declared and unanswered. A
        // turn-scoped read says so; a lifetime read hands back turn one's verdict, and a lease rule
        // built on it holds (or frees) the working copy on an answer to a question nobody has asked
        // any more.
        {
            let store = open_store(&tmp);
            assert_eq!(
                store.last_reply_escalated(&member_thread).unwrap(),
                None,
                "{label}: turn one's answer is not turn two's — nobody has answered turn two yet"
            );
            let q = store.thread_quorum(&member_thread, NOW).unwrap().unwrap();
            assert!(
                !q.complete && q.outstanding == vec!["local/coder".to_string()],
                "{label}: …because the re-declaration genuinely re-opened the debt: {q:?}"
            );
        }

        // ---- turn TWO ----------------------------------------------------------------------
        let second = coder_answers(&tmp, &member_thread, turn_two);
        let delivered_two = second["completed"]["delivered"]
            .as_str()
            .unwrap_or_else(|| panic!("{label}: turn two consolidated: {second}"))
            .to_string();
        {
            let store = open_store(&tmp);
            assert_eq!(
                store.last_reply_escalated(&member_thread).unwrap(),
                Some(escalated_two),
                "{label}: turn two's answer is turn two's"
            );
            assert_eq!(
                store.last_reply_escalated(&channel_thread).unwrap(),
                Some(escalated_two),
                "{label}: and it reached the channel thread through the RE-declaration, which is \
                 the path no single-turn test exercises"
            );
        }
        assert_eq!(
            delivered_two.contains("ESCALATION"),
            escalated_two,
            "{label}: turn two's handover says this turn's verdict, not the last one's: \
             {delivered_two}"
        );
        // …over this turn's answers alone, escalating or not (6j6v.pf6j's rule, here with an
        // escalation in the mix for the first time).
        assert!(
            delivered_two.contains(if escalated_two {
                "TURN-TWO-CANNOT"
            } else {
                "TURN-TWO-DONE"
            }),
            "{label}: {delivered_two}"
        );
        assert!(
            !delivered_two.contains("TURN-ONE-"),
            "{label}: and NOT turn one's: {delivered_two}"
        );

        // The `kind` column is the carrier nxf 6j6v.1xw1 reads, so it is checked at the source too —
        // and on the ENGINE's own two discharges rather than on every message of the thread, since
        // the requester's two requests sit between them. Both are there, each marked for ITS turn:
        // the record keeps the history, and it is only the READ that is scoped to a turn.
        let store = open_store(&tmp);
        let discharges: Vec<String> = store
            .messages_in_thread(&channel_thread)
            .unwrap()
            .into_iter()
            .filter(|m| m.sender == format!("{ORIGIN}/__delivered__"))
            .map(|m| m.kind)
            .collect();
        assert_eq!(
            discharges,
            vec![
                if escalated_one {
                    KIND_ESCALATION
                } else {
                    "info"
                }
                .to_string(),
                if escalated_two {
                    KIND_ESCALATION
                } else {
                    "info"
                }
                .to_string(),
            ],
            "{label}: two turns, two discharges, each marked for ITS turn"
        );
    }
}
