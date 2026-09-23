//! The operation is a thread TREE (nxf 6j6v.a71h) — the parent edge and `nxc status`, from the
//! command line.
//!
//! The item's acceptance is written around ONE chain (its §2), and the binding word about it is
//! "not a paper mapping: the chain is DRIVEN". So this suite drives it: real `nxc` invocations, the
//! dry worker, one session per role, each acting under the stamps the sidecar gives a spawned role.
//! Nothing here constructs a `ThreadRoot` by hand or writes a parent anywhere — every edge in the
//! tree is one the surface stamped because of where the caller was standing.
//!
//! **The supervisor is real since 6j6v.pf6j.** This suite used to TYPE it — a declared channel with
//! exactly one member stood in for the `Betreuer #coding`/`Betreuer #review` of §2, because the
//! two-level channel did not exist yet. It does now: a `send --to <channel>` opens a CHANNEL thread
//! with exactly two ends (requester ↔ the reserved `<origin>/__channel__`), and the supervisor opens
//! one thread per member below it. So the chain of §2 gained one level per channel it crosses — the
//! member thread the supervisor opened — and the roles that used to be "the supervisor" are what
//! they always were in the declaration: its MEMBERS. Every edge is still stamped by the code under
//! test, and every one of them still follows §3.1: a thread hangs under the thread it was sent from.

use assert_cmd::Command;
use nexus_chat::workspace::ChatWorkspaceExt;
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-08-14T10:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config())
        .expect("seed chat workspace");
    tmp
}

/// The human at the keyboard: no spawned-context stamps at all, so it stands in no thread.
fn human(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    // Deliberately NOT `NXF_DETERMINISTIC_IDS`: that counter is shared by messages, threads and
    // sessions, so in a chain this long a thread id collides with an unrelated MESSAGE id — and
    // `reply --thread <id>` resolves a message target first, so the reply would silently land in a
    // different thread. Real ULIDs are what production mints and the only ids this chain can be
    // driven with; every id here is read back off a receipt, so nothing needs them to be fixed.
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

fn write_declarations(tmp: &TempDir) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in [
        "pm",
        "coding-lead",
        "coder",
        "review-lead",
        "reviewer-a",
        "reviewer-b",
        "reviewer-c",
    ] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
        )
        .unwrap();
    }
    // The two declared channels of §2 (one member each: the SUPERVISOR — see the module doc), plus
    // one two-member board, which is the only shape today that leaves a thread OPEN while one of its
    // assignees has already spoken (nxf 6j6v.1q6d's case), plus the same board with a WINDOW on it,
    // which is what lets one member be consolidated away and then answer anyway (nxf 6j6v.93zd).
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: coding\n  members: [coding-lead]\n- name: review\n  members: [review-lead]\n\
         - name: pair\n  members: [reviewer-a, reviewer-b]\n\
         - name: timed\n  members: [reviewer-a, reviewer-b]\n  timeout: 5m\n",
    )
    .unwrap();
}

/// [`human`] on a different wall clock — the only way to drive a member past its window.
fn at(tmp: &TempDir, when: &str) -> Command {
    let mut c = human(tmp);
    c.env("NXC_NOW", when);
    c
}

/// The thread the supervisor opened for `handle` below `channel_thread` (nxf 6j6v.pf6j) — the
/// two-ended conversation a channel member is actually triggered into and answers in.
fn member_thread_of(tmp: &TempDir, channel_thread: &str, handle: &str) -> String {
    let store = nexus_chat::workspace::Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store");
    let want = format!("local/{handle}");
    store
        .supervised_children(channel_thread)
        .unwrap()
        .into_iter()
        .find(|t| {
            store
                .thread_quorum(t, NOW)
                .unwrap()
                .is_some_and(|q| q.expects == [want.clone()])
        })
        .unwrap_or_else(|| panic!("no member thread for {handle} under {channel_thread}"))
}

/// The whole tree of the operation `thread` belongs to, as `nxc status --thread` reports it.
fn tree(tmp: &TempDir, thread: &str) -> Value {
    let report = json_of(human(tmp).args(["--json", "status", "--thread", thread]));
    report["operations"]
        .as_array()
        .and_then(|ops| ops.first().cloned())
        .unwrap_or_else(|| panic!("one operation for {thread}: {report}"))
}

