//! C2 (#916.2): the ordering axis through the CLI seam — every read verb ranks by default, with
//! a `--sort` override and a loud error on an unknown key. Priorities are chosen so rank order
//! (priority asc) differs from id order, which is what makes the default observable.

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

/// Deterministic ids (`ab12.0001`, `0002`, …) so creation order == id order, which is what makes
/// the `--sort id` expectations predictable; `NXF_NOW` pins the clock for the same reason.
fn nxf() -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    c.env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXF_NOW", "2026-06-23T00:00:00Z");
    c
}

fn init(dir: &Path) {
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(dir)
        .assert()
        .success();
}

/// Create a task with the given priority; returns its id. Lower P = higher priority.
fn create(dir: &Path, title: &str, priority: &str) -> String {
    let out = nxf()
        .args([
            "create",
            "--type",
            "bug",
            "--title",
            title,
            "--description",
            "d",
            "--priority",
            priority,
            "--json",
        ])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    v["id"].as_str().unwrap().to_string()
}

fn dep(dir: &Path, from: &str, to: &str) {
    nxf()
        .args(["dep", "add", from, to])
        .current_dir(dir)
        .assert()
        .success();
}

fn ids(dir: &Path, args: &[&str]) -> Vec<String> {
    let mut full: Vec<&str> = args.to_vec();
    full.push("--json");
    let out = nxf()
        .args(full)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn list_defaults_to_id_and_next_to_rank_with_overrides() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let low = create(tmp.path(), "low", "P2"); // ab12.0001
    let high = create(tmp.path(), "high", "P0"); // ab12.0002
    let mid = create(tmp.path(), "mid", "P1"); // ab12.0003
    let by_id = vec![low.clone(), high.clone(), mid.clone()]; // 0001, 0002, 0003
    let by_rank = vec![high.clone(), mid.clone(), low.clone()]; // P0, P1, P2

    // list defaults to id (plugin-independent); next defaults to rank.
    assert_eq!(ids(tmp.path(), &["list"]), by_id, "list defaults to id");
    assert_eq!(ids(tmp.path(), &["next"]), by_rank, "next defaults to rank");

    // The override flips each to the other order.
    assert_eq!(
        ids(tmp.path(), &["list", "--sort", "rank"]),
        by_rank,
        "list --sort rank"
    );
    assert_eq!(
        ids(tmp.path(), &["next", "--sort", "id"]),
        by_id,
        "next --sort id"
    );
}

#[test]
fn blocked_ranks_by_default_and_sort_id_overrides() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let low = create(tmp.path(), "low blocked", "P2"); // ab12.0001
    let high = create(tmp.path(), "high blocked", "P0"); // ab12.0002
    let blocker = create(tmp.path(), "blocker", "P0"); // ab12.0003
    dep(tmp.path(), &low, &blocker); // low depends on blocker
    dep(tmp.path(), &high, &blocker); // high depends on blocker

    // Default = rank: the higher-priority blocked item first (the C2 fix; was id before).
    assert_eq!(
        ids(tmp.path(), &["blocked"]),
        vec![high.clone(), low.clone()],
        "blocked defaults to rank, not id"
    );
    // --sort id overrides.
    assert_eq!(
        ids(tmp.path(), &["blocked", "--sort", "id"]),
        vec![low, high],
        "blocked --sort id"
    );
}

#[test]
fn next_always_includes_claimed_work_and_the_flag_is_gone() {
    // The finish-first tiers removed `--include-in-progress`: claimed work is always in `next`,
    // ranked first (Tier 1 — a claimed, childless item is finishable). The flag itself is no longer
    // a recognized argument, so an old script using it fails loudly rather than silently changing
    // meaning.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let ready = create(tmp.path(), "ready item", "P1"); // ab12.0001, stays open
    let claimed = create(tmp.path(), "claimed item", "P1"); // ab12.0002
    nxf()
        .args(["claim", &claimed])
        .current_dir(tmp.path())
        .assert()
        .success();

    assert_eq!(
        ids(tmp.path(), &["next"]),
        vec![claimed, ready],
        "claimed work is present by default, ranked first"
    );
    nxf()
        .args(["next", "--include-in-progress"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("--include-in-progress"));
}

#[test]
fn the_removed_ready_verb_is_no_longer_a_subcommand() {
    // 0fc: `ready` was fully removed (folded into `next` since C4 #916.3). The migration stub is
    // gone too, so `nxf ready` now fails as clap's generic unrecognized subcommand — no alias.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    nxf()
        .args(["ready"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("unrecognized subcommand"));
}

#[test]
fn next_and_prime_show_a_resolved_parent_for_a_child() {
    // #916.7: a child's parent (belongs_to) surfaces resolved in `next` and prime — JSON `parent`
    // object + a `↳` follow-up line in both human views; a top-level item has neither.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    // ab12.0001 project, ab12.0002 child of it.
    nxf()
        .args([
            "create",
            "--type",
            "epic",
            "--title",
            "Ship v1",
            "--description",
            "d",
            "--priority",
            "P1",
            "--json",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();
    let child = create(tmp.path(), "child", "P1"); // ab12.0002
    nxf()
        .args(["update", &child, "--set", "parent=ab12.0001"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // next --json: the child carries a resolved parent; the project (top-level) carries null.
    let out = nxf()
        .args(["next", "--json"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let rec = |id: &str| {
        v.as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(rec(&child)["parent"]["id"], "ab12.0001");
    assert_eq!(rec(&child)["parent"]["title"], "Ship v1");
    assert_eq!(rec(&child)["parent"]["type"], "epic");
    assert_eq!(rec("ab12.0001")["parent"], serde_json::Value::Null);

    // next human: the child gets an indented `↳ <parent-id> · <title>` line; the local parent id
    // is shown by its bare suffix (ykv).
    nxf()
        .args(["next"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicates::str::contains("↳ 0001 · Ship v1"));

    // prime human: the same parent sub-line appears in the embedded next block.
    nxf()
        .args(["prime"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicates::str::contains("↳ `0001` · Ship v1"));
}

#[test]
fn an_unknown_sort_key_is_a_loud_error() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    create(tmp.path(), "t", "P1");
    nxf()
        .args(["list", "--sort", "bogus"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("unknown sort key"));
}
