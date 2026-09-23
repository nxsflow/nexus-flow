//! The CLAIM AREA of the working-tree lease is a SUBTREE, not a run (nxf 6j6v.1xw1), and since nxf
//! 6j6v.8y6t the subtree of the OPERATION.
//!
//! Two questions with two different answers, which is the whole of 1xw1:
//!
//! * **WHETHER** a chain is protected comes from the channel DECLARATION, derived UPWARD from its
//!   declared members — ONE hop, never transitive. `#coding` is protected because `coder` says it
//!   needs the working copy; `#review`, whose members only read, does not inherit that from
//!   `#coding` merely by sharing a member with it. UNCHANGED by 8y6t, expressly.
//! * **HOW FAR** it reaches is a SUBTREE — everything below the anchor, including the `#review`
//!   round `#coding` is waiting on. What MOVED is the anchor: 1xw1 put it at the claim thread (T2)
//!   and left T1, the conversation with the human, outside; 8y6t puts it at the root of the
//!   OPERATION, so T1 is in. The risk 1xw1 named by that boundary — one unanswered human keeping the
//!   working copy until tomorrow — is answered by 8y6t's other half instead: the lease's bound is
//!   the operation's own declared windows rather than a flat two hours.
//!
//! `the_claim_is_the_operation.rs` next door is 8y6t's own suite — the owner's measured chain, the
//! two-round window, and the bound. This file is 1xw1's, carried forward onto the new anchor.
//!
//! And RELEASE is "the claim thread is CLOSED", not "nothing happens to be outstanding":
//! a `reply` without `--escalate` closes it, a `reply --escalate` does not, and — the owner's
//! ruling carried into this item — neither does an open QUESTION. In both hand-back shapes nobody is
//! working and the task is mid-flight, which is the state the lease exists for.
//!
//! Everything here is driven through the real `nxc` binary against the dry worker: no thread, no
//! parent edge, no expectation and no lease row is constructed by hand. **With ONE exception, and it
//! is a named one**: a `question` reply has no entrance on either surface since nxf 6j6v.ckeq took
//! `reply --kind`, so the three cases that need one call [`reply_with_a_question`], which drives the
//! same `orchestration::reply` the binary drives, one layer below the flag that is gone. Everything
//! around it in those tests is still the binary.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::working_tree::WorkScope;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-08-16T09:00:00Z";
/// Inside `WORKING_TREE_LEASE_BOUND` ("2h") from [`NOW`] — the lease taken at [`NOW`] still stands
/// here, so a chain that is still holding at this instant is holding because the RULE says so and
/// not because the clock has not run out.
const A_LONG_WAIT_LATER: &str = "2026-08-16T10:59:59Z";
/// One second past [`NOW`] + the bound: the hard-death backstop, and the reason "holds too long" is
/// bounded rather than a wedge.
const PAST_THE_BOUND: &str = "2026-08-16T11:00:01Z";

// ---- harness ----------------------------------------------------------------------------------

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

/// The human at the keyboard: no spawned-context stamps, so it stands in no thread.
///
/// Deliberately NOT `NXF_DETERMINISTIC_IDS` (nxf 6j6v.k8bw / a71h finding A): that counter is
/// shared by messages, threads and sessions, so in a chain this long a thread id collides with an
/// unrelated MESSAGE id and `reply --thread` lands in the wrong thread. Every id here is read back
/// off a receipt.
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

/// [`human`] on a different wall clock — how "the human does not answer for a long time" is driven.
fn at(tmp: &TempDir, when: &str) -> Command {
    let mut c = human(tmp);
    c.env("NXC_NOW", when);
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

/// Post a `MessageKind::Question` reply into `thread`, IN PROCESS, with the whole of
/// [`orchestration::reply`]'s routing behind it — the turn discharge, the consolidation, the wake.
///
/// **Why not `nxc reply --kind question`, which is what these tests used to type.** `--kind` came
/// off the command line with nxf 6j6v.ckeq (decision 3: `kind` falls as a CALLER option and stays as
/// a CARRIER, with exactly two of its five values in use), so `question` has no entrance on either
/// surface any anymore. The RULE it feeds is untouched and is the owner's, of 2026-08-16:
/// `working_tree::hands_the_task_back` treats an open question exactly like an escalation, because
/// for a working copy the two are the same fact — nobody is working, the task is mid-flight. Losing
/// the entrance must not lose the rule, so these three cases drive the same `orchestration::reply`
/// the CLI drives, one layer below the flag that is gone.
///
/// It writes into the SAME workspace and the SAME dry log the `nxc` subprocesses around it use, so
/// `started`/`trigger_lines` see its triggers exactly as they see theirs. The store handle is
/// dropped before returning, so the next `nxc` invocation opens the file cleanly.
fn reply_with_a_question(tmp: &TempDir, actor: &str, session: &str, thread: &str, body: &str) {
    let ws = Workspace::resolve(None, tmp.path()).expect("resolve workspace");
    let root = ws.workspace_root().expect("workspace root").to_path_buf();
    let db_path = ws.db_path_str().expect("db path");
    let mut store = ws.open_chat_store().expect("open chat store");
    let defs = nexus_chat::definitions::Definitions::resolve(&root).expect("declarations");
    let worker = nexus_chat::worker::WorkerConfig::Dry {
        log: Some(tmp.path().join("dry.log")),
    }
    .build()
    .expect("dry worker");
    let ctx = nexus_chat::orchestration::Ctx {
        now: NOW,
        origin: "local",
        actor,
        session: Some(session),
        hop: 0,
        defs: &defs,
        worker: &*worker,
        timer: &nexus_chat::timer::DisabledTimer,
        namer: &nexus_chat::naming::DryNamer,
        db_path: &db_path,
        project_claude_md: None,
        module_primes: None,
        machines: None,
    };
    nexus_chat::orchestration::reply(
        &ctx,
        &mut store,
        nexus_chat::orchestration::ReplyRequest {
            target: thread,
            body,
            kind: nexus_chat::model::MessageKind::Question,
            priority: nexus_chat::model::Priority::Normal,
            disposition: nexus_chat::model::Disposition::InTurn,
            refs: nexus_chat::model::Refs {
                session_id: Some(session.to_string()),
                ..Default::default()
            },
            model: None,
            if_unanswered: false,
        },
    )
    .expect("the question is posted and routed");
}

/// Every dry-log line that OPENS a trigger entry (the message field embeds real newlines, so only
/// the first physical line of an entry starts with `trigger role=`).
fn trigger_lines(tmp: &TempDir) -> Vec<String> {
    std::fs::read_to_string(tmp.path().join("dry.log"))
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("trigger role="))
        .map(str::to_string)
        .collect()
}

