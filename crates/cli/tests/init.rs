//! Integration tests for `nxf init` and workspace discovery (E2.2, 4oa.4, ha4).
//!
//! `assert_cmd` runs the binary non-interactively (stdin is not a TTY), so these exercise
//! the non-interactive contract: `--plugin` is required, validated, and drives `config.toml`;
//! a fresh init self-ignores `.nxs/`. The interactive numbered chooser is hard to drive
//! headlessly and is covered by a pure unit test (`render_plugin_choices`) in `commands`.

use assert_cmd::Command;
use nxs_test_support::PinHome;
use std::fs;
use std::process::Command as StdCommand;
use tempfile::TempDir;

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

#[test]
fn init_with_plugin_creates_workspace_files() {
    let tmp = TempDir::new().unwrap();
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let ws = tmp.path().join(".nxs");
    assert!(ws.join("db.sqlite").exists(), "db.sqlite created");
    assert!(ws.join("replica.toml").exists(), "replica.toml created");
    assert!(ws.join("config.toml").exists(), "config.toml created");

    // replica.toml carries a site_id and a prefix; config.toml selects a plugin.
    let replica = fs::read_to_string(ws.join("replica.toml")).unwrap();
    assert!(
        replica.contains("site_id"),
        "replica has site_id: {replica}"
    );
    assert!(replica.contains("prefix"), "replica has prefix: {replica}");
    let config = fs::read_to_string(ws.join("config.toml")).unwrap();
    assert!(
        config.contains("issue-tracker"),
        "config selects the chosen plugin: {config}"
    );
}

/// nxf 6j6v.y12q: the workspace an AGENT sets up gets a clock like every other.
///
/// `nxs init` has registered the workspace it creates since 6j6v.npf9, and `--json`/`--quiet` — the
/// way an agent enters — never reaches `nxs init`: it runs flow's own init and stops there. Since
/// 6j6v.8see the service's list is where a workspace's deadlines come from, so this path produced a
/// workspace with no clock and nothing said so.
#[test]
fn init_puts_the_workspace_on_the_list_the_background_service_attends() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join(".fake-home");
    let out = nxf()
        .args(["init", "--plugin", "issue-tracker", "--json"])
        .current_dir(tmp.path())
        .pin_home(&home)
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("json output");
    assert_eq!(
        v["service"]["registered"],
        serde_json::json!(true),
        "and it SAYS so, rather than leaving an agent to find out: {v}"
    );

    let registry = fs::read_to_string(home.join(".nexusflow").join("workspaces.toml"))
        .expect("the service's registry — the list that gives a workspace its clock");
    let root = tmp.path().canonicalize().unwrap();
    assert!(
        registry.contains(root.to_str().unwrap()),
        "this workspace must be on the list: {registry}"
    );
}

/// The `service:` line on the HUMAN surface (review of PR #421, Test Quality #1).
///
/// `nxm`'s identical line is pinned byte-for-byte by three golden corpora; `nxf`'s had nothing —
/// every golden invocation of `nxf init` passes `--json`, so a dropped or garbled `println!` here
/// would have gone unnoticed. Asserted rather than goldened because the goldens all seed their
/// sandbox with the machine-readable form, and adding a human one would churn every id in them.
#[test]
fn init_tells_a_human_that_the_workspace_is_registered_with_the_service() {
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .pin_home(tmp.path().join(".fake-home"))
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.contains("service: registered with the background service"),
        "the summary must say the workspace got a clock: {stdout}"
    );
    assert!(
        stdout.contains("deadlines"),
        "and name what that is FOR — `registered` alone says nothing: {stdout}"
    );
}

#[test]
fn init_without_plugin_non_tty_fails_and_lists_plugins_with_descriptions() {
    // Non-interactive (assert_cmd) + no `--plugin` must fail loudly (4oa.4: no silent
    // default) and print the valid plugins WITH their descriptions.
    let tmp = TempDir::new().unwrap();
    let out = nxf().arg("init").current_dir(tmp.path()).assert().failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr);
    assert!(
        stderr.contains("--plugin is required"),
        "explains the missing required flag: {stderr}"
    );
    assert!(
        stderr.contains("issue-tracker") && stderr.contains("personal-todo"),
        "lists both plugin names: {stderr}"
    );
    // A description (not just the bare name) is shown for each.
    assert!(
        stderr.contains("issue tracking") || stderr.contains("Software issue"),
        "shows the issue-tracker description: {stderr}"
    );
    assert!(
        stderr.to_lowercase().contains("to-do") || stderr.to_lowercase().contains("todo"),
        "shows the personal-todo description: {stderr}"
    );
    // No workspace was created on the failed path.
    assert!(!tmp.path().join(".nxs").exists());
}

