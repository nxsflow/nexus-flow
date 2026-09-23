//! **A channel declares its FLOW** (nxf 6j6v.hq71, DoD point 1) — the second cut's own build.
//!
//! nxf 6j6v.pf6j made the supervisor the INCOMING router, which is what the draft's §3.2 says a
//! fixed order needs ("nur ein eingehender Betreuer kann eine feste Reihenfolge herstellen"). What
//! was still missing was a way for an author to DECLARE that order. `flow: sequential` is it, and
//! the order it binds is the order of `members` — there is no second list, because "es gibt keinen
//! zweiten Begriff dafuer".
//!
//! Everything here is driven through the real `nxc` binary against the dry worker; nothing
//! constructs a thread, a parent edge or an expectation by hand.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-08-16T10:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

/// The human at the keyboard: no spawned-context stamps, so it stands in no thread.
///
/// Deliberately NOT `NXF_DETERMINISTIC_IDS` — that counter is shared by messages, threads and
/// sessions, so in a chain this long a thread id collides with an unrelated MESSAGE id and
/// `reply --thread <id>` lands in the wrong thread (nxf 6j6v.k8bw).
fn human(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"))
        // **The dry timer on EVERY call, not only where scheduling is asserted** — the rule
        // `channel_member_timeout.rs` states at its own harness, adopted here when nxf 6j6v.rs9k
        // gave this suite a channel with a declared `timeout`. An unset `NXC_TIMER` means the REAL
        // `at`, so a test that opens a windowed channel would leave one-shot jobs on the machine it
        // ran on.
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

/// [`human`] on a different wall clock — the only way to drive a member past its window.
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

/// Every dry-log line that opens a trigger entry, in the order the engine started them. The message
/// field embeds real newlines, so only the FIRST physical line of an entry starts with `trigger
/// role=`.
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

/// The threads the supervisor opened below `parent`, **in `thread_id` order — which is NOT reliably
/// the order they were opened in**. `supervised_children` sorts by id, ids are ULIDs, and a ULID
/// sorts by mint time only to the millisecond; a parallel fan-out mints its whole set inside one, so
/// its children can come back in any order at all.
///
/// The `next_back()` uses below are safe in spite of that, and only for a reason worth stating: they
/// are all on ORDERED channels, whose slots are minted one per step in separate `nxc` processes
/// milliseconds apart, so "the newest id" really is "the one just opened" there. Do not copy them
/// onto a parallel channel.
fn slots(tmp: &TempDir, parent: &str) -> Vec<String> {
    open_store(tmp).supervised_children(parent).unwrap()
}

/// The one thread the supervisor opened below `parent` for `handle`.
fn slot_for(tmp: &TempDir, parent: &str, handle: &str) -> String {
    let store = open_store(tmp);
    let want = format!("local/{handle}");
    store
        .supervised_children(parent)
        .unwrap()
        .into_iter()
        .find(|t| {
            store
                .thread_quorum(t, NOW)
                .unwrap()
                .is_some_and(|q| q.expects == [want.clone()])
        })
        .unwrap_or_else(|| panic!("no slot for {handle} under {parent}"))
}

// ---- DoD point 1: the declared order binds ----------------------------------------------------

const ORDERED: &str = "- name: coding\n  members: [coder, reviewer]\n  flow: sequential\n";
const UNORDERED: &str = "- name: coding\n  members: [coder, reviewer]\n";

#[test]
fn a_declared_flow_starts_one_step_at_a_time_in_the_declared_order() {
    // The whole of "Ein Kanal deklariert seinen Ablauf": the supervisor takes the request in and
    // opens the FIRST declared step only. The second exists nowhere — no thread, no session, no
    // obligation — until the first has settled.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "reviewer"], ORDERED);

    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();

    assert_eq!(
        started_roles(&tmp),
        vec!["pm".to_string(), "coder".to_string()],
        "step one and nothing else has been started"
    );
    assert_eq!(
        slots(&tmp, &board).len(),
        1,
        "one slot thread, for step one only"
    );

    // Step one answers. THAT is what starts step two — not a timer, not a second request.
    let coder_slot = slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &coder_slot,
        "first cut is in",
    ]));

    assert_eq!(
        started_roles(&tmp),
        vec![
            "pm".to_string(),
            "coder".to_string(),
            "reviewer".to_string()
        ],
        "step two starts only once step one has settled, and in the declared order"
    );
    let after = slots(&tmp, &board);
    assert_eq!(after.len(), 2, "one slot per step, opened in step order");
    assert_eq!(after[0], coder_slot, "{after:?}");

    // …and the channel has NOT consolidated yet: a flow ends at its last step.
    let store = open_store(&tmp);
    assert!(
        !store.thread_quorum(&board, NOW).unwrap().unwrap().complete,
        "the flow is mid-way, so the requester is still owed an answer"
    );
    drop(store);

    // Step two answers: the flow is over, so the supervisor consolidates and the requester is woken.
    let reviewer_slot = slot_for(&tmp, &board, "reviewer");
    let last = json_of(
        persona(&tmp, "reviewer", &session_of(&tmp, "reviewer")).args([
            "--json",
            "reply",
            "--thread",
            &reviewer_slot,
            "looks good",
        ]),
    );
    assert!(
        last["completed"].is_object(),
        "the last step's answer is what finishes the flow: {last}"
    );
    assert_eq!(
        last["woke"], pm,
        "…and the requester hears about it: {last}"
    );
    assert!(
        open_store(&tmp)
            .thread_quorum(&board, NOW)
            .unwrap()
            .unwrap()
            .complete,
        "the channel thread is discharged"
    );
}

#[test]
fn a_channel_that_declares_no_flow_still_starts_every_member_at_once() {
    // The default is not a hole and nothing about it moved (nxf 6j6v.pf6j's fan-out, unchanged):
    // the SAME two members, the SAME order in the file, and both are started by the opening call.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "reviewer"], UNORDERED);

    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();

    assert_eq!(
        started_roles(&tmp),
        vec![
            "pm".to_string(),
            "coder".to_string(),
            "reviewer".to_string()
        ],
        "an undeclared flow fans out to everyone in one go"
    );
    assert_eq!(
        slots(&tmp, &board).len(),
        2,
        "one thread per member, at once"
    );
}

// ---- DoD point 3: `x08d` line (b), proven BY CONSTRUCTION --------------------------------------

/// The SAME fixed order the retired `build-and-ship` workflow declared — a role step, then a
/// channel step — declared as a CHANNEL: implement, then review.
const AS_A_CHANNEL: &str = "- name: review\n  members: [reviewer-a, reviewer-b, reviewer-c]\n\
                            - name: build-and-ship\n  members: [coder, review]\n  flow: sequential\n";