/// Whether the worker was ever handed a trigger for `handle` — the load-bearing evidence that a
/// rival did or did not START, since a queued trigger is one the worker never heard about.
fn started(tmp: &TempDir, handle: &str) -> bool {
    trigger_lines(tmp)
        .iter()
        .any(|l| l.starts_with(&format!("trigger role={handle} ")))
}

/// How many times the worker was handed a trigger for `handle` — a RESUME is a second line, so this
/// is how "was this session ever put back to work?" is read off the runtime's own record.
fn trigger_count(tmp: &TempDir, handle: &str) -> usize {
    trigger_lines(tmp)
        .iter()
        .filter(|l| l.starts_with(&format!("trigger role={handle} ")))
        .count()
}

/// The internal session id the worker was handed for `handle`.
fn dry_session(tmp: &TempDir, handle: &str) -> String {
    let marker = format!("trigger role={handle} session=");
    trigger_lines(tmp)
        .iter()
        .find(|l| l.starts_with(&marker))
        .map(|l| {
            l[marker.len()..]
                .split_whitespace()
                .next()
                .unwrap()
                .to_string()
        })
        .unwrap_or_else(|| panic!("no dry-log trigger for {handle}"))
}

/// One thread's working-tree state as the BOARD renders it (`nxc threads show --json`) — the
/// `"holding" | "waiting" | null` a human or an app reads, not something this test remembered.
///
/// `reader` is a handle that may actually see the thread: `threads show` enforces the channel's own
/// visibility, so a declared channel's board is read by one of its members and a DM by its own two
/// ends. Which reader is used never changes the answer — `working_tree_status_by_thread` takes no
/// caller at all — it only decides whether the board can be read.
fn board_state(tmp: &TempDir, reader: &str, when: &str, thread: &str) -> Value {
    let view = json_of(
        at(tmp, when)
            .env("NXC_ACTOR", reader)
            .args(["--json", "threads", "show", thread]),
    );
    assert!(
        view.as_object().unwrap().contains_key("working_tree"),
        "the key is always rendered, even in the ordinary case: {view}"
    );
    view["working_tree"].clone()
}

/// The thread the supervisor opened for `handle` below `channel_thread` (nxf 6j6v.pf6j).
fn member_thread_of(tmp: &TempDir, channel_thread: &str, handle: &str) -> String {
    let store = open_store(tmp);
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

/// The declarations of a71h §2's chain, with ONE thing said about the working copy in the whole
/// file: `coder` declares that it needs it.
///
/// * `#coding` declares **nothing** — it is protected because a declared MEMBER of it is.
/// * `#review` declares nothing either, and its members only read. It shares the member
///   `coding-lead` with `#coding`, which is what a TRANSITIVE derivation would carry the protection
///   across; one hop must not.
fn write_declarations(tmp: &TempDir, coding_extra: &str) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["pm", "coding-lead", "reviewer-a", "reviewer-b"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
        )
        .unwrap();
    }
    for handle in ["coder", "rival", "late-rival", "helper"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!(
                "handle: {handle}\nsystem_prompt: You are {handle}.\nworking_tree: exclusive\n"
            ),
        )
        .unwrap();
    }
    std::fs::write(
        roles.join("channels.yaml"),
        format!(
            "- name: coding\n  members: [coding-lead, coder]\n{coding_extra}\
             - name: review\n  members: [coding-lead, reviewer-a, reviewer-b]\n\
             - name: rework\n  members: [coding-lead, helper]\n"
        ),
    )
    .unwrap();
}

/// The channel declaration text, so a test can assert that the protection it observes is DERIVED
/// and not declared.
fn channels_yaml(tmp: &TempDir) -> String {
    std::fs::read_to_string(tmp.path().join(".nxs-personas").join("channels.yaml")).unwrap()
}

/// **The pm answers the human, which is what ENDS THE OPERATION** (nxf 6j6v.8y6t).
///
/// Since the claim area is the operation rather than the subtree under `#coding`, a round finishing
/// is no longer the last thing that has to happen: T1 — the conversation the human opened — is
/// inside the area, and the pm still owes an answer there while it decides whether to commission a
/// second round. That reply is the release, and every test below that used to end at "the round
/// finished, so the rival starts" now walks the one step that remains.
fn the_pm_answers_the_human(tmp: &TempDir, c: &Chain) {
    json_of(persona(tmp, "pm", &c.pm).args(["--json", "reply", "--thread", &c.t1, "shipped"]));
}

