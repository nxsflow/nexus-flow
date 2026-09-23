//! **A persona started under a changed declaration says so, and every session records which
//! version it ran under** (nxf 6j6v.pkw9).
//!
//! The item's three acceptance points, one section each:
//!
//! 1. a session holds on to the version its prompt was built from — `declarationHash` in its
//!    `spec.json`;
//! 2. when a role's declaration changes between two spawns, the CALLER is told — a
//!    `declaration_changed` entry in the receipt's `warnings`;
//! 3. the guide says the folder belongs in version control — prose, held by `guide_examples.rs`
//!    and by reading, not by this file.
//!
//! **Driven black-box through the real `nxc`**, because the claim is about what a caller sees. The
//! measured incident behind the item was found by holding a stored `spec.json` against the file on
//! disk BY HAND, and a library-level assertion on a struct field would re-state the plumbing
//! without ever showing that the hand-work is now unnecessary. What the dry worker records
//! (`declaration=<short>`) and what the receipt carries (`declaration.hash`, full) are asserted
//! against EACH OTHER, so the funnel, the worker seam and the receipt are pinned as one chain
//! rather than three separately-true facts.
//!
//! **What is deliberately NOT asserted anywhere here: a refusal.** The item's own words — *"als
//! Warnklasse im Empfangsschein, nicht als Refus. Eine Aenderung ist meistens gewollt; eine
//! unbemerkte nie."* Every case below therefore also asserts `spawned: true` and a zero exit, and
//! `the_warning_is_not_a_refusal` makes that the whole point of one test rather than a rider on the
//! others.

use assert_cmd::Command;
use nexus_chat::workspace::{chat_config, setup};
use serde_json::Value;
use tempfile::TempDir;

/// A chat workspace with two declared personas. Two, because the record is kept PER ROLE and a
/// single-role fixture could not tell that apart from one kept per workspace.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    write_role(
        &tmp,
        "coder",
        "system_prompt: You are coder.\ntools: [Bash]\n",
    );
    write_role(
        &tmp,
        "reviewer",
        "system_prompt: You are reviewer.\ntools: [Bash]\n",
    );
    tmp
}

/// Write `.nxs-personas/<handle>.yaml`. `body` is everything after the `handle:` line, so a test
/// changes a declaration by calling this again with different text — which is exactly what a branch
/// switch does to the folder.
fn write_role(tmp: &TempDir, handle: &str, body: &str) {
    let dir = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{handle}.yaml")),
        format!("handle: {handle}\n{body}"),
    )
    .unwrap();
}

/// The human at the keyboard, at a stated instant. `now` varies per call because the finding names
/// WHEN the previous spawn ran, and a fixture with one frozen clock could not show that it does.
fn human(tmp: &TempDir, now: &str) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", now)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
    c
}

/// `nxc send --to <handle> --json`, asserted to succeed, as a parsed receipt.
///
/// The success assertion is not incidental: a `declaration_changed` warning must NOT make the verb
/// exit non-zero (see the module header), so every call in this file that produces one also proves
/// it did not turn the send red.
fn send_to(tmp: &TempDir, now: &str, handle: &str) -> Value {
    let out = human(tmp, now)
        .args(["send", "--to", handle, "--json", "do the thing", "--no-ref"])
        .assert()
        .success();
    serde_json::from_slice(&out.get_output().stdout).expect("a send receipt is valid json")
}

/// `nxc send --to <handle> --json` on a worker that starts NOTHING, as a parsed receipt.
///
/// `NXC_WORKER` is resolved on first USE, so an unknown value is not a startup refusal: the message
/// is posted, the obligation is registered, and the SPAWN is what fails — the injection
/// `failed_consequences.rs` uses to break a consequence at the real path.
///
/// It asserts FAILURE, and the contrast is the point: `step_skipped` turns the verb red because
/// something that was supposed to happen did not, while `declaration_changed` leaves it green
/// because everything did.
fn send_to_with_no_worker(tmp: &TempDir, now: &str, handle: &str) -> Value {
    let out = human(tmp, now)
        .env("NXC_WORKER", "not-a-real-worker")
        .args(["send", "--to", handle, "--json", "do the thing", "--no-ref"])
        .assert()
        .failure();
    serde_json::from_slice(&out.get_output().stdout).expect("a send receipt is valid json")
}

