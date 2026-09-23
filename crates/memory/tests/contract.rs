//! Integration tests for `nxm`'s contract verbs (nexus-flow-aye.22): init self-assembly,
//! agent-manifest, and prime — parity with nxf (spec §5.2/§6). Black-box over the built binary.

use assert_cmd::Command;
use nxs_test_support::PinHome;
use serde_json::Value;
use tempfile::TempDir;

fn nxm(dir: &std::path::Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxm");
    c.current_dir(dir)
        .env("NXM_ACTOR", "alice")
        .env("NXM_NOW", "2026-06-20T10:00:00Z");
    c
}

fn json_of(out: &[u8]) -> Value {
    serde_json::from_str(String::from_utf8_lossy(out).trim()).expect("valid json")
}

fn read(p: &std::path::Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

/// nxf 6j6v.y12q, nxm's half: the workspace an AGENT sets up gets a clock like every other.
///
/// `nxm init` returns to the umbrella only on a real terminal; under `--json` — the way an agent
/// enters — it runs memory's own init and never reaches `nxs init`, which is the only path that
/// used to register the workspace. Since 6j6v.8see that registry is where a workspace's deadlines
/// come from.
#[test]
fn init_puts_the_workspace_on_the_list_the_background_service_attends() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join(".fake-home");
    let out = nxm(tmp.path())
        .args(["--json", "init"])
        .pin_home(&home)
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(
        v["service"]["registered"],
        serde_json::json!(true),
        "and it SAYS so, rather than leaving an agent to find out: {v}"
    );

    let registry = read(&home.join(".nexusflow").join("workspaces.toml"));
    let root = tmp.path().canonicalize().unwrap();
    assert!(
        registry.contains(root.to_str().unwrap()),
        "this workspace must be on the list: {registry}"
    );
}

/// The half `init_quiet_prints_nothing_yet_still_wires_workspace_and_one_hook` cannot reach
/// (review of PR #421, Test Quality #2): a `--quiet` run that HAS something to say.
///
/// That test asserts an empty stderr, and `Registration.notes` is empty on every ordinary success
/// — so it passes whether the code suppresses notes or not. This one forces a note by planting a
/// second instance's registry that already attends this workspace, which is the overlap
/// nxf 6j6v.gd9p says must be reported the moment it is created. `--quiet` silences the SUMMARY,
/// never a problem: a swallowed one is the class of bug 6j6v.y12q exists to end.
#[test]
fn a_quiet_init_still_says_the_workspace_is_now_attended_by_two_instances() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join(".fake-home");
    let root = tmp.path().canonicalize().unwrap();
    // A sibling instance's registry, already attending this very workspace.
    let dev = home.join(".nexusflow-dev");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::write(
        dev.join("workspaces.toml"),
        format!(
            "[[workspace]]\nname = \"proj\"\npath = \"{}\"\n",
            root.display()
        ),
    )
    .unwrap();

    let out = nxm(tmp.path())
        .args(["init", "--quiet"])
        .pin_home(&home)
        .assert()
        .success();
    assert!(
        out.get_output().stdout.is_empty(),
        "`--quiet` still prints no summary: {:?}",
        String::from_utf8_lossy(&out.get_output().stdout)
    );
    let stderr = String::from_utf8_lossy(&out.get_output().stderr);
    assert!(
        stderr.contains("more than one nexus-flow service instance"),
        "the overlap this init just created must be said, quiet or not: {stderr}"
    );
}