/// a71h §2's chain, driven as far as "the `#review` round is fanned out and nobody has answered".
struct Chain {
    t1: String,
    t2: String,
    t3_lead: String,
    t3_coder: String,
    t4: String,
    t5: Vec<(&'static str, String, String)>,
    pm: String,
    coder: String,
    coding_lead: String,
}

fn drive_chain_to_the_review_round(tmp: &TempDir) -> Chain {
    // Nutzer -> send --to pm -> T1
    let opened = json_of(human(tmp).args(["--json", "send", "--to", "pm", "ship the release"]));
    let t1 = opened["thread_id"].as_str().unwrap().to_string();
    let pm = opened["session"].as_str().unwrap().to_string();

    // pm -> send --to #coding -> T2, and the supervisor opens one thread per member below it.
    let t2 =
        json_of(persona(tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();
    let t3_lead = member_thread_of(tmp, &t2, "coding-lead");
    let t3_coder = member_thread_of(tmp, &t2, "coder");
    let coding_lead = dry_session(tmp, "coding-lead");
    let coder = dry_session(tmp, "coder");

    // coding-lead -> send --to #review -> T4 (hangs under T3lead: the thread it is SENT FROM)
    let t4 = json_of(persona(tmp, "coding-lead", &coding_lead).args([
        "--json",
        "send",
        "--to",
        "review",
        "review the first cut",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let t5 = ["reviewer-a", "reviewer-b"]
        .into_iter()
        .map(|who| {
            let thread = member_thread_of(tmp, &t4, who);
            (who, thread, dry_session(tmp, who))
        })
        .collect();

    Chain {
        t1,
        t2,
        t3_lead,
        t3_coder,
        t4,
        t5,
        pm,
        coder,
        coding_lead,
    }
}

/// The rival order: a second, unrelated chain that also needs the working copy.
fn order_a_rival(tmp: &TempDir) -> String {
    json_of(human(tmp).args(["--json", "send", "--to", "rival", "the next job"]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string()
}

// ---- 1: WHETHER — derived upward from the declared members, ONE hop --------------------------

#[test]
fn a_channel_is_protected_because_a_declared_member_needs_it_and_review_does_not_inherit_that() {
    // Acceptance point 1. `#coding` says nothing about the working copy; `coder`, one of its
    // declared members, says `working_tree: exclusive`. That is enough, and it is enough for the
    // WHOLE channel: `coding-lead`, which declares nothing, is part of a protected chain because of
    // what it is part of.
    //
    // The observable difference is what happens when somebody else is holding. Without the upward
    // derivation the channel's own declaration is silent, so only the member that declares it for
    // itself asks for the lease — and a board opens HALF queued and half running, its two members
    // on opposite sides of a lease that is supposed to cover the chain.
    //
    // And it stops after one hop. `#review`'s members only READ, so `#review` is shared — even
    // though it shares the member `coding-lead` with the now-protected `#coding`. A transitive
    // derivation (a member of an exclusive channel counts as exclusive) would carry the protection
    // across that shared member and end with every channel exclusive, which is exactly what the
    // owner's "EIN SPRUNG" excludes.
    let tmp = workspace();
    write_declarations(&tmp, "");
    assert!(
        !channels_yaml(&tmp).contains("working_tree"),
        "neither channel declares it — whatever protection shows up below is DERIVED"
    );

    // Somebody unrelated takes the working copy first.
    let rival = order_a_rival(&tmp);
    assert_eq!(board_state(&tmp, "carsten", NOW, &rival), "holding");

    // `#coding` opens behind it, and BOTH members wait — the one that declares the need and the one
    // that does not.
    let coding = json_of(human(&tmp).args(["--json", "send", "--to", "coding", "build it"]));
    let coding_thread = coding["thread_id"].as_str().unwrap().to_string();
    for who in ["coder", "coding-lead"] {
        assert!(
            !started(&tmp, who),
            "{who} belongs to a protected channel, so it waits with the rest of it:\n{}",
            trigger_lines(&tmp).join("\n")
        );
        let member = member_thread_of(&tmp, &coding_thread, who);
        assert_eq!(board_state(&tmp, "carsten", NOW, &member), "waiting");
    }
    // Where that fact is NOT visible, recorded rather than asserted away: a channel send's own
    // receipt reports `spawned: true` with no queue fields even when its whole fan-out is parked,
    // because `surface::send_to`'s channel branch does not relay the members' admissions. That is
    // pre-existing (it is reachable on any `working_tree: exclusive` channel opening behind a
    // holder) and it belongs to the receipt's own visibility, not to this item — the board above is
    // where the requester finds out.
    assert_eq!(coding["spawned"].as_bool(), Some(true), "{coding}");

    // `#review` opens behind the same holder and does NOT wait: it inherits nothing.
    let review = json_of(human(&tmp).args(["--json", "send", "--to", "review", "read this"]));
    let review_thread = review["thread_id"].as_str().unwrap().to_string();
    assert_eq!(review["spawned"].as_bool(), Some(true), "{review}");
    assert!(
        review.get("queue_position").is_none(),
        "a reading channel does not queue behind a working copy it never needed: {review}"
    );
    for who in ["reviewer-a", "reviewer-b"] {
        assert!(started(&tmp, who), "{who} ran straight away");
        let member = member_thread_of(&tmp, &review_thread, who);
        assert_eq!(
            board_state(&tmp, "carsten", NOW, &member),
            Value::Null,
            "#review does not inherit #coding's need through the member they share"
        );
    }
    assert_eq!(
        board_state(&tmp, "carsten", NOW, &review_thread),
        Value::Null
    );
}

// ---- 2: HOW FAR — the OPERATION, root included (nxf 6j6v.8y6t) -------------------------------

#[test]
fn the_claim_area_is_the_whole_operation_and_the_thread_the_human_opened_is_in_it() {
    // **This test used to assert the opposite of its second half, and the flip is nxf 6j6v.8y6t.**
    // Under 6j6v.1xw1 the area was the subtree from the CLAIM THREAD — `#coding`'s own channel
    // thread — and T1, the conversation with the human, was deliberately outside it so that one
    // unanswered human could not hold this device's working copy until tomorrow.
    //
    // Two costs of that boundary were measured on 2026-08-22 and are what the owner decided (a) on:
    // the claim fell as soon as ONE round was worked off, so a second coding round of the same epic
    // had a window in front of it that a foreign chain could step into; and an operation whose
    // exclusive step comes late could be overtaken by one that started later.
    //
    // So the anchor is the OPERATION'S ROOT, and T1 is IN the area. The risk 1xw1 named is answered
    // by the other half of the same item rather than dropped: the lease's bound is now the
    // operation's own declared windows instead of a flat two hours, and the claim still arises only
    // at the first exclusive step — see `an_operation_that_declares_its_windows_bounds_its_own_lease`
    // and `a_chain_with_no_exclusive_persona_anywhere_claims_nothing` below.
    let tmp = workspace();
    write_declarations(&tmp, "");
    let c = drive_chain_to_the_review_round(&tmp);

    let store = open_store(&tmp);
    assert_eq!(
        store.working_tree_holder(NOW).unwrap(),
        Some(WorkScope::Thread(c.t1.clone()).key()),
        "the claim is taken under the operation's root, not under the channel that declared the need"
    );
    let mut area = store
        .work_scope_threads(&WorkScope::Thread(c.t1.clone()))
        .unwrap();
    area.sort();
    // Paired with a handle that may read each thread's board, since `threads show` enforces the
    // channel's own visibility — the reader never changes the answer, only whether it can be asked.
    let mut in_the_area = vec![
        (c.t1.clone(), "carsten"),
        (c.t2.clone(), "pm"),
        (c.t3_lead.clone(), "pm"),
        (c.t3_coder.clone(), "pm"),
        (c.t4.clone(), "coding-lead"),
    ];
    in_the_area.extend(c.t5.iter().map(|(_, t, _)| (t.clone(), "coding-lead")));
    let mut want: Vec<String> = in_the_area.iter().map(|(t, _)| t.clone()).collect();
    want.sort();
    assert_eq!(
        area, want,
        "the claim area is the whole operation — T1, the #coding board, both its members, the \
         #review round below one of them, and that round's members"
    );

    // The same fact as a reader of the board meets it.
    for (thread, reader) in &in_the_area {
        assert_eq!(
            board_state(&tmp, reader, NOW, thread),
            "holding",
            "every thread of the operation reads as holding: {thread}"
        );
    }
}

// ---- 3: the protection holds while #coding waits on #review ----------------------------------

#[test]
fn the_claim_holds_while_coding_waits_on_the_review_round_below_it() {
    // Acceptance point 3. T4 and the T5* lie UNDER T2, so the gap between "the coder is done" and
    // "the review is in" is inside the claim area rather than a hole in it.
    let tmp = workspace();
    write_declarations(&tmp, "");
    let c = drive_chain_to_the_review_round(&tmp);
    let rival = order_a_rival(&tmp);
    assert!(
        !started(&tmp, "rival"),
        "the rival waits on chain one from the start"
    );
    assert_eq!(board_state(&tmp, "carsten", NOW, &rival), "waiting");

    // The coder finishes its own member thread. #coding still owes coding-lead's answer, and
    // coding-lead is waiting on #review.
    json_of(persona(&tmp, "coder", &c.coder).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_coder,
        "implemented",
    ]));
    assert!(
        !started(&tmp, "rival"),
        "one of two members answering releases nothing"
    );
    assert_eq!(board_state(&tmp, "pm", NOW, &c.t2), "holding");

    // The review round answers. Its consolidation discharges T4 and resumes coding-lead — which
    // still owes T3lead, so the claim area still owes something and the working copy stays put.
    for (who, thread, session) in &c.t5 {
        json_of(persona(&tmp, who, session).args([
            "--json",
            "reply",
            "--thread",
            thread,
            "looks good to me",
        ]));
    }
    assert!(
        !started(&tmp, "rival"),
        "#coding is still mid-flight: the reviewers answered, its own member has not"
    );
    assert_eq!(board_state(&tmp, "pm", NOW, &c.t2), "holding");
    assert_eq!(board_state(&tmp, "coding-lead", NOW, &c.t4), "holding");
}

// ---- 3b: work commissioned from INSIDE the held area inherits it (nxf 6j6v.43kw) -------------

/// The self-blocking claim, on the plainest surface it has: `send --to <persona>` for a role that
/// declares `working_tree: exclusive`, issued from inside a chain that is already holding.
///
/// Before this fix the nested trigger resolved to its OWN fresh thread as its claim key, failed the
/// exact-match compare-and-swap, and queued **behind its own parent**. The parent could then never
/// release — what it was waiting for was the trigger it had just parked — so nothing started, no
/// error was reported, and the queue would not drain even past the two-hour backstop.
///
/// The evidence is what the runtime produced: whether `helper` was handed to the worker at all, and
/// what the board says the chain is doing. The rival is ordered alongside so the test also shows the
/// fix did not simply hand the working copy to everyone — it is still waiting at the end.
#[test]
fn an_exclusive_persona_commissioned_from_inside_the_held_area_starts_instead_of_queuing() {
    let tmp = workspace();
    write_declarations(&tmp, "");
    let c = drive_chain_to_the_review_round(&tmp);
    let rival = order_a_rival(&tmp);
    assert!(!started(&tmp, "rival"), "the rival waits on chain one");

    // The coder, standing in its own member thread inside the #coding claim area, commissions a
    // second exclusive persona — a helper on the same working copy, which is what a chain does.
    let receipt = json_of(persona(&tmp, "coder", &c.coder).args([
        "--json",
        "send",
        "--to",
        "helper",
        "take the second half",
    ]));
    assert_eq!(
        receipt["queue_position"],
        Value::Null,
        "not queued: this is the holder's own work, so it inherits the claim — receipt {receipt}"
    );
    assert_eq!(receipt["queued_behind"], Value::Null);
    assert_eq!(receipt["spawned"], Value::Bool(true));
    assert!(
        started(&tmp, "helper"),
        "and the worker was actually handed it, rather than the chain deadlocking in silence"
    );

    // The lease did not move: it is still the #coding claim thread that holds, and the helper's
    // thread is inside that area rather than a claim of its own.
    let helper_thread = receipt["thread_id"].as_str().unwrap().to_string();
    let store = open_store(&tmp);
    assert_eq!(
        store.working_tree_holder(NOW).unwrap(),
        Some(WorkScope::Thread(c.t1.clone()).key()),
        "inheriting is not a hand-over — the OPERATION's own root keeps the lease (nxf 6j6v.8y6t)"
    );
    assert!(
        store
            .work_scope_threads(&WorkScope::Thread(c.t2.clone()))
            .unwrap()
            .contains(&helper_thread),
        "and the thread it inherited into is part of that area, so the reply that ends it releases"
    );

    // The rival is still outside, which is the half a too-eager fix would break.
    assert!(
        !started(&tmp, "rival"),
        "a rival chain still waits its turn"
    );
    assert_eq!(board_state(&tmp, "carsten", NOW, &rival), "waiting");
}

/// The same rule for the shape the item was first written about: a nested exclusive CHANNEL. It is
/// worth its own case because it takes a different route to the same key — the fan-out opens a
/// channel thread and one member thread per member, and `lease_claim_thread` resolves each member UP
/// to the channel thread, which is itself inside the held area.
#[test]
fn a_nested_exclusive_channel_inherits_the_area_it_was_opened_from() {
    let tmp = workspace();
    write_declarations(&tmp, "");
    let c = drive_chain_to_the_review_round(&tmp);
    let rival = order_a_rival(&tmp);

    // `#rework` is protected the same way `#coding` is: its member `helper` declares it, nothing on
    // the channel itself does (acceptance point 1's derivation).
    assert!(
        !channels_yaml(&tmp).contains("working_tree"),
        "the protection is derived from the members, not written on the channel"
    );
    let receipt = json_of(persona(&tmp, "coding-lead", &c.coding_lead).args([
        "--json",
        "send",
        "--to",
        "rework",
        "second pass on the same tree",
    ]));
    assert_eq!(
        receipt["queue_position"],
        Value::Null,
        "the nested board opens instead of parking behind the chain that opened it — {receipt}"
    );
    assert!(started(&tmp, "helper"), "its exclusive member was started");

    let store = open_store(&tmp);
    assert_eq!(
        store.working_tree_holder(NOW).unwrap(),
        Some(WorkScope::Thread(c.t1.clone()).key()),
        "still one claim over the whole operation, not a second one nested inside it"
    );
    assert!(!started(&tmp, "rival"));
    assert_eq!(board_state(&tmp, "carsten", NOW, &rival), "waiting");
}

// ---- 4: a plain reply closes the claim thread, and the rival starts only after that -----------

#[test]
fn a_reply_without_escalate_on_the_claim_thread_releases_it_and_the_rival_starts_only_after_that() {
    // Acceptance point 4, with the rival actually racing rather than assumed: it is ordered while
    // chain one is mid-flight and its start is checked at EVERY earlier step, so "only after that"
    // is a sequence this test walks and not a single end-state.
    let tmp = workspace();
    write_declarations(&tmp, "");
    let c = drive_chain_to_the_review_round(&tmp);
    let rival = order_a_rival(&tmp);

    json_of(persona(&tmp, "coder", &c.coder).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_coder,
        "implemented",
    ]));
    assert!(
        !started(&tmp, "rival"),
        "the coder finished and the claim thread is not closed, so the rival has not started"
    );

    for (who, thread, session) in &c.t5 {
        json_of(persona(&tmp, who, session).args([
            "--json",
            "reply",
            "--thread",
            thread,
            "looks good to me",
        ]));
        assert_eq!(
            board_state(&tmp, "coding-lead", NOW, thread),
            "holding",
            "{who} answered INSIDE the claim area — the release path weighed that reply and refused"
        );
        assert!(
            !started(&tmp, "rival"),
            "{who} answered inside the claim area, which closes nothing at the claim thread"
        );
    }

    // The last member of #coding answers, WITHOUT `--escalate`. Its set settles and the supervisor
    // discharges the channel thread — which used to be the release.
    json_of(persona(&tmp, "coding-lead", &c.coding_lead).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_lead,
        "reviewed and wrapped up",
    ]));

    // **And since nxf 6j6v.8y6t it is NOT the release, which is this item's whole point.** The
    // OPERATION is not over: the pm has been resumed with the round's answer and still owes T1, so
    // it is at exactly the moment where it might commission a second coding round. Under the old
    // anchor the checkout was free right here, and that gap is the window a foreign chain could take
    // it in — measured, and the reason the anchor moved.
    assert!(
        !started(&tmp, "rival"),
        "the round finished but the operation did not: the pm still owes the human — triggers so \
         far:\n{}",
        trigger_lines(&tmp).join("\n")
    );
    assert_eq!(board_state(&tmp, "pm", NOW, &c.t2), "holding");
    assert_eq!(board_state(&tmp, "carsten", NOW, &rival), "waiting");

    // The pm answers the human. THAT ends the operation, and that is the release.
    the_pm_answers_the_human(&tmp, &c);

    assert!(
        started(&tmp, "rival"),
        "the reply that ended the operation released the working copy and started the chain \
         waiting for it — triggers so far:\n{}",
        trigger_lines(&tmp).join("\n")
    );
    assert_eq!(board_state(&tmp, "carsten", NOW, &rival), "holding");
    assert_eq!(board_state(&tmp, "pm", NOW, &c.t2), Value::Null);
}

// ---- 5: an escalation does not release, not even after a long silence -------------------------

#[test]
fn an_escalation_that_reaches_the_human_does_not_release_not_even_after_a_long_wait() {
    // Acceptance point 5. `--escalate` discharges the sender's TURN (6j6v.cg8g) exactly like a
    // finished reply, so after this nothing anywhere is outstanding — the old rule would release
    // right here. The CLAIM is a different question: the task is mid-flight, the question is
    // travelling upward, and the working copy stays protected.
    //
    // Driven all the way to T1: the escalation reaches the human, and the human says nothing.
    let tmp = workspace();
    write_declarations(&tmp, "");
    let c = drive_chain_to_the_review_round(&tmp);
    let rival = order_a_rival(&tmp);

    json_of(persona(&tmp, "coder", &c.coder).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_coder,
        "implemented",
    ]));
    for (who, thread, session) in &c.t5 {
        json_of(persona(&tmp, who, session).args([
            "--json",
            "reply",
            "--thread",
            thread,
            "looks good to me",
        ]));
    }
    json_of(persona(&tmp, "coding-lead", &c.coding_lead).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_lead,
        "--escalate",
        "I cannot: the release credentials are missing",
    ]));

    // The question travels one level at a time. The pm was woken by that consolidation and hands it
    // on to the human, who is the only party that can answer it.
    json_of(persona(&tmp, "pm", &c.pm).args([
        "--json",
        "reply",
        "--thread",
        &c.t1,
        "--escalate",
        "we are blocked on the release credentials",
    ]));

    let store = open_store(&tmp);
    assert!(
        !store
            .work_scope_has_outstanding(&WorkScope::Thread(c.t2.clone()), NOW)
            .unwrap(),
        "nothing in the claim area owes anybody a reply any more — the quorum rule alone would \
         release here, which is exactly why the release rule is 'the claim thread is CLOSED'"
    );
    assert!(
        !started(&tmp, "rival"),
        "an escalating reply does not close the claim thread — triggers so far:\n{}",
        trigger_lines(&tmp).join("\n")
    );
    assert_eq!(board_state(&tmp, "pm", NOW, &c.t2), "holding");
    assert_eq!(board_state(&tmp, "carsten", NOW, &rival), "waiting");

    // …and not even when the human does not answer for a long time. This instant is still INSIDE the
    // two-hour backstop, so the hold is the rule's doing and not the clock's.
    assert_eq!(board_state(&tmp, "pm", A_LONG_WAIT_LATER, &c.t2), "holding");

    // **And the long wait is made LOAD-BEARING rather than observational** (branch review, Minor 5).
    // Reading the board and re-asserting that the first rival is still parked cannot fail at a later
    // instant, because nothing attempts an acquire at that instant. So a SECOND order is issued AT
    // `A_LONG_WAIT_LATER` — a real compare-and-swap against the escalating holder, almost two hours
    // after it took the lease — and it has to lose.
    let late = json_of(at(&tmp, A_LONG_WAIT_LATER).args([
        "--json",
        "send",
        "--to",
        "late-rival",
        "a third job, hours later",
    ]));
    assert_eq!(
        late["queued_behind"].as_str(),
        Some(format!("thread:{}", c.t1).as_str()),
        "the acquire at the late instant was refused BY the escalating chain: {late}"
    );
    assert_eq!(late["spawned"].as_bool(), Some(false), "{late}");
    assert!(
        !started(&tmp, "late-rival"),
        "and nothing is running for it"
    );
    assert!(
        !started(&tmp, "rival"),
        "nor for the one that was already waiting"
    );

    // The benign error direction, and its bound. Whoever escalates wrongly holds the copy too long
    // — harmless next to a working copy released while the task is mid-flight — and "too long" ends
    // at the hard-death backstop rather than never.
    assert_eq!(
        board_state(&tmp, "pm", PAST_THE_BOUND, &c.t2),
        Value::Null,
        "holding too long is bounded by WORKING_TREE_LEASE_BOUND, not unbounded"
    );
}