/// The `declaration_changed` entries of a receipt — normally zero or one, and asserted as a LIST so
/// a second one could never hide behind the first.
fn declaration_findings(receipt: &Value) -> Vec<&Value> {
    receipt["warnings"]
        .as_array()
        .expect("`warnings` is always present, even empty")
        .iter()
        .filter(|w| w["class"] == "declaration_changed")
        .collect()
}

/// Every `declaration=` field the dry worker recorded, in trigger order — the short version stamp
/// the funnel handed to the worker seam.
fn declarations_at_the_worker(tmp: &TempDir) -> Vec<String> {
    std::fs::read_to_string(tmp.path().join("dry.log"))
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("trigger role="))
        .map(|l| {
            let start = l.find("declaration=").expect("every trigger stamps one") + 12;
            l[start..]
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

// ---- 1. the session records the version it ran under ------------------------------------------

#[test]
fn the_spec_the_sidecar_reads_carries_the_version_the_prompt_was_built_from() {
    // The engine's half of acceptance point 1 ends at the spec file, exactly as the `grantedTools`
    // key's does (`an_obligation_comes_with_its_means.rs`). `SidecarWorker` writes it BEFORE the
    // spawn, so a sidecar path that cannot exist is enough to read the file back — and is what
    // keeps this test from starting a `node`.
    use nexus_chat::worker::{Coordinator, RoleSpec, TriggerRequest, Worker};

    let tmp = TempDir::new().unwrap();
    let worker = nexus_chat::worker::SidecarWorker {
        sidecar: tmp.path().join("no-such-sidecar.mjs"),
        cwd: tmp.path().to_path_buf(),
    };
    let _ = worker.trigger(TriggerRequest {
        internal_session: "s-version".to_string(),
        resume_real: None,
        message: "go".to_string(),
        role: RoleSpec {
            handle: "coder".to_string(),
            system_prompt: "You are coder.".to_string(),
            use_claude_code_preset: false,
            tools: None,
            granted_tools: Vec::new(),
            permissions: None,
            model: None,
            declaration_hash: Some("a".repeat(64)),
        },
        reply_thread: None,
        coordinator: Coordinator::Persona,
        terms: Default::default(),
        env: Default::default(),
    });

    let spec: Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.path().join(".nxs/agent-logs/s-version.spec.json"))
            .expect("the spec is written before the spawn"),
    )
    .expect("valid json");
    assert_eq!(
        spec["declarationHash"],
        Value::String("a".repeat(64)),
        "{spec}"
    );
}

#[test]
fn the_version_stamp_follows_the_declaration_and_not_the_file() {
    // The projection claim, which is the reason this hashes a `RoleDecl` and not the bytes on disk
    // (see `declaration_version`'s header): re-writing the same declaration with different
    // formatting and a comment is NOT a change, and would be one if the file were hashed. Getting
    // this wrong would make the warning fire on every re-checkout of an unchanged folder, which is
    // the fastest way to teach a reader to ignore it.
    let tmp = workspace();
    send_to(&tmp, "2026-09-03T10:00:00Z", "coder");
    write_role(
        &tmp,
        "coder",
        "# reformatted, and a comment added\nsystem_prompt: \"You are coder.\"\ntools:\n  - Bash\n",
    );
    let second = send_to(&tmp, "2026-09-03T11:00:00Z", "coder");

    assert!(
        declaration_findings(&second).is_empty(),
        "the same declaration, written differently, is not a change: {second}"
    );
    let stamps = declarations_at_the_worker(&tmp);
    assert_eq!(stamps.len(), 2, "two spawns: {stamps:?}");
    assert_eq!(
        stamps[0], stamps[1],
        "…and both ran under one version: {stamps:?}"
    );
}

// ---- 2. a change between two spawns reaches the caller ----------------------------------------