/// `(thread_id, parent, state)` per thread of an operation, in the order the tree renders them.
fn shape(op: &Value) -> Vec<(String, Option<String>, String)> {
    op["threads"]
        .as_array()
        .expect("threads")
        .iter()
        .map(|t| {
            (
                t["thread_id"].as_str().unwrap().to_string(),
                t["parent"].as_str().map(str::to_string),
                t["state"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// One thread's derived state, read out of the tree its operation renders as.
fn state_in(tmp: &TempDir, root: &str, id: &str) -> String {
    shape(&tree(tmp, root))
        .into_iter()
        .find(|(t, _, _)| t == id)
        .map(|(_, _, s)| s)
        .unwrap_or_else(|| panic!("no {id} in the tree of {root}"))
}

// ---- nxf 6j6v.93zd: the derived place, and only the dead end that FAILED in it ------------------

#[test]
fn only_the_dead_end_that_failed_reads_orphaned_beside_the_one_that_finished() {
    // Both shapes in ONE operation, which is the only way to show that the state discriminates
    // rather than merely fires: two member threads of the same board, both discharged, both
    // childless, both under the same settled parent — one whose answer the channel consolidated,
    // one whose answer arrived after the turn was over and went nowhere at all.
    //
    // The failed consequence is REAL and is broken at the real path: reviewer-b's window runs out,
    // `member_set_is_settled` counts it as settled on that ground, the supervisor consolidates
    // without it, and reviewer-b then answers into a turn nobody is listening to any more. Nothing
    // here constructs a state or asserts a field the test set itself.
    let tmp = workspace();
    write_declarations(&tmp);

    // 10:00 — the board is commissioned; each member's window runs to 10:05.
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "timed", "look at this"]));
    let board = opened["thread_id"].as_str().unwrap().to_string();
    let a = member_thread_of(&tmp, &board, "reviewer-a");
    let b = member_thread_of(&tmp, &board, "reviewer-b");

    // 10:10 — reviewer-a answers. reviewer-b has shown no sign of life for the whole window, so the
    // SET is settled and the channel consolidates on the spot, in this very call.
    let settled = json_of(
        at(&tmp, "2026-08-14T10:10:00Z")
            .env("NXC_ACTOR", "reviewer-a")
            .args(["--json", "reply", "--thread", &a, "looks fine"]),
    );
    assert!(
        settled["completed"].is_object(),
        "the set settled and was consolidated inside this reply: {settled}"
    );

    // 10:15 — reviewer-b's answer lands anyway. It discharges its own thread and drives nothing:
    // the channel thread no longer expects the supervisor, so the supervisor is a clean no-op.
    //
    // **Through the TEARDOWN's door, since nxf 6j6v.0vd9** — and that is the shape this actually
    // has in a real run rather than a way round the refusal. A plain `nxc reply --thread` into a
    // set that has been consolidated and handed on is refused now, exactly so that an agent typing
    // it is not told its answer landed somewhere; what still discharges such a thread is the
    // sidecar teardown, which fires precisely when a session ends still OWING an answer — which
    // reviewer-b does, its window having struck without it. That door stays open for the reason
    // `surface::settle_if_unanswered` gives: a thread nobody will ever answer must not be left
    // owing an answer forever. The dead end it leaves behind is the same one, which is what this
    // test is about.
    let late = json_of(
        at(&tmp, "2026-08-14T10:15:00Z")
            .env("NXC_ACTOR", "reviewer-b")
            .args([
                "--json",
                "reply",
                "--thread",
                &b,
                "--if-unanswered",
                "here is what I found",
            ]),
    );
    assert_eq!(
        late["posted"], true,
        "reviewer-b still owed an answer, so the settle really wrote one: {late}"
    );
    assert!(
        late["completed"].is_null() && late["woke"].is_null(),
        "the answer arrived and the consequence did not: {late}"
    );

    // The two dead ends, side by side in one tree.
    assert_eq!(
        state_in(&tmp, &board, &a),
        "answered",
        "reviewer-a's branch FINISHED — the channel moved on from it, so it is no alarm"
    );
    assert_eq!(
        state_in(&tmp, &board, &b),
        "orphaned",
        "reviewer-b's branch FAILED — it answered into a turn nothing was left to consume it"
    );

    // …and the human at the terminal finds it without being told where to look: the bare listing,
    // and the rendered line that names the state in words.
    let rendered = stdout_of(human(&tmp).args(["status"]));
    assert!(
        rendered.contains(&format!("{b}  decl:timed  ORPHANED")),
        "the workspace listing shows the failed dead end:\n{rendered}"
    );
    assert!(
        !rendered.contains(&format!("{a}  decl:timed  ORPHANED")),
        "…and only that one:\n{rendered}"
    );
}

// ---- acceptance 1: the parent edge, by the §3.1 rule, without exception -------------------------

#[test]
fn a_send_from_a_bare_terminal_opens_a_root_and_a_send_from_inside_a_session_hangs_under_it() {
    // §3.1's two ends. A human has no session, so nothing is open around the caller and the thread
    // is a ROOT — which is exactly §3.4's "the human stands at the ENDS of the chain". The persona
    // it started IS standing in a thread, so what that persona opens hangs under it, with no
    // argument passed and nothing for the agent to remember.
    let tmp = workspace();
    write_declarations(&tmp);

    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let t1 = opened["thread_id"].as_str().unwrap().to_string();
    let pm_session = opened["session"].as_str().unwrap().to_string();

    let handed_on = json_of(persona(&tmp, "pm", &pm_session).args([
        "--json",
        "send",
        "--to",
        "coder",
        "implement it",
    ]));
    let t2 = handed_on["thread_id"].as_str().unwrap().to_string();

    let op = tree(&tmp, &t2);
    assert_eq!(
        op["root"], t1,
        "the operation is anchored on the human's thread: {op}"
    );
    assert_eq!(
        shape(&op)
            .into_iter()
            .map(|(id, parent, _)| (id, parent))
            .collect::<Vec<_>>(),
        vec![(t1.clone(), None), (t2.clone(), Some(t1.clone()))],
        "the pm's send hangs under the thread it was sent FROM"
    );
}

#[test]
fn a_send_to_a_declared_channel_hangs_under_the_senders_thread_too() {
    // The other entrance (`send --to <channel>` → `facade::ask_under`). Both have to stamp, or the
    // tree would break at exactly the point §2's chain crosses into a channel.
    let tmp = workspace();
    write_declarations(&tmp);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let t1 = opened["thread_id"].as_str().unwrap().to_string();
    let pm_session = opened["session"].as_str().unwrap().to_string();

    let board = json_of(
        persona(&tmp, "pm", &pm_session).args(["--json", "send", "--to", "coding", "build it"]),
    );
    let t2 = board["thread_id"].as_str().unwrap().to_string();

    let op = tree(&tmp, &t2);
    let threads = shape(&op);
    assert_eq!(
        threads.iter().find(|(id, _, _)| id == &t2).unwrap().1,
        Some(t1),
        "the board thread hangs under the pm's own thread: {op}"
    );
}

#[test]
fn the_parent_is_the_thread_sent_from_not_the_one_acted_on_behalf_of() {
    // §3.1's ONE worked example, and the reason the rule needed writing down at all: the `#coding`
    // supervisor acts on behalf of its channel thread T2, but when it commissions the review it is
    // reading the coder's answer in T3. The item's ruling: T4 hangs under T3, not under T2.
    let tmp = workspace();
    write_declarations(&tmp);

    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let pm_session = opened["session"].as_str().unwrap().to_string();
    let t2 = json_of(
        persona(&tmp, "pm", &pm_session).args(["--json", "send", "--to", "coding", "build it"]),
    );
    let t2_id = t2["thread_id"].as_str().unwrap().to_string();
    let lead = dry_session(&tmp, "coding-lead");

    // The supervisor opens the coder's thread out of its channel thread…
    let t3 = json_of(persona(&tmp, "coding-lead", &lead).args([
        "--json",
        "send",
        "--to",
        "coder",
        "implement it",
    ]));
    let t3_id = t3["thread_id"].as_str().unwrap().to_string();
    let coder = t3["session"].as_str().unwrap().to_string();

    // …the coder answers, which resumes the supervisor IN T3…
    json_of(persona(&tmp, "coder", &coder).args(["--json", "reply", "--thread", &t3_id, "done"]));

    // …and the review it commissions now hangs under T3.
    let t4 = json_of(persona(&tmp, "coding-lead", &lead).args([
        "--json",
        "send",
        "--to",
        "review",
        "please review",
    ]));
    let t4_id = t4["thread_id"].as_str().unwrap().to_string();

    let op = tree(&tmp, &t4_id);
    let parent_of = |id: &str| -> Option<String> {
        shape(&op)
            .into_iter()
            .find(|(t, _, _)| t == id)
            .and_then(|(_, p, _)| p)
    };
    assert_eq!(
        parent_of(&t4_id),
        Some(t3_id.clone()),
        "T4's parent is T3 (the thread sent FROM), never T2 (the thread acted on behalf of): {op}"
    );
    // …and the tree stays readable anyway, because T3 itself hangs under T2 — now via the member
    // thread the #coding supervisor opened for `coding-lead`, which is the thread that role was
    // triggered into and is therefore the thread it sends FROM (§3.1, unchanged).
    let t2_member = member_thread_of(&tmp, &t2_id, "coding-lead");
    assert_eq!(parent_of(&t3_id), Some(t2_member.clone()), "{op}");
    assert_eq!(parent_of(&t2_member), Some(t2_id), "{op}");
}

/// The internal session id the dry worker was handed for `role` — what a spawned role would find in
/// its own `NXC_SESSION`. Read out of the trigger log rather than guessed, so the test acts under
/// the session the engine really bound. (A channel fan-out reports no session on its receipt: the
/// board is the addressable thing, so the member sessions are only in the trigger log.)
fn dry_session(tmp: &TempDir, role: &str) -> String {
    let log = std::fs::read_to_string(tmp.path().join("dry.log")).unwrap_or_default();
    log.lines()
        .filter(|l| l.starts_with(&format!("trigger role={role} ")))
        .filter_map(|l| {
            l.split_whitespace()
                .find_map(|w| w.strip_prefix("session=").map(str::to_string))
        })
        .next_back()
        .unwrap_or_else(|| panic!("no dry trigger for {role} in:\n{log}"))
}

// ---- acceptance 3: the §2 chain, driven, and the whole tree read back ---------------------------

#[test]
fn the_chain_from_section_two_is_driven_across_three_channels_and_the_whole_tree_reads_back() {
    let tmp = workspace();
    write_declarations(&tmp);

    // Nutzer -> send --to pm -> T1
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship the release"]));
    let t1 = opened["thread_id"].as_str().unwrap().to_string();
    let pm = opened["session"].as_str().unwrap().to_string();

    // pm -> send --to #coding -> T2
    let t2 =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();
    let coding_lead = dry_session(&tmp, "coding-lead");

    // Betreuer #coding -> send --to coder -> T3
    let handed = json_of(persona(&tmp, "coding-lead", &coding_lead).args([
        "--json",
        "send",
        "--to",
        "coder",
        "implement it",
    ]));
    let t3 = handed["thread_id"].as_str().unwrap().to_string();
    let coder = handed["session"].as_str().unwrap().to_string();

    // coder -> reply --thread T3
    json_of(persona(&tmp, "coder", &coder).args([
        "--json",
        "reply",
        "--thread",
        &t3,
        "first cut is in",
    ]));

    // Betreuer #coding -> send --to #review -> T4
    let t4 = json_of(persona(&tmp, "coding-lead", &coding_lead).args([
        "--json",
        "send",
        "--to",
        "review",
        "review the first cut",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let review_lead = dry_session(&tmp, "review-lead");

    // Betreuer #review -> send --to <je Reviewer> -> T5a/T5b/T5c
    let mut reviewer_threads = Vec::new();
    for who in ["reviewer-a", "reviewer-b", "reviewer-c"] {
        let r = json_of(persona(&tmp, "review-lead", &review_lead).args([
            "--json",
            "send",
            "--to",
            who,
            "read this independently",
        ]));
        reviewer_threads.push((
            who,
            r["thread_id"].as_str().unwrap().to_string(),
            r["session"].as_str().unwrap().to_string(),
        ));
    }
    // 3 Reviewer -> reply --thread T5* (parallel, unabhaengig voneinander)
    for (who, thread, session) in &reviewer_threads {
        json_of(persona(&tmp, who, session).args([
            "--json",
            "reply",
            "--thread",
            thread,
            "looks good to me",
        ]));
    }

    // Betreuer #review -> reply --thread T4 -> zurueck an Betreuer #coding.
    //
    // Since 6j6v.pf6j `review-lead` answers in ITS OWN member thread below T4; discharging that
    // settles the #review supervisor's whole set, and the supervisor is what then delivers up to T4's
    // requester. The step of §2 is unchanged — who does the delivering is.
    let t4_member = member_thread_of(&tmp, &t4, "review-lead");
    json_of(persona(&tmp, "review-lead", &review_lead).args([
        "--json",
        "reply",
        "--thread",
        &t4_member,
        "three approvals",
    ]));
    // Betreuer #coding -> reply --thread T3 -> coder wird fortgesetzt
    json_of(persona(&tmp, "coding-lead", &coding_lead).args([
        "--json",
        "reply",
        "--thread",
        &t3,
        "reviewers are happy, wrap it up",
    ]));
    // coder -> reply --thread T3 -> Abschlussbericht
    json_of(persona(&tmp, "coder", &coder).args([
        "--json",
        "reply",
        "--thread",
        &t3,
        "final report: shipped",
    ]));
    // Betreuer #coding -> reply --thread T2 -> pm wird fortgesetzt (same move, one level down).
    let t2_member = member_thread_of(&tmp, &t2, "coding-lead");
    json_of(persona(&tmp, "coding-lead", &coding_lead).args([
        "--json",
        "reply",
        "--thread",
        &t2_member,
        "coding is done",
    ]));
    // pm -> reply --thread T1 -> Nutzer
    json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &t1,
        "the release is out",
    ]));

    // ---- the whole tree, from ANY of its threads, across every channel it crossed.
    let op = tree(&tmp, &reviewer_threads[2].1);
    assert_eq!(
        op["root"], t1,
        "one operation, anchored on the human's thread"
    );
    let threads = shape(&op);
    let expected_edges: Vec<(String, Option<String>)> = vec![
        (t1.clone(), None),
        (t2.clone(), Some(t1.clone())),
        (t2_member.clone(), Some(t2.clone())),
        (t3.clone(), Some(t2_member.clone())),
        (t4.clone(), Some(t3.clone())),
        (t4_member.clone(), Some(t4.clone())),
        (reviewer_threads[0].1.clone(), Some(t4_member.clone())),
        (reviewer_threads[1].1.clone(), Some(t4_member.clone())),
        (reviewer_threads[2].1.clone(), Some(t4_member.clone())),
    ];
    assert_eq!(
        threads
            .iter()
            .map(|(id, parent, _)| (id.clone(), parent.clone()))
            .collect::<Vec<_>>(),
        expected_edges,
        "NINE threads, one tree, depth-first from the root — §2's seven plus the member thread each \
         of the two channels' supervisors opened (nxf 6j6v.pf6j): {op}"
    );

    // Three channels at least — the point of the anchor NOT being the channel (§3.2).
    let channels: std::collections::BTreeSet<String> = op["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["channel_id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        channels.len() >= 3 && channels.contains("decl:coding") && channels.contains("decl:review"),
        "the chain spans several channels, and no single one of them sees it whole: {channels:?}"
    );
    // …and each of the three is a THIRD of the truth, which is the argument for the operation being
    // the anchor: `--channel coding` finds the operation only through its root, which is elsewhere.
    let by_channel = json_of(human(&tmp).args(["--json", "status", "--channel", "coding"]));
    assert_eq!(
        by_channel["operations"].as_array().unwrap().len(),
        0,
        "the chain's root is not in #coding, so #coding is not an entry point to it: {by_channel}"
    );

    // The states at the end of a chain that ran to completion (a71h's DoD, three states + the root
    // carve-out of §3.4).
    let state_of = |id: &str| -> String {
        threads
            .iter()
            .find(|(t, _, _)| t == id)
            .map(|(_, _, s)| s.clone())
            .unwrap()
    };
    assert_eq!(state_of(&t1), "answered", "the root has its answer");
    assert_eq!(
        op["threads"][0]["awaiting_human"], true,
        "…and it is waiting on the human at the other end, which is NOT a standstill (§3.4)"
    );
    for (label, id) in [
        ("T2", &t2),
        ("T2/member", &t2_member),
        ("T3", &t3),
        ("T4", &t4),
        ("T4/member", &t4_member),
    ] {
        assert_eq!(
            state_of(id),
            "answered",
            "{label} was answered and carried on: {op}"
        );
    }
    // The rendered tree a human actually reads, indented by depth — one operation, nine threads,
    // over the three channels it crossed.
    let rendered = stdout_of(human(&tmp).args(["status", "--thread", &t1]));
    let lines: Vec<&str> = rendered.lines().collect();
    assert_eq!(
        lines.len(),
        10,
        "a header plus one line per thread:\n{rendered}"
    );
    assert!(
        lines[0].starts_with(&format!("operation {t1}")),
        "{rendered}"
    );
    assert!(
        lines[1].starts_with(&format!("  {t1}")) && lines[1].contains("awaiting you"),
        "the root sits at depth 0 and says the human has it:\n{rendered}"
    );
    assert!(
        lines[5].starts_with(&format!("          {t4}")) && lines[5].contains("decl:review"),
        "T4 is five levels down now, still in the third channel:\n{rendered}"
    );
    assert!(
        lines[6].starts_with(&format!("            {t4_member}")),
        "the #review supervisor's own member thread sits between T4 and the reviewers:\n{rendered}"
    );
    assert!(
        lines[7].starts_with(&format!("              {}", reviewer_threads[0].1)),
        "and each reviewer thread one level below that:\n{rendered}"
    );

    for (who, thread, _) in &reviewer_threads {
        // **The false positive a71h reported, and 6j6v.93zd's answer to it.** These three leaves
        // satisfy that item's formula word for word — discharged, no child, a parent that owes
        // nothing — from the moment the #review supervisor consolidates their answers into T4, and
        // they used to read `orphaned` here, in a chain where every single step worked. What tells
        // them from a branch that died is that their parent MOVED after they were discharged: the
        // supervisor's consolidating discharge of T4 IS their answers being acted on.
        assert_eq!(
            state_of(thread),
            "answered",
            "{who}: a dead end the supervisor consolidated is finished, not failed"
        );
    }
}

// ---- acceptance 2: the three forms ------------------------------------------------------------

#[test]
fn status_has_three_forms_and_the_channel_one_is_an_entry_point_not_an_anchor() {
    let tmp = workspace();
    write_declarations(&tmp);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let t1 = opened["thread_id"].as_str().unwrap().to_string();
    let pm = opened["session"].as_str().unwrap().to_string();
    let t2 =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();

    // (a) everything open in the workspace
    let all = json_of(human(&tmp).args(["--json", "status"]));
    assert_eq!(all["operations"].as_array().unwrap().len(), 1);
    assert_eq!(all["operations"][0]["root"], t1);
    assert_eq!(
        all["operations"][0]["threads"].as_array().unwrap().len(),
        3,
        "the whole tree, not just the root — T1, the #coding channel thread, and the member thread \
         its supervisor opened (nxf 6j6v.pf6j): {all}"
    );

    // (b) the operations that STARTED in a channel — the root's channel, not any thread's
    let dm = all["operations"][0]["channel_id"]
        .as_str()
        .unwrap()
        .to_string();
    let by_root_channel = json_of(human(&tmp).args(["--json", "status", "--channel", &dm]));
    assert_eq!(by_root_channel["operations"][0]["root"], t1);
    let by_inner_channel = json_of(human(&tmp).args(["--json", "status", "--channel", "coding"]));
    assert_eq!(
        by_inner_channel["operations"].as_array().unwrap().len(),
        0,
        "#coding carries T2, but the OPERATION did not start there: {by_inner_channel}"
    );

    // (c) one operation, named by any of its threads — the same tree from the leaf as from the root
    let from_leaf = json_of(human(&tmp).args(["--json", "status", "--thread", &t2]));
    let from_root = json_of(human(&tmp).args(["--json", "status", "--thread", &t1]));
    assert_eq!(
        from_leaf, from_root,
        "the tree is the operation, not the entry point"
    );

    // …and an unknown thread is `not_found`, not an empty answer.
    human(&tmp)
        .args(["status", "--thread", "m-nosuchthread"])
        .assert()
        .failure();
}

#[test]
fn the_human_readable_status_tells_a_root_waiting_on_its_human_from_one_that_hangs() {
    // Acceptance 5. Both are "nothing is happening"; only one of them is a problem. §3.4: a root
    // waiting on its human is the NORMAL state, and only there — no human stands inside the flow.
    let tmp = workspace();
    write_declarations(&tmp);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let t1 = opened["thread_id"].as_str().unwrap().to_string();
    let pm = opened["session"].as_str().unwrap().to_string();

    let waiting = stdout_of(human(&tmp).args(["status"]));
    assert!(
        waiting.contains("waiting on local/pm"),
        "before the answer the root is waiting on its AGENT: {waiting}"
    );
    assert!(
        !waiting.contains("awaiting you"),
        "…and nobody is waiting on the human yet: {waiting}"
    );

    json_of(persona(&tmp, "pm", &pm).args(["--json", "reply", "--thread", &t1, "here it is"]));

    // **`--all`, because since nxf 6j6v.1vxs the bare listing is not where a finished operation
    // is.** That is this test's own distinction seen from the other side: "the human has it" is the
    // NORMAL end, and the default view answers "is anything still going on here?", so the normal end
    // left it. The claim under test is unchanged and still the point — an answered ROOT must say the
    // human has it rather than read as a thread nobody works on — it is now made where that
    // operation lives. The bare form's silence is asserted below.
    let answered = stdout_of(human(&tmp).args(["status", "--all"]));
    assert!(
        answered.contains("awaiting you (answered by local/pm)"),
        "an answered ROOT says the human has it, rather than reading as a thread nobody works on: \
         {answered}"
    );
    // **And it is NOT live, which is the half nxf 6j6v.1vxs turned round.** This block used to
    // assert the opposite — "an operation whose result nobody has picked up is not finished" — and
    // that reading is what made the default view grow without bound: `awaiting_human` is derived
    // from two facts that never stop being true, so an operation that entered on it never left.
    // Nothing is owed here, nothing was handed back, no checkout is held: the operation is over,
    // and the flag on its root says whose it is now, not that something is still running.
    let json = json_of(human(&tmp).args(["--json", "status", "--all"]));
    assert_eq!(json["operations"][0]["live"], false, "{json}");
    assert_eq!(json["operations"][0]["threads"][0]["awaiting_human"], true);
    assert_eq!(
        json["operations"][0]["open"], 0,
        "nothing is owed any more — which is exactly why it must not read as 'hangs': {json}"
    );
    // The bare listing is therefore quiet, and says where the rest went rather than leaving a
    // reader to conclude the workspace is empty.
    let bare = stdout_of(human(&tmp).args(["status"]));
    assert!(
        bare.contains("nothing open") && bare.contains("--all"),
        "the default view answers \"is anything still going on here?\" and points at the rest: \
         {bare}"
    );
}

// ---- acceptance 4: derived, no process record --------------------------------------------------

#[test]
fn the_status_is_derived_and_nothing_in_the_store_stands_beside_it() {
    // §3.3, and the item's own DoD: "im Store entsteht kein neuer Vorgangs-Datensatz". Three
    // separate claims, because each could fail on its own.
    let tmp = workspace();
    write_declarations(&tmp);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let t1 = opened["thread_id"].as_str().unwrap().to_string();
    let pm = opened["session"].as_str().unwrap().to_string();
    json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]));

    let db = tmp.path().join(".nxs").join("db.sqlite");
    let conn = rusqlite::Connection::open(&db).unwrap();

    // (1) No table was created to hold an operation. The tree lives in `threads.parent`, which is a
    // column on a view that already existed.
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for forbidden in ["operations", "operation_runs", "thread_trees", "chains"] {
        assert!(
            !tables.contains(&forbidden.to_string()),
            "a process record crept in: {tables:?}"
        );
    }

    // (2) There is no process record beside the tree at all. It used to be worth asserting that
    // `workflow_runs` was EMPTY here — the record this verb replaced; 6j6v.dvyq §3 removed the
    // record, and `channel_flow.rs` asserts the table itself is gone.

    // (3) Reading the status appends NOTHING to the op log — it is a derivation, not a record that
    // has to be kept up to date.
    let ops_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM ops", [], |r| r.get(0))
        .unwrap();
    json_of(human(&tmp).args(["--json", "status"]));
    json_of(human(&tmp).args(["--json", "status", "--thread", &t1]));
    json_of(human(&tmp).args(["--json", "status", "--channel", "coding"]));
    let ops_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM ops", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ops_before, ops_after, "status writes nothing at all");
}

