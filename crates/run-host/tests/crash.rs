//! The crash harness (§12.4, epic DoD 7): abort the host at every declared boundary, at a sample of its occurrences,
//! over the happy, rework and withdraw scripts and once with a second crash during recovery, restart, and assert the
//! whole list, over `ScriptedTree` and once more over the real `GitTree` (§12.11). Nothing here measures time or CPU:
//! a run is ticks, and every assertion reads files.

mod common;

use common::tree::{pid_alive, until};
use common::{git, Crashbox};
use nexus_run_contracts::{EventBody, ParkOutcome};
use nexus_run_host::test_support::PortLine;
use nexus_run_host::Boundary;

#[test]
fn abort_after_claim_cas_before_claim_taken_never_queues_the_run_behind_itself() {
    let box_ = Crashbox::new("happy");
    assert!(box_.run_until_abort(Boundary::AfterClaimCasBeforeClaimTaken, 1));
    let log = box_.restart_and_run_to_completion();
    assert_eq!(log.count("claim_taken"), 1);
    assert!(
        !log.events()
            .iter()
            .any(|e| e.key.starts_with("claim_busy/")),
        "queued behind itself"
    );
    assert_eq!(log.count("delivered"), 1);
    assert!(box_
        .timeline()
        .no_two_sessions_alive_for_one(("build", "coder")));
    assert!(box_.store().claim_absent());
}

/// The whole list of epic DoD 7.
fn assert_the_list(box_: &Crashbox, script: &str, at: &str) {
    let log = box_.log();
    let want = if script == "withdraw" {
        "withdrawn"
    } else {
        "delivered"
    };
    assert_eq!(
        log.finished_outcome().as_deref(),
        Some(want),
        "{at}: the run finished as the script says"
    );
    for (key, started) in log.started_per_session() {
        assert_eq!(started, 1, "{at}: {key} has one `started`");
    }
    let timeline = box_.timeline();
    assert_eq!(
        timeline.first_overlap(),
        None,
        "{at}: two sessions of one seat"
    );
    assert!(
        !log.events().iter().any(|event| {
            let run = box_.run_id();
            matches!(&event.body, nexus_run_contracts::EventBody::ClaimBusy { holder } if *holder == run)
        }),
        "{at}: queued behind itself"
    );
    assert_eq!(log.count("claim_released"), 1, "{at}: released once");
    let store = box_.store();
    assert!(store.claim_absent(), "{at}: the claim outlived the run");
    assert_eq!(store.timers_armed(), 0, "{at}: a timer outlived the run");

    let lines = box_.messages();
    let posts_with = |key: &str| {
        lines
            .iter()
            .filter(|line| matches!(line, PortLine::Post { key: k, .. } if k == key))
            .count()
    };
    let run = box_.run_id();
    assert_eq!(
        posts_with(&format!("root/{run}")),
        1,
        "{at}: one root message"
    );
    assert_eq!(
        posts_with(&format!("result/{run}")),
        1,
        "{at}: one delivered result"
    );
    let mut threads: Vec<&str> = lines
        .iter()
        .filter_map(|line| match line {
            PortLine::Open { thread, .. } => Some(thread.as_str()),
            _ => None,
        })
        .collect();
    threads.sort_unstable();
    threads.dedup();
    let sessions = timeline.spawns();
    for key in &sessions {
        assert_eq!(
            posts_with(&format!("turn/{key}")),
            1,
            "{at}: one turn for {key}"
        );
    }
    assert!(
        threads.len() > sessions.len(),
        "{at}: a thread for the root and one per session"
    );
    for key in &sessions {
        assert!(
            timeline.signals(key).len() <= 1,
            "{at}: {key} was signalled {:?}, and these sessions end on the first",
            timeline.signals(key)
        );
    }
    assert_the_tree(box_, script, at);
    let next = box_.run_next_to_prepared();
    let prepared = next
        .events()
        .iter()
        .find_map(|event| match &event.body {
            nexus_run_contracts::EventBody::TreePrepared { base, found } => {
                Some((base.clone(), found.clone()))
            }
            _ => None,
        })
        .expect("the next run prepared");
    assert_eq!(prepared.1, None, "{at}: the next run found a dirty tree");
    assert_eq!(
        git(&box_.v2_dir(), &["rev-parse", "HEAD"]),
        prepared.0.commit,
        "{at}: the next run's tree is at its base"
    );
}

