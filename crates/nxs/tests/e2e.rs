//! P3 slice-proof end-to-end (nexus-flow-aye.30 DoD): the real `nxf`/`nxm`/`nxs` binaries, wired
//! through the assembler + fan-out, prove the umbrella's central guarantees:
//!
//!   1. flow-only `nxf init` wires EXACTLY ONE SessionStart hook → `nxs prime`.
//!   2. `nxs prime` delivers flow's prime (and only flow's, while flow is the only module).
//!   3. `nxm init` afterwards re-assembles without a second hook; both blocks are present, and
//!      `nxs prime` then fans out to BOTH modules.
//!
//! Requires the sibling binaries built (a `cargo test`/workspace build provides them — the
//! assembler + fan-out resolve `nxf`/`nxm` next to `nxs` in the target dir). The fuller E2E suite
//! is S8; this is the slice-close proof.

use assert_cmd::Command;
use nxs_test_support::PinHome;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

fn bin(name: &str, dir: &Path) -> Command {
    let mut c = Command::cargo_bin(name).unwrap_or_else(|_| panic!("{name} binary built"));
    c.current_dir(dir);
    // Never this machine's real `~` (nxf 6j6v.npf9). `nxs init` WRITES to the background
    // service's workspace registry now, so a suite that runs it under the developer's own home
    // adds one dead `TempDir` entry per case to `~/.nexusflow/workspaces.toml` — rebuilding, a
    // test run at a time, exactly the 98 stale entries that registry was once measured at. This
    // suite builds its own command rather than going through `nxs_test_support::cargo_bin` (it
    // inherits none of its defaults), so it says the home part itself.
    c.pin_home(nxs_test_support::pinned_home());
    // Never this machine's real scheduler (nxf 6j6v.74c0). Nothing here opens a channel with a
    // declared `timeout:`, so nothing here would schedule anything today — but "today" is the
    // whole argument against leaving it unsaid: on macOS an unset `NXC_TIMER` means the real
    // `launchd` backend, and the day a case here grows one it would bootstrap a one-shot agent
    // into the developer's login session, due to fire in a `TempDir` that is already gone.
    // `crates/chat/tests/no_test_arms_the_real_scheduler.rs` is what asks for this line.
    c.env("NXC_TIMER", "dry");
    // …and never a real model for a thread name (nxf 6j6v.e76c), for the reason one line up: this
    // suite builds its own command instead of going through `cargo_bin`, which sets both.
    c.env("NXC_NAMER", "dry");
    c
}

/// Every SessionStart hook command in `.claude/settings.json`.
fn session_start_hooks(dir: &Path) -> Vec<String> {
    let raw = std::fs::read_to_string(dir.join(".claude/settings.json")).expect("settings.json");
    let settings: Value = serde_json::from_str(&raw).unwrap();
    settings["hooks"]["SessionStart"]
        .as_array()
        .expect("SessionStart array")
        .iter()
        .flat_map(|g| g["hooks"].as_array().unwrap().clone())
        .filter_map(|h| h["command"].as_str().map(String::from))
        .collect()
}

/// The SessionStart commands a workspace with these module binaries should carry — one per module,
/// with the `|| cat NEXUS_MEMORY.md` fallback on exactly one of them (nxf n2m6 + a2a1, Ruling R1).
/// Built through the assembler's own seam rather than spelled out, so a change to the shape reddens
/// the assembler's unit tests rather than silently rewriting what this file expects.
/// Assert the workspace carries exactly one hook per named module, and that exactly one of them
/// carries the fallback.
///
/// **Compared IN ORDER since nxf 6j6v.shwz.** It used to be compared as a set, deliberately:
///
/// > The order entries end up in depends on the order the modules were initialized — a workspace
/// > built by `nxf init` then `nxm init` lists them differently from one built the other way
/// > round, and neither is roster order — and no behaviour depends on it: the host does not
/// > guarantee the order it runs SessionStart hooks in (a foreign hook was observed landing
/// > between two of ours), which is why no block may refer to another. Pinning file order here
/// > would be pinning an accident.
///
/// Every word of that was true of the code as it stood, and the last sentence is why this test
/// could not have caught the defect: the order WAS an accident, so pinning it would have pinned
/// the accident. The owner's answer was not to pin what varied but to stop it varying — the hooks
/// are written in the same order `nxs prime` fans out in, which is roster order, which is where
/// memory going last carries a reason (it is the one block a session can fetch back). So the
/// file order is a decided thing now, and pinning it pins a decision.
///
/// The host still does not guarantee execution order, and no block may refer to another. That is
/// unchanged and is a different claim from "the file we write is deterministic".
fn assert_wired(dir: &Path, binaries: &[&str]) {
    let actual = session_start_hooks(dir);
    assert_eq!(
        actual,
        nxs_init::assembler::hook_commands(binaries),
        "one SessionStart hook per active module, in the order `nxs prime` fans out"
    );
    let carriers: Vec<String> = actual
        .iter()
        .filter(|c| c.contains("cat NEXUS_MEMORY.md"))
        .cloned()
        .collect();
    assert_eq!(
        carriers,
        binaries
            .iter()
            .filter(|b| **b == "nxm")
            .map(|b| format!("{b} prime{}", nxs_init::assembler::HOOK_FALLBACK))
            .collect::<Vec<_>>(),
        "the fallback rides memory's entry and no other — and none at all without memory: \
         {actual:?}"
    );
}