const REVIEWERS: [&str; 3] = ["reviewer-a", "reviewer-b", "reviewer-c"];

/// Every reviewer answers on the thread the `review` supervisor opened for it, in declaration order.
fn all_reviewers_answer(tmp: &TempDir, review_thread: &str) {
    for who in REVIEWERS {
        let slot = slot_for(tmp, review_thread, who);
        json_of(persona(tmp, who, &session_of(tmp, who)).args([
            "--json",
            "reply",
            "--thread",
            &slot,
            "looks good to me",
        ]));
    }
}

#[test]
fn a_declared_flow_runs_the_fixed_order_that_used_to_need_a_run() {
    // **`6j6v.x08d` line (b), by construction rather than on paper.** Line (b) said the mapping "a
    // channel with a fixed order" presupposes channels-as-flows, and that until then
    // `workflow start` was a THIRD mutation verb beside `send --to`/`reply --thread`.
    //
    // **This test used to drive the order BOTH ways in one body** — once through
    // `workflow start` + `workflow step done`, once through `send --to`, asserting the two started
    // the same sessions in the same order. That differential is what made the removal in
    // 6j6v.dvyq §3 a construction proof rather than a claim, and it is why the (A) half is gone
    // rather than red: the surface it drove no longer exists. What is left is the half that
    // survived, and it asserts the same thing it always did — the declared order, and nothing else,
    // decides which sessions start and when.
    //
    // The other half of the proof now lives where every other removed entrance's does:
    // `consolidated_entrances.rs`, which shuts the door and drives the replacement.
    let b = workspace();
    write_roles(
        &b,
        &["coder", "reviewer-a", "reviewer-b", "reviewer-c"],
        AS_A_CHANNEL,
    );

    let board = json_of(human(&b).args([
        "--json",
        "send",
        "--to",
        "build-and-ship",
        "ship the release",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let coder_slot = slot_for(&b, &board, "coder");
    json_of(persona(&b, "coder", &session_of(&b, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &coder_slot,
        "implemented",
    ]));
    // Step two is a CHANNEL, so the supervisor opened that channel's own thread below this one.
    let review_board = slots(&b, &board)
        .into_iter()
        .find(|t| open_store(&b).thread_channel(t).as_deref() == Some("decl:review"))
        .expect("the second step opened the review channel's own thread");
    all_reviewers_answer(&b, &review_board);

    assert_eq!(
        started_roles(&b),
        vec![
            "coder".to_string(),
            "reviewer-a".to_string(),
            "reviewer-b".to_string(),
            "reviewer-c".to_string()
        ],
        "the declared order is the order the sessions started in: the role step, then the channel \
         step's fan-out"
    );
    let store = open_store(&b);
    assert!(
        store.thread_quorum(&board, NOW).unwrap().unwrap().complete,
        "and the flow ran to its end"
    );
    // The run record is not merely unused here — it is gone from the schema (6j6v.dvyq §3), which
    // is what makes "no second mechanism" a fact about the database rather than about this fixture.
    let table: i64 = store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='workflow_runs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(table, 0, "no `workflow_runs` table exists any more");
}

// ---- the whole chain of `6j6v.a71h` §2, driven over declared flows ----------------------------

/// §2's three channels, as DECLARATIONS rather than as a sequence somebody types by hand.
/// `#coding` runs coder and then `#review`; `#review` asks its three reviewers at once and FOLDS.
const A71H_CHAIN: &str = "- name: coding\n  members: [coder, review]\n  flow: sequential\n\
                          - name: review\n  members: [reviewer-a, reviewer-b, reviewer-c]\n  \
                          on_complete: summarize\n  summary_prompt: Fold the three reviews.\n";

#[test]
fn the_chain_from_a71h_section_two_runs_itself_from_the_declaration_alone() {
    // nxf 6j6v.a71h §2 is the chain the whole surface was designed against, and Task 2 proved the
    // TREE over it — driven by hand, with a lead persona in each channel typing the `send --to` that
    // moved the chain on. This proves the FLOW over it: nobody types the order. `#coding` receives,
    // ITS supervisor orders coder → `#review`, `#review` fans out to three reviewers and folds, and
    // the answer travels back down to the human.
    //
    // The human sends twice in the whole chain (to `pm`, and `pm` to `#coding`) and every party in
    // between only ever answers the thread it was asked on.
    let tmp = workspace();
    write_roles(
        &tmp,
        &["pm", "coder", "reviewer-a", "reviewer-b", "reviewer-c"],
        A71H_CHAIN,
    );

    // Nutzer -> send --to pm -> T1
    let opened = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship the release"]));
    let t1 = opened["thread_id"].as_str().unwrap().to_string();
    let pm = opened["session"].as_str().unwrap().to_string();

    // pm -> send --to #coding -> T2, and the supervisor opens step one: T3, the coder's own thread.
    let t2 =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();
    let t3 = slot_for(&tmp, &t2, "coder");
    assert_eq!(
        slots(&tmp, &t2).len(),
        1,
        "#review has not been asked yet — it is step two"
    );

    // coder -> reply --thread T3. The #coding supervisor advances to step two, which IS #review:
    // it opens that channel's own thread T4 below T2, and #review's supervisor fans out to T5a/b/c.
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &t3,
        "first cut is in",
    ]));
    let t4 = slots(&tmp, &t2)
        .into_iter()
        .find(|t| open_store(&tmp).thread_channel(t).as_deref() == Some("decl:review"))
        .expect("step two opened #review's own channel thread");
    let reviewer_threads: Vec<String> = REVIEWERS
        .iter()
        .map(|who| slot_for(&tmp, &t4, who))
        .collect();

    // 3 Reviewer -> reply --thread T5* (parallel, unabhaengig voneinander). The third settles
    // #review's set, and because #review FOLDS, that spawns its consolidator rather than delivering.
    for (who, thread) in REVIEWERS.iter().zip(&reviewer_threads) {
        json_of(
            persona(&tmp, who, &session_of(&tmp, who))
                .args(["--json", "reply", "--thread", thread, "approved"]),
        );
    }
    let synth = session_of(&tmp, "__synth__");

    // The fold lands. That discharges T4 — which is a STEP of #coding's flow and its last one, so
    // #coding consolidates in the same call and the pm is woken with the answer.
    let folded = json_of(
        human(&tmp)
            .env("NXC_ACTOR", "__synth__")
            .env("NXC_SESSION", &synth)
            .args([
                "--json",
                "reply",
                "--thread",
                &t4,
                "three approvals, ship it",
            ]),
    );
    assert_eq!(
        folded["woke"], pm,
        "the answer travelled back down to the requester of #coding: {folded}"
    );

    // pm -> reply --thread T1 -> Nutzer.
    json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &t1,
        "the release is out",
    ]));

    // ---- the whole tree, from any thread of it: SEVEN threads over THREE channels (§2's own
    // count), and every one of them answered.
    let op = json_of(human(&tmp).args(["--json", "status", "--thread", &reviewer_threads[2]]))
        ["operations"][0]
        .clone();
    assert_eq!(op["root"], t1, "one operation, anchored on the human: {op}");
    let edges: Vec<(String, Option<String>)> = op["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["thread_id"].as_str().unwrap().to_string(),
                t["parent"].as_str().map(str::to_string),
            )
        })
        .collect();
    assert_eq!(
        edges,
        vec![
            (t1.clone(), None),
            (t2.clone(), Some(t1.clone())),
            (t3.clone(), Some(t2.clone())),
            (t4.clone(), Some(t2.clone())),
            (reviewer_threads[0].clone(), Some(t4.clone())),
            (reviewer_threads[1].clone(), Some(t4.clone())),
            (reviewer_threads[2].clone(), Some(t4.clone())),
        ],
        "seven threads: T4 hangs under T2 because the thread it was sent FROM is the #coding \
         channel thread the supervisor stands on (a71h §3.1's mechanical rule, with the supervisor \
         where a lead persona used to be): {op}"
    );
    let channels: std::collections::BTreeSet<String> = op["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["channel_id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        channels.len() == 3 && channels.contains("decl:coding") && channels.contains("decl:review"),
        "three channels, and no single one of them sees the chain whole: {channels:?}"
    );
    let states: Vec<&str> = op["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["state"].as_str().unwrap())
        .collect();
    assert!(
        states.iter().all(|s| *s == "answered"),
        "every thread of a chain that ran to completion is answered, none orphaned: {states:?}"
    );
    assert_eq!(
        op["threads"][0]["awaiting_human"], true,
        "…and the root waits on the human at the other end, which is not a standstill: {op}"
    );

    // Nothing about this needed a second mechanism beside the declaration — which is a fact about
    // the schema since 6j6v.dvyq §3 removed the run record, asserted once in
    // `a_declared_flow_runs_the_fixed_order_that_used_to_need_a_run` above.
}

