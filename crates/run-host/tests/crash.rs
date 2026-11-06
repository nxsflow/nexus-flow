//! The crash harness (§12.4, epic DoD 7): abort the host at every declared boundary, at its occurrences, over
//! the happy, rework and withdraw scripts and once with a second crash during recovery, restart, and assert the
//! whole list. Nothing here measures time or CPU: a run is ticks, and every assertion reads files.

mod common;

use common::{git, Crashbox};
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

/// The whole list of epic DoD 7 (over `ScriptedTree`; the `GitTree` pass is ticket j6ba's).
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

/// Aborts at each occurrence, restarts, and asserts the list, on a few threads, because each case is its own
/// processes and its own directory. Returns the boundary kinds this script passed.
fn sweep(script: &str) -> Vec<String> {
    let counts = Crashbox::new(script).count_boundaries();
    let mut cases: Vec<(Boundary, u32)> = Vec::new();
    for boundary in Boundary::ALL {
        let total = counts.get(boundary.name()).copied().unwrap_or(0);
        cases.extend(occurrences(total).into_iter().map(|nth| (*boundary, nth)));
    }
    let cases = std::sync::Mutex::new(cases);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| loop {
                let case = cases.lock().unwrap_or_else(|e| e.into_inner()).pop();
                let Some((boundary, nth)) = case else {
                    return;
                };
                let box_ = Crashbox::new(script);
                let at = format!("{script} at {}:{nth}", boundary.name());
                box_.run_until_abort(boundary, nth);
                box_.restart_and_run_to_completion();
                assert_the_list(&box_, script, &at);
            });
        }
    });
    counts.into_keys().collect()
}

#[test]
fn the_happy_script_survives_an_abort_at_every_occurrence_of_every_boundary() {
    sweep("happy");
}

#[test]
fn the_rework_script_survives_an_abort_at_every_occurrence_of_every_boundary() {
    sweep("rework");
}

#[test]
fn the_withdraw_script_survives_an_abort_at_every_occurrence_of_every_boundary() {
    let passed = sweep("withdraw");
    assert!(
        passed.contains(&Boundary::BeforeSignal.name().to_owned()),
        "the withdraw script signals a live session"
    );
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
