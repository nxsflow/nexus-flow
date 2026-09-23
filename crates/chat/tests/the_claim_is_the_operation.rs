//! **The working-tree claim is scoped to the OPERATION** (nxf 6j6v.8y6t, owner decision (a) of
//! 2026-08-23) — the measurement of 2026-08-22 re-run against the fix, plus the two properties that
//! decision is judged by.
//!
//! The chain is the one the owner's question named, and every step is probed from OUTSIDE with a
//! single exclusive commission, exactly as the original measurement was:
//!
//! ```text
//! #epic-implementation -> pm -> #coding -> coder -> #reviewing -> security-specialist -> coder
//! ```
//!
//! One probe per measurement, in a workspace driven to that step and no further — the probe is
//! itself `working_tree: exclusive`, so a second one in the same workspace would be queuing behind
//! the FIRST probe rather than behind the chain, and the reading would be about the wrong thing.
//!
//! **What the red baseline said, and what changes.** Steps 2 to 4 already behaved and must not move;
//! step 1 is where the finding sat. It stays a START, and that is the owner's decision rather than
//! an omission: way (a) holds nothing during planning ON THE FIRST PASS, because the claim arises at
//! the first `exclusive` step and there has not been one yet. What it does close is the same state
//! reached the SECOND time — between two coding rounds of one epic, where the claim exists and the
//! pm is planning inside it. `a_second_coding_round_of_the_same_epic_has_no_window_in_front_of_it`
//! is that measurement, and it is the one the finding's point 2 is about.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-08-22T09:00:00Z";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    write_declarations(&tmp, "");
    tmp
}

/// The human at the keyboard: no spawned-context stamps, so it stands in no thread and every
/// conversation it opens is the ROOT of an operation of its own.
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

/// [`human`] on a different wall clock — how "a rival asks once the window has passed" is driven.
fn at(tmp: &TempDir, when: &str) -> Command {
    let mut c = human(tmp);
    c.env("NXC_NOW", when);
    c
}

/// Every one-shot job this workspace has scheduled, in order — `DryTimer`'s record of what a real
/// clock would have been asked to run.
fn timer_lines(tmp: &TempDir) -> Vec<String> {
    std::fs::read_to_string(tmp.path().join("timer.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn persona(tmp: &TempDir, handle: &str, session: &str) -> Command {
    let mut c = human(tmp);
    c.env("NXC_ACTOR", handle).env("NXC_SESSION", session);
    c
}

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    serde_json::from_slice(&out.get_output().stdout).expect("valid json")
}

fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// The declarations of the owner's chain. `coder` and `doc-writer` need the working copy; `pm` and
/// `security-specialist` do not, and no CHANNEL declares anything about it — which is the point:
/// `#epic-implementation`'s only member is `pm`, so the one-hop derivation gives it no need of its
/// own, and it is protected here only because it is the ROOT OF THE OPERATION the coder works in.
///
/// `channels_extra` appends to the channel list, so one test can add a declared window without
/// giving every other test one.
fn write_declarations(tmp: &TempDir, channels_extra: &str) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["pm", "security-specialist"] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
        )
        .unwrap();
    }
    for handle in ["coder", "doc-writer"] {
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
            "- name: epic-implementation\n  members: [pm]\n\
             - name: coding\n  members: [coder]\n\
             - name: reviewing\n  members: [security-specialist]\n{channels_extra}"
        ),
    )
    .unwrap();
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
    let start = line.find(&marker).unwrap() + marker.len();
    line[start..]
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Whether the worker was ever handed a trigger for `handle` — a queued trigger is one the worker
/// never heard about, so this is what "did it START?" means here.
fn started(tmp: &TempDir, handle: &str) -> bool {
    trigger_lines(tmp)
        .iter()
        .any(|l| field(l, "role") == handle)
}

/// The newest session the engine started for `role`.
fn session_of(tmp: &TempDir, role: &str) -> String {
    trigger_lines(tmp)
        .iter()
        .filter(|l| field(l, "role") == role)
        .map(|l| field(l, "session"))
        .next_back()
        .unwrap_or_else(|| panic!("no trigger for {role}"))
}

/// The thread the supervisor opened for `handle` below `channel_thread`.
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
        .unwrap_or_else(|| panic!("no slot for {handle} under {channel_thread}"))
}

