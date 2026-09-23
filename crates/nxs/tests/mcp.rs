//! MCP seam (E9 #76u) — the read-only vertical, driven end-to-end by a real MCP client.
//!
//! These tests spawn the actual `nxs mcp serve` binary over stdio and connect with the `rmcp`
//! client (the same protocol a host like Claude Desktop speaks), so they exercise the whole seam:
//! the `nxs mcp serve` command wiring, the rmcp handshake, and the flow read tools over the shared
//! `.nxs/` store. Gated on the `mcp` feature — the lean build has no server to talk to.
#![cfg(feature = "mcp")]

use assert_cmd::cargo::cargo_bin;
use nxs_test_support::PinHome;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use tempfile::TempDir;
use tokio::process::Command;

/// A pinned reference time so every server the tests spawn derives `ready`/`blocked`/`next` against
/// the same clock (and the prime fan-out is byte-stable).
const NOW: &str = "2026-06-22T12:00:00Z";

/// The full flow tool surface after the write-tools slices: the three read tools, the nine
/// #76u.11 write tools, and the four #76u.12 write tools (contributes add/remove + archive/unarchive).
/// The single source the surface-contract + annotation tests assert against.
const FLOW_TOOLS: &[&str] = &[
    // read (#4ti / #76u.9 / #76u.10)
    "flow_list",
    "flow_next",
    "flow_show",
    // read (#nes — the remaining read tools)
    "flow_blocked",
    "flow_search",
    "flow_note_list",
    "flow_mention_list",
    "flow_schema",
    // write (#wpw / #yie)
    "flow_create",
    "flow_update",
    "flow_claim",
    "flow_close",
    "flow_dep_add",
    "flow_dep_remove",
    "flow_mention_add",
    "flow_mention_remove",
    "flow_note_add",
    // write (#76u.12)
    "flow_contributes_add",
    "flow_contributes_remove",
    "flow_archive",
    "flow_unarchive",
    // labels (h89s.3)
    "flow_label_add",
    "flow_label_remove",
    "flow_label_list",
];

/// The always-on UMBRELLA tools (0jq8) — not flow ops, but part of the base surface on every
/// workspace: `list_workspaces` reads the global `~/.nexusflow/` registry, independent of the active
/// modules.
const UMBRELLA_TOOLS: &[&str] = &["list_workspaces"];

/// Create a flow-only workspace fixture (issue-tracker plugin) in a fresh tempdir.
fn flow_fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let status = StdCommand::new(cargo_bin("nxf"))
        .current_dir(tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .env("NXF_DETERMINISTIC_IDS", "1")
        .status()
        .expect("spawn nxf init");
    assert!(status.success(), "nxf init should set up a flow workspace");
    tmp
}

/// Run an `nxf` command against the fixture (pinned clock + deterministic ids).
fn nxf(dir: &Path, args: &[&str]) {
    let status = StdCommand::new(cargo_bin("nxf"))
        .current_dir(dir)
        .args(args)
        .env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXF_NOW", NOW)
        .status()
        .expect("spawn nxf");
    assert!(status.success(), "nxf {args:?} should succeed");
}

/// Build a tool-call with a JSON object of arguments.
fn call_with(name: &'static str, args: Value) -> CallToolRequestParams {
    let obj: Map<String, Value> = args.as_object().cloned().unwrap_or_default();
    CallToolRequestParams::new(name).with_arguments(obj)
}