#[test]
fn init_self_assembles_workspace_agent_files_and_hook() {
    let tmp = TempDir::new().unwrap();
    let out = nxm(tmp.path()).args(["--json", "init"]).assert().success();
    let v = json_of(&out.get_output().stdout);

    assert_eq!(v["ok"], true);
    // memory registered as an active module.
    assert_eq!(v["modules"], serde_json::json!(["memory"]));
    // foundation delegated: the substrate db + memories view exist.
    assert!(
        tmp.path().join(".nxs/db.sqlite").is_file(),
        "substrate db materialized"
    );

    // nexus-flow-0lj.2: AGENTS.md carries only the single nxs-owned discovery pointer (naming the
    // active tools + pointing at `nxs prime`). The memory rule (forbid MEMORY.md, mandate
    // `nxm remember`) + the command list now live in `nxm prime`, NOT AGENTS.md.
    let agents = read(&tmp.path().join("AGENTS.md"));
    assert!(
        agents.contains("<!-- BEGIN NEXUS "),
        "single nxs-owned pointer: {agents}"
    );
    assert!(
        agents.contains("(memory)"),
        "names the active tool: {agents}"
    );
    assert!(
        agents.contains("nxs prime"),
        "points at the umbrella: {agents}"
    );
    assert!(
        !agents.contains("BEGIN NEXUS-MEMORY"),
        "no per-module NEXUS-MEMORY block any more: {agents}"
    );
    // The memory RULE ("durable knowledge only via `nxm remember`, never a hand-kept MEMORY.md")
    // lives in `nxm prime`, not here. What AGENTS.md does carry since 6j6v.8q88 is the BINDING to
    // the generated `NEXUS_MEMORY.md` — a pointer to a file, not a module's operating prose — so
    // the guard names the rule rather than matching any path that ends in `memory.md`.
    assert!(
        !agents.to_lowercase().contains(" memory.md")
            && !agents.to_lowercase().contains("durable knowledge"),
        "the memory rule lives in `nxm prime`, not AGENTS.md: {agents}"
    );
    assert!(
        agents.contains("@NEXUS_MEMORY.md"),
        "the generated project-memory document is bound for agents that read no hooks: {agents}"
    );

    // P3-S5, **superseded 2026-08-28 (nxf n2m6 + a2a1)**: this used to assert the single
    // SessionStart hook → `nxs prime` (the umbrella fan-out), "NOT `nxm prime` (that is only the
    // fan-out target the manifest declares)". The host truncates per hook OUTPUT, so the wiring is
    // now one entry per active module and `nxm prime` is exactly what a memory-only workspace
    // wires. Being the only (hence first) entry, it carries the `|| cat NEXUS_MEMORY.md` fallback.
    let settings = json_of(read(&tmp.path().join(".claude/settings.json")).as_bytes());
    let cmds: Vec<String> = settings["hooks"]["SessionStart"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["hooks"].as_array().unwrap().clone())
        .filter_map(|h| h["command"].as_str().map(String::from))
        .collect();
    assert_eq!(
        cmds,
        vec![format!("nxm prime{}", nxs_init::assembler::HOOK_FALLBACK)]
    );
    assert_eq!(v["hook"]["hook_added"], true);
    assert_eq!(v["hook"]["commands"], serde_json::json!(cmds));

    // aye.31: a memory-only init leaves flow addable + inactive → the additive `advertisement`
    // field cross-sells it (agent-addressed; no setup without the user's permission).
    let ad = v["advertisement"]
        .as_str()
        .expect("advertisement field present when flow is inactive");
    assert!(ad.contains("issue tracker"), "cross-sells flow: {ad}");
    assert!(ad.contains("nxs init"), "points at nxs init: {ad}");
}

#[test]
fn init_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    nxm(tmp.path()).arg("init").assert().success();
    let agents_first = read(&tmp.path().join("AGENTS.md"));
    let settings_first = read(&tmp.path().join(".claude/settings.json"));

    // A second init must not duplicate the block or the hook.
    let out = nxm(tmp.path()).args(["--json", "init"]).assert().success();
    assert_eq!(
        json_of(&out.get_output().stdout)["hook"]["hook_added"],
        false
    );
    assert_eq!(
        read(&tmp.path().join("AGENTS.md")),
        agents_first,
        "block stable"
    );
    assert_eq!(
        read(&tmp.path().join(".claude/settings.json")),
        settings_first,
        "settings stable"
    );
    assert_eq!(
        agents_first.matches("<!-- BEGIN NEXUS ").count(),
        1,
        "one nxs-owned pointer"
    );
}