// ---- the owner's ruling: an open QUESTION holds the claim exactly like an escalation ----------

#[test]
fn an_open_question_holds_the_claim_exactly_as_an_escalation_does() {
    // `MessageKind::Question` is deliberately NOT an escalation (6j6v.wt37: "what do you mean by
    // X?" is a different thing from "I cannot"), and it discharges the turn just as an escalation
    // does — so `last_reply_escalated` answers `Some(false)` for it and a member waiting on an
    // answer is indistinguishable from one that is finished.
    //
    // For the LEASE the difference is no difference: in both shapes nobody is working and the task
    // is mid-flight, so releasing lets a rival start into a copy the questioner will resume into.
    // The claim is held, and it is held from the MEMBER thread — a question, unlike an escalation,
    // is not something the channel's consolidator carries upward, so reading the claim thread alone
    // would miss it.
    let tmp = workspace();
    write_declarations(&tmp, "");
    let c = drive_chain_to_the_review_round(&tmp);
    let rival = order_a_rival(&tmp);

    json_of(persona(&tmp, "coder", &c.coder).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_coder,
        "implemented",
    ]));
    for (who, thread, session) in &c.t5 {
        json_of(persona(&tmp, who, session).args([
            "--json",
            "reply",
            "--thread",
            thread,
            "looks good to me",
        ]));
    }
    reply_with_a_question(
        &tmp,
        "coding-lead",
        &c.coding_lead,
        &c.t3_lead,
        "which of the two release channels do you mean?",
    );

    let store = open_store(&tmp);
    assert_eq!(
        store.last_reply_escalated(&c.t3_lead).unwrap(),
        Some(false),
        "a question is NOT an escalation, and the derivation says so — which is why the claim rule \
         cannot be built on that answer alone"
    );
    assert!(
        !store
            .work_scope_has_outstanding(&WorkScope::Thread(c.t2.clone()), NOW)
            .unwrap(),
        "and it discharged the turn, so nothing is outstanding either"
    );
    assert!(
        !started(&tmp, "rival"),
        "an unanswered question holds the claim exactly like an escalation — triggers so \
         far:\n{}",
        trigger_lines(&tmp).join("\n")
    );
    assert_eq!(board_state(&tmp, "pm", NOW, &c.t2), "holding");
    assert_eq!(
        board_state(&tmp, "carsten", A_LONG_WAIT_LATER, &rival),
        "waiting"
    );
}