/// Run `nxf --json <args>` against the fixture (pinned clock) and parse its stdout.
/// Drop the CLI-only additive presentation labels (ee2h: `priority_label`/`type_label`, which
/// `nxf show`/`list`/`next --json` decorate the record with). The MCP `structuredContent` seam
/// deliberately stays the plugin-independent canonical record — adding the labels there is a
/// separate contract decision with its own parity-gate update (out of ee2h scope) — so the
/// conformance comparison is against the CLI's canonical record, not its presentation decoration.
/// Recurses so it also reaches the record `show --json` nests under `item`.
fn without_presentation_labels(v: &Value) -> Value {
    match v {
        Value::Array(xs) => Value::Array(xs.iter().map(without_presentation_labels).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .filter(|(k, _)| k.as_str() != "priority_label" && k.as_str() != "type_label")
                .map(|(k, val)| (k.clone(), without_presentation_labels(val)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn nxf_json(dir: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let out = StdCommand::new(cargo_bin("nxf"))
        .current_dir(dir)
        .args(&full)
        .env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXF_NOW", NOW)
        .output()
        .expect("spawn nxf --json");
    assert!(
        out.status.success(),
        "nxf --json {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("nxf --json output parses")
}

/// Call a read tool and return its (non-error) structuredContent.
async fn tool_sc(
    client: &RunningService<RoleClient, ()>,
    name: &'static str,
    args: Value,
) -> Value {
    let res = client
        .call_tool(call_with(name, args))
        .await
        .unwrap_or_else(|e| panic!("{name} call: {e}"));
    assert_ne!(res.is_error, Some(true), "{name} should not be an error");
    res.structured_content
        .unwrap_or_else(|| panic!("{name} returns structuredContent"))
}

/// Spawn `nxs mcp serve --workspace <dir>` and connect an MCP client over stdio.
async fn connect(dir: &Path) -> RunningService<RoleClient, ()> {
    let mut cmd = Command::new(cargo_bin("nxs"));
    cmd.arg("mcp")
        .arg("serve")
        .arg("--workspace")
        .arg(dir)
        .env("NXS_NOW", NOW)
        .env("NXF_NOW", NOW)
        // Mint sequential ids in the served store too, so a write tool (`flow_create`/`flow_note_add`)
        // produces the SAME id as `nxf <cmd>` on an identically-seeded fixture — the write parity
        // tests compare receipts across the two seams (#ei9).
        .env("NXF_DETERMINISTIC_IDS", "1");
    let transport = TokioChildProcess::new(cmd).expect("spawn nxs mcp serve");
    ().serve(transport)
        .await
        .expect("client connects and the initialize handshake succeeds")
}

/// Spawn `nxs mcp serve` with NEITHER `--workspace` NOR `--db`, under a controlled `HOME` so the
/// App-Data-Home (#76u.7) resolves inside `home`, and a `cwd` that holds its OWN `.nxs/` (to prove
/// cwd is NOT discovered). XDG vars are cleared so the Linux data dir falls back under `HOME` too.
async fn connect_default(home: &Path, cwd: &Path) -> RunningService<RoleClient, ()> {
    let mut cmd = StdCommand::new(cargo_bin("nxs"));
    cmd.arg("mcp")
        .arg("serve")
        .current_dir(cwd)
        .pin_home(home)
        .env_remove("NXS_WORKSPACE")
        .env_remove("NXS_DB")
        .env("NXS_NOW", NOW)
        .env("NXF_NOW", NOW);
    let transport = TokioChildProcess::new(Command::from(cmd)).expect("spawn nxs mcp serve");
    ().serve(transport)
        .await
        .expect("client connects and the initialize handshake succeeds")
}

/// Spawn `nxs mcp serve --workspace <dir> --actor <actor>` (the launch-default actor, #zxj) and
/// connect. `NXS_ACTOR`/`NXF_ACTOR` are cleared so the assertion proves the `--actor` launch flag —
/// not an ambient env identity — is what lands on a write with no per-tool actor.
async fn connect_with_actor(dir: &Path, actor: &str) -> RunningService<RoleClient, ()> {
    let mut cmd = Command::new(cargo_bin("nxs"));
    cmd.arg("mcp")
        .arg("serve")
        .arg("--workspace")
        .arg(dir)
        .arg("--actor")
        .arg(actor)
        .env("NXS_NOW", NOW)
        .env("NXF_NOW", NOW)
        .env("NXF_DETERMINISTIC_IDS", "1")
        .env_remove("NXS_ACTOR")
        .env_remove("NXF_ACTOR");
    let transport = TokioChildProcess::new(cmd).expect("spawn nxs mcp serve");
    ().serve(transport)
        .await
        .expect("client connects and the initialize handshake succeeds")
}

/// Spawn the server with NO `--actor` but an `NXF_ACTOR` env identity set (and `NXS_ACTOR` cleared),
/// so a write with no per-tool/launch actor proves the bottom hybrid tier — the `NXF_ACTOR`/`USER`
/// env fallback (#zxj) — lands in `op.author` through a real persisted write.
async fn connect_with_env_actor(dir: &Path, nxf_actor: &str) -> RunningService<RoleClient, ()> {
    let mut cmd = Command::new(cargo_bin("nxs"));
    cmd.arg("mcp")
        .arg("serve")
        .arg("--workspace")
        .arg(dir)
        .env("NXS_NOW", NOW)
        .env("NXF_NOW", NOW)
        .env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXF_ACTOR", nxf_actor)
        .env_remove("NXS_ACTOR");
    let transport = TokioChildProcess::new(cmd).expect("spawn nxs mcp serve");
    ().serve(transport)
        .await
        .expect("client connects and the initialize handshake succeeds")
}

/// Spawn `nxs mcp serve` under a controlled `HOME` (XDG cleared) so the global
/// `~/.nexusflow/workspaces.toml` registry AND the auto-init'd App-Data-Home default both resolve
/// inside `home` — for the umbrella `list_workspaces` tests, which read the registry independent of
/// any served board (0jq8). No `--workspace`: the served board is irrelevant to the registry read.
async fn connect_home(home: &Path) -> RunningService<RoleClient, ()> {
    let mut cmd = StdCommand::new(cargo_bin("nxs"));
    cmd.arg("mcp")
        .arg("serve")
        .pin_home(home)
        .env_remove("NXS_WORKSPACE")
        .env_remove("NXS_DB")
        .env("NXS_NOW", NOW)
        .env("NXF_NOW", NOW);
    let transport = TokioChildProcess::new(Command::from(cmd)).expect("spawn nxs mcp serve");
    ().serve(transport)
        .await
        .expect("client connects and the initialize handshake succeeds")
}

/// Drive a write parity case (#ei9): apply `tool`+`args` to an MCP fixture and the matching
/// `nxf <cli>` to an IDENTICALLY-seeded CLI fixture, and return both receipts. The two fixtures must
/// already hold the same state (seeded the same), with `now`/`actor` pinned on both sides, so any
/// difference is a real seam divergence — not clock/id noise.
async fn write_receipts(
    mcp_fix: &Path,
    cli_fix: &Path,
    tool: &'static str,
    args: Value,
    cli: &[&str],
) -> (Value, Value) {
    let client = connect(mcp_fix).await;
    let tool_receipt = tool_sc(&client, tool, args).await;
    client.cancel().await.expect("clean shutdown");
    let cli_receipt = nxf_json(cli_fix, cli);
    (tool_receipt, cli_receipt)
}

/// The `author` recorded on every op in the workspace store — lets a test assert the actor hybrid
/// (#zxj) threads the right identity onto a write (the receipt does not carry the author).
fn op_authors(dir: &Path) -> Vec<String> {
    use nexus_flow_facade::workspace::{Workspace, WorkspaceExt};
    let ws = Workspace::resolve(None, dir).expect("resolve workspace");
    let store = ws.open_store().expect("open store");
    store.export().iter().map(|o| o.author.clone()).collect()
}

/// Every `flow_*` / `memory_*` identifier token in `s` (the seam's tool-name convention) — used to
/// cross-check rendered instructions against the live tools/list (ctm). Byte-indexed scan: the
/// prefixes + identifier chars are ASCII, and any non-ASCII byte is skipped by char width.
fn tool_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let prefix = ["flow_", "memory_"]
            .into_iter()
            .find(|p| rest.starts_with(p));
        if let Some(p) = prefix {
            let after = &rest[p.len()..];
            let len = after
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            out.push(format!("{p}{}", &after[..len]));
            i += p.len() + len;
        } else {
            i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
        }
    }
    out
}

/// Find the first `.nxs/` workspace directory anywhere under `root` (the auto-init'd App-Data-Home
/// lands at a platform-specific depth below `HOME`, so we search rather than hardcode the path).
fn find_nxs_under(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name() == Some(std::ffi::OsStr::new(".nxs")) {
                    return Some(path);
                }
                stack.push(path);
            }
        }
    }
    None
}

#[tokio::test]
async fn server_initializes_and_lists_tools_over_stdio() {
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;

    // initialize: the server announces itself.
    let info = client
        .peer_info()
        .expect("server info is available after the initialize handshake");
    assert!(
        !info.server_info.name.is_empty(),
        "server announces an implementation name"
    );

    // tools/list responds without error (the skeleton may expose zero or more tools).
    client
        .list_all_tools()
        .await
        .expect("tools/list returns a result over stdio");

    client.cancel().await.expect("clean shutdown");
}

// ---- #4ti / #76u.6: the flow read tools (prime is now instructions-only, not a tool) ------------

#[tokio::test]
async fn tools_list_exposes_the_flow_read_tools_and_no_prime_tool() {
    // The read tools are present in the (now read+write) surface, and prime is still NOT a tool — it
    // is delivered only via initialize.instructions (#76u.6). The exact full surface is pinned by
    // `tools_list_exposes_the_full_flow_surface`.
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;

    let tools = client.list_all_tools().await.expect("tools/list");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    for read in [
        "flow_list",
        "flow_next",
        "flow_show",
        "flow_blocked",
        "flow_search",
        "flow_note_list",
        "flow_mention_list",
    ] {
        assert!(names.contains(&read), "the read tool {read} is present");
    }
    assert!(
        !names.contains(&"flow_prime"),
        "prime is instructions-only, never a tool (#76u.6)"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_list_returns_canonical_item_records() {
    let tmp = flow_fixture();
    nxf(
        tmp.path(),
        &[
            "create",
            "--type",
            "feature",
            "--title",
            "Write the docs",
            "--description",
            "the body",
            "--priority",
            "P1",
        ],
    );
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(CallToolRequestParams::new("flow_list"))
        .await
        .expect("flow_list call");
    assert_ne!(res.is_error, Some(true), "a healthy list is not an error");
    let sc = res
        .structured_content
        .expect("flow_list returns structuredContent");
    // The canonical list record is an array of item records (epic: item-array per op), wrapped
    // under `items` so structuredContent is an MCP-legal object (#76u.9).
    let items = sc["items"]
        .as_array()
        .expect("list structuredContent.items is an array");
    assert_eq!(items.len(), 1, "the one created item");
    assert_eq!(items[0]["title"], "Write the docs");
    assert_eq!(items[0]["type"], "feature");

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_show_unknown_id_maps_to_a_not_found_tool_error() {
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with("flow_show", json!({ "id": "zzzz.999" })))
        .await
        .expect("flow_show call returns a tool result (not a protocol error)");
    assert_eq!(res.is_error, Some(true), "an unknown id is a tool error");
    let sc = res
        .structured_content
        .expect("error carries a structured envelope");
    assert_eq!(
        sc["error"]["kind"], "not_found",
        "domain error mapped to the closed kind set"
    );
    assert!(
        sc["error"]["msg"].is_string(),
        "error carries a human message"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_contributes_add_unknown_id_maps_to_a_not_found_tool_error() {
    // #76u.12 (review #145 Code/Test): the contributes-edge write tools had only happy-path + parity
    // coverage; pin the failing case directly at the MCP seam too. A contributes edge to a missing
    // item is a `not_found` tool error (is_error + the closed-kind envelope), matching `nxf
    // contributes add` — not a panic and not a silent success.
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;

    for tool in ["flow_contributes_add", "flow_contributes_remove"] {
        let res = client
            .call_tool(call_with(
                tool,
                json!({ "from": "zzzz.999", "to": "zzzz.998" }),
            ))
            .await
            .unwrap_or_else(|_| panic!("{tool} returns a tool result, not a protocol error"));
        assert_eq!(
            res.is_error,
            Some(true),
            "{tool} on a missing item is a tool error"
        );
        let sc = res
            .structured_content
            .unwrap_or_else(|| panic!("{tool} error carries a structured envelope"));
        assert_eq!(
            sc["error"]["kind"], "not_found",
            "{tool}: domain error mapped to the closed kind set"
        );
    }

    client.cancel().await.expect("clean shutdown");
}

// ---- #42x: cross-seam parity + determinism (read) ----------------------------------------------

/// Seed a board with a parent/child hierarchy, a dependency, and a contributes-to edge, so the
/// parity assertions exercise NON-trivial record shapes — a parent-join in `next`, a `blocked` entry
/// in `prime`, and `deps`/`contributes_to` in `show` — not just bare independent items.
///
/// Deterministic ids (suffix 0001..0004):
/// - `0001` epic — the container.
/// - `0002` feature, child of `0001` — ready, so `next` carries a non-empty parent-join.
/// - `0003` bug — a ready prerequisite.
/// - `0004` feature, child of `0001`, depends on `0003`, contributes to `0001` — blocked (so `prime`
///   has a `blocked` entry) and a rich `show` (non-empty `deps` + `contributes_to`).
fn seed(dir: &Path) {
    let create = |title: &str, ty: &str, prio: &str, parent: Option<&str>| {
        let mut args = vec![
            "create",
            "--type",
            ty,
            "--title",
            title,
            "--description",
            "d",
            "--priority",
            prio,
        ];
        if let Some(p) = parent {
            args.extend_from_slice(&["--parent", p]);
        }
        nxf(dir, &args);
    };
    create("Build it", "epic", "P1", None); // 0001
    create("Ready child", "feature", "P1", Some("0001")); // 0002
    create("Prereq", "bug", "P2", None); // 0003
    create("Blocked + linked", "feature", "P1", Some("0001")); // 0004
    nxf(dir, &["dep", "add", "0004", "0003"]); // 0004 depends on 0003 → 0004 blocked, 0003 ready
    nxf(dir, &["contributes", "add", "0004", "0001"]); // 0004 contributes to 0001
}

#[tokio::test]
async fn flow_list_structured_content_matches_nxf_list_json() {
    let tmp = flow_fixture();
    seed(tmp.path());
    let client = connect(tmp.path()).await;

    let tool = tool_sc(&client, "flow_list", json!({})).await;
    // ee2h: the seam stays the canonical record; strip the CLI-only *_label decoration.
    let cli = without_presentation_labels(&nxf_json(tmp.path(), &["list"]));
    // Parity refined for list ops (#76u.9): the CLI-identical array lives under `items`; the seam
    // wraps it so structuredContent is an MCP-legal object.
    assert_eq!(
        tool["items"], cli,
        "flow_list structuredContent.items == the canonical `nxf list --json` record"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_next_structured_content_matches_nxf_next_json() {
    let tmp = flow_fixture();
    seed(tmp.path());
    let client = connect(tmp.path()).await;

    let tool = tool_sc(&client, "flow_next", json!({})).await;
    // ee2h: the seam stays the canonical record; strip the CLI-only *_label decoration.
    let cli = without_presentation_labels(&nxf_json(tmp.path(), &["next"]));
    // Parity refined for list ops (#76u.9): the CLI-identical array lives under `items`.
    assert_eq!(
        tool["items"], cli,
        "flow_next structuredContent.items == the canonical `nxf next --json` record"
    );
    // The ready child (0002) carries a parent-join, so parity covers the non-trivial join shape.
    assert!(
        tool["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["parent"].is_object()),
        "the parity fixture exercises a non-empty parent-join in next"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_show_structured_content_matches_nxf_show_json() {
    let tmp = flow_fixture();
    seed(tmp.path());
    let client = connect(tmp.path()).await;

    // 0004 is the rich item: it depends on 0003 and contributes to 0001, so the composite record
    // exercises deps + contributes_to (not just the trivial empty shape).
    let tool = tool_sc(&client, "flow_show", json!({ "id": "0004" })).await;
    // ee2h: the seam stays the canonical record; strip the CLI-only *_label decoration.
    let cli = without_presentation_labels(&nxf_json(tmp.path(), &["show", "0004"]));
    assert_eq!(
        tool, cli,
        "flow_show structuredContent == the canonical `nxf show <id> --json` record"
    );
    assert!(
        !tool["deps"].as_array().unwrap().is_empty(),
        "the parity fixture exercises a non-empty deps list"
    );
    assert!(
        !tool["contributes_to"].as_array().unwrap().is_empty(),
        "the parity fixture exercises a non-empty contributes_to list"
    );

    client.cancel().await.expect("clean shutdown");
}

// ---- #nes: the remaining read tools (blocked/search/note_list/mention_list) cross-seam parity ----

#[tokio::test]
async fn flow_blocked_structured_content_matches_nxf_blocked_json() {
    let tmp = flow_fixture();
    seed(tmp.path());
    let client = connect(tmp.path()).await;

    let tool = tool_sc(&client, "flow_blocked", json!({})).await;
    // yfwt: `nxf blocked --json` now carries the CLI-only priority_label/type_label; strip them so
    // the comparison stays against the shared seam record (bbq6's `custom` join rides BOTH sides).
    let cli = without_presentation_labels(&nxf_json(tmp.path(), &["blocked"]));
    // Parity refined for list ops (#76u.9): the CLI-identical array lives under `items`.
    assert_eq!(
        tool["items"], cli,
        "flow_blocked structuredContent.items == `nxf blocked --json`"
    );
    // 0004 depends on 0003, so it is blocked and carries a non-empty `blockers` list — the parity
    // covers the non-trivial blocker-join shape, not just a bare item array.
    let rows = tool["items"].as_array().expect("blocked items is an array");
    assert!(
        rows.iter()
            .any(|r| r["blockers"].as_array().is_some_and(|b| !b.is_empty())),
        "the parity fixture exercises a non-empty blockers join"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_blocked_sort_param_routes_through_the_seam() {
    // #nes (Test Quality #2): prove the `sort` param reaches the compute (not silently dropped). Two
    // blocked items whose rank order (priority-first) disagrees with id order, so `sort=id` reorders
    // vs the default rank — and both match the CLI byte-for-byte.
    let tmp = flow_fixture();
    let create = |title: &str, prio: &str| {
        nxf(
            tmp.path(),
            &[
                "create",
                "--type",
                "feature",
                "--title",
                title,
                "--description",
                "d",
                "--priority",
                prio,
            ],
        );
    };
    create("Prereq A", "P2"); // 0001
    create("Prereq B", "P2"); // 0002
    create("Low blocked", "P2"); // 0003 — lower priority, lower id
    create("High blocked", "P1"); // 0004 — higher priority, higher id
    nxf(tmp.path(), &["dep", "add", "0003", "0001"]); // 0003 blocked
    nxf(tmp.path(), &["dep", "add", "0004", "0002"]); // 0004 blocked
    let client = connect(tmp.path()).await;

    let ids = |v: &Value| {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let by_id = tool_sc(&client, "flow_blocked", json!({ "sort": "id" })).await;
    assert_eq!(
        by_id["items"],
        without_presentation_labels(&nxf_json(tmp.path(), &["blocked", "--sort", "id"])),
        "flow_blocked sort=id == `nxf blocked --sort id --json`"
    );
    let default = tool_sc(&client, "flow_blocked", json!({})).await;
    // Default is rank (priority-first): 0004 (P1) leads; id order would lead with 0003. If `sort`
    // were ignored the two would be identical — and the parity above vs `--sort id` would also fail.
    assert_ne!(
        ids(&default),
        ids(&by_id),
        "the sort param actually reorders the blocked set (rank ≠ id)"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_search_structured_content_matches_nxf_search_json() {
    let tmp = flow_fixture();
    seed(tmp.path());
    let client = connect(tmp.path()).await;

    // Every seeded item shares the description "d", so the query matches across lanes (ready +
    // blocked) — exercising the lane-grouped order, not a single trivial hit.
    let tool = tool_sc(&client, "flow_search", json!({ "query": "d" })).await;
    // yfwt: strip the CLI-only priority_label/type_label `search --json` now carries.
    let cli = without_presentation_labels(&nxf_json(tmp.path(), &["search", "d"]));
    assert_eq!(
        tool["items"], cli,
        "flow_search structuredContent.items == `nxf search <query> --json`"
    );
    assert!(
        tool["items"].as_array().is_some_and(|a| a.len() > 1),
        "the parity fixture exercises a multi-lane match set"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_search_filter_params_route_through_the_seam() {
    // #nes (Test Quality #1): the param-struct → SearchArgs mapping is the ONLY logic flow_search
    // adds over the facade-tested compute. Drive each filter through the seam and compare to the
    // equivalent `nxf search … --json`, so a swapped mapping (status↔type, include↔archived_only)
    // ships red, not silent. `type` exercises the `#[serde(rename = "type")]`→item_type slot.
    let tmp = flow_fixture();
    seed(tmp.path());
    // An archived match so `--archived-only` has a hit and the live scope has something to exclude.
    nxf(
        tmp.path(),
        &[
            "create",
            "--type",
            "bug",
            "--title",
            "Archived match",
            "--description",
            "d",
            "--priority",
            "P2",
        ],
    ); // 0005
    nxf(tmp.path(), &["close", "0005", "--reason", "done"]);
    nxf(tmp.path(), &["archive", "0005"]);
    let client = connect(tmp.path()).await;

    // type filter (the rename): matches only the live bug (0003); the archived bug 0005 is excluded.
    let by_type = tool_sc(
        &client,
        "flow_search",
        json!({ "query": "d", "type": "bug" }),
    )
    .await;
    assert_eq!(
        by_type["items"],
        without_presentation_labels(&nxf_json(tmp.path(), &["search", "d", "--type", "bug"])),
        "flow_search type filter == `nxf search d --type bug --json`"
    );

    // status filter: a status→item_type swap would filter items whose TYPE == "open" (none) and
    // diverge from the CLI.
    let by_status = tool_sc(
        &client,
        "flow_search",
        json!({ "query": "d", "status": "open" }),
    )
    .await;
    assert_eq!(
        by_status["items"],
        without_presentation_labels(&nxf_json(tmp.path(), &["search", "d", "--status", "open"])),
        "flow_search status filter == `nxf search d --status open --json`"
    );

    // archive scope: archived_only must find ONLY the archived item — an include_archived swap would
    // instead return the live matches too and diverge.
    let archived = tool_sc(
        &client,
        "flow_search",
        json!({ "query": "d", "archived_only": true }),
    )
    .await;
    assert_eq!(
        archived["items"],
        without_presentation_labels(&nxf_json(tmp.path(), &["search", "d", "--archived-only"])),
        "flow_search archived_only == `nxf search d --archived-only --json`"
    );
    assert!(
        archived["items"].as_array().is_some_and(|a| !a.is_empty()),
        "the archived fixture makes --archived-only a non-empty, discriminating case"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_note_list_structured_content_matches_nxf_note_list_json() {
    let tmp = flow_fixture();
    seed(tmp.path());
    // A worklog note on 0004 so the list is non-empty (the `{id, body}` shape, not the empty array).
    nxf(tmp.path(), &["note", "add", "0004", "first worklog entry"]);
    let client = connect(tmp.path()).await;

    let tool = tool_sc(&client, "flow_note_list", json!({ "id": "0004" })).await;
    let cli = nxf_json(tmp.path(), &["note", "list", "0004"]);
    assert_eq!(
        tool["items"], cli,
        "flow_note_list structuredContent.items == `nxf note list <id> --json`"
    );
    let notes = tool["items"].as_array().expect("notes is an array");
    assert_eq!(notes.len(), 1, "the one worklog note");
    assert_eq!(notes[0]["body"], "first worklog entry");

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_mention_list_structured_content_matches_nxf_mention_list_json() {
    let tmp = flow_fixture();
    seed(tmp.path());
    // 0004 cites 0001 so the list is non-empty (a plain array of the cited ids).
    nxf(tmp.path(), &["mention", "add", "0004", "0001"]);
    let client = connect(tmp.path()).await;

    let tool = tool_sc(&client, "flow_mention_list", json!({ "id": "0004" })).await;
    let cli = nxf_json(tmp.path(), &["mention", "list", "0004"]);
    assert_eq!(
        tool["items"], cli,
        "flow_mention_list structuredContent.items == `nxf mention list <id> --json`"
    );
    assert!(
        tool["items"].as_array().is_some_and(|a| !a.is_empty()),
        "the parity fixture exercises a non-empty mention list"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_schema_exposes_the_type_system() {
    // y5j8: flow_schema surfaces the active plugin's valid types + containment roles, byte-identical
    // to `nxf schema --json`. The cross-seam parity case for the schema record (the first one).
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;
    let sc = tool_sc(&client, "flow_schema", json!({})).await;
    // Byte-identical to the CLI --json record.
    assert_eq!(
        sc,
        nxf_json(tmp.path(), &["schema"]),
        "flow_schema ≙ nxf schema --json"
    );
    // It exposes the container role (issue-tracker: epic is the container, depth 2).
    assert_eq!(sc["hierarchy"]["types"]["epic"]["container"], json!(true));
    assert_eq!(sc["hierarchy"]["max_depth"], json!(2));
    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_label_add_and_list_match_nxf_label_json() {
    // h89s.3: the label add receipt + the label list structuredContent are byte-identical to their
    // `nxf label … --json`. Two fixtures so the CLI and MCP writes don't share state.
    let mcp = flow_fixture();
    let cli = flow_fixture();
    seed(mcp.path());
    seed(cli.path());
    let client = connect(mcp.path()).await;

    let tool_add = tool_sc(
        &client,
        "flow_label_add",
        json!({ "id": "0004", "label": "urgent", "now": NOW, "actor": "alice" }),
    )
    .await;
    let cli_add = nxf_json(cli.path(), &["label", "add", "0004", "urgent"]);
    assert_eq!(
        tool_add, cli_add,
        "flow_label_add receipt == `nxf label add --json`"
    );
    assert_eq!(tool_add["ok"], true);

    let tool_list = tool_sc(&client, "flow_label_list", json!({ "id": "0004" })).await;
    let cli_list = nxf_json(cli.path(), &["label", "list", "0004"]);
    assert_eq!(
        tool_list["items"], cli_list,
        "flow_label_list structuredContent.items == `nxf label list <id> --json`"
    );
    assert_eq!(tool_list["items"], json!(["urgent"]));

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_show_structured_content_carries_labels() {
    // h89s.3: labels ride the shared facade `show` record, so the MCP structuredContent carries
    // them just like `nxf show --json` (and stays byte-identical).
    let tmp = flow_fixture();
    seed(tmp.path());
    nxf(tmp.path(), &["label", "add", "0004", "urgent"]);
    let client = connect(tmp.path()).await;

    let tool = tool_sc(&client, "flow_show", json!({ "id": "0004" })).await;
    // ee2h: the seam stays the canonical record; strip the CLI-only *_label decoration. The user
    // OR-set `labels` field (asserted below) is unrelated and rides the shared show record.
    let cli = without_presentation_labels(&nxf_json(tmp.path(), &["show", "0004"]));
    assert_eq!(
        tool, cli,
        "flow_show structuredContent == the canonical `nxf show --json` record"
    );
    assert_eq!(
        tool["labels"],
        json!(["urgent"]),
        "show carries the label set"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn note_and_mention_list_on_an_unknown_id_map_to_a_not_found_tool_error() {
    // Like `flow_show`, an id-addressed read of a missing item is a `not_found` tool error (is_error
    // + the closed-kind envelope), matching `nxf note/mention list` — never a panic or empty success.
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;

    for tool in ["flow_note_list", "flow_mention_list"] {
        let res = client
            .call_tool(call_with(tool, json!({ "id": "zzzz.999" })))
            .await
            .unwrap_or_else(|_| panic!("{tool} returns a tool result, not a protocol error"));
        assert_eq!(
            res.is_error,
            Some(true),
            "{tool} on a missing item is a tool error"
        );
        let sc = res
            .structured_content
            .unwrap_or_else(|| panic!("{tool} error carries a structured envelope"));
        assert_eq!(
            sc["error"]["kind"], "not_found",
            "{tool}: domain error mapped to the closed kind set"
        );
    }

    client.cancel().await.expect("clean shutdown");
}

// (#76u.6) `flow_prime` is no longer a tool — prime is delivered only via initialize.instructions
// (#76u.5), which are STATIC (rules + tool reference + nudge) and carry no live snapshot. The live
// ready/blocked sets come from `flow_next`/`flow_blocked` instead, whose cross-seam parity is pinned
// directly above.

#[tokio::test]
async fn read_tools_are_deterministic_under_a_pinned_now() {
    let tmp = flow_fixture();
    seed(tmp.path());
    let client = connect(tmp.path()).await;

    // Twice with the same pinned `now` → byte-equal structuredContent, across the now-bearing reads
    // (flow_next, flow_search) and the clock-independent flow_blocked (#nes — the new tools too).
    for (name, args) in [
        ("flow_next", json!({ "now": NOW })),
        ("flow_search", json!({ "query": "d", "now": NOW })),
        ("flow_blocked", json!({})),
    ] {
        let a = tool_sc(&client, name, args.clone()).await;
        let b = tool_sc(&client, name, args).await;
        assert_eq!(
            a.to_string(),
            b.to_string(),
            "{name}: same now ⇒ byte-identical"
        );
    }

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_search_now_crossing_a_defer_boundary_reorders_the_lane_grouping() {
    // #nes (Test Quality #3): prove `now` is HONORED by flow_search on the seam, not silently
    // dropped — a determinism check alone can't (an ignored `now` is still deterministic). Two
    // matching items, one deferred: before its defer date it sits in the LATER `deferred` lane, after
    // it joins the ready group and rank order (priority-first) puts it FIRST — so the id order flips.
    let tmp = flow_fixture();
    nxf(
        tmp.path(),
        &[
            "create",
            "--type",
            "feature",
            "--title",
            "alpha match",
            "--description",
            "d",
            "--priority",
            "P2",
        ],
    ); // 0001 — ready, lower priority
    nxf(
        tmp.path(),
        &[
            "create",
            "--type",
            "feature",
            "--title",
            "beta match",
            "--description",
            "d",
            "--priority",
            "P1",
            "--defer",
            "2026-06-25T00:00:00Z",
        ],
    ); // 0002 — higher priority, deferred until the 25th
    let client = connect(tmp.path()).await;

    let order = |v: &Value| {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let before = tool_sc(
        &client,
        "flow_search",
        json!({ "query": "match", "now": "2026-06-24T00:00:00Z" }),
    )
    .await;
    let after = tool_sc(
        &client,
        "flow_search",
        json!({ "query": "match", "now": "2026-06-26T00:00:00Z" }),
    )
    .await;
    let (bo, ao) = (order(&before), order(&after));
    assert_eq!(
        bo.len(),
        2,
        "both items match the query regardless of `now`"
    );
    assert_eq!(
        bo.iter().collect::<std::collections::BTreeSet<_>>(),
        ao.iter().collect::<std::collections::BTreeSet<_>>(),
        "the matched SET is unchanged by `now` (only the lane grouping moves)"
    );
    assert_ne!(
        bo, ao,
        "`now` crossing the defer boundary reorders the lane grouping — proof the param is honored"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_next_now_crossing_a_defer_boundary_changes_the_ready_set() {
    let tmp = flow_fixture();
    // An item deferred until 2026-06-25 is not ready before that date, but is after — a LEGITIMATE
    // (not a determinism-bug) divergence driven by `now`.
    nxf(
        tmp.path(),
        &[
            "create",
            "--type",
            "feature",
            "--title",
            "Later",
            "--description",
            "d",
            "--priority",
            "P1",
            "--defer",
            "2026-06-25T00:00:00Z",
        ],
    );
    let client = connect(tmp.path()).await;

    let before = tool_sc(
        &client,
        "flow_next",
        json!({ "now": "2026-06-24T00:00:00Z" }),
    )
    .await;
    let after = tool_sc(
        &client,
        "flow_next",
        json!({ "now": "2026-06-26T00:00:00Z" }),
    )
    .await;
    let count = |v: &Value| v["items"].as_array().map(|a| a.len()).unwrap_or(0);
    assert_eq!(
        count(&before),
        0,
        "deferred item is not ready before its defer date"
    );
    assert_eq!(
        count(&after),
        1,
        "the same item becomes ready once `now` passes the defer date"
    );

    client.cancel().await.expect("clean shutdown");
}

// ---- #wpw / #ei9: flow_create + flow_update write tools, cross-seam parity ----------------------

/// The deterministic `nxf create` args mirrored by the `flow_create` tool call below.
const CREATE_ARGS: &[&str] = &[
    "create",
    "--type",
    "feature",
    "--title",
    "Write tools",
    "--description",
    "the body",
    "--priority",
    "P1",
];

#[tokio::test]
async fn flow_create_structured_content_matches_nxf_create_json() {
    // Cross-seam parity (#ei9): flow_create's receipt is byte-identical to `nxf create --json` for
    // the same fresh state — deterministic ids (so both mint `0001`) + pinned now/actor on both
    // sides. Two identical fresh fixtures, since `create` is a minting op (one per call).
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();

    let client = connect(mcp_fix.path()).await;
    let tool = tool_sc(
        &client,
        "flow_create",
        json!({
            "type": "feature",
            "title": "Write tools",
            "description": "the body",
            "priority": "P1",
            "now": NOW,
            "actor": "alice",
        }),
    )
    .await;
    client.cancel().await.expect("clean shutdown");

    let cli = nxf_json(cli_fix.path(), CREATE_ARGS);
    assert_eq!(tool, cli, "flow_create receipt == `nxf create --json`");
    assert_eq!(tool["title"], "Write tools", "the created item's title");
    assert_eq!(tool["type"], "feature");
    assert_eq!(tool["status"], "open", "a fresh item is open");
}

#[tokio::test]
async fn flow_update_structured_content_matches_nxf_update_json() {
    // Parity over an EXISTING item: set a field via flow_update on one seeded board and via
    // `nxf update --set` on an identical one; the returned canonical records must be byte-equal.
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();
    seed(mcp_fix.path());
    seed(cli_fix.path());

    let (tool, cli) = write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "flow_update",
        json!({ "id": "0002", "set": ["priority=P0"], "now": NOW, "actor": "alice" }),
        &["update", "0002", "--set", "priority=P0"],
    )
    .await;
    assert_eq!(tool, cli, "flow_update receipt == `nxf update --json`");
    assert_eq!(tool["priority"], "0", "P0 stored as the canonical ordinal");
}

#[tokio::test]
async fn flow_claim_structured_content_matches_nxf_claim_json() {
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();
    seed(mcp_fix.path());
    seed(cli_fix.path());

    let (tool, cli) = write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "flow_claim",
        json!({ "id": "0002", "now": NOW, "actor": "alice" }),
        &["claim", "0002"],
    )
    .await;
    assert_eq!(tool, cli, "flow_claim receipt == `nxf claim --json`");
    assert_eq!(tool["status"], "in_progress", "claim marks it in progress");
}

#[tokio::test]
async fn flow_close_structured_content_matches_nxf_close_json() {
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();
    seed(mcp_fix.path());
    seed(cli_fix.path());

    let (tool, cli) = write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "flow_close",
        json!({ "id": "0003", "reason": "shipped", "now": NOW, "actor": "alice" }),
        &["close", "0003", "--reason", "shipped"],
    )
    .await;
    assert_eq!(tool, cli, "flow_close receipt == `nxf close --json`");
    assert_eq!(tool["status"], "closed");
    assert_eq!(tool["closing_comment"], "shipped", "the mandatory reason");
    assert_eq!(tool["closed_at"], NOW, "closed_at is the pinned now");
}

#[tokio::test]
async fn flow_dep_add_structured_content_matches_nxf_dep_add_json() {
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();
    seed(mcp_fix.path());
    seed(cli_fix.path());

    // 0002 does not yet depend on 0003 in the seed — adding it is a fresh, cycle-free edge.
    let (tool, cli) = write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "flow_dep_add",
        json!({ "from": "0002", "to": "0003", "now": NOW, "actor": "alice" }),
        &["dep", "add", "0002", "0003"],
    )
    .await;
    assert_eq!(tool, cli, "flow_dep_add receipt == `nxf dep add --json`");
    assert_eq!(tool["ok"], true);
    // The receipt carries the FULL ids (the machine contract), like the CLI's `emit_edge` json.
    assert!(
        tool["msg"].as_str().unwrap().contains(" -> "),
        "the edge receipt names the directed edge: {}",
        tool["msg"]
    );
}

#[tokio::test]
async fn flow_dep_remove_structured_content_matches_nxf_dep_remove_json() {
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();
    seed(mcp_fix.path());
    seed(cli_fix.path());

    // The seed wired 0004 -> 0003; removing it is the observed-remove parity case.
    let (tool, cli) = write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "flow_dep_remove",
        json!({ "from": "0004", "to": "0003", "now": NOW, "actor": "alice" }),
        &["dep", "remove", "0004", "0003"],
    )
    .await;
    assert_eq!(
        tool, cli,
        "flow_dep_remove receipt == `nxf dep remove --json`"
    );
    assert!(tool["msg"].as_str().unwrap().starts_with("removed "));
}

#[tokio::test]
async fn flow_mention_add_structured_content_matches_nxf_mention_add_json() {
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();
    seed(mcp_fix.path());
    seed(cli_fix.path());

    let (tool, cli) = write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "flow_mention_add",
        json!({ "from": "0002", "to": "0003", "now": NOW, "actor": "alice" }),
        &["mention", "add", "0002", "0003"],
    )
    .await;
    assert_eq!(
        tool, cli,
        "flow_mention_add receipt == `nxf mention add --json`"
    );
    assert!(tool["msg"].as_str().unwrap().contains("mentions"));
}

#[tokio::test]
async fn flow_mention_remove_structured_content_matches_nxf_mention_remove_json() {
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();
    seed(mcp_fix.path());
    seed(cli_fix.path());

    // Observed-remove of an absent edge still succeeds (and is byte-identical) on both seams.
    let (tool, cli) = write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "flow_mention_remove",
        json!({ "from": "0002", "to": "0003", "now": NOW, "actor": "alice" }),
        &["mention", "remove", "0002", "0003"],
    )
    .await;
    assert_eq!(
        tool, cli,
        "flow_mention_remove receipt == `nxf mention remove --json`"
    );
}

#[tokio::test]
async fn flow_note_add_structured_content_matches_nxf_note_add_json() {
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();
    seed(mcp_fix.path());
    seed(cli_fix.path());

    // note_add is a MINTING op: each call mints a fresh random-ULID note id (unaffected by
    // NXF_DETERMINISTIC_IDS, which only seeds ITEM ids), so the `id` legitimately differs across two
    // independent stores. Parity therefore holds over the receipt SHAPE + the deterministic `body`;
    // the id is asserted present + string-shaped on both seams, like the CLI's `{ id, body }`.
    let (tool, cli) = write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "flow_note_add",
        json!({ "id": "0002", "text": "a worklog entry", "now": NOW, "actor": "alice" }),
        &["note", "add", "0002", "a worklog entry"],
    )
    .await;
    let keys = |v: &Value| {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    assert_eq!(keys(&tool), keys(&cli), "same receipt shape as the CLI");
    assert_eq!(keys(&tool), vec!["body", "id"], "the {{id, body}} receipt");
    assert_eq!(tool["body"], cli["body"], "body parity across the seam");
    assert_eq!(tool["body"], "a worklog entry");
    assert!(
        tool["id"].as_str().is_some_and(|s| !s.is_empty())
            && cli["id"].as_str().is_some_and(|s| !s.is_empty()),
        "both seams mint a non-empty note id"
    );
}

// ---- #76u.12: contributes-to + archive/unarchive write tools ------------------------------------

#[tokio::test]
async fn flow_contributes_add_structured_content_matches_nxf_contributes_add_json() {
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();
    seed(mcp_fix.path());
    seed(cli_fix.path());

    // The seed wires only 0004 contributes-to 0001; 0002 -> 0001 is a fresh edge.
    let (tool, cli) = write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "flow_contributes_add",
        json!({ "from": "0002", "to": "0001", "now": NOW, "actor": "alice" }),
        &["contributes", "add", "0002", "0001"],
    )
    .await;
    assert_eq!(
        tool, cli,
        "flow_contributes_add receipt == `nxf contributes add --json`"
    );
    assert_eq!(tool["ok"], true);
    assert!(
        tool["msg"].as_str().unwrap().contains("contributes to"),
        "the receipt names the contributes-to edge: {}",
        tool["msg"]
    );
}

#[tokio::test]
async fn flow_contributes_remove_structured_content_matches_nxf_contributes_remove_json() {
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();
    seed(mcp_fix.path());
    seed(cli_fix.path());

    // The seed wired 0004 contributes-to 0001; removing it is the observed-remove parity case.
    let (tool, cli) = write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "flow_contributes_remove",
        json!({ "from": "0004", "to": "0001", "now": NOW, "actor": "alice" }),
        &["contributes", "remove", "0004", "0001"],
    )
    .await;
    assert_eq!(
        tool, cli,
        "flow_contributes_remove receipt == `nxf contributes remove --json`"
    );
    assert!(tool["msg"]
        .as_str()
        .unwrap()
        .starts_with("removed contributes "));
}

#[tokio::test]
async fn flow_archive_structured_content_matches_nxf_archive_json() {
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();
    seed(mcp_fix.path());
    seed(cli_fix.path());
    // archive needs a CLOSED root: 0003 is a childless leaf, so close it on BOTH fixtures first
    // (identically, with pinned now) — then the archive receipts must be byte-equal across the seams.
    nxf(mcp_fix.path(), &["close", "0003", "--reason", "done"]);
    nxf(cli_fix.path(), &["close", "0003", "--reason", "done"]);

    let (tool, cli) = write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "flow_archive",
        json!({ "ids": ["0003"], "now": NOW, "actor": "alice" }),
        &["archive", "0003"],
    )
    .await;
    assert_eq!(tool, cli, "flow_archive receipt == `nxf archive --json`");
    // The batch receipt shape: an `archived` affected-list + per-root `results`.
    assert_eq!(tool["results"][0]["status"], "archived");
    assert!(
        tool["archived"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().ends_with("0003")),
        "0003 is in the affected list: {}",
        tool["archived"]
    );
}

#[tokio::test]
async fn flow_unarchive_structured_content_matches_nxf_unarchive_json() {
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();
    seed(mcp_fix.path());
    seed(cli_fix.path());
    // Set up an archived 0003 on both fixtures (close → archive), then unarchive it across the seams.
    for dir in [mcp_fix.path(), cli_fix.path()] {
        nxf(dir, &["close", "0003", "--reason", "done"]);
        nxf(dir, &["archive", "0003"]);
    }

    let (tool, cli) = write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "flow_unarchive",
        json!({ "ids": ["0003"], "now": NOW, "actor": "alice" }),
        &["unarchive", "0003"],
    )
    .await;
    assert_eq!(
        tool, cli,
        "flow_unarchive receipt == `nxf unarchive --json`"
    );
    assert_eq!(tool["results"][0]["status"], "unarchived");
}

#[tokio::test]
async fn flow_archive_open_item_is_a_partial_failure_receipt_not_a_tool_error() {
    // archive is a PARTIAL batch: a root that fails its precondition (0001 is open) is reported in
    // `results` with a reason code — NOT an isError tool result (so `tool_sc`, which asserts the
    // result is not an error, succeeds). Byte-identical to `nxf archive --json` on the same state.
    let mcp_fix = flow_fixture();
    let cli_fix = flow_fixture();
    seed(mcp_fix.path());
    seed(cli_fix.path());

    let (tool, cli) = write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "flow_archive",
        json!({ "ids": ["0001"], "now": NOW, "actor": "alice" }),
        &["archive", "0001"],
    )
    .await;
    assert_eq!(tool, cli, "partial-failure receipt parity across the seam");
    assert_eq!(tool["results"][0]["status"], "failed");
    assert_eq!(tool["results"][0]["reason"], "not-closed");
    assert_eq!(
        tool["archived"].as_array().unwrap().len(),
        0,
        "a failed precondition archives nothing"
    );
}

#[tokio::test]
async fn write_tools_are_deterministic_under_a_pinned_now_and_ids() {
    // Determinism (#ei9): the SAME write against two identically-seeded boards, with now/actor/ids
    // pinned, yields byte-identical receipts — there is no hidden clock/random source in the seam.
    let a = flow_fixture();
    let b = flow_fixture();
    seed(a.path());
    seed(b.path());

    let args = json!({ "type": "bug", "title": "Dup", "description": "d", "priority": "P2",
                       "now": NOW, "actor": "alice" });
    let ca = connect(a.path()).await;
    let ra = tool_sc(&ca, "flow_create", args.clone()).await;
    ca.cancel().await.unwrap();
    let cb = connect(b.path()).await;
    let rb = tool_sc(&cb, "flow_create", args).await;
    cb.cancel().await.unwrap();

    assert_eq!(
        ra.to_string(),
        rb.to_string(),
        "pinned now/actor/ids ⇒ byte-identical create receipts"
    );
}

// ---- #zxj: the actor hybrid threads the right identity onto a write ------------------------------

#[tokio::test]
async fn per_tool_actor_lands_on_the_op() {
    // A per-tool `actor` argument is recorded as the op author — the per-call override of the hybrid.
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;
    let _ = tool_sc(
        &client,
        "flow_create",
        json!({ "type": "bug", "title": "T", "description": "d", "priority": "P1",
                "now": NOW, "actor": "alice" }),
    )
    .await;
    client.cancel().await.expect("clean shutdown");

    let authors = op_authors(tmp.path());
    assert!(
        authors.iter().any(|a| a == "alice"),
        "the per-tool actor authors the create ops: {authors:?}"
    );
    assert!(
        !authors.iter().any(|a| a == "nxs"),
        "no op slips through under the fallback identity when an actor was given: {authors:?}"
    );
}

#[tokio::test]
async fn launch_actor_default_lands_on_a_write_without_a_per_tool_actor() {
    // With no per-tool actor, the server-instance `--actor` launch default authors the write (#zxj).
    let tmp = flow_fixture();
    let client = connect_with_actor(tmp.path(), "bob").await;
    let _ = tool_sc(
        &client,
        "flow_create",
        json!({ "type": "bug", "title": "T", "description": "d", "priority": "P1", "now": NOW }),
    )
    .await;
    client.cancel().await.expect("clean shutdown");

    let authors = op_authors(tmp.path());
    assert!(
        authors.iter().any(|a| a == "bob"),
        "the launch --actor default authors the write: {authors:?}"
    );
}

#[tokio::test]
async fn env_actor_fallback_lands_on_a_write_without_per_tool_or_launch_actor() {
    // The bottom hybrid tier: with no per-tool actor AND no `--actor` launch default, the
    // `NXF_ACTOR`/`USER` env fallback authors the write — the CLI's identity, proven here through a
    // real persisted write (not just the `pick_actor` unit seam).
    let tmp = flow_fixture();
    let client = connect_with_env_actor(tmp.path(), "carol").await;
    let _ = tool_sc(
        &client,
        "flow_create",
        json!({ "type": "bug", "title": "T", "description": "d", "priority": "P1", "now": NOW }),
    )
    .await;
    client.cancel().await.expect("clean shutdown");

    let authors = op_authors(tmp.path());
    assert!(
        authors.iter().any(|a| a == "carol"),
        "the NXF_ACTOR env fallback authors the write when no per-tool/launch actor is given: \
         {authors:?}"
    );
}

// ---- #wpw/#yie/#76u.11: the full write surface + per-op annotations ------------------------------

#[tokio::test]
async fn tools_list_exposes_the_full_flow_surface() {
    // The surface contract after the write slice: the three read tools + the nine write tools, every
    // one self-describing (the #ygj "works from tools/list alone" floor for hosts without instructions).
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;

    let tools = client.list_all_tools().await.expect("tools/list");
    let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    names.sort_unstable();
    let mut expected: Vec<&str> = FLOW_TOOLS.iter().chain(UMBRELLA_TOOLS).copied().collect();
    expected.sort_unstable();
    assert_eq!(
        names, expected,
        "the full flow read+write surface plus the always-on umbrella tools"
    );
    for t in &tools {
        assert!(
            t.description.as_ref().is_some_and(|d| !d.is_empty()),
            "tool {} carries a description",
            t.name
        );
    }

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn write_tools_advertise_mutation_annotations_per_op() {
    // Epic invariant: every mutation carries `readOnlyHint:false` + `openWorldHint:false` (closed
    // local board) + a title, and the additive/destructive + idempotent hints PER OP — so a strict
    // host (Claude Desktop/Cowork) can badge/guard each write correctly (not auto-approve it as safe).
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;
    let tools = client.list_all_tools().await.expect("tools/list");

    // (name, destructive_hint, idempotent_hint) — the per-op contract.
    let expected: &[(&str, bool, bool)] = &[
        ("flow_create", false, false), // mints a new item (additive, new id each call)
        ("flow_update", true, true),   // overwrites field values; re-applying is idempotent
        ("flow_claim", true, true),    // overwrites status; idempotent
        ("flow_close", true, true),    // overwrites status/comment; idempotent
        ("flow_dep_add", false, true), // additive edge; idempotent
        ("flow_dep_remove", true, true), // removes an edge; idempotent
        ("flow_mention_add", false, true), // additive edge; idempotent
        ("flow_mention_remove", true, true), // removes an edge; idempotent
        ("flow_note_add", false, false), // appends a note (additive, new id each call)
        // #76u.12
        ("flow_contributes_add", false, true), // additive edge; idempotent
        ("flow_contributes_remove", true, true), // removes an edge; idempotent
        ("flow_archive", true, true),          // overwrites the archived field; idempotent
        ("flow_unarchive", true, true),        // clears the archived field; idempotent
        // labels (h89s.3)
        ("flow_label_add", false, true), // additive label; idempotent
        ("flow_label_remove", true, true), // removes a label; idempotent
    ];
    for (name, destructive, idempotent) in expected {
        let tool = tools
            .iter()
            .find(|t| t.name.as_ref() == *name)
            .unwrap_or_else(|| panic!("{name} is in tools/list"));
        let ann = tool
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("{name} carries machine-readable annotations"));
        assert_eq!(
            ann.read_only_hint,
            Some(false),
            "{name} is a mutation (readOnlyHint:false)"
        );
        assert_eq!(
            ann.open_world_hint,
            Some(false),
            "{name} acts on the closed local board (openWorldHint:false)"
        );
        assert_eq!(
            ann.destructive_hint,
            Some(*destructive),
            "{name} destructiveHint"
        );
        assert_eq!(
            ann.idempotent_hint,
            Some(*idempotent),
            "{name} idempotentHint"
        );
        assert!(
            ann.title.as_ref().is_some_and(|t| !t.is_empty()),
            "{name} carries a human-readable title"
        );
    }

    client.cancel().await.expect("clean shutdown");
}

// ---- write-path error mapping across the seam ---------------------------------------------------

#[tokio::test]
async fn flow_create_unknown_type_maps_to_a_validation_error() {
    // A type the active plugin does not declare is a `validation` tool error (not a panic / protocol
    // error), carrying the closed-kind envelope — the write surface's own error-mapping proof.
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with(
            "flow_create",
            json!({ "type": "nonsense", "title": "T", "description": "d", "priority": "P1" }),
        ))
        .await
        .expect("call returns a tool result, not a protocol error");
    assert_eq!(res.is_error, Some(true), "an unknown type is a tool error");
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "validation",
        "mapped to the validation kind"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_update_unknown_id_maps_to_a_not_found_error() {
    // A write against a missing item is a `not_found` tool error across the seam — the write-path
    // analogue of `flow_show`'s read not_found, locking in that mutations map domain failures the
    // same way (envelope, not a panic / protocol error).
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with(
            "flow_update",
            json!({ "id": "zzzz.999", "set": ["priority=P0"], "now": NOW }),
        ))
        .await
        .expect("call returns a tool result, not a protocol error");
    assert_eq!(res.is_error, Some(true), "an unknown id is a tool error");
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "not_found",
        "mapped to the not_found kind"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_update_malformed_set_maps_to_a_validation_error() {
    // A `set` entry that is not `field=value` reaches the facade's `expected field=value` reject — a
    // `validation` tool error through the UPDATE path (distinct from create's type validation), so
    // the write surface's validation mapping is pinned on the update tool too.
    let tmp = flow_fixture();
    seed(tmp.path());
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with(
            "flow_update",
            json!({ "id": "0002", "set": ["not-a-pair"], "now": NOW }),
        ))
        .await
        .expect("call returns a tool result, not a protocol error");
    assert_eq!(
        res.is_error,
        Some(true),
        "a malformed set entry is a tool error"
    );
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "validation",
        "mapped to the validation kind"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_dep_add_cycle_maps_to_a_cycle_error() {
    // A dependency cycle is rejected AT WRITE TIME and surfaces as the `cycle` kind — a write-path
    // kind the read surface could never emit. 0004 already depends on 0003 (seed), so 0003 -> 0004
    // would close a loop.
    let tmp = flow_fixture();
    seed(tmp.path());
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with(
            "flow_dep_add",
            json!({ "from": "0003", "to": "0004", "now": NOW }),
        ))
        .await
        .expect("call returns a tool result, not a protocol error");
    assert_eq!(res.is_error, Some(true), "a cycle is a tool error");
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "cycle",
        "the write-time cycle rejection maps to the cycle kind"
    );

    client.cancel().await.expect("clean shutdown");
}

// ---- #76u.1: prime fan-out as initialize.instructions ------------------------------------------

#[tokio::test]
async fn initialize_instructions_are_static_usage_with_a_flow_next_nudge() {
    let tmp = flow_fixture();
    seed(tmp.path());
    let client = connect(tmp.path()).await;

    let info = client.peer_info().expect("server info");
    let instructions = info
        .instructions
        .as_deref()
        .expect("initialize carries usage instructions for the launch workspace");
    // The command references are seam-rendered as TOOL names, not CLI syntax (epic invariant).
    assert!(
        instructions.contains("flow_next") && instructions.contains("flow_show"),
        "instructions name the MCP tools: {instructions}"
    );
    assert!(
        !instructions.contains("nxf next"),
        "instructions use tool names instead of CLI syntax: {instructions}"
    );
    // A prominent session-start nudge to pull the live ready set via the tool (#76u.5).
    let head = instructions.lines().take(3).collect::<Vec<_>>().join(" ");
    assert!(
        head.contains("flow_next"),
        "the flow_next nudge is up front: {head}"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn instructions_reference_only_registered_tools() {
    // ctm: every `flow_*`/`memory_*` span the rendered instructions name MUST be a real tool on the
    // active read-only surface. The earlier blind prefix-swap turned write-verb CORE rules
    // (`nxf claim`, `nxf close --reason`, `nxf note add`, `nxf mention add`) into non-existent
    // flow_claim/flow_close/flow_note/flow_mention. Cross-check EVERY tool token against tools/list —
    // the old test only checked the flow_next/flow_show subset that happened to translate cleanly.
    let tmp = flow_fixture();
    seed(tmp.path());
    let client = connect(tmp.path()).await;

    let instructions = client
        .peer_info()
        .unwrap()
        .instructions
        .clone()
        .expect("instructions present");
    let tools: std::collections::HashSet<String> = client
        .list_all_tools()
        .await
        .expect("tools/list")
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();

    let tokens = tool_tokens(&instructions);
    assert!(
        tokens.iter().any(|t| t == "flow_next"),
        "instructions name at least the flow_next tool: {instructions}"
    );
    for tok in &tokens {
        assert!(
            tools.contains(tok),
            "instructions reference a non-existent tool `{tok}` (not in tools/list {tools:?})"
        );
    }

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn instructions_are_deterministic_under_a_pinned_now() {
    let tmp = flow_fixture();
    seed(tmp.path());

    let a = connect(tmp.path()).await;
    let one = a.peer_info().unwrap().instructions.clone();
    a.cancel().await.unwrap();

    let b = connect(tmp.path()).await;
    let two = b.peer_info().unwrap().instructions.clone();
    b.cancel().await.unwrap();

    assert!(one.is_some(), "instructions present");
    assert_eq!(one, two, "pinned NXS_NOW ⇒ byte-stable instructions");
}

#[tokio::test]
async fn instructions_are_static_and_byte_stable_across_store_state() {
    // #76u.5: instructions are STATIC (rules + tool reference + nudge), never a live snapshot — so
    // they cannot go stale between chats. Adding work must NOT change the instructions a byte, and a
    // created item's title must never leak into them.
    let tmp = flow_fixture();

    let before = {
        let c = connect(tmp.path()).await;
        let i = c.peer_info().unwrap().instructions.clone();
        c.cancel().await.unwrap();
        i.expect("instructions present")
    };
    assert!(
        !before.contains("Distinctive Title XYZ"),
        "no live snapshot leaks into instructions"
    );

    nxf(
        tmp.path(),
        &[
            "create",
            "--type",
            "feature",
            "--title",
            "Distinctive Title XYZ",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );

    let after = {
        let c = connect(tmp.path()).await;
        let i = c.peer_info().unwrap().instructions.clone();
        c.cancel().await.unwrap();
        i.expect("instructions present")
    };
    assert_eq!(
        before, after,
        "instructions are byte-stable regardless of store state"
    );
    assert!(
        !after.contains("Distinctive Title XYZ"),
        "the new item never appears in instructions"
    );
}

#[tokio::test]
async fn missing_launch_workspace_starts_cleanly_without_instructions() {
    // An empty dir (no `.nxs/`): the server must start and answer initialize without instructions —
    // no crash (epic acceptance #5).
    let empty = TempDir::new().unwrap();
    let client = connect(empty.path()).await;

    let info = client.peer_info().expect("server still initializes");
    assert!(
        info.instructions.is_none(),
        "no workspace ⇒ no instructions, but a clean start"
    );
    // tools/list still works (the tools are workspace-independent until called).
    client
        .list_all_tools()
        .await
        .expect("tools/list still responds");

    client.cancel().await.expect("clean shutdown");
}

// ---- #76u.7: the App-Data-Home default workspace (no --workspace/--db) ---------------------------

#[tokio::test]
async fn default_workspace_auto_inits_app_data_home_not_cwd() {
    // No --workspace/--db: the server must auto-init and serve a fresh App-Data-Home under HOME, NOT
    // discover the `.nxs/` in the process cwd — cwd discovery is dropped for the MCP persona (#76u.7).
    let home = TempDir::new().unwrap();
    // A seeded flow workspace as the cwd: its distinctive item must NOT appear if cwd is ignored.
    let cwd = flow_fixture();
    nxf(
        cwd.path(),
        &[
            "create",
            "--type",
            "feature",
            "--title",
            "CWD ITEM MUST BE IGNORED",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );

    let client = connect_default(home.path(), cwd.path()).await;

    // The server initialized against the auto-init'd App-Data-Home, so it pushes usage instructions
    // and the three flow read tools — proof a workspace was found/created with no flags.
    let info = client.peer_info().expect("server info");
    assert!(
        info.instructions.is_some(),
        "the auto-init'd default workspace yields connect-time instructions"
    );
    let tools = client.list_all_tools().await.expect("tools/list");
    assert_eq!(
        tools.len(),
        FLOW_TOOLS.len() + UMBRELLA_TOOLS.len(),
        "the full flow read+write surface + umbrella tools over the default workspace"
    );

    // The served board is the FRESH App-Data-Home, not the seeded cwd: flow_list is empty and the
    // cwd's distinctive item never appears.
    let list = tool_sc(&client, "flow_list", json!({})).await;
    assert_eq!(
        list["items"].as_array().map(|a| a.len()),
        Some(0),
        "the App-Data-Home board starts empty — the seeded cwd was not discovered"
    );

    client.cancel().await.expect("clean shutdown");

    // Auto-init actually wrote `.nxs/` under the App-Data-Home, and it lives beneath the STABLE
    // neutral nxs identifier (`<data-home>/com.nxsflow.nxs/.nxs/`, 4cmg) — pinning the default
    // board's on-disk location so a regression changing the identifier/resolution is caught here,
    // not silently in the field.
    let nxs = find_nxs_under(home.path()).expect("a `.nxs/` was auto-initialized under HOME");
    assert!(
        nxs.ends_with("com.nxsflow.nxs/.nxs"),
        "the default board is <data-home>/com.nxsflow.nxs/.nxs: {}",
        nxs.display()
    );
}

// ---- #ygj: acceptance — a real MCP client, read-only, end-to-end --------------------------------

#[tokio::test]
async fn acceptance_read_only_vertical_end_to_end() {
    // A real MCP client (rmcp) connects over stdio to `nxs mcp serve` and drives the whole
    // read-only surface against a seeded workspace — the dogfood gate for the vertical.
    let tmp = flow_fixture();
    nxf(
        tmp.path(),
        &[
            "create",
            "--type",
            "epic",
            "--title",
            "Ship MCP",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );
    nxf(
        tmp.path(),
        &[
            "create",
            "--type",
            "feature",
            "--title",
            "Read tools",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );
    let client = connect(tmp.path()).await;

    // initialize advertises the server + pushes the static usage instructions (prime-as-guidance).
    let info = client.peer_info().expect("server info");
    assert_eq!(info.server_info.name, "nxs");
    assert!(
        info.instructions.is_some(),
        "connect-time usage instructions"
    );

    // Self-describing floor (#ygj): every tool carries a description even for hosts that ignore
    // instructions — so the surface is learnable from tools/list alone.
    let tools = client.list_all_tools().await.expect("tools/list");
    assert_eq!(
        tools.len(),
        FLOW_TOOLS.len() + UMBRELLA_TOOLS.len(),
        "the full flow read+write surface + umbrella tools (no flow_prime — #76u.6)"
    );
    assert!(
        tools
            .iter()
            .all(|t| t.description.as_ref().is_some_and(|d| !d.is_empty())),
        "every tool is self-describing"
    );

    // list → next → show, all read-only, all returning canonical records (prime is instructions-only).
    let list = tool_sc(&client, "flow_list", json!({})).await;
    assert_eq!(
        list["items"].as_array().map(|a| a.len()),
        Some(2),
        "both items listed"
    );

    let next = tool_sc(&client, "flow_next", json!({})).await;
    assert!(next["items"].is_array(), "next is the ranked ready set");

    let show = tool_sc(&client, "flow_show", json!({ "id": "0001" })).await;
    assert_eq!(
        show["item"]["title"], "Ship MCP",
        "show resolves the first item"
    );
    assert!(
        show["deps"].is_array() && show["notes"].is_array(),
        "show is the composite record"
    );

    client.cancel().await.expect("clean shutdown");
}

// ---- error-envelope mapping across the seam (closes the Test Quality gaps) ----------------------

#[tokio::test]
async fn flow_next_unknown_sort_maps_to_a_validation_error() {
    // An unknown `sort` key reaches `read::parse_sort` and must surface as a `validation` tool
    // result — NOT a panic or a JSON-RPC protocol error.
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with("flow_next", json!({ "sort": "bogus" })))
        .await
        .expect("call returns a tool result, not a protocol error");
    assert_eq!(
        res.is_error,
        Some(true),
        "an unknown sort key is a tool error"
    );
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "validation",
        "mapped to the validation kind"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn flow_next_malformed_now_maps_to_a_validation_error() {
    // A malformed `now` reaches `validate::iso_date` (via resolve_now) and must surface as
    // `validation`. (`flow_next` is the now-taking read tool since `flow_prime` is instructions-only.)
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with("flow_next", json!({ "now": "not-a-date" })))
        .await
        .expect("call returns a tool result, not a protocol error");
    assert_eq!(res.is_error, Some(true), "a malformed now is a tool error");
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "validation",
        "mapped to the validation kind"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn tool_call_without_a_workspace_maps_to_no_workspace() {
    // The server starts cleanly against an empty dir (no `.nxs/`); a tool CALL then hits the
    // per-call `open()` failure, which must map to the `no_workspace` envelope (not a panic).
    let empty = TempDir::new().unwrap();
    let client = connect(empty.path()).await;

    let res = client
        .call_tool(CallToolRequestParams::new("flow_list"))
        .await
        .expect("call returns a tool result, not a protocol error");
    assert_eq!(res.is_error, Some(true), "no workspace ⇒ a tool error");
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "no_workspace",
        "mapped to the no_workspace kind"
    );

    // `flow_schema` is pure over the plugin config, so its ONLY failure mode is the shared per-call
    // `open()` (workspace+plugin resolve) — the same path that must map to the `no_workspace`
    // envelope, never a panic, even though the tool itself never opens a store (y5j8).
    let res = client
        .call_tool(CallToolRequestParams::new("flow_schema"))
        .await
        .expect("flow_schema returns a tool result, not a protocol error");
    assert_eq!(res.is_error, Some(true), "no workspace ⇒ a tool error");
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "no_workspace",
        "flow_schema maps a missing workspace to the no_workspace kind",
    );

    client.cancel().await.expect("clean shutdown");
}

// ---- #76u.9: structuredContent is a JSON OBJECT for every read tool (strict-host MCP contract) ---

#[tokio::test]
async fn every_read_tool_returns_object_structured_content_not_a_top_level_array() {
    // MCP defines `CallToolResult.structuredContent` as a JSON OBJECT. The canonical list/next
    // records are top-level ARRAYS (byte-identical to `nxf list/next --json`); strict hosts (Claude
    // Cowork/Desktop) reject a top-level array as "missing structuredContent". The seam must wrap an
    // array under `items` so EVERY read tool returns an object (#76u.9). `flow_show` is already one.
    let tmp = flow_fixture();
    seed(tmp.path());
    let client = connect(tmp.path()).await;

    for (name, args) in [
        ("flow_list", json!({})),
        ("flow_next", json!({})),
        ("flow_show", json!({ "id": "0001" })),
        // #nes read tools: the three list-shaped ones wrap under `items`; note/mention return the
        // wrapped array even when empty (id-addressed, so 0001 has no notes/mentions here).
        ("flow_blocked", json!({})),
        ("flow_search", json!({ "query": "d" })),
        ("flow_note_list", json!({ "id": "0001" })),
        ("flow_mention_list", json!({ "id": "0001" })),
    ] {
        let sc = tool_sc(&client, name, args).await;
        assert!(
            sc.is_object(),
            "{name} structuredContent must be a JSON object (MCP contract), got: {sc}"
        );
        assert!(
            !sc.is_array(),
            "{name} structuredContent must not be a top-level array (strict hosts reject it)"
        );
    }

    // The list-shaped records carry their canonical array under the `items` wrapper key.
    let list = tool_sc(&client, "flow_list", json!({})).await;
    assert!(
        list["items"].is_array(),
        "flow_list wraps its canonical array under `items`: {list}"
    );
    let next = tool_sc(&client, "flow_next", json!({})).await;
    assert!(
        next["items"].is_array(),
        "flow_next wraps its canonical array under `items`: {next}"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn text_content_mirrors_the_wrapped_structured_content() {
    // `tool_result` wraps BOTH channels and its doc claims they mirror each other: the text
    // projection must parse to the SAME object as structuredContent — including the `items` wrapper
    // for list-shaped tools (#76u.9 review). Pin it so a future divergence between the two channels
    // is caught. Exercises the wrapped (list) and the pass-through (show) shapes.
    let tmp = flow_fixture();
    seed(tmp.path());
    let client = connect(tmp.path()).await;

    for (name, args) in [
        ("flow_list", json!({})),
        ("flow_show", json!({ "id": "0001" })),
    ] {
        let res = client
            .call_tool(call_with(name, args))
            .await
            .unwrap_or_else(|e| panic!("{name} call: {e}"));
        let sc = res
            .structured_content
            .clone()
            .unwrap_or_else(|| panic!("{name} returns structuredContent"));
        let text = res
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_else(|| panic!("{name} returns a text content block"));
        let parsed: Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{name} text content is JSON: {e}"));
        assert_eq!(
            parsed, sc,
            "{name} text channel parses to the same wrapped object as structuredContent"
        );
    }

    client.cancel().await.expect("clean shutdown");
}

// ---- #76u.10: read tools carry machine-readable read-only annotations ---------------------------

#[tokio::test]
async fn read_tools_advertise_read_only_and_closed_world_annotations() {
    // Strict hosts (Claude Desktop/Cowork) read `annotations.readOnlyHint` to badge or auto-approve
    // safe reads — prose in the description is not machine-readable. Every flow read tool must carry
    // `readOnlyHint: true` and `openWorldHint: false` (the board is a closed local domain) (#76u.10).
    let tmp = flow_fixture();
    let client = connect(tmp.path()).await;

    let tools = client.list_all_tools().await.expect("tools/list");
    for name in [
        "flow_next",
        "flow_show",
        "flow_list",
        "flow_blocked",
        "flow_search",
        "flow_note_list",
        "flow_mention_list",
        "flow_schema",
        // umbrella read tool (0jq8 Test #3) — same closed-world read contract as the flow reads.
        "list_workspaces",
    ] {
        let tool = tools
            .iter()
            .find(|t| t.name.as_ref() == name)
            .unwrap_or_else(|| panic!("{name} is in tools/list"));
        let ann = tool
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("{name} carries machine-readable annotations"));
        assert_eq!(
            ann.read_only_hint,
            Some(true),
            "{name} advertises readOnlyHint:true"
        );
        assert_eq!(
            ann.open_world_hint,
            Some(false),
            "{name} advertises openWorldHint:false (closed local board)"
        );
        assert!(
            ann.title.as_ref().is_some_and(|t| !t.is_empty()),
            "{name} carries a human-readable title annotation"
        );
    }

    client.cancel().await.expect("clean shutdown");
}

// ====== memory seam (E9 #76u.2 read / #76u.3 write / #76u.4 parity) ===============================
//
// The memory tool line — the exact counterpart to the flow seam above (#wpw/#yie/#ei9), over the
// memory Record-Facade. Memory tools are FAN-OUT-GATED: registered only when the launch workspace has
// the memory module active (#76u.2), so a flow-only workspace never shows them. Each tool's
// structuredContent is byte-identical to `nxm <cmd> --json` for the same state, with now/actor pinned
// on both seams (#76u.4).

/// The full memory tool surface (3 read + 5 write), gated on an active memory module.
const MEMORY_TOOLS: &[&str] = &[
    // read (#76u.2)
    "memory_list",
    "memory_search",
    "memory_show",
    // write (#76u.3)
    "memory_add",
    "memory_update",
    "memory_close",
    // classification + order (6j6v.9a1r)
    "memory_classify",
    "memory_reorder",
];

/// Create a memory-only workspace fixture (`nxm init`) in a fresh tempdir.
fn memory_fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let status = StdCommand::new(cargo_bin("nxm"))
        .current_dir(tmp.path())
        .arg("init")
        .status()
        .expect("spawn nxm init");
    assert!(
        status.success(),
        "nxm init should set up a memory workspace"
    );
    tmp
}

/// A flow+memory workspace fixture: both modules active in one shared `.nxs/`.
fn flow_and_memory_fixture() -> TempDir {
    let tmp = flow_fixture();
    let status = StdCommand::new(cargo_bin("nxm"))
        .current_dir(tmp.path())
        .arg("init")
        .status()
        .expect("spawn nxm init");
    assert!(
        status.success(),
        "nxm init joins the existing flow workspace"
    );
    tmp
}

/// Run an `nxm` command against the fixture (pinned clock + actor).
fn nxm(dir: &Path, args: &[&str]) {
    let status = StdCommand::new(cargo_bin("nxm"))
        .current_dir(dir)
        .args(args)
        .env("NXM_NOW", NOW)
        .env("NXM_ACTOR", "alice")
        .status()
        .expect("spawn nxm");
    assert!(status.success(), "nxm {args:?} should succeed");
}

/// Run `nxm --json <args>` against the fixture (pinned clock + actor) and parse its stdout.
fn nxm_json(dir: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let out = StdCommand::new(cargo_bin("nxm"))
        .current_dir(dir)
        .args(&full)
        .env("NXM_NOW", NOW)
        .env("NXM_ACTOR", "alice")
        .output()
        .expect("spawn nxm --json");
    assert!(
        out.status.success(),
        "nxm --json {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("nxm --json output parses")
}

/// Seed a memory fixture with two facts (key-ordered: auth-jwt before dolt-phantoms).
fn seed_memories(dir: &Path) {
    nxm(
        dir,
        &[
            "remember",
            "auth uses JWT, not sessions",
            "--key",
            "auth-jwt",
            "--introduction",
            "in one line",
        ],
    );
    nxm(
        dir,
        &[
            "remember",
            "Dolt phantom DBs hide in three places",
            "--key",
            "dolt-phantoms",
            "--introduction",
            "in one line",
        ],
    );
}

/// The set of tool names the server exposes for `dir` (drives the fan-out gating assertions).
async fn tool_names(client: &RunningService<RoleClient, ()>) -> std::collections::HashSet<String> {
    client
        .list_all_tools()
        .await
        .expect("tools/list")
        .into_iter()
        .map(|t| t.name.to_string())
        .collect()
}

/// Apply a memory write `tool`+`args` to an MCP fixture and the matching `nxm <cli>` to an
/// identically-seeded CLI fixture, returning both receipts (now/actor pinned on both seams).
async fn mem_write_receipts(
    mcp_fix: &Path,
    cli_fix: &Path,
    tool: &'static str,
    args: Value,
    cli: &[&str],
) -> (Value, Value) {
    let client = connect(mcp_fix).await;
    let tool_receipt = tool_sc(&client, tool, args).await;
    client.cancel().await.expect("clean shutdown");
    let cli_receipt = nxm_json(cli_fix, cli);
    (tool_receipt, cli_receipt)
}

// ---- #76u.2: the memory surface is fan-out-gated on an active memory module ----------------------

#[tokio::test]
async fn memory_tools_appear_only_when_the_memory_module_is_active() {
    // A memory workspace exposes the full memory surface; a flow-only workspace exposes NONE of it.
    let mem = memory_fixture();
    let client = connect(mem.path()).await;
    let names = tool_names(&client).await;
    for t in MEMORY_TOOLS {
        assert!(
            names.contains(*t),
            "memory tool {t} is present on an active memory workspace"
        );
    }
    client.cancel().await.expect("clean shutdown");

    let flow = flow_fixture();
    let client = connect(flow.path()).await;
    let names = tool_names(&client).await;
    for t in MEMORY_TOOLS {
        assert!(
            !names.contains(*t),
            "memory tool {t} is absent on a flow-only workspace (#76u.2)"
        );
    }
    assert!(
        names.contains("flow_next"),
        "flow tools remain present on a flow-only workspace"
    );
    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn both_module_surfaces_coexist_in_a_combined_workspace() {
    // flow + memory both active in one `.nxs/`: the merged router exposes BOTH full surfaces.
    let tmp = flow_and_memory_fixture();
    let client = connect(tmp.path()).await;
    let names = tool_names(&client).await;
    for t in FLOW_TOOLS {
        assert!(
            names.contains(*t),
            "flow tool {t} present in a combined workspace"
        );
    }
    for t in MEMORY_TOOLS {
        assert!(
            names.contains(*t),
            "memory tool {t} present in a combined workspace"
        );
    }
    client.cancel().await.expect("clean shutdown");
}

// ---- #76u.4: cross-seam parity (read) -----------------------------------------------------------

#[tokio::test]
async fn memory_list_structured_content_matches_nxm_memories_json() {
    let tmp = memory_fixture();
    seed_memories(tmp.path());
    let client = connect(tmp.path()).await;

    let tool = tool_sc(&client, "memory_list", json!({})).await;
    let cli = nxm_json(tmp.path(), &["memories"]);
    // List ops wrap their CLI-identical array under `items` (the MCP-legal object shim, #76u.9).
    assert_eq!(
        tool["items"], cli,
        "memory_list structuredContent.items == `nxm memories --json`"
    );
    assert_eq!(
        tool["items"].as_array().unwrap().len(),
        2,
        "both seeded memories are listed"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn memory_search_structured_content_matches_nxm_memories_search_json() {
    let tmp = memory_fixture();
    seed_memories(tmp.path());
    let client = connect(tmp.path()).await;

    let tool = tool_sc(&client, "memory_search", json!({ "query": "dolt" })).await;
    let cli = nxm_json(tmp.path(), &["memories", "dolt"]);
    assert_eq!(
        tool["items"], cli,
        "memory_search structuredContent.items == `nxm memories <q> --json`"
    );
    let items = tool["items"].as_array().unwrap();
    assert_eq!(
        items.len(),
        1,
        "the case-insensitive substring narrows to one"
    );
    assert_eq!(items[0]["key"], "dolt-phantoms");

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn memory_show_structured_content_matches_nxm_recall_json() {
    let tmp = memory_fixture();
    seed_memories(tmp.path());
    let client = connect(tmp.path()).await;

    let tool = tool_sc(&client, "memory_show", json!({ "key": "auth-jwt" })).await;
    let cli = nxm_json(tmp.path(), &["recall", "auth-jwt"]);
    assert_eq!(
        tool, cli,
        "memory_show structuredContent == `nxm recall <key> --json`"
    );
    assert_eq!(tool["body"], "auth uses JWT, not sessions");
    assert_eq!(tool["active"], true);

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn memory_show_unknown_key_maps_to_a_not_found_tool_error() {
    let tmp = memory_fixture();
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with("memory_show", json!({ "key": "ghost" })))
        .await
        .expect("memory_show returns a tool result (not a protocol error)");
    assert_eq!(res.is_error, Some(true), "an unknown key is a tool error");
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "not_found",
        "domain error mapped to the closed kind set"
    );

    client.cancel().await.expect("clean shutdown");
}

// ---- #76u.3 / #76u.4: the memory write tools, cross-seam parity ----------------------------------

#[tokio::test]
async fn memory_add_structured_content_matches_nxm_remember_json() {
    // memory_add with no key mints the content-hash auto-key — deterministic, so two fresh fixtures
    // produce byte-identical records. Mirrors `nxm remember <text>` (no --key).
    let mcp_fix = memory_fixture();
    let cli_fix = memory_fixture();

    let (tool, cli) = mem_write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "memory_add",
        json!({ "introduction": "in one line", "text": "always run tests with -race", "now": NOW, "actor": "alice" }),
        &["remember", "always run tests with -race", "--introduction", "in one line"],
    )
    .await;
    assert_eq!(tool, cli, "memory_add receipt == `nxm remember --json`");
    assert_eq!(tool["body"], "always run tests with -race");
    assert_eq!(
        tool["author"], "alice",
        "the per-tool actor authors the fact"
    );
    assert_eq!(tool["updated"], NOW, "the explicit now is stamped");
    assert!(
        tool["key"].as_str().unwrap().starts_with("f-"),
        "no --key ⇒ the content-hash auto-key"
    );
    assert_eq!(tool["active"], true);
}

#[tokio::test]
async fn memory_update_structured_content_matches_nxm_remember_key_json() {
    // memory_update upserts in place under an explicit key. Seed both fixtures with the key, then
    // overwrite the body on each. Mirrors `nxm remember <text> --key <key>`.
    let mcp_fix = memory_fixture();
    let cli_fix = memory_fixture();
    nxm(
        mcp_fix.path(),
        &[
            "remember",
            "v1",
            "--key",
            "k",
            "--introduction",
            "in one line",
        ],
    );
    nxm(
        cli_fix.path(),
        &[
            "remember",
            "v1",
            "--key",
            "k",
            "--introduction",
            "in one line",
        ],
    );

    let (tool, cli) = mem_write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "memory_update",
        json!({ "introduction": "in one line", "key": "k", "text": "v2", "now": NOW, "actor": "alice" }),
        &["remember", "v2", "--key", "k", "--introduction", "in one line"],
    )
    .await;
    assert_eq!(
        tool, cli,
        "memory_update receipt == `nxm remember --key --json`"
    );
    assert_eq!(tool["key"], "k");
    assert_eq!(tool["body"], "v2", "the body is overwritten in place");
}

#[tokio::test]
async fn memory_close_structured_content_matches_nxm_forget_json() {
    // memory_close forgets (a reversible tombstone). Its receipt is the CLI's `{ ok, key }` shape —
    // NOT the facade tombstone record — so it must mirror `nxm forget <key> --json` exactly.
    let mcp_fix = memory_fixture();
    let cli_fix = memory_fixture();
    nxm(
        mcp_fix.path(),
        &[
            "remember",
            "secret",
            "--key",
            "k",
            "--introduction",
            "in one line",
        ],
    );
    nxm(
        cli_fix.path(),
        &[
            "remember",
            "secret",
            "--key",
            "k",
            "--introduction",
            "in one line",
        ],
    );

    let (tool, cli) = mem_write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "memory_close",
        json!({ "key": "k", "now": NOW, "actor": "alice" }),
        &["forget", "k"],
    )
    .await;
    assert_eq!(tool, cli, "memory_close receipt == `nxm forget --json`");
    assert_eq!(
        tool,
        json!({ "ok": true, "key": "k" }),
        "the CLI's forget receipt shape (not the facade record)"
    );
}

#[tokio::test]
async fn memory_close_unknown_key_maps_to_a_not_found_tool_error() {
    // Forgetting a missing key is a `not_found` tool error across the seam (the existence check is in
    // the facade), not a panic / protocol error.
    let tmp = memory_fixture();
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with(
            "memory_close",
            json!({ "key": "ghost", "now": NOW }),
        ))
        .await
        .expect("call returns a tool result, not a protocol error");
    assert_eq!(res.is_error, Some(true), "an unknown key is a tool error");
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "not_found",
        "mapped to the not_found kind"
    );

    client.cancel().await.expect("clean shutdown");
}

// ---- 6j6v.9a1r: the tools can SET the classification, pinned against `nxm --json` ---------------
//
// Over MCP a memory could be read with its category/reach/references but never filed with them, so
// an MCP-writing agent produced exactly the class of entry the retrieval rule (6j6v.srpg) can show
// nowhere: no reach, invisible on every surface. These pin the fix the same way the rest of the
// seam is pinned — byte-parity with the matching `nxm <cmd> --json` invocation.

#[tokio::test]
async fn memory_add_files_the_memory_and_matches_nxm_remember_with_the_same_flags() {
    let mcp_fix = memory_fixture();
    let cli_fix = memory_fixture();

    let (tool, cli) = mem_write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "memory_add",
        json!({ "introduction": "in one line",
            "text": "the closing comment names the flake",
            "category": "rules",
            "scope": "item",
            "refs": ["6j6v.srpg", "6j6v.9a1r"],
            "now": NOW,
            "actor": "alice",
        }),
        &[
            "remember",
            "the closing comment names the flake",
            "--category",
            "rules",
            "--scope",
            "item",
            "--refs",
            "6j6v.srpg,6j6v.9a1r",
            "--introduction",
            "in one line",
        ],
    )
    .await;
    assert_eq!(tool, cli, "memory_add receipt == `nxm remember --json`");
    assert_eq!(tool["category"], "rules");
    assert_eq!(tool["scope"], "item");
    assert_eq!(
        tool["refs"],
        json!(["6j6v.9a1r", "6j6v.srpg"]),
        "canonical: sorted and deduplicated"
    );
}

#[tokio::test]
async fn memory_add_without_the_trio_still_matches_the_unclassified_cli_write() {
    // The compatibility half: a caller that sends none of the three writes exactly what this tool
    // wrote before they existed — the status-quo defaults, byte for byte.
    let mcp_fix = memory_fixture();
    let cli_fix = memory_fixture();

    let (tool, cli) = mem_write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "memory_add",
        json!({ "introduction": "in one line", "text": "unfiled fact", "now": NOW, "actor": "alice" }),
        &["remember", "unfiled fact", "--introduction", "in one line"],
    )
    .await;
    assert_eq!(tool, cli, "an omitted trio is not a write");
    assert_eq!(tool["category"], "unsorted");
    assert_eq!(tool["scope"], "project");
    assert_eq!(tool["refs"], json!([]));
}

#[tokio::test]
async fn memory_update_files_in_place_and_matches_nxm_remember_key_with_the_same_flags() {
    let mcp_fix = memory_fixture();
    let cli_fix = memory_fixture();
    nxm(
        mcp_fix.path(),
        &[
            "remember",
            "v1",
            "--key",
            "k",
            "--introduction",
            "in one line",
        ],
    );
    nxm(
        cli_fix.path(),
        &[
            "remember",
            "v1",
            "--key",
            "k",
            "--introduction",
            "in one line",
        ],
    );

    let (tool, cli) = mem_write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "memory_update",
        json!({ "introduction": "in one line",
            "key": "k",
            "text": "v2",
            "category": "architecture",
            "scope": "global",
            "now": NOW,
            "actor": "alice",
        }),
        &[
            "remember",
            "v2",
            "--key",
            "k",
            "--category",
            "architecture",
            "--scope",
            "global",
            "--introduction",
            "in one line",
        ],
    )
    .await;
    assert_eq!(tool, cli, "memory_update receipt == `nxm remember --json`");
    assert_eq!(tool["body"], "v2");
    assert_eq!(tool["scope"], "global");
    assert_eq!(tool["refs"], json!([]), "an omitted flag is not a reset");
}

#[tokio::test]
async fn memory_classify_files_an_existing_memory_and_matches_nxm_classify() {
    let mcp_fix = memory_fixture();
    let cli_fix = memory_fixture();
    for fix in [mcp_fix.path(), cli_fix.path()] {
        nxm(
            fix,
            &[
                "remember",
                "durable body",
                "--key",
                "k",
                "--introduction",
                "in one line",
            ],
        );
    }

    let (tool, cli) = mem_write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "memory_classify",
        json!({
            "key": "k",
            "scope": "item",
            "refs": ["6j6v.srpg"],
            "now": NOW,
            "actor": "alice",
        }),
        &["classify", "k", "--scope", "item", "--refs", "6j6v.srpg"],
    )
    .await;
    assert_eq!(
        tool, cli,
        "memory_classify receipt == `nxm classify --json`"
    );
    assert_eq!(tool["scope"], "item");
    assert_eq!(tool["refs"], json!(["6j6v.srpg"]));
    assert_eq!(
        tool["body"], "durable body",
        "filing never touches the text — that is what the memory IS"
    );
    assert_eq!(
        tool["category"], "unsorted",
        "an omitted flag leaves its register alone"
    );
}

#[tokio::test]
async fn memory_classify_with_nothing_to_set_is_a_validation_error() {
    // The facade's shared guard: a caller that meant to change something deserves to hear that it
    // did not, rather than get a silent no-op receipt.
    let tmp = memory_fixture();
    nxm(
        tmp.path(),
        &[
            "remember",
            "body",
            "--key",
            "k",
            "--introduction",
            "in one line",
        ],
    );
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with(
            "memory_classify",
            json!({ "key": "k", "now": NOW }),
        ))
        .await
        .expect("call returns a tool result, not a protocol error");
    assert_eq!(res.is_error, Some(true));
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "validation"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn an_unknown_reach_is_rejected_before_anything_is_written() {
    // The reach is a closed vocabulary, so a typo must be a loud error and not a stored word that
    // no surface can read. And it is rejected BEFORE the write: the memory must not exist after.
    let tmp = memory_fixture();
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with(
            "memory_add",
            json!({ "introduction": "in one line", "text": "a fact", "scope": "workspace", "now": NOW }),
        ))
        .await
        .expect("call returns a tool result, not a protocol error");
    assert_eq!(res.is_error, Some(true), "an unknown reach is a tool error");
    let envelope = res.structured_content.unwrap();
    assert_eq!(envelope["error"]["kind"], "validation");
    assert!(
        envelope["error"]["msg"]
            .as_str()
            .unwrap()
            .contains("item | project | global"),
        "the message names the accepted set: {envelope}"
    );
    let listed = tool_sc(&client, "memory_list", json!({})).await;
    assert_eq!(
        listed["items"],
        json!([]),
        "a rejected call wrote nothing: {listed}"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn an_empty_reach_is_a_value_not_an_omission() {
    // PR #282 review, Code Quality #3: `""` must be rejected like any other unknown reach — the
    // empty-means-unset convention belongs to the `workspace`/`actor` fallback knobs, and letting
    // it leak onto `scope` would mean "leave it as it was" here while `nxm --scope ""` errors.
    let tmp = memory_fixture();
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with(
            "memory_add",
            json!({ "introduction": "in one line", "text": "a fact", "scope": "", "now": NOW }),
        ))
        .await
        .expect("call returns a tool result, not a protocol error");
    assert_eq!(res.is_error, Some(true), "an empty reach is a tool error");
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "validation"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn memory_reorder_stores_the_reading_order_and_matches_nxm_reorder() {
    let mcp_fix = memory_fixture();
    let cli_fix = memory_fixture();
    for fix in [mcp_fix.path(), cli_fix.path()] {
        nxm(
            fix,
            &[
                "remember",
                "a body",
                "--key",
                "a",
                "--introduction",
                "in one line",
            ],
        );
        nxm(
            fix,
            &[
                "remember",
                "b body",
                "--key",
                "b",
                "--introduction",
                "in one line",
            ],
        );
    }

    let (tool, cli) = mem_write_receipts(
        mcp_fix.path(),
        cli_fix.path(),
        "memory_reorder",
        json!({ "keys": ["b", "a"], "now": NOW, "actor": "alice" }),
        &["reorder", "b", "a"],
    )
    .await;
    // `nxm reorder --json` returns the ARRAY of records; the tool wraps a non-object under `items`
    // (#76u.9), exactly like the list-shaped reads — so compare the array to the wrapped one.
    assert_eq!(
        tool["items"], cli,
        "memory_reorder receipt == `nxm reorder --json`"
    );
    let ordered: Vec<(&str, i64)> = tool["items"]
        .as_array()
        .expect("records array")
        .iter()
        .map(|r| (r["key"].as_str().unwrap(), r["ordinal"].as_i64().unwrap()))
        .collect();
    assert_eq!(ordered, [("b", 1), ("a", 2)], "position 1, 2, … in order");
}

#[tokio::test]
async fn memory_reorder_rejects_an_unknown_key_without_moving_anything() {
    let tmp = memory_fixture();
    nxm(
        tmp.path(),
        &[
            "remember",
            "a body",
            "--key",
            "a",
            "--introduction",
            "in one line",
        ],
    );
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with(
            "memory_reorder",
            json!({ "keys": ["a", "ghost"], "now": NOW }),
        ))
        .await
        .expect("call returns a tool result, not a protocol error");
    assert_eq!(res.is_error, Some(true));
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "not_found"
    );
    let shown = tool_sc(&client, "memory_show", json!({ "key": "a" })).await;
    assert_eq!(
        shown["ordinal"],
        Value::Null,
        "a rejected sequence leaves the order exactly as it was: {shown}"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn memory_add_empty_text_maps_to_a_validation_error() {
    // An empty body reaches the facade's shared guard and surfaces as a `validation` tool error.
    let tmp = memory_fixture();
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with(
            "memory_add",
            json!({ "text": "", "introduction": "in one line", "now": NOW }),
        ))
        .await
        .expect("call returns a tool result, not a protocol error");
    assert_eq!(res.is_error, Some(true), "an empty body is a tool error");
    assert_eq!(
        res.structured_content.unwrap()["error"]["kind"],
        "validation",
        "mapped to the validation kind"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn memory_add_without_an_introduction_is_refused_at_the_tool_boundary() {
    // Review of PR #381, Integrity #6. `introduction` is a plain required `String` in the add/update
    // params, so serde rejects an omitted one before the handler ever runs — but every other test
    // here supplies it, so nothing pinned that the boundary actually holds. A future edit making it
    // `Option<String>` "for convenience" would sail through the whole suite and quietly reopen the
    // one guarantee this feature is: that no seam can write a memory the session start cannot name.
    let tmp = memory_fixture();
    let client = connect(tmp.path()).await;

    let res = client
        .call_tool(call_with(
            "memory_add",
            json!({ "text": "a fact with no line to read it by", "now": NOW }),
        ))
        .await
        .expect("call returns a tool result, not a protocol error");
    // Refused during PARAMETER DESERIALIZATION — the handler is never entered, which is the
    // strongest form this guarantee can take — and the refusal names the missing field, so a
    // caller is told what to send rather than left to guess.
    assert_eq!(
        res.is_error,
        Some(true),
        "an omitted introduction is refused"
    );
    let text = res
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<String>();
    assert!(
        text.contains("introduction"),
        "…and the refusal names the field: {text}"
    );

    // …and nothing was written.
    let listed = tool_sc(&client, "memory_list", json!({})).await;
    assert_eq!(
        listed["items"].as_array().map(Vec::len),
        Some(0),
        "the refused call left the store empty: {listed}"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn memory_write_tools_are_deterministic_under_a_pinned_now() {
    // The SAME memory_add against two fresh fixtures, with now/actor pinned, yields byte-identical
    // receipts — no hidden clock/random in the memory seam (the auto-key is a pure content hash).
    let a = memory_fixture();
    let b = memory_fixture();

    let args = json!({ "text": "deterministic fact", "introduction": "in one line", "now": NOW, "actor": "alice" });
    let ca = connect(a.path()).await;
    let ra = tool_sc(&ca, "memory_add", args.clone()).await;
    ca.cancel().await.unwrap();
    let cb = connect(b.path()).await;
    let rb = tool_sc(&cb, "memory_add", args).await;
    cb.cancel().await.unwrap();

    assert_eq!(
        ra.to_string(),
        rb.to_string(),
        "pinned now/actor ⇒ byte-identical add receipts"
    );
}

// ---- #zxj: the actor hybrid threads the right identity onto a memory write ----------------------
//
// The memory receipt carries `author` directly (the MemoryRecord), so these assert it from the
// receipt rather than reading the store (the flow analogue uses op_authors since flow receipts omit
// the author). Together they cover the launch-default + env-fallback hybrid tiers for memory writes,
// and pin the DELIBERATE cross-seam divergence flagged by the PR review (CQ#3 / I&R#1): an unpinned
// memory MCP write authors under the umbrella server's nxs identity (NXF_ACTOR), not nxm's NXM_ACTOR.

/// Spawn the server with NO per-tool/launch actor but BOTH env identities set to DIFFERENT values —
/// `NXF_ACTOR` (the nxs server's env fallback) and `NXM_ACTOR` (the `nxm` CLI's) — so a memory write
/// proves it resolves the author through the umbrella's ONE hybrid (NXF_ACTOR, #zxj), never the
/// per-module NXM_ACTOR the `nxm` CLI would use. `NXS_ACTOR` is cleared so no launch default applies.
async fn connect_memory_env_actors(
    dir: &Path,
    nxf_actor: &str,
    nxm_actor: &str,
) -> RunningService<RoleClient, ()> {
    let mut cmd = Command::new(cargo_bin("nxs"));
    cmd.arg("mcp")
        .arg("serve")
        .arg("--workspace")
        .arg(dir)
        .env("NXS_NOW", NOW)
        .env("NXM_NOW", NOW)
        .env("NXF_ACTOR", nxf_actor)
        .env("NXM_ACTOR", nxm_actor)
        .env_remove("NXS_ACTOR");
    let transport = TokioChildProcess::new(cmd).expect("spawn nxs mcp serve");
    ().serve(transport)
        .await
        .expect("client connects and the initialize handshake succeeds")
}

#[tokio::test]
async fn memory_launch_actor_default_authors_a_write_without_a_per_tool_actor() {
    // With no per-tool actor, the server-instance `--actor` launch default authors the memory write
    // (#zxj) — the memory analogue of the flow seam's launch_actor_default test, proven via the
    // memory_add receipt's `author`.
    let tmp = memory_fixture();
    let client = connect_with_actor(tmp.path(), "bob").await;
    let rec = tool_sc(
        &client,
        "memory_add",
        json!({ "introduction": "in one line", "text": "a launch-actor fact", "now": NOW }),
    )
    .await;
    client.cancel().await.expect("clean shutdown");
    assert_eq!(
        rec["author"], "bob",
        "the launch --actor default authors the memory write"
    );
}

#[tokio::test]
async fn memory_unpinned_write_authors_under_the_nxs_identity_not_nxms() {
    // The convergent PR-review finding (CQ#3 / I&R#1), locked in: with no per-tool or launch `--actor`,
    // a memory write resolves the author through the umbrella server's ONE hybrid — NXF_ACTOR/USER/
    // "nxs" (#zxj) — NOT the `nxm` CLI's NXM_ACTOR/"nxm". So the byte-parity with `nxm <cmd> --json`
    // holds with `actor` pinned (as every parity test drives it); the unpinned default is deliberately
    // the nxs server identity. Both env identities are set to different values, so the write must pick
    // NXF_ACTOR — proving memory MCP writes do not use nxm's NXM_ACTOR default.
    let tmp = memory_fixture();
    let client = connect_memory_env_actors(tmp.path(), "nxs-server", "nxm-cli").await;
    let rec = tool_sc(
        &client,
        "memory_add",
        json!({ "introduction": "in one line", "text": "whose identity authors this?", "now": NOW }),
    )
    .await;
    client.cancel().await.expect("clean shutdown");
    assert_eq!(
        rec["author"], "nxs-server",
        "an unpinned memory write authors under the nxs server hybrid (NXF_ACTOR)"
    );
    assert_ne!(
        rec["author"], "nxm-cli",
        "memory MCP writes do NOT fall back to nxm's NXM_ACTOR default (#zxj)"
    );
}

// ---- #76u.2 / #76u.3: memory tool annotations ---------------------------------------------------

#[tokio::test]
async fn memory_read_tools_advertise_read_only_annotations() {
    let tmp = memory_fixture();
    let client = connect(tmp.path()).await;
    let tools = client.list_all_tools().await.expect("tools/list");

    for name in ["memory_list", "memory_search", "memory_show"] {
        let tool = tools
            .iter()
            .find(|t| t.name.as_ref() == name)
            .unwrap_or_else(|| panic!("{name} is in tools/list"));
        let ann = tool
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("{name} carries machine-readable annotations"));
        assert_eq!(ann.read_only_hint, Some(true), "{name} readOnlyHint:true");
        assert_eq!(
            ann.open_world_hint,
            Some(false),
            "{name} openWorldHint:false (closed local store)"
        );
        assert!(
            ann.title.as_ref().is_some_and(|t| !t.is_empty()),
            "{name} carries a title"
        );
    }

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn memory_write_tools_advertise_mutation_annotations_per_op() {
    let tmp = memory_fixture();
    let client = connect(tmp.path()).await;
    let tools = client.list_all_tools().await.expect("tools/list");

    // (name, destructive_hint, idempotent_hint) — the per-op contract.
    let expected: &[(&str, bool, bool)] = &[
        ("memory_add", false, true), // content-addressed capture: additive, idempotent (hash key)
        ("memory_update", true, true), // overwrites the body under a key; idempotent upsert
        ("memory_close", true, true), // forgets (removes from the active set); idempotent
        // 6j6v.9a1r: both leave the memory's text alone but overwrite the classification/order
        // registers in place — destructive by this seam's own per-op rule (an op that overwrites an
        // existing field value), and idempotent (re-applying lands the same state).
        ("memory_classify", true, true),
        ("memory_reorder", true, true),
    ];
    for (name, destructive, idempotent) in expected {
        let tool = tools
            .iter()
            .find(|t| t.name.as_ref() == *name)
            .unwrap_or_else(|| panic!("{name} is in tools/list"));
        let ann = tool
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("{name} carries annotations"));
        assert_eq!(
            ann.read_only_hint,
            Some(false),
            "{name} is a mutation (readOnlyHint:false)"
        );
        assert_eq!(
            ann.open_world_hint,
            Some(false),
            "{name} openWorldHint:false"
        );
        assert_eq!(
            ann.destructive_hint,
            Some(*destructive),
            "{name} destructiveHint"
        );
        assert_eq!(
            ann.idempotent_hint,
            Some(*idempotent),
            "{name} idempotentHint"
        );
        assert!(
            ann.title.as_ref().is_some_and(|t| !t.is_empty()),
            "{name} carries a title"
        );
    }

    client.cancel().await.expect("clean shutdown");
}

// ---- 6j6v.srpg: the retrieval rule rides BOTH flow seams identically ----------------------------

#[tokio::test]
async fn flow_show_and_flow_next_carry_the_item_memories_exactly_as_the_cli_does() {
    // The seam invariant this rule had to respect: a tool's structuredContent is byte-identical to
    // the matching `nxf <cmd> --json`. `nxf show`/`nxf next` gained the item-scoped memory join, so
    // an agent reading over MCP must get the SAME join — otherwise the two seams answer "what does
    // this item carry" differently, which is the drift the parity gate exists to prevent.
    let tmp = flow_and_memory_fixture();
    nxf(
        tmp.path(),
        &[
            "create",
            "--type",
            "feature",
            "--title",
            "Rule carrier",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );
    let id = "ab12.0001";
    nxm(
        tmp.path(),
        &[
            "remember",
            "the distillate this item carries",
            "--key",
            "ticket",
            "--scope",
            "item",
            "--refs",
            id,
            "--introduction",
            "in one line",
        ],
    );
    let client = connect(tmp.path()).await;

    let tool = tool_sc(&client, "flow_show", json!({ "id": id })).await;
    let cli = without_presentation_labels(&nxf_json(tmp.path(), &["show", id]));
    assert_eq!(
        tool, cli,
        "flow_show == `nxf show --json`, memories included"
    );
    assert_eq!(
        tool["memories"][0]["body"], "the distillate this item carries",
        "the full record, not a pointer: {tool}"
    );

    let tool = tool_sc(&client, "flow_next", json!({})).await;
    let cli = without_presentation_labels(&nxf_json(tmp.path(), &["next"]));
    assert_eq!(
        tool["items"], cli,
        "flow_next == `nxf next --json`, hint included"
    );
    assert_eq!(
        tool["items"][0]["memories"], 1,
        "the hint is a count on the work list: {tool}"
    );

    client.cancel().await.expect("clean shutdown");
}

// ---- #76u.2: the session-start nudge names memory_list once memory is active ---------------------

#[tokio::test]
async fn instructions_name_memory_list_when_memory_is_active() {
    // Once memory is active, the static session-start nudge points at `memory_list` (the flat dump)
    // alongside `flow_next` — and `memory_list` is a real registered tool (ctm holds via the
    // instructions_reference_only_registered_tools cross-check on the flow surface).
    let tmp = flow_and_memory_fixture();
    let client = connect(tmp.path()).await;

    let instructions = client
        .peer_info()
        .expect("server info")
        .instructions
        .clone()
        .expect("instructions for an active workspace");
    let head = instructions.lines().take(3).collect::<Vec<_>>().join(" ");
    assert!(
        head.contains("flow_next") && head.contains("memory_list"),
        "both session-start nudges are up front: {head}"
    );
    // Every memory_/flow_ token named in the instructions is a registered tool.
    let tools = tool_names(&client).await;
    for tok in tool_tokens(&instructions) {
        assert!(
            tools.contains(&tok),
            "instructions reference a non-existent tool `{tok}` (not in tools/list)"
        );
    }

    client.cancel().await.expect("clean shutdown");
}

// ============================================================================================
// #c3i — `nxs mcp install`: the host-config writer, driven for real through the binary.
//
// These run the actual `nxs mcp install` under a controlled `HOME` (XDG cleared, mirroring
// `connect_default`) so the host config paths resolve inside a tempdir — never the developer's
// real `~/Library/Application Support/Claude/…`. They assert the absolute-path entry is written
// and that a re-run is idempotent (the two acceptance criteria of #c3i).
// ============================================================================================

/// Run `nxs mcp install --json <args>` under `HOME=home` (XDG cleared) and parse stdout.
fn mcp_install_json(home: &Path, args: &[&str]) -> Value {
    let mut full = vec!["mcp", "install", "--json"];
    full.extend_from_slice(args);
    let out = StdCommand::new(cargo_bin("nxs"))
        .args(&full)
        .pin_home(home)
        .output()
        .expect("spawn nxs mcp install");
    assert!(
        out.status.success(),
        "nxs mcp install {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("nxs mcp install --json output parses")
}

/// The single host record for `host_id` in an install report.
fn host_record<'a>(report: &'a Value, host_id: &str) -> &'a Value {
    report["hosts"]
        .as_array()
        .expect("hosts array")
        .iter()
        .find(|h| h["host"] == host_id)
        .unwrap_or_else(|| panic!("host `{host_id}` in report {report}"))
}

#[test]
fn mcp_install_writes_an_absolute_path_entry_and_is_idempotent() {
    let home = TempDir::new().unwrap();
    // Explicit `--host cursor` force-writes even though nothing is installed — so we exercise the
    // writer without depending on a real host being present in CI.
    let report = mcp_install_json(home.path(), &["--host", "cursor"]);

    // The command is the ABSOLUTE path of the running binary + `mcp serve` (acceptance: absolute).
    let command = report["command"].as_str().expect("command string");
    assert!(
        Path::new(command).is_absolute(),
        "entry command is absolute: {command}"
    );
    assert_eq!(
        Path::new(command).file_name().and_then(|s| s.to_str()),
        Some("nxs"),
        "entry command points at the nxs binary: {command}"
    );
    let cursor = host_record(&report, "cursor");
    assert_eq!(cursor["status"], "written");
    // A plain install (no `--setup`) ran no workspace materialization → the `setup` key is present
    // and null (guards against a stray/absent key regression in the #65y envelope).
    assert_eq!(report["setup"], json!(null), "no setup → setup is null");

    // The file on disk carries the entry under `mcpServers.nxs`, pointing at `mcp serve` with no
    // pinned workspace (the App-Data-Home default).
    let config_path = home.path().join(".cursor").join("mcp.json");
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(written["mcpServers"]["nxs"]["command"], json!(command));
    assert_eq!(
        written["mcpServers"]["nxs"]["args"],
        json!(["mcp", "serve"])
    );

    // Idempotency (acceptance: re-run changes nothing) — the report says `unchanged` and the bytes
    // on disk are identical.
    let before = std::fs::read(&config_path).unwrap();
    let rerun = mcp_install_json(home.path(), &["--host", "cursor"]);
    assert_eq!(host_record(&rerun, "cursor")["status"], "unchanged");
    assert_eq!(
        std::fs::read(&config_path).unwrap(),
        before,
        "re-run rewrote nothing"
    );
}

#[test]
fn mcp_install_runner_npx_writes_the_portable_shim_entry() {
    // #kz8: `--runner npx` writes the machine-independent `npx @nexus-flow/mcp` entry (the "nxs not
    // installed at all" path) instead of the absolute-path one, pinning the workspace after `--`.
    let home = TempDir::new().unwrap();
    let report = mcp_install_json(
        home.path(),
        &[
            "--host",
            "cursor",
            "--runner",
            "npx",
            "--workspace",
            "/proj/x",
        ],
    );

    // The reported command is the npx invocation, not a local binary path.
    assert_eq!(report["command"], json!("npx -y @nexus-flow/mcp"));
    assert_eq!(host_record(&report, "cursor")["status"], "written");

    // On disk: the entry launches `npx`, and the host args ride after `--` (the shim prepends
    // `mcp serve`), so nxs need not exist on the host yet.
    let config_path = home.path().join(".cursor").join("mcp.json");
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(written["mcpServers"]["nxs"]["command"], json!("npx"));
    assert_eq!(
        written["mcpServers"]["nxs"]["args"],
        json!(["-y", "@nexus-flow/mcp", "--", "--workspace", "/proj/x"])
    );
}

#[test]
fn mcp_install_autodetects_installed_hosts_and_pins_a_workspace() {
    let home = TempDir::new().unwrap();
    // Simulate "Cursor installed" by creating its config dir; Claude Desktop + Windsurf stay absent.
    std::fs::create_dir_all(home.path().join(".cursor")).unwrap();

    let report = mcp_install_json(home.path(), &["--workspace", "/proj/x"]);
    // Auto-detect: the present host is written, the absent ones skipped.
    assert_eq!(host_record(&report, "cursor")["status"], "written");
    assert_eq!(host_record(&report, "claude-desktop")["status"], "skipped");
    assert_eq!(host_record(&report, "windsurf")["status"], "skipped");
    assert_eq!(report["workspace"], json!("/proj/x"));

    // The pinned workspace lands in the entry's args.
    let config_path = home.path().join(".cursor").join("mcp.json");
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(
        written["mcpServers"]["nxs"]["args"],
        json!(["mcp", "serve", "--workspace", "/proj/x"])
    );
}

#[test]
fn mcp_install_writes_the_amazon_quick_host_config() {
    // 5kky: Amazon Quick (the renamed Amazon Q Developer) reads the same `mcpServers` schema from
    // its global CLI config at `~/.aws/amazonq/mcp.json`. An explicit `--host amazon-quick` force-
    // writes there through the shared, non-destructive writer.
    let home = TempDir::new().unwrap();
    let report = mcp_install_json(home.path(), &["--host", "amazon-quick"]);

    let quick = host_record(&report, "amazon-quick");
    assert_eq!(quick["status"], "written");
    assert_eq!(quick["label"], "Amazon Quick");

    // On disk: the entry sits under `mcpServers.nxs` at the Amazon Q Developer CLI global path,
    // pointing at `mcp serve` with the absolute nxs command.
    let config_path = home.path().join(".aws").join("amazonq").join("mcp.json");
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    let command = report["command"].as_str().expect("command string");
    assert_eq!(written["mcpServers"]["nxs"]["command"], json!(command));
    assert_eq!(
        written["mcpServers"]["nxs"]["args"],
        json!(["mcp", "serve"])
    );
}

#[test]
fn mcp_install_preserves_a_users_existing_server_entries() {
    let home = TempDir::new().unwrap();
    let config_path = home.path().join(".cursor").join("mcp.json");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    // A pre-existing host config with an unrelated server + a top-level key.
    std::fs::write(
        &config_path,
        r#"{"theme":"dark","mcpServers":{"other":{"command":"/bin/other","args":[]}}}"#,
    )
    .unwrap();

    mcp_install_json(home.path(), &["--host", "cursor"]);

    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(written["theme"], json!("dark"), "top-level key preserved");
    assert_eq!(
        written["mcpServers"]["other"],
        json!({"command": "/bin/other", "args": []}),
        "the user's other server is preserved"
    );
    assert!(
        written["mcpServers"]["nxs"].is_object(),
        "ours is added alongside"
    );
}

#[test]
fn mcp_install_fails_loudly_on_a_corrupt_targeted_host_and_preserves_it() {
    // An explicitly named `--host` whose config is corrupt is a loud, non-zero-exit failure that
    // leaves the user's file byte-for-byte intact — never a truncating clobber (the non-destructive
    // promise, proven at the process level).
    let home = TempDir::new().unwrap();
    let cfg = home.path().join(".cursor").join("mcp.json");
    std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    std::fs::write(&cfg, "{ this is not json").unwrap();

    let out = StdCommand::new(cargo_bin("nxs"))
        .args(["mcp", "install", "--json", "--host", "cursor"])
        .pin_home(home.path())
        .output()
        .expect("spawn nxs mcp install");

    assert!(
        !out.status.success(),
        "a corrupt targeted host is a loud failure"
    );
    let env: Value = serde_json::from_slice(&out.stdout).expect("error envelope on stdout parses");
    assert_eq!(env["error"]["kind"], "validation");
    assert_eq!(
        std::fs::read_to_string(&cfg).unwrap(),
        "{ this is not json",
        "the corrupt config is left untouched"
    );
}

// ============================================================================================
// 0jq8 — the multi-workspace registry (`~/.nexusflow/workspaces.toml`) + `list_workspaces`.
//
// These run under a controlled `HOME` so the global registry resolves inside a tempdir. They prove
// the two acceptance halves: the registry is READ + MAINTAINED (auto-registered by `nxs mcp install
// --workspace`), and the `list_workspaces` tool returns a DETERMINISTIC list — byte-identical to the
// `nxs mcp workspaces --json` CLI parity partner (the epic invariant, extended to the umbrella op).
// ============================================================================================

/// Run `nxs mcp workspaces --json` under `HOME=home` (XDG cleared) and parse stdout.
fn mcp_workspaces_json(home: &Path) -> Value {
    let out = StdCommand::new(cargo_bin("nxs"))
        .args(["mcp", "workspaces", "--json"])
        .pin_home(home)
        .output()
        .expect("spawn nxs mcp workspaces");
    assert!(
        out.status.success(),
        "nxs mcp workspaces --json: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("nxs mcp workspaces --json output parses")
}

/// Run `nxs mcp workspaces` (NO `--json`) under `HOME=home` (XDG cleared) and return stdout — the
/// human-readable render (0jq8 Test #1).
fn mcp_workspaces_human(home: &Path) -> String {
    let out = StdCommand::new(cargo_bin("nxs"))
        .args(["mcp", "workspaces"])
        .pin_home(home)
        .output()
        .expect("spawn nxs mcp workspaces");
    assert!(
        out.status.success(),
        "nxs mcp workspaces: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("nxs mcp workspaces output is UTF-8")
}

#[test]
fn mcp_workspaces_human_output_hints_when_empty_then_lists_registered_boards() {
    // The human (non-`--json`) branch of `nxs mcp workspaces` (0jq8 Test #1): the empty-registry hint,
    // then the aligned table naming a registered board's name + path.
    let home = TempDir::new().unwrap();
    assert!(
        mcp_workspaces_human(home.path()).contains("No workspaces registered"),
        "empty registry prints the register hint"
    );

    let ws = TempDir::new().unwrap();
    let ws_path = ws.path().to_str().unwrap();
    let ws_name = ws.path().file_name().unwrap().to_str().unwrap();
    mcp_install_json(home.path(), &["--host", "cursor", "--workspace", ws_path]);

    let listed = mcp_workspaces_human(home.path());
    assert!(
        listed.contains(ws_name),
        "human table names the board: {listed:?}"
    );
    assert!(
        listed.contains(ws_path),
        "human table shows the path: {listed:?}"
    );
}

#[test]
fn mcp_install_registers_the_pinned_workspace_idempotently() {
    // The "registry maintained" half (0jq8): a pinned `--workspace` is upserted into the registry
    // under a name derived from its final path component, and re-running changes nothing.
    let home = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    let ws_path = ws.path().to_str().unwrap();
    let ws_name = ws.path().file_name().unwrap().to_str().unwrap();

    mcp_install_json(home.path(), &["--host", "cursor", "--workspace", ws_path]);
    let listed = mcp_workspaces_json(home.path());
    assert_eq!(
        listed,
        json!([{ "name": ws_name, "path": ws_path }]),
        "the pinned workspace is registered as {{name: basename, path}}"
    );

    // Re-run: idempotent — still exactly one entry (no duplicate).
    mcp_install_json(home.path(), &["--host", "cursor", "--workspace", ws_path]);
    assert_eq!(
        mcp_workspaces_json(home.path()),
        listed,
        "re-registering the same workspace adds no duplicate"
    );
}

#[test]
fn mcp_install_without_a_pinned_workspace_registers_nothing() {
    // The App-Data-Home default (no `--workspace`) is the unnamed fallback, not a named board — it is
    // NOT auto-registered, so the registry stays empty.
    let home = TempDir::new().unwrap();
    mcp_install_json(home.path(), &["--host", "cursor"]);
    assert_eq!(
        mcp_workspaces_json(home.path()),
        json!([]),
        "a flag-less install registers no workspace"
    );
}

#[tokio::test]
async fn list_workspaces_matches_nxs_mcp_workspaces_json_and_is_deterministic() {
    // The `list_workspaces` tool's structuredContent.items is byte-identical to the
    // `nxs mcp workspaces --json` CLI (the parity partner), and both are name-sorted regardless of
    // on-disk order — the deterministic-list acceptance.
    let home = TempDir::new().unwrap();
    let reg = home.path().join(".nexusflow").join("workspaces.toml");
    std::fs::create_dir_all(reg.parent().unwrap()).unwrap();
    // Deliberately out of order on disk.
    std::fs::write(
        &reg,
        "[[workspace]]\nname = \"zeta\"\npath = \"/z\"\n\
         [[workspace]]\nname = \"alpha\"\npath = \"/a\"\n",
    )
    .unwrap();

    let client = connect_home(home.path()).await;
    let tool = tool_sc(&client, "list_workspaces", json!({})).await;
    client.cancel().await.expect("clean shutdown");

    let cli = mcp_workspaces_json(home.path());
    assert_eq!(
        tool["items"], cli,
        "list_workspaces structuredContent.items == `nxs mcp workspaces --json`"
    );
    assert_eq!(
        cli,
        json!([
            { "name": "alpha", "path": "/a" },
            { "name": "zeta", "path": "/z" },
        ]),
        "the list is deterministic — name-sorted, not on-disk order"
    );
}

#[tokio::test]
async fn list_workspaces_of_an_empty_registry_is_an_empty_list() {
    // No `~/.nexusflow/workspaces.toml` at all → an empty list, not an error (both seams).
    let home = TempDir::new().unwrap();
    let client = connect_home(home.path()).await;
    let tool = tool_sc(&client, "list_workspaces", json!({})).await;
    client.cancel().await.expect("clean shutdown");
    assert_eq!(tool["items"], json!([]), "empty registry → empty items");
    assert_eq!(
        mcp_workspaces_json(home.path()),
        json!([]),
        "and the CLI agrees"
    );
}

// ============================================================================================
// #65y — `nxs mcp install --setup`: register the server AND materialize the workspace it opens.
//
// These run the real `nxs mcp install --setup` under a controlled `HOME` (mirroring the c3i tests)
// and assert the init target matches the server's launch resolution: no `--workspace` → the board
// lands at the App-Data-Home and the entry stays flag-less; an explicit `--workspace` → the board
// lands there and the entry pins it. The final test drives the whole loop — install-setup, then the
// DEFAULT server serving that exact board through its config.
// ============================================================================================

#[test]
fn mcp_install_setup_materializes_a_personal_todo_board_at_the_app_data_home() {
    // #65y default tier (path a): `--setup` with NO `--workspace` materializes the board at the
    // App-Data-Home the server defaults to, and the host entry omits `--workspace` — so setup and the
    // server resolve the SAME board (never two).
    let home = TempDir::new().unwrap();
    let report = mcp_install_json(home.path(), &["--setup", "--host", "cursor"]);

    // The report's setup block names the seated plugin + the board root.
    assert_eq!(
        report["setup"]["plugin"], "personal-todo",
        "the install-setup default is personal-todo, not flow's issue-tracker default"
    );
    let root = report["setup"]["workspace"]
        .as_str()
        .expect("setup.workspace is a path");

    // The board is materialized there, seated with personal-todo (read from config.toml — the same
    // config the server reads, keeping it plugin-agnostic).
    let config = std::fs::read_to_string(Path::new(root).join(".nxs").join("config.toml"))
        .expect("config.toml at the App-Data-Home");
    assert!(
        config.contains("personal-todo"),
        "config seats personal-todo: {config}"
    );

    // The host entry omits `--workspace`; the server defaults to that same App-Data-Home.
    assert_eq!(report["workspace"], json!(null), "no --workspace pinned");
    let written: Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join(".cursor").join("mcp.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        written["mcpServers"]["nxs"]["args"],
        json!(["mcp", "serve"]),
        "flag-less entry so the server resolves its App-Data-Home default"
    );
}

#[test]
fn mcp_install_setup_with_explicit_workspace_pins_and_seats_the_coding_plugin() {
    // #65y explicit tier (path b): `--setup --workspace <path> --plugin issue-tracker` materializes
    // the board at <path> AND pins that same path in the entry — the coding case. Setup target ==
    // the pinned launch root, so the server opens exactly that board.
    let home = TempDir::new().unwrap();
    let proj = TempDir::new().unwrap();
    let proj_path = proj.path().to_str().unwrap();
    let report = mcp_install_json(
        home.path(),
        &[
            "--setup",
            "--workspace",
            proj_path,
            "--plugin",
            "issue-tracker",
            "--host",
            "cursor",
        ],
    );

    assert_eq!(report["setup"]["plugin"], "issue-tracker");
    assert_eq!(report["setup"]["workspace"], json!(proj_path));
    assert_eq!(
        report["workspace"],
        json!(proj_path),
        "the entry pins the same path the board was materialized at"
    );

    let config = std::fs::read_to_string(proj.path().join(".nxs").join("config.toml")).unwrap();
    assert!(
        config.contains("issue-tracker"),
        "config seats issue-tracker: {config}"
    );
    let written: Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join(".cursor").join("mcp.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        written["mcpServers"]["nxs"]["args"],
        json!(["mcp", "serve", "--workspace", proj_path]),
    );
}

#[tokio::test]
async fn mcp_install_setup_board_is_served_by_the_default_server() {
    // #65y acceptance: install --setup creates the board the DEFAULT `nxs mcp serve` (no --workspace)
    // opens, and the server renders/validates through THAT workspace's config — proven by creating a
    // `todo` at priority `soon`, both personal-todo-only vocabulary (issue-tracker would reject
    // them). This closes the init-target == server-resolution loop end to end.
    let home = TempDir::new().unwrap();
    let cwd = TempDir::new().unwrap();
    mcp_install_json(home.path(), &["--setup", "--host", "cursor"]);

    // The default server (no --workspace/--db) under the same HOME resolves the App-Data-Home the
    // install just seeded; `cwd` is a throwaway dir (with no `.nxs/`) to prove cwd is not discovered.
    let client = connect_default(home.path(), cwd.path()).await;
    let receipt = tool_sc(
        &client,
        "flow_create",
        json!({
            "type": "todo",
            "title": "Buy milk",
            "description": "the weekly shop",
            "priority": "soon",
            "now": NOW,
            "actor": "alice",
        }),
    )
    .await;
    client.cancel().await.expect("clean shutdown");

    assert_eq!(
        receipt["type"], "todo",
        "the default server serves the install-created personal-todo board"
    );
    assert_eq!(receipt["title"], "Buy milk");
}

#[test]
fn mcp_install_setup_absolutizes_a_relative_workspace_so_setup_and_entry_agree() {
    // #65y trap: a RELATIVE `--workspace` is absolutized so the materialized board and the pinned
    // entry are the SAME absolute path. A bare relative string would be re-resolved against the
    // host's own cwd at serve time, opening a different (empty) board than setup created.
    let home = TempDir::new().unwrap();
    let cwd = TempDir::new().unwrap();
    let out = StdCommand::new(cargo_bin("nxs"))
        .args([
            "mcp",
            "install",
            "--json",
            "--setup",
            "--workspace",
            "board", // relative — resolved against the install-process cwd below
            "--host",
            "cursor",
        ])
        .current_dir(cwd.path())
        .pin_home(home.path())
        .output()
        .expect("spawn nxs mcp install");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();

    // The reported setup path is ABSOLUTE and keeps the `board` tail (derived from the report to stay
    // robust to macOS `/var → /private/var` cwd normalization).
    let ws = report["setup"]["workspace"]
        .as_str()
        .expect("setup.workspace");
    assert!(
        Path::new(ws).is_absolute(),
        "relative --workspace absolutized: {ws}"
    );
    assert!(ws.ends_with("board"), "keeps the relative tail: {ws}");

    // The board exists at that absolute path, and BOTH the entry's pinned workspace + the args name
    // the same absolute path — never the bare relative `board`.
    assert!(
        Path::new(ws).join(".nxs").join("config.toml").is_file(),
        "board materialized at the absolute path {ws}"
    );
    assert_eq!(
        report["workspace"],
        json!(ws),
        "the entry pins the absolute path, not the relative `board`"
    );
    let written: Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join(".cursor").join("mcp.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        written["mcpServers"]["nxs"]["args"],
        json!(["mcp", "serve", "--workspace", ws]),
    );
}

#[test]
fn mcp_install_plugin_without_setup_still_materializes_the_board() {
    // #65y: `--plugin <id>` IMPLIES `--setup` — the changelog promises it, and the whole point of the
    // feature is that a `--plugin` never leaves a host entry pointing at a board that was never
    // created. Exercises the `plugin.is_some()` half of the setup gate WITHOUT `--setup`.
    let home = TempDir::new().unwrap();
    let report = mcp_install_json(
        home.path(),
        &["--plugin", "issue-tracker", "--host", "cursor"],
    );

    // Setup ran despite no `--setup`: the receipt carries the block and the board exists.
    assert_eq!(
        report["setup"]["plugin"], "issue-tracker",
        "a bare --plugin triggers setup"
    );
    let root = report["setup"]["workspace"]
        .as_str()
        .expect("setup.workspace");
    let config = std::fs::read_to_string(Path::new(root).join(".nxs").join("config.toml"))
        .expect("board materialized even though --setup was omitted");
    assert!(
        config.contains("issue-tracker"),
        "config seats the plugin: {config}"
    );
}

#[test]
fn mcp_install_setup_with_unknown_plugin_writes_no_host_entry() {
    // #65y ordering guarantee: setup (incl. plugin validation) runs BEFORE any host config is
    // written, so a bogus `--plugin` fails loudly and leaves NO half-registered host entry behind.
    let home = TempDir::new().unwrap();
    std::fs::create_dir_all(home.path().join(".cursor")).unwrap();
    let out = StdCommand::new(cargo_bin("nxs"))
        .args([
            "mcp", "install", "--json", "--setup", "--plugin", "bogus", "--host", "cursor",
        ])
        .pin_home(home.path())
        .output()
        .expect("spawn nxs mcp install");

    assert!(
        !out.status.success(),
        "an unknown --plugin is a loud failure"
    );
    let env: Value = serde_json::from_slice(&out.stdout).expect("error envelope on stdout");
    assert_eq!(env["error"]["kind"], "validation");
    assert!(
        !home.path().join(".cursor").join("mcp.json").exists(),
        "no host entry is written when setup fails first"
    );
}

#[test]
fn mcp_install_explicit_plugin_against_an_existing_board_warns_and_keeps_it() {
    // #65y (never-clobber + honest report): an explicit `--plugin` that disagrees with an already-
    // seated board is a no-op — setup never re-seats. The board + receipt keep the REAL plugin, and
    // the ignored request is surfaced on STDERR (not stdout, which stays the pure `--json` channel).
    let home = TempDir::new().unwrap();
    // Seed a personal-todo board at the App-Data-Home.
    mcp_install_json(home.path(), &["--setup", "--host", "cursor"]);

    // Now ask for issue-tracker against that existing personal-todo board.
    let out = StdCommand::new(cargo_bin("nxs"))
        .args([
            "mcp",
            "install",
            "--json",
            "--setup",
            "--plugin",
            "issue-tracker",
            "--host",
            "cursor",
        ])
        .pin_home(home.path())
        .output()
        .expect("spawn nxs mcp install");
    assert!(out.status.success());

    // The receipt reports the REAL (seated) plugin, not the ignored request.
    let report: Value = serde_json::from_slice(&out.stdout).expect("json on stdout");
    assert_eq!(
        report["setup"]["plugin"], "personal-todo",
        "the seated plugin is unchanged"
    );
    // The board on disk is still personal-todo.
    let root = report["setup"]["workspace"].as_str().unwrap();
    let config = std::fs::read_to_string(Path::new(root).join(".nxs").join("config.toml")).unwrap();
    assert!(
        config.contains("personal-todo"),
        "board never re-seated: {config}"
    );
    // The ignored `--plugin` is surfaced on stderr (stdout is clean JSON).
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ignoring") && stderr.contains("issue-tracker"),
        "the ignored --plugin is warned about on stderr: {stderr}"
    );
}

// ---- #ppa: per-tool workspace override (hybrid discovery) ---------------------------------------

#[tokio::test]
async fn per_tool_workspace_override_targets_a_second_board() {
    // S7/ppa: a server launched pinned to board A can serve board B on a single tool call by passing
    // `workspace`, with no registry. The override lists B; the same tool with no override still lists
    // the launch default (A) — so the override redirects and the default-only path stays green.
    let a = flow_fixture();
    let b = flow_fixture();
    nxf(
        a.path(),
        &[
            "create",
            "--type",
            "feature",
            "--title",
            "A-only",
            "--description",
            "d",
            "--priority",
            "P1",
        ],
    );
    nxf(
        b.path(),
        &[
            "create",
            "--type",
            "bug",
            "--title",
            "B-only",
            "--description",
            "d",
            "--priority",
            "P2",
        ],
    );

    let client = connect(a.path()).await; // launch-pinned to A

    // Override → B's board (compare to `nxf list --json` run IN B).
    let over = tool_sc(
        &client,
        "flow_list",
        json!({ "workspace": b.path().to_str().unwrap() }),
    )
    .await;
    assert_eq!(
        over["items"],
        without_presentation_labels(&nxf_json(b.path(), &["list"])),
        "the per-tool workspace override lists B's board, not the launch (A) board"
    );

    // No override → the launch default (A). Proves the default-only path is unchanged.
    let deflt = tool_sc(&client, "flow_list", json!({})).await;
    assert_eq!(
        deflt["items"],
        without_presentation_labels(&nxf_json(a.path(), &["list"])),
        "with no override the launch (A) board is served, exactly as before"
    );
    // Sanity: the two boards are genuinely different, so the override is doing real work.
    assert_ne!(over["items"], deflt["items"], "A and B are distinct boards");

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn per_tool_workspace_override_to_a_pathless_dir_is_a_no_workspace_domain_error() {
    // The override's `no_workspace` is a DOMAIN error (a tool result), NOT a server crash — distinct
    // from a missing launch default, which fails at `serve()` START. A path with no `.nxs/` returns
    // is_error + the closed `no_workspace` kind, and the server keeps serving the launch board.
    let a = flow_fixture();
    let empty = TempDir::new().unwrap(); // a real dir, but no `.nxs/` in it or its (temp) ancestry
    let client = connect(a.path()).await;

    let res = client
        .call_tool(call_with(
            "flow_list",
            json!({ "workspace": empty.path().to_str().unwrap() }),
        ))
        .await
        .expect("a bad override is a tool result, not a protocol error / crash");
    assert_eq!(
        res.is_error,
        Some(true),
        "an override to a dir without .nxs/ is a domain error"
    );
    let sc = res
        .structured_content
        .expect("the domain error carries a structured envelope");
    assert_eq!(
        sc["error"]["kind"], "no_workspace",
        "the override's missing-workspace maps to the closed `no_workspace` kind"
    );

    // The server survived the bad override: a no-override call still serves the launch board.
    let deflt = tool_sc(&client, "flow_list", json!({})).await;
    assert!(
        deflt["items"].is_array(),
        "the launch (A) board is still served after a bad override"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn per_tool_workspace_override_writes_land_in_the_target_board_only() {
    // S7/ppa cross-board ISOLATION for a WRITE: a mutation via the override must land in board B and
    // leave the launch board A untouched. The two read-only override tests above can't catch a leak
    // here — a regress would be silent cross-board corruption. This also pins actor + workspace
    // together on one write.
    let a = flow_fixture();
    let b = flow_fixture();
    let client = connect(a.path()).await; // launch-pinned to A; both boards start EMPTY

    // Create on B via the override, attributed to an explicit per-tool actor.
    let created = tool_sc(
        &client,
        "flow_create",
        json!({
            "type": "feature",
            "title": "B-only-write",
            "description": "d",
            "priority": "P1",
            "workspace": b.path().to_str().unwrap(),
            "actor": "alice",
            "now": NOW,
        }),
    )
    .await;
    let new_id = created["id"]
        .as_str()
        .expect("the new item's id")
        .to_string();

    // Present on board B — witnessed independently by the CLI reading B directly.
    let b_list = nxf_json(b.path(), &["list"]);
    assert!(
        b_list
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == new_id.as_str()),
        "the override write landed on board B: {b_list}"
    );
    // The launch board A is still EMPTY — the write did NOT leak into it (isolation guarantee).
    let a_list = tool_sc(&client, "flow_list", json!({})).await;
    assert!(
        a_list["items"].as_array().unwrap().is_empty(),
        "the launch board A stays empty; the override write did not leak into it: {a_list}"
    );
    // actor + workspace pinned together on the one write: the op is authored by alice in B, and no
    // alice-authored op exists in A.
    assert!(
        op_authors(b.path()).iter().any(|au| au == "alice"),
        "the write is attributed to the per-tool actor in B"
    );
    assert!(
        op_authors(a.path()).iter().all(|au| au != "alice"),
        "no alice-authored op leaked into the launch board A"
    );

    client.cancel().await.expect("clean shutdown");
}
