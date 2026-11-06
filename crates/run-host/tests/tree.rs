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

fn refusal(result: &TreeResult) -> (&TreeRefusal, &str) {
    match result {
        TreeResult::Refused { reason, text } => (reason, text),
        other => panic!("not refused: {other:?}"),
    }
}

fn found_of(result: TreeResult) -> Option<ParkOutcome> {
    match result {
        TreeResult::Prepared { found, .. } => found,
        other => panic!("not prepared: {other:?}"),
    }
}

fn assert_clean_at_the_base(bench: &Bench) {
    assert_eq!(run_git(&bench.dir, &["status", "--porcelain"]), "", "clean");
    assert_eq!(bench.head(), bench.origin_main(), "at the base");
    assert!(
        !raw_git(&bench.dir, &["symbolic-ref", "-q", "HEAD"])
            .status
            .success(),
        "detached"
    );
}

/// Everything under a directory, bytes included: what "byte-identical" compares.
fn snapshot(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &std::path::Path, seen: &mut Vec<(String, Vec<u8>)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, seen);
            } else {
                seen.push((path.display().to_string(), std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut seen = Vec::new();
    walk(dir, &mut seen);
    seen
}

// ---- the clone ----------------------------------------------------------------------------------------------------

#[test]
fn a_missing_directory_is_cloned_linked_and_left_detached_at_the_remotes_default_branch() {
    let bench = Bench::new();
    let tree = bench.tree();
    let TreeResult::Prepared { base, found } = bench.prepare(&tree, &fresh_run()) else {
        panic!("not prepared");
    };
    assert_eq!(base.branch, "main");
    assert_eq!(base.commit, run_git(&bench.origin, &["rev-parse", "HEAD"]));
    assert_eq!(found, None);
    assert_clean_at_the_base(&bench);
    assert_eq!(
        bench.dir.join(".nxs").canonicalize().unwrap(),
        bench.nxs_target,
        "the root's canonical target"
    );
    assert_eq!(
        normalised(&run_git(&bench.dir, &["remote", "get-url", "origin"])),
        normalised(&run_git(&bench.root, &["remote", "get-url", "origin"])),
        "pushes land where the root's do"
    );
    let parent = bench.dir.parent().unwrap();
    assert!(
        !std::fs::read_dir(parent)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains(".cloning-")),
        "the sibling is gone"
    );
}

fn normalised(url: &str) -> String {
    nexus_run_host::git_tree::normalise_origin_url(url)
}

/// A bare `git diff main` in the coder's or a reviewer's hands fails loudly rather than comparing with a stale branch.
#[test]
fn the_clone_leaves_no_local_default_branch() {
    let bench = Bench::new();
    bench.prepared();
    assert_eq!(run_git(&bench.dir, &["branch", "--list", "main"]), "");
    assert!(!raw_git(
        &bench.dir,
        &["rev-parse", "--verify", "-q", "refs/heads/main"]
    )
    .status
    .success());
}

#[test]
fn what_a_crash_during_the_clone_left_is_deleted_and_the_clone_redone() {
    let bench = Bench::new();
    let sibling = bench.dir.with_file_name("root-v2.cloning-01ABC");
    std::fs::create_dir_all(sibling.join("half")).unwrap();
    bench.prepared();
    assert!(!sibling.exists(), "the sibling stayed");
    assert_clean_at_the_base(&bench);
}

#[test]
fn a_sibling_of_another_directory_is_left_alone() {
    let bench = Bench::new();
    let other = bench.dir.with_file_name("other.cloning-01ABC");
    std::fs::create_dir_all(&other).unwrap();
    bench.prepared();
    assert!(other.exists());
}

#[test]
fn ignored_files_survive_a_prepare_and_a_park() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::create_dir_all(bench.dir.join("target")).unwrap();
    std::fs::write(bench.dir.join("target/kept"), b"a build").unwrap();
    let run = fresh_run();
    bench.park(&tree, &run);
    bench.prepare(&tree, &run);
    assert!(bench.dir.join("target/kept").exists());
}

// ---- the link -----------------------------------------------------------------------------------------------------

#[test]
fn a_missing_link_is_recreated() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::remove_file(bench.dir.join(".nxs")).unwrap();
    let result = bench.prepare(&tree, &fresh_run());
    assert!(matches!(result, TreeResult::Prepared { .. }), "{result:?}");
    assert_eq!(
        bench.dir.join(".nxs").canonicalize().unwrap(),
        bench.nxs_target
    );
}

#[test]
fn a_link_resolving_elsewhere_is_refused_and_left_as_it_is() {
    let bench = Bench::new();
    let tree = bench.prepared();
    let other = bench.nxs_target.with_file_name("another-workspace");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::remove_file(bench.dir.join(".nxs")).unwrap();
    std::os::unix::fs::symlink(&other, bench.dir.join(".nxs")).unwrap();
    let result = bench.prepare(&tree, &fresh_run());
    assert_eq!(refusal(&result).0, &TreeRefusal::ForeignDirectory);
    assert_eq!(
        bench.dir.join(".nxs").canonicalize().unwrap(),
        other,
        "repointed"
    );
}

#[test]
fn a_link_that_resolves_nowhere_is_refused() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::remove_file(bench.dir.join(".nxs")).unwrap();
    std::os::unix::fs::symlink("/nowhere/at/all", bench.dir.join(".nxs")).unwrap();
    let result = bench.prepare(&tree, &fresh_run());
    assert_eq!(refusal(&result).0, &TreeRefusal::ForeignDirectory);
}

#[test]
fn the_excludes_are_written_on_every_prepare_and_keep_what_is_there() {
    let bench = Bench::new();
    let tree = bench.prepared();
    let exclude = bench.dir.join(".git/info/exclude");
    std::fs::write(&exclude, "mine").unwrap();
    bench.prepare(&tree, &fresh_run());
    bench.prepare(&tree, &fresh_run());
    assert_eq!(
        std::fs::read_to_string(&exclude).unwrap(),
        "mine\n.nxs\n.nxf/\n"
    );
}

/// For a repository that does not ignore them itself: the park must not commit the link or the gate log.
#[test]
fn a_park_commits_neither_the_link_nor_the_gate_log_where_the_repository_ignores_neither() {
    let bench = Bench::new();
    std::fs::remove_file(bench.origin.join(".gitignore")).unwrap();
    run_git(&bench.origin, &["add", "-A"]);
    run_git(&bench.origin, &["commit", "-q", "-m", "no ignore file"]);
    let tree = bench.prepared();
    std::fs::create_dir_all(bench.dir.join(".nxf/review")).unwrap();
    std::fs::write(bench.dir.join(".nxf/review/gates.txt"), b"green").unwrap();
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    let ParkOutcome::Committed { commit, .. } = park_outcome(bench.park(&tree, &fresh_run()))
    else {
        panic!("not committed");
    };
    let files = run_git(&bench.dir, &["show", "--name-only", "--format=", &commit]);
    assert_eq!(files, "stray.rs");
}

// ---- identity -----------------------------------------------------------------------------------------------------

#[test]
fn a_directory_holding_another_repository_is_foreign_at_once_naming_the_path_and_the_remedy() {
    let bench = Bench::new();
    let base = bench.origin.parent().unwrap();
    origin_with_main(&base.join("other-origin"));
    run_git(base, &["clone", "-q", "other-origin", "root-v2"]);
    let result = bench.prepare(&bench.tree(), &fresh_run());
    let (reason, text) = refusal(&result);
    assert_eq!(reason, &TreeRefusal::ForeignDirectory);
    assert!(text.contains(&bench.dir.display().to_string()), "{text}");
    assert!(text.contains("remedy"), "{text}");
    assert!(
        !text.contains("other-origin"),
        "a URL stays out of status: {text}"
    );
}