// ---- a second request to an ordered channel -----------------------------------------------------

#[test]
fn a_second_request_to_an_ordered_channel_runs_the_order_again_from_its_first_step() {
    // A follow-up into a channel thread hands the channel its next turn (nxf 6j6v.pf6j/6j6v.cg8g).
    // For a `parallel` channel that means waking every member at once, which is what it did the
    // first time too. For an ORDERED one it must NOT: a declared sequence asked a second time is
    // that sequence again, step one first — otherwise a channel's second use would silently stop
    // being the flow its author declared.
    //
    // The pass gets FRESH slot threads, which is what makes each pass answerable on its own and what
    // keeps the second consolidation from gathering the first pass's answers.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "reviewer"], ORDERED);

    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();
    // Pass one, both steps.
    for who in ["coder", "reviewer"] {
        let slot = slot_for(&tmp, &board, who);
        json_of(
            persona(&tmp, who, &session_of(&tmp, who))
                .args(["--json", "reply", "--thread", &slot, "done"]),
        );
    }
    let first_pass = slots(&tmp, &board);
    assert_eq!(first_pass.len(), 2, "{first_pass:?}");

    // The requester asks again.
    json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "now do the follow-up",
    ]));
    let after = slots(&tmp, &board);
    assert_eq!(
        after.len(),
        3,
        "one FRESH slot for step one of the new pass, and nothing else yet: {after:?}"
    );
    assert_eq!(&after[..2], &first_pass[..], "the first pass is untouched");
    assert_eq!(
        started_roles(&tmp),
        vec![
            "pm".to_string(),
            "coder".to_string(),
            "reviewer".to_string(),
            // the first pass consolidating and waking its requester
            "pm".to_string(),
            "coder".to_string()
        ],
        "the second pass started step one again — not both members at once"
    );

    // …and the second pass runs to its own end, from its own first step.
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        after.last().unwrap(),
        "follow-up implemented",
    ]));
    assert_eq!(
        started_roles(&tmp).last().map(String::as_str),
        Some("reviewer"),
        "step two of the second pass"
    );
    let last_slot = slots(&tmp, &board).pop().unwrap();
    let closing = json_of(
        persona(&tmp, "reviewer", &session_of(&tmp, "reviewer")).args([
            "--json",
            "reply",
            "--thread",
            &last_slot,
            "still good",
        ]),
    );
    assert_eq!(
        closing["woke"], pm,
        "the second pass consolidated and answered the requester: {closing}"
    );
    // The delivery carries THIS pass's answers, not the first pass's as well.
    let delivered = closing["completed"]["delivered"].as_str().unwrap();
    assert!(
        delivered.contains("follow-up implemented") && delivered.contains("still good"),
        "{delivered}"
    );
    assert!(
        !delivered.contains("done"),
        "the previous pass's answers are not gathered again: {delivered}"
    );
}

// ---- nxf 6j6v.ma7v: what an ordered flow does when a step says "I cannot" ---------------------

#[test]
fn an_ordered_flow_stops_at_a_step_that_escalated_instead_of_starting_the_next_one() {
    // **This test used to assert the OPPOSITE, and the flip is nxf 6j6v.ma7v.** It stood here as a
    // characterisation — "whether a fixed order should halt when one of its steps reports `I cannot`
    // is an owner decision this item does not settle" — pinned rather than left accidental, with the
    // note that a decision flipping the behaviour should turn it red. It did.
    //
    // What decided it was a measured run, not a preference. On 2026-08-25 a coder used `--escalate`
    // EXPRESSLY to keep the flow from advancing — he had understood the incident it would cause and
    // said so in the body — and the next step's session started ten seconds later all the same, into
    // a checkout still standing on the pre-review commit. Four threads of that one run carried the
    // escalation mark for the same reason, each without effect. With a normal reply starting the
    // successor, an escalation starting it too, and silence getting the session recalled, the member
    // had no correct move left; the only thing that held the flow was a sentence in the commissioning
    // prompt.
    //
    // So: a step that reports it could not carry out its task does not commission the work that was
    // to follow it. The chain ends, the escalation goes up, and the caller decides — which is the
    // reading `--escalate`'s own help has always implied and the one hq71 §4 left to the supervisor
    // to write down.
    //
    // What is asserted here as well, unchanged from the older shape: the escalation is not lost on
    // the way. It rides the channel thread's own discharge, so nxf 6j6v.e9qj's rule holds (never
    // folded away) and nxf 6j6v.1xw1 still reads it off the channel thread to decide whether a
    // working copy may be released.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "reviewer"], ORDERED);

    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();

    // Step one cannot do it.
    let coder_slot = slot_for(&tmp, &board, "coder");
    let receipt = json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &coder_slot,
        "--escalate",
        "I cannot: the credentials are missing",
    ]));

    // The order STOPS. Step two exists nowhere — no session, and no slot thread either. The third
    // entry in that list is not step two: it is the PM being resumed with the escalation, which is
    // the chain ending upwards rather than carrying on downwards.
    assert_eq!(
        started_roles(&tmp),
        vec!["pm".to_string(), "coder".to_string(), "pm".to_string()],
        "the step after an escalation is not started; the requester is resumed instead: {:?}",
        started_roles(&tmp)
    );
    assert_eq!(
        slots(&tmp, &board).len(),
        1,
        "…and no slot thread was minted for it either"
    );

    // …and the escalation went UP in the same write path, to the party that commissioned the round.
    assert_eq!(
        receipt["woke"], pm,
        "the requester is told, in the receipt of the very reply that stopped the flow: {receipt}"
    );
    let store = open_store(&tmp);
    assert_eq!(
        store.last_reply_escalated(&coder_slot).unwrap(),
        Some(true),
        "the step that could not deliver still says so on its own thread"
    );
    assert_eq!(
        store.last_reply_escalated(&board).unwrap(),
        Some(true),
        "…and the whole flow's answer of record carries it, which is what 6j6v.1xw1 reads"
    );
}

