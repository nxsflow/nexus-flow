//! The channel has TWO LEVELS (nxf 6j6v.pf6j, the structural part of 6j6v.hq71).
//!
//! Until this item a `send --to <channel>` stamped ONE thread, set `expects_reply_from` to every
//! member and triggered all of them onto that same thread — requester and members in one
//! conversation. The model this suite pins is the one the owner decided on 2026-08-13/14:
//!
//! ```text
//! T4   requester       <-> supervisor #review     (the channel thread: exactly two ends)
//! T5a  supervisor      <-> reviewer-a             (one thread PER member)
//! T5b  supervisor      <-> reviewer-b
//! T5c  supervisor      <-> reviewer-c
//! ```
//!
//! The supervisor is MACHINERY OF THE ENGINE — never a person, and not a declared role either. It is
//! the reserved identity `<origin>/__channel__`, it is the incoming router (`send --to #coding` lands
//! at it and IT opens the per-member sends), and it runs synchronously in the write path of the verb
//! that reaches it.
//!
//! Everything here is driven through the real `nxc` binary against the dry worker; nothing
//! constructs a thread, a parent edge or an expectation by hand.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-08-14T10:00:00Z";
/// The reserved identity the supervisor acts under, qualified as it appears in `expects_reply_from`.
const SUPERVISOR: &str = "local/__channel__";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

/// The human at the keyboard: no spawned-context stamps, so it stands in no thread.
///
/// Deliberately NOT `NXF_DETERMINISTIC_IDS` — that counter is shared by messages, threads and
/// sessions, so in a chain this long a thread id collides with an unrelated MESSAGE id and
/// `reply --thread <id>` (which resolves a message target first) lands in the wrong thread. Every id
/// here is read back off a receipt, so nothing needs them fixed (nxf 6j6v.a71h, finding A).
fn human(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
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

fn stdout_of(cmd: &mut Command) -> String {
    let out = cmd.assert().success();
    String::from_utf8_lossy(&out.get_output().stdout).into_owned()
}

fn json_of(cmd: &mut Command) -> Value {
    serde_json::from_str(stdout_of(cmd).trim()).expect("valid json")
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// Three reviewers in one declared channel, plus a `pm` to drive the incoming direction from.
fn write_declarations(tmp: &TempDir, channels: &str) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["pm", "reviewer-a", "reviewer-b", "reviewer-c"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n"),
        )
        .unwrap();
    }
    std::fs::write(roles.join("channels.yaml"), channels).unwrap();
}

const THREE_REVIEWERS: &str = "- name: review\n  members: [reviewer-a, reviewer-b, reviewer-c]\n  \
                               visibility: all_members\n";

/// Every dry-log line that opens a trigger entry. The message field embeds real newlines, so only
/// the FIRST physical line of an entry starts with `trigger role=`.
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

/// The whole tree of the operation `thread` belongs to, as `nxc status --thread` reports it.
fn tree(tmp: &TempDir, thread: &str) -> Value {
    let report = json_of(human(tmp).args(["--json", "status", "--thread", thread]));
    report["operations"]
        .as_array()
        .and_then(|ops| ops.first().cloned())
        .unwrap_or_else(|| panic!("one operation for {thread}: {report}"))
}

