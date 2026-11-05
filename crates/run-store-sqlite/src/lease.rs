//! The per-run owner lease (§7): `kv['owner/<run>']`.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use nexus_run_contracts::{Epoch, RunId, RunStore, Secs, StoreError, StoreResult, Timestamp};

use crate::{db, SqliteRunStore};

const OWNER_PREFIX: &str = "owner/";

/// How long a lease holds without a renewal. The only time bound in the store, and it bounds who
/// decides, not what a run holds.
pub const LEASE_EXPIRY: Secs = Secs(30);

/// The value of `owner/<run>`, in compact JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub owner_id: String,
    pub epoch: Epoch,
    pub expires_at: Timestamp,
}

impl SqliteRunStore {
    /// Stub until the lease tests: from absent to epoch 1.
    pub fn acquire_or_renew(
        &self,
        run: &RunId,
        owner_id: &str,
        now: Timestamp,
    ) -> StoreResult<Lease> {
        let key = format!("{OWNER_PREFIX}{run}");
        let lease = Lease {
            owner_id: owner_id.to_owned(),
            epoch: 1,
            expires_at: now.plus(LEASE_EXPIRY),
        };
        let value = serde_json::to_vec(&lease).map_err(|_| StoreError::Backend {
            what: "a lease is JSON".to_owned(),
        })?;
        self.compare_and_set(&key, None, Some(&value))?;
        Ok(lease)
    }
}

pub(crate) fn is_owner_key(key: &str) -> bool {
    key.starts_with(OWNER_PREFIX)
}

/// The epoch of a run's lease, or `None` while nobody ever held it.
pub(crate) fn current_epoch(conn: &Connection, run: &str) -> StoreResult<Option<Epoch>> {
    let key = format!("{OWNER_PREFIX}{run}");
    let value: Option<Vec<u8>> = conn
        .query_row("SELECT value FROM kv WHERE key = ?1", [&key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(db)?;
    value
        .map(|bytes| {
            serde_json::from_slice::<Lease>(&bytes)
                .map(|lease| lease.epoch)
                .map_err(|_| StoreError::Backend {
                    what: format!("{key} does not decode"),
                })
        })
        .transpose()
}

pub(crate) fn check_owner_write(
    _key: &str,
    _current: Option<&[u8]>,
    _new: Option<&[u8]>,
) -> StoreResult<()> {
    Ok(())
}

pub(crate) fn check_effect_write(_tx: &Connection, _key: &str, _value: &[u8]) -> StoreResult<()> {
    Ok(())
}