#[test]
fn an_escalation_stops_an_ordered_channel_only_and_leaves_a_parallel_one_alone() {
    // The other half of ma7v's rule, and the reason the branch sits behind the `flow` filter rather
    // than in the consolidation: on a `parallel` channel everyone was asked at once, so there is no
    // successor to withhold. An escalating member there changes what is handed UP (6j6v.wt37) and
    // nothing about who was started — which is what it did before this item, down to the byte.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "reviewer"], UNORDERED);

    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();
    assert_eq!(
        started_roles(&tmp),
        vec![
            "pm".to_string(),
            "coder".to_string(),
            "reviewer".to_string()
        ],
        "a parallel channel asked both at once, before anybody escalated"
    );

    let coder_slot = slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &coder_slot,
        "--escalate",
        "I cannot: the credentials are missing",
    ]));

    // The other member is still expected, and answering completes the round exactly as always.
    let reviewer_slot = slot_for(&tmp, &board, "reviewer");
    let closing = json_of(
        persona(&tmp, "reviewer", &session_of(&tmp, "reviewer")).args([
            "--json",
            "reply",
            "--thread",
            &reviewer_slot,
            "nothing to review",
        ]),
    );
    assert_eq!(
        closing["woke"], pm,
        "the parallel round still consolidates and answers the requester: {closing}"
    );
    let store = open_store(&tmp);
    assert_eq!(
        store.last_reply_escalated(&board).unwrap(),
        Some(true),
        "…carrying the escalation up, which is wt37's rule and is untouched here"
    );
}

// ---- review round 1: the refusals, and the fourth failed-consequence site ----------------------

/// `nxc` whose worker starts nothing. `LazyWorker` resolves `NXC_WORKER` on FIRST USE, so an unknown
/// value is not a startup refusal — the message is written, the obligation is registered, and the
/// SPAWN is what fails. The same injection `failed_consequences.rs` uses, at the same real path.
fn persona_no_worker(tmp: &TempDir, handle: &str, session: &str) -> Command {
    let mut c = persona(tmp, handle, session);
    c.env("NXC_WORKER", "not-a-real-worker");
    c
}

/// Whether any finding on `receipt` names `needle` — "the step that never answered is named",
/// asked of the whole list rather than of a position in it, since the order findings are discovered
/// in is not part of any contract.
fn warns_about(receipt: &Value, needle: &str) -> bool {
    warnings(receipt)
        .iter()
        .any(|w| w["detail"].as_str().unwrap_or_default().contains(needle))
}

/// The `warnings` a receipt carries — always present, so an absent key is itself the failure.
fn warnings(receipt: &Value) -> Vec<Value> {
    receipt["warnings"]
        .as_array()
        .unwrap_or_else(|| {
            panic!("`warnings` is the defined place and is never omitted: {receipt}")
        })
        .clone()
}

// `an_ad_hoc_expect_over_a_declared_flow_is_refused_by_name` stood here and pinned PREFLIGHT (2b):
// `ask --expect` could not be laid over a `flow: sequential` channel, because the engine reads the
// order off the declaration every time it advances and honouring both lists would mean the list it
// reads is not the list the caller wrote.
//
// Both the flag and the preflight went with 6j6v.dvyq §3. The invariant `flow_steps`' doc leans on
// — the re-derived step list equals the opened one — is now STRUCTURAL rather than guarded: there
// is no second list a caller could supply, because `ChannelOpenRequest` no longer carries a field
// to supply one with. The DECLARED twin of the refusal, below, is untouched and still fails closed:
// an `expects: <subset>` written into a sequential channel is a real second list and still has to
// be refused at the verb that would run it.

