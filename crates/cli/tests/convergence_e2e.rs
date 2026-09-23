//! E4-T5 acceptance — the closing vertical cut of the structure-sync spine.
//!
//! Two replicas create and update issues **offline**, then sync through a single dumb
//! relay and converge to an identical materialized state with **no manual merge**. This
//! is the one claim E4 exists to make: the beads-style branch-switch-drift bug class is
//! structurally dead, not merely rarer. Prefixes are fixture-coordinated (Slice 1; `bab`
//! hardens cross-replica prefix distinctness in Slice 2).

mod common;

use assert_cmd::Command;
use common::spawn_relay;
use nxs_test_support::PinHome;
use std::path::Path;
use tempfile::TempDir;

fn nxf(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    c.current_dir(dir);
    c
}

/// The umbrella binary — the sync verb moved here in aye.2.1 (`nxs sync bind/run`). `bind` now
/// upserts the workspace into `~/.nexusflow/workspaces.toml` (kgn5) on every success — route
/// `HOME` at a dir-local sandbox, through [`PinHome`], so this test never touches the developer's
/// real registry. That helper carries the variables that would out-vote a pinned home (the `XDG_*`
/// overrides, and the service instance this repo's `.envrc` names) rather than leaving each of
/// them to a line here.
fn nxs(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxs");
    c.current_dir(dir).pin_home(dir.join(".fake-home"));
    c
}

/// Init a workspace, then pin its replica identity to a known `(site, prefix)` so ids are
/// deterministically distinct across replicas (the §9 "fixtures-coordinated prefixes").
fn init_replica(dir: &Path, site: i64, prefix: &str) {
    nxf(dir)
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    std::fs::write(
        dir.join(".nxs").join("replica.toml"),
        format!("site_id = {site}\nprefix = \"{prefix}\"\n"),
    )
    .unwrap();
}

fn run_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    // The sync verb moved to the umbrella (aye.2.1): route `sync …` to `nxs`, the rest to `nxf`.
    let mut cmd = if args.first() == Some(&"sync") {
        nxs(dir)
    } else {
        nxf(dir)
    };
    let out = cmd
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("json output")
}

/// Create a task with a fixture id `{prefix}.{suffix}`... but the CLI mints the id, so we
/// create then return the minted id.
fn create_task(dir: &Path, title: &str) -> String {
    run_json(
        dir,
        &[
            "create",
            "--type",
            "bug",
            "--title",
            title,
            "--description",
            "d",
            "--priority",
            "P1",
            "--json",
        ],
    )["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The full materialized item list as the canonical, deterministic `--json` record set.
fn list(dir: &Path) -> serde_json::Value {
    run_json(dir, &["list", "--json"])
}

/// Drive both replicas to a fixed point: two rounds of (A then B) drains a star relay.
fn converge(a: &Path, b: &Path, relay: &str) {
    for _ in 0..2 {
        run_json(a, &["sync", "run", "--remote", relay, "--json"]);
        run_json(b, &["sync", "run", "--remote", relay, "--json"]);
    }
}

#[test]
fn two_replicas_converge_on_distinct_offline_creates() {
    let relay = spawn_relay();
    let a = TempDir::new().unwrap();
    let b = TempDir::new().unwrap();
    init_replica(a.path(), 111, "aaaa");
    init_replica(b.path(), 222, "bbbb");

    // Each works OFFLINE first.
    let a_id = create_task(a.path(), "Task on A");
    let b_id = create_task(b.path(), "Task on B");
    assert!(a_id.starts_with("aaaa."));
    assert!(b_id.starts_with("bbbb."));

    // Before sync the replicas disagree (each sees only its own task).
    assert_ne!(list(a.path()), list(b.path()));

    // Bind to one shared stream (A creates, B joins out-of-band), then sync.
    let stream = run_json(
        a.path(),
        &["sync", "bind", "--no-daemon", "--create", "--json"],
    )["stream_id"]
        .as_str()
        .unwrap()
        .to_string();
    run_json(
        b.path(),
        &["sync", "bind", "--no-daemon", "--join", &stream, "--json"],
    );
    converge(a.path(), b.path(), &relay);

    // Identical materialized state on both, no manual merge — both tasks on both sides.
    let la = list(a.path());
    assert_eq!(la, list(b.path()), "replicas converge");
    let ids: Vec<&str> = la
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [a_id.as_str(), b_id.as_str()],
        "both tasks present, ordered by id"
    );
}

#[test]
fn concurrent_offline_edits_to_a_shared_item_converge() {
    let relay = spawn_relay();
    let a = TempDir::new().unwrap();
    let b = TempDir::new().unwrap();
    init_replica(a.path(), 111, "aaaa");
    init_replica(b.path(), 222, "bbbb");

    // A creates a task and both replicas learn it.
    let id = create_task(a.path(), "Shared");
    let stream = run_json(
        a.path(),
        &["sync", "bind", "--no-daemon", "--create", "--json"],
    )["stream_id"]
        .as_str()
        .unwrap()
        .to_string();
    run_json(
        b.path(),
        &["sync", "bind", "--no-daemon", "--join", &stream, "--json"],
    );
    converge(a.path(), b.path(), &relay);
    assert_eq!(list(a.path()), list(b.path()), "shared item replicated");

    // Now diverge OFFLINE on different fields of the SAME item: A closes it, B retitles it.
    run_json(a.path(), &["close", &id, "--reason", "done on A", "--json"]);
    run_json(
        b.path(),
        &["update", &id, "--set", "title=Renamed on B", "--json"],
    );

    // Reconnect and sync → both fields merge, no manual resolution.
    converge(a.path(), b.path(), &relay);
    assert_eq!(list(a.path()), list(b.path()), "concurrent edits converge");

    let item = run_json(a.path(), &["show", &id, "--json"])["item"].clone();
    assert_eq!(item["status"], "closed", "A's close survives the merge");
    assert_eq!(
        item["title"], "Renamed on B",
        "B's retitle survives the merge"
    );
    assert_eq!(item["closing_comment"], "done on A");
}