/// THE PROBE: one exclusive commission from outside the chain, issued by the human, so it is the
/// root of an operation of its own and can only inherit if the claim areas overlap — which they
/// never do here.
///
/// `true` when it STARTED, `false` when it was parked behind the chain.
fn an_exclusive_commission_from_outside_starts(tmp: &TempDir) -> bool {
    let receipt = json_of(human(tmp).args([
        "--json",
        "send",
        "--to",
        "doc-writer",
        "write the release notes",
        "--no-ref",
    ]));
    receipt["spawned"].as_bool().unwrap_or(false)
}

/// [`an_exclusive_commission_from_outside_starts`] on a given wall clock.
fn an_outside_commission_at(tmp: &TempDir, when: &str) -> Value {
    json_of(at(tmp, when).args([
        "--json",
        "send",
        "--to",
        "doc-writer",
        "write the release notes",
        "--no-ref",
    ]))
}

/// How far to drive the owner's chain before probing.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Step {
    /// Nothing has been asked at all — the baseline, read by `step_0` before any chain exists.
    #[allow(dead_code)]
    Nothing = 0,
    /// `#epic-implementation` is open and the `pm` is working. No exclusive step has been reached.
    PmPlanning = 1,
    /// `#coding` is open below it and the `coder` is working.
    CoderWorking = 2,
    /// `#reviewing` is open below the coder.
    ReviewRunning = 3,
    /// The `security-specialist` has commissioned the `coder` again out of the review.
    CoderRecommissioned = 4,
}

struct Chain {
    epic: String,
    pm_slot: String,
    coding: String,
}

