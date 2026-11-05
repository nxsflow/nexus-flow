//! The two write paths (§7). Anyone puts inputs into a run's inbox with `enqueue`; only the owner moves
//! them into the log, with `append`. Each call is one `BEGIN IMMEDIATE` transaction, and a return
//! before its commit rolls everything back.
//!
//! `append` checks in the order [`StoreError::PRECEDENCE`] fixes. The errors outside that list sit where
//! they refuse the least: a malformed call is refused before the store is read; `UnknownRun` after the
//! fence, because a run without an owner is fenced whether it exists or not; `VersionTooOld` before the
//! keys, so a host too old for a run cannot consume that run's inbox rows through a `DuplicateKey`.

use std::collections::BTreeSet;

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use nexus_run_contracts::{
    Appended, Cas, Enqueued, Epoch, Event, EventBody, Key, RunId, Seq, StoreError, StoreResult,
};

use crate::lease::{check_epoch_keeps_rising, check_owner_value, current_epoch, is_owner_key};
use crate::{db, EFFECT_PREFIX};

pub(crate) fn append(
    conn: &mut Connection,
    run: &RunId,
    expected_seq: Seq,
    fence: Epoch,
    events: &[Event],
    consume: &[Key],
    cas: &[Cas],
) -> StoreResult<Appended> {
    check_call(run, events, cas)?;
    let tx = immediate(conn)?;
    if current_epoch(&tx, run.as_str())? != Some(fence) {
        return Err(StoreError::Fenced);
    }
    let row = read_run(&tx, run)?;
    let first_is_commissioned = matches!(
        events.first().map(|event| &event.body),
        Some(EventBody::Commissioned { .. })
    );
    if row.is_none() && !first_is_commissioned {
        return Err(StoreError::UnknownRun);
    }
    let last_seq = row.as_ref().map_or(0, |row| row.last_seq);
    if last_seq != expected_seq {
        return Err(StoreError::SeqMismatch);
    }
    let max_v = row.as_ref().map_or(0, |row| row.max_v);
    if events.iter().any(|event| event.v < max_v) {
        return Err(StoreError::VersionTooOld { max_v });
    }
    for event in events {
        if let Some(seq) = logged_seq(&tx, run, &event.key)? {
            // Already in the log, which is success for the caller. The rows go with this error, or the
            // host would read the same input again on every tick.
            delete_inbox_rows(&tx, run, consume)?;
            tx.commit().map_err(db)?;
            return Err(StoreError::DuplicateKey { seq });
        }
    }
    if row.is_none() {
        create_run(&tx, run, &events[0])?;
    }
    check_commissioned_position(events, last_seq, row.as_ref())?;
    let mut seq = last_seq;
    for event in events {
        seq += 1;
        insert_event(&tx, event, seq)?;
    }
    delete_inbox_rows(&tx, run, consume)?;
    let timers = crate::timers::apply(&tx, run, events)?;
    for item in cas {
        apply_cas(&tx, item)?;
    }
    let highest = events.iter().map(|event| event.v).max().unwrap_or(0);
    let finishes = events.iter().any(|event| is_finished(&event.body));
    tx.execute(
        "UPDATE runs SET last_seq = ?2, max_v = max(max_v, ?3), finished = finished OR ?4
         WHERE run = ?1",
        params![run.as_str(), seq, highest, finishes],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(Appended {
        last_seq: seq,
        timers,
    })
}

pub(crate) fn enqueue(
    conn: &mut Connection,
    run: &RunId,
    event: &Event,
    cas: &[Cas],
) -> StoreResult<Enqueued> {
    check_call(run, std::slice::from_ref(event), cas)?;
    let tx = immediate(conn)?;
    let waiting: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM inbox WHERE run = ?1 AND key = ?2",
            [run.as_str(), &event.key],
            |row| row.get(0),
        )
        .optional()
        .map_err(db)?;
    if waiting.is_some() || logged_seq(&tx, run, &event.key)?.is_some() {
        return Ok(Enqueued::Duplicate);
    }
    let commissioned = matches!(event.body, EventBody::Commissioned { .. });
    match (read_run(&tx, run)?, commissioned) {
        (None, true) => create_run(&tx, run, event)?,
        (None, false) => return Err(StoreError::UnknownRun),
        (Some(_), true) => {
            return Err(invalid(format!(
                "commissioned {} for a run that has one",
                event.key
            )))
        }
        (Some(_), false) => {}
    }
    for item in cas {
        apply_cas(&tx, item)?;
    }
    // An input has no position until it is appended.
    let mut waiting_input = event.clone();
    waiting_input.seq = 0;
    tx.execute(
        "INSERT INTO inbox (run, key, at, origin, body) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            run.as_str(),
            event.key,
            event.at.unix_millis(),
            event.origin.as_str(),
            wire(&waiting_input)?,
        ],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(Enqueued::Accepted)
}