// ---- acceptance 6: the three states -------------------------------------------------------------

#[test]
fn the_three_states_are_derivable_from_the_same_chain_as_it_advances() {
    // open → answered → ORPHANED, each observed on a thread of a real chain at the moment it holds.
    let tmp = workspace();
    write_declarations(&tmp);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let t1 = opened["thread_id"].as_str().unwrap().to_string();
    let pm = opened["session"].as_str().unwrap().to_string();
    let handed =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coder", "implement it"]));
    let t2 = handed["thread_id"].as_str().unwrap().to_string();
    let coder = handed["session"].as_str().unwrap().to_string();

    let state_of = |thread: &str, id: &str| -> String {
        shape(&tree(&tmp, thread))
            .into_iter()
            .find(|(t, _, _)| t == id)
            .map(|(_, _, s)| s)
            .unwrap()
    };

    // OPEN: the coder owes an answer.
    assert_eq!(state_of(&t1, &t2), "open");

    // The coder answers. T2 is now discharged, has no child — and its parent T1 is still OPEN,
    // because the pm has not answered the human yet. Somebody upstream is still waiting for
    // something, so this is ANSWERED, not orphaned.
    json_of(persona(&tmp, "coder", &coder).args(["--json", "reply", "--thread", &t2, "done"]));
    assert_eq!(state_of(&t1, &t2), "answered");
    assert_eq!(state_of(&t1, &t1), "open", "the pm still owes the human");

    // The pm answers the human. T1 is now the answered ROOT — never orphaned, whatever the rule
    // says, because its human is the other end of the chain (§3.4).
    json_of(persona(&tmp, "pm", &pm).args(["--json", "reply", "--thread", &t1, "shipped"]));
    assert_eq!(state_of(&t1, &t1), "answered");
    // **And T2 stays ANSWERED, which is the sharpening of nxf 6j6v.93zd.** It satisfies a71h's
    // formula word for word — discharged, no child, a parent that owes nothing — and until this item
    // it read `orphaned` here, on a chain in which nothing whatsoever went wrong. What settles it is
    // that the parent MOVED after this branch was discharged: the pm's answer to the human is the
    // coder's report being acted on.
    assert_eq!(
        state_of(&t1, &t2),
        "answered",
        "a dead end the chain moved on from is finished, not failed"
    );
    let op = tree(&tmp, &t1);
    assert_eq!(op["threads"][0]["awaiting_human"], true);
    assert_eq!(
        op["threads"][1]["awaiting_human"], false,
        "only ever true at the top — no human stands inside the flow: {op}"
    );

    // ORPHANED, the third state, on the shape that genuinely earns it: the pm commissions one more
    // piece of work and then the chain is closed out above it, so the coder's answer lands where
    // nothing is left to consume it. Same three conditions as T2 — and this time the parent never
    // moved again.
    let more = json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "send",
        "--to",
        "coder",
        "and the changelog",
    ]));
    let t3 = more["thread_id"].as_str().unwrap().to_string();
    let coder_again = more["session"].as_str().unwrap().to_string();
    assert_eq!(state_of(&t1, &t3), "open", "the coder owes this one");
    json_of(persona(&tmp, "coder", &coder_again).args([
        "--json",
        "reply",
        "--thread",
        &t3,
        "changelog written",
    ]));
    assert_eq!(
        state_of(&t1, &t3),
        "orphaned",
        "the answer arrived, the consequence did not: T1 was already closed out to its human"
    );
}