#[test]
fn flow_only_then_memory_assembles_one_hook_per_module_and_fans_out() {
    let tmp = TempDir::new().unwrap();

    // 1. flow-only init → one SessionStart hook, flow's own.
    //
    // **Superseded 2026-08-28 (nxf n2m6 + a2a1):** this used to read "exactly one SessionStart
    // hook → `nxs prime` (NOT `nxf prime`)", which was right while one hook carried the composed
    // fan-out against one budget. The host truncates per hook OUTPUT, so there is one entry per
    // active module now and `nxf prime` is exactly what flow-only wires.
    bin("nxf", tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    assert_wired(tmp.path(), &["nxf"]);
    // nexus-flow-0lj.2: ONE nxs-owned discovery pointer naming the active tools — not per-module
    // blocks. flow-only → it names flow, not memory.
    let agents = std::fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
    assert!(
        agents.contains("<!-- BEGIN NEXUS "),
        "the single pointer is present"
    );
    assert!(agents.contains("(flow)"), "names flow: {agents}");
    assert!(
        !agents.contains("(flow, memory)") && !agents.contains("BEGIN NEXUS-"),
        "no memory yet, no per-module blocks: {agents}"
    );

    // 2. `nxs prime` delivers flow's prime — and only flow's, while flow is the only module.
    let prime_flow = bin("nxs", tmp.path()).arg("prime").assert().success();
    let out = String::from_utf8_lossy(&prime_flow.get_output().stdout).to_string();
    assert!(
        out.contains("nexus-flow"),
        "fan-out delivers flow's prime: {out}"
    );
    assert!(
        !out.contains("nexus-memory"),
        "memory absent until nxm init: {out}"
    );

    // 3. `nxm init` re-assembles: memory BRINGS ITS OWN entry, flow's is not duplicated, and the
    //    fallback lands on memory's entry (nxf 6j6v.1k6y) rather than being handed out twice.
    bin("nxm", tmp.path()).arg("init").assert().success();
    assert_wired(tmp.path(), &["nxf", "nxm"]);
    let agents = std::fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
    assert_eq!(
        agents.matches("<!-- BEGIN NEXUS ").count(),
        1,
        "still exactly one pointer (refreshed, not duplicated)"
    );
    assert!(
        agents.contains("(flow, memory)"),
        "the one pointer now names both, flow before memory (roster order — the same sequence \
         `nxs prime` fans out in, since nxf 6j6v.shwz): {agents}"
    );
    assert!(
        !agents.contains("BEGIN NEXUS-"),
        "no per-module blocks: {agents}"
    );

    // …and `nxs prime` now fans out to BOTH modules.
    let prime_both = bin("nxs", tmp.path()).arg("prime").assert().success();
    let out = String::from_utf8_lossy(&prime_both.get_output().stdout).to_string();
    assert!(out.contains("nexus-flow"), "flow still primed: {out}");
    assert!(out.contains("nexus-memory"), "memory now primed too: {out}");
}

#[test]
fn nxs_prime_json_nests_each_active_modules_record_with_one_shared_now() {
    let tmp = TempDir::new().unwrap();
    bin("nxf", tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    bin("nxm", tmp.path()).arg("init").assert().success();

    let out = bin("nxs", tmp.path())
        .args(["prime", "--json"])
        .env("NXS_NOW", "2026-06-22T12:00:00Z")
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).expect("nxs prime --json");
    assert_eq!(v["now"], "2026-06-22T12:00:00Z", "one shared now");
    let modules = v["modules"].as_array().expect("modules array");
    let keys: Vec<&str> = modules
        .iter()
        .map(|m| m["module"].as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        vec!["flow", "memory"],
        "both active modules, in order"
    );
    // Each carries its module's own prime record nested under `prime`.
    assert!(
        modules[0]["prime"].is_object(),
        "flow's prime record nested"
    );
    assert!(
        modules[1]["prime"].is_object(),
        "memory's prime record nested"
    );
}

#[test]
fn nxc_is_seated_in_the_chooser_and_the_nxs_prime_fan_out() {
    // nexus-chat-M1 T3 DoD: chat appears in the `nxs init` chooser + the `nxs prime` fan-out purely
    // from its `inventory` self-registration — the ONLY manual `nxs` change is the load-bearing
    // `use nexus_chat as _;` link line in `main.rs` (spec §5.2).
    let tmp = TempDir::new().unwrap();

    // Roster-driven chooser: `nxs init --module chat` is the non-interactive form of picking chat in
    // the chooser. It succeeding at all proves the self-registration seated chat in the roster the
    // chooser reads (an unknown `--module` is a loud error).
    let init = bin("nxs", tmp.path())
        .args(["init", "--module", "chat", "--json"])
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&init.get_output().stdout).expect("nxs init --json");
    assert!(
        v["modules"].as_array().unwrap().iter().any(|m| m == "chat"),
        "chat is active after `nxs init --module chat`: {v}"
    );
    assert_eq!(
        v["hook"]["commands"],
        // Unlike the on-disk file (see `assert_wired`), the REPORT's order is the assembler's own
        // and is deterministic roster order, so it is asserted exactly.
        serde_json::json!(nxs_init::assembler::hook_commands(&["nxc"])),
        "chat init wires chat's own hook"
    );

    // `nxs prime` fans out to chat's prime (text form).
    let prime = bin("nxs", tmp.path()).arg("prime").assert().success();
    let out = String::from_utf8_lossy(&prime.get_output().stdout).to_string();
    assert!(
        out.contains("nexus-chat"),
        "fan-out delivers chat's prime: {out}"
    );

    // …and under `--json`, chat's own prime record nests in the fan-out, in roster order.
    let prime_json = bin("nxs", tmp.path())
        .args(["prime", "--json"])
        .env("NXS_NOW", "2026-07-10T00:00:00Z")
        .assert()
        .success();
    let v: Value =
        serde_json::from_slice(&prime_json.get_output().stdout).expect("nxs prime --json");
    let chat = v["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["module"] == "chat")
        .expect("chat present in the prime fan-out");
    assert!(
        chat["prime"].is_object(),
        "chat's prime record is nested under the fan-out: {chat}"
    );
}

#[test]
fn chat_joins_an_existing_flow_and_memory_workspace_with_its_own_hook_and_fans_out_three() {
    // nexus-chat-M1 T3 coexistence smoke (fuller coexistence + sync is T4): chat activating on top
    // of a flow+memory workspace adds ITS OWN hook and nothing else, and `nxs prime` must fan out
    // all three in ROSTER order — chat neither displaces the siblings nor duplicates the wiring.
    //
    // **Corrected 2026-08-29: this said "active-module order", and had been wrong since
    // 6j6v.xbnh** — which is when the fan-out started sorting into roster order precisely so that
    // it would NOT depend on which module a workspace set up first. Nothing here changed; the
    // sentence describing it was simply left behind, and nxf 6j6v.shwz is what made the two
    // orders worth telling apart again.
    //
    // **Superseded 2026-08-28 (nxf n2m6 + a2a1):** this used to require that chat "must NOT add a
    // second hook". One hook per active module is the shape now, so chat adding exactly one is the
    // contract; what it still must not do is touch the other two.
    let tmp = TempDir::new().unwrap();
    bin("nxf", tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    bin("nxm", tmp.path()).arg("init").assert().success();
    bin("nxc", tmp.path()).arg("init").assert().success();

    // Three modules, three hooks, and exactly one of them with the fallback.
    assert_wired(tmp.path(), &["nxf", "nxc", "nxm"]);
    // The single nxs-owned pointer names all three, in ROSTER order.
    //
    // **Changed 2026-08-29 (nxf 6j6v.shwz): it used to be active-module order** — here
    // `(flow, memory, chat)`, because chat joined last. The sort that gives the hooks a decided
    // order sits in `assemble_with`, the one place every assembly path reaches, so the pointer's
    // list is sorted with them. That is the point rather than a side effect: activation order is
    // the accident this item removes, and a pointer that named the tools in one sequence while the
    // hooks and the fan-out used another would be a second order surviving inside the fix for the
    // first.
    let agents = std::fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
    assert_eq!(
        agents.matches("<!-- BEGIN NEXUS ").count(),
        1,
        "still exactly one pointer"
    );
    assert!(
        agents.contains("(flow, chat, memory)"),
        "the one pointer names all three in roster order: {agents}"
    );

    // `nxs prime` fans out all three.
    let prime = bin("nxs", tmp.path()).arg("prime").assert().success();
    let out = String::from_utf8_lossy(&prime.get_output().stdout).to_string();
    for needle in ["nexus-flow", "nexus-memory", "nexus-chat"] {
        assert!(out.contains(needle), "fan-out includes {needle}: {out}");
    }
}

// ---- `nxs init` (aye.29): drive modules, frame everything, no sub-init bleed-through ----

fn agents(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("AGENTS.md")).unwrap_or_default()
}

#[test]
fn nxs_init_drives_flow_and_frames_the_output_without_bleed_through() {
    let tmp = TempDir::new().unwrap();
    let out = bin("nxs", tmp.path())
        .args(["init", "--module", "flow"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();

    // `nxs` frames the visible output…
    assert!(
        stdout.contains("nexus-flow is set up"),
        "nxs frames the summary: {stdout}"
    );
    assert!(stdout.contains("flow"), "names the module set up: {stdout}");
    // …and NO raw sub-init line bleeds through (the golden contract). nxf's human init prints
    // `initialized workspace at …`; driven `--quiet` + captured stdout means it never appears.
    assert!(
        !stdout.contains("initialized workspace at"),
        "no flow sub-init bleed-through: {stdout}"
    );

    // The real work happened: the nxs-owned pointer naming flow + flow's own SessionStart hook.
    assert!(
        agents(tmp.path()).contains("<!-- BEGIN NEXUS ") && agents(tmp.path()).contains("(flow)"),
        "discovery pointer assembled naming flow"
    );
    assert_wired(tmp.path(), &["nxf"]);
}

#[test]
fn nxs_init_with_flow_and_memory_sets_up_both_with_one_hook_each_no_bleed_through() {
    let tmp = TempDir::new().unwrap();
    let out = bin("nxs", tmp.path())
        .args(["init", "--module", "flow", "--module", "memory"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();

    assert!(stdout.contains("nexus-flow is set up"), "framed: {stdout}");
    // Neither module's own init banner bleeds through nxs's frame.
    assert!(
        !stdout.contains("initialized workspace at"),
        "no flow bleed-through: {stdout}"
    );
    assert!(
        !stdout.contains("initialized memory at"),
        "no memory bleed-through: {stdout}"
    );

    // One nxs-owned pointer naming both tools in roster order, with one hook per module.
    let a = agents(tmp.path());
    assert_eq!(a.matches("<!-- BEGIN NEXUS ").count(), 1, "one pointer");
    assert!(
        a.contains("(flow, memory)"),
        "names both tools, flow before memory: {a}"
    );
    assert!(!a.contains("BEGIN NEXUS-"), "no per-module blocks: {a}");
    assert_wired(tmp.path(), &["nxf", "nxm"]);

    // …and `nxs prime` then fans out to both modules.
    let prime = bin("nxs", tmp.path()).arg("prime").assert().success();
    let p = String::from_utf8_lossy(&prime.get_output().stdout).to_string();
    assert!(
        p.contains("nexus-flow") && p.contains("nexus-memory"),
        "both primed: {p}"
    );
}

#[test]
fn nxs_init_json_is_a_machine_record_with_no_prompts() {
    let tmp = TempDir::new().unwrap();
    let out = bin("nxs", tmp.path())
        .args(["init", "--json", "--module", "flow", "--module", "memory"])
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).expect("nxs init --json");
    assert_eq!(v["ok"], true);
    let mods: Vec<&str> = v["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    assert_eq!(mods, vec!["flow", "memory"], "both active, roster order");
    assert_eq!(
        v["hook"]["commands"],
        serde_json::json!(nxs_init::assembler::hook_commands(&["nxf", "nxm"])),
        "the report lists them in roster order — flow first, and flow carries the fallback"
    );
    assert_eq!(v["hook"]["wired"], true);
}

#[test]
fn nxs_init_non_interactive_with_no_module_defaults_to_flow() {
    // No flags + non-interactive (assert_cmd has no TTY) → flow, no prompt, no hang.
    let tmp = TempDir::new().unwrap();
    bin("nxs", tmp.path()).arg("init").assert().success();
    assert!(
        agents(tmp.path()).contains("(flow)"),
        "flow set up by default"
    );
    assert_wired(tmp.path(), &["nxf"]);
}

#[test]
fn nxs_init_json_reports_already_active_for_an_informative_re_run() {
    // 5jz.4: a fresh init reports nothing already active; a re-run that adds a module reports what
    // was already there. An agent re-running `nxs init --json` can tell what it set up from what was
    // already present — the re-run is informative, not a silent, ambiguous repeat.
    let tmp = TempDir::new().unwrap();

    // Fresh workspace: flow set up, nothing was active before → already_active is empty.
    let fresh = bin("nxs", tmp.path())
        .args(["init", "--json", "--module", "flow"])
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&fresh.get_output().stdout).unwrap();
    assert_eq!(
        v["already_active"].as_array().unwrap().len(),
        0,
        "fresh init: nothing was already active: {v}"
    );
    assert_eq!(v["modules"], serde_json::json!(["flow"]));

    // Re-run adding memory to the existing flow workspace: flow was already active, memory is new.
    let rerun = bin("nxs", tmp.path())
        .args(["init", "--json", "--module", "memory"])
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&rerun.get_output().stdout).unwrap();
    assert_eq!(
        v["already_active"],
        serde_json::json!(["flow"]),
        "re-run reports flow as already active: {v}"
    );
    assert_eq!(
        v["modules"],
        serde_json::json!(["flow", "memory"]),
        "both modules active after the re-run: {v}"
    );
}

#[test]
fn nxs_init_regenerates_a_deleted_agents_file_even_when_the_module_is_already_active() {
    // nexus-flow-6c4: the assembler used to hang off driving a module, and `nxs init` skips a module
    // that is already active — so a re-run drove nothing and never re-assembled. Deleting AGENTS.md
    // then re-running `nxs init` (with the module already active) must bring it BACK, header and all.
    let tmp = TempDir::new().unwrap();

    // First init: flow set up, AGENTS.md present.
    bin("nxs", tmp.path())
        .args(["init", "--module", "flow"])
        .assert()
        .success();
    assert!(
        tmp.path().join("AGENTS.md").exists(),
        "first init writes AGENTS.md"
    );

    // The user (or a stray clean) deletes AGENTS.md.
    std::fs::remove_file(tmp.path().join("AGENTS.md")).unwrap();

    // Re-run `nxs init` with flow ALREADY active → the module is NOT re-driven, yet the umbrella's
    // unconditional post-loop assemble regenerates the file.
    let out = bin("nxs", tmp.path())
        .args(["init", "--module", "flow", "--json"])
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(
        v["already_active"],
        serde_json::json!(["flow"]),
        "flow was already active — nothing was driven: {v}"
    );

    let a = agents(tmp.path());
    assert!(
        a.contains("<!-- BEGIN NEXUS "),
        "AGENTS.md was regenerated: {a}"
    );
    assert!(
        a.contains("## nexus-flow tools for agents"),
        "regenerated with the Markdown header (s1w): {a}"
    );
    assert!(a.contains("(flow)"), "names the active tool: {a}");
    // Still exactly flow's one hook — the idempotent assemble never double-wires it.
    assert_wired(tmp.path(), &["nxf"]);
}

#[test]
fn nxs_init_with_flow_renders_a_copy_paste_first_command_success_moment() {
    // 5jz.6: the human frame ends with flow's OWN copy-paste first command (self-registered, the
    // same one `nxf init` shows natively) — a clear "now do this" success moment. `--json` stays a
    // pure machine record with no such prose (byte-stable), so only the human path gains it.
    let tmp = TempDir::new().unwrap();
    let out = bin("nxs", tmp.path())
        .args(["init", "--module", "flow"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.contains("nxf create"),
        "renders flow's copy-paste first command in the frame: {stdout}"
    );

    // The machine path carries no success-moment prose — a clean record an agent parses.
    let json = bin("nxs", TempDir::new().unwrap().path())
        .args(["init", "--json", "--module", "flow"])
        .assert()
        .success();
    let jstr = String::from_utf8_lossy(&json.get_output().stdout);
    assert!(
        !jstr.contains("your first move"),
        "no success-moment prose in --json: {jstr}"
    );
}

#[test]
fn nxs_init_rejects_an_unknown_module() {
    let tmp = TempDir::new().unwrap();
    bin("nxs", tmp.path())
        .args(["init", "--json", "--module", "ghost"])
        .assert()
        .failure()
        .stdout(predicates::str::contains("\"kind\":\"validation\""))
        .stdout(predicates::str::contains("ghost"));
}

#[test]
fn nxs_init_surfaces_a_failing_module_init_loudly() {
    // The in-process fan-out failure composition (5jz.3): set up a memory-only workspace first, then
    // ask `nxs init` to add flow. flow's in-process init refuses to nest into the existing `.nxs/`,
    // so its error propagates DIRECTLY — `nxs init` must FAIL loudly, surfacing the real error, not
    // swallow it. With in-process composition there is no child subprocess, so the nesting error is
    // its own `conflict` kind (not an `io` child-exit wrapper).
    let tmp = TempDir::new().unwrap();
    bin("nxm", tmp.path()).arg("init").assert().success();

    let out = bin("nxs", tmp.path())
        .args(["init", "--json", "--module", "flow"])
        .assert()
        .failure();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).expect("json error envelope");
    assert_eq!(
        v["error"]["kind"], "conflict",
        "the in-process nesting refusal surfaces as a conflict: {v}"
    );
    assert!(
        v["error"]["msg"]
            .as_str()
            .unwrap()
            .contains("already inside a workspace"),
        "the real error is surfaced, not swallowed: {v}"
    );
}

#[test]
fn nxs_init_passes_the_plugin_through_to_flow_in_process() {
    // 5jz.3: with flow set up IN-PROCESS (not a `--quiet` subprocess), a non-interactive `--plugin`
    // must still reach flow and land in config.toml — the pass-through the agent entry depends on.
    let tmp = TempDir::new().unwrap();
    bin("nxs", tmp.path())
        .args(["init", "--module", "flow", "--plugin", "personal-todo"])
        .assert()
        .success();
    let config = std::fs::read_to_string(tmp.path().join(".nxs/config.toml")).unwrap();
    assert!(
        config.contains("personal-todo"),
        "flow's --plugin landed in config via the in-process path: {config}"
    );
}

#[test]
fn nxs_init_rejects_the_db_flag_loudly() {
    // `init` is cwd-scoped (it CREATES the workspace), so it cannot honor `--db`/`NXS_DB`. Rather
    // than accept-and-ignore (a silent surprise), it rejects the flag with a validation error.
    let tmp = TempDir::new().unwrap();
    bin("nxs", tmp.path())
        .args(["init", "--db", "/tmp/nxs-bogus.sqlite", "--json"])
        .assert()
        .failure()
        .stdout(predicates::str::contains("\"kind\":\"validation\""))
        .stdout(predicates::str::contains("--db"));
    // …and it created nothing in the cwd.
    assert!(
        !tmp.path().join(".nxs").exists(),
        "rejected before any setup"
    );
}

// ── `nxs setup claude` (nexus-flow-5od) — the canonical host-integration verb ─

#[test]
fn nxs_setup_claude_reassembles_agent_files_and_rewires_the_hook() {
    // nexus-flow-5od + 6c4: the canonical umbrella verb (re)assembles the agent files AND wires the
    // single `nxs prime` hook. Deleting BOTH the agent file and the whole `.claude/` proves it
    // rebuilds both, not just the hook.
    let tmp = TempDir::new().unwrap();
    bin("nxf", tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    std::fs::remove_file(tmp.path().join("AGENTS.md")).unwrap();
    std::fs::remove_dir_all(tmp.path().join(".claude")).ok();

    let out = bin("nxs", tmp.path())
        .args(["setup", "claude", "--json"])
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).expect("setup claude --json");
    assert_eq!(v["ok"], true);
    assert_eq!(v["hook_added"], true, "the removed hook is re-wired: {v}");
    assert_eq!(
        v["agents"], "created",
        "the deleted AGENTS.md is regenerated: {v}"
    );

    let a = agents(tmp.path());
    assert!(
        a.contains("<!-- BEGIN NEXUS ") && a.contains("## nexus-flow tools for agents"),
        "agent file rebuilt with the header: {a}"
    );
    assert_wired(tmp.path(), &["nxf"]);

    // Idempotent: a second run changes nothing.
    let again = bin("nxs", tmp.path())
        .args(["setup", "claude", "--json"])
        .assert()
        .success();
    let v2: Value = serde_json::from_slice(&again.get_output().stdout).unwrap();
    assert_eq!(v2["hook_added"], false, "re-run does not re-wire: {v2}");
    assert_eq!(v2["permission_added"], false, "re-run adds no perms: {v2}");
}

#[test]
fn nxs_setup_claude_rejects_the_db_flag_loudly() {
    // Like `init`, `setup claude` is cwd-scoped (it wires host integration for the walk-up
    // workspace), so it rejects `--db`/`NXS_DB` loudly rather than silently writing to the cwd
    // workspace instead of the pointed-at one. Covers BOTH the flag and the `NXS_DB` env form.
    let tmp = TempDir::new().unwrap();
    bin("nxf", tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    // Remove the init-wired `.claude` so a rejection that wrote nothing is observable.
    std::fs::remove_dir_all(tmp.path().join(".claude")).ok();

    for mut cmd in [
        {
            let mut c = bin("nxs", tmp.path());
            c.args(["setup", "claude", "--db", "/tmp/nxs-bogus.sqlite", "--json"]);
            c
        },
        {
            let mut c = bin("nxs", tmp.path());
            c.args(["setup", "claude", "--json"])
                .env("NXS_DB", "/tmp/nxs-bogus.sqlite");
            c
        },
    ] {
        cmd.assert()
            .failure()
            .stdout(predicates::str::contains("\"kind\":\"validation\""))
            .stdout(predicates::str::contains("--db"));
    }
    // Rejected before any wiring — the removed `.claude` was NOT re-created.
    assert!(
        !tmp.path().join(".claude").exists(),
        "rejected before any host integration was written"
    );
}

#[test]
fn nxs_setup_claude_outside_a_workspace_fails_loudly() {
    // No `.nxs/` up-tree → a loud `no_workspace` error, never a stray `.claude/` in a random dir.
    let tmp = TempDir::new().unwrap();
    let out = bin("nxs", tmp.path())
        .args(["setup", "claude", "--json"])
        .assert()
        .failure();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).expect("json error envelope");
    assert_eq!(v["error"]["kind"], "no_workspace", "loud failure: {v}");
    assert!(
        !tmp.path().join(".claude").exists(),
        "nothing scattered outside a workspace"
    );
}

// ── beads → nxs migration (nexus-flow-6ef) ───────────────────────────────────

/// Build a beads project skeleton under `dir`: a managed block in AGENTS.md, `bd prime` hooks in
/// `.claude/settings.json`, and a tickets-only `.beads/issues.jsonl` export (the fallback source,
/// exercised here because the test clears `bd` from PATH). Three tickets: an epic, a closed child
/// (parent-child + close reason), and a child that blocks on the first child.
fn seed_beads_project(dir: &Path) {
    std::fs::write(
        dir.join("AGENTS.md"),
        "# Agent Instructions\n\nHand-written note about bd I wrote myself.\n\n\
         <!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:abc -->\n\
         ## Beads\nmanaged content\n<!-- END BEADS INTEGRATION -->\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join(".claude")).unwrap();
    std::fs::write(
        dir.join(".claude/settings.json"),
        r#"{"hooks":{"SessionStart":[{"matcher":"","hooks":[{"type":"command","command":"bd prime"}]}],"PreCompact":[{"hooks":[{"command":"bd prime"}]}]},"model":"opus"}"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.join(".beads")).unwrap();
    let issues = [
        r#"{"_type":"issue","id":"nexus-flow-epic","title":"The epic","description":"big","status":"open","priority":1,"issue_type":"epic"}"#,
        r#"{"_type":"issue","id":"nexus-flow-a","title":"closed child","description":"d","status":"closed","priority":2,"issue_type":"task","closed_at":"2026-02-02T00:00:00Z","close_reason":"shipped","dependencies":[{"issue_id":"nexus-flow-a","depends_on_id":"nexus-flow-epic","type":"parent-child"}]}"#,
        r#"{"_type":"issue","id":"nexus-flow-b","title":"blocked child","description":"d","status":"open","priority":2,"issue_type":"feature","dependencies":[{"issue_id":"nexus-flow-b","depends_on_id":"nexus-flow-epic","type":"parent-child"},{"issue_id":"nexus-flow-b","depends_on_id":"nexus-flow-a","type":"blocks"}]}"#,
    ];
    std::fs::write(dir.join(".beads/issues.jsonl"), issues.join("\n") + "\n").unwrap();
}

/// The service flags really reach the migration path (review of PR #419, Test Quality #3).
///
/// `migrate_beads::run` takes the flag as one more argument and hands it to the same
/// `background_service::apply` the ordinary frame uses, so the BEHAVIOUR is covered where that
/// function is tested. What no test covered is the argument itself: passing `None` there instead
/// of the caller's value compiles, runs, and looks exactly like a user who said nothing.
///
/// Split by platform for the reason the flags themselves are: `--no-service` records an answer
/// only where there is a service to decline, and `--service` may only be exercised where it
/// installs NOTHING — which is the same line.
#[test]
#[cfg(target_os = "macos")]
fn the_migration_path_carries_the_service_flag_it_was_given() {
    let tmp = TempDir::new().unwrap();
    seed_beads_project(tmp.path());
    let out = bin("nxs", tmp.path())
        .args(["init", "--from-beads", "--no-service", "--json"])
        .env("PATH", tmp.path())
        .env("NXS_NOW", "2026-06-26T00:00:00Z")
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).expect("migration --json");
    assert_eq!(
        v["service"]["answer"], "declined",
        "the flag the caller typed reached the offer, not a `None` in its place: {v}"
    );
}

#[test]
#[cfg(not(target_os = "macos"))]
fn the_migration_path_carries_the_service_flag_it_was_given() {
    let tmp = TempDir::new().unwrap();
    seed_beads_project(tmp.path());
    let out = bin("nxs", tmp.path())
        .args(["init", "--from-beads", "--service", "--json"])
        .env("PATH", tmp.path())
        .env("NXS_NOW", "2026-06-26T00:00:00Z")
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(
        stderr.contains("nxs sync daemon"),
        "the flag reached the offer, which answered it by name for this platform: {stderr}"
    );
}

#[test]
fn nxs_init_from_beads_migrates_tickets_and_rolls_back_beads_config() {
    let tmp = TempDir::new().unwrap();
    seed_beads_project(tmp.path());

    // `--from-beads --json` consents non-interactively. PATH is a bd-free dir so `bd` is NOT found
    // → the tickets-only `issues.jsonl` fallback is used (sibling nxf/nxm resolve via current_exe,
    // not PATH, so the assembler shell-out still works).
    let out = bin("nxs", tmp.path())
        .args(["init", "--from-beads", "--json"])
        .env("PATH", tmp.path())
        .env("NXS_NOW", "2026-06-26T00:00:00Z")
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).expect("migration --json");

    // Report: not a dry run; all three tickets imported; the closed child kept its reason; the
    // parent + dep edges were carried; memories skipped (tickets-only fallback).
    assert_eq!(v["dry_run"], false);
    assert_eq!(
        v["executed"]["imported_issues"], 3,
        "all tickets imported: {v}"
    );
    assert_eq!(v["executed"]["imported_closed"], 1, "one closed ticket");
    assert_eq!(v["executed"]["parent_edges"], 2, "two parent-child edges");
    assert_eq!(v["executed"]["dep_edges"], 1, "one blocks edge");
    assert_eq!(
        v["memories_supported"], false,
        "fallback carries no memories"
    );

    // Workspace set up; nxs hook wired; beads block + hook rolled back; prose kept; backup written.
    assert!(tmp.path().join(".nxs").exists(), "nxs workspace created");
    // The beads import brings a memory store along with the board, so this workspace ends up with
    // flow AND memory active — two modules, two hooks, and the `bd prime` hook gone.
    assert_wired(tmp.path(), &["nxf", "nxm"]);
    let settings = std::fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap();
    assert!(
        !settings.contains("bd prime"),
        "all bd prime hooks removed: {settings}"
    );
    assert!(settings.contains("\"model\""), "unrelated settings kept");
    let agents = std::fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
    assert!(
        !agents.contains("BEGIN BEADS INTEGRATION"),
        "beads block removed: {agents}"
    );
    assert!(
        agents.contains("Hand-written note about bd I wrote myself."),
        "prose kept: {agents}"
    );
    assert!(
        agents.contains("<!-- BEGIN NEXUS "),
        "nxs discovery pointer written: {agents}"
    );
    assert!(
        std::fs::read_dir(tmp.path().join(".beads/migration-backups"))
            .unwrap()
            .next()
            .is_some(),
        "a timestamped backup was written"
    );

    // nxf 6j6v.npf9: a migration is a workspace being BORN, and this path returns from `init`
    // before its frame is ever reached — so the one route that sets a workspace up from an
    // existing project was the one route that never told the background service about it.
    assert_eq!(
        v["service"]["registered"], true,
        "the migrated workspace reached the service's list: {v}"
    );
    let registry = std::fs::read_to_string(
        nxs_test_support::pinned_home()
            .join(".nexusflow")
            .join("workspaces.toml"),
    )
    .expect("the migration wrote the registry");
    let root = tmp.path().canonicalize().unwrap().display().to_string();
    assert!(
        registry.contains(&root),
        "and by its project root: {registry}"
    );
    // beads data is preserved (never deleted).
    assert!(
        tmp.path().join(".beads/issues.jsonl").exists(),
        ".beads/ data preserved"
    );

    // The migrated board is queryable through flow: three items, one closed.
    let list = bin("nxf", tmp.path())
        .args(["list", "--json"])
        .assert()
        .success();
    let items: Value = serde_json::from_slice(&list.get_output().stdout).expect("nxf list --json");
    assert_eq!(
        items.as_array().unwrap().len(),
        3,
        "three migrated items on the board"
    );

    // Idempotence: a re-run does NOT re-offer the migration even though `.beads/` (and its
    // issues.jsonl) is still on disk — the migration is done once `.nxs/` exists. The re-run is a
    // normal, no-op init (a `workspace`/`modules` record, NOT a migration dry-run offer), and it
    // does not duplicate the board.
    let rerun = bin("nxs", tmp.path())
        .args(["init", "--json"])
        .env("PATH", tmp.path())
        .assert()
        .success();
    let v2: Value = serde_json::from_slice(&rerun.get_output().stdout).expect("re-run --json");
    assert!(
        v2.get("dry_run").is_none(),
        "re-run must not re-offer the migration: {v2}"
    );
    assert!(
        v2.get("modules").is_some(),
        "re-run is a normal init record: {v2}"
    );
    let list2 = bin("nxf", tmp.path())
        .args(["list", "--json"])
        .assert()
        .success();
    let items2: Value = serde_json::from_slice(&list2.get_output().stdout).unwrap();
    assert_eq!(
        items2.as_array().unwrap().len(),
        3,
        "re-run did not duplicate the board"
    );
}

#[test]
fn nxs_init_in_a_beads_project_without_consent_is_a_no_op() {
    // Non-interactive (`--json`) without `--from-beads`: the migration is OFFERED, not performed —
    // no workspace, no rollback, beads left exactly as it was (consent is mandatory).
    let tmp = TempDir::new().unwrap();
    seed_beads_project(tmp.path());
    let out = bin("nxs", tmp.path())
        .args(["init", "--json"])
        .env("PATH", tmp.path())
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).expect("offer --json");
    assert_eq!(v["dry_run"], true, "a preview, not a run");
    assert_eq!(v["migrated"], false);
    assert!(
        v["hint"].as_str().unwrap().contains("--from-beads"),
        "tells how to opt in"
    );
    // Nothing changed: no workspace, beads block + hook intact.
    assert!(
        !tmp.path().join(".nxs").exists(),
        "no workspace created without consent"
    );
    let agents = std::fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
    assert!(
        agents.contains("BEGIN BEADS INTEGRATION"),
        "beads block untouched"
    );
    let settings = std::fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap();
    assert!(settings.contains("bd prime"), "bd hook untouched");
}

/// Write an executable stub `bd` into `dir` that prints `export_jsonl` on any invocation — so the
/// migration's `bd export` path (the one that carries memories) is exercised without a real beads
/// install. Returns the directory to put on `PATH`. Unix-only (a `/bin/sh` script).
#[cfg(unix)]
fn stub_bd(dir: &Path, export_jsonl: &str) {
    use std::os::unix::fs::PermissionsExt;
    // The test sets PATH to ONLY this dir (so the stub `bd` shadows any real one), which means the
    // script can't rely on external commands like `cat`. `printf` is a POSIX sh builtin, so emit one
    // single-quoted printf arg per JSONL line (the records use double quotes, never single).
    let args: String = export_jsonl.lines().map(|l| format!(" '{l}'")).collect();
    let script = format!("#!/bin/sh\nprintf '%s\\n'{args}\n");
    let bd = dir.join("bd");
    std::fs::write(&bd, script).unwrap();
    std::fs::set_permissions(&bd, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn nxs_init_from_beads_imports_memories_via_the_bd_export_path() {
    // The fallback e2e clears `bd` from PATH (tickets-only); this one provides a stub `bd` whose
    // `export` carries a `memory` record, exercising the FULL path through `run`→`execute`→nxm and
    // the `imported_memories` report field — the wiring the fallback path can't reach.
    let tmp = TempDir::new().unwrap();
    seed_beads_project(tmp.path()); // gives the detection signal (block + hook)
    let binsdir = TempDir::new().unwrap();
    let export = concat!(
        r#"{"_type":"issue","id":"nf-epic","title":"Epic via bd","description":"d","status":"open","priority":1,"issue_type":"epic"}"#,
        "\n",
        r#"{"_type":"issue","id":"nf-child","title":"Child via bd","description":"d","status":"open","priority":2,"issue_type":"feature","dependencies":[{"issue_id":"nf-child","depends_on_id":"nf-epic","type":"parent-child"}]}"#,
        "\n",
        r#"{"_type":"memory","key":"bd-fact","value":"a fact remembered in beads"}"#,
    );
    stub_bd(binsdir.path(), export);

    let out = bin("nxs", tmp.path())
        .args(["init", "--from-beads", "--json"])
        .env("PATH", binsdir.path()) // only the stub `bd` is resolvable; nxf/nxm resolve as siblings
        .env("NXS_NOW", "2026-06-26T00:00:00Z")
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).expect("migration --json");
    assert_eq!(
        v["memories_supported"], true,
        "bd export carries memories: {v}"
    );
    assert_eq!(v["executed"]["imported_issues"], 2);
    assert_eq!(
        v["executed"]["imported_memories"], 1,
        "the memory was imported: {v}"
    );
    assert_eq!(v["executed"]["parent_edges"], 1);

    // The memory landed in nxm under its beads key.
    let mem = bin("nxm", tmp.path())
        .args(["memories", "--json"])
        .assert()
        .success();
    let mems: Value =
        serde_json::from_slice(&mem.get_output().stdout).expect("nxm memories --json");
    assert!(
        mems.as_array()
            .unwrap()
            .iter()
            .any(|m| m["key"] == "bd-fact"),
        "the bd-remembered fact is in nxm: {mems}"
    );
}

// ---- `NEXUS_MEMORY.md` as a build product (6j6v.8q88) ------------------------------------------

/// The two halves of the fallback path, proven over the whole umbrella rather than over one module:
/// exactly one wired SessionStart hook carries a `|| cat NEXUS_MEMORY.md` tail (the first the
/// assembler wires — Ruling R1, nxf a2a1; it was the single `nxs prime` hook until n2m6 + a2a1),
/// so on a machine WITHOUT nexus-flow the file is what a session actually gets.
///
/// **This was a whole-document subset condition until nxf 6j6v.xbnh** ("it may deliver less, never
/// more"). The owner decision of 2026-08-27 ends that deliberately: the file keeps every memory's
/// FULL TEXT, because it is the versioned, diff-able record of what the memories say and an
/// index-only file would move every body out of every change proposal. What replaces the condition
/// is stated over the file's two halves, and asserted here against the REAL `nxs prime` output
/// rather than an in-process rehearsal:
///
/// 1. the INDEX half is `nxs prime`'s own bytes — which is what makes the file's notice ("you have
///    already been given the index below") true rather than a claim;
/// 2. the full-text half is what `nxm recall <key>` serves, and `nxs prime` carries none of it.
#[test]
fn the_projected_documents_index_is_verbatim_what_nxs_prime_delivers() {
    let tmp = TempDir::new().unwrap();
    bin("nxs", tmp.path())
        .args(["init", "--module", "flow", "--module", "memory", "--json"])
        .assert()
        .success();
    for (key, body, introduction) in [
        ("auth-jwt", "auth uses JWT, not sessions", "auth is JWT"),
        (
            "release-tags",
            "a published tag is never moved — signatures hang off it",
            "never move a published tag",
        ),
    ] {
        bin("nxm", tmp.path())
            .args([
                "remember",
                body,
                "--key",
                key,
                "--introduction",
                introduction,
            ])
            .assert()
            .success();
    }

    let prime = bin("nxs", tmp.path()).arg("prime").assert().success();
    let prime = String::from_utf8_lossy(&prime.get_output().stdout).to_string();

    let doc = std::fs::read_to_string(tmp.path().join("NEXUS_MEMORY.md"))
        .expect("the writes projected the document");
    let index = doc
        .split_once("## Index (")
        .expect("the document carries an index")
        .1
        .split_once("\n\n")
        .expect("…with entries under its heading")
        .1
        .split_once("\n\n## Full text")
        .expect("…and a full-text half beneath")
        .0;
    assert!(
        prime.contains(index),
        "the index half must occur verbatim in `nxs prime`:\n--- index ---\n{index}\n--- prime \
         ---\n{prime}"
    );
    assert!(
        doc.contains("a published tag is never moved")
            && !prime.contains("a published tag is never moved"),
        "the bodies live in the file and NOT in the session start:\n--- doc ---\n{doc}\n--- prime \
         ---\n{prime}"
    );
}

/// A PATH holding `cat` and nothing else — the machine the fallback exists for: a contributor who
/// cloned the repository (and therefore has `.claude/settings.json` and its hook) but has never
/// installed nexus-flow, so `nxs` is not a command at all.
#[cfg(unix)]
fn path_without_nexus_flow(tmp: &Path) -> std::path::PathBuf {
    let bin = tmp.join("fake-bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink("/bin/cat", bin.join("cat")).unwrap();
    bin
}

/// Run the wired hook command that CARRIES THE FALLBACK, exactly as a shell would, on that machine.
///
/// **Superseded 2026-08-28 (nxf n2m6 + a2a1):** there was one hook command and it was the fallback
/// carrier, so this ran the constant. With one entry per active module the fallback hangs on the
/// FIRST entry only (Ruling R1), and that is the one this has to run — running any other would
/// prove nothing about the contributor-without-nexus-flow path, since the others deliberately have
/// no `|| cat` at all.
#[cfg(unix)]
fn run_hook_without_nexus_flow(cwd: &Path, path: &Path) -> std::process::Output {
    let hooks = session_start_hooks(cwd);
    let carrier = hooks
        .iter()
        .find(|c| c.contains("cat NEXUS_MEMORY.md"))
        .unwrap_or_else(|| panic!("one wired hook carries the fallback: {hooks:?}"));
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(carrier)
        .current_dir(cwd)
        .env("PATH", path)
        .output()
        .expect("run the hook command through a shell")
}

/// The other half of the same hook: with nexus-flow absent, `cat NEXUS_MEMORY.md` IS the session
/// context. Run here exactly as a shell would run it, so the fallback is proven end to end rather
/// than reasoned about.
#[cfg(unix)]
#[test]
fn the_fallback_half_of_the_hook_delivers_the_context_without_nexus_flow() {
    let tmp = TempDir::new().unwrap();
    bin("nxs", tmp.path())
        .args(["init", "--module", "memory", "--json"])
        .assert()
        .success();
    bin("nxm", tmp.path())
        .args([
            "remember",
            "auth uses JWT, not sessions",
            "--key",
            "auth-jwt",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();

    let path = path_without_nexus_flow(tmp.path());
    let out = run_hook_without_nexus_flow(tmp.path(), &path);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains("auth uses JWT, not sessions"),
        "the fallback delivered the project's memory; got:\n{stdout}"
    );
}

/// A workspace that has remembered nothing has no document — a legitimate state, and one the hook
/// must survive without spilling a shell error into the agent's context window.
#[cfg(unix)]
#[test]
fn the_hook_stays_quiet_when_there_is_no_document_to_fall_back_to() {
    let tmp = TempDir::new().unwrap();
    bin("nxs", tmp.path())
        .args(["init", "--module", "memory", "--json"])
        .assert()
        .success();
    assert!(!tmp.path().join("NEXUS_MEMORY.md").exists());

    let path = path_without_nexus_flow(tmp.path());
    let out = run_hook_without_nexus_flow(tmp.path(), &path);
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("No such file"),
        "no missing-file error is injected into the session: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// **The wired order does not depend on which module was initialized first** (nxf 6j6v.shwz).
///
/// The mirror of `the_session_start_ceiling.rs`'s
/// `a_workspace_that_added_memory_first_still_fans_out_in_the_same_order`, which pinned the same
/// property for the `nxs prime` fan-out and left the HOOKS free to vary. They did: the assembler
/// resolved modules in `config.toml`'s `active_modules` order, which is the order `init` appended
/// them, so a workspace set up memory-first wired `nxm prime` ahead of `nxf prime`. Measured in the
/// field on 2026-08-29: 15 of 16 workspaces had flow first and `manufakt-io`, whose config reads
/// `["memory", "flow"]`, had memory first.
///
/// One order for both, and it is the fan-out's: memory last, because it is the one block a session
/// can fetch back afterwards.
#[test]
fn the_wired_order_is_the_fan_out_order_whichever_module_was_initialized_first() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    bin("nxf", dir)
        .args(["init", "--quiet", "--plugin", "issue-tracker"])
        .assert()
        .success();
    bin("nxm", dir).arg("init").assert().success();

    // Rewrite `active_modules` into the order a workspace whose memory was set up first carries —
    // on the FILE, because `init` refuses to run inside an existing workspace, and because the file
    // is what the assembler read its order from. Same fixture shape as
    // `the_session_start_ceiling.rs`'s fan-out test, which is the point: one property, two
    // surfaces, and until now only one of them held it.
    let path = dir.join(".nxs/config.toml");
    let config = std::fs::read_to_string(&path).unwrap();
    let reordered = config.replace(
        r#"active_modules = ["flow", "memory"]"#,
        r#"active_modules = ["memory", "flow"]"#,
    );
    assert_ne!(
        reordered, config,
        "the premise: the order really is in the file:\n{config}"
    );
    std::fs::write(&path, &reordered).unwrap();

    // **The settings file is removed first, and that is not incidental.** With it left in place
    // both wanted entries are already present, so the wiring writes nothing and the file keeps the
    // order the FIRST init happened to give it — and this test passes without the order ever
    // having been decided. It did, on the first draft. Removing the file forces the assembler to
    // say what order it wants, which is the only thing under test here; the convergence of a file
    // that already exists in the wrong order is asserted separately below.
    std::fs::remove_file(dir.join(".claude/settings.json")).unwrap();
    bin("nxs", dir).args(["setup", "claude"]).assert().success();
    assert_wired(dir, &["nxf", "nxm"]);

    // …and a file that DOES exist, in the other order, is converged rather than left alone.
    std::fs::write(
        dir.join(".claude/settings.json"),
        r#"{ "hooks": { "SessionStart": [
            { "matcher": "", "hooks": [ { "type": "command", "command": "nxm prime || cat NEXUS_MEMORY.md 2>/dev/null" } ] },
            { "matcher": "", "hooks": [ { "type": "command", "command": "nxf prime" } ] }
        ] } }"#,
    )
    .unwrap();
    bin("nxs", dir).args(["setup", "claude"]).assert().success();
    assert_wired(dir, &["nxf", "nxm"]);
}