#[test]
fn a_declaration_changed_between_two_spawns_is_named_on_the_second_receipt() {
    // THE ACCEPTANCE, and the item's own scenario in miniature: a rule is edited between two runs
    // of the same role — in the proving ground by a branch switch that rolled one BACK — and the
    // caller of the second run finds out from the receipt instead of by comparing files by hand.
    let tmp = workspace();
    let first = send_to(&tmp, "2026-09-03T10:00:00Z", "coder");
    assert!(
        declaration_findings(&first).is_empty(),
        "a role that never ran here has nothing to be compared against: {first}"
    );

    write_role(
        &tmp,
        "coder",
        "system_prompt: |\n  You are coder.\n  Never wait for something you commissioned.\ntools: [Bash]\n",
    );
    let second = send_to(&tmp, "2026-09-03T11:30:00Z", "coder");

    let found = declaration_findings(&second);
    assert_eq!(
        found.len(),
        1,
        "exactly one, and only on the second: {second}"
    );
    let finding = found[0];
    let change = &finding["declaration"];
    assert_eq!(change["role"], "coder", "{finding}");
    assert_eq!(
        change["previous_seen"], "2026-09-03T10:00:00Z",
        "the finding dates the run that used the OTHER version: {finding}"
    );
    assert_ne!(
        change["hash"], change["previous_hash"],
        "the two versions are what makes it a change: {finding}"
    );
    assert_eq!(
        finding["session"], second["session"],
        "it names the session that is running under the new version: {finding}"
    );

    // The chain, end to end: what the funnel handed the worker is the short form of what the
    // receipt reports, on BOTH sides of the change. Asserting the two against each other is what
    // makes this a claim about one pipeline rather than two independently plausible values.
    let stamps = declarations_at_the_worker(&tmp);
    assert_eq!(stamps.len(), 2, "{stamps:?}");
    assert_eq!(
        change["previous_hash"].as_str().unwrap()[..12],
        stamps[0],
        "the previous version on the receipt IS what the first spawn ran under"
    );
    assert_eq!(
        change["hash"].as_str().unwrap()[..12],
        stamps[1],
        "and the current one IS what this spawn was handed"
    );
}

#[test]
fn a_role_is_compared_only_against_its_own_previous_version() {
    // The record is kept PER ROLE, and this is what says so: editing `reviewer` between two runs of
    // `coder` must say nothing about `coder`. A record kept per workspace — the obvious cheaper
    // shape — would fire here, and would then fire on every spawn in any workspace whose folder is
    // edited at all, which is a warning nobody would read twice.
    let tmp = workspace();
    send_to(&tmp, "2026-09-03T10:00:00Z", "coder");
    write_role(
        &tmp,
        "reviewer",
        "system_prompt: You are reviewer, and stricter now.\ntools: [Bash]\n",
    );
    let coder_again = send_to(&tmp, "2026-09-03T11:00:00Z", "coder");

    assert!(
        declaration_findings(&coder_again).is_empty(),
        "coder's own declaration did not move: {coder_again}"
    );

    // …and the edit is not LOST either: it is reported the next time the role it belongs to runs.
    let reviewer = send_to(&tmp, "2026-09-03T12:00:00Z", "reviewer");
    assert!(
        declaration_findings(&reviewer).is_empty(),
        "reviewer's FIRST run here has nothing to compare against either: {reviewer}"
    );
}

#[test]
fn an_unchanged_declaration_is_reported_on_no_spawn_however_many_there_are() {
    // The steady state, and the property that decides whether any of this is usable: a workspace
    // whose folder is not moving must be silent. A finding that rides every send is a finding a
    // reader learns to skip, and it would drown the classes `warnings` already carries.
    let tmp = workspace();
    for (i, now) in [
        "2026-09-03T10:00:00Z",
        "2026-09-03T10:05:00Z",
        "2026-09-03T10:10:00Z",
    ]
    .into_iter()
    .enumerate()
    {
        let receipt = send_to(&tmp, now, "coder");
        assert!(
            declaration_findings(&receipt).is_empty(),
            "send {i} reported a change in a folder nobody touched: {receipt}"
        );
    }
}

