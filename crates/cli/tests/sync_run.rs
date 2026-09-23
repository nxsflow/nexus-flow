//! `nxs sync run` — one client-driven push/pull pass against a live relay (E4 T4). The verb moved
//! from `nxf` to the umbrella in aye.2.1; items are still created via `nxf`, then synced via `nxs`.

mod common;

use assert_cmd::Command;
use common::{spawn_relay, spawn_relay_pull_fails};
use nxs_test_support::PinHome;
use std::path::Path;
use tempfile::TempDir;

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

/// The umbrella binary. `bind` now upserts the workspace into `~/.nexusflow/workspaces.toml`
/// (kgn5) on every success — route `HOME` at a dir-local sandbox, through [`PinHome`], so these
/// tests never touch the developer's real registry. That helper carries the variables that would
/// out-vote a pinned home (the `XDG_*` overrides, and the service instance this repo's `.envrc`
/// names) rather than leaving each of them to a line here.
fn nxs(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxs");
    c.current_dir(dir).pin_home(dir.join(".fake-home"));
    c
}

/// Run an `nxs` sync subcommand and parse its `--json`.
fn sync_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let out = nxs(dir)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("json output")
}

fn init_with_task(dir: &Path) {
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(dir)
        .assert()
        .success();
    nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            "T",
            "--description",
            "d",
            "--priority",
            "P1",
            "--json",
        ])
        .current_dir(dir)
        .assert()
        .success();
}

#[test]
fn sync_run_pushes_local_ops_then_is_idempotent() {
    let relay = spawn_relay();
    let tmp = TempDir::new().unwrap();
    init_with_task(tmp.path());
    sync_json(
        tmp.path(),
        &["sync", "bind", "--no-daemon", "--create", "--json"],
    );

    // First pass pushes the 5 create ops (type/title/status + description/priority).
    let first = sync_json(tmp.path(), &["sync", "run", "--remote", &relay, "--json"]);
    assert_eq!(first["pushed"], 5);

    // Second pass has nothing new to push (watermark advanced, persisted).
    let second = sync_json(tmp.path(), &["sync", "run", "--remote", &relay, "--json"]);
    assert_eq!(second["pushed"], 0, "re-sync pushes nothing new");
}

#[test]
fn a_mid_pass_failure_still_persists_the_push_watermark() {
    // The engine advances pushed_through per durable batch; this proves the CLI persists that
    // progress even when the pass ERRORS. Push succeeds, then pull fails → `sync run` exits
    // non-zero. The push watermark must have been saved, so a later healthy sync re-pushes
    // NOTHING. Without persist-on-error the watermark would be lost and all ops re-pushed.
    let failing = spawn_relay_pull_fails();
    let tmp = TempDir::new().unwrap();
    init_with_task(tmp.path());
    sync_json(
        tmp.path(),
        &["sync", "bind", "--no-daemon", "--create", "--json"],
    );

    // First pass: the 5 create ops push fine, but the pull half is injected to fail.
    let out = nxs(tmp.path())
        .args(["sync", "run", "--remote", &failing, "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        v["error"]["kind"], "io",
        "the pass surfaced the pull failure"
    );

    // A second pass against a HEALTHY relay must push nothing: the push watermark survived the
    // first pass's error. (The two relays are independent, so a re-push would surface as pushed>0.)
    let healthy = spawn_relay();
    let second = sync_json(tmp.path(), &["sync", "run", "--remote", &healthy, "--json"]);
    assert_eq!(
        second["pushed"], 0,
        "push watermark persisted across the failed pass — no re-push"
    );
}

#[test]
fn sync_run_without_binding_is_rejected() {
    let relay = spawn_relay();
    let tmp = TempDir::new().unwrap();
    init_with_task(tmp.path());
    // No `sync bind` → run must fail loudly rather than implicitly binding.
    let out = nxs(tmp.path())
        .args(["sync", "run", "--remote", &relay, "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["kind"], "validation");
}

/// Test Quality #1 (PR #263): the spec calls optional `--remote` the key compatibility change,
/// and `endpoint::resolve`'s precedence is unit-tested — but nothing drove `nxs sync run` through
/// the actual CLI seam WITHOUT `--remote`. Bind with a per-workspace `--endpoint`, then run
/// with no `--remote` at all: the pass must resolve that endpoint on its own and actually reach
/// it (a real push into a real, spawned relay — `HOME` isolated via the `nxs()` helper, daemon
/// autostart skipped via `--no-daemon`, exactly like every other test in this file).
#[test]
fn sync_run_without_remote_resolves_and_reaches_the_bound_endpoint() {
    let relay = spawn_relay();
    let tmp = TempDir::new().unwrap();
    init_with_task(tmp.path());
    sync_json(
        tmp.path(),
        &[
            "sync",
            "bind",
            "--no-daemon",
            "--create",
            "--endpoint",
            &relay,
            "--json",
        ],
    );

    // No `--remote` at all — the endpoint must come from the workspace's own bound config.
    let out = sync_json(tmp.path(), &["sync", "run", "--json"]);
    assert_eq!(
        out["pushed"], 5,
        "the bound --endpoint was resolved and actually reached"
    );

    // A second run with nothing new to push proves this isn't a fluke of the first push: the
    // watermark advanced against the SAME resolved endpoint.
    let second = sync_json(tmp.path(), &["sync", "run", "--json"]);
    assert_eq!(
        second["pushed"], 0,
        "re-sync against the same resolved endpoint pushes nothing new"
    );
}

/// Test Quality #2 (PR #263): `nxs sync endpoint <url>` (set) and `nxs sync endpoint` (show) had
/// no CLI-level test — only the underlying `save_global_to`/`load_global_from` functions were
/// unit-tested. `HOME` isolated via the `nxs()` helper, same as every other test in this file.
#[test]
fn sync_endpoint_set_then_show_round_trips_at_the_cli_seam() {
    let tmp = TempDir::new().unwrap();

    // No default set yet.
    let unset = sync_json(tmp.path(), &["sync", "endpoint", "--json"]);
    assert_eq!(unset["default_endpoint"], serde_json::Value::Null);

    let set = sync_json(
        tmp.path(),
        &["sync", "endpoint", "https://relay.example", "--json"],
    );
    assert_eq!(set["default_endpoint"], "https://relay.example");

    // A later, flag-less invocation shows what was just set, not just this call's own echo.
    let shown = sync_json(tmp.path(), &["sync", "endpoint", "--json"]);
    assert_eq!(shown["default_endpoint"], "https://relay.example");
}
