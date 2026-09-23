//! **No role is started without a coordinator standing between the sender and it** (nxf 6j6v.ntp9).
//!
//! The claim the item asks for, in its own words: *there is no path in the source on which a role is
//! triggered without a coordinator — shown by a test AT THE SEAM, not by reading.* The seam is
//! [`nexus_chat::worker::Worker::trigger`]: every session that ever runs is handed to it as one
//! [`nexus_chat::worker::TriggerRequest`], and that request carries
//! [`nexus_chat::worker::Coordinator`] — the named place that admitted it.
//!
//! **Reading would not do**, and the reason is on the record twice. `send --to <persona>` looked
//! like it had a coordinator, because `crate::surface::send_to` opened a thread and composed a wake
//! line before handing over — but those were the SURFACE's, so the property held for `nxc` and for
//! nothing else that reaches the same verb (nxf 41j0.vhsk's point about a host's own worker). And
//! the `summarize` synthesizer genuinely does bypass `trigger_role`, the funnel a reader would check
//! — it is still the channel's own supervisor that starts it, which is a fact about the call site
//! and not about the function.
//!
//! So everything here is driven through the real `nxc` binary against the dry worker, which records
//! `coordinator=` on every trigger it is handed, and nothing constructs a request by hand. The last
//! test is the claim itself: over one round it asserts the WHOLE record, as an exact ordered list of
//! `(role, coordinator)` pairs.
//!
//! **Exact, and not "every value is one of the four"** (independent review of PR #378, Test Quality
//! #1). That weaker assertion was the first cut of this file and it could not fail: `Coordinator` is
//! a required, non-`Option` enum with exactly those four variants, so it re-stated what the type
//! already guarantees. Proof it was hollow: naming the synthesizer `Persona` — the ONE spawn this
//! module's own header calls the reason reading would not do — left the suite green. The vector is
//! what makes a wrong answer fail, and it is why each test below spells out what it expects rather
//! than checking a set membership.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-08-26T10:00:00Z";

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
        // The dry timer on every call: a channel with a declared `timeout` would otherwise leave
        // one-shot `at` jobs on the machine the suite ran on.
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

fn write_roles(tmp: &TempDir, roles: &[(&str, &str)], channels: &str) {
    let dir = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&dir).unwrap();
    for (handle, extra) in roles {
        std::fs::write(
            dir.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\ntools: [Bash]\n{extra}"),
        )
        .unwrap();
    }
    std::fs::write(dir.join("channels.yaml"), channels).unwrap();
}

/// Every dry-log line that opens a trigger entry, in the order the engine started them. The message
/// field embeds real newlines, so only the FIRST physical line of an entry starts with
/// `trigger role=`.
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

/// `(role, coordinator)` for every trigger so far — what this whole suite reads.
fn admissions(tmp: &TempDir) -> Vec<(String, String)> {
    trigger_lines(tmp)
        .iter()
        .map(|l| (field(l, "role"), field(l, "coordinator")))
        .collect()
}

fn session_of(tmp: &TempDir, role: &str) -> String {
    trigger_lines(tmp)
        .iter()
        .filter(|l| field(l, "role") == role)
        .map(|l| field(l, "session"))
        .next_back()
        .unwrap_or_else(|| panic!("no trigger for {role} in {:?}", trigger_lines(tmp)))
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

const PLAIN: &str = "";
const EXCLUSIVE: &str = "working_tree: exclusive\n";

// ---- one entrance at a time -------------------------------------------------------------------

#[test]
fn a_task_addressed_to_a_persona_is_admitted_by_the_persona_coordinator() {
    // The entrance the item is about. Before nxf 6j6v.ntp9 this path had no named place at all: the
    // surface materialised the conversation, opened the thread and called the spawn primitive, and
    // the answer to "who stood between the sender and the role" was nobody.
    let tmp = workspace();
    write_roles(&tmp, &[("coder", PLAIN)], "[]\n");

    json_of(human(&tmp).args(["--json", "send", "--to", "coder", "build it"]));

    assert_eq!(
        admissions(&tmp),
        vec![("coder".to_string(), "persona".to_string())],
        "one commission, admitted by the persona coordinator"
    );
}

#[test]
fn the_coordinator_opens_the_thread_and_hands_its_id_back() {
    // The half of the same move that is not about the name (nxf 6j6v.ntp9, "Faden anlegen und die
    // Id zurueckgeben"). The receipt's `thread_id` is now the coordinator's answer rather than
    // something the surface had opened before it called — which is what makes it impossible for any
    // other route into the same verb to reach a role with nothing to answer into.
    let tmp = workspace();
    write_roles(&tmp, &[("coder", PLAIN)], "[]\n");

    let receipt = json_of(human(&tmp).args(["--json", "send", "--to", "coder", "build it"]));
    let thread = receipt["thread_id"].as_str().expect("a thread id");

    assert_eq!(
        receipt["expects"],
        serde_json::json!(["local/coder"]),
        "and the obligation is declared on it: {receipt}"
    );
    let quorum = open_store(&tmp)
        .thread_quorum(thread, NOW)
        .unwrap()
        .expect("the thread the coordinator opened");
    assert_eq!(
        quorum.expects,
        vec!["local/coder".to_string()],
        "the register agrees with the receipt"
    );
}

#[test]
fn every_step_of_a_declared_channel_is_admitted_by_its_supervisor() {
    // The path that already had one. It shares `coordinator_commission_in` with the persona path,
    // so the value cannot be derived from being in that body — this is what pins that the caller
    // states it and states it correctly.
    let tmp = workspace();
    write_roles(
        &tmp,
        &[("coder", PLAIN), ("reviewer", PLAIN)],
        "- name: coding\n  members: [coder, reviewer]\n  flow: sequential\n",
    );

    let board = json_of(human(&tmp).args(["--json", "send", "--to", "coding", "build it"]))
        ["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        admissions(&tmp),
        vec![("coder".to_string(), "channel".to_string())],
        "step one, admitted by the channel supervisor"
    );

    // Step two is opened by the supervisor when step one settles. NOT a different spawn site — both
    // the fan-out and the advance reach `open_flow_step` and the same `coordinator_commission_in`
    // (the first cut of this comment claimed otherwise; independent review of PR #378, Test Quality
    // #10). What it is worth having for is the end-to-end ADVANCE: it proves the supervisor's value
    // survives a settled set and a second entrance into the same body.
    let coder = session_of(&tmp, "coder");
    let slot = slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &coder).args(["--json", "reply", "--thread", &slot, "done"]));
    human(&tmp)
        .args(["session", "ended", &coder])
        .assert()
        .success();

    assert_eq!(
        admissions(&tmp),
        vec![
            ("coder".to_string(), "channel".to_string()),
            ("reviewer".to_string(), "channel".to_string()),
        ],
        "the flow advanced, and the supervisor is what advanced it"
    );
}