// ---- acceptance 7: the edge thread → session (nxf 6j6v.1q6d) -----------------------------------

#[test]
fn a_silent_assignee_of_an_open_thread_is_reachable_from_the_thread_alone() {
    // **The case the whole item exists for.** 1q6d was opened by a human staring at a silent
    // conversation for ten and a half minutes while a finished answer sat uncollected in a transcript
    // nobody could reach — and an OPEN thread is by definition one whose assignee has not answered
    // yet. So the consumer must get from the thread to the session while there is not a single
    // message from the assignee to read a return address off.
    let tmp = workspace();
    write_declarations(&tmp);

    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let thread = opened["thread_id"].as_str().unwrap().to_string();
    let pm = opened["session"].as_str().unwrap().to_string();

    // The pm is working and has said NOTHING in the thread — this is the ten-minute silence.
    persona(&tmp, "pm", &pm)
        .args(["transcript", "append", "--session", &pm])
        .write_stdin("{\"kind\":\"assistant\",\"data\":{\"text\":\"drafting the plan\"}}\n")
        .assert()
        .success();

    // The consumer knows the thread id and nothing else.
    let op = tree(&tmp, &thread);
    let t = &op["threads"][0];
    assert_eq!(t["state"], "open", "nobody has answered: {op}");
    assert_eq!(
        t["session"].as_str(),
        Some(pm.as_str()),
        "the session working on it rides on the thread even though it has posted nothing: {op}"
    );

    // …and from there the EXISTING transcript path carries on: the human can watch it think.
    let transcript = json_of(human(&tmp).args(["--json", "transcript", "show", &pm]));
    assert!(
        serde_json::to_string(&transcript)
            .unwrap()
            .contains("drafting the plan"),
        "the silence is watchable after all: {transcript}"
    );
}