#[test]
fn init_json_without_plugin_emits_validation_error_to_stdout() {
    // Under `--json` (a GLOBAL flag), `NxfError::emit` routes the structured envelope to
    // stdout (the agent-facing contract), not stderr. The missing-plugin failure must still
    // surface there with `kind == "validation"`.
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["--json", "init"])
        .current_dir(tmp.path())
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("error envelope is json on stdout");
    assert_eq!(
        v["error"]["kind"], "validation",
        "missing --plugin is a validation error: {stdout}"
    );
    assert!(!tmp.path().join(".nxs").exists(), "nothing created");
}

#[test]
fn init_json_unknown_plugin_emits_validation_error_to_stdout() {
    // Same contract for an unknown `--plugin`: the structured envelope goes to stdout.
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["--json", "init", "--plugin", "bogus"])
        .current_dir(tmp.path())
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("error envelope is json on stdout");
    assert_eq!(
        v["error"]["kind"], "validation",
        "unknown plugin is a validation error: {stdout}"
    );
    assert!(!tmp.path().join(".nxs").exists(), "nothing created");
}

#[test]
fn init_with_chosen_plugin_writes_that_plugin_into_config() {
    let tmp = TempDir::new().unwrap();
    nxf()
        .args(["init", "--plugin", "personal-todo"])
        .current_dir(tmp.path())
        .assert()
        .success();
    let config = fs::read_to_string(tmp.path().join(".nxs/config.toml")).unwrap();
    assert!(
        config.contains("personal-todo"),
        "config selects personal-todo: {config}"
    );
}

#[test]
fn init_with_unknown_plugin_is_a_validation_error() {
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["init", "--plugin", "bogus"])
        .current_dir(tmp.path())
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr);
    assert!(
        stderr.contains("unknown plugin"),
        "rejects an unknown plugin: {stderr}"
    );
    assert!(!tmp.path().join(".nxs").exists(), "nothing created");
}

#[test]
fn init_json_includes_prefix_plugin_and_next_steps() {
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["--json", "init", "--plugin", "personal-todo"])
        .current_dir(tmp.path())
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("init emits json");
    assert_eq!(v["ok"], true);
    assert_eq!(v["plugin"], "personal-todo");
    assert!(v["prefix"].as_str().is_some_and(|p| !p.is_empty()));
    let steps = v["next_steps"].as_array().expect("next_steps is an array");
    assert!(!steps.is_empty(), "next_steps is non-empty");
    // Each step carries a copy-pasteable command.
    assert!(steps.iter().any(|s| s["command"]
        .as_str()
        .is_some_and(|c| c.contains("nxf create"))));
}

#[test]
fn init_json_includes_the_advertisement_field_when_a_module_is_inactive() {
    // aye.31: a flow-only init leaves memory addable + inactive, so the additive `advertisement`
    // field carries agent-addressed copy — make the user aware, set nothing up without permission.
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["--json", "init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim()).unwrap();
    let ad = v["advertisement"]
        .as_str()
        .expect("advertisement field present");
    assert!(ad.contains("nexus-memory"), "cross-sells memory: {ad}");
    assert!(ad.contains("nxs init"), "points at nxs init: {ad}");
    assert!(
        ad.to_lowercase().contains("permission"),
        "instructs not to set up without permission: {ad}"
    );
    // The pre-existing contract is intact (additive field only).
    assert_eq!(v["ok"], true);
    assert_eq!(v["plugin"], "issue-tracker");
}

#[test]
fn init_human_prints_the_upsell_cta() {
    // Interactive (non-quiet) init ends with a CTA pointing at `nxs init`. Under assert_cmd (no
    // TTY) the manufakt theme is plain, so the CTA is byte-stable plain text.
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    assert!(
        stdout.contains("nxs init"),
        "human init shows the upsell CTA: {stdout}"
    );
}

#[test]
fn init_self_ignores_the_workspace_dir() {
    // ha4: `.nxs/.gitignore` must exist and contain `*`.
    let tmp = TempDir::new().unwrap();
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success();
    let gitignore = tmp.path().join(".nxs/.gitignore");
    assert!(gitignore.is_file(), ".nxs/.gitignore exists");
    assert_eq!(fs::read_to_string(gitignore).unwrap(), "*\n");
}

#[test]
fn init_inside_a_git_repo_leaves_no_untracked_nexusflow() {
    // ha4 acceptance: a fresh init inside a git repo shows no untracked `.nxs` entries.
    let tmp = TempDir::new().unwrap();
    let git_init = StdCommand::new("git")
        .args(["init"])
        .current_dir(tmp.path())
        .output()
        .expect("running `git init` (git must be available)");
    assert!(
        git_init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&git_init.stderr)
    );

    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let porcelain = StdCommand::new("git")
        .args(["status", "--porcelain"])
        .current_dir(tmp.path())
        .output()
        .expect("git status");
    let status = String::from_utf8_lossy(&porcelain.stdout);
    assert!(
        !status.contains(".nxs"),
        "no untracked .nxs entries; git status was:\n{status}"
    );
}