// ---- the hazard wt37 disclosed: an escalation on a `summarize` channel ------------------------

#[test]
fn a_summarize_channel_whose_member_escalates_does_not_release_the_claim_thread() {
    // The shape task 4 disclosed rather than left to be found: on a channel that SUMMARISES, the
    // channel thread's answer of record used to be the synthesizer's own reply, whose kind the
    // engine does not author — so a member's escalation died one level down and the claim thread
    // read as finished while a member was stuck.
    //
    // 6j6v.e9qj closed that at the source (a declared consolidator never folds an escalating set),
    // and this pins the consequence for the LEASE, which is the thing that would have gone wrong.
    let tmp = workspace();
    write_declarations(
        &tmp,
        "  on_complete: summarize\n  summary_prompt: fold the answers\n",
    );
    let c = drive_chain_to_the_review_round(&tmp);
    let rival = order_a_rival(&tmp);

    for (who, thread, session) in &c.t5 {
        json_of(persona(&tmp, who, session).args([
            "--json",
            "reply",
            "--thread",
            thread,
            "looks good to me",
        ]));
    }
    json_of(persona(&tmp, "coder", &c.coder).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_coder,
        "implemented",
    ]));
    json_of(persona(&tmp, "coding-lead", &c.coding_lead).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_lead,
        "--escalate",
        "I cannot: the release credentials are missing",
    ]));

    assert!(
        !started(&tmp, "__synth__"),
        "the escalating set was passed through, not folded — no synthesizer ran:\n{}",
        trigger_lines(&tmp).join("\n")
    );
    assert!(
        !started(&tmp, "rival"),
        "and the claim thread of a SUMMARISING channel is not released under a member that could \
         not deliver:\n{}",
        trigger_lines(&tmp).join("\n")
    );
    assert_eq!(board_state(&tmp, "pm", NOW, &c.t2), "holding");
    assert_eq!(
        board_state(&tmp, "carsten", A_LONG_WAIT_LATER, &rival),
        "waiting"
    );
}

