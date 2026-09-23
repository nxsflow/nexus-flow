//! The unpinned actor path for `nxm`, black-box over the built binary — the memory-side twin of
//! `crates/cli/tests/op_stamp.rs` (review finding Test Quality #1, PR #311).
//!
//! `std::env::var` reports a variable that is SET BUT EMPTY as `Ok("")`, so the `actor()` chain
//! took it literally and authored every op of that session with no identity at all. On an
//! append-only log that is unrepairable — attribution cannot be backfilled. The rule itself
//! (`nxs_foundation::model::resolve_author`: blank is absent, not an identity) is unit-tested;
//! this pins the WIRING at `nxm`'s own call site, which is where it was wrong.

use assert_cmd::Command;
use tempfile::TempDir;

/// An `nxm` invocation with a set-but-BLANK actor at both tiers, and the determinism knobs
/// removed so the fallback branch runs regardless of the runner's environment.
fn nxm_blank_actor(dir: &std::path::Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxm");
    c.current_dir(dir)
        .env_remove("NXM_NOW")
        .env("NXM_ACTOR", "")
        .env("USER", "   ");
    c
}

fn authors(dir: &std::path::Path) -> Vec<String> {
    let conn = rusqlite::Connection::open(dir.join(".nxs/db.sqlite")).unwrap();
    let mut stmt = conn.prepare("SELECT author FROM ops").unwrap();
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows
}

#[test]
fn a_set_but_empty_actor_env_never_authors_an_op() {
    let tmp = TempDir::new().unwrap();
    nxm_blank_actor(tmp.path())
        .args(["--json", "init"])
        .assert()
        .success();
    nxm_blank_actor(tmp.path())
        .args([
            "--json",
            "remember",
            "--key",
            "k",
            "the fact",
            "--introduction",
            "in one line",
        ])
        .assert()
        .success();

    let authors = authors(tmp.path());
    assert!(!authors.is_empty(), "the remember emitted ops");
    assert!(
        authors.iter().all(|a| !a.trim().is_empty()),
        "a blank NXM_ACTOR/USER must fall through, never author an op: {authors:?}"
    );
    assert!(
        authors.iter().all(|a| a == "nxm"),
        "with every tier blank the binary's own name is the identity of record, got {authors:?}"
    );
}
