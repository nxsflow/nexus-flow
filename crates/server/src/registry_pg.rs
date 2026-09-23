//! [`PostgresPrefixRegistry`]: the durable production [`PrefixRegistry`] backend
//! (bab/§6, spec §5/§10). A pure swap behind the same SQL-free trait — claim-if-free with
//! the exact same outcome as the SQLite impl.
//!
//! It reuses `registry::candidate_prefix` so a reassigned replica derives the **same**
//! fresh prefix on either backend: that is what makes the differential parity check
//! value-for-value, not merely property-for-property. Concurrency is handled the same way
//! as [`PostgresOpStore`](crate::store_pg): a per-stream, transaction-scoped advisory lock
//! makes the read-owner + claim one atomic step (the SQLite mutex's per-stream analogue),
//! so two replicas racing on the same prefix yield exactly one `Registered` and the rest
//! `Reassigned`.

use nxs_sync::protocol::{MachineHello, RegisterOutcome, StreamId};
use postgres::Transaction;

use crate::presence::{MachineRecord, MachinesPage, PresenceStore};
use crate::registry::{candidate_prefix, PrefixRegistry, REASSIGN_ATTEMPTS};
use crate::store::{StoreError, StoreResult};
use crate::store_pg::{
    advisory_xact_lock_stream, build_pool, off_runtime, pg_err, PoolHandle, REGISTRY_LOCK_CLASS,
};

/// Postgres-backed [`PrefixRegistry`]. Pooled like the op store so concurrent registrants
/// race on independent sessions; the advisory lock — not a client mutex — orders them.
pub struct PostgresPrefixRegistry {
    pool: PoolHandle,
}

impl PostgresPrefixRegistry {
    pub fn connect(url: &str) -> StoreResult<PostgresPrefixRegistry> {
        // Same reason as `PostgresOpStore::connect`: `init_schema` is a real blocking driver
        // call, and the relay makes it from its async `main` (see `store_pg::off_runtime`).
        off_runtime(|| {
            let reg = PostgresPrefixRegistry {
                pool: build_pool(url)?,
            };
            reg.init_schema()?;
            Ok(reg)
        })
    }

    /// Same shape as the SQLite tables. PK `(stream_id, prefix)`: one owner per prefix per
    /// stream — the claim-if-free key. UNIQUE `(stream_id, replica_uuid)`: one prefix per
    /// replica per stream — makes the idempotent "what did this replica get?" lookup
    /// single-valued. `machine_presence` (6j6v.f0b5): one row per machine per stream, overwritten
    /// by every announcement — `CREATE TABLE IF NOT EXISTS`, so a relay database from before
    /// presence gains the table on the first boot of a relay that has it, and loses nothing.
    fn init_schema(&self) -> StoreResult<()> {
        let mut client = self.pool.get().map_err(pg_err)?;
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS prefix_claims(
                     stream_id    TEXT NOT NULL,
                     prefix       TEXT NOT NULL,
                     replica_uuid TEXT NOT NULL,
                     PRIMARY KEY (stream_id, prefix),
                     UNIQUE (stream_id, replica_uuid)
                 );
                 CREATE TABLE IF NOT EXISTS machine_presence(
                     stream_id     TEXT   NOT NULL,
                     machine_id    TEXT   NOT NULL,
                     name          TEXT   NOT NULL,
                     last_seen     BIGINT NOT NULL,
                     interval_secs BIGINT NOT NULL,
                     PRIMARY KEY (stream_id, machine_id)
                 );
                 CREATE INDEX IF NOT EXISTS machine_presence_recency
                     ON machine_presence(stream_id, last_seen);",
            )
            .map_err(pg_err)?;
        Ok(())
    }
}

impl PrefixRegistry for PostgresPrefixRegistry {
    fn presence(&self) -> Option<&dyn PresenceStore> {
        Some(self)
    }

    // Reached from the `/register` handler, so it runs inside the runtime like the op
    // store's two methods — same guard, same reason.
    fn register(
        &self,
        stream: &StreamId,
        prefix: &str,
        replica_uuid: &str,
    ) -> StoreResult<RegisterOutcome> {
        off_runtime(|| self.register_blocking(stream, prefix, replica_uuid))
    }
}