// ---- fix round 1: the counter-shape that killed the one-level narrowing --------------------------

#[test]
fn a_question_two_levels_down_holds_the_claim_although_every_thread_above_it_consolidated() {
    // THE COUNTER-SHAPE (review of this item, Critical 1). My first cut asked only the claim thread
    // and its supervised children, on the argument that a hand-back deeper down is already covered:
    // *"if the level ABOVE it has not acted, that level's own thread is OUTSTANDING; if it HAS
    // acted, the hand-back is answered."*
    //
    // The second horn is FALSE, and this is the shape that shows it. The level above can act by
    // CONSOLIDATING THE SET rather than by answering, and a question is exactly the kind that lets
    // it: a question discharges the turn (6j6v.cg8g), so the set settles; it is not an escalation,
    // so the round folds or passes through normally; and the questioner is never resumed. Every
    // thread from T5a up to T2 ends discharged, nothing is outstanding anywhere, and the one level
    // the old read could reach carries nothing but ordinary results.
    //
    // The difference from the test above is ONE argument: the question is asked on T5a by a
    // reviewer instead of on T3_lead by the coding lead.
    let tmp = workspace();
    write_declarations(&tmp, "");
    let c = drive_chain_to_the_review_round(&tmp);
    let rival = order_a_rival(&tmp);

    let (question_who, question_thread, question_session) = c.t5[0].clone();
    reply_with_a_question(
        &tmp,
        question_who,
        &question_session,
        &question_thread,
        "which of the two release branches am I reviewing?",
    );
    // …and everything ABOVE it finishes normally, which is the whole point.
    let (who_b, thread_b, session_b) = c.t5[1].clone();
    json_of(persona(&tmp, who_b, &session_b).args([
        "--json",
        "reply",
        "--thread",
        &thread_b,
        "looks good to me",
    ]));
    json_of(persona(&tmp, "coder", &c.coder).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_coder,
        "implemented",
    ]));
    json_of(persona(&tmp, "coding-lead", &c.coding_lead).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_lead,
        "reviewed and wrapped up",
    ]));

    let store = open_store(&tmp);
    assert!(
        !store
            .work_scope_has_outstanding(&WorkScope::Thread(c.t2.clone()), NOW)
            .unwrap(),
        "every thread in the claim area is discharged — the quorum rule alone would release"
    );
    assert_eq!(
        store.last_reply_kind(&c.t2).unwrap().as_deref(),
        Some("info"),
        "and the claim thread's own discharge is an ordinary result: the question did not travel up"
    );
    assert_eq!(
        trigger_count(&tmp, question_who),
        1,
        "the questioner was never resumed — it was started once and is still waiting for its answer"
    );

    // The claim is held anyway, because the SUBTREE is asked.
    assert!(
        !started(&tmp, "rival"),
        "a question two levels down holds the claim: releasing here would start a rival into a \
         working copy {question_who} is waiting to resume into — triggers so far:\n{}",
        trigger_lines(&tmp).join("\n")
    );
    assert_eq!(board_state(&tmp, "pm", NOW, &c.t2), "holding");
    assert_eq!(board_state(&tmp, "carsten", NOW, &rival), "waiting");
}