#[test]
fn a_declared_expects_subset_over_a_flow_is_refused_at_the_verb_that_would_run_it() {
    // The declared twin of the refusal above (review round 1, Important 1). `fan_out_targets`
    // returns a subset verbatim, so this channel would have run `[reviewer, coder]` while its
    // `members` said `[coder, reviewer]`. It fails closed at the point of USE, not only in `prime`.
    let tmp = workspace();
    write_roles(
        &tmp,
        &["pm", "coder", "reviewer"],
        "- name: coding\n  members: [coder, reviewer]\n  flow: sequential\n  \
         expects: [reviewer, coder]\n",
    );

    let out = human(&tmp)
        .args(["send", "--to", "coding", "build it"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains("flow=sequential") && stderr.contains("expects"),
        "the refusal names both lists: {stderr}"
    );
    assert!(
        started_roles(&tmp).is_empty(),
        "nothing ran in the subset's order, because nothing ran: {:?}",
        started_roles(&tmp)
    );
}

#[test]
fn a_flow_step_that_could_not_be_started_is_named_in_the_receipt_and_delivers_no_stale_pass() {
    // **The fourth site of nxf 6j6v.93zd's class 1**, in the item whose DoD is "one test per class".
    // A step of a declared flow that cannot start is REPORTED rather than propagated: the reply that
    // settled the step before it is durable by then, so `warnings` is the honest answer.
    //
    // And the second half is what the review traced: the flow must not later deliver somebody else's
    // pass as the answer. `current_pass` returns an EMPTY pass when no slot serves step one, and an
    // empty pass is never settled — so a flow that stopped stays visibly stopped.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "reviewer"], ORDERED);

    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();

    // Step one answers — under a worker that can start nothing, so step two never starts.
    let coder_slot = slot_for(&tmp, &board, "coder");
    let settled = json_of(
        persona_no_worker(&tmp, "coder", &session_of(&tmp, "coder")).args([
            "--json",
            "reply",
            "--thread",
            &coder_slot,
            "first cut is in",
        ]),
    );
    assert_eq!(settled["posted"], true, "the reply IS durable: {settled}");

    let found = warnings(&settled);
    assert_eq!(found.len(), 1, "{settled}");
    assert_eq!(found[0]["class"], "step_skipped", "{settled}");
    assert_eq!(
        found[0]["thread"], board,
        "the flow that stopped is named: {settled}"
    );
    assert_eq!(
        found[0]["reason"], "trigger_failed",
        "machine-readable, and as fine as the runtime itself distinguishes: {settled}"
    );
    let detail = found[0]["detail"].as_str().unwrap();
    assert!(
        detail.contains("reviewer") && detail.contains("not-a-real-worker"),
        "the step that did not run, and why: {settled}"
    );

    // …and the flow really did stop. A ROLE step opens its thread BEFORE it triggers, so step two's
    // thread exists — with its obligation declared and nobody working on it, which is exactly the
    // class-1 shape (`failed_consequences.rs` asserts the same thing for a member: "owes an answer it
    // was never asked for out loud"). That is why this must be reported rather than swallowed.
    let after = slots(&tmp, &board);
    assert_eq!(after.len(), 2, "{after:?}");
    let store = open_store(&tmp);
    assert!(
        !store
            .thread_quorum(after.last().unwrap(), NOW)
            .unwrap()
            .unwrap()
            .outstanding
            .is_empty(),
        "step two's thread owes an answer nobody was asked for"
    );
    drop(store);
    assert!(
        !open_store(&tmp)
            .thread_quorum(&board, NOW)
            .unwrap()
            .unwrap()
            .complete,
        "nothing was delivered — a stopped flow must not answer"
    );

    // The requester asks again. Step one of the SECOND pass cannot start either, so no slot serves
    // step one of it — and the previous pass, which IS settled, must not be handed over as the
    // answer to this new request.
    let again = json_of(persona_no_worker(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "try again",
    ]));
    assert_eq!(again["posted"], true, "{again}");
    assert!(
        again["completed"].is_null(),
        "the previous pass's answers are not the answer to this request: {again}"
    );
    assert!(
        !open_store(&tmp)
            .thread_quorum(&board, NOW)
            .unwrap()
            .unwrap()
            .complete,
        "the channel thread stays open where `nxc status` reads it, rather than answering stale"
    );
}

#[test]
fn a_thread_whose_requester_left_an_empty_return_address_keeps_its_pre_existing_behaviour() {
    // **The ⚠ the review could not settle, settled — and the answer is that it was reachable.**
    // `--ref session_id=` parses to `Some("")` with no filter (`cli.rs`'s `parse_refs`) and `ask`
    // takes `--ref`, so an empty return address predates this item. The path that reaches
    // `route_completion` with one is the NON-DECLARED quorum board: a declared channel's completion
    // is owned by the supervisor (`reply`'s block (5) is guarded by `!supervised`), which has always
    // passed an empty address of its own, while a raw `ask` board goes through block (5) and the
    // guard this item relaxed.
    //
    // Relaxing on "the address is empty" would therefore have turned THIS caller from "skipped wake
    // reported, nothing routed" into "routed" — a change to a pre-existing path. The relaxation is
    // conditioned on the thread being a supervisor-opened flow STEP instead, which is the same
    // question `reply`'s own substitution asks, and this pins that a raw board lands exactly where it
    // always did.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "reviewer"], UNORDERED);

    // A board on a channel NO DECLARATION names, so nothing supervises it. Built through the
    // library rather than the command line: `channels create` and `ask` both went with 6j6v.dvyq
    // §3, and the SHAPE they produced — an unsupervised multi-handle board — is still perfectly
    // reachable, it just is not something an agent types any more. `facade::ask` is the write both
    // verbs delegated to and is untouched.
    let board = {
        let mut store = open_store(&tmp);
        store.set_channel_field("c-raw", "kind", "group", "local/pm");
        store.add_member("c-raw", "local/pm", "local/pm");
        nexus_chat::facade::ask(
            &mut store,
            nexus_chat::facade::AskRequest {
                now: NOW,
                origin: "local",
                actor: "pm",
                channel: "c-raw",
                body: "look at this",
                expect: &["local/coder".to_string(), "local/reviewer".to_string()],
                deadline: None,
                kind: nexus_chat::model::MessageKind::Question,
                priority: nexus_chat::model::Priority::Normal,
                // The empty return address this test is ABOUT: `--ref session_id=` parses to
                // `Some("")` with no filter, so it predates the item and is still writable.
                refs: nexus_chat::model::Refs {
                    session_id: Some(String::new()),
                    ..Default::default()
                },
            },
        )
        .expect("the board opens")
        .thread_id
    };
    assert!(
        slots(&tmp, &board).is_empty(),
        "a raw board has no supervisor and no step threads"
    );

    json_of(
        persona(&tmp, "coder", "s-coder").args(["--json", "reply", "--thread", &board, "done"]),
    );
    // The second expected handle settles the board, so this is the call that would route.
    let settling = json_of(
        persona(&tmp, "reviewer", "s-reviewer")
            .args(["--json", "reply", "--thread", &board, "done too"]),
    );

    assert_eq!(
        settling["wake_skipped"]["reason"], "unresolved",
        "the empty address is still reported as a skipped wake, exactly as before: {settling}"
    );
    assert!(
        settling["completed"].is_null(),
        "…and nothing was routed for it: {settling}"
    );
}

/// An ordered channel whose FIRST step is a nested channel — the one shape where a step can fail to
/// start and mint no slot at all, because every preflight of a nested channel runs before anything is
/// opened.
const NESTED_FIRST: &str = "- name: coding\n  members: [review, reviewer]\n  flow: sequential\n\
                            - name: review\n  members: [coder]\n";
/// The same declarations after an author edit that would make step one unrunnable — an empty member
/// list, which `open_declared_channel_and_fan_out`'s PREFLIGHT (2a) refuses as "nobody to ask".
///
/// **It no longer reaches the running operation** (nxf 6j6v.n92p), which is what the test below now
/// says. The declarations an operation opened under are the ones it runs to its end under, so this
/// edit governs the NEXT operation and not this one.
const NESTED_FIRST_BROKEN: &str =
    "- name: coding\n  members: [review, reviewer]\n  flow: sequential\n\
     - name: review\n  members: []\n";