#[test]
fn a_repository_with_no_origin_in_the_directory_is_foreign() {
    let bench = Bench::new();
    std::fs::create_dir_all(&bench.dir).unwrap();
    run_git(&bench.dir, &["init", "-q"]);
    let result = bench.prepare(&bench.tree(), &fresh_run());
    assert_eq!(refusal(&result).0, &TreeRefusal::ForeignDirectory);
}

#[test]
fn a_root_with_no_origin_is_foreign_with_its_remedy() {
    let bench = Bench::new();
    run_git(&bench.root, &["remote", "remove", "origin"]);
    let result = bench.prepare(&bench.tree(), &fresh_run());
    let (reason, text) = refusal(&result);
    assert_eq!(reason, &TreeRefusal::ForeignDirectory);
    assert!(
        text.contains("no `origin`") && text.contains("remote add"),
        "{text}"
    );
    assert!(!bench.dir.exists(), "nothing was cloned");
}

#[test]
fn a_directory_that_is_no_repository_is_refused_even_inside_another_repository() {
    let bench = Bench::new();
    // The temp directory itself is a repository, as a workspace's parent can be: discovery must not find it.
    let parent = bench.dir.parent().unwrap();
    run_git(parent, &["init", "-q"]);
    std::fs::create_dir_all(&bench.dir).unwrap();
    std::fs::write(bench.dir.join("somebodys"), b"file").unwrap();
    let result = bench.prepare(&bench.tree(), &fresh_run());
    assert_eq!(refusal(&result).0, &TreeRefusal::NotARepository);
    assert!(bench.dir.join("somebodys").exists());
}

/// The scheme and the suffix do not make it another repository; the mismatch's remedy is for the case they do.
#[test]
fn the_same_origin_written_differently_is_the_same_repository() {
    let bench = Bench::new();
    let tree = bench.prepared();
    let plain = bench.origin.display().to_string();
    run_git(
        &bench.dir,
        &["remote", "set-url", "origin", &format!("file://{plain}/")],
    );
    let result = bench.prepare(&tree, &fresh_run());
    assert!(matches!(result, TreeResult::Prepared { .. }), "{result:?}");
}

// ---- the rescue ---------------------------------------------------------------------------------------------------

fn dirty_rescue_on(bench: &Bench, set_up: &[&[&str]]) -> (RunId, ParkOutcome) {
    let tree = bench.prepared();
    for args in set_up {
        run_git(&bench.dir, args);
    }
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    let run = fresh_run();
    let found = found_of(bench.prepare(&tree, &run)).expect("found");
    assert_clean_at_the_base(bench);
    assert!(!bench.dir.join("stray.rs").exists());
    (run, found)
}

#[test]
fn a_dirty_detached_head_is_rescued_onto_a_park_branch_this_run_names() {
    let bench = Bench::new();
    let (run, found) = dirty_rescue_on(&bench, &[]);
    let ParkOutcome::Committed { branch, commit } = found else {
        panic!("{found:?}");
    };
    assert!(
        branch.starts_with(&format!("nxs/park/run-{run}/")),
        "{branch}"
    );
    assert_eq!(run_git(&bench.dir, &["rev-parse", &branch]), commit);
    assert!(run_git(&bench.dir, &["show", "--stat", &commit]).contains("stray.rs"));
    assert!(run_git(&bench.dir, &["log", "-1", "--format=%s", &commit])
        .contains("found_dirty_at_prepare"));
}

#[test]
fn a_dirty_local_main_gets_a_branch_and_is_never_on_own_branch() {
    let bench = Bench::new();
    let (_, found) = dirty_rescue_on(&bench, &[&["switch", "-q", "-c", "main", "origin/main"]]);
    assert!(matches!(found, ParkOutcome::Committed { .. }), "{found:?}");
}

#[test]
fn a_dirty_feat_branch_is_on_own_branch() {
    let bench = Bench::new();
    let (_, found) = dirty_rescue_on(&bench, &[&["switch", "-q", "-c", "feat/x"]]);
    let ParkOutcome::OnOwnBranch { branch, commit } = found else {
        panic!("{found:?}");
    };
    assert_eq!(branch, "feat/x");
    assert!(run_git(&bench.dir, &["show", "--stat", &commit]).contains("stray.rs"));
    assert_eq!(bench.park_branches(), Vec::<String>::new());
}

#[test]
fn a_clean_detached_head_ahead_of_the_base_gets_a_branch() {
    let bench = Bench::new();
    let tree = bench.prepared();
    run_git(
        &bench.dir,
        &["commit", "-q", "--allow-empty", "-m", "ahead"],
    );
    let ahead = bench.head();
    let found = found_of(bench.prepare(&tree, &fresh_run())).expect("found");
    let ParkOutcome::Committed { commit, .. } = found else {
        panic!("{found:?}");
    };
    assert_eq!(commit, ahead);
    assert_clean_at_the_base(&bench);
}

/// Ancestry, not inequality: the remote moved on, HEAD did not, and there is nothing to save.
#[test]
fn a_clean_head_behind_the_base_gets_no_branch() {
    let bench = Bench::new();
    let tree = bench.prepared();
    advance_origin(&bench.origin);
    run_git(&bench.dir, &["fetch", "-q", "origin"]);
    assert_ne!(bench.head(), bench.origin_main(), "behind it");
    let run = fresh_run();
    assert_eq!(
        park_outcome(bench.park(&tree, &run)),
        ParkOutcome::Clean { branch: None }
    );
    assert_eq!(found_of(bench.prepare(&tree, &run)), None);
    assert_eq!(bench.park_branches(), Vec::<String>::new());
    assert_clean_at_the_base(&bench);
}

#[test]
fn a_prepare_after_the_remote_moved_stands_at_the_new_commit() {
    let bench = Bench::new();
    let tree = bench.prepared();
    let moved = advance_origin(&bench.origin);
    let TreeResult::Prepared { base, .. } = bench.prepare(&tree, &fresh_run()) else {
        panic!("not prepared");
    };
    assert_eq!(base.commit, moved);
    assert_eq!(bench.head(), moved);
}

/// A failed fetch comes after the rescue: the retry finds HEAD on the park branch, and has to name it again.
#[test]
fn found_is_kept_across_a_failed_fetch() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    let moved_aside = bench.origin.with_file_name("origin-away");
    std::fs::rename(&bench.origin, &moved_aside).unwrap();
    let run = fresh_run();
    let first = bench.prepare(&tree, &run);
    assert_eq!(
        refusal(&first).0,
        &TreeRefusal::Git {
            reason: GitReason::FetchFailed
        },
        "{first:?}"
    );
    std::fs::rename(&moved_aside, &bench.origin).unwrap();
    let found = found_of(bench.prepare(&tree, &run)).expect("still named");
    let ParkOutcome::Committed { branch, .. } = found else {
        panic!("{found:?}");
    };
    assert_eq!(
        bench.park_branches(),
        vec![branch],
        "one branch, not a second"
    );
    assert_clean_at_the_base(&bench);
}

/// The window after the checkout: HEAD has left the park branch, and the result never reached the log.
#[test]
fn found_is_kept_when_the_result_of_a_finished_prepare_is_lost() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    let run = fresh_run();
    let first = found_of(bench.prepare(&tree, &run)).expect("found");
    assert_eq!(found_of(bench.prepare(&tree, &run)), Some(first));
}

#[test]
fn another_runs_park_branch_is_nothing_this_run_found() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    bench.park(&tree, &fresh_run());
    assert_eq!(found_of(bench.prepare(&tree, &fresh_run())), None);
}

// ---- refusals inside the window -----------------------------------------------------------------------------------

