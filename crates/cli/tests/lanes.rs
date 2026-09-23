//! C3 (#916.4): the lane verbs `deferred`/`closed`/`archived` through the CLI seam, plus the
//! partition guarantee end-to-end. Each lane carries its natural in-lane order; `--sort` overrides
//! it. Archiving has its own verb in C5 — here the `archived` cell is set directly on the store,
//! the same approach the C1 `archived.rs` suite uses.

use assert_cmd::Command;
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use std::path::Path;
use tempfile::TempDir;

/// Deterministic ids + a pinned clock so creation order == id order and defer reasoning is stable.
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

/// Create an open task; returns its id.
fn create(dir: &Path, title: &str) -> String {
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
            "P1",
            "--json",
        ])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice::<serde_json::Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Create a deferred task (open, future defer date); returns its id.
fn create_deferred(dir: &Path, title: &str, defer: &str) -> String {
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
            "P1",
            "--defer",
            defer,
            "--json",
        ])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice::<serde_json::Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn close(dir: &Path, id: &str) {
    nxf()
        .args(["close", id, "--reason", "done"])
        .current_dir(dir)
        .assert()
        .success();
}

/// Set the `archived` cell directly (the `archive` verb is C5).
fn archive_cell(dir: &Path, id: &str, when: &str) {
    let mut store = workspace::discover(dir).unwrap().open_store().unwrap();
    store.set_field(id, "archived", Some(when.to_string()), "test");
}

/// Ids returned by a read verb, in the order the verb emits them (NOT re-sorted).
fn ids(dir: &Path, args: &[&str]) -> Vec<String> {
    ids_where(dir, args, |_| true)
}

/// Ids returned by a read verb, keeping only the records matching `keep`.
fn ids_where(dir: &Path, args: &[&str], keep: impl Fn(&serde_json::Value) -> bool) -> Vec<String> {
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
    serde_json::from_slice::<serde_json::Value>(&out)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| keep(i))
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

/// The **ready lane**: the `open` rows of `next`.
///
/// `next` is no longer a lane reader — since the finish-first tiers it recommends `ready ∪
/// in_progress` (docs/specs/next-finish-first-tiers.md §2), so reading the ready LANE from it
/// requires dropping the claimed rows. The two sets are disjoint by status (one `open`, one
/// `in_progress`), so filtering on `status` recovers `ready` exactly. This mirrors how the
/// `in_progress` lane is read (`list --status in_progress`): neither lane has a verb of its own.
fn ready_lane(dir: &Path) -> Vec<String> {
    ids_where(dir, &["next"], |i| i["status"] == "open")
}

#[test]
fn deferred_lists_future_deferred_work_in_defer_date_order() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let ready = create(tmp.path(), "ready now");
    let later = create_deferred(tmp.path(), "later", "2026-12-01T00:00:00Z");
    let sooner = create_deferred(tmp.path(), "sooner", "2026-09-01T00:00:00Z");

    // Default order is defer-date ascending — soonest first — and the non-deferred item is absent.
    let deferred = ids(tmp.path(), &["deferred"]);
    assert_eq!(deferred, vec![sooner.clone(), later.clone()]);
    assert!(!deferred.contains(&ready), "a ready item is not deferred");

    // `--sort id` overrides the defer-date default.
    assert_eq!(
        ids(tmp.path(), &["deferred", "--sort", "id"]),
        vec![later, sooner.clone()]
    );

    // Once `--now` passes the soonest defer date, that item drops out of the deferred lane.
    let after_sooner = ids(tmp.path(), &["deferred", "--now", "2026-10-01T00:00:00Z"]);
    assert!(!after_sooner.contains(&sooner));
}

#[test]
fn closed_lists_closed_work_newest_first_and_excludes_archived() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = create(tmp.path(), "first closed");
    let b = create(tmp.path(), "second closed");
    let c = create(tmp.path(), "closed then archived");
    close(tmp.path(), &a);
    close(tmp.path(), &b);
    close(tmp.path(), &c);
    // All three closed at the pinned NXF_NOW, so close-recency ties break by id (deterministic).
    archive_cell(tmp.path(), &c, "2026-06-24T00:00:00Z");

    let closed = ids(tmp.path(), &["closed"]);
    assert!(
        closed.contains(&a) && closed.contains(&b),
        "closed lane lists closed items"
    );
    assert!(
        !closed.contains(&c),
        "an archived item leaves the closed lane (archived precedence)"
    );
}