#[test]
fn agent_manifest_declares_nxms_contribution() {
    let tmp = TempDir::new().unwrap();
    // Static contribution — no workspace needed.
    let out = nxm(tmp.path())
        .args(["--json", "agent-manifest"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["prime_command"], "nxm prime");
    assert_eq!(v["hook"]["event"], "SessionStart");
    assert_eq!(v["hook"]["command"], "nxm prime");
    // Since nexus-flow-0lj.2 the manifest carries no per-module Markdown block — just the prime
    // fan-out target + the hook (the umbrella writes the single nxs-owned AGENTS.md pointer).
    assert!(
        v.get("agents_section").is_none() && v.get("claude_section").is_none(),
        "no per-module section fields remain on the manifest: {v}"
    );
    assert!(
        !tmp.path().join(".nxs").exists(),
        "agent-manifest creates no workspace"
    );
}

#[test]
fn prime_states_the_rule_lists_commands_and_replays_memories() {
    let tmp = TempDir::new().unwrap();
    nxm(tmp.path()).arg("init").assert().success();
    nxm(tmp.path())
        .args([
            "remember",
            "auth uses JWT",
            "--key",
            "auth-jwt",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    nxm(tmp.path())
        .args([
            "remember",
            "Dolt phantoms",
            "--key",
            "dolt-phantoms",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();

    let out = nxm(tmp.path()).arg("prime").assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout);

    // (1) the memory rule — explicitly forbids MEMORY.md, mandates nxm remember.
    //
    // **Superseded 2026-08-28 (nxf q065, task 4).** The human view no longer carries
    // `facade::PRIME_MEMORY_RULE` (that constant is unchanged and still rides `--json`'s
    // `memory_rule`, see that constant's own doc comment) — it opens with `PRIME_INTRO` instead,
    // which restates the same prohibition without the CRITICAL/NEVER-caps emphasis or the word
    // "only". The assertions below were rewritten against what the human view ACTUALLY says now,
    // not against the retired wording.
    assert!(
        text.to_lowercase().contains("memory.md"),
        "forbids MEMORY.md: {text}"
    );
    assert!(
        text.contains("Never write a `MEMORY.md`") && text.contains("nxm remember"),
        "mandates remember: {text}"
    );
    assert!(
        text.contains("never replayed"),
        "rule spells out the consequence (a MEMORY.md is never replayed): {text}"
    );
    assert!(
        text.contains("one durable channel"),
        "rule still says `nxm remember` is the one channel, even without the word \"only\": {text}"
    );
    // (2) the key commands.
    assert!(
        text.contains("nxm recall") && text.contains("nxm forget"),
        "lists commands: {text}"
    );
    // (3) every active memory as exactly ONE line — its key and the introduction its author
    //     wrote (6j6v.xbnh). The bodies are NOT here: that is the whole saving, and `nxm recall`
    //     is what fetches one. "Every" holds at this fixture's size and is what this case checks;
    //     the block bounds the index by a byte budget since 6j6v.5jm3, and what happens past it —
    //     the leading lines, the excerpt heading, the pointer to `nxm index` — is pinned in
    //     `facade`'s own tests, where the arithmetic can be provoked without an 80-memory CLI
    //     fixture.
    assert!(text.contains("- **auth-jwt**: in one line"), "{text}");
    assert!(text.contains("- **dolt-phantoms**: in one line"), "{text}");
    assert!(
        !text.contains("auth uses JWT") && !text.contains("Dolt phantoms"),
        "no body is replayed: {text}"
    );
    // Memories appear in key order.
    let a = text.find("auth-jwt").unwrap();
    let d = text.find("dolt-phantoms").unwrap();
    assert!(a < d, "key-sorted");
}

#[test]
fn prime_json_carries_memories_and_count() {
    let tmp = TempDir::new().unwrap();
    nxm(tmp.path()).arg("init").assert().success();
    nxm(tmp.path())
        .args([
            "remember",
            "x",
            "--key",
            "k",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();
    let out = nxm(tmp.path()).args(["--json", "prime"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["count"], 1);
    assert_eq!(v["memories"][0]["key"], "k");
    assert_eq!(v["memories"][0]["body"], "x");
}

#[test]
fn prime_with_no_memories_invites_capture() {
    let tmp = TempDir::new().unwrap();
    nxm(tmp.path()).arg("init").assert().success();
    let out = nxm(tmp.path()).arg("prime").assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout);
    assert!(text.contains("## Memories (0)"));
    assert!(
        text.to_lowercase().contains("capture"),
        "invites capture: {text}"
    );
}

#[test]
fn init_quiet_prints_nothing_yet_still_wires_workspace_and_one_hook() {
    // Driven/quiet mode (TB-10, S5): `nxm init --quiet` renders no banner (so no sub-init output
    // bleeds through when the umbrella drives it) while still wiring the module's own SessionStart
    // hook (one per active module since nxf n2m6 + a2a1).
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join(".fake-home");
    let out = nxm(tmp.path())
        .args(["init", "--quiet"])
        .pin_home(&home)
        .assert()
        .success();
    assert!(
        out.get_output().stdout.is_empty(),
        "quiet init prints nothing to stdout: {:?}",
        String::from_utf8_lossy(&out.get_output().stdout)
    );
    assert!(
        out.get_output().stderr.is_empty(),
        "nor anything to stderr — the umbrella owns every line here, including the ones the \
         registration below could make (nxf 6j6v.y12q): {:?}",
        String::from_utf8_lossy(&out.get_output().stderr)
    );
    assert!(
        tmp.path().join(".nxs/db.sqlite").is_file(),
        "workspace still created"
    );
    // nxf 6j6v.y12q: SILENT is not the same as skipped. The driven seam registers the workspace
    // like every other init path — what `--quiet` takes away is the saying, not the doing.
    let registry = read(&home.join(".nexusflow").join("workspaces.toml"));
    assert!(
        registry.contains(tmp.path().canonicalize().unwrap().to_str().unwrap()),
        "the quiet path still puts the workspace on the service's list: {registry}"
    );
    let settings = json_of(read(&tmp.path().join(".claude/settings.json")).as_bytes());
    let cmds: Vec<String> = settings["hooks"]["SessionStart"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["hooks"].as_array().unwrap().clone())
        .filter_map(|h| h["command"].as_str().map(String::from))
        .collect();
    assert_eq!(
        cmds,
        vec![format!("nxm prime{}", nxs_init::assembler::HOOK_FALLBACK)],
        "the module's own hook is wired even when quiet"
    );
}
