//! The timer index (§7): derived from the events of an `append`, in its transaction, so it never
//! disagrees with the log. A timer id has one row, whatever re-arms it.

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;

use nexus_run_contracts::{
    Decision, Event, EventBody, RunId, StoreError, StoreResult, Timer, TimerChange,
    TimerChangeKind, TimerPurpose, Timestamp,
};

use crate::db;

const ARMED: &str = "armed";
const CLEARED: &str = "cleared";
const FIRED: &str = "fired";

/// Applies what `events` imply, in order, and returns what changed. A clear or a firing that finds
/// nothing armed changes nothing, and neither does the firing of an instant the timer was re-armed away
/// from: it has its own input coming.
pub(crate) fn apply(
    tx: &Transaction<'_>,
    run: &RunId,
    events: &[Event],
) -> StoreResult<Vec<TimerChange>> {
    let mut changes = Vec::new();
    for event in events {
        match &event.body {
            EventBody::Decision(Decision::TimerSet {
                timer,
                fire_at,
                purpose,
            }) => {
                arm(tx, run, timer, *fire_at, *purpose)?;
                changes.push(change(timer, TimerChangeKind::Armed, *fire_at));
            }
            EventBody::Decision(Decision::TimerCleared { timer }) => {
                if let Some(fire_at) = clear(tx, run, timer)? {
                    changes.push(change(timer, TimerChangeKind::Cleared, fire_at));
                }
            }
            EventBody::TimerFired { timer, fire_at } => {
                if fire(tx, run, timer, *fire_at)? {
                    changes.push(change(timer, TimerChangeKind::Fired, *fire_at));
                }
            }
            EventBody::Decision(Decision::Finished { .. }) => {
                changes.extend(clear_all(tx, run)?);
            }
            _ => {}
        }
    }
    Ok(changes)
}

/// Armed rows due at `now`, oldest first. A purpose this version cannot read comes from a newer writer,
/// whose run this host may not step anyway (§13); skipping that row keeps it from stopping every other
/// run's timers, which this one query serves.
pub(crate) fn due(conn: &Connection, now: Timestamp) -> StoreResult<Vec<Timer>> {
    let mut statement = conn
        .prepare(
            "SELECT run, timer, fire_at, purpose FROM timers
             WHERE state = ?1 AND fire_at <= ?2 ORDER BY fire_at, run, timer",
        )
        .map_err(db)?;
    let rows = statement
        .query_map(params![ARMED, now.unix_millis()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(db)?;
    let mut due = Vec::new();
    for row in rows {
        let (run, timer, fire_at, purpose) = row.map_err(db)?;
        let Ok(purpose) = serde_json::from_value::<TimerPurpose>(Value::String(purpose)) else {
            continue;
        };
        due.push(Timer {
            run: crate::stored_run_id(&run)?,
            timer,
            fire_at: Timestamp::from_unix_millis(fire_at),
            purpose,
        });
    }
    Ok(due)
}

fn change(timer: &str, change: TimerChangeKind, fire_at: Timestamp) -> TimerChange {
    TimerChange {
        timer: timer.to_owned(),
        change,
        fire_at,
    }
}

fn arm(
    tx: &Transaction<'_>,
    run: &RunId,
    timer: &str,
    fire_at: Timestamp,
    purpose: TimerPurpose,
) -> StoreResult<()> {
    let purpose = serde_json::to_value(purpose)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| StoreError::Backend {
            what: "a timer purpose is a word".to_owned(),
        })?;
    tx.execute(
        "INSERT INTO timers (run, timer, fire_at, purpose, state) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (run, timer) DO UPDATE SET
             fire_at = excluded.fire_at, purpose = excluded.purpose, state = excluded.state",
        params![run.as_str(), timer, fire_at.unix_millis(), purpose, ARMED],
    )
    .map_err(db)?;
    Ok(())
}

fn clear(tx: &Transaction<'_>, run: &RunId, timer: &str) -> StoreResult<Option<Timestamp>> {
    let fire_at: Option<i64> = tx
        .query_row(
            "UPDATE timers SET state = ?1 WHERE run = ?2 AND timer = ?3 AND state = ?4
             RETURNING fire_at",
            params![CLEARED, run.as_str(), timer, ARMED],
            |row| row.get(0),
        )
        .optional()
        .map_err(db)?;
    Ok(fire_at.map(Timestamp::from_unix_millis))
}

fn fire(tx: &Transaction<'_>, run: &RunId, timer: &str, fire_at: Timestamp) -> StoreResult<bool> {
    let changed = tx
        .execute(
            "UPDATE timers SET state = ?1
             WHERE run = ?2 AND timer = ?3 AND state = ?4 AND fire_at = ?5",
            params![FIRED, run.as_str(), timer, ARMED, fire_at.unix_millis()],
        )
        .map_err(db)?;
    Ok(changed == 1)
}

/// `finished` is the end of the run, so nothing of it may stay armed: a timer firing into a run nobody
/// steps any more would come back as due on every tick.
fn clear_all(tx: &Transaction<'_>, run: &RunId) -> StoreResult<Vec<TimerChange>> {
    let mut statement = tx
        .prepare("SELECT timer FROM timers WHERE run = ?1 AND state = ?2 ORDER BY fire_at, timer")
        .map_err(db)?;
    let timers = statement
        .query_map(params![run.as_str(), ARMED], |row| row.get::<_, String>(0))
        .map_err(db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db)?;
    let mut changes = Vec::new();
    for timer in timers {
        if let Some(fire_at) = clear(tx, run, &timer)? {
            changes.push(change(&timer, TimerChangeKind::Cleared, fire_at));
        }
    }
    Ok(changes)
}