#[test]
fn a_spawn_the_worker_refused_does_not_consume_the_change() {
    // The row means *the version the last spawn of this role RAN under*, and this is what says so:
    // a trigger the worker refused started nothing, so it must not advance the record. If it did,
    // the very next spawn would compare against a version nothing ever ran — the change would be
    // gone, silently, which is precisely the class this whole item exists to end.
    //
    // It is also the shape a real workspace meets: a persona edited, addressed once while the
    // sidecar could not start, then addressed again after the machine was fixed.
    let tmp = workspace();
    send_to(&tmp, "2026-09-03T10:00:00Z", "coder");
    write_role(
        &tmp,
        "coder",
        "system_prompt: You are coder, and here is the rule that went missing.\ntools: [Bash]\n",
    );

    let refused = send_to_with_no_worker(&tmp, "2026-09-03T11:00:00Z", "coder");
    assert_eq!(
        refused["spawned"], false,
        "the premise: nothing started here: {refused}"
    );
    assert!(
        declaration_findings(&refused).is_empty(),
        "and nothing ran under the new version, so there is nothing to report yet: {refused}"
    );

    let started = send_to(&tmp, "2026-09-03T12:00:00Z", "coder");
    let found = declaration_findings(&started);
    assert_eq!(
        found.len(),
        1,
        "the change survived the refused spawn and is named on the one that ran: {started}"
    );
    assert_eq!(
        found[0]["declaration"]["previous_seen"], "2026-09-03T10:00:00Z",
        "and it is compared against the last spawn that actually RAN, not the refused one: {started}"
    );
}

#[test]
fn the_warning_is_not_a_refusal() {
    // The item is explicit that this reports and does not refuse, and there are two ways to break
    // that: not starting the session, and starting it while telling a script the call failed. Both
    // are checked here — `send_to`'s own `.success()` is the second, and `spawned` is the first.
    let tmp = workspace();
    send_to(&tmp, "2026-09-03T10:00:00Z", "coder");
    write_role(
        &tmp,
        "coder",
        "system_prompt: You are coder, with a new rule.\ntools: [Bash]\n",
    );
    let second = send_to(&tmp, "2026-09-03T11:00:00Z", "coder");

    assert_eq!(
        declaration_findings(&second).len(),
        1,
        "the premise: this send DID report the change: {second}"
    );
    assert_eq!(
        second["spawned"], true,
        "and started the session anyway — reporting is not refusing: {second}"
    );
    assert_eq!(
        declarations_at_the_worker(&tmp).len(),
        2,
        "both sends reached the worker"
    );
}

#[test]
fn a_recorded_change_is_reported_once_and_not_on_every_spawn_after_it() {
    // **The cries-wolf class, and it is not hypothetical** (review of PR #425, Test Quality #1).
    // The reviewer found this gap by MUTATION: swapping `note_spawn_declaration`'s upsert for an
    // `INSERT OR IGNORE` freezes a role's recorded version at whatever it first saw, so every spawn
    // after one real edit keeps comparing against a stale hash and warns forever — and all seven
    // tests stayed green, because every one of them stopped after the spawn that warns.
    //
    // A warning that fires on every call is a warning nobody reads, which would quietly undo the
    // whole item. So the assertion that matters is the THIRD spawn: same declaration as the second,
    // and silent.
    let tmp = workspace();
    send_to(&tmp, "2026-09-03T10:00:00Z", "coder");
    write_role(
        &tmp,
        "coder",
        "system_prompt: You are coder, under a new rule.\ntools: [Bash]\n",
    );
    let changed = send_to(&tmp, "2026-09-03T11:00:00Z", "coder");
    assert_eq!(
        declaration_findings(&changed).len(),
        1,
        "the premise: the second spawn reported the change: {changed}"
    );

    // Nothing edited in between — the declaration on disk is the one the spawn above ran under.
    let settled = send_to(&tmp, "2026-09-03T12:00:00Z", "coder");
    assert!(
        declaration_findings(&settled).is_empty(),
        "the change was already reported and recorded; saying it again on every later spawn is how \
         a warning stops being read: {settled}"
    );
    let fourth = send_to(&tmp, "2026-09-03T13:00:00Z", "coder");
    assert!(
        declaration_findings(&fourth).is_empty(),
        "…and it stays quiet, rather than being quiet exactly once: {fourth}"
    );

    let stamps = declarations_at_the_worker(&tmp);
    assert_eq!(stamps.len(), 4, "{stamps:?}");
    assert_eq!(
        stamps[1], stamps[3],
        "the last three spawns ran under one version, which is why only one of them reported: \
         {stamps:?}"
    );
}