#[test]
fn a_tree_in_the_middle_of_a_merge_is_mid_sequence_and_nothing_is_committed() {
    for (file, what) in [
        ("MERGE_HEAD", SequenceKind::Merge),
        ("rebase-merge", SequenceKind::Rebase),
        ("CHERRY_PICK_HEAD", SequenceKind::CherryPick),
        ("REVERT_HEAD", SequenceKind::Revert),
        ("BISECT_LOG", SequenceKind::Bisect),
    ] {
        let bench = Bench::new();
        let tree = bench.prepared();
        std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
        std::fs::write(bench.dir.join(".git").join(file), b"x").unwrap();
        let result = bench.prepare(&tree, &fresh_run());
        assert_eq!(
            refusal(&result).0,
            &TreeRefusal::MidSequence { what: what.clone() }
        );
        assert!(refusal(&result).1.contains("remedy"), "{result:?}");
        assert_eq!(bench.park_branches(), Vec::<String>::new());
        assert!(bench.dir.join("stray.rs").exists());
    }
}

#[test]
fn a_planted_lock_of_any_kind_is_lock_held_naming_the_file_and_a_prepare_works_once_it_is_gone() {
    for lock in [
        "index.lock",
        "HEAD.lock",
        "config.lock",
        "refs/heads/x.lock",
    ] {
        let bench = Bench::new();
        let tree = bench.prepared();
        let planted = bench.dir.join(".git").join(lock);
        std::fs::write(&planted, b"").unwrap();
        let result = bench.prepare(&tree, &fresh_run());
        let (reason, text) = refusal(&result);
        assert_eq!(
            reason,
            &TreeRefusal::Git {
                reason: GitReason::LockHeld
            },
            "{lock}"
        );
        assert!(text.contains(lock) && text.contains("remedy"), "{text}");
        assert!(planted.exists(), "{lock} is nobody's to remove here");
        std::fs::remove_file(&planted).unwrap();
        assert!(matches!(
            bench.prepare(&tree, &fresh_run()),
            TreeResult::Prepared { .. }
        ));
    }
}

#[test]
fn a_park_refused_by_a_lock_saves_nothing_and_the_retry_parks() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    let lock = bench.dir.join(".git/index.lock");
    std::fs::write(&lock, b"").unwrap();
    let run = fresh_run();
    let result = bench.park(&tree, &run);
    assert_eq!(
        refusal(&result).0,
        &TreeRefusal::Git {
            reason: GitReason::LockHeld
        }
    );
    std::fs::remove_file(&lock).unwrap();
    assert!(matches!(
        park_outcome(bench.park(&tree, &run)),
        ParkOutcome::Committed { .. }
    ));
}

#[test]
fn an_unreachable_remote_ends_as_fetch_failed() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::rename(&bench.origin, bench.origin.with_file_name("gone")).unwrap();
    let result = bench.prepare(&tree, &fresh_run());
    assert_eq!(
        refusal(&result).0,
        &TreeRefusal::Git {
            reason: GitReason::FetchFailed
        }
    );
}

// ---- the remote's default branch ----------------------------------------------------------------------------------

#[test]
fn an_empty_remote_is_refused_naming_the_missing_default_and_leaves_no_directory() {
    let bench = Bench::new();
    let base = bench.origin.parent().unwrap();
    let empty = base.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    run_git(&empty, &["init", "-q", "--bare"]);
    run_git(
        &bench.root,
        &["remote", "set-url", "origin", &empty.display().to_string()],
    );
    let result = bench.prepare(&bench.tree(), &fresh_run());
    let (reason, text) = refusal(&result);
    assert_eq!(
        reason,
        &TreeRefusal::Git {
            reason: GitReason::ExitNonzero
        }
    );
    assert!(
        text.contains("no default branch") && text.contains("set-head"),
        "{text}"
    );
    assert!(!bench.dir.exists());
    assert!(!std::fs::read_dir(base)
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().contains(".cloning-")));
}

#[test]
fn a_removed_origin_head_is_refused_with_its_remedy() {
    let bench = Bench::new();
    let tree = bench.prepared();
    run_git(&bench.dir, &["remote", "set-head", "origin", "-d"]);
    let result = bench.prepare(&tree, &fresh_run());
    let text = refusal(&result).1;
    assert!(
        text.contains("no default branch") && text.contains("set-head"),
        "{text}"
    );
    run_git(&bench.dir, &["remote", "set-head", "origin", "--auto"]);
    assert!(matches!(
        bench.prepare(&tree, &fresh_run()),
        TreeResult::Prepared { .. }
    ));
}

#[test]
fn a_default_branch_the_prune_took_away_is_a_dangling_origin_head_refused_with_its_remedy() {
    let bench = Bench::new();
    let tree = bench.prepared();
    // Git 2.48 and later would repoint origin/HEAD during the fetch by itself; older ones ignore this and leave it.
    run_git(
        &bench.dir,
        &["config", "remote.origin.followRemoteHEAD", "never"],
    );
    run_git(&bench.origin, &["branch", "-m", "main", "trunk"]);
    let result = bench.prepare(&tree, &fresh_run());
    let text = refusal(&result).1;
    assert!(
        text.contains("dangling") && text.contains("set-head"),
        "{text}"
    );
}

// ---- the park -----------------------------------------------------------------------------------------------------

#[test]
fn a_foreign_directorys_withdrawn_run_leaves_the_foreign_repository_byte_identical_and_releases() {
    let bench = Bench::new();
    let base = bench.origin.parent().unwrap();
    origin_with_main(&base.join("other-origin"));
    run_git(base, &["clone", "-q", "other-origin", "root-v2"]);
    std::fs::write(bench.dir.join("theirs.rs"), b"somebody's work").unwrap();
    let before = snapshot(&bench.dir);
    let outcome = park_outcome(bench.park(&bench.tree(), &fresh_run()));
    assert_eq!(outcome, ParkOutcome::NotHeld);
    assert_eq!(snapshot(&bench.dir), before);
}

#[test]
fn a_directory_linked_elsewhere_is_not_parked_either() {
    let bench = Bench::new();
    let tree = bench.prepared();
    let other = bench.nxs_target.with_file_name("another-workspace");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::remove_file(bench.dir.join(".nxs")).unwrap();
    std::os::unix::fs::symlink(&other, bench.dir.join(".nxs")).unwrap();
    std::fs::write(bench.dir.join("theirs.rs"), b"work").unwrap();
    assert_eq!(
        park_outcome(bench.park(&tree, &fresh_run())),
        ParkOutcome::NotHeld
    );
    assert!(bench.dir.join("theirs.rs").exists());
    assert_eq!(bench.park_branches(), Vec::<String>::new());
}

#[test]
fn a_directory_that_is_no_repository_at_park_time_is_left_alone_and_released() {
    let bench = Bench::new();
    std::fs::create_dir_all(&bench.dir).unwrap();
    std::fs::write(bench.dir.join("somebodys"), b"file").unwrap();
    let before = snapshot(&bench.dir);
    assert_eq!(
        park_outcome(bench.park(&bench.tree(), &fresh_run())),
        ParkOutcome::NotHeld
    );
    assert_eq!(snapshot(&bench.dir), before);
}

#[test]
fn a_directory_moved_aside_is_missing_which_releases() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::rename(&bench.dir, bench.dir.with_file_name("aside")).unwrap();
    assert_eq!(
        park_outcome(bench.park(&tree, &fresh_run())),
        ParkOutcome::Missing
    );
}

/// An unmounted volume is no gone directory: nothing is released and nothing cloned.
#[test]
fn missing_is_said_only_when_the_parent_and_the_root_exist() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::rename(&bench.dir, bench.dir.with_file_name("aside")).unwrap();
    assert!(tree_state(&bench.root, &bench.dir).unwrap().missing);

    let root_aside = bench.root.with_file_name("root-aside");
    std::fs::rename(&bench.root, &root_aside).unwrap();
    assert!(
        tree_state(&bench.root, &bench.dir).is_err(),
        "no root: no answer"
    );
    let parked = bench.park(&tree, &fresh_run());
    assert!(refusal(&parked).1.contains("not there"), "{parked:?}");
    let prepared = bench.prepare(&tree, &fresh_run());
    assert!(refusal(&prepared).1.contains("not there"), "{prepared:?}");
    assert!(!bench.dir.exists(), "nothing was cloned");
    std::fs::rename(&root_aside, &bench.root).unwrap();

    let elsewhere = bench
        .dir
        .parent()
        .unwrap()
        .join("no")
        .join("such")
        .join("root-v2");
    assert!(
        tree_state(&bench.root, &elsewhere).is_err(),
        "no parent: no answer"
    );
}