fn threads_of(op: &Value) -> Vec<Value> {
    op["threads"].as_array().expect("threads").clone()
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

// ---- acceptance 1: two levels ------------------------------------------------------------------

#[test]
fn a_three_member_channel_is_four_threads_with_two_ends_each_and_the_members_do_not_see_each_other()
{
    let tmp = workspace();
    write_declarations(&tmp, THREE_REVIEWERS);

    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "review", "review the diff"]));
    let tc = opened["thread_id"].as_str().unwrap().to_string();

    // The channel thread has exactly two ends: the requester opened it, and it expects the
    // SUPERVISOR — not the three members.
    assert_eq!(
        strs(&opened["expects"]),
        vec![SUPERVISOR.to_string()],
        "the channel thread's other end is the supervisor: {opened}"
    );

    let op = tree(&tmp, &tc);
    let threads = threads_of(&op);
    assert_eq!(
        threads.len(),
        4,
        "one channel thread plus one thread per member: {op}"
    );

    // Every thread has exactly two ends: one opener, one expected handle.
    for t in &threads {
        assert_eq!(
            strs(&t["expects"]).len(),
            1,
            "every thread of the two-level model has exactly two ends: {t}"
        );
    }

    // The channel thread is the root of the four; the three member threads hang under it, each
    // expecting exactly one reviewer.
    let mut members: Vec<(String, String)> = Vec::new();
    for t in &threads {
        let id = t["thread_id"].as_str().unwrap().to_string();
        if id == tc {
            assert_eq!(strs(&t["expects"]), vec![SUPERVISOR.to_string()]);
            continue;
        }
        assert_eq!(
            t["parent"].as_str(),
            Some(tc.as_str()),
            "a member thread hangs under the channel thread: {t}"
        );
        assert_eq!(
            t["opener"].as_str(),
            Some(SUPERVISOR),
            "a member thread is opened BY the supervisor: {t}"
        );
        members.push((id, strs(&t["expects"])[0].clone()));
    }
    let mut expected: Vec<String> = members.iter().map(|(_, h)| h.clone()).collect();
    expected.sort();
    assert_eq!(
        expected,
        vec!["local/reviewer-a", "local/reviewer-b", "local/reviewer-c"],
        "one thread per member, and each names exactly its own member: {op}"
    );

    // Each member is triggered on its OWN thread — the wake text names it, not the channel thread.
    let lines = trigger_lines(&tmp);
    assert_eq!(lines.len(), 3, "one session per member: {lines:?}");
    let log = std::fs::read_to_string(tmp.path().join("dry.log")).unwrap();
    for (member_thread, _) in &members {
        assert!(
            log.contains(&format!("nxc reply --thread {member_thread}")),
            "every member is told ITS OWN thread: {log}"
        );
    }
    assert!(
        !log.contains(&format!("--thread {tc}")),
        "no member is ever pointed at the channel thread: {log}"
    );

    // The members do not see each other. `visibility: all_members` is deliberate here — even under
    // the most permissive policy there is nothing of reviewer-b in reviewer-a's thread, because it
    // is a different conversation rather than a filtered view of one.
    for (member_thread, member) in &members {
        let board = json_of(human(&tmp).args(["--json", "threads", "show", member_thread]));
        let senders: Vec<String> = board["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .map(|m| m["sender"].as_str().unwrap().to_string())
            .collect();
        for s in &senders {
            assert!(
                s == SUPERVISOR || s == member,
                "thread {member_thread} may only carry the supervisor and {member}: {senders:?}"
            );
        }
    }
}

/// The supervisor's identity cannot be CLAIMED by a declaration (nxf 6j6v.pf6j, fix round 1).
///
/// Everything about the two levels is decided from `opener == <origin>/__channel__`: which threads
/// form the set, whose claim area a member competes under, who may re-declare a member's turn, and
/// who counts as the requester for `requester_only`. A declared role able to take that handle could
/// put itself where only the engine belongs. The reservation was documented for a whole round
/// before it was enforced; this drives the real loader against a real declaration directory so the
/// claim is checked rather than asserted.
#[test]
fn a_declaration_cannot_claim_the_supervisors_identity() {
    let tmp = workspace();
    write_declarations(&tmp, THREE_REVIEWERS);
    std::fs::write(
        tmp.path().join(".nxs-personas").join("__channel__.yaml"),
        "handle: __channel__\nsystem_prompt: I would like to be the supervisor.\n",
    )
    .unwrap();

    // Every verb that resolves declarations refuses to load this workspace at all — loudly, at the
    // catalogue, rather than by letting a forged opener through to the routing.
    let out = human(&tmp)
        .args(["--json", "send", "--to", "review", "review the diff"])
        .assert()
        .failure();
    let err: Value =
        serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim()).unwrap();
    assert_eq!(err["error"]["kind"], "validation", "{err}");
    let msg = err["error"]["msg"].as_str().unwrap_or_default();
    assert!(
        msg.contains("__channel__") && msg.contains("reserved"),
        "the refusal names the handle and why: {err}"
    );
}

