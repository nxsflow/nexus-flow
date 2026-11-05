//! The per-run owner lease (§7): `kv['owner/<run>']`, one per run, so two hosts may own different runs
//! of one store. Also the two values that hang off it: the epoch on an effect entry, and `host-seen`.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use nexus_run_contracts::{Epoch, RunId, RunStore, Secs, StoreError, StoreResult, Timestamp};

use crate::{db, SqliteRunStore, EFFECT_PREFIX};

const OWNER_PREFIX: &str = "owner/";
const HOST_SEEN_PREFIX: &str = "host-seen/";

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

/// The value of every `effect/<run>/<key>` entry (§8.3): the phase of the effect and the epoch that
/// wrote it. `state` is `pending`, `pending:<id>` or `done:<id>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectEntry {
    pub state: String,
    pub epoch: Epoch,
}

impl SqliteRunStore {
    /// Takes or keeps the lease of `run` for `owner_id`, by compare-and-set on the exact value read:
    /// from absent to epoch 1; from an expired lease, whoever held it, to epoch + 1; a live lease of
    /// `owner_id` is moved forward once less than half the expiry remains, and until then writes
    /// nothing. `CasFailed` on `owner/<run>` means another host holds the run, or just won the race for
    /// it: skip the run this tick.
    pub fn acquire_or_renew(
        &self,
        run: &RunId,
        owner_id: &str,
        now: Timestamp,
    ) -> StoreResult<Lease> {
        let key = owner_key(run.as_str());
        let current = self.get(&key)?;
        let epoch = match current.as_deref() {
            None => 1,
            Some(bytes) => {
                let held = decode_lease(&key, bytes)?;
                let remaining_ms = held.expires_at.unix_millis() - now.unix_millis();
                if remaining_ms <= 0 {
                    held.epoch + 1
                } else if held.owner_id != owner_id {
                    return Err(StoreError::CasFailed { key });
                } else if remaining_ms >= expiry_ms() / 2 {
                    return Ok(held);
                } else {
                    held.epoch
                }
            }
        };
        let lease = Lease {
            owner_id: owner_id.to_owned(),
            epoch,
            expires_at: now.plus(LEASE_EXPIRY),
        };
        self.compare_and_set(&key, current.as_deref(), Some(&encode(&lease)?))?;
        Ok(lease)
    }

    /// Records that a service attends this workspace, when the stored instant is at least half the
    /// attended window away from `now` in either direction: attended is two expiries, so half is one,
    /// and an idle workspace costs no WAL write. Losing the CAS to another host is fine, because that
    /// host has just written the same fact.
    pub fn mark_host_seen(&self, workspace_id: &str, now: Timestamp) -> StoreResult<()> {
        let key = format!("{HOST_SEEN_PREFIX}{workspace_id}");
        let stored = self.get(&key)?;
        let seen = stored.as_deref().and_then(decode_instant);
        if seen.is_some_and(|at| (now.unix_millis() - at.unix_millis()).abs() < expiry_ms()) {
            return Ok(());
        }
        let value = encode(&now)?;
        match self.compare_and_set(&key, stored.as_deref(), Some(&value)) {
            Err(StoreError::CasFailed { .. }) => Ok(()),
            other => other,
        }
    }

    /// When a service last recorded this workspace. A value that does not read counts as never, and
    /// the next [`Self::mark_host_seen`] replaces it.
    pub fn host_seen(&self, workspace_id: &str) -> StoreResult<Option<Timestamp>> {
        let stored = self.get(&format!("{HOST_SEEN_PREFIX}{workspace_id}"))?;
        Ok(stored.as_deref().and_then(decode_instant))
    }
}

fn expiry_ms() -> i64 {
    // 30 s, so the product fits.
    LEASE_EXPIRY.0 as i64 * 1000
}

fn owner_key(run: &str) -> String {
    format!("{OWNER_PREFIX}{run}")
}

pub(crate) fn is_owner_key(key: &str) -> bool {
    key.starts_with(OWNER_PREFIX)
}

fn encode<T: Serialize>(value: &T) -> StoreResult<Vec<u8>> {
    serde_json::to_vec(value).map_err(|error| StoreError::Backend {
        what: format!("a kv value cannot be written: {error}"),
    })
}

fn decode_instant(bytes: &[u8]) -> Option<Timestamp> {
    serde_json::from_slice(bytes).ok()
}

fn decode_lease(key: &str, bytes: &[u8]) -> StoreResult<Lease> {
    serde_json::from_slice(bytes).map_err(|_| StoreError::Backend {
        what: format!("{key} does not decode"),
    })
}

/// The epoch of a run's lease, or `None` while nobody ever held it.
pub(crate) fn current_epoch(conn: &Connection, run: &str) -> StoreResult<Option<Epoch>> {
    let key = owner_key(run);
    let value: Option<Vec<u8>> = conn
        .query_row("SELECT value FROM kv WHERE key = ?1", [&key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(db)?;
    value
        .map(|bytes| decode_lease(&key, &bytes).map(|lease| lease.epoch))
        .transpose()
}

/// An owner key is never deleted, whatever the comparison would say, and only ever holds a lease.
pub(crate) fn check_owner_value(key: &str, new: Option<&[u8]>) -> StoreResult<()> {
    let Some(new) = new else {
        return Err(StoreError::Protected {
            key: key.to_owned(),
        });
    };
    serde_json::from_slice::<Lease>(new)
        .map(|_| ())
        .map_err(|_| StoreError::InvalidEvent {
            what: format!("{key} holds a lease"),
        })
}

/// The epoch never restarts, so a stale owner cannot pass the fence again (ABA).
pub(crate) fn check_epoch_keeps_rising(
    key: &str,
    current: Option<&[u8]>,
    new: Option<&[u8]>,
) -> StoreResult<()> {
    let (Some(current), Some(new)) = (current, new) else {
        return Ok(());
    };
    if decode_lease(key, new)?.epoch < decode_lease(key, current)?.epoch {
        return Err(StoreError::Protected {
            key: key.to_owned(),
        });
    }
    Ok(())
}

/// An `effect/<run>/<key>` entry says which epoch wrote it, and that has to be the run's current one, so
/// a stale owner cannot move a ledger the new owner is reading.
pub(crate) fn check_effect_write(conn: &Connection, key: &str, value: &[u8]) -> StoreResult<()> {
    let invalid = |what: &str| StoreError::InvalidEvent {
        what: format!("{key}: {what}"),
    };
    let run = key
        .strip_prefix(EFFECT_PREFIX)
        .and_then(|rest| rest.split_once('/'))
        .and_then(|(run, _)| RunId::parse(run))
        .ok_or_else(|| invalid("an effect key is effect/<run>/<key>"))?;
    let entry: EffectEntry = serde_json::from_slice(value)
        .map_err(|_| invalid("an effect entry records its state and its epoch"))?;
    if current_epoch(conn, run.as_str())? != Some(entry.epoch) {
        return Err(StoreError::Fenced);
    }
    Ok(())
}