#[test]
fn the_version_stamp_survives_a_hash_that_is_not_hex() {
    // `declaration_version::short` cuts on a CHARACTER boundary rather than at byte 12, because one
    // of its callers renders a value a HOST supplied on a hand-built request — and `&s[..12]` in
    // the worker seam would be a panic over a display detail (review of PR #425, Test Quality #5).
    // Every value this crate produces is hex, where the two are identical, so this is the only way
    // to reach the guard at all.
    use nexus_chat::worker::{Coordinator, DryWorker, RoleSpec, TriggerRequest, Worker};

    let tmp = TempDir::new().unwrap();
    let log = tmp.path().join("dry.log");
    DryWorker {
        log: Some(log.clone()),
    }
    .trigger(TriggerRequest {
        internal_session: "s-emoji".to_string(),
        resume_real: None,
        message: "go".to_string(),
        role: RoleSpec {
            handle: "coder".to_string(),
            system_prompt: "You are coder.".to_string(),
            use_claude_code_preset: false,
            tools: None,
            granted_tools: Vec::new(),
            permissions: None,
            model: None,
            // Multi-byte from the very first character, so a byte-index cut lands mid-character.
            declaration_hash: Some("äöüßéèêëàâîïôûùç".to_string()),
        },
        reply_thread: None,
        coordinator: Coordinator::Persona,
        terms: Default::default(),
        env: Default::default(),
    })
    .expect("the worker records the trigger rather than panicking on the stamp");

    let got = std::fs::read_to_string(&log).unwrap();
    assert!(
        got.contains("declaration=äöüßéèêëàâîï "),
        "twelve CHARACTERS of it, and the next key still starts where a reader expects: {got}"
    );
}

// ---- 3. the paths that are NOT a `send --to` receipt --------------------------------------------
//
// PR #425's first cut surfaced the finding on the direct-commission receipt and nowhere else, and
// called `fire_queued_trigger` "the one spawn path with no receipt to put it on". The independent
// review counted five (Code Quality #1, Integrity & Robustness #1), and the two below are the ones
// structurally closest to the measured incident: a chain hop resumed under rules that moved, and a
// spawn started by an UNATTENDED sweep, where a stderr breadcrumb reaches nobody at all.

#[test]
fn a_wake_that_resumes_an_older_operation_says_which_version_it_runs_under() {
    // **A wake is a spawn, and until PR #425's review it was a spawn nobody could hear** (Code
    // Quality #1). `route_completion` returns a `CompletionWake` whose `warnings` field is already
    // threaded onto the reply receipt — and its success arm hardcoded an empty vector, so the one
    // wake that closes a commission reported nothing whatever it started.
    //
    // The scenario is TWO operations, and that is not incidental: inside one operation
    // `declaration_freeze` guarantees there is no drift to report, which is its whole job. What a
    // wake CAN say is that the version it resumes under is not the version the last spawn of that
    // role used — here because a newer operation has since started the same role under an edited
    // file, and this one is still bound to the older catalogue. That session really is running
    // under yesterday's rules, and this receipt is where a caller finds out.
    let tmp = workspace();
    write_role(
        &tmp,
        "pm",
        "system_prompt: You are the PM.\ntools: [Bash]\n",
    );

    // Operation A: the PM is commissioned, and commissions the coder in turn.
    let op_a = send_to(&tmp, "2026-09-03T10:00:00Z", "pm");
    let pm_session = op_a["session"].as_str().expect("a pm session").to_string();
    let out = human(&tmp, "2026-09-03T10:05:00Z")
        .env("NXC_ACTOR", "pm")
        .env("NXC_SESSION", &pm_session)
        .args(["send", "--to", "coder", "--json", "build it", "--no-ref"])
        .assert()
        .success();
    let handed_over: Value =
        serde_json::from_slice(&out.get_output().stdout).expect("a send receipt is valid json");
    let coder_thread = handed_over["thread_id"]
        .as_str()
        .expect("a coder thread")
        .to_string();
    let coder_session = handed_over["session"]
        .as_str()
        .expect("a coder session")
        .to_string();

    // The PM's rules move, exactly as a branch switch would move them…
    write_role(
        &tmp,
        "pm",
        "system_prompt: |\n  You are the PM.\n  Never wait for something you commissioned.\ntools: [Bash]\n",
    );
    // …and a NEW operation starts the PM under them, which is what records the new version.
    let op_b = send_to(&tmp, "2026-09-03T10:30:00Z", "pm");
    assert_eq!(
        declaration_findings(&op_b).len(),
        1,
        "the premise: the fresh commission ran under the edited file and said so: {op_b}"
    );

    // Now operation A finishes. Its PM is resumed under the catalogue A was frozen with — the OLD
    // one — which is a different version from the one the last spawn of this role used.
    let reply = human(&tmp, "2026-09-03T11:00:00Z")
        .env("NXC_ACTOR", "coder")
        .env("NXC_SESSION", &coder_session)
        .args(["--json", "reply", "--thread", &coder_thread, "done"])
        .assert()
        .success();
    let reply: Value =
        serde_json::from_slice(&reply.get_output().stdout).expect("a reply receipt is valid json");

    assert_eq!(
        reply["woke"].as_str(),
        Some(pm_session.as_str()),
        "the premise: this reply really did wake the PM of operation A: {reply}"
    );
    let found = declaration_findings(&reply);
    assert_eq!(
        found.len(),
        1,
        "and the receipt of the reply that woke it says which version it woke under: {reply}"
    );
    assert_eq!(found[0]["declaration"]["role"], "pm", "{reply}");
    assert_eq!(
        found[0]["session"], pm_session,
        "…naming the session that was resumed, not the one that replied: {reply}"
    );
}