/// Drive the chain to `step` and hand back its threads.
fn drive_to(tmp: &TempDir, step: Step) -> Chain {
    let epic = json_of(human(tmp).args([
        "--json",
        "send",
        "--to",
        "epic-implementation",
        "ship epic E1b",
        "--no-ref",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let pm_slot = member_thread_of(tmp, &epic, "pm");
    let mut coding = String::new();
    if step >= Step::CoderWorking {
        coding = json_of(persona(tmp, "pm", &session_of(tmp, "pm")).args([
            "--json",
            "send",
            "--to",
            "coding",
            "implement T4",
            "--no-ref",
        ]))["thread_id"]
            .as_str()
            .unwrap()
            .to_string();
    }
    if step >= Step::ReviewRunning {
        json_of(persona(tmp, "coder", &session_of(tmp, "coder")).args([
            "--json",
            "send",
            "--to",
            "reviewing",
            "review the first cut",
            "--no-ref",
        ]));
    }
    if step >= Step::CoderRecommissioned {
        json_of(
            persona(
                tmp,
                "security-specialist",
                &session_of(tmp, "security-specialist"),
            )
            .args([
                "--json",
                "send",
                "--to",
                "coder",
                "please fix the input validation",
                "--no-ref",
            ]),
        );
    }
    Chain {
        epic,
        pm_slot,
        coding,
    }
}

// ---- the measurement, step by step ------------------------------------------------------------

#[test]
fn step_0_nothing_is_running_so_an_outside_exclusive_commission_starts() {
    let tmp = workspace();
    assert!(
        an_exclusive_commission_from_outside_starts(&tmp),
        "an idle workspace hands the working copy to whoever asks first"
    );
}

#[test]
fn step_1_the_first_planning_phase_still_holds_nothing_and_that_is_the_decision() {
    // **The finding's own step 1, and it reads the same as before — deliberately.** Way (b) would
    // have changed it (look ahead over the declared chain and claim at once); the owner refused it
    // because it holds the working copy through pure thinking and serialises hardest. Way (a) keeps
    // the claim arising at the first `exclusive` step, so before there has been one there is nothing
    // to protect and nothing is protected.
    //
    // What the decision DOES close is the same picture one round later — see
    // `a_second_coding_round_of_the_same_epic_has_no_window_in_front_of_it`, where the pm is planning
    // INSIDE an operation that already holds.
    let tmp = workspace();
    drive_to(&tmp, Step::PmPlanning);
    assert!(
        an_exclusive_commission_from_outside_starts(&tmp),
        "no exclusive step has been reached, so the epic holds nothing yet"
    );
}

#[test]
fn step_2_the_coder_working_holds_the_copy_against_the_outside() {
    let tmp = workspace();
    drive_to(&tmp, Step::CoderWorking);
    assert!(
        !an_exclusive_commission_from_outside_starts(&tmp),
        "the first exclusive step took the claim; an outside chain waits"
    );
    assert!(!started(&tmp, "doc-writer"));
}

#[test]
fn step_3_the_nested_review_round_does_not_drop_the_claim() {
    let tmp = workspace();
    drive_to(&tmp, Step::ReviewRunning);
    assert!(
        !an_exclusive_commission_from_outside_starts(&tmp),
        "the claim carries through the nested round, exactly as it did before this item"
    );
}

#[test]
fn step_4_the_review_recommissioning_the_coder_is_the_same_operation_and_runs() {
    let tmp = workspace();
    drive_to(&tmp, Step::CoderRecommissioned);
    assert!(
        started(&tmp, "coder"),
        "the coder is commissioned again from inside the held operation, so it inherits and runs"
    );
    assert!(
        !an_exclusive_commission_from_outside_starts(&tmp),
        "…while the outside is still held off"
    );
}

#[test]
fn the_claim_names_the_operations_root_and_covers_the_planning_thread_above_the_coding_board() {
    // The same fact as the store records it, so the four measurements above are anchored to a key
    // rather than only to an observed behaviour.
    let tmp = workspace();
    let c = drive_to(&tmp, Step::CoderWorking);
    let store = open_store(&tmp);
    assert_eq!(
        store.working_tree_holder(NOW).unwrap(),
        Some(format!("thread:{}", c.epic)),
        "the lease is held under the OPERATION's root, not under the channel that declared the need"
    );
    let area = store
        .work_scope_threads(&nexus_chat::working_tree::WorkScope::Thread(c.epic.clone()))
        .unwrap();
    for thread in [&c.epic, &c.pm_slot, &c.coding] {
        assert!(
            area.contains(thread),
            "the planning half of the operation is inside the claim area too: {thread}"
        );
    }
}

// ---- the property the decision is FOR ---------------------------------------------------------

#[test]
fn a_second_coding_round_of_the_same_epic_has_no_window_in_front_of_it() {
    // Point 2 of the finding: "die Operation ist zwischen zwei Runden nicht atomar". The claim used
    // to fall the moment the subtree under `#coding` was worked off, so a `pm` that then commissioned
    // a SECOND round of the same epic had a gap in front of it — to the caller, an interruption in
    // the middle of its own operation.
    //
    // Driven end to end: round one finishes, the outside probes, round two is commissioned.
    let tmp = workspace();
    let c = drive_to(&tmp, Step::CoderWorking);

    // Round one finishes. The set settles, `#coding` consolidates and the pm is resumed with the
    // answer — which is exactly the moment it decides whether to run another round.
    let coder_slot = member_thread_of(&tmp, &c.coding, "coder");
    json_of(persona(&tmp, "coder", &session_of(&tmp, "coder")).args([
        "--json",
        "reply",
        "--thread",
        &coder_slot,
        "T4 implemented",
    ]));

    assert!(
        !an_exclusive_commission_from_outside_starts(&tmp),
        "THE WINDOW: round one is over and the operation is not, so the copy is not free"
    );

    // Round two, from the same pm, in the same epic.
    let round_two = json_of(
        persona(&tmp, "pm", &session_of(&tmp, "pm"))
            .args(["--json", "send", "--to", "coding", "now T5", "--no-ref"]),
    );
    assert_eq!(
        round_two["queue_position"],
        Value::Null,
        "and the second round is not parked behind anything: it is the same claim — {round_two}"
    );
    assert!(
        !started(&tmp, "doc-writer"),
        "the outside chain never got in between the two rounds — triggers:\n{}",
        trigger_lines(&tmp).join("\n")
    );
}

#[test]
fn a_chain_with_no_exclusive_persona_anywhere_claims_nothing() {
    // The fourth acceptance point, unchanged by this item and asserted so it stays that way: moving
    // the anchor must not make an operation claim the working copy just for existing.
    let tmp = workspace();
    let epic = json_of(human(&tmp).args([
        "--json",
        "send",
        "--to",
        "epic-implementation",
        "just think about it",
        "--no-ref",
    ]))["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    let store = open_store(&tmp);
    assert_eq!(
        store.working_tree_holder(NOW).unwrap(),
        None,
        "nobody in this chain declared a need for the working copy, so nothing is claimed"
    );
    assert!(!epic.is_empty());
    drop(store);
    assert!(
        an_exclusive_commission_from_outside_starts(&tmp),
        "…and an outside exclusive commission is not held off by it"
    );
}

// ---- the bound: the operation's own windows, not a flat two hours ------------------------------

#[test]
fn an_operation_that_declares_no_window_falls_back_to_the_named_backstop() {
    // The answer to the owner's first question, at the end where it is observable: a step that
    // declares no timeout leaves the area with nothing to derive a bound from, so the lease runs to
    // `WORKING_TREE_LEASE_BOUND` — today's behaviour, kept for today's declarations.
    let tmp = workspace();
    drive_to(&tmp, Step::CoderWorking);
    let (_, expires) = open_store(&tmp)
        .working_tree_lease_row()
        .unwrap()
        .expect("a lease was taken");
    assert_eq!(
        expires, "2026-08-22T11:00:00Z",
        "NOW + the two-hour backstop, because nothing in this operation declared a window"
    );
}

/// A workspace whose channels carry the declared windows in `windows` — `(channel, timeout)` —
/// driven to the coder's step, i.e. with the claim taken and its expiry derived from those windows.
fn workspace_with_windows(windows: [(&str, &str); 3]) -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    write_declarations(&tmp, "");
    let members = |name: &str| match name {
        "epic-implementation" => "pm",
        "coding" => "coder",
        _ => "security-specialist",
    };
    let yaml: String = windows
        .iter()
        .map(|(name, timeout)| {
            format!(
                "- name: {name}\n  members: [{}]\n  timeout: {timeout}\n",
                members(name)
            )
        })
        .collect();
    std::fs::write(tmp.path().join(".nxs-personas").join("channels.yaml"), yaml).unwrap();
    drive_to(&tmp, Step::CoderWorking);
    tmp
}

/// The instant the lease of [`workspace_with_windows`] runs to.
fn lease_expiry_with_windows(windows: [(&str, &str); 3]) -> String {
    let tmp = workspace_with_windows(windows);
    open_store(&tmp)
        .working_tree_lease_row()
        .unwrap()
        .expect("a lease was taken")
        .1
}

#[test]
fn an_operation_that_declares_its_windows_bounds_its_own_lease_by_them() {
    // The owner's rule: *"Die 2-Stunden-Grenze ergibt keinen Sinn. Sie muss an den Timeouts der
    // einzelnen Aktionen hängen."* Both directions, because both matter and they fail differently.
    //
    // SHORTER than the backstop: an operation whose every step promises minutes is reclaimable in
    // minutes after a hard death instead of wedging this device for two hours. The bound is the
    // LATEST of the windows, not the soonest — the operation is not done until its outermost
    // obligation is.
    assert_eq!(
        lease_expiry_with_windows([
            ("epic-implementation", "45m"),
            ("coding", "30m"),
            ("reviewing", "20m"),
        ]),
        "2026-08-22T09:45:00Z",
        "the latest window the operation declared, which here is well inside the old flat bound"
    );

    // LONGER than the backstop: the residual `WORKING_TREE_LEASE_BOUND`'s own doc has been carrying
    // — a chain that legitimately runs past two hours used to lose the checkout under itself while
    // it worked. A six-hour epic now holds a six-hour lease, because that is what it declared.
    assert_eq!(
        lease_expiry_with_windows([
            ("epic-implementation", "6h"),
            ("coding", "30m"),
            ("reviewing", "20m"),
        ]),
        "2026-08-22T15:00:00Z",
        "a declared window past the old flat bound now holds the lease past it too"
    );
}

#[test]
fn a_parked_commission_is_woken_at_the_holders_derived_expiry_not_at_a_flat_two_hours() {
    // **What instant is actually ARMED, not just what the synchronous call answered** (review of
    // PR #376, Test Quality #3). The whole point of deriving the bound is that something comes back
    // for the queue at the right moment; a test that only reads "did it start?" cannot see a wrong
    // moment at all, and that blind spot is how the sweep's own re-arm sat two hours out while this
    // one sat a minute out for the identical case.
    //
    // **The windows are SECONDS since nxf 6j6v.xb24**, and the reason is this test's own subject.
    // Since that item, a wait behind a claim also comes back on the liveness cadence — a minute —
    // because the holder can die inside its bound; with the 45-minute window this used to declare,
    // the armed instant would be that cadence and the derived bound would be invisible here again.
    // A bound that falls INSIDE the minute is what makes the assertion below about the derivation
    // rather than about whichever clock happens to be sooner, and it now tells three candidates
    // apart instead of two: the derived bound (09:00:30), the flat backstop (11:00) and the cadence
    // (09:01).
    let tmp = workspace_with_windows([
        ("epic-implementation", "30s"),
        ("coding", "20s"),
        ("reviewing", "10s"),
    ]);
    let holder_expires = open_store(&tmp)
        .working_tree_lease_row()
        .unwrap()
        .expect("a lease was taken")
        .1;
    assert_eq!(holder_expires, "2026-08-22T09:00:30Z", "premise");

    let parked = an_outside_commission_at(&tmp, NOW);
    assert_eq!(parked["spawned"], false, "premise: it parked — {parked}");
    let parked_thread = parked["thread_id"].as_str().unwrap();

    let all = timer_lines(&tmp);
    let armed: Vec<&String> = all
        .iter()
        .filter(|l| l.contains(&format!("thread={parked_thread}")))
        .collect();
    assert!(
        !armed.is_empty(),
        "a parked commission arms the tick that will drain it — timer log:\n{}",
        all.join("\n")
    );
    assert!(
        armed
            .iter()
            .any(|l| l.contains(&format!("deadline={holder_expires}"))),
        "…and it is armed at the HOLDER's own derived expiry (09:00:30), not at a flat two hours \
         (11:00) and not at the liveness cadence (09:01): {armed:?}"
    );
}

#[test]
fn a_window_that_has_already_passed_frees_the_copy_and_one_that_has_not_does_not() {
    // **The derived bound actually governs the acquire, in both directions** (review of PR #376,
    // Test Quality #2 — no test drove an already-expired declared window). The operation says its
    // steps have a minute; a rival that asks inside that minute waits, and one that asks after it
    // reclaims — which under a flat two-hour bound it could not have done for another 119 minutes.
    //
    // Nothing is alive here (the dry worker answers `false` for every session), which is exactly the
    // case the clock is FOR. The other one — expired but still writing — is
    // `a_rival_does_not_take_an_expired_lease_from_a_chain_whose_process_is_still_writing`, next
    // door, where a worker can actually say a process lives.
    let tmp = workspace_with_windows([
        ("epic-implementation", "1m"),
        ("coding", "1m"),
        ("reviewing", "1m"),
    ]);
    assert_eq!(
        open_store(&tmp)
            .working_tree_lease_row()
            .unwrap()
            .expect("a lease was taken")
            .1,
        "2026-08-22T09:01:00Z",
        "premise: the operation declared one minute, so that is what its lease runs to"
    );

    let holder = open_store(&tmp)
        .working_tree_holder("2026-08-22T09:00:30Z")
        .unwrap()
        .expect("the coder's operation holds the copy inside its window");

    let inside = an_outside_commission_at(&tmp, "2026-08-22T09:00:30Z");
    assert_eq!(
        inside["spawned"], false,
        "inside the declared window the claim stands: {inside}"
    );
    assert_eq!(
        inside["queued_behind"].as_str(),
        Some(holder.as_str()),
        "…and what it waits on is the coder's operation: {inside}"
    );
    let inside_thread = inside["thread_id"].as_str().unwrap().to_string();

    // Past the window, with nothing alive behind the claim, the copy IS reclaimed — under the old
    // flat bound neither of these would have moved for another 119 minutes. Where it GOES is nxf
    // 6j6v.yd4w: to the commission that was already standing in line, not to this newcomer. This
    // assertion used to read `spawned: true` for exactly that reason.
    let past = an_outside_commission_at(&tmp, "2026-08-22T09:01:30Z");
    assert_eq!(
        past["spawned"], false,
        "the newcomer does not overtake the one already waiting: {past}"
    );
    assert_eq!(
        past["queued_behind"].as_str(),
        Some(format!("thread:{inside_thread}").as_str()),
        "…it waits on THAT one — which is only possible because the dead claim's window had run \
         out and its copy moved on: {past}"
    );
    assert_eq!(
        open_store(&tmp)
            .working_tree_holder("2026-08-22T09:01:30Z")
            .unwrap(),
        Some(format!("thread:{inside_thread}")),
        "and the chain that waited holds the copy now"
    );
}
