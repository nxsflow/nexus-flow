//! The per-run owner lease as the host uses it (§7): acquire or renew at the top of every tick, skip a run
//! another live host holds, and re-check immediately before each effect, so a host that lost a run executes
//! nothing for it.

use nexus_run_contracts::{Epoch, RunId, StoreError, Timestamp};
use nexus_run_store_sqlite::SqliteRunStore;

use crate::tree_worker::LeaseView;

/// What this tick may do with a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Holding {
    Ours(Epoch),
    /// Another host holds it, or the store could not say: skip it, inputs and effects alike.
    Skip,
}

pub(crate) fn acquire(
    store: &SqliteRunStore,
    run: &RunId,
    owner_id: &str,
    now: Timestamp,
) -> Result<Holding, StoreError> {
    match store.acquire_or_renew(run, owner_id, now) {
        Ok(lease) => Ok(Holding::Ours(lease.epoch)),
        Err(StoreError::CasFailed { .. }) => Ok(Holding::Skip),
        Err(error) => Err(error),
    }
}

/// The check immediately before an effect: the lease still names this owner at this epoch, unexpired.
pub(crate) fn still_ours(
    store: &SqliteRunStore,
    run: &RunId,
    owner_id: &str,
    epoch: Epoch,
    now: Timestamp,
) -> bool {
    matches!(
        store.lease(run),
        Ok(Some(lease))
            if lease.owner_id == owner_id && lease.epoch == epoch && lease.expires_at > now
    )
}
