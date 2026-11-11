//! What the reviews of the host and the working directory left to stand before the first real run (6j6v.tv58's
//! notes): a withdrawal has to end, nothing that is not this engine's is touched, and a slow park is not a refusal.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::tree::*;
use nexus_run_contracts::{ParkOutcome, RunId, TreeRefusal};
use nexus_run_host::git_tree::GitBounds;
use nexus_run_host::test_support::SimClock;
use nexus_run_host::{TreeResult, WorkTree};

fn fresh_run() -> RunId {
    RunId::new(ulid::Ulid::new())
}

fn parked(result: TreeResult) -> ParkOutcome {
    match result {
        TreeResult::Parked(outcome) => outcome,
        other => panic!("not parked: {other:?}"),
    }
}

fn refusal(result: &TreeResult) -> (&TreeRefusal, &str) {
    match result {
        TreeResult::Refused { reason, text } => (reason, text),
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn a_park_with_origin_head_unset_saves_dirty_work_on_a_detached_head() {
    let bench = Bench::new();
    let tree = bench.prepared();
    run_git(&bench.dir, &["remote", "set-head", "origin", "-d"]);
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();
    let run = fresh_run();

    let ParkOutcome::Committed { branch, commit } = parked(bench.park(&tree, &run)) else {
        panic!("not committed");
    };

    assert!(
        branch.starts_with(&format!("nxs/park/run-{run}/")),
        "{branch}"
    );
    assert!(run_git(&bench.dir, &["show", "--stat", &commit]).contains("stray.rs"));
}

#[test]
fn a_park_with_a_dangling_origin_head_saves_dirty_work_on_a_detached_head() {
    let bench = Bench::new();
    let tree = bench.prepared();
    run_git(
        &bench.dir,
        &["update-ref", "-d", "refs/remotes/origin/main"],
    );
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();

    let outcome = parked(bench.park(&tree, &fresh_run()));

    let ParkOutcome::Committed { commit, .. } = outcome else {
        panic!("not committed: {outcome:?}");
    };
    assert!(run_git(&bench.dir, &["show", "--stat", &commit]).contains("stray.rs"));
}

/// With no base known, nothing can say a clean detached HEAD holds nothing of its own, so it keeps a name.
#[test]
fn a_park_with_no_base_known_keeps_a_clean_detached_head_on_a_branch() {
    let bench = Bench::new();
    let tree = bench.prepared();
    let head = bench.head();
    run_git(&bench.dir, &["remote", "set-head", "origin", "-d"]);

    let ParkOutcome::Committed { commit, .. } = parked(bench.park(&tree, &fresh_run())) else {
        panic!("not committed");
    };

    assert_eq!(commit, head);
}

#[test]
fn a_park_with_no_base_known_commits_on_the_branch_head_stands_on() {
    let bench = Bench::new();
    let tree = bench.prepared();
    run_git(&bench.dir, &["checkout", "-q", "-b", "feat/x"]);
    run_git(&bench.dir, &["remote", "set-head", "origin", "-d"]);
    std::fs::write(bench.dir.join("stray.rs"), b"work").unwrap();

    let outcome = parked(bench.park(&tree, &fresh_run()));

    assert!(
        matches!(&outcome, ParkOutcome::OnOwnBranch { branch, .. } if branch == "feat/x"),
        "{outcome:?}"
    );
    assert_eq!(run_git(&bench.dir, &["status", "--porcelain"]), "");
}

fn dir_is_a_link_to_the_root(bench: &Bench) {
    std::os::unix::fs::symlink(&bench.root, &bench.dir).unwrap();
    std::fs::write(bench.root.join("the-old-engines-work.rs"), b"work").unwrap();
}

#[test]
fn a_directory_that_resolves_to_the_root_is_foreign_at_prepare_and_the_root_keeps_its_head() {
    let bench = Bench::new();
    dir_is_a_link_to_the_root(&bench);
    let head = run_git(&bench.root, &["symbolic-ref", "HEAD"]);
    let tree = bench.tree();

    let result = bench.prepare(&tree, &fresh_run());

    assert_eq!(refusal(&result).0, &TreeRefusal::ForeignDirectory);
    assert_eq!(run_git(&bench.root, &["symbolic-ref", "HEAD"]), head);
    assert!(bench.root.join("the-old-engines-work.rs").exists());
    assert!(run_git(&bench.root, &["status", "--porcelain"]).contains("the-old-engines-work.rs"));
}

#[test]
fn a_directory_that_resolves_to_the_root_is_not_held_at_park_and_nothing_is_committed() {
    let bench = Bench::new();
    dir_is_a_link_to_the_root(&bench);
    let before = run_git(&bench.root, &["rev-parse", "HEAD"]);
    let tree = bench.tree();

    assert_eq!(
        parked(bench.park(&tree, &fresh_run())),
        ParkOutcome::NotHeld
    );

    assert_eq!(run_git(&bench.root, &["rev-parse", "HEAD"]), before);
    assert!(run_git(&bench.root, &["status", "--porcelain"]).contains("the-old-engines-work.rs"));
}

/// A person's own directory that merely starts like a clone's sibling is a person's.
#[test]
fn only_a_sibling_whose_suffix_is_a_ulid_is_removed() {
    let bench = Bench::new();
    let backup = bench.dir.with_file_name("root-v2.cloning-backup");
    std::fs::create_dir_all(&backup).unwrap();
    std::fs::write(backup.join("mine"), b"keep").unwrap();
    let left = bench
        .dir
        .with_file_name(format!("root-v2.cloning-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(left.join("half")).unwrap();

    bench.prepared();

    assert!(
        backup.join("mine").exists(),
        "a person's directory was deleted"
    );
    assert!(!left.exists(), "what a crash left stayed");
}

/// `step` hangs until `release` exists, past the common bound of the simulated clock: nothing about the wait is real
/// time. A killed step answers within the pause, and a correct one never answers early.
fn a_slow_park_step_survives_the_common_bound(step: &str) {
    let bench = Bench::new();
    bench.prepared();
    std::fs::write(bench.dir.join("big.rs"), b"work").unwrap();
    let started = bench.dir.with_file_name("started");
    let release = bench.dir.with_file_name("release");
    let git = bench.fake_git(&format!(
        r#"case "$*" in *"{step}"*) : > "{0}"; while [ ! -e "{1}" ]; do sleep 0.02; done;; esac"#,
        started.display(),
        release.display()
    ));
    let clock = Arc::new(SimClock::at(1_000_000));
    let tree = bench.tree_on_clock(&git, GitBounds::default(), clock.clone());
    tree.begin(bench.park_op(&fresh_run())).unwrap();
    until("the step started", || started.exists());

    clock.advance(GitBounds::default().other.as_millis() as i64 + 1_000);
    let pause = Instant::now() + Duration::from_millis(500);
    while Instant::now() < pause {
        assert!(
            tree.poll().is_none(),
            "the step was ended at the common bound"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    std::fs::write(&release, b"").unwrap();
    assert!(matches!(
        finished(&tree),
        TreeResult::Parked(ParkOutcome::Committed { .. })
    ));
}

#[test]
fn a_parks_add_outlives_the_common_bound() {
    a_slow_park_step_survives_the_common_bound("add -A");
}

#[test]
fn a_parks_commit_outlives_the_common_bound() {
    a_slow_park_step_survives_the_common_bound("commit -q");
}

/// git is gone, a helper in its own group holds the pipe and git's lock stays behind: ended, and the lock answered for.
#[test]
fn a_helper_of_the_childs_group_holding_the_pipe_is_ended_and_its_lock_cleared() {
    let bench = Bench::new();
    bench.prepared();
    let pid_file = bench.dir.with_file_name("helper-pid");
    let git = bench.fake_git(&format!(
        r#"case "$*" in *"fetch -q"*) : > .git/index.lock; sleep 600 & echo $! > "{0}"; exit 0;; esac"#,
        pid_file.display()
    ));
    let tree = bench.tree_with_git(&git, GitBounds::default());

    let result = bench.prepare(&tree, &fresh_run());

    assert!(refusal(&result).1.contains("output"), "{result:?}");
    let helper: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let alive = pid_alive(helper);
    let _ = std::process::Command::new("kill")
        .args(["-KILL", &helper.to_string()])
        .status();
    assert!(!alive, "the helper stayed in the group");
    assert!(
        !bench.dir.join(".git/index.lock").exists(),
        "the lock stayed"
    );
}

/// The status read has nobody's ledger behind it, and the same rule: a helper of its group holding the pipe does not
/// stay behind it.
#[test]
fn a_helper_holding_the_pipe_of_a_status_read_is_ended() {
    let bench = Bench::new();
    bench.prepared();
    let pid_file = bench.dir.with_file_name("helper-pid");
    let git = bench.fake_git(&format!(
        r#"case "$*" in *"status --porcelain"*) sleep 600 & echo $! > "{0}"; exit 0;; esac"#,
        pid_file.display()
    ));

    let read = nexus_run_host::git_tree::tree_state_with_git(&git, &bench.root, &bench.dir);

    assert!(read.is_err(), "{read:?}");
    let helper: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let alive = pid_alive(helper);
    let _ = std::process::Command::new("kill")
        .args(["-KILL", &helper.to_string()])
        .status();
    assert!(!alive, "the helper stayed behind the status read");
}

#[test]
fn the_remedy_of_a_bisect_says_how_a_bisect_ends() {
    let bench = Bench::new();
    let tree = bench.prepared();
    std::fs::write(bench.dir.join(".git/BISECT_LOG"), b"x").unwrap();

    let result = bench.prepare(&tree, &fresh_run());

    let text = refusal(&result).1;
    assert!(text.contains("`git bisect reset`"), "{text}");
    assert!(!text.contains("--abort"), "{text}");
    assert_eq!(
        refusal(&result).0,
        &TreeRefusal::MidSequence {
            what: nexus_run_contracts::SequenceKind::Bisect
        }
    );
}

/// PROBE (6j6v.zame), removed before hand-over: how often a program written in this process, or by `cp`, cannot be
/// run while other threads fork; and what the status read answers through such a program.
#[test]
fn probe_text_file_busy() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let bench = Bench::new();
    bench.prepared();
    let stop = Arc::new(AtomicBool::new(false));
    let forkers: Vec<_> = (0..6)
        .map(|_| {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let _ = std::process::Command::new("true").status();
                }
            })
        })
        .collect();
    let body = "#!/bin/sh\nexit 0\n";
    let (mut in_process, mut by_cp, mut reads_not_started) = (0, 0, Vec::new());
    for i in 0..300 {
        let here = bench.dir.with_file_name(format!("probe-here-{i}"));
        std::fs::write(&here, body).unwrap();
        make_executable(&here);
        if let Err(error) = std::process::Command::new(&here).status() {
            if error.raw_os_error() == Some(26) {
                in_process += 1;
            }
        }
        let copied = bench.dir.with_file_name(format!("probe-cp-{i}"));
        write_program(&copied, body);
        if let Err(error) = std::process::Command::new(&copied).status() {
            if error.raw_os_error() == Some(26) {
                by_cp += 1;
            }
        }
        let git = bench.dir.with_file_name(format!("probe-git-{i}"));
        std::fs::write(&git, body).unwrap();
        make_executable(&git);
        if let Err(error) =
            nexus_run_host::git_tree::tree_state_with_git(&git, &bench.root, &bench.dir)
        {
            reads_not_started.push(error.what);
        }
    }
    stop.store(true, Ordering::SeqCst);
    for forker in forkers {
        forker.join().unwrap();
    }
    reads_not_started.dedup();
    panic!(
        "probe-result in_process_busy={in_process}/300 cp_busy={by_cp}/300 status_read_errors={reads_not_started:?}"
    );
}
