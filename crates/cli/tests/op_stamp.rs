//! The unpinned production stamping path. The parity differential (`parity.rs`) pins `now` and
//! `actor` on both seams for determinism, which deliberately bypasses the CLI's fallback branch:
//! when `NXF_NOW`/`NXF_ACTOR` are unset, `resolve_now` falls back to the system clock and `actor`
//! to `$USER`/`"nxf"`. This guards that fallback — a real, non-empty `wall_clock` and `author`
//! land on every emitted op, never the blank default (Test Quality review #4).

use assert_cmd::Command;
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use std::path::Path;
use tempfile::TempDir;

/// An `nxf` invocation with the determinism knobs explicitly removed, so the test exercises the
/// fallback branch regardless of the runner's environment.
fn nxf(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    c.current_dir(dir)
        .env_remove("NXF_NOW")
        .env_remove("NXF_ACTOR")
        .env_remove("NXF_DETERMINISTIC_IDS");
    c
}

#[test]
fn unpinned_writes_stamp_a_real_wall_clock_and_actor() {
    let tmp = TempDir::new().unwrap();
    nxf(tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    nxf(tmp.path())
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
        .assert()
        .success();

    let ops = workspace::discover(tmp.path())
        .unwrap()
        .open_store()
        .unwrap()
        .export();
    assert!(!ops.is_empty(), "the create emitted ops");
    // The actor fell back to $USER / "nxf" — non-empty either way, never blank.
    assert!(
        ops.iter().all(|o| !o.author.is_empty()),
        "every op carries a non-empty fallback actor"
    );
    // `now` fell back to the system clock: a real RFC3339 UTC timestamp, never the blank default
    // the column carries when no source is wired.
    assert!(
        ops.iter()
            .all(|o| o.wall_clock.contains('T') && o.wall_clock.ends_with('Z')),
        "every op carries a real fallback wall_clock, got {:?}",
        ops.first().map(|o| o.wall_clock.clone())
    );
}

#[test]
fn a_set_but_empty_actor_env_never_authors_an_op() {
    // Review finding Test Quality #1 (PR #311). `std::env::var` reports a variable that is SET BUT
    // EMPTY as `Ok("")`, so the CLI's `actor()` chain took it literally and authored every op of
    // that session with no identity at all — on a log where attribution cannot be backfilled.
    // The rule (`nxs_foundation::model::resolve_author`: blank is absent, not an identity) is unit-
    // tested; this pins the WIRING through the real binary, which is where it was wrong before.
    let tmp = TempDir::new().unwrap();
    nxf(tmp.path())
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    // Both tiers set-but-blank: `NXF_ACTOR` empty, `USER` whitespace-only. Nothing is left to
    // resolve to except the binary's own name.
    nxs_test_support::cargo_bin("nxf")
        .current_dir(tmp.path())
        .env_remove("NXF_NOW")
        .env_remove("NXF_DETERMINISTIC_IDS")
        .env("NXF_ACTOR", "")
        .env("USER", "   ")
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
        .assert()
        .success();

    let ops = workspace::discover(tmp.path())
        .unwrap()
        .open_store()
        .unwrap()
        .export();
    assert!(!ops.is_empty(), "the create emitted ops");
    assert!(
        ops.iter().all(|o| !o.author.trim().is_empty()),
        "a blank *_ACTOR/USER must fall through, never author an op: {:?}",
        ops.iter().map(|o| o.author.clone()).collect::<Vec<_>>()
    );
    assert!(
        ops.iter().all(|o| o.author == "nxf"),
        "with every tier blank the binary's own name is the identity of record, got {:?}",
        ops.first().map(|o| o.author.clone())
    );
}