#[test]
fn init_writes_the_single_nxs_owned_discovery_pointer() {
    // nexus-flow-0lj.2: `nxf init` writes ONE nxs-owned discovery pointer (naming the active tools +
    // pointing at `nxs prime`) — NOT a per-module `NEXUS-FLOW` block. Operating instructions and the
    // compaction-recovery hint live in `nxs prime`, not AGENTS.md.
    let tmp = TempDir::new().unwrap();
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success();
    let agents = tmp.path().join("AGENTS.md");
    assert!(agents.is_file(), "init creates AGENTS.md");
    let body = fs::read_to_string(&agents).unwrap();
    assert!(
        body.contains("<!-- BEGIN NEXUS ") && body.contains("<!-- END NEXUS -->"),
        "the single nxs-owned NEXUS pointer is present: {body}"
    );
    assert!(
        !body.contains("BEGIN NEXUS-FLOW"),
        "no per-module NEXUS-FLOW block any more: {body}"
    );
    assert!(
        body.to_lowercase().contains("do not edit") || body.to_lowercase().contains("managed"),
        "block marks itself managed: {body}"
    );
    assert!(
        body.contains("(flow)"),
        "names the active tool for discovery: {body}"
    );
    assert!(
        body.contains("nxs prime"),
        "points the agent at the umbrella `nxs prime`: {body}"
    );
    assert!(
        !body.contains("nxf prime"),
        "assembled AGENTS.md must not surface the per-module verb `nxf prime`: {body}"
    );
    // No CLAUDE.md was conjured when none existed.
    assert!(
        !tmp.path().join("CLAUDE.md").exists(),
        "no CLAUDE.md created"
    );
}

#[test]
fn init_json_reports_onboarding() {
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["--json", "init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v["onboarding"]["agents"], "created");
    assert_eq!(v["onboarding"]["claude"], "absent");
}

#[test]
fn init_leaves_a_preexisting_claude_md_exactly_as_it_found_it() {
    // nxf 6j6v.q6e3: `CLAUDE.md` is the file of the host that RUNS the SessionStart hook this same
    // command wires, so `init` puts nothing in it — neither the managed block nor an `@AGENTS.md`
    // import, which would pull the block in by another door together with its `@NEXUS_MEMORY.md`
    // line: an import of every memory's BODY, beside the budgeted index the hook just delivered.
    let tmp = TempDir::new().unwrap();
    let mine = "# CLAUDE\n\nProject notes.\n";
    fs::write(tmp.path().join("CLAUDE.md"), mine).unwrap();
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap(),
        mine,
        "byte-for-byte"
    );
    // …while the file that IS the assembler's carries the block, memory binding and all.
    let agents = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
    assert!(
        agents.contains("<!-- BEGIN NEXUS "),
        "the pointer goes to AGENTS.md: {agents}"
    );
}

#[test]
fn init_takes_a_retired_block_back_out_of_an_existing_claude_md() {
    // The migration half of the same ticket: the block is marked "managed, regenerated by
    // `nxs init`", so an owner who deletes it by hand has it back on the next init of any module.
    // Only the tool can retire what the tool writes — so an ordinary run does.
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("AGENTS.md"), "# shared\n").unwrap();
    fs::write(
        tmp.path().join("CLAUDE.md"),
        "# mine\n\nkeep me.\n\n<!-- BEGIN NEXUS v:1 — managed, do not edit (regenerated by \
         `nxs init`) -->\n## nexus-flow tools for agents\n\nold text\n<!-- END NEXUS -->\n",
    )
    .unwrap();
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success();
    let claude = fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap();
    assert!(
        !claude.contains("BEGIN NEXUS"),
        "the retired block is gone: {claude}"
    );
    assert!(
        claude.contains("# mine") && claude.contains("keep me."),
        "and nothing of the owner's went with it: {claude}"
    );
}