#[test]
fn an_answered_question_deep_in_the_subtree_clears_the_hold_before_the_bound() {
    // The other half of Critical 1, and the one the ruling asked to be answered with evidence rather
    // than argument: does a subtree-wide read CLEAR itself once the hand-back is answered, or does
    // it wedge the working copy until the two-hour backstop?
    //
    // It clears, and the mechanism is the turn watermark. Answering a member's question means
    // re-commissioning that round — a reply into the channel thread from its requester, which is the
    // same door the first `send --to` used (6j6v.pf6j). That RE-DECLARES every member's expectation,
    // which moves `expects_reply_from_v`, so `last_reply_kind` on the questioner's thread is `None`
    // again ("nothing has answered this turn") and stays that way until it answers for real.
    //
    // So the read is self-clearing on any chain whose hand-back is actually answered, and holds only
    // where nobody answered it — which is exactly the discrimination the claim needs.
    let tmp = workspace();
    write_declarations(&tmp, "");
    let c = drive_chain_to_the_review_round(&tmp);
    let rival = order_a_rival(&tmp);

    let (question_who, question_thread, question_session) = c.t5[0].clone();
    let (who_b, thread_b, session_b) = c.t5[1].clone();
    reply_with_a_question(
        &tmp,
        question_who,
        &question_session,
        &question_thread,
        "which of the two release branches am I reviewing?",
    );
    json_of(persona(&tmp, who_b, &session_b).args([
        "--json",
        "reply",
        "--thread",
        &thread_b,
        "looks good to me",
    ]));
    json_of(persona(&tmp, "coder", &c.coder).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_coder,
        "implemented",
    ]));
    assert!(
        !started(&tmp, "rival"),
        "precondition: the claim is still held — at THIS instant by the outstanding rule, because \
         coding-lead has not answered T3_lead yet. What the hand-back rule holds on its own is the \
         test above; what this test is about starts on the next line"
    );

    // THE ANSWER. `coding-lead` opened the review round, so a reply from it into the round's own
    // channel thread is what hands out the next turn — and that is what answers the question.
    json_of(persona(&tmp, "coding-lead", &c.coding_lead).args([
        "--json",
        "reply",
        "--thread",
        &c.t4,
        "the release branch is release/2.0 — carry on",
    ]));
    let store = open_store(&tmp);
    assert_eq!(
        store.last_reply_kind(&question_thread).unwrap(),
        None,
        "the re-declaration moved the watermark: the question is no longer THIS turn's answer"
    );
    assert!(
        trigger_count(&tmp, question_who) >= 2,
        "and the questioner was actually put back to work: {:?}",
        trigger_lines(&tmp)
    );

    // The round answers again, and the chain finishes normally.
    for (who, thread, session) in [
        (
            question_who,
            question_thread.as_str(),
            question_session.as_str(),
        ),
        (who_b, thread_b.as_str(), session_b.as_str()),
    ] {
        json_of(persona(&tmp, who, session).args([
            "--json",
            "reply",
            "--thread",
            thread,
            "release/2.0 looks good",
        ]));
    }
    json_of(persona(&tmp, "coding-lead", &c.coding_lead).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_lead,
        "reviewed and wrapped up",
    ]));

    the_pm_answers_the_human(&tmp, &c);
    assert!(
        started(&tmp, "rival"),
        "an ANSWERED question holds nothing: the claim was released at NOW, far inside the \
         backstop — triggers:\n{}",
        trigger_lines(&tmp).join("\n")
    );
    assert_eq!(board_state(&tmp, "carsten", NOW, &rival), "holding");
    assert_eq!(board_state(&tmp, "pm", NOW, &c.t2), Value::Null);
}