pub(crate) fn immediate(conn: &mut Connection) -> StoreResult<Transaction<'_>> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db)
}

/// One compare-and-set. A failed comparison is `CasFailed`, and the caller's return rolls back.
pub(crate) fn apply_cas(tx: &Transaction<'_>, item: &Cas) -> StoreResult<()> {
    let owner_key = is_owner_key(&item.key);
    if owner_key {
        check_owner_value(&item.key, item.to.as_deref())?;
    }
    if item.key.starts_with(EFFECT_PREFIX) {
        if let Some(value) = &item.to {
            crate::lease::check_effect_write(tx, &item.key, value)?;
        }
    }
    let current: Option<Vec<u8>> = tx
        .query_row("SELECT value FROM kv WHERE key = ?1", [&item.key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(db)?;
    if current != item.from {
        return Err(StoreError::CasFailed {
            key: item.key.clone(),
        });
    }
    if owner_key {
        check_epoch_keeps_rising(&item.key, current.as_deref(), item.to.as_deref())?;
    }
    match &item.to {
        Some(value) => tx.execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![item.key, value],
        ),
        None => tx.execute("DELETE FROM kv WHERE key = ?1", [&item.key]),
    }
    .map_err(db)?;
    Ok(())
}

/// What a call is refused for whatever the store holds. An `owner/*` key changes only through
/// `compare_and_set`, so it never rides along in a list.
pub(crate) fn check_call(run: &RunId, events: &[Event], cas: &[Cas]) -> StoreResult<()> {
    let mut keys = BTreeSet::new();
    for event in events {
        check_event(run, event)?;
        if !keys.insert(event.key.as_str()) {
            return Err(invalid(format!("key {} twice in one call", event.key)));
        }
    }
    let mut cas_keys = BTreeSet::new();
    for item in cas {
        if is_owner_key(&item.key) {
            return Err(StoreError::Protected {
                key: item.key.clone(),
            });
        }
        if !cas_keys.insert(item.key.as_str()) {
            return Err(invalid(format!("CAS on {} twice in one call", item.key)));
        }
    }
    Ok(())
}

fn check_event(run: &RunId, event: &Event) -> StoreResult<()> {
    if &event.run != run {
        return Err(invalid(format!("event {} is of another run", event.key)));
    }
    let EventBody::Commissioned {
        run: named,
        workspace_id,
        tenant,
        root_thread,
        ..
    } = &event.body
    else {
        return Ok(());
    };
    if named != run {
        return Err(invalid("commissioned names another run".to_owned()));
    }
    if workspace_id.is_empty() || workspace_id.contains(['/', '\\']) {
        return Err(invalid(
            "commissioned.workspace_id is the opaque id, never a path".to_owned(),
        ));
    }
    if tenant.is_empty() || tenant.contains(['/', '\\']) {
        return Err(invalid(
            "commissioned.tenant is an opaque id, never a path".to_owned(),
        ));
    }
    let id_form = root_thread
        .strip_prefix("m-")
        .and_then(|ulid| RunId::parse(&format!("r-{ulid}")));
    if id_form.is_none() {
        return Err(invalid(
            "commissioned.root_thread is a message id".to_owned(),
        ));
    }
    Ok(())
}

/// `commissioned` opens a log, so it is its first event, and `workspace_id` and `tenant` never change
/// after it.
fn check_commissioned_position(
    events: &[Event],
    last_seq: Seq,
    row: Option<&RunRowState>,
) -> StoreResult<()> {
    for (index, event) in events.iter().enumerate() {
        let EventBody::Commissioned {
            workspace_id,
            tenant,
            ..
        } = &event.body
        else {
            continue;
        };
        if last_seq != 0 || index != 0 {
            return Err(invalid(format!(
                "commissioned {} is not the first event of its run",
                event.key
            )));
        }
        if row.is_some_and(|row| &row.workspace_id != workspace_id) {
            return Err(invalid("commissioned changes workspace_id".to_owned()));
        }
        if row.is_some_and(|row| &row.tenant != tenant) {
            return Err(invalid("commissioned changes tenant".to_owned()));
        }
    }
    Ok(())
}

fn invalid(what: String) -> StoreError {
    StoreError::InvalidEvent { what }
}

fn is_finished(body: &EventBody) -> bool {
    matches!(
        body,
        EventBody::Decision(nexus_run_contracts::Decision::Finished { .. })
    )
}

pub(crate) struct RunRowState {
    pub(crate) last_seq: Seq,
    pub(crate) max_v: u32,
    pub(crate) workspace_id: String,
    pub(crate) tenant: String,
}

pub(crate) fn read_run(conn: &Connection, run: &RunId) -> StoreResult<Option<RunRowState>> {
    conn.query_row(
        "SELECT last_seq, max_v, workspace_id, tenant FROM runs WHERE run = ?1",
        [run.as_str()],
        |row| {
            Ok(RunRowState {
                last_seq: row.get(0)?,
                max_v: row.get(1)?,
                workspace_id: row.get(2)?,
                tenant: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(db)
}

/// The row of a run `commissioned` creates. `max_v` stays 0 until something is appended.
pub(crate) fn create_run(tx: &Transaction<'_>, run: &RunId, event: &Event) -> StoreResult<()> {
    let EventBody::Commissioned {
        workspace_id,
        tenant,
        ..
    } = &event.body
    else {
        return Err(StoreError::UnknownRun);
    };
    tx.execute(
        "INSERT INTO runs (run, workspace_id, tenant, finished, last_seq, max_v, commissioned_at)
         VALUES (?1, ?2, ?3, 0, 0, 0, ?4)",
        params![run.as_str(), workspace_id, tenant, event.at.unix_millis()],
    )
    .map_err(db)?;
    Ok(())
}

pub(crate) fn logged_seq(conn: &Connection, run: &RunId, key: &str) -> StoreResult<Option<Seq>> {
    conn.query_row(
        "SELECT seq FROM events WHERE run = ?1 AND key = ?2",
        [run.as_str(), key],
        |row| row.get(0),
    )
    .optional()
    .map_err(db)
}

fn delete_inbox_rows(tx: &Transaction<'_>, run: &RunId, consume: &[Key]) -> StoreResult<()> {
    for key in consume {
        tx.execute(
            "DELETE FROM inbox WHERE run = ?1 AND key = ?2",
            [run.as_str(), key],
        )
        .map_err(db)?;
    }
    Ok(())
}

/// The store owns the position: an input comes out of the inbox with none, and a seq an event claims
/// could leave a gap.
fn insert_event(tx: &Transaction<'_>, event: &Event, seq: Seq) -> StoreResult<()> {
    let mut positioned = event.clone();
    positioned.seq = seq;
    tx.execute(
        "INSERT INTO events (run, seq, at, v, key, origin, body)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            event.run.as_str(),
            seq,
            event.at.unix_millis(),
            event.v,
            event.key,
            event.origin.as_str(),
            wire(&positioned)?,
        ],
    )
    .map_err(db)?;
    Ok(())
}

pub(crate) fn wire(event: &Event) -> StoreResult<String> {
    serde_json::to_string(event)
        .map_err(|_| invalid(format!("event {} cannot be written", event.key)))
}