#[test]
fn init_warns_about_an_orphaned_legacy_dir() {
    // aye.2.3: a partial legacy `.nexusflow/` (db.sqlite but no replica.toml — a crashed init) is
    // not migrated. `nxf init` must warn (to stderr) before creating a fresh `.nxs/`, so the stale
    // db one dir over is not silently stranded; the fresh workspace is still created and the orphan
    // is left untouched (we warn, we don't delete).
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join(".nexusflow");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("db.sqlite"), b"stale-db-bytes").unwrap();

    let out = nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr);
    assert!(
        stderr.contains(".nexusflow") && stderr.to_lowercase().contains("orphan"),
        "warns about the orphaned legacy dir: {stderr}"
    );
    assert!(
        tmp.path().join(".nxs").join("db.sqlite").exists(),
        "fresh workspace still created"
    );
    assert!(
        legacy.join("db.sqlite").exists(),
        "orphan left untouched (warned, not deleted)"
    );
}

#[test]
fn init_does_not_warn_without_an_orphaned_legacy_dir() {
    // The common case: a clean dir → no spurious orphan warning on stderr.
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr);
    assert!(
        !stderr.to_lowercase().contains("orphan"),
        "no orphan warning on a clean init: {stderr}"
    );
}

#[test]
fn second_init_in_same_tree_is_rejected() {
    let tmp = TempDir::new().unwrap();
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // A second init anywhere under the same workspace must fail loudly.
    let sub = tmp.path().join("sub");
    fs::create_dir(&sub).unwrap();
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(&sub)
        .assert()
        .failure();
}

#[test]
fn init_quiet_prints_nothing_yet_still_wires_workspace_and_one_hook() {
    // Driven/quiet mode (TB-10, S5): the seam `nxs init` uses to drive `nxf init` — it must render
    // NO banner (so no sub-init output bleeds through) while still doing the full setup + wiring
    // the module's own SessionStart hook (one per active module since nxf n2m6 + a2a1).
    let tmp = TempDir::new().unwrap();
    let out = nxf()
        .args(["init", "--plugin", "issue-tracker", "--quiet"])
        .current_dir(tmp.path())
        .assert()
        .success();
    assert!(
        out.get_output().stdout.is_empty(),
        "quiet init prints nothing to stdout: {:?}",
        String::from_utf8_lossy(&out.get_output().stdout)
    );
    // …yet the workspace exists and flow's own SessionStart hook is wired.
    assert!(
        tmp.path().join(".nxs/db.sqlite").is_file(),
        "workspace still created"
    );
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap(),
    )
    .unwrap();
    let cmds: Vec<String> = settings["hooks"]["SessionStart"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["hooks"].as_array().unwrap().clone())
        .filter_map(|h| h["command"].as_str().map(String::from))
        .collect();
    assert_eq!(
        cmds,
        vec!["nxf prime".to_string()],
        "the module's own hook is wired even when quiet (nxf n2m6: one entry per active module) \
         — and it carries no `|| cat` tail: since nxf 6j6v.1k6y that belongs to memory's entry, \
         and a flow-only workspace has no `NEXUS_MEMORY.md` to read"
    );
}

#[test]
fn init_quiet_without_plugin_uses_flows_default() {
    // Driven mode (aye.29): the umbrella `nxs init` drives `nxf init --quiet` without knowing
    // flow's plugin vocabulary (it is roster-only). So quiet WITHOUT `--plugin` must NOT error like
    // the interactive/json paths — flow picks its OWN sensible default (issue-tracker). This keeps
    // plugin knowledge in flow; `nxf init` (typed directly) still prompts / requires the choice.
    let tmp = TempDir::new().unwrap();
    nxf()
        .args(["init", "--quiet"])
        .current_dir(tmp.path())
        .assert()
        .success();
    let config = fs::read_to_string(tmp.path().join(".nxs/config.toml")).unwrap();
    assert!(
        config.contains("issue-tracker"),
        "quiet-without-plugin seats flow's default plugin: {config}"
    );
}

#[test]
fn flow_example_type_resolves_the_active_plugins_root_type_from_a_real_workspace() {
    // nexus-flow-92zt (PR-review Test Quality #3): the `nxs init` "your first move" banner fills its
    // `{type}` slot from `flow_example_type`, which discovers a REAL workspace and loads its plugin.
    // Drive the actual `nxf init` to materialize the workspace on disk, then assert the resolution
    // end-to-end — the piece the pure `first_commands` substitution unit test cannot cover.
    for (plugin, want) in [("issue-tracker", "epic"), ("personal-todo", "project")] {
        let tmp = TempDir::new().unwrap();
        nxf()
            .args(["init", "--plugin", plugin])
            .current_dir(tmp.path())
            .assert()
            .success();
        assert_eq!(
            nexus_flow_cli::flow_example_type(tmp.path()).as_deref(),
            Some(want),
            "{plugin}: banner resolves its declared root type from the real workspace"
        );
    }
    // A directory with no workspace resolves to None — the banner then renders its safe fallback.
    let empty = TempDir::new().unwrap();
    assert_eq!(nexus_flow_cli::flow_example_type(empty.path()), None);
}
