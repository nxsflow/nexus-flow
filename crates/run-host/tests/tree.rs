//! The tree tests of §12.11, over the real `GitTree` against temporary repositories with a file-path origin.

mod common;

use common::tree::*;
use nexus_run_contracts::{GitReason, ParkOutcome, RunId, SequenceKind, TreeRefusal};
use nexus_run_host::git_tree::{tree_state, GitBounds};
use nexus_run_host::{TreeResult, WorkTree};

/// The finisher's clean-tree check (§6.6): a clone has no self-ignoring `.nxf/.gitignore`, so the verifier's gate log
/// and a session's local settings have to be ignored by the file the clone carries, not by this machine's excludes.
#[test]
fn finisher_clean_check_passes_in_a_fresh_clone_with_a_gate_log() {
    let bench = Bench::new();
    let base = bench.origin.parent().expect("a parent");
    run_git(base, &["clone", "-q", "origin", "clone"]);
    let clone = base.join("clone");
    std::fs::create_dir_all(clone.join(".nxf/review")).expect("the gate log's directory");
    std::fs::write(clone.join(".nxf/review/gates-x.txt"), b"green").expect("a gate log");
    std::fs::create_dir_all(clone.join(".claude")).expect("the settings' directory");
    std::fs::write(clone.join(".claude/settings.local.json"), b"{}").expect("local settings");

    assert_eq!(
        run_git(&clone, &["status", "--porcelain"]),
        "",
        "the tree is clean"
    );
    for ignored in [".nxf/review/gates-x.txt", ".claude/settings.local.json"] {
        let answer = run_git(&clone, &["check-ignore", "-v", ignored]);
        assert!(
            answer.starts_with(".gitignore:"),
            "{ignored} by: {answer:?}"
        );
    }
}

fn fresh_run() -> RunId {
    RunId::new(ulid::Ulid::new())
}

fn park_outcome(result: TreeResult) -> ParkOutcome {
    match result {
        TreeResult::Parked(outcome) => outcome,
        other => panic!("not parked: {other:?}"),
    }
}

/// The acceptance case of the ticket: what the next holder finds.
#[test]
fn a_withdrawn_runs_uncommitted_file_is_parked_and_the_next_run_starts_clean() {
    let bench = Bench::new();
    let tree = bench.tree();
    let (run_a, run_b) = (fresh_run(), fresh_run());
    let prepared = bench.prepare(&tree, &run_a);
    assert!(
        matches!(&prepared, TreeResult::Prepared { base, found: None } if base.branch == "main"),
        "{prepared:?}"
    );
    std::fs::write(bench.dir.join("stray.rs"), "fn x() {}").unwrap();

    let ParkOutcome::Committed { branch, commit } = park_outcome(bench.park(&tree, &run_a)) else {
        panic!("not committed");
    };
    assert!(
        branch.starts_with(&format!("nxs/park/run-{run_a}/")),
        "{branch}"
    );
    assert!(run_git(&bench.dir, &["show", "--stat", &commit]).contains("stray.rs"));

    let next = bench.prepare(&tree, &run_b);
    assert!(
        matches!(next, TreeResult::Prepared { found: None, .. }),
        "{next:?}"
    );
    assert_eq!(run_git(&bench.dir, &["status", "--porcelain"]), "");
    assert!(!bench.dir.join("stray.rs").exists());
    assert_eq!(bench.head(), bench.origin_main());
    assert!(
        run_git(&bench.dir, &["branch", "--list", &branch]).contains(&branch),
        "still recoverable"
    );
}