#[test]
fn an_edit_between_two_passes_does_not_reach_the_operation_that_is_running() {
    // **This test used to prove the opposite, and the swap is the whole of nxf 6j6v.n92p's freeze.**
    // It read: a declaration read for pass one can have changed by pass two, so a step of pass two
    // fails its preflight, mints nothing, and — this was the property — the requester is NOT handed
    // pass one's answer in place of the one nobody produced. The edit was the realistic way in.
    //
    // The edit is no longer a way in at all: an operation is bound to the declarations it opened
    // under (`declaration_freeze.rs`), so the author's change takes effect in the next operation and
    // pass two runs the order it was declared with. That is asserted here, at the flow, because this
    // file is where a reader comes to find out what a second pass does.
    //
    // **The property it used to carry has moved rather than gone**, to the one cause that still
    // reaches "a step that mints no thread": a refused precondition. See
    // `preconditions.rs::a_pass_whose_first_step_a_hurdle_refused_delivers_nothing_rather_than_the_pass_before_it`,
    // which is the same assertion over the same ordering in `supervisor_hand_out_next_turn` — the
    // channel thread is re-armed only once step one has actually started.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "reviewer"], NESTED_FIRST);

    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();

    // Pass one, both steps: the nested #review channel, then the reviewer.
    let review_board = slots(&tmp, &board)
        .into_iter()
        .find(|t| open_store(&tmp).thread_channel(t).as_deref() == Some("decl:review"))
        .expect("step one opened #review's own channel thread");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &slot_for(&tmp, &review_board, "coder"),
        "reviewed by the nested channel",
    ]));
    let closing = json_of(
        persona(&tmp, "reviewer", &session_of(&tmp, "reviewer")).args([
            "--json",
            "reply",
            "--thread",
            &slot_for(&tmp, &board, "reviewer"),
            "and by me",
        ]),
    );
    assert!(
        closing["completed"].is_object(),
        "pass one delivered: {closing}"
    );

    // The author edits `channels.yaml` so step one COULD no longer run — an empty member list for
    // the nested channel, which its own preflight refuses before anything persists.
    std::fs::write(
        tmp.path().join(".nxs-personas").join("channels.yaml"),
        NESTED_FIRST_BROKEN,
    )
    .unwrap();

    // The requester asks again, in the operation that is already running. Step one starts anyway:
    // this operation is bound to the declarations it opened under, and under those `review` has a
    // member.
    let before = slots(&tmp, &board).len();
    let again = json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "another round please",
    ]));
    assert_eq!(again["posted"], true, "{again}");
    assert_eq!(
        slots(&tmp, &board).len(),
        before + 1,
        "pass two opened its own step-one slot under the declarations the operation began with: \
         {again}"
    );
    assert!(
        warnings(&again).is_empty(),
        "and nothing was reported, because nothing failed: {again}"
    );

    // **And nothing was delivered for the new request yet** — pass two has only just begun, so the
    // previous pass's answer must not be handed back as the answer to it.
    assert!(
        again["completed"].is_null(),
        "the previous pass is not the answer to this request: {again}"
    );
}

#[test]
fn a_late_answer_from_a_running_pass_does_not_consolidate_it_against_a_newer_request() {
    // **Review round 2's reproduction.** A follow-up into the channel thread starts pass two while a
    // step of pass one is still working — nothing gates that, and nothing should: the requester is
    // entitled to ask again. What must not happen is what the round-1 redesign allowed: pass one's
    // straggler finally answers, its slot reads as the last step of a full pass, and the supervisor
    // folds pass one's answers against pass two's request — after which the channel thread carries
    // the delivered marker and pass two becomes a silent no-op.
    //
    // **Two defences refuse it, and this test is held by EITHER** — which is worth saying, because it
    // is why removing just one of them does not turn this test red (round 3's `M-R3b` found that, and
    // it is the reason the gate has a test of its own further down):
    //
    //  * `current_pass` is ANCHORED at the newest slot serving step one, so the straggler — older
    //    than where pass two began — is not in the derived set at all and `find` answers `None`;
    //  * and the settled-set gate would refuse the set anyway, because pass two's own step one is
    //    outstanding in it.
    //
    // The mirror ordering, where these two come apart, is
    // `a_new_pass_advances_on_its_own_answer_even_while_the_previous_pass_is_still_owed_one`.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "reviewer"], ORDERED);

    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();

    // Pass one: step one answers, so step two opens and the reviewer starts working.
    let a1 = slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &a1,
        "first cut",
    ]));
    let b1 = slots(&tmp, &board)
        .into_iter()
        .next_back()
        .expect("step two opened");
    assert_ne!(b1, a1);

    // The requester asks again WHILE the reviewer is still working. Pass two opens its own step one.
    let again = json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "actually, do this instead",
    ]));
    assert!(
        again["completed"].is_null(),
        "asking again delivers nothing by itself: {again}"
    );
    let a2 = slots(&tmp, &board)
        .into_iter()
        .next_back()
        .expect("pass two opened its step one");
    assert_ne!(a2, b1, "pass two's step one is a fresh slot");

    // Pass one's straggler finally answers. It is the last step of the DERIVED pass — and it must
    // still deliver nothing, because pass two's step one is outstanding.
    let late = json_of(
        persona(&tmp, "reviewer", &session_of(&tmp, "reviewer")).args([
            "--json",
            "reply",
            "--thread",
            &b1,
            "late review of the old request",
        ]),
    );
    assert!(
        late["completed"].is_null(),
        "pass one's answers are not folded against pass two's request: {late}"
    );
    assert!(
        late["woke"].is_null(),
        "…and nobody is woken with them: {late}"
    );

    // …and pass two is still ALIVE rather than a silent no-op: its step one answers, the flow
    // advances to step two, and only then is anything delivered.
    let store = open_store(&tmp);
    assert!(
        !store.thread_quorum(&board, NOW).unwrap().unwrap().complete,
        "the channel thread still owes pass two an answer"
    );
    drop(store);
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &a2,
        "second cut",
    ]));
    let b2 = slots(&tmp, &board)
        .into_iter()
        .next_back()
        .expect("pass two advanced to its step two");
    assert_ne!(b2, b1, "a fresh slot, not pass one's");
    let closing = json_of(
        persona(&tmp, "reviewer", &session_of(&tmp, "reviewer")).args([
            "--json",
            "reply",
            "--thread",
            &b2,
            "review of the new request",
        ]),
    );
    assert_eq!(
        closing["woke"], pm,
        "pass two runs to its end and answers its own requester: {closing}"
    );
    let delivered = closing["completed"]["delivered"].as_str().unwrap();
    assert!(
        delivered.contains("second cut") && delivered.contains("review of the new request"),
        "and it delivers pass TWO's answers: {delivered}"
    );
    assert!(
        !delivered.contains("late review of the old request"),
        "…not the straggler's: {delivered}"
    );
}