#[test]
fn each_member_thread_names_its_own_assignee_with_no_tie_break_left_to_lose() {
    // REWRITTEN for nxf 6j6v.pf6j. It used to drive both members onto ONE board thread, where the
    // edge had to CHOOSE between two silent sessions standing on the same thread and did so by
    // session age — a documented, deliberately temporary tie-break, and the reason a board of three
    // reviewers could read as a board of one. There is nothing left to tie-break: each member has a
    // thread of its own with exactly one assignee, so both branches of the edge are exercised here
    // side by side and each has exactly one possible answer.
    //
    // Branch 1 (the party still owed) and branch 2 (the one that answered) now live on DIFFERENT
    // threads of the same operation, which is the strongest form of "neither can be satisfied by
    // accident": a regression to either branch alone gets one of them wrong.
    //
    // What this test does NOT separate, said out loud (review Test Quality #1): the OUTER guard that
    // shuts branch 1 off on a settled thread from branch 1's own inner "has this session spoken"
    // clause. On `a_thread` below the only session standing is the one that answered, so both guards
    // exclude it and a broken outer guard would print the same bytes. That is not a hole this suite
    // can close — since pf6j the surface gives a member thread exactly ONE session, which is the
    // property under test here — so the scene where the two guards disagree (a second session
    // resumed into a settled thread, silent) is driven where the guard lives, by
    // `facade`'s `a_session_resumed_into_a_settled_thread_does_not_displace_the_one_that_answered`.
    let tmp = workspace();
    write_declarations(&tmp);

    let board = json_of(human(&tmp).args(["--json", "send", "--to", "pair", "look at this"]));
    let channel_thread = board["thread_id"].as_str().unwrap().to_string();
    let a = dry_session(&tmp, "reviewer-a");
    let b = dry_session(&tmp, "reviewer-b");
    assert_ne!(a, b, "both members really were started");
    let a_thread = member_thread_of(&tmp, &channel_thread, "reviewer-a");
    let b_thread = member_thread_of(&tmp, &channel_thread, "reviewer-b");

    json_of(
        persona(&tmp, "reviewer-a", &a).args(["--json", "reply", "--thread", &a_thread, "lgtm"]),
    );
    persona(&tmp, "reviewer-b", &b)
        .args(["transcript", "append", "--session", &b])
        .write_stdin("{\"kind\":\"assistant\",\"data\":{\"text\":\"reading the diff\"}}\n")
        .assert()
        .success();

    let op = tree(&tmp, &channel_thread);
    let thread_named = |id: &str| -> Value {
        op["threads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["thread_id"].as_str() == Some(id))
            .cloned()
            .unwrap_or_else(|| panic!("{id} in {op}"))
    };

    // BRANCH 1 — reviewer-b's own thread is still OPEN, and the only party it can be waiting for is
    // reviewer-b. A return-address-only read answers `None` here (b has posted nothing at all), so
    // this is what fails if the silent edge ever regresses.
    let b_view = thread_named(&b_thread);
    assert_eq!(b_view["state"], "open", "reviewer-b still owes: {b_view}");
    assert_eq!(
        b_view["outstanding"],
        serde_json::json!(["local/reviewer-b"]),
        "one thread, one end, no ambiguity about who is owed: {b_view}"
    );
    assert_eq!(
        b_view["session"].as_str(),
        Some(b.as_str()),
        "the silent party being waited for: {b_view}"
    );
    let transcript = json_of(human(&tmp).args(["--json", "transcript", "show", &b]));
    assert!(
        serde_json::to_string(&transcript)
            .unwrap()
            .contains("reading the diff"),
        "and its transcript reads back: {transcript}"
    );

    // BRANCH 2 — reviewer-a's own thread is settled, so the silent branch cannot fire on it (its
    // guard is exactly "somebody expected has not spoken since the declaration") and what is left is
    // the return address of the answer.
    let a_view = thread_named(&a_thread);
    assert_eq!(a_view["state"], "answered", "{a_view}");
    assert_eq!(
        a_view["session"].as_str(),
        Some(a.as_str()),
        "the return address of the answer, on the thread that carries it: {a_view}"
    );

    // …and neither thread's answer can be the other's: the two branches never met a shared register.
    assert_ne!(a_view["session"], b_view["session"], "{op}");
}

#[test]
fn status_reads_the_same_for_every_caller_because_there_is_no_caller() {
    // 1q6d's second acceptance point: no human/agent vocabulary came into being — not as a
    // declaration field, not as a derivation from the participants. The strongest form of that
    // promise is structural (`facade::status` takes no caller handle at all), and this is the
    // behavioural half: the same report, byte for byte, for a human at a terminal, for the persona
    // that is being watched, and for an unrelated third party who is a member of nothing here.
    let tmp = workspace();
    write_declarations(&tmp);
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "plan the release"]));
    let thread = opened["thread_id"].as_str().unwrap().to_string();
    let pm = opened["session"].as_str().unwrap().to_string();
    json_of(persona(&tmp, "pm", &pm).args(["--json", "reply", "--thread", &thread, "here it is"]));

    // `--all`, because the round above is answered and a finished operation left the default
    // listing with nxf 6j6v.1vxs. The claim is about the CALLER making no difference, so the form
    // only has to be one that has something to say — and it must be the same form for all three.
    let as_human = stdout_of(human(&tmp).args(["--json", "status", "--all"]));
    let as_the_persona = stdout_of(persona(&tmp, "pm", &pm).args(["--json", "status", "--all"]));
    let as_a_stranger = stdout_of(
        persona(&tmp, "reviewer-c", "m-not-a-session").args(["--json", "status", "--all"]),
    );
    assert_eq!(as_human, as_the_persona, "no projection by caller");
    assert_eq!(as_human, as_a_stranger, "and none by membership either");
    assert!(
        as_human.contains("\"session\":\"") && as_human.contains("awaiting_human"),
        "…including the two fields a caller-dependent read would have been tempted to hide: \
         {as_human}"
    );
}