// ---- acceptance 2: the supervisor is also the INCOMING router ----------------------------------

#[test]
fn a_send_to_a_channel_lands_at_the_supervisor_and_the_supervisor_opens_the_member_sends() {
    // The incoming direction, not the fan-out: `pm -> send --to review` never addresses a member.
    // What the pm's own call produces is a conversation with the supervisor; the per-member sends
    // are the SUPERVISOR's, and the store says so — it is the opener of every one of them, and the
    // pm is an end of none.
    let tmp = workspace();
    write_declarations(&tmp, THREE_REVIEWERS);

    let t1 = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let pm_session = t1["session"].as_str().unwrap().to_string();

    let opened = json_of(persona(&tmp, "pm", &pm_session).args([
        "--json",
        "send",
        "--to",
        "review",
        "review the diff",
    ]));
    let tc = opened["thread_id"].as_str().unwrap().to_string();

    let store = open_store(&tmp);
    let openers: Vec<(String, Option<String>)> = store
        .connection()
        .prepare("SELECT thread_id, opener FROM threads ORDER BY thread_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let member_threads: Vec<&(String, Option<String>)> = openers
        .iter()
        .filter(|(_, o)| o.as_deref() == Some(SUPERVISOR))
        .collect();
    assert_eq!(
        member_threads.len(),
        3,
        "the supervisor — not the pm — opened one thread per member: {openers:?}"
    );
    assert_eq!(
        openers
            .iter()
            .find(|(id, _)| id == &tc)
            .and_then(|(_, o)| o.clone()),
        Some("local/pm".to_string()),
        "…and the channel thread itself is the pm's own conversation with the supervisor"
    );

    // The pm is an end of the CHANNEL thread and of no member thread: it asked the supervisor, and
    // the supervisor asked the members.
    let op = tree(&tmp, &tc);
    let mut member_count = 0;
    for t in threads_of(&op) {
        if t["parent"].as_str() != Some(tc.as_str()) {
            continue;
        }
        member_count += 1;
        assert!(
            !strs(&t["expects"]).contains(&"local/pm".to_string()),
            "no member thread has the requester as an end: {t}"
        );
        assert_eq!(t["opener"].as_str(), Some(SUPERVISOR), "{t}");
    }
    assert_eq!(member_count, 3, "three member threads under it: {op}");
    let channel_thread = threads_of(&op)
        .into_iter()
        .find(|t| t["thread_id"].as_str() == Some(tc.as_str()))
        .expect("the channel thread is in its own operation");
    assert_eq!(
        strs(&channel_thread["expects"]),
        vec![SUPERVISOR.to_string()],
        "what the pm's own call produced is a conversation with the supervisor: {channel_thread}"
    );
}

// ---- acceptance 3: SYNCHRONOUS, in the write path ----------------------------------------------

/// The consequence has already happened by the time the call returns — in BOTH directions.
///
/// hq71 §1, the owner's decision of 2026-08-14: message written → obligation recognised as
/// discharged → supervisor runs → next thread opened and the sessions STARTED → return. What is
/// waited on is the starting, not the result; the latency is a spawn. The reason it is not an
/// asynchronous event some worker picks up is the one that decides the whole design: only here does
/// a failed consequence have an addressee, because the caller is still running.
///
/// The proof is deliberately taken from the RECEIPT of the causing call plus the trigger log AS IT
/// STANDS at that moment — nothing is polled, nothing is waited for, and no second command is run in
/// between.
#[test]
fn the_consequence_has_happened_by_the_time_the_call_returns_in_both_directions() {
    let tmp = workspace();
    write_declarations(&tmp, THREE_REVIEWERS);

    let t1 = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let pm_session = t1["session"].as_str().unwrap().to_string();
    assert_eq!(trigger_lines(&tmp).len(), 1, "the pm itself");

    // INCOMING: the pm's own `send --to <channel>` returns only once the supervisor has opened every
    // member thread AND started every member session.
    let opened = json_of(persona(&tmp, "pm", &pm_session).args([
        "--json",
        "send",
        "--to",
        "review",
        "review the diff",
    ]));
    let tc = opened["thread_id"].as_str().unwrap().to_string();
    let started = trigger_lines(&tmp);
    assert_eq!(
        started.len(),
        4,
        "the three member sessions are already started when the send returns: {started:?}"
    );
    let mut roles: Vec<String> = started[1..].iter().map(|l| field(l, "role")).collect();
    roles.sort();
    assert_eq!(roles, vec!["reviewer-a", "reviewer-b", "reviewer-c"]);
    assert_eq!(
        opened["spawned"].as_bool(),
        Some(true),
        "…and the receipt says so: {opened}"
    );

    // OUTGOING: the member replies. The first two settle their own threads and nothing else happens
    // — the set is not complete, so there is no consequence to have.
    let store = open_store(&tmp);
    let mut member_threads: Vec<(String, String)> = store
        .supervised_children(&tc)
        .unwrap()
        .into_iter()
        .map(|t| {
            let q = store.thread_quorum(&t, NOW).unwrap().unwrap();
            (q.expects[0].clone(), t)
        })
        .collect();
    member_threads.sort();
    drop(store);

    for (handle, thread) in &member_threads[..2] {
        let bare = handle.rsplit('/').next().unwrap();
        let session = field(
            started
                .iter()
                .find(|l| field(l, "role") == bare)
                .expect("triggered"),
            "session",
        );
        let receipt = json_of(persona(&tmp, bare, &session).args([
            "--json",
            "reply",
            "--thread",
            thread,
            "looks fine",
        ]));
        assert_eq!(receipt["posted"], true, "{receipt}");
        assert!(
            receipt["completed"].is_null() && receipt["woke"].is_null(),
            "the set is not settled, so this reply has no consequence yet: {receipt}"
        );
        assert_eq!(
            trigger_lines(&tmp).len(),
            4,
            "and nothing was started for it"
        );
    }

    // The LAST one. By the time THIS call returns, the supervisor has consolidated and the requester
    // has been started — both are in the receipt of the very call that caused them.
    let (last_handle, last_thread) = &member_threads[2];
    let bare = last_handle.rsplit('/').next().unwrap();
    let session = field(
        started
            .iter()
            .find(|l| field(l, "role") == bare)
            .expect("triggered"),
        "session",
    );
    let receipt = json_of(persona(&tmp, bare, &session).args([
        "--json",
        "reply",
        "--thread",
        last_thread,
        "approved",
    ]));
    let delivered = receipt["completed"]["delivered"]
        .as_str()
        .unwrap_or_else(|| panic!("the consolidation ran inside this call: {receipt}"));
    assert!(
        delivered.contains("looks fine") && delivered.contains("approved"),
        "…and it folded the answers from all three member threads: {receipt}"
    );
    assert_eq!(
        receipt["woke"].as_str(),
        Some(pm_session.as_str()),
        "the requester was started before this call returned: {receipt}"
    );
    assert_eq!(
        trigger_lines(&tmp).len(),
        5,
        "exactly one new session — the requester's, waited on for its STARTING and nothing more"
    );
}

// ---- acceptance 5: the SUPERVISOR waits for a SET OF THREADS ------------------------------------

/// The quorum mechanic is rebuilt, not extended.
///
/// It is no longer a THREAD with N expected handles waiting for its replies — every thread has
/// exactly one expectation now — but the SUPERVISOR waiting for a SET of threads, derived from the
/// tree rather than recorded anywhere. It resumes when all of them have answered, and the deadline
/// that could strike instead hangs on the MEMBER thread, behind which stands exactly one session
/// (which is what makes 6j6v.nf38's per-member reset possible at all).
#[test]
fn the_supervisor_waits_for_a_set_of_threads_and_the_deadline_hangs_on_each_member() {
    let tmp = workspace();
    write_declarations(
        &tmp,
        "- name: review\n  members: [reviewer-a, reviewer-b, reviewer-c]\n  timeout: 20m\n",
    );

    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "review", "review the diff"]));
    let tc = opened["thread_id"].as_str().unwrap().to_string();
    let deadline = opened["deadline"]
        .as_str()
        .expect("the channel's declared timeout resolved to an instant")
        .to_string();

    let store = open_store(&tmp);
    let members = store.supervised_children(&tc).unwrap();
    assert_eq!(members.len(), 3);
    for m in &members {
        let q = store.thread_quorum(m, NOW).unwrap().unwrap();
        assert_eq!(
            q.expects.len(),
            1,
            "no thread waits for more than one handle any more: {q:?}"
        );
        assert_eq!(
            q.deadline.as_deref(),
            Some(deadline.as_str()),
            "the deadline hangs on the member thread, where exactly one session stands: {q:?}"
        );
    }
    // Nothing records the membership of the set: it IS the tree.
    assert_eq!(
        store.supervised_children(&tc).unwrap(),
        members,
        "the set is derived from the parent edge, not from a second bookkeeping table"
    );
    drop(store);

    // Two of three answered: the supervisor does not resume, and the channel thread still owes.
    let sessions: Vec<(String, String)> = trigger_lines(&tmp)
        .iter()
        .map(|l| (field(l, "role"), field(l, "session")))
        .collect();
    let thread_of = |handle: &str| -> String {
        let store = open_store(&tmp);
        let want = format!("local/{handle}");
        store
            .supervised_children(&tc)
            .unwrap()
            .into_iter()
            .find(|t| store.thread_quorum(t, NOW).unwrap().unwrap().expects == [want.clone()])
            .expect("a thread per member")
    };
    for handle in ["reviewer-a", "reviewer-b"] {
        let session = sessions
            .iter()
            .find(|(r, _)| r == handle)
            .map(|(_, s)| s.clone())
            .unwrap();
        json_of(persona(&tmp, handle, &session).args([
            "--json",
            "reply",
            "--thread",
            &thread_of(handle),
            "fine",
        ]));
    }
    let store = open_store(&tmp);
    assert!(
        !store.thread_quorum(&tc, NOW).unwrap().unwrap().complete,
        "two of three is not the answer"
    );
    drop(store);

    // The third settles the SET, and only then does the supervisor act.
    let session = sessions
        .iter()
        .find(|(r, _)| r == "reviewer-c")
        .map(|(_, s)| s.clone())
        .unwrap();
    let receipt = json_of(persona(&tmp, "reviewer-c", &session).args([
        "--json",
        "reply",
        "--thread",
        &thread_of("reviewer-c"),
        "fine",
    ]));
    assert!(
        !receipt["completed"].is_null(),
        "all three answered — the supervisor resumed: {receipt}"
    );
    assert!(
        open_store(&tmp)
            .thread_quorum(&tc, NOW)
            .unwrap()
            .unwrap()
            .complete,
        "and the channel thread is settled"
    );
}