#[test]
fn an_answer_travelling_back_up_names_the_return_path() {
    // `ChainMove::Unwind` — nothing is commissioned here, so the coordinator that commissioned this
    // work is one level up and this is its result arriving. It is a real spawn at the seam either
    // way, which is why it has to answer the question too.
    let tmp = workspace();
    write_roles(&tmp, &[("pm", PLAIN), ("coder", PLAIN)], "[]\n");

    let pm = json_of(human(&tmp).args(["--json", "send", "--to", "pm", "ship it"]))["session"]
        .as_str()
        .unwrap()
        .to_string();
    let handed =
        json_of(persona(&tmp, "pm", &pm).args(["--json", "send", "--to", "coder", "build it"]))
            ["thread_id"]
            .as_str()
            .unwrap()
            .to_string();
    let coder = session_of(&tmp, "coder");

    json_of(persona(&tmp, "coder", &coder).args(["--json", "reply", "--thread", &handed, "done"]));

    assert_eq!(
        admissions(&tmp),
        vec![
            ("pm".to_string(), "persona".to_string()),
            ("coder".to_string(), "persona".to_string()),
            ("pm".to_string(), "return".to_string()),
        ],
        "two commissions down, one answer back up"
    );
}

#[test]
fn a_commission_released_from_the_working_tree_queue_names_the_release() {
    // The fourth named place, and the one a reader is most likely to miss: `fire_queued_trigger`
    // starts a role from a row in a queue, minutes after the coordinator that admitted it returned.
    // The admission is not re-made there — the declaration and the obligation are re-derived — so
    // the honest answer is "released", not a second claim to have coordinated it.
    let tmp = workspace();
    write_roles(&tmp, &[("coder", EXCLUSIVE), ("rival", EXCLUSIVE)], "[]\n");

    let first = json_of(human(&tmp).args(["--json", "send", "--to", "coder", "build it"]));
    let coder_thread = first["thread_id"].as_str().unwrap().to_string();
    let coder = first["session"].as_str().unwrap().to_string();

    let queued = json_of(human(&tmp).args(["--json", "send", "--to", "rival", "and this"]));
    assert_eq!(
        queued["queue_position"], 1,
        "the second exclusive chain is parked, not started: {queued}"
    );
    // The QUEUED arm builds its own receipt, so it is its own chance to forget the thread the
    // coordinator opened (independent review of PR #378, Test Quality #9). A parked commission has
    // one — that is what it will be fired into — and its caller needs the id to watch it.
    assert!(
        queued["thread_id"].as_str().is_some_and(|t| !t.is_empty()),
        "a parked commission still names the thread the coordinator opened for it: {queued}"
    );
    assert_eq!(
        admissions(&tmp),
        vec![("coder".to_string(), "persona".to_string())],
        "nothing has started for the parked commission yet"
    );

    // The first chain answers, which ends its operation and gives the working copy back.
    json_of(persona(&tmp, "coder", &coder).args([
        "--json",
        "reply",
        "--thread",
        &coder_thread,
        "done",
    ]));

    assert_eq!(
        admissions(&tmp).last().cloned(),
        Some(("rival".to_string(), "released".to_string())),
        "the release is what started it, and says so: {:?}",
        admissions(&tmp)
    );
}