/// The same ordered channel after an author edit that renames its FIRST step to a handle no slot of
/// the running flow serves — the shape that makes a pass underivable mid-flight.
const ORDERED_STEP_ONE_RENAMED: &str =
    "- name: coding\n  members: [stand-in, reviewer]\n  flow: sequential\n  timeout: 5m\n";
/// Its before-the-edit twin, with a window so a step that never answers can still SETTLE.
const ORDERED_TIMED: &str =
    "- name: coding\n  members: [coder, reviewer]\n  flow: sequential\n  timeout: 5m\n";

#[test]
fn a_consolidation_whose_pass_is_underivable_folds_the_answers_not_the_request() {
    // Review round 2's second minor, GUARDED rather than merely documented — and this is the shape
    // that reaches the guard. `current_pass` anchors on the newest slot serving the FIRST declared
    // step, so renaming that step mid-flight to a handle nothing serves leaves it no anchor at all
    // and it answers EMPTY. `collected_replies` reads an empty member
    // list as "this board has no member threads at all" and hands back the channel thread's own
    // messages, which on a channel means folding the REQUESTER's own request and delivering it as the
    // answer to itself.
    //
    // `supervisor_consider_set`'s settled-set gate refuses an empty pass outright, so the way in is
    // the PULL path: `workflow tick` asks its own `due` question over EVERY slot, and a step whose
    // window ran out is settled without anybody having answered it. A consolidation therefore
    // over-includes rather than under-includes.
    let tmp = workspace();
    write_roles(
        &tmp,
        &["pm", "coder", "reviewer", "stand-in"],
        ORDERED_TIMED,
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
        "REQUEST-TEXT",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    // Step one answers, which opens step two. Step two never does.
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &slot_for(&tmp, &board, "coder"),
        "CODER-ANSWER",
    ]));
    assert_eq!(slots(&tmp, &board).len(), 2, "step two opened");

    // The author renames step one. Nothing serves it now, so the pass is underivable.
    std::fs::write(
        tmp.path().join(".nxs-personas").join("channels.yaml"),
        ORDERED_STEP_ONE_RENAMED,
    )
    .unwrap();

    // Past step two's window: every slot is settled, so the PULL path consolidates.
    let ticked =
        json_of(at(&tmp, "2026-08-16T10:30:00Z").args(["--json", "tick", "--thread", &board]));
    let delivered = ticked["delivered"]
        .as_str()
        .unwrap_or_else(|| panic!("the tick consolidated: {ticked}"));
    assert!(
        delivered.contains("CODER-ANSWER"),
        "an underivable pass falls back to every slot, so the members' ANSWERS are what is \
         delivered — never the requester's own request folded as the answer to itself: {delivered}"
    );
}

#[test]
fn a_new_pass_advances_on_its_own_answer_even_while_the_previous_pass_is_still_owed_one() {
    // **The mirror of the test above, and review round 3's open item.** Same three steps — pass one's
    // step two is still working when the requester asks again — but now pass TWO's step one answers
    // FIRST. Its advance is legitimate and must happen: nothing about a straggler from an earlier
    // pass has any bearing on it.
    //
    // Round 2's settled-set gate alone refused exactly this, silently and with nothing in the
    // receipt, because the derived set still carried the straggler's outstanding slot. What makes it
    // right is `current_pass` being ANCHORED at the pass's own first step: a slot older than where
    // this pass began is not in this pass at all.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "reviewer"], ORDERED);

    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();

    let a1 = slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &a1,
        "first cut",
    ]));
    let b1 = slots(&tmp, &board).into_iter().next_back().unwrap();

    // The requester asks again while the reviewer is still working: pass two mints its own step one.
    json_of(persona(&tmp, "pm", &pm).args([
        "--json",
        "reply",
        "--thread",
        &board,
        "actually, do this instead",
    ]));
    let a2 = slots(&tmp, &board).into_iter().next_back().unwrap();
    assert_ne!(a2, b1);

    // **Pass two's step one answers FIRST.** It must advance — the straggler is not its business.
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &a2,
        "second cut",
    ]));
    let after = slots(&tmp, &board);
    assert_eq!(
        after.len(),
        4,
        "pass two advanced to its own step two rather than stalling: {after:?}"
    );
    let b2 = after.into_iter().next_back().unwrap();
    assert_ne!(b2, b1, "a fresh slot for pass two, not pass one's");

    // The straggler finally answers. It belongs to no live pass, so it is a clean no-op.
    let late = json_of(
        persona(&tmp, "reviewer", &session_of(&tmp, "reviewer")).args([
            "--json",
            "reply",
            "--thread",
            &b1,
            "late review of the old request",
        ]),
    );
    assert!(
        late["completed"].is_null(),
        "a straggler from a finished-with pass delivers nothing: {late}"
    );

    // …and pass two finishes on its own answers, with the straggler's nowhere in them.
    let closing = json_of(
        persona(&tmp, "reviewer", &session_of(&tmp, "reviewer")).args([
            "--json",
            "reply",
            "--thread",
            &b2,
            "review of the new request",
        ]),
    );
    assert_eq!(closing["woke"], pm, "{closing}");
    let delivered = closing["completed"]["delivered"].as_str().unwrap();
    assert!(
        delivered.contains("second cut") && delivered.contains("review of the new request"),
        "{delivered}"
    );
    assert!(
        !delivered.contains("late review of the old request"),
        "the straggler's answer is not folded into pass two's delivery: {delivered}"
    );
}

#[test]
fn a_second_reply_on_a_step_already_answered_does_not_open_the_next_one_twice() {
    // **What the settled-set gate still guards, once `current_pass` is anchored.** The anchor keeps
    // OTHER passes out of the set; it says nothing about this pass being mid-flight. A member may
    // reply into its own thread more than once — `reply` has no membership gate and nothing stops it
    // — so a second reply on step one, while step two is still working, reaches
    // `supervisor_consider_set` with a slot that is settled and at a position that has a successor.
    // Without the gate the supervisor advances again and opens step two a SECOND time: a duplicate
    // thread, a duplicate task and a second paid session, for a step already under way.
    let tmp = workspace();
    write_roles(&tmp, &["pm", "coder", "reviewer"], ORDERED);

    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();

    let a1 = slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &a1,
        "first cut",
    ]));
    let after_first = slots(&tmp, &board);
    assert_eq!(
        after_first.len(),
        2,
        "step two opened once: {after_first:?}"
    );

    // The same member speaks again on the SAME step, while step two is still working.
    let again = json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &a1,
        "…and one more thing",
    ]));
    assert_eq!(
        again["posted"], true,
        "the second reply IS durable: {again}"
    );
    assert_eq!(
        slots(&tmp, &board),
        after_first,
        "step two was not opened a second time: a step already under way is not re-commissioned"
    );
    assert!(
        again["completed"].is_null(),
        "…and nothing was consolidated over a pass that is still running: {again}"
    );
}