#[test]
fn a_clean_branch_level_with_its_upstream_is_clean_and_names_the_branch() {
    let bench = Bench::new();
    let tree = bench.prepared();
    run_git(&bench.origin, &["branch", "feat/x"]);
    run_git(&bench.dir, &["fetch", "-q", "origin"]);
    run_git(&bench.dir, &["switch", "-q", "feat/x"]);
    assert_eq!(
        park_outcome(bench.park(&tree, &fresh_run())),
        ParkOutcome::Clean {
            branch: Some("feat/x".to_owned())
        }
    );
}

/// The ride-along (§12.11): `git switch feat/x` lands on a local commit an earlier park left, and only a look at
/// `origin/<branch>..HEAD` shows it. That is what the park owes a notice for.
#[test]
fn a_clean_branch_with_local_commits_ahead_of_its_upstream_is_on_own_branch_and_the_coders_check_reports_it(
) {
    let bench = Bench::new();
    let tree = bench.prepared();
    run_git(&bench.origin, &["branch", "feat/x"]);
    run_git(&bench.dir, &["fetch", "-q", "origin"]);
    run_git(&bench.dir, &["switch", "-q", "feat/x"]);
    run_git(
        &bench.dir,
        &["commit", "-q", "--allow-empty", "-m", "a local park commit"],
    );
    let local = bench.head();
    let run = fresh_run();
    assert_eq!(
        park_outcome(bench.park(&tree, &run)),
        ParkOutcome::OnOwnBranch {
            branch: "feat/x".to_owned(),
            commit: local.clone()
        }
    );
    assert_eq!(
        run_git(&bench.dir, &["rev-list", "origin/feat/x..HEAD"]),
        local
    );
    assert_eq!(
        run_git(&bench.dir, &["rev-list", "HEAD..origin/feat/x"]),
        "",
        "and only behind reads clean"
    );
    let found = found_of(bench.prepare(&tree, &fresh_run()));
    assert_eq!(
        found,
        Some(ParkOutcome::OnOwnBranch {
            branch: "feat/x".to_owned(),
            commit: local.clone()
        })
    );
    run_git(&bench.dir, &["switch", "-q", "feat/x"]);
    assert_eq!(bench.head(), local, "the next ride-along lands on it");
}

#[test]
fn a_branch_the_remote_never_had_with_commits_is_on_own_branch() {
    let bench = Bench::new();
    let tree = bench.prepared();
    run_git(&bench.dir, &["switch", "-q", "-c", "feat/y"]);
    run_git(
        &bench.dir,
        &["commit", "-q", "--allow-empty", "-m", "local"],
    );
    assert!(matches!(
        park_outcome(bench.park(&tree, &fresh_run())),
        ParkOutcome::OnOwnBranch { .. }
    ));
}

#[test]
fn a_fresh_branch_with_no_commits_is_clean() {
    let bench = Bench::new();
    let tree = bench.prepared();
    run_git(&bench.dir, &["switch", "-q", "-c", "feat/y"]);
    assert_eq!(
        park_outcome(bench.park(&tree, &fresh_run())),
        ParkOutcome::Clean {
            branch: Some("feat/y".to_owned())
        }
    );
}

#[test]
fn a_clean_local_main_at_the_base_is_clean_and_names_it() {
    let bench = Bench::new();
    let tree = bench.prepared();
    run_git(&bench.dir, &["switch", "-q", "-c", "main", "origin/main"]);
    assert_eq!(
        park_outcome(bench.park(&tree, &fresh_run())),
        ParkOutcome::Clean {
            branch: Some("main".to_owned())
        }
    );
}

#[test]
fn a_park_retried_after_a_crash_between_the_commit_and_its_record_reuses_the_branch() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    let run = fresh_run();
    let first = park_outcome(bench.park(&tree, &run));
    assert_eq!(park_outcome(bench.park(&tree, &run)), first);
    assert_eq!(bench.park_branches().len(), 1);
}

#[test]
fn a_crash_between_the_new_branch_and_the_commit_commits_on_that_branch() {
    let bench = Bench::new();
    let tree = bench.prepared();
    let run = fresh_run();
    let branch = format!("nxs/park/run-{run}/20261105T100000Z");
    run_git(&bench.dir, &["switch", "-q", "-c", &branch]);
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    let ParkOutcome::Committed {
        branch: parked,
        commit,
    } = park_outcome(bench.park(&tree, &run))
    else {
        panic!("not committed");
    };
    assert_eq!(parked, branch);
    assert!(run_git(&bench.dir, &["show", "--stat", &commit]).contains("stray.rs"));
    assert_eq!(bench.park_branches().len(), 1);
}

#[test]
fn the_parks_message_names_the_run_and_the_occasion() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    let run = fresh_run();
    let ParkOutcome::Committed { commit, branch } = park_outcome(bench.park(&tree, &run)) else {
        panic!("not committed");
    };
    let subject = run_git(&bench.dir, &["log", "-1", "--format=%s", &commit]);
    assert!(
        subject.contains(run.as_str()) && subject.contains("withdrawn"),
        "{subject}"
    );
    let instant = branch.rsplit('/').next().unwrap();
    assert_eq!(instant.len(), "20261105T100000Z".len(), "{instant}");
    assert!(instant.ends_with('Z') && instant.contains('T'), "{instant}");
}

// ---- the runner's requirements ------------------------------------------------------------------------------------

/// The bytes decide (6j6v.2af2: a decode failure once read a dirty tree as clean). A status that is not UTF-8 is dirty.
#[test]
fn a_status_that_is_not_utf8_reads_as_dirty() {
    let bench = Bench::new();
    bench.prepared();
    let git = bench
        .fake_git(r#"case "$*" in *"status --porcelain"*) printf '?? \377\376\n'; exit 0;; esac"#);
    let state =
        nexus_run_host::git_tree::tree_state_with_git(&git, &bench.root, &bench.dir).unwrap();
    assert!(state.dirty);
    // And prepare does not pass over it as clean: it tries to save it, and git has nothing to commit.
    let tree = bench.tree_with_git(&git, GitBounds::default());
    let result = bench.prepare(&tree, &fresh_run());
    assert!(
        !matches!(result, TreeResult::Prepared { found: None, .. }),
        "{result:?}"
    );
}

#[test]
fn a_branch_name_that_is_not_utf8_is_a_refusal_never_another_branch() {
    let bench = Bench::new();
    bench.prepared();
    let git = bench.fake_git(
        r#"case "$*" in *"symbolic-ref -q --short HEAD"*) printf 'feat/\377\n'; exit 0;; esac"#,
    );
    let tree = bench.tree_with_git(&git, GitBounds::default());
    let result = bench.park(&tree, &fresh_run());
    assert!(refusal(&result).1.contains("not UTF-8"), "{result:?}");
}

#[test]
fn a_failing_post_commit_hook_does_not_run_and_does_not_fail_the_park() {
    let bench = Bench::new();
    let tree = bench.prepared();
    let marker = bench.dir.with_file_name("the-hook-ran");
    for hook in ["pre-commit", "post-commit", "commit-msg"] {
        let path = bench.dir.join(".git/hooks").join(hook);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!("#!/bin/sh\ntouch {}\nexit 1\n", marker.display()),
        )
        .unwrap();
        make_executable(&path);
    }
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    assert!(matches!(
        park_outcome(bench.park(&tree, &fresh_run())),
        ParkOutcome::Committed { .. }
    ));
    assert!(!marker.exists());
}