// ---- acceptance 6: `send --to <channel>` is the only way to start a flow ------------------------

#[test]
fn a_send_to_a_channel_starts_the_whole_flow_and_it_is_the_only_way_in() {
    // hq71's own DoD: "`nxc send --to <kanal>` startet ihn, und es ist der einzige Weg dorthin."
    // This test is the "expressible" half, proven by construction: a channel fans out, waits,
    // consolidates and delivers, with no run record anywhere in the picture.
    //
    // It used to ALSO assert `SELECT COUNT(*) FROM workflow_runs` came back 0 — the strongest form
    // of "nothing about the flow needs the run record" available while the record still existed.
    // 6j6v.dvyq §3 removed the record, so that assertion moved to the one place where it is still
    // a claim about the database rather than about a fixture:
    // `channel_flow.rs::a_declared_flow_runs_the_fixed_order_that_used_to_need_a_run` asserts the
    // TABLE is gone.
    let tmp = workspace();
    write_declarations(&tmp, THREE_REVIEWERS);

    let t1 = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let pm_session = t1["session"].as_str().unwrap().to_string();
    let opened = json_of(persona(&tmp, "pm", &pm_session).args([
        "--json",
        "send",
        "--to",
        "review",
        "review the diff",
    ]));
    let tc = opened["thread_id"].as_str().unwrap().to_string();

    let store = open_store(&tmp);
    let threads: Vec<(String, String)> = store
        .supervised_children(&tc)
        .unwrap()
        .into_iter()
        .map(|t| {
            let q = store.thread_quorum(&t, NOW).unwrap().unwrap();
            (q.expects[0].rsplit('/').next().unwrap().to_string(), t)
        })
        .collect();
    drop(store);
    let sessions: Vec<(String, String)> = trigger_lines(&tmp)
        .iter()
        .map(|l| (field(l, "role"), field(l, "session")))
        .collect();
    for (handle, thread) in &threads {
        let session = sessions
            .iter()
            .find(|(r, _)| r == handle)
            .map(|(_, s)| s.clone())
            .unwrap();
        json_of(
            persona(&tmp, handle, &session).args(["--json", "reply", "--thread", thread, "fine"]),
        );
    }

    let store = open_store(&tmp);
    assert!(
        store.thread_quorum(&tc, NOW).unwrap().unwrap().complete,
        "the flow ran to its end"
    );
}

