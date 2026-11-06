//! One test in a binary of its own, because it exports `GIT_DIR` for the whole process: a service started from a git
//! hook, or a terminal with the variable set, must not point the tree at another repository.

mod common;

use common::tree::*;
use nexus_run_contracts::{ParkOutcome, RunId};
use nexus_run_host::TreeResult;

#[test]
fn an_exported_git_dir_is_ignored() {
    let bench = Bench::new();
    let other = bench.origin.with_file_name("other");
    origin_with_main(&other);
    let other_head = run_git(&other, &["rev-parse", "HEAD"]);
    let other_git = other.join(".git");
    let tree = bench.prepared();

    // From here on every git the process starts inherits these, unless it removes them.
    std::env::set_var("GIT_DIR", &other_git);
    std::env::set_var("GIT_WORK_TREE", &other);
    std::env::set_var("GIT_INDEX_FILE", other_git.join("index"));

    let run = RunId::new(ulid::Ulid::new());
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    let parked = bench.park(&tree, &run);
    let found = bench.prepare(&tree, &RunId::new(ulid::Ulid::new()));
    let state = nexus_run_host::git_tree::tree_state(&bench.root, &bench.dir);

    std::env::remove_var("GIT_DIR");
    std::env::remove_var("GIT_WORK_TREE");
    std::env::remove_var("GIT_INDEX_FILE");

    assert!(
        matches!(parked, TreeResult::Parked(ParkOutcome::Committed { .. })),
        "{parked:?}"
    );
    assert!(
        matches!(found, TreeResult::Prepared { found: None, .. }),
        "{found:?}"
    );
    assert_eq!(
        state.unwrap().commit.as_deref(),
        Some(bench.head().as_str())
    );
    assert_eq!(
        run_git(&other, &["rev-parse", "HEAD"]),
        other_head,
        "the other repository moved"
    );
    assert_eq!(
        run_git(&other, &["status", "--porcelain"]),
        "",
        "the other repository was written to"
    );
    assert_eq!(run_git(&other, &["for-each-ref", "refs/heads/nxs/"]), "");
}