impl PostgresPrefixRegistry {
    fn register_blocking(
        &self,
        stream: &StreamId,
        prefix: &str,
        replica_uuid: &str,
    ) -> StoreResult<RegisterOutcome> {
        let sid = stream.as_str();
        let mut client = self.pool.get().map_err(pg_err)?;
        let mut tx = client.transaction().map_err(pg_err)?;

        // Serialise registrants of THIS stream so the read-owner + claim below is atomic
        // (mirrors SQLite's mutex, but per-stream — other streams register concurrently), in
        // the registry lock domain so a register never contends with an append on the same DB.
        advisory_xact_lock_stream(&mut tx, REGISTRY_LOCK_CLASS, sid)?;

        // 1. Already registered? Return the stable answer keyed by uuid (idempotent even
        //    after a crash that lost the client's adopted prefix).
        if let Some(row) = tx
            .query_opt(
                "SELECT prefix FROM prefix_claims WHERE stream_id = $1 AND replica_uuid = $2",
                &[&sid, &replica_uuid],
            )
            .map_err(pg_err)?
        {
            let existing: String = row.get(0);
            let outcome = if existing == prefix {
                RegisterOutcome::Registered
            } else {
                RegisterOutcome::Reassigned {
                    new_prefix: existing,
                }
            };
            tx.commit().map_err(pg_err)?;
            return Ok(outcome);
        }

        // 2. Is the requested prefix free in this stream? Claim it if so.
        if !prefix_taken(&mut tx, sid, prefix)? {
            claim(&mut tx, sid, prefix, replica_uuid)?;
            tx.commit().map_err(pg_err)?;
            return Ok(RegisterOutcome::Registered);
        }

        // 3. Collision with another replica → assign a fresh free prefix derived from this
        //    replica's uuid (same derivation as the SQLite impl, so reassignment is
        //    backend-independent and distinct from any other replica's sequence).
        for attempt in 0..REASSIGN_ATTEMPTS {
            let cand = candidate_prefix(replica_uuid, attempt);
            if !prefix_taken(&mut tx, sid, &cand)? {
                claim(&mut tx, sid, &cand, replica_uuid)?;
                tx.commit().map_err(pg_err)?;
                return Ok(RegisterOutcome::Reassigned { new_prefix: cand });
            }
        }
        Err(StoreError(format!(
            "prefix space exhausted for stream {sid} after {REASSIGN_ATTEMPTS} attempts"
        )))
    }
}

// Reached from the `/machines` handlers, inside the runtime — the same `off_runtime` guard as
// `register`. No advisory lock: an announcement upserts its own key, and the prune deletes only rows
// already past the window — never one a concurrent announcement is writing, which is stamped `now`.
impl PresenceStore for PostgresPrefixRegistry {
    fn announce(
        &self,
        stream: &StreamId,
        hello: &MachineHello,
        now: i64,
        forget_before: i64,
    ) -> StoreResult<()> {
        off_runtime(|| {
            let mut client = self.pool.get().map_err(pg_err)?;
            let mut tx = client.transaction().map_err(pg_err)?;
            tx.execute(
                "INSERT INTO machine_presence(stream_id, machine_id, name, last_seen, interval_secs)
                 VALUES($1, $2, $3, $4, $5)
                 ON CONFLICT (stream_id, machine_id) DO UPDATE SET
                     name = EXCLUDED.name,
                     last_seen = EXCLUDED.last_seen,
                     interval_secs = EXCLUDED.interval_secs",
                &[
                    &stream.as_str(),
                    &hello.machine_id,
                    &hello.name,
                    &now,
                    &(hello.interval_secs as i64),
                ],
            )
            .map_err(pg_err)?;
            tx.execute(
                "DELETE FROM machine_presence WHERE stream_id = $1 AND last_seen < $2",
                &[&stream.as_str(), &forget_before],
            )
            .map_err(pg_err)?;
            tx.commit().map_err(pg_err)?;
            Ok(())
        })
    }

    fn machines(
        &self,
        stream: &StreamId,
        seen_since: i64,
        limit: usize,
    ) -> StoreResult<MachinesPage> {
        off_runtime(|| {
            let mut client = self.pool.get().map_err(pg_err)?;
            // `COLLATE "C"` on the tie-break: the database's own collation (en_US and friends)
            // orders text differently from SQLite's byte order, and the backends must agree value
            // for value — the parity suite compares the traces exactly. One row past the limit is
            // how "there were more" is known without a second query.
            let rows = client
                .query(
                    "SELECT machine_id, name, last_seen, interval_secs FROM machine_presence
                     WHERE stream_id = $1 AND last_seen >= $2
                     ORDER BY last_seen DESC, machine_id COLLATE \"C\" ASC
                     LIMIT $3",
                    &[&stream.as_str(), &seen_since, &(limit as i64 + 1)],
                )
                .map_err(pg_err)?;
            let mut records: Vec<MachineRecord> = rows
                .iter()
                .map(|row| MachineRecord {
                    machine_id: row.get(0),
                    name: row.get(1),
                    last_seen: row.get(2),
                    interval_secs: row.get::<_, i64>(3).max(0) as u64,
                })
                .collect();
            let truncated = records.len() > limit;
            records.truncate(limit);
            Ok(MachinesPage { records, truncated })
        })
    }
}

fn prefix_taken(tx: &mut Transaction<'_>, sid: &str, prefix: &str) -> StoreResult<bool> {
    let taken = tx
        .query_opt(
            "SELECT 1 FROM prefix_claims WHERE stream_id = $1 AND prefix = $2",
            &[&sid, &prefix],
        )
        .map_err(pg_err)?
        .is_some();
    Ok(taken)
}

fn claim(tx: &mut Transaction<'_>, sid: &str, prefix: &str, replica_uuid: &str) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO prefix_claims(stream_id, prefix, replica_uuid) VALUES($1,$2,$3)",
        &[&sid, &prefix, &replica_uuid],
    )
    .map_err(pg_err)?;
    Ok(())
}