#[test]
fn a_trigger_started_by_the_sweep_reports_on_the_tick_receipt() {
    // **The unattended path** (review of PR #425, Integrity & Robustness #1). A commission that had
    // to wait for the working copy is started by whoever gives the lease up, not by the caller that
    // made it — and this test drives exactly that through `tick`'s sweep, which the background
    // service performs with no terminal anywhere (the manual verb this test used to drive,
    // `nxc release`, is gone: nxf 6j6v.b9nf). The first cut only `eprintln!`d here, which in that
    // context is silence.
    let tmp = workspace();
    write_role(
        &tmp,
        "coder",
        "system_prompt: You are coder.\ntools: [Bash]\nworking_tree: exclusive\n",
    );

    // (1) takes the working copy and runs under the version on disk.
    let holder = send_to(&tmp, "2026-09-03T10:00:00Z", "coder");
    let holder_thread = holder["thread_id"].as_str().expect("a holding thread");
    assert_eq!(holder["spawned"], true, "{holder}");

    // (2) the rules move…
    write_role(
        &tmp,
        "coder",
        "system_prompt: You are coder, and here is the rule that went missing.\n\
         tools: [Bash]\nworking_tree: exclusive\n",
    );

    // (3) …and a second commission is PARKED behind the first, so nothing starts and nothing is
    // recorded. It freezes the new version for its own operation, which is what it will be
    // composed from when it is finally fired.
    let parked = send_to(&tmp, "2026-09-03T10:10:00Z", "coder");
    assert_eq!(
        parked["spawned"], false,
        "the premise: this one is waiting for the working copy: {parked}"
    );
    assert!(
        declaration_findings(&parked).is_empty(),
        "a parked commission started nothing, so it has nothing to report yet: {parked}"
    );

    // (4) the sweep fires it — past the holder's two-hour bound (`coder` declares no `timeout:`,
    // so the flat backstop applies), with the dry worker answering `false` for every session, which
    // is exactly "nothing is running behind it" — and THIS receipt is the only one a caller ever
    // sees for it. `--thread` only picks which board's own completion this tick ALSO re-checks; the
    // working-tree sweep at its front runs unconditionally first, whichever thread is named.
    let ticked = human(&tmp, "2026-09-03T12:00:01Z")
        .args(["--json", "tick", "--thread", holder_thread])
        .assert()
        .success();
    let ticked: Value =
        serde_json::from_slice(&ticked.get_output().stdout).expect("a tick receipt is valid json");
    let swept = &ticked["working_tree"];
    assert!(
        !swept.is_null(),
        "the premise: the lease was past its bound with nothing running, so the sweep must report \
         it: {ticked}"
    );

    assert_eq!(
        swept["started"].as_array().map(Vec::len),
        Some(1),
        "the premise: the parked commission really was started: {ticked}"
    );
    let found = declaration_findings(swept);
    assert_eq!(
        found.len(),
        1,
        "and the session it started runs under a version the last spawn did not: {ticked}"
    );
    assert_eq!(found[0]["declaration"]["role"], "coder", "{ticked}");
    assert_ne!(
        found[0]["declaration"]["hash"], found[0]["declaration"]["previous_hash"],
        "{ticked}"
    );
}