#[test]
fn no_configured_identity_uses_the_fallback_and_a_configured_one_is_kept() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    let ParkOutcome::Committed { commit, .. } = park_outcome(bench.park(&tree, &fresh_run()))
    else {
        panic!("not committed");
    };
    assert_eq!(
        run_git(&bench.dir, &["log", "-1", "--format=%an <%ae>", &commit]),
        "nxs <nxs@localhost>"
    );

    run_git(&bench.dir, &["config", "user.name", "Somebody"]);
    run_git(
        &bench.dir,
        &["config", "user.email", "somebody@example.com"],
    );
    std::fs::write(bench.dir.join("stray2.rs"), b"work").unwrap();
    let run = fresh_run();
    bench.prepare(&tree, &run);
    std::fs::write(bench.dir.join("stray3.rs"), b"work").unwrap();
    let ParkOutcome::Committed { commit, .. } = park_outcome(bench.park(&tree, &run)) else {
        panic!("not committed");
    };
    assert_eq!(
        run_git(&bench.dir, &["log", "-1", "--format=%an <%ae>", &commit]),
        "Somebody <somebody@example.com>"
    );
}

/// `-c core.sshCommand=` alone stops ssh with "cannot run" (j6ba's step 0): an ssh origin has to reach ssh.
#[test]
fn an_ssh_origin_reaches_ssh() {
    let bench = Bench::new();
    run_git(
        &bench.root,
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://nobody@127.0.0.1:1/x.git",
        ],
    );
    let result = bench.prepare(&bench.tree(), &fresh_run());
    let text = refusal(&result).1;
    assert!(!text.contains("cannot run"), "{text}");
}

#[test]
fn a_transport_outside_the_allowed_protocols_never_runs() {
    let bench = Bench::new();
    let marker = bench.dir.with_file_name("the-transport-ran");
    let url = format!("ext::sh -c touch% {}", marker.display());
    run_git(&bench.root, &["config", "remote.origin.url", &url]);
    let result = bench.prepare(&bench.tree(), &fresh_run());
    assert!(matches!(result, TreeResult::Refused { .. }), "{result:?}");
    assert!(!marker.exists());
}

/// Git refuses a URL that starts with a dash by itself these days, so what is tested is the `--` the runner owes it.
#[test]
fn an_origin_that_reads_like_an_option_is_a_url_and_nothing_else() {
    let bench = Bench::new();
    let marker = bench.dir.with_file_name("the-option-ran");
    let seen = bench.dir.with_file_name("clone-arguments");
    let git = bench.fake_git(&format!(
        r#"case "$*" in *"clone -q"*) echo "$*" > {};; esac"#,
        seen.display()
    ));
    let url = format!("--upload-pack=touch {}", marker.display());
    run_git(&bench.root, &["config", "remote.origin.url", &url]);
    let tree = bench.tree_with_git(&git, GitBounds::default());
    let result = bench.prepare(&tree, &fresh_run());
    assert!(matches!(result, TreeResult::Refused { .. }), "{result:?}");
    assert!(!marker.exists());
    let arguments = std::fs::read_to_string(&seen).expect("the clone ran");
    assert!(
        arguments.contains(&format!("clone -q -- {url} ")),
        "{arguments}"
    );
}

/// A token in the URL stays out of the refusal's text, and out of the ledger while the clone runs.
#[test]
fn a_token_bearing_url_reaches_neither_the_refusal_nor_the_ledger() {
    let bench = Bench::new();
    let seen = bench.dir.with_file_name("ledger-during-the-clone");
    let git = bench.fake_git(&format!(
        r#"case "$*" in *"clone -q"*) cat {} > {}; echo "fatal: unable to access 'https://x-access-token:ghp_canary@127.0.0.1/a.git/'" >&2; exit 128;; esac"#,
        bench.ledger.path().display(),
        seen.display()
    ));
    run_git(
        &bench.root,
        &[
            "remote",
            "set-url",
            "origin",
            "https://x-access-token:ghp_canary@127.0.0.1:1/a.git",
        ],
    );
    let tree = bench.tree_with_git(&git, GitBounds::default());
    let result = bench.prepare(&tree, &fresh_run());
    let text = refusal(&result).1;
    assert!(
        text.contains("unable to access"),
        "git's text is kept: {text}"
    );
    assert!(!text.contains("ghp_canary"), "{text}");
    let ledger = std::fs::read_to_string(&seen).expect("the clone ran");
    assert!(ledger.contains("\"step\":\"clone\""), "{ledger}");
    assert!(
        !ledger.contains("ghp_canary") && !ledger.contains("127.0.0.1"),
        "{ledger}"
    );
}

/// A helper of another session holding stderr: git exited, and what it said is not known.
#[test]
fn a_helper_holding_stderr_after_the_exit_is_a_refusal_never_an_empty_answer() {
    let bench = Bench::new();
    bench.prepared();
    let pid_file = bench.dir.with_file_name("helper-pid");
    let git = bench.fake_git(&format!(
        r#"case "$*" in *"fetch -q"*) perl -e 'use POSIX; POSIX::setsid(); open(F, ">{0}"); print F $$; close(F); exec("sleep", "600")' & exit 0;; esac"#,
        pid_file.display()
    ));
    let tree = bench.tree_with_git(&git, GitBounds::default());
    let result = bench.prepare(&tree, &fresh_run());
    until("the helper wrote its pid", || {
        std::fs::read_to_string(&pid_file).is_ok_and(|text| !text.is_empty())
    });
    let helper = std::fs::read_to_string(&pid_file).unwrap();
    let _ = std::process::Command::new("kill")
        .args(["-KILL", helper.trim()])
        .status();
    assert!(refusal(&result).1.contains("output"), "{result:?}");
}

#[test]
fn a_helper_holding_stderr_past_the_kill_is_a_refusal_and_the_operation_answers() {
    let bench = Bench::new();
    bench.prepared();
    let pid_file = bench.dir.with_file_name("helper-pid");
    let git = bench.fake_git(&format!(
        r#"case "$*" in *"fetch -q"*) perl -e 'use POSIX; POSIX::setsid(); open(F, ">{0}"); print F $$; close(F); exec("sleep", "600")' & exec sleep 600;; esac"#,
        pid_file.display()
    ));
    let bounds = GitBounds {
        fetch: std::time::Duration::from_millis(500),
        ..GitBounds::default()
    };
    let tree = bench.tree_with_git(&git, bounds);
    let result = bench.prepare(&tree, &fresh_run());
    until("the helper wrote its pid", || {
        std::fs::read_to_string(&pid_file).is_ok_and(|text| !text.is_empty())
    });
    let helper = std::fs::read_to_string(&pid_file).unwrap();
    let alive = pid_alive(helper.trim().parse().unwrap());
    let _ = std::process::Command::new("kill")
        .args(["-KILL", helper.trim()])
        .status();
    assert!(
        alive,
        "the helper survived the group kill, or this tests nothing"
    );
    assert_eq!(
        refusal(&result).0,
        &TreeRefusal::Git {
            reason: GitReason::BoundExceeded
        }
    );
}

/// `git://` is a transport git allows by default, so only `GIT_ALLOW_PROTOCOL` refuses it.
#[test]
fn a_transport_git_allows_by_default_is_refused_by_the_allowed_protocols() {
    let bench = Bench::new();
    run_git(
        &bench.root,
        &["remote", "set-url", "origin", "git://127.0.0.1:1/x.git"],
    );
    let result = bench.prepare(&bench.tree(), &fresh_run());
    assert!(refusal(&result).1.contains("not allowed"), "{result:?}");
}

