//! The timer index: derived from the events of an `append`, in the same transaction.

use rusqlite::Transaction;

use nexus_run_contracts::{Event, RunId, StoreResult, TimerChange};

pub(crate) fn apply(
    _tx: &Transaction<'_>,
    _run: &RunId,
    _events: &[Event],
) -> StoreResult<Vec<TimerChange>> {
    Ok(Vec::new())
}