#[test]
fn an_answered_escalation_releases_the_claim_before_the_two_hour_bound() {
    // Minor 7: the changelog says the working copy "stays protected until it is answered", and
    // nothing in the first round showed an ANSWER turning into a release — only the backstop did.
    // This is that path, at the claim thread itself: the requester of the protected channel replies
    // into it, which re-declares every member's turn (6j6v.pf6j), and the members answer again.
    //
    // The same watermark that clears a member's question clears the channel thread's own escalation,
    // so the promise is now a test rather than a sentence.
    let tmp = workspace();
    write_declarations(&tmp, "");
    let c = drive_chain_to_the_review_round(&tmp);
    let rival = order_a_rival(&tmp);

    for (who, thread, session) in &c.t5 {
        json_of(persona(&tmp, who, session).args([
            "--json",
            "reply",
            "--thread",
            thread,
            "looks good to me",
        ]));
    }
    json_of(persona(&tmp, "coder", &c.coder).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_coder,
        "implemented",
    ]));
    json_of(persona(&tmp, "coding-lead", &c.coding_lead).args([
        "--json",
        "reply",
        "--thread",
        &c.t3_lead,
        "--escalate",
        "I cannot: the release credentials are missing",
    ]));
    assert!(
        !started(&tmp, "rival"),
        "precondition: the escalation is holding the claim"
    );
    let store = open_store(&tmp);
    assert_eq!(
        store.last_reply_kind(&c.t2).unwrap().as_deref(),
        Some("escalation"),
        "…and it is holding it AT the claim thread, which is what is about to be answered"
    );
    drop(store);

    // THE ANSWER, from the party that commissioned the channel.
    json_of(persona(&tmp, "pm", &c.pm).args([
        "--json",
        "reply",
        "--thread",
        &c.t2,
        "here are the credentials — carry on",
    ]));
    let store = open_store(&tmp);
    assert_eq!(
        store.last_reply_kind(&c.t2).unwrap(),
        None,
        "the re-declaration moved the watermark: the escalation is not THIS turn's answer"
    );
    drop(store);

    for (who, thread) in [("coder", &c.t3_coder), ("coding-lead", &c.t3_lead)] {
        let session = if who == "coder" {
            &c.coder
        } else {
            &c.coding_lead
        };
        json_of(persona(&tmp, who, session).args([
            "--json",
            "reply",
            "--thread",
            thread,
            "done with the credentials in hand",
        ]));
    }

    the_pm_answers_the_human(&tmp, &c);
    assert!(
        started(&tmp, "rival"),
        "an answered escalation releases, and it releases at NOW rather than at the \
         backstop — triggers:\n{}",
        trigger_lines(&tmp).join("\n")
    );
    assert_eq!(board_state(&tmp, "carsten", NOW, &rival), "holding");
}