/// The root is the old engine's working copy, and its lock is that engine's: a read of ours there that is killed
/// answers for the v2 directory's locks and for no others.
#[test]
fn a_read_of_the_root_killed_at_its_bound_never_clears_a_lock_there() {
    let bench = Bench::new();
    let git = bench.fake_git(
        r#"case "$*" in *"remote get-url origin"*) : > .git/index.lock; exec sleep 600;; esac"#,
    );
    let bounds = GitBounds {
        other: std::time::Duration::from_millis(500),
        ..GitBounds::default()
    };
    let tree = bench.tree_with_git(&git, bounds);
    let result = bench.prepare(&tree, &fresh_run());
    assert_eq!(
        refusal(&result).0,
        &TreeRefusal::Git {
            reason: GitReason::BoundExceeded
        }
    );
    assert!(
        bench.root.join(".git/index.lock").exists(),
        "a lock in the root was cleared"
    );
}

// ---- status -------------------------------------------------------------------------------------------------------

#[test]
fn the_state_of_a_directory_is_read_live() {
    let bench = Bench::new();
    bench.prepared();
    let state = tree_state(&bench.root, &bench.dir).unwrap();
    assert_eq!(state.commit.as_deref(), Some(bench.head().as_str()));
    assert_eq!(state.branch, None);
    assert!(!state.dirty && !state.mid_sequence && !state.missing);

    run_git(&bench.dir, &["switch", "-q", "-c", "feat/x"]);
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    std::fs::write(bench.dir.join(".git/MERGE_HEAD"), b"x").unwrap();
    let state = tree_state(&bench.root, &bench.dir).unwrap();
    assert_eq!(state.branch.as_deref(), Some("feat/x"));
    assert!(state.dirty && state.mid_sequence);
}

/// Reading a state takes no lock: it runs beside the operation the worker is doing.
#[test]
fn reading_the_state_leaves_the_tree_byte_identical() {
    let bench = Bench::new();
    bench.prepared();
    std::fs::write(bench.dir.join("README"), b"changed").unwrap();
    let before = snapshot(&bench.dir.join(".git"));
    tree_state(&bench.root, &bench.dir).unwrap();
    assert_eq!(snapshot(&bench.dir.join(".git")), before);
}

#[test]
fn tree_state_of_something_that_is_no_repository_has_no_answer() {
    let bench = Bench::new();
    std::fs::create_dir_all(&bench.dir).unwrap();
    assert!(tree_state(&bench.root, &bench.dir).is_err());
}

