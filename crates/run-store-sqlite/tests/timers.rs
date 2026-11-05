//! The timer index (§7, M4): derived from the events of an `append`, and what it changed comes back.

mod fixtures;

use nexus_run_contracts::{
    Cas, RunStore, Timer, TimerChange, TimerChangeKind, TimerPurpose, Timestamp,
};
use nexus_run_store_sqlite::SqliteRunStore;

fn change(timer: &str, change: TimerChangeKind, fire_at: Timestamp) -> TimerChange {
    TimerChange {
        timer: timer.into(),
        change,
        fire_at,
    }
}

fn store_with_run() -> (
    tempfile::TempDir,
    SqliteRunStore,
    nexus_run_contracts::RunId,
    u64,
) {
    let (dir, store) = fixtures::temp_store();
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&store, &run);
    (dir, store, run, fence)
}

fn timer_rows(dir: &tempfile::TempDir) -> i64 {
    let conn = rusqlite::Connection::open(dir.path().join("run.sqlite")).unwrap();
    conn.query_row("SELECT count(*) FROM timers", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn timer_set_arms_a_row_and_append_returns_the_change() {
    let (dir, store, run, fence) = store_with_run();
    let fire_at = fixtures::at(1000);
    let appended = store
        .append(
            &run,
            1,
            fence,
            &[fixtures::timer_set(&run, "d1", "quiet/a", fire_at)],
            &[],
            &[],
        )
        .unwrap();
    assert_eq!(
        appended.timers,
        [change("quiet/a", TimerChangeKind::Armed, fire_at)]
    );
    assert_eq!(timer_rows(&dir), 1);
}

#[test]
fn due_timers_returns_armed_rows_with_fire_at_at_or_before_now_across_runs() {
    let (_dir, store, run, fence) = store_with_run();
    let other = fixtures::other_run_id();
    let other_fence = fixtures::owned_run(&store, &other);
    store
        .append(
            &run,
            1,
            fence,
            &[
                fixtures::timer_set(&run, "d1", "later", fixtures::at(2000)),
                fixtures::timer_set(&run, "d2", "sooner", fixtures::at(1000)),
            ],
            &[],
            &[],
        )
        .unwrap();
    store
        .append(
            &other,
            1,
            other_fence,
            &[fixtures::timer_set(
                &other,
                "d1",
                "elsewhere",
                fixtures::at(1500),
            )],
            &[],
            &[],
        )
        .unwrap();

    assert!(store.due_timers(fixtures::at(999)).unwrap().is_empty());
    assert_eq!(
        store.due_timers(fixtures::at(1000)).unwrap(),
        [Timer {
            run: run.clone(),
            timer: "sooner".into(),
            fire_at: fixtures::at(1000),
            purpose: TimerPurpose::Quiet,
        }],
        "the boundary is included"
    );
    let due: Vec<_> = store
        .due_timers(fixtures::at(2000))
        .unwrap()
        .into_iter()
        .map(|timer| timer.timer)
        .collect();
    assert_eq!(due, ["sooner", "elsewhere", "later"], "oldest first");
}

#[test]
fn timer_set_on_an_armed_id_updates_its_row() {
    let (dir, store, run, fence) = store_with_run();
    store
        .append(
            &run,
            1,
            fence,
            &[fixtures::timer_set(
                &run,
                "d1",
                "quiet/a",
                fixtures::at(1000),
            )],
            &[],
            &[],
        )
        .unwrap();
    let appended = store
        .append(
            &run,
            2,
            fence,
            &[fixtures::timer_set(
                &run,
                "d2",
                "quiet/a",
                fixtures::at(5000),
            )],
            &[],
            &[],
        )
        .unwrap();
    assert_eq!(
        appended.timers,
        [change(
            "quiet/a",
            TimerChangeKind::Armed,
            fixtures::at(5000)
        )]
    );
    assert_eq!(timer_rows(&dir), 1, "re-arm is one row update");
    assert!(store.due_timers(fixtures::at(4999)).unwrap().is_empty());
    assert_eq!(store.due_timers(fixtures::at(5000)).unwrap().len(), 1);
}

#[test]
fn timer_cleared_cancels_and_reports_the_row_it_cancelled() {
    let (dir, store, run, fence) = store_with_run();
    let fire_at = fixtures::at(1000);
    store
        .append(
            &run,
            1,
            fence,
            &[fixtures::timer_set(&run, "d1", "quiet/a", fire_at)],
            &[],
            &[],
        )
        .unwrap();
    let appended = store
        .append(
            &run,
            2,
            fence,
            &[fixtures::timer_cleared(&run, "d2", "quiet/a")],
            &[],
            &[],
        )
        .unwrap();
    assert_eq!(
        appended.timers,
        [change("quiet/a", TimerChangeKind::Cleared, fire_at)]
    );
    assert!(store.due_timers(fixtures::at(9000)).unwrap().is_empty());
    assert_eq!(timer_rows(&dir), 1);

    let nothing_armed = store
        .append(
            &run,
            3,
            fence,
            &[
                fixtures::timer_cleared(&run, "d3", "quiet/a"),
                fixtures::timer_cleared(&run, "d4", "never/armed"),
            ],
            &[],
            &[],
        )
        .unwrap();
    assert!(nothing_armed.timers.is_empty(), "nothing changed");
}

#[test]
fn timer_fired_disarms_the_firing_it_names_and_not_a_later_re_arm() {
    let (_dir, store, run, fence) = store_with_run();
    let (first, second) = (fixtures::at(1000), fixtures::at(5000));
    store
        .append(
            &run,
            1,
            fence,
            &[
                fixtures::timer_set(&run, "d1", "quiet/a", first),
                fixtures::timer_set(&run, "d2", "quiet/a", second),
            ],
            &[],
            &[],
        )
        .unwrap();
    let stale = store
        .append(
            &run,
            3,
            fence,
            &[fixtures::timer_fired(&run, "quiet/a", first)],
            &[],
            &[],
        )
        .unwrap();
    assert!(stale.timers.is_empty(), "this firing was superseded");
    assert_eq!(store.due_timers(second).unwrap().len(), 1);

    let fired = store
        .append(
            &run,
            4,
            fence,
            &[fixtures::timer_fired(&run, "quiet/a", second)],
            &[],
            &[],
        )
        .unwrap();
    assert_eq!(
        fired.timers,
        [change("quiet/a", TimerChangeKind::Fired, second)]
    );
    assert!(store.due_timers(second).unwrap().is_empty());
}

#[test]
fn a_fired_timer_can_be_armed_again_on_its_one_row() {
    let (dir, store, run, fence) = store_with_run();
    let first = fixtures::at(1000);
    store
        .append(
            &run,
            1,
            fence,
            &[
                fixtures::timer_set(&run, "d1", "quiet/a", first),
                fixtures::timer_fired(&run, "quiet/a", first),
                fixtures::timer_set(&run, "d2", "quiet/a", fixtures::at(2000)),
            ],
            &[],
            &[],
        )
        .unwrap();
    assert_eq!(timer_rows(&dir), 1);
    assert_eq!(store.due_timers(fixtures::at(2000)).unwrap().len(), 1);
}

#[test]
fn a_rolled_back_append_changes_no_timer() {
    let (_dir, store, run, fence) = store_with_run();
    let list = [Cas {
        key: "thread/absent".into(),
        from: Some(b"x".to_vec()),
        to: None,
    }];
    let err = store
        .append(
            &run,
            1,
            fence,
            &[fixtures::timer_set(&run, "d1", "quiet/a", fixtures::t0())],
            &[],
            &list,
        )
        .unwrap_err();
    assert_eq!(err.kind(), "cas_failed");
    assert!(store.due_timers(fixtures::at(1)).unwrap().is_empty());
}

#[test]
fn finished_leaves_no_timer_armed_and_reports_what_it_cancelled() {
    let (_dir, store, run, fence) = store_with_run();
    let fire_at = fixtures::at(1000);
    store
        .append(
            &run,
            1,
            fence,
            &[fixtures::timer_set(&run, "d1", "quiet/a", fire_at)],
            &[],
            &[],
        )
        .unwrap();
    let appended = store
        .append(&run, 2, fence, &[fixtures::finished(&run, "end")], &[], &[])
        .unwrap();
    assert_eq!(
        appended.timers,
        [change("quiet/a", TimerChangeKind::Cleared, fire_at)]
    );
    assert!(store.due_timers(fixtures::at(9000)).unwrap().is_empty());
}

#[test]
fn a_row_with_a_purpose_this_version_does_not_know_does_not_stop_the_other_runs_timers() {
    let (dir, store, run, fence) = store_with_run();
    store
        .append(
            &run,
            1,
            fence,
            &[fixtures::timer_set(
                &run,
                "d1",
                "quiet/a",
                fixtures::at(1000),
            )],
            &[],
            &[],
        )
        .unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("run.sqlite")).unwrap();
    conn.execute(
        "INSERT INTO timers (run, timer, fire_at, purpose, state)
         VALUES (?1, 'from/the/future', 0, 'a_purpose_from_the_future', 'armed')",
        [fixtures::other_run_id().as_str()],
    )
    .unwrap();
    let due = store.due_timers(fixtures::at(1000)).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].timer, "quiet/a");
}