// ---- the incoming router, turn two: the supervisor re-declares -----------------------------------

/// The second turn, end to end — and the re-declaration nothing else in the engine performs.
///
/// A `reply` into the CHANNEL thread from its requester is the same door as the first `send --to`:
/// the supervisor takes it in and hands every member its next turn on its OWN existing thread. Two
/// writes make that turn real, and both are checked here through the surface a role actually uses:
/// `facade::set_expects` re-opens the debt (nxf 6j6v.cg8g: a re-declaration is a QUESTION, so the
/// member owes again), and the resume carries the obligation, so the sidecar's teardown net —
/// `nxc reply --thread <id> --if-unanswered` — has something to discharge. Before this item nothing
/// re-declared on a resume at all, so from turn two on that net was a silent no-op.
///
/// **The honest bound on this proof**: it drives sequential `nxc` subprocesses, each of which opens
/// the store fresh. It therefore does NOT cover the shape of nxf 6j6v.fc5p — a long-lived `Engine`
/// handle beside a short-lived subprocess, which do not observe each other's Lamport clock.
#[test]
fn a_reply_into_the_channel_thread_re_opens_every_members_turn_on_its_own_thread() {
    let tmp = workspace();
    write_declarations(&tmp, "- name: pair\n  members: [reviewer-a, reviewer-b]\n");

    let t1 = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let pm_session = t1["session"].as_str().unwrap().to_string();
    let opened = json_of(persona(&tmp, "pm", &pm_session).args([
        "--json",
        "send",
        "--to",
        "pair",
        "first look",
    ]));
    let tc = opened["thread_id"].as_str().unwrap().to_string();

    let sessions: Vec<(String, String)> = trigger_lines(&tmp)
        .iter()
        .map(|l| (field(l, "role"), field(l, "session")))
        .collect();
    let thread_of = |handle: &str| -> String {
        let store = open_store(&tmp);
        let want = format!("local/{handle}");
        store
            .supervised_children(&tc)
            .unwrap()
            .into_iter()
            .find(|t| store.thread_quorum(t, NOW).unwrap().unwrap().expects == [want.clone()])
            .expect("a thread per member")
    };
    let session_of = |handle: &str| -> String {
        sessions
            .iter()
            .find(|(r, _)| r == handle)
            .map(|(_, s)| s.clone())
            .unwrap()
    };

    // Turn one, answered by both.
    for handle in ["reviewer-a", "reviewer-b"] {
        json_of(persona(&tmp, handle, &session_of(handle)).args([
            "--json",
            "reply",
            "--thread",
            &thread_of(handle),
            "turn one",
        ]));
    }
    // The teardown net correctly finds nothing owed now — this is the state it has to tell apart.
    let settled = json_of(
        persona(&tmp, "reviewer-a", &session_of("reviewer-a")).args([
            "--json",
            "reply",
            "--thread",
            &thread_of("reviewer-a"),
            "--if-unanswered",
            "anything left?",
        ]),
    );
    assert_eq!(
        settled["posted"], false,
        "turn one is discharged, so the net stays silent: {settled}"
    );
    let before = trigger_lines(&tmp).len();
    // The whole log, not the `trigger role=` lines: `msg=` embeds real newlines, so the wake TEXT
    // this test is about is in the bytes between those lines and not on them.
    let log_before = std::fs::read_to_string(tmp.path().join("dry.log")).unwrap_or_default();

    // The requester asks again, in the channel thread — the supervisor's own end of it.
    let again = json_of(persona(&tmp, "pm", &pm_session).args([
        "--json",
        "reply",
        "--thread",
        &tc,
        "one more pass, please",
    ]));
    assert_eq!(again["posted"], true, "{again}");

    // Every member's turn is re-opened ON ITS OWN THREAD, and its session was resumed for it.
    let store = open_store(&tmp);
    for handle in ["reviewer-a", "reviewer-b"] {
        let q = store
            .thread_quorum(&thread_of(handle), NOW)
            .unwrap()
            .unwrap();
        assert_eq!(
            q.outstanding,
            vec![format!("local/{handle}")],
            "the re-declaration re-opened {handle}'s debt: {q:?}"
        );
        assert!(!q.complete, "{q:?}");
    }
    assert_eq!(
        store.thread_quorum(&tc, NOW).unwrap().unwrap().outstanding,
        vec![SUPERVISOR.to_string()],
        "…and the supervisor owes the requester again"
    );
    drop(store);
    let resumed = trigger_lines(&tmp);
    assert_eq!(
        resumed.len(),
        before + 2,
        "both members were resumed, inside the call that asked: {resumed:?}"
    );
    for handle in ["reviewer-a", "reviewer-b"] {
        assert_eq!(
            field(
                resumed[resumed.len() - 2..]
                    .iter()
                    .find(|l| field(l, "role") == handle)
                    .unwrap(),
                "session"
            ),
            session_of(handle),
            "the SAME session, resumed rather than replaced"
        );
    }

    // …and the wake that carried the second turn NAMES the obligation it just re-declared (nxf
    // 6j6v.dq59). This was the one of the four `reply_guidance` call sites with no content
    // assertion on it (review of PR #452, Test Quality · Medium): the re-declaration and the
    // sentence that tells the member how to answer it are written a few lines apart, and until
    // this a wake composed with an empty block here would have stayed green.
    let log_after = std::fs::read_to_string(tmp.path().join("dry.log")).unwrap();
    let second_turn = &log_after[log_before.len()..];
    for handle in ["reviewer-a", "reviewer-b"] {
        assert!(
            second_turn.contains(&format!("nxc reply --thread {}", thread_of(handle))),
            "{handle}'s second-turn wake names its own thread and the verb: {second_turn}"
        );
    }
    assert!(
        second_turn.contains("needs no thread id"),
        "one conversation is open for each member, so each is offered the id-free form: \
         {second_turn}"
    );

    // THE POINT: the teardown net now posts, where before this item it was a silent no-op from turn
    // two on — the end-to-end half 6j6v.cg8g could only prove at the predicate level.
    let net = json_of(
        persona(&tmp, "reviewer-a", &session_of("reviewer-a")).args([
            "--json",
            "reply",
            "--thread",
            &thread_of("reviewer-a"),
            "--if-unanswered",
            "turn two",
        ]),
    );
    assert_eq!(
        net["posted"], true,
        "the second turn is genuinely owed, so the sidecar's own teardown call discharges it: {net}"
    );
}