/// The CLI process reads `tree.in_flight` from the ledger file: it has no worker to ask.
#[test]
fn a_separate_process_sees_an_in_flight_clone_from_the_ledger_and_nothing_once_the_child_is_dead() {
    let bench = Bench::new();
    let git = bench.fake_git(r#"case "$*" in *"clone -q"*) exec sleep 600;; esac"#);
    let tree = bench.tree_with_git(&git, GitBounds::default());
    tree.begin(bench.prepare_op(&fresh_run())).unwrap();
    until("the clone is recorded", || {
        bench
            .ledger
            .read()
            .is_some_and(|e| e.step == "clone" && e.pid.is_some())
    });
    let pid = bench.ledger.read().unwrap().pid.unwrap();
    let ask = || {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_run-host-crashbox"))
            .arg(bench.origin.parent().unwrap())
            .arg("tree-state")
            .output()
            .expect("the other process runs");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let during = ask();
    assert!(during.contains("in_flight prepare clone"), "{during}");
    assert!(during.contains("missing true"), "{during}");
    tree.cancel();
    finished(&tree);
    assert!(!pid_alive(pid), "the clone was killed");
    assert!(ask().contains("in_flight none"));
}

#[test]
fn a_cancelled_clone_leaves_an_empty_group_and_the_next_prepare_clones() {
    let bench = Bench::new();
    let git = bench.fake_git(r#"case "$*" in *"clone -q"*) sleep 600 & wait;; esac"#);
    let tree = bench.tree_with_git(&git, GitBounds::default());
    tree.begin(bench.prepare_op(&fresh_run())).unwrap();
    until("the clone is recorded", || {
        bench
            .ledger
            .read()
            .is_some_and(|e| e.step == "clone" && e.pid.is_some())
    });
    let pid = bench.ledger.read().unwrap().pid.unwrap();
    tree.cancel();
    assert!(matches!(finished(&tree), TreeResult::Refused { .. }));
    until("the group is empty", || group_members(pid).is_empty());
    bench.prepared();
    assert_clean_at_the_base(&bench);
}

// ---- under a real host --------------------------------------------------------------------------------------------

mod under_a_host {
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use nexus_run_contracts::{
        Event, EventBody, GitReason, Origin, ParkOutcome, RunId, RunStore, Secs, TreeRefusal,
    };
    use nexus_run_host::bridge;
    use nexus_run_host::git_tree::{GitBounds, GitTree};
    use nexus_run_host::test_support::scripted_backend::{Cue, SessionInfo};
    use nexus_run_host::test_support::{
        read_port_lines, scenario_named, PortLine, Rig, StoreProbe, CLOCK_START, WORKSPACE,
    };
    use nexus_run_host::{LeaseView, TreeLedger, WorkTree};

    use super::{hermetic, make_executable, run_git, snapshot, until, which_git};
    use crate::common::tree::group_members;

    fn silent(_: &SessionInfo) -> Cue {
        Cue::Silent
    }

    fn git_rig(dir: &std::path::Path) -> Rig<GitTree> {
        hermetic();
        Rig::git_tree(dir, CLOCK_START, Box::new(silent))
    }

    fn real_path(temp: &tempfile::TempDir) -> std::path::PathBuf {
        temp.path().canonicalize().unwrap()
    }

    fn clone_directory(dir: &std::path::Path) -> std::path::PathBuf {
        let v2 = dir.join("root-v2");
        run_git(dir, &["clone", "-q", "origin", "root-v2"]);
        std::os::unix::fs::symlink(dir.join("nxs-target"), v2.join(".nxs")).unwrap();
        run_git(&v2, &["switch", "-q", "--detach", "origin/main"]);
        run_git(&v2, &["branch", "-q", "-D", "main"]);
        v2
    }

    fn outcome_of(rig: &Rig<GitTree>, run: &RunId) -> Option<ParkOutcome> {
        rig.events(run).iter().find_map(|event| match &event.body {
            EventBody::TreeParked { outcome } => Some(outcome.clone()),
            _ => None,
        })
    }

    fn position(rig: &Rig<GitTree>, run: &RunId, kind: &str) -> usize {
        rig.events(run)
            .iter()
            .position(|event| event.body.kind() == kind)
            .unwrap_or_else(|| panic!("no {kind}: {:?}", rig.kinds()))
    }

    fn resume_run(rig: &Rig<GitTree>, run: &RunId) {
        let event = Event::new(
            run.clone(),
            0,
            rig.now(),
            format!("resume_run/{}", ulid::Ulid::new()),
            Origin::Cli,
            EventBody::ResumeRun,
        );
        rig.store.enqueue(run, &event, &[]).unwrap();
    }

    /// `MERGE_HEAD` and a planted lock: transient inside the window, `tree_refused` when it closes, the remedy in
    /// status, and a resume once it is gone prepares.
    #[test]
    fn a_planted_lock_refuses_inside_the_window_parks_with_the_remedy_and_a_resume_prepares() {
        for plant in [".git/index.lock", ".git/MERGE_HEAD"] {
            let temp = tempfile::tempdir().unwrap();
            let dir = real_path(&temp);
            let mut rig = git_rig(&dir);
            rig.policy.tree_window = Secs(60);
            let v2 = clone_directory(&dir);
            std::fs::write(v2.join(plant), b"x").unwrap();
            let run = rig.commission("work").run;
            let check = run.clone();
            rig.run_until(3_000, move |rig| rig.has(&check, "tree_refused"));
            let status = bridge::status(&rig.bridge, &run, rig.now())
                .unwrap()
                .unwrap();
            let refusal = status.local.unwrap().tree.refusal.expect("in status");
            assert!(refusal.text.contains("remedy"), "{}", refusal.text);
            assert!(
                refusal.text.contains(plant.trim_start_matches(".git/")) || plant.contains("MERGE")
            );
            std::fs::remove_file(v2.join(plant)).unwrap();
            resume_run(&rig, &run);
            let check = run.clone();
            rig.run_until(3_000, move |rig| rig.has(&check, "tree_prepared"));
            assert_eq!(run_git(&v2, &["status", "--porcelain"]), "");
        }
    }

    /// Three fetches fail, the fourth does not: the run never parks.
    #[test]
    fn a_fetch_failing_three_times_and_then_succeeding_prepares_without_parking() {
        let temp = tempfile::tempdir().unwrap();
        let dir = real_path(&temp);
        hermetic();
        let count = dir.join("fetches");
        let git = dir.join("fake-git");
        std::fs::write(
            &git,
            format!(
                "#!/bin/sh\ncase \"$*\" in *\"fetch -q\"*)\n  echo x >> {count}\n  if [ \"$(wc -l < {count})\" -le 3 ]; then echo 'the network is down' >&2; exit 1; fi;;\nesac\nexec {real} \"$@\"\n",
                count = count.display(),
                real = which_git(),
            ),
        )
        .unwrap();
        make_executable(&git);
        let mut rig = Rig::git_tree_using(
            &dir,
            CLOCK_START,
            Box::new(silent),
            GitBounds::default(),
            &git,
        );
        let run = rig.commission("work").run;
        let check = run.clone();
        rig.run_until(3_000, move |rig| rig.has(&check, "tree_prepared"));
        assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(), 4);
        assert!(!rig.has(&run, "tree_refused"));
        assert!(!rig.has(&run, "parked"));
        assert_eq!(
            rig.store.get(&format!("tree_refusal/{run}")).unwrap(),
            None,
            "the refusal went with the success"
        );
    }

    /// §12.11's loop, TQ #14: a real `GitTree` under a real `Host::tick`, the remote a listener that accepts and never
    /// answers, the bound injected. The tick keeps its budget, the other run advances, the operation ends
    /// `bound_exceeded`, its process group is empty and no lock stays.
    #[test]
    fn a_fetch_against_a_listener_that_never_answers_leaves_the_tick_in_budget_and_ends_at_its_bound(
    ) {
        let temp = tempfile::tempdir().unwrap();
        let dir = real_path(&temp);
        hermetic();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&accepted);
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming().flatten() {
                counted.fetch_add(1, Ordering::SeqCst);
                held.push(stream);
            }
        });
        let bounds = GitBounds {
            fetch: Duration::from_secs(40),
            ..GitBounds::default()
        };
        let mut rig = Rig::git_tree_bounded(&dir, CLOCK_START, Box::new(silent), bounds);
        let v2 = clone_directory(&dir);
        let silent_remote = format!("https://127.0.0.1:{port}/repo.git");
        run_git(
            &dir.join("root"),
            &["remote", "set-url", "origin", &silent_remote],
        );
        run_git(&v2, &["remote", "set-url", "origin", &silent_remote]);

        let first = rig.commission("first").run;
        rig.clock.advance(1_000);
        let second = rig.commission("second").run;
        let mut slowest = Duration::ZERO;
        let mut tick = |rig: &mut Rig<GitTree>, advance: i64| {
            let started = Instant::now();
            rig.host.tick(rig.now());
            slowest = slowest.max(started.elapsed());
            rig.clock.advance(advance);
            std::thread::sleep(Duration::from_millis(1));
        };
        // The clock stands until the remote really has the connection: the bound then passes on a fetch in its wait.
        let limit = Instant::now() + Duration::from_secs(60);
        while accepted.load(Ordering::SeqCst) == 0 {
            assert!(
                Instant::now() < limit,
                "the fetch never connected: {:?}",
                rig.kinds()
            );
            tick(&mut rig, 0);
        }
        let entry = rig.tree.ledger().read().expect("the fetch is recorded");
        assert_eq!(entry.step, "fetch");
        let pid = entry.pid.expect("with its pid");
        assert!(
            group_members(pid)
                .iter()
                .any(|member| member.contains("remote-https")),
            "the transport helper is there: {:?}",
            group_members(pid)
        );
        let key = format!("tree_refusal/{first}");
        let limit = Instant::now() + Duration::from_secs(60);
        while rig.store.get(&key).unwrap().is_none() {
            assert!(Instant::now() < limit, "no refusal: {:?}", rig.kinds());
            tick(&mut rig, 500);
        }
        let stored: serde_json::Value =
            serde_json::from_slice(&rig.store.get(&key).unwrap().unwrap()).unwrap();
        assert_eq!(stored["reason"]["reason"], "bound_exceeded", "{stored}");
        until("the group is empty", || group_members(pid).is_empty());
        assert!(!v2.join(".git/index.lock").exists());
        assert!(
            !nexus_run_host::git_tree::tree_state(&dir.join("root"), &v2)
                .unwrap()
                .dirty
        );

        // Against a machine with other work: a tick that waited on the fetch would take its whole span.
        assert!(slowest < Duration::from_secs(5), "a tick took {slowest:?}");
        assert!(rig.has(&second, "root_opened"), "the other run advanced");
        assert!(rig.has(&second, "claim_busy"), "and waits behind the first");
        assert_eq!(
            rig.store.lease(&first).unwrap().unwrap().epoch,
            1,
            "the lease never moved"
        );

        // Nothing outlives the test: the retry the host started is cancelled.
        rig.tree.cancel();
        until("the retry ended", || {
            rig.tree.poll();
            !rig.tree.busy()
        });
    }

    /// The withdraw of §12.11, and what the next holder finds.
    #[test]
    fn a_withdrawn_runs_work_is_parked_before_the_claim_is_released_and_the_next_run_finds_nothing()
    {
        let temp = tempfile::tempdir().unwrap();
        let dir = real_path(&temp);
        hermetic();
        let mut rig = Rig::git_tree(&dir, CLOCK_START, scenario_named("withdraw"));
        let receipt = rig.commission("work");
        let run = receipt.run.clone();
        let v2 = rig.bridge.layout.v2_dir();
        let stray = v2.join("stray.txt");
        rig.run_until(3_000, move |_| stray.exists());
        bridge::withdraw(
            &rig.bridge,
            &receipt.root_thread,
            &Rig::requester(),
            "stop",
            rig.now(),
        )
        .unwrap();
        rig.run_to_finished(&run);
        let Some(ParkOutcome::Committed { branch, commit }) = outcome_of(&rig, &run) else {
            panic!("{:?}", rig.events(&run));
        };
        assert!(
            branch.starts_with(&format!("nxs/park/run-{run}/")),
            "{branch}"
        );
        assert!(position(&rig, &run, "tree_parked") < position(&rig, &run, "claim_released"));
        assert!(rig.store.get_claim_holder().is_none());
        let discharge = read_port_lines(&dir.join("messages.jsonl"))
            .into_iter()
            .find_map(|line| match line {
                PortLine::Post { key, body, .. } if key == format!("result/{run}") => Some(body),
                _ => None,
            })
            .expect("the discharge");
        assert!(discharge.contains(&branch), "{discharge}");

        rig.backend.set_scenario(Box::new(silent));
        let next = rig.commission("next").run;
        let check = next.clone();
        rig.run_until(3_000, move |rig| rig.has(&check, "tree_prepared"));
        let found = rig
            .events(&next)
            .iter()
            .find_map(|event| match &event.body {
                EventBody::TreePrepared { found, .. } => Some(found.clone()),
                _ => None,
            });
        assert_eq!(found, Some(None));
        assert!(!v2.join("stray.txt").exists());
        assert_eq!(run_git(&v2, &["status", "--porcelain"]), "");
        assert_eq!(
            run_git(&v2, &["rev-parse", "HEAD"]),
            run_git(&v2, &["rev-parse", "origin/main"])
        );
        assert!(run_git(&v2, &["show", "--name-only", "--format=", &commit]).contains("stray.txt"));
    }

    #[test]
    fn a_queued_run_withdrawn_beside_a_dirty_holder_leaves_the_holder_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let dir = real_path(&temp);
        hermetic();
        let mut rig = Rig::git_tree(&dir, CLOCK_START, scenario_named("withdraw"));
        let holder = rig.commission("holder");
        let v2 = rig.bridge.layout.v2_dir();
        let stray = v2.join("stray.txt");
        rig.run_until(3_000, move |_| stray.exists());
        let before = snapshot(&v2);
        let queued = rig.commission("queued");
        let check = queued.run.clone();
        rig.run_until(200, move |rig| rig.has(&check, "claim_busy"));
        bridge::withdraw(
            &rig.bridge,
            &queued.root_thread,
            &Rig::requester(),
            "stop",
            rig.now(),
        )
        .unwrap();
        rig.run_to_finished(&queued.run);
        assert_eq!(outcome_of(&rig, &queued.run), Some(ParkOutcome::NotHeld));
        assert_eq!(snapshot(&v2), before, "the holder's tree moved");
        assert_eq!(
            rig.store.get_claim_holder().as_deref(),
            Some(holder.run.as_str())
        );
        assert!(run_git(&v2, &["status", "--porcelain"]).contains("stray.txt"));
    }

    /// The refusal stands across a restart with the claim held, and removing the lock parks and releases.
    #[test]
    fn a_park_refused_by_a_lock_holds_the_claim_across_a_restart_and_parks_once_it_is_gone() {
        let temp = tempfile::tempdir().unwrap();
        let dir = real_path(&temp);
        hermetic();
        let mut rig = Rig::git_tree(&dir, CLOCK_START, scenario_named("withdraw"));
        let receipt = rig.commission("work");
        let run = receipt.run.clone();
        let v2 = rig.bridge.layout.v2_dir();
        let stray = v2.join("stray.txt");
        rig.run_until(3_000, move |_| stray.exists());
        let lock = v2.join(".git/index.lock");
        std::fs::write(&lock, b"").unwrap();
        bridge::withdraw(
            &rig.bridge,
            &receipt.root_thread,
            &Rig::requester(),
            "stop",
            rig.now(),
        )
        .unwrap();
        let key = format!("tree_refusal/{run}");
        rig.run_until(3_000, |rig| rig.store.get(&key).unwrap().is_some());

        let ledger = TreeLedger::new(&dir.join("agent-logs"), WORKSPACE);
        rig.restart_with(GitTree::new(
            &dir.join("root"),
            ledger,
            GitBounds::default(),
        ));
        for _ in 0..40 {
            rig.tick();
        }
        let status = bridge::status(&rig.bridge, &run, rig.now())
            .unwrap()
            .unwrap();
        assert_eq!(status.waiting_for.as_deref(), Some("the park"));
        let refusal = status
            .local
            .unwrap()
            .tree
            .refusal
            .expect("the refusal in status");
        assert_eq!(
            refusal.reason,
            TreeRefusal::Git {
                reason: GitReason::LockHeld
            }
        );
        assert!(refusal.text.contains("index.lock"), "{}", refusal.text);
        assert_eq!(rig.store.get_claim_holder().as_deref(), Some(run.as_str()));
        assert!(!rig.has(&run, "claim_released"));
        assert!(lock.exists(), "a lock nobody can answer for stays");

        std::fs::remove_file(&lock).unwrap();
        rig.run_to_finished(&run);
        assert!(matches!(
            outcome_of(&rig, &run),
            Some(ParkOutcome::Committed { .. })
        ));
        assert!(rig.store.get_claim_holder().is_none());
    }

    #[test]
    fn a_directory_moved_aside_during_finishing_is_missing_and_releases() {
        let temp = tempfile::tempdir().unwrap();
        let dir = real_path(&temp);
        let mut rig = git_rig(&dir);
        let receipt = rig.commission("work");
        let check = receipt.run.clone();
        rig.run_until(3_000, move |rig| rig.has(&check, "tree_prepared"));
        let v2 = rig.bridge.layout.v2_dir();
        std::fs::rename(&v2, dir.join("aside")).unwrap();
        bridge::withdraw(
            &rig.bridge,
            &receipt.root_thread,
            &Rig::requester(),
            "stop",
            rig.now(),
        )
        .unwrap();
        rig.run_to_finished(&receipt.run);
        assert_eq!(outcome_of(&rig, &receipt.run), Some(ParkOutcome::Missing));
        assert!(rig.store.get_claim_holder().is_none());
    }

    /// Refused at once and for good, then withdrawn: the identity check comes first and the repository keeps every
    /// byte, and the claim goes.
    #[test]
    fn a_foreign_directory_parks_the_run_at_once_and_a_withdrawal_releases_without_touching_it() {
        let temp = tempfile::tempdir().unwrap();
        let dir = real_path(&temp);
        let mut rig = git_rig(&dir);
        super::origin_with_main(&dir.join("other-origin"));
        run_git(&dir, &["clone", "-q", "other-origin", "root-v2"]);
        let v2 = rig.bridge.layout.v2_dir();
        std::fs::write(v2.join("theirs"), b"work").unwrap();
        let before = snapshot(&v2);
        let receipt = rig.commission("work");
        let check = receipt.run.clone();
        rig.run_until(3_000, move |rig| rig.has(&check, "tree_refused"));
        let reason = rig
            .events(&receipt.run)
            .iter()
            .find_map(|event| match &event.body {
                EventBody::TreeRefused { reason } => Some(reason.clone()),
                _ => None,
            });
        assert_eq!(reason, Some(TreeRefusal::ForeignDirectory));
        bridge::withdraw(
            &rig.bridge,
            &receipt.root_thread,
            &Rig::requester(),
            "stop",
            rig.now(),
        )
        .unwrap();
        rig.run_to_finished(&receipt.run);
        assert_eq!(outcome_of(&rig, &receipt.run), Some(ParkOutcome::NotHeld));
        assert_eq!(snapshot(&v2), before);
        assert!(rig.store.get_claim_holder().is_none());
    }

    /// A happy run over the real tree, for the record of what `status` fills under `local.tree`: H2 of the manufakt
    /// review.
    #[test]
    fn status_fills_every_field_of_the_tree_under_local() {
        let temp = tempfile::tempdir().unwrap();
        let dir = real_path(&temp);
        let mut rig = git_rig(&dir);
        let run = rig.commission("work").run;
        let check = run.clone();
        rig.run_until(3_000, move |rig| rig.has(&check, "tree_prepared"));
        let v2 = rig.bridge.layout.v2_dir();
        std::fs::write(v2.join("stray"), b"work").unwrap();
        let status = bridge::status(&rig.bridge, &run, rig.now())
            .unwrap()
            .unwrap();
        let wire = serde_json::to_value(&status).unwrap();
        assert!(
            wire.get("tree").is_none(),
            "RunStatus has no tree of its own: {wire}"
        );
        let tree = status.local.unwrap().tree;
        assert_eq!(tree.path, v2.display().to_string());
        let base = tree.base.expect("the base");
        assert_eq!(base.branch, "main");
        assert_eq!(tree.commit.as_deref(), Some(base.commit.as_str()));
        assert_eq!(tree.branch, None);
        assert_eq!(tree.dirty, Some(true));
        assert_eq!(tree.found, None);
        assert_eq!(tree.parked, None);
        assert_eq!(tree.refusal, None);
        assert_eq!(tree.in_flight, None);
    }
}