/// The tree clean, HEAD where `tree_parked` says, and a withdrawn file on exactly one park branch.
fn assert_the_tree(box_: &Crashbox, script: &str, at: &str) {
    let dir = box_.v2_dir();
    assert_eq!(
        git(&dir, &["status", "--porcelain"]),
        "",
        "{at}: the tree is clean"
    );
    let parked = box_
        .log()
        .events()
        .iter()
        .find_map(|event| match &event.body {
            nexus_run_contracts::EventBody::TreeParked { outcome } => Some(outcome.clone()),
            _ => None,
        })
        .expect("the park is recorded");
    if script == "withdraw" {
        let nexus_run_contracts::ParkOutcome::Committed { branch, .. } = parked else {
            panic!("{at}: the withdrawn work was committed");
        };
        assert_eq!(
            git(&dir, &["symbolic-ref", "--short", "HEAD"]),
            branch,
            "{at}"
        );
        let branches = git(&dir, &["branch", "--list", "nxs/park/*"]);
        assert_eq!(
            branches.lines().count(),
            1,
            "{at}: exactly one park branch: {branches}"
        );
        let on_it = git(&dir, &["show", "--name-only", "--format=", &branch]);
        assert!(on_it.contains("stray.txt"), "{at}: the file is on it");
    }
    assert!(
        !std::fs::read_dir(dir.parent().expect("a parent"))
            .expect("the parent reads")
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().contains(".cloning-")),
        "{at}: a clone sibling was left behind"
    );
}

/// How often a clean run passes each boundary, per kind: every kind, and its occurrences up to a sample of
/// six, first and last among them. A kind a script never passes is named, and `Boundary::ALL` is checked
/// across the scripts below.
fn occurrences(total: u32) -> Vec<u32> {
    const SAMPLE: u32 = 6;
    if total <= SAMPLE {
        return (1..=total).collect();
    }
    (0..SAMPLE)
        .map(|i| 1 + i * (total - 1) / (SAMPLE - 1))
        .collect()
}