/// Turn TWO's consolidation carries turn two's answers, and nothing else (nxf 6j6v.pf6j, fix round 1).
///
/// The defect this pins is the one that had already appeared twice before it in this work order, in
/// two other places: a predicate written over a thread's LIFETIME inside machinery scoped to the
/// TURN. `collected_replies` read every message of every member thread, so the second round folded
/// the first round's answers back in — together with the literal `[pass_through delivered]` marker
/// the engine posts to record that the first round was handed over — and delivered that to the
/// requester as if it were the new answer. Every existing test stayed green, because every one of
/// them stopped after turn one.
///
/// There was no end-to-end turn-two consolidation test at all. This is it.
#[test]
fn the_second_turns_consolidation_carries_only_the_second_turns_answers() {
    let tmp = workspace();
    write_declarations(&tmp, "- name: pair\n  members: [reviewer-a, reviewer-b]\n");

    let t1 = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let pm_session = t1["session"].as_str().unwrap().to_string();
    let opened = json_of(persona(&tmp, "pm", &pm_session).args([
        "--json",
        "send",
        "--to",
        "pair",
        "first look",
    ]));
    let tc = opened["thread_id"].as_str().unwrap().to_string();

    let sessions: Vec<(String, String)> = trigger_lines(&tmp)
        .iter()
        .map(|l| (field(l, "role"), field(l, "session")))
        .collect();
    let session_of = |handle: &str| -> String {
        sessions
            .iter()
            .find(|(r, _)| r == handle)
            .map(|(_, s)| s.clone())
            .unwrap()
    };
    let thread_of = |handle: &str| -> String {
        let store = open_store(&tmp);
        let want = format!("local/{handle}");
        store
            .supervised_children(&tc)
            .unwrap()
            .into_iter()
            .find(|t| store.thread_quorum(t, NOW).unwrap().unwrap().expects == [want.clone()])
            .expect("a thread per member")
    };

    // Turn one, both members answering with a text this test can look for later.
    json_of(
        persona(&tmp, "reviewer-a", &session_of("reviewer-a")).args([
            "--json",
            "reply",
            "--thread",
            &thread_of("reviewer-a"),
            "TURN-ONE-A",
        ]),
    );
    let first = json_of(
        persona(&tmp, "reviewer-b", &session_of("reviewer-b")).args([
            "--json",
            "reply",
            "--thread",
            &thread_of("reviewer-b"),
            "TURN-ONE-B",
        ]),
    );
    let delivered_one = first["completed"]["delivered"]
        .as_str()
        .expect("turn one consolidated")
        .to_string();
    assert!(
        delivered_one.contains("TURN-ONE-A") && delivered_one.contains("TURN-ONE-B"),
        "turn one is unchanged: {delivered_one}"
    );
    assert!(
        delivered_one.contains("first look"),
        "…including the request it answers: {delivered_one}"
    );

    // The requester asks again; the supervisor re-opens both turns.
    json_of(persona(&tmp, "pm", &pm_session).args([
        "--json",
        "reply",
        "--thread",
        &tc,
        "second look",
    ]));
    json_of(
        persona(&tmp, "reviewer-a", &session_of("reviewer-a")).args([
            "--json",
            "reply",
            "--thread",
            &thread_of("reviewer-a"),
            "TURN-TWO-A",
        ]),
    );
    let second = json_of(
        persona(&tmp, "reviewer-b", &session_of("reviewer-b")).args([
            "--json",
            "reply",
            "--thread",
            &thread_of("reviewer-b"),
            "TURN-TWO-B",
        ]),
    );

    let delivered_two = second["completed"]["delivered"]
        .as_str()
        .unwrap_or_else(|| panic!("turn two consolidated: {second}"));
    assert!(
        delivered_two.contains("TURN-TWO-A") && delivered_two.contains("TURN-TWO-B"),
        "turn two carries turn two's answers: {delivered_two}"
    );
    assert!(
        !delivered_two.contains("TURN-ONE-A") && !delivered_two.contains("TURN-ONE-B"),
        "…and NOT turn one's, which is what a lifetime-scoped read folded back in: {delivered_two}"
    );
    assert!(
        !delivered_two.contains("[pass_through delivered]"),
        "…nor the engine's own marker recording that turn one was handed over: {delivered_two}"
    );
    assert!(
        delivered_two.contains("second look") && !delivered_two.contains("first look"),
        "and the request it answers is THIS turn's: {delivered_two}"
    );
    assert_eq!(
        second["woke"].as_str(),
        Some(pm_session.as_str()),
        "the requester is woken for the second turn exactly as for the first: {second}"
    );
}