// ---- nxf 6j6v.rs9k: a step whose DEADLINE strikes is settled, and settled means the flow moves on

/// Three steps, because a MIDDLE one is the whole question (nxf 6j6v.rs9k) — with two steps every
/// lapse is a lapse of the last one, where consolidating is right and the defect is invisible. The
/// `timeout` is what lets a step settle without anybody answering it.
const ORDERED_THREE_TIMED: &str = "- name: coding\n  members: [coder, reviewer, shipper]\n  \
                                   flow: sequential\n  timeout: 5m\n";

/// The instant the middle step's window has run out — what the one-shot job scheduled for it fires
/// at, and what a hand-driven `nxc tick` stands in for here.
const PAST_THE_WINDOW: &str = "2026-08-16T10:30:00Z";

#[test]
fn a_middle_step_whose_window_lapses_advances_the_flow_instead_of_consolidating() {
    // **The two halves of the declared meaning, driven end to end.** `Flow::Sequential` says each
    // step starts "once the one before it has SETTLED", and `settled` is `complete || stale`
    // everywhere in this engine — so a step whose window ran out is a reason to advance, exactly as
    // an answer is. Before this item only an ANSWER reached the advance: a lapse left the set
    // reading settled, the tick consolidated it, and every step after the lapsed one never ran.
    let tmp = workspace();
    write_roles(
        &tmp,
        &["pm", "coder", "reviewer", "shipper"],
        ORDERED_THREE_TIMED,
    );

    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();

    // Step one answers, which starts step two. Step two says nothing at all, ever.
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &slot_for(&tmp, &board, "coder"),
        "CODER-ANSWER",
    ]));
    assert_eq!(
        started_roles(&tmp),
        vec![
            "pm".to_string(),
            "coder".to_string(),
            "reviewer".to_string()
        ],
        "step two is running and step three does not exist yet"
    );

    // The middle step's window runs out and the job that watches it fires.
    let armed_before = timer_lines(&tmp).len();
    let ticked = json_of(at(&tmp, PAST_THE_WINDOW).args(["--json", "tick", "--thread", &board]));

    assert_eq!(
        started_roles(&tmp),
        vec![
            "pm".to_string(),
            "coder".to_string(),
            "reviewer".to_string(),
            "shipper".to_string()
        ],
        "a lapsed MIDDLE step settles that step and starts the next one — the flow is not over: \
         {ticked}"
    );
    assert!(
        ticked["delivered"].is_null(),
        "…so nothing was consolidated and nothing was delivered: {ticked}"
    );
    assert!(
        !open_store(&tmp)
            .thread_quorum(&board, PAST_THE_WINDOW)
            .unwrap()
            .unwrap()
            .complete,
        "the requester is still owed the answer of a flow that is still running"
    );
    // …and the step that never answered is NAMED rather than silently absorbed — under the class a
    // caller branches on, and pointing at the slot it stayed silent on.
    let lapse = warnings(&ticked)
        .into_iter()
        .find(|w| {
            w["detail"]
                .as_str()
                .unwrap_or_default()
                .contains("reviewer")
        })
        .unwrap_or_else(|| panic!("the lapse is reported: {ticked}"));
    assert_eq!(lapse["class"], "step_unanswered", "{lapse}");
    assert_eq!(
        lapse["thread"],
        Value::String(slot_for(&tmp, &board, "reviewer")),
        "the finding points at the slot that stayed silent, so an app can go and look at it: \
         {lapse}"
    );
    // **The automatic chain survives the advance** (nxf 6j6v.nf38): an `at` job fires once, so the
    // step just started needs its own, or this flow's next lapse would wait for somebody to ask by
    // hand. The advance re-arms it — this is that job.
    let armed = timer_lines(&tmp);
    assert!(
        armed.len() > armed_before && armed.last().is_some_and(|l| l.contains(&board)),
        "the window of the step this advance started is watched by a job of its own: {armed:?}"
    );

    // The last step answers. NOW the flow is over, the requester is woken — and the consolidation
    // still says which step it has no answer from.
    let last = json_of(
        persona(&tmp, "shipper", &session_of(&tmp, "shipper"))
            .env("NXC_NOW", "2026-08-16T10:31:00Z")
            .args([
                "--json",
                "reply",
                "--thread",
                &slot_for(&tmp, &board, "shipper"),
                "SHIPPED",
            ]),
    );
    assert!(
        last["completed"].is_object(),
        "the LAST step's answer is what finishes the flow: {last}"
    );
    assert_eq!(
        last["woke"], pm,
        "…and the requester hears about it: {last}"
    );
    assert!(
        warns_about(&last, "reviewer"),
        "the delivery reports the step it carries no answer from: {last}"
    );
}

#[test]
fn a_lapse_on_the_last_step_still_consolidates_the_pass() {
    // **The companion, and the reason the defect above stayed invisible.** On the LAST step there is
    // nothing to advance to, so a lapse there means the flow is over and consolidating is the right
    // answer — which is what 6j6v.hq71's own empty-run test rests on. The fix must not turn every
    // lapse into an advance.
    let tmp = workspace();
    write_roles(
        &tmp,
        &["pm", "coder", "reviewer", "shipper"],
        ORDERED_THREE_TIMED,
    );

    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let board =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coding", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &slot_for(&tmp, &board, "coder"),
        "CODER-ANSWER",
    ]));
    json_of(
        persona(&tmp, "reviewer", &session_of(&tmp, "reviewer")).args([
            "--json",
            "reply",
            "--thread",
            &slot_for(&tmp, &board, "reviewer"),
            "REVIEWER-ANSWER",
        ]),
    );
    assert_eq!(
        started_roles(&tmp).last().map(String::as_str),
        Some("shipper"),
        "the flow reached its last step"
    );

    // The last step's window runs out.
    let ticked = json_of(at(&tmp, PAST_THE_WINDOW).args(["--json", "tick", "--thread", &board]));

    let delivered = ticked["delivered"]
        .as_str()
        .unwrap_or_else(|| panic!("a lapse on the last step consolidates: {ticked}"));
    assert!(
        delivered.contains("CODER-ANSWER") && delivered.contains("REVIEWER-ANSWER"),
        "…and delivers the answers the pass DID collect: {delivered}"
    );
    assert_eq!(
        ticked["woke"], pm,
        "the requester is woken by the consolidation: {ticked}"
    );
    assert!(
        warns_about(&ticked, "shipper"),
        "…and the step that never answered is named here too: {ticked}"
    );
}