// ---- the two paths a commission does NOT take -------------------------------------------------

#[test]
fn a_member_handed_its_next_turn_is_admitted_by_the_supervisor_that_hands_it_out() {
    // `supervisor_hand_out_next_turn` reaches `trigger_role` DIRECTLY, not through the coordinator —
    // a second turn of a channel round resumes the member's existing session instead of commissioning
    // a new one. A reader chasing `coordinator_commission` would never arrive here, which is why it
    // needs its own line in the record (independent review of PR #378, Test Quality #2).
    let tmp = workspace();
    write_roles(
        &tmp,
        &[("pm", PLAIN), ("coder", PLAIN)],
        "- name: coding\n  members: [coder]\n",
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
    let coder = session_of(&tmp, "coder");
    let slot = slot_for(&tmp, &board, "coder");

    // Round one settles and is consolidated back to the pm…
    json_of(persona(&tmp, "coder", &coder).args(["--json", "reply", "--thread", &slot, "done"]));
    // …and the pm opens a SECOND turn on the same board, which is what hands the member its next one.
    json_of(persona(&tmp, "pm", &pm).args(["--json", "reply", "--thread", &board, "now do this"]));

    assert_eq!(
        admissions(&tmp),
        vec![
            ("pm".to_string(), "persona".to_string()),
            ("coder".to_string(), "channel".to_string()),
            ("pm".to_string(), "return".to_string()),
            ("coder".to_string(), "channel".to_string()),
        ],
        "the second turn is the supervisor's too, and the answer between them travelled back up"
    );
}

#[test]
fn a_completed_board_wakes_its_requester_along_the_return_path() {
    // `wake_role_requester` — the third `Unwind` site, and the one that carries a DECLARED channel's
    // consolidation back to whoever opened it. `pass_through` rather than `summarize` so the answer
    // reaches the requester directly instead of via a synthesizer (Test Quality #2, gap (c)).
    let tmp = workspace();
    write_roles(
        &tmp,
        &[("pm", PLAIN), ("coder", PLAIN)],
        "- name: coding\n  members: [coder]\n  on_complete: pass_through\n",
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
    let coder = session_of(&tmp, "coder");
    let slot = slot_for(&tmp, &board, "coder");
    json_of(persona(&tmp, "coder", &coder).args(["--json", "reply", "--thread", &slot, "done"]));

    assert_eq!(
        admissions(&tmp),
        vec![
            ("pm".to_string(), "persona".to_string()),
            ("coder".to_string(), "channel".to_string()),
            ("pm".to_string(), "return".to_string()),
        ],
        "the board's answer went back to its requester, and the return path says so"
    );
}

// ---- the claim itself -------------------------------------------------------------------------

#[test]
fn the_whole_record_of_a_round_is_exactly_this_and_every_line_names_who_admitted_it() {
    // **This is the acceptance point.** Everything above names ONE entrance; this drives a round
    // that goes through several at once — a persona commission, a declared channel's fan-out, and a
    // consolidation folded by a model that is not a declared role at all — and asserts the WHOLE
    // record as an exact ordered vector.
    //
    // The exactness is the test. Asserting only that each value is one of the four known tokens
    // could not fail: `Coordinator` is a required enum with exactly those variants. Naming the
    // synthesizer `Persona` — the one spawn that bypasses `trigger_role`, and the reason this file
    // exists at all — passed that version and fails this one.
    let tmp = workspace();
    write_roles(
        &tmp,
        &[("pm", PLAIN), ("coder", PLAIN), ("reviewer", PLAIN)],
        "- name: coding\n  members: [coder, reviewer]\n  flow: sequential\n  \
         on_complete: summarize\n  summary_prompt: Fold the results.\n",
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

    // `sequential`, so the steps are ordered and so is the record — a parallel fan-out mints its
    // whole set inside one millisecond and its children come back in any order at all.
    for handle in ["coder", "reviewer"] {
        let session = session_of(&tmp, handle);
        let slot = slot_for(&tmp, &board, handle);
        json_of(
            persona(&tmp, handle, &session).args(["--json", "reply", "--thread", &slot, "done"]),
        );
        human(&tmp)
            .args(["session", "ended", &session])
            .assert()
            .success();
    }

    assert_eq!(
        admissions(&tmp),
        vec![
            ("pm".to_string(), "persona".to_string()),
            ("coder".to_string(), "channel".to_string()),
            ("reviewer".to_string(), "channel".to_string()),
            // The synthesizer: not a declared role, does not pass `trigger_role`, and STILL names
            // the coordinator that started it. This line is the one a reader of the source could not
            // have got right by reading `trigger_role`'s call sites.
            ("__synth__".to_string(), "channel".to_string()),
        ],
        "every line of the record, in order, with who admitted it — full log:\n{}",
        trigger_lines(&tmp).join("\n")
    );
}