#[test]
fn archived_lists_archived_work_any_status() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let open_archived = create(tmp.path(), "open then archived");
    let closed_archived = create(tmp.path(), "closed then archived");
    close(tmp.path(), &closed_archived);
    archive_cell(tmp.path(), &open_archived, "2026-06-20T00:00:00Z");
    archive_cell(tmp.path(), &closed_archived, "2026-06-25T00:00:00Z");

    // Archive-date descending: the more recently archived item first, regardless of status.
    let archived = ids(tmp.path(), &["archived"]);
    assert_eq!(archived, vec![closed_archived, open_archived]);
}

#[test]
fn lanes_partition_the_board_through_the_cli() {
    // The partition guarantee end-to-end: every live item the CLI can produce lands in exactly
    // one lane. (`in_progress` has no verb — it is read via `list --status in_progress`.)
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let ready = create(tmp.path(), "ready");
    let blocker = create(tmp.path(), "blocker"); // also ready
    let blocked = create(tmp.path(), "blocked");
    nxf()
        .args(["dep", "add", &blocked, &blocker])
        .current_dir(tmp.path())
        .assert()
        .success();
    let deferred = create_deferred(tmp.path(), "deferred", "2026-12-01T00:00:00Z");
    let claimed = create(tmp.path(), "claimed"); // claimed + actionable → in_progress lane
    nxf()
        .args(["claim", &claimed])
        .current_dir(tmp.path())
        .assert()
        .success();
    // The hard cases the facade test pins, mirrored end-to-end: an item that is claimed BUT then
    // blocked, and one claimed BUT deferred, are NON-actionable in_progress — `next`/`blocked`/
    // `deferred` all skip them, so they live only in the raw `list --status in_progress` lane.
    let claimed_blocked = create(tmp.path(), "claimed-then-blocked");
    nxf()
        .args(["claim", &claimed_blocked])
        .current_dir(tmp.path())
        .assert()
        .success();
    nxf()
        .args(["dep", "add", &claimed_blocked, &blocker])
        .current_dir(tmp.path())
        .assert()
        .success();
    let claimed_deferred =
        create_deferred(tmp.path(), "claimed-then-deferred", "2026-12-01T00:00:00Z");
    nxf()
        .args(["claim", &claimed_deferred])
        .current_dir(tmp.path())
        .assert()
        .success();
    let closed = create(tmp.path(), "closed");
    close(tmp.path(), &closed);
    let archived = create(tmp.path(), "archived");
    archive_cell(tmp.path(), &archived, "2026-06-22T00:00:00Z");

    let next = ready_lane(tmp.path()); // the ready lane = the open rows of `next`
    let blocked_lane = ids(tmp.path(), &["blocked"]);
    let deferred_lane = ids(tmp.path(), &["deferred"]);
    let in_progress = ids(tmp.path(), &["list", "--status", "in_progress"]);
    let closed_lane = ids(tmp.path(), &["closed"]);
    let archived_lane = ids(tmp.path(), &["archived"]);

    let mut union: Vec<String> = [
        next.clone(),
        blocked_lane.clone(),
        deferred_lane.clone(),
        in_progress.clone(),
        closed_lane.clone(),
        archived_lane.clone(),
    ]
    .concat();
    union.sort();
    let before = union.len();
    union.dedup();
    assert_eq!(
        before,
        union.len(),
        "lanes are disjoint (no item in two lanes)"
    );

    // The non-actionable claimed items are reachable ONLY via the raw in_progress lane — never
    // dropped from the partition (the substance of review Finding #1).
    assert!(
        in_progress.contains(&claimed_blocked),
        "claimed-then-blocked stays in the in_progress lane"
    );
    assert!(
        in_progress.contains(&claimed_deferred),
        "claimed-then-deferred stays in the in_progress lane"
    );
    assert!(!next.contains(&claimed_blocked) && !blocked_lane.contains(&claimed_blocked));
    assert!(!next.contains(&claimed_deferred) && !deferred_lane.contains(&claimed_deferred));

    let all = [
        &ready,
        &blocker,
        &blocked,
        &deferred,
        &claimed,
        &claimed_blocked,
        &claimed_deferred,
        &closed,
        &archived,
    ];
    for id in all {
        assert!(union.contains(id), "{id} must appear in exactly one lane");
    }
    assert_eq!(
        union.len(),
        all.len(),
        "all live items are partitioned across the lanes"
    );
}