/// One sweep at a time, whatever libtest runs beside it: every case is several processes, and eight sweeps at once took
/// a machine to a load of 277 (and its timing with it).
static ONE_SWEEP: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs each case on a few threads, because each is its own processes and its own directory, and fails with EVERY
/// case's own message: a sweep that went red once with its message lost is why this collects them (7mht).
fn in_parallel<C: Send>(cases: Vec<C>, run: impl Fn(C) + Sync) {
    let _one_sweep = ONE_SWEEP.lock().unwrap_or_else(|e| e.into_inner());
    let cases = std::sync::Mutex::new(cases);
    let failures = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| loop {
                let case = cases.lock().unwrap_or_else(|e| e.into_inner()).pop();
                let Some(case) = case else {
                    return;
                };
                let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(case)));
                if let Err(panic) = failed {
                    let message = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                        .unwrap_or_else(|| "a panic with no text".to_owned());
                    failures
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(message);
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap_or_else(|e| e.into_inner());
    assert!(
        failures.is_empty(),
        "{} cases failed:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}

/// Aborts at each sampled occurrence, restarts, and asserts the list. Returns the boundary kinds this script passed.
fn sweep_over(
    script: &str,
    new_box: fn(&str) -> Crashbox,
    check: fn(&Crashbox, &str),
) -> Vec<String> {
    let counts = new_box(script).count_boundaries();
    let mut cases: Vec<(Boundary, u32)> = Vec::new();
    for boundary in Boundary::ALL {
        let total = counts.get(boundary.name()).copied().unwrap_or(0);
        cases.extend(occurrences(total).into_iter().map(|nth| (*boundary, nth)));
    }
    in_parallel(cases, |(boundary, nth)| {
        let box_ = new_box(script);
        let at = format!("{script} at {}:{nth}", boundary.name());
        box_.run_until_abort(boundary, nth);
        box_.restart_and_run_to_completion();
        assert_the_list(&box_, script, &at);
        check(&box_, &at);
    });
    counts.into_keys().collect()
}

fn sweep(script: &str) -> Vec<String> {
    sweep_over(script, Crashbox::new, |_, _| {})
}

#[test]
fn the_happy_script_survives_an_abort_at_a_sample_of_occurrences_of_every_boundary() {
    sweep("happy");
}

#[test]
fn the_rework_script_survives_an_abort_at_a_sample_of_occurrences_of_every_boundary() {
    sweep("rework");
}

#[test]
fn the_withdraw_script_survives_an_abort_at_a_sample_of_occurrences_of_every_boundary() {
    let passed = sweep("withdraw");
    assert!(
        passed.contains(&Boundary::BeforeSignal.name().to_owned()),
        "the withdraw script signals a live session"
    );
}

// ---- the pass over the real GitTree (§12.11) ------------------------------------------------------------------------

/// "A git child of this engine never outlives the engine": when a restarted host made its first call, no git of an
/// earlier process was alive.
fn no_git_outlived_its_host(box_: &Crashbox, at: &str) {
    assert_eq!(box_.git_overlaps(), "", "{at}: a git outlived its host");
    assert!(
        !box_.git_calls().is_empty(),
        "{at}: the watch saw no git, so it proves nothing"
    );
}

/// The owed notice goes out once, whichever delivery the abort fell between.
fn one_notice_for_committed_work(box_: &Crashbox, at: &str) {
    no_git_outlived_its_host(box_, at);
    let run = box_.run_id();
    let log = box_.log();
    let parked = log
        .events()
        .iter()
        .find_map(|event| match &event.body {
            EventBody::TreeParked { outcome } => Some(outcome.clone()),
            _ => None,
        })
        .expect("the park is recorded");
    assert!(
        matches!(parked, ParkOutcome::Committed { .. }),
        "{at}: {parked:?}"
    );
    let notices = box_
        .messages()
        .iter()
        .filter(|line| matches!(line, PortLine::Post { key, .. } if *key == format!("park/{run}")))
        .count();
    assert_eq!(notices, 1, "{at}: the park notice");
}

/// Exactly the one rescue, named in `tree_prepared`, kept across every abort, and the file on its branch.
fn the_rescue_is_found_once(box_: &Crashbox, at: &str) {
    no_git_outlived_its_host(box_, at);
    let run = box_.run_id();
    let found = box_
        .log()
        .events()
        .iter()
        .find_map(|event| match &event.body {
            EventBody::TreePrepared { found, .. } => Some(found.clone()),
            _ => None,
        })
        .expect("prepared");
    let Some(ParkOutcome::Committed { branch, commit }) = found else {
        panic!("{at}: the predecessor's file was not found: {found:?}");
    };
    assert!(
        branch.starts_with(&format!("nxs/park/run-{run}/")),
        "{at}: {branch}"
    );
    let dir = box_.v2_dir();
    assert_eq!(
        git(&dir, &["branch", "--list", "nxs/park/*"])
            .lines()
            .count(),
        1,
        "{at}: exactly one park branch"
    );
    assert!(
        git(&dir, &["show", "--name-only", "--format=", &commit]).contains("stray.txt"),
        "{at}: the file is on it"
    );
}

#[test]
fn over_git_tree_the_happy_script_survives_an_abort_at_a_sample_of_occurrences_of_every_boundary() {
    sweep_over("happy", Crashbox::over_git_tree, no_git_outlived_its_host);
}

#[test]
fn over_git_tree_the_withdraw_script_survives_an_abort_at_a_sample_of_occurrences_of_every_boundary(
) {
    sweep_over(
        "withdraw",
        Crashbox::over_git_tree,
        no_git_outlived_its_host,
    );
}

#[test]
fn over_git_tree_the_rework_script_survives_an_abort_at_a_sample_of_occurrences_of_every_boundary()
{
    sweep_over("rework", Crashbox::over_git_tree, no_git_outlived_its_host);
}

#[test]
fn over_git_tree_the_dirty_at_prepare_script_keeps_found_across_an_abort_at_a_sample_of_every_boundary(
) {
    sweep_over(
        "dirty_at_prepare",
        Crashbox::over_git_tree,
        the_rescue_is_found_once,
    );
}

#[test]
fn over_git_tree_the_park_notice_goes_out_once_across_an_abort_at_a_sample_of_every_boundary() {
    sweep_over(
        "leaves_work",
        Crashbox::over_git_tree,
        one_notice_for_committed_work,
    );
}

/// A run that left nothing owes no notice: `Clean` is not `Committed` with an empty commit.
#[test]
fn over_git_tree_a_run_that_left_nothing_owes_no_park_notice() {
    let box_ = Crashbox::over_git_tree("happy");
    box_.restart_and_run_to_completion();
    let run = box_.run_id();
    let parked = box_
        .log()
        .events()
        .iter()
        .find_map(|event| match &event.body {
            EventBody::TreeParked { outcome } => Some(outcome.clone()),
            _ => None,
        });
    assert_eq!(parked, Some(ParkOutcome::Clean { branch: None }));
    assert!(
        !box_.messages().iter().any(
            |line| matches!(line, PortLine::Post { key, .. } if *key == format!("park/{run}"))
        ),
        "a notice for nothing"
    );
}

/// The worker dies between the spawn and the byte: the child exits without git. The same box with no abort does run
/// git, so the watch is shown to see it.
#[test]
fn a_child_whose_worker_died_before_the_byte_exits_without_running_git() {
    let box_ = Crashbox::over_git_tree("happy");
    assert!(box_.run_until_abort(Boundary::AfterGitSpawnBeforeRecord, 1));
    let record = box_
        .dir()
        .join("agent-logs/v2/tree")
        .join(format!("{}.child", nexus_run_host::test_support::WORKSPACE));
    // The child writes this as its first act, which the abort does not have to have waited for.
    let mut pid = None;
    until("the child recorded itself", || {
        pid = std::fs::read_to_string(&record)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok());
        pid.is_some()
    });
    let pid = pid.expect("a pid");
    until("the child is gone", || !pid_alive(pid));
    assert!(
        !box_.git_calls().iter().any(|call| call.contains("get-url")),
        "git ran: {:?}",
        box_.git_calls()
    );

    box_.restart_and_run_to_completion();
    assert!(
        box_.git_calls().iter().any(|call| call.contains("get-url")),
        "the watch sees a git that does run"
    );
    assert_the_list(&box_, "happy", "after the gate");
}

/// Killed from outside with a git child holding `index.lock`: recovery kills the child before its own first git,
/// clears the lock that child made, and finishes. `add` and `commit`, a park's and a rescue's.
#[test]
fn an_abort_during_git_add_or_commit_kills_the_child_clears_its_lock_and_finishes() {
    for (script, step) in [
        ("withdraw", " add -A"),
        ("withdraw", " --no-gpg-sign -m"),
        ("dirty_at_prepare", " add -A"),
        ("dirty_at_prepare", " --no-gpg-sign -m"),
        ("happy", "fetch -q"),
    ] {
        let at = format!("{script} during{step}");
        let box_ = Crashbox::over_git_tree(script);
        let mut host = box_.spawn_blocking_on(step);
        until(&at, || box_.blocked_git().is_some());
        let blocked = box_.blocked_git().expect("blocked");
        let lock = box_.v2_dir().join(".git/index.lock");
        assert!(lock.exists(), "{at}: the child holds the lock");
        host.kill();
        assert!(
            pid_alive(blocked),
            "{at}: the child outlives the host until recovery"
        );

        box_.restart_and_run_to_completion();
        assert!(!pid_alive(blocked), "{at}: recovery killed the child");
        assert!(!lock.exists(), "{at}: the lock stayed");
        assert_eq!(box_.git_overlaps(), "", "{at}: a git outlived its host");
        assert_the_list(&box_, script, &at);
        if script == "dirty_at_prepare" {
            the_rescue_is_found_once(&box_, &at);
        }
    }
}

/// Every kind is passed by some script, so `Boundary::ALL` holds no boundary nobody can abort at.
#[test]
fn every_declared_boundary_is_reached_by_a_script() {
    let mut passed = Vec::new();
    for script in ["happy", "rework", "withdraw"] {
        passed.extend(Crashbox::new(script).count_boundaries().into_keys());
    }
    for boundary in Boundary::ALL {
        assert!(
            passed.contains(&boundary.name().to_owned()),
            "{} is declared and never reached",
            boundary.name()
        );
    }
}

/// A second crash during recovery: the restarted host dies again on its way, and the third start finishes.
#[test]
fn a_second_crash_during_recovery_changes_nothing() {
    let pairs = [
        (
            Boundary::AfterSpawnBeforeChildJson,
            Boundary::AfterEffectBeforeResult,
        ),
        (
            Boundary::AfterClaimCasBeforeClaimTaken,
            Boundary::AfterClaimCasBeforeClaimTaken,
        ),
        (Boundary::AfterPostBeforeDone, Boundary::AfterPostBeforeDone),
        (
            Boundary::AfterTreeResultBeforeInput,
            Boundary::BeforeTreeLedger,
        ),
        (Boundary::BeforeAppend, Boundary::AfterAppendBeforeEffect),
        (
            Boundary::BetweenFactsReadAndOffset,
            Boundary::BetweenFactsReadAndOffset,
        ),
    ];
    for script in ["happy", "withdraw"] {
        for (first, second) in pairs {
            let box_ = Crashbox::new(script);
            let at = format!("{script} at {} then {}", first.name(), second.name());
            box_.run_until_abort(first, 1);
            box_.restart_and_abort_again(second, 1);
            box_.restart_and_run_to_completion();
            assert_the_list(&box_, script, &at);
        }
    }
}

/// The finisher's half of "no duplicate PR" that is the engine's: a second spawn only after a confirmed end.
#[test]
fn an_abort_during_the_finisher_never_starts_a_second_finisher_beside_the_first() {
    let box_ = Crashbox::new("happy");
    let counts = box_.count_boundaries();
    // The last spawns of a run are the finisher's.
    let last = counts[Boundary::AfterSpawnBeforeChildJson.name()];
    let box_ = Crashbox::new("happy");
    assert!(box_.run_until_abort(Boundary::AfterSpawnBeforeChildJson, last));
    box_.restart_and_run_to_completion();
    let timeline = box_.timeline();
    assert!(timeline.spawns().iter().any(|key| key.contains("/finish/")));
    assert!(timeline.no_two_sessions_alive_for_one(("finish", "finisher")));
    assert!(box_.store().claim_absent());
}

/// The executor's own test (§12.4): every write it makes is behind a listed boundary. The journal holds the
/// boundaries a run passed and the writes it made, in order. Two writes with no boundary between them would be a
/// crash the harness can never put between them, so the only pairs allowed are named here, with why.
#[test]
fn every_write_the_executor_makes_is_behind_a_listed_boundary() {
    /// A drain's facts are each their own idempotent enqueue, keyed by offset, and the next session's come right
    /// after the last one's offset: nothing about one session's delivery depends on another's. A crash anywhere
    /// among them is a redelivery, which the key makes a `Duplicate`.
    const ALLOWED: [(&str, &str); 2] = [
        ("write enqueue", "write enqueue"),
        ("write offset", "write enqueue"),
    ];
    let mut together = std::collections::BTreeSet::new();
    for script in ["happy", "rework", "withdraw"] {
        let journal = Crashbox::new(script).journal();
        assert!(
            journal.iter().any(|line| line == "write post"),
            "{script}: {journal:?}"
        );
        for pair in journal.windows(2) {
            if pair[0].starts_with("write ") && pair[1].starts_with("write ") {
                together.insert((pair[0].clone(), pair[1].clone()));
            }
        }
    }
    let unexplained: Vec<_> = together
        .iter()
        .filter(|(first, second)| !ALLOWED.contains(&(first.as_str(), second.as_str())))
        .collect();
    assert_eq!(
        unexplained,
        Vec::<&(String, String)>::new(),
        "two writes with no boundary between them"
    );
}
