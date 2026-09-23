//! [`PrefixRegistry`]: the relay's claim-if-free prefix coordinator (bab/§6). It is what
//! makes cross-replica prefix distinctness UNconditional: prefixes are still minted
//! offline, but the first time a replica syncs a stream it `register`s its prefix under
//! its durable `replica_uuid`. If the prefix already belongs to a *different* replica, the
//! relay assigns a fresh free one (`Reassigned`) — resolving the collision exactly when a
//! merge would otherwise alias two replicas' same-id creates, and never sooner.
//!
//! Like [`OpStore`](crate::store::OpStore) the trait is SQL-free so the Postgres/DynamoDB
//! backends (spec §10) drop in behind it. Slice 2 carries one SQLite impl.

use std::sync::Mutex;

use nxs_sync::protocol::{MachineHello, RegisterOutcome, StreamId};
use rusqlite::{params, Connection, OptionalExtension};

use crate::presence::{MachineRecord, MachinesPage, PresenceStore};
use crate::store::{StoreError, StoreResult};

/// Crockford base32 (lowercase), matching `core::id`'s prefix alphabet so a reassigned
/// prefix reads like any other. The exact alphabet is not correctness-critical (the relay
/// hands the client an opaque 4-char prefix), only visual consistency.
pub(crate) const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// How many derived candidates to try before declaring the stream's prefix space full.
/// The space is 32^4 ≈ 1M; a stream with that many replicas is not a real scenario, so
/// exhausting this many distinct candidates is a loud error, not a retry loop.
pub(crate) const REASSIGN_ATTEMPTS: u32 = 100_000;

/// A 4-char candidate prefix derived deterministically from `(replica_uuid, attempt)`.
/// Seeding on the uuid means two replicas that collided on the *same* requested prefix
/// derive *different* candidate sequences, so their reassignments never collide with each
/// other either. Pure FNV-1a → no RNG, fully reproducible.
pub(crate) fn candidate_prefix(replica_uuid: &str, attempt: u32) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mix = |h: &mut u64, b: u8| {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for b in replica_uuid.as_bytes() {
        mix(&mut h, *b);
    }
    for b in attempt.to_le_bytes() {
        mix(&mut h, b);
    }
    (0..4)
        .map(|i| ALPHABET[((h >> (i * 5)) & 0x1f) as usize] as char)
        .collect()
}

/// The relay's prefix coordinator. One operation: register a `(prefix, replica_uuid)`
/// claim for a stream, claim-if-free, returning the outcome.
pub trait PrefixRegistry {
    /// Claim `prefix` for `replica_uuid` in `stream`. Idempotent and stable: a replica that
    /// re-registers always gets the SAME answer (its own prefix, or the prefix it was
    /// reassigned). Atomic under concurrent registrants (the durable backends' contract).
    fn register(
        &self,
        stream: &StreamId,
        prefix: &str,
        replica_uuid: &str,
    ) -> StoreResult<RegisterOutcome>;

    /// The presence board this registry keeps beside its claims, if any (6j6v.f0b5, see
    /// [`crate::presence`]). Every in-repo backend has one. DEFAULTED to `None` so a registry an
    /// embedder wrote before presence existed keeps compiling — the relay then answers the
    /// presence route with 404, exactly as a relay that predates it, which every client reads as
    /// "no presence here".
    fn presence(&self) -> Option<&dyn PresenceStore> {
        None
    }
}

/// SQLite-backed [`PrefixRegistry`]. The connection sits behind a `Mutex`, which serializes
/// registrants so the read-owner + claim is one atomic step (the SQLite stand-in for
/// Postgres' conditional insert / DynamoDB's conditional put).
pub struct SqlitePrefixRegistry {
    conn: Mutex<Connection>,
}

impl SqlitePrefixRegistry {
    pub fn open_in_memory() -> StoreResult<SqlitePrefixRegistry> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    /// File-backed durable registry (the relay's default).
    pub fn open(path: &str) -> StoreResult<SqlitePrefixRegistry> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        Self::from_conn(conn)
    }

    fn from_conn(conn: Connection) -> StoreResult<SqlitePrefixRegistry> {
        // PK (stream_id, prefix): one owner per prefix per stream — the claim-if-free key.
        // UNIQUE (stream_id, replica_uuid): one prefix per replica per stream — makes the
        // idempotent "what did this replica get?" lookup single-valued.
        // `machine_presence` (6j6v.f0b5) rides in the same file: one row per machine per stream,
        // overwritten by every announcement — state, not history (see `crate::presence`).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS prefix_claims(
                 stream_id    TEXT NOT NULL,
                 prefix       TEXT NOT NULL,
                 replica_uuid TEXT NOT NULL,
                 PRIMARY KEY (stream_id, prefix),
                 UNIQUE (stream_id, replica_uuid)
             );
             CREATE TABLE IF NOT EXISTS machine_presence(
                 stream_id     TEXT    NOT NULL,
                 machine_id    TEXT    NOT NULL,
                 name          TEXT    NOT NULL,
                 last_seen     INTEGER NOT NULL,
                 interval_secs INTEGER NOT NULL,
                 PRIMARY KEY (stream_id, machine_id)
             );
             CREATE INDEX IF NOT EXISTS machine_presence_recency
                 ON machine_presence(stream_id, last_seen);",
        )?;
        Ok(SqlitePrefixRegistry {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl PrefixRegistry for SqlitePrefixRegistry {
    fn presence(&self) -> Option<&dyn PresenceStore> {
        Some(self)
    }

    fn register(
        &self,
        stream: &StreamId,
        prefix: &str,
        replica_uuid: &str,
    ) -> StoreResult<RegisterOutcome> {
        let conn = self.lock();

        // 1. Already registered? Return the stable answer keyed by uuid (idempotent even
        //    after a crash that lost the client's adopted prefix).
        let owned: Option<String> = conn
            .query_row(
                "SELECT prefix FROM prefix_claims WHERE stream_id=?1 AND replica_uuid=?2",
                params![stream.as_str(), replica_uuid],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing) = owned {
            return Ok(if existing == prefix {
                RegisterOutcome::Registered
            } else {
                RegisterOutcome::Reassigned {
                    new_prefix: existing,
                }
            });
        }

        // 2. Is the requested prefix free in this stream? Claim it if so.
        if !prefix_taken(&conn, stream, prefix)? {
            claim(&conn, stream, prefix, replica_uuid)?;
            return Ok(RegisterOutcome::Registered);
        }

        // 3. Collision with another replica → assign a fresh free prefix derived from this
        //    replica's uuid (distinct from any other replica's derivation).
        for attempt in 0..REASSIGN_ATTEMPTS {
            let cand = candidate_prefix(replica_uuid, attempt);
            if !prefix_taken(&conn, stream, &cand)? {
                claim(&conn, stream, &cand, replica_uuid)?;
                return Ok(RegisterOutcome::Reassigned { new_prefix: cand });
            }
        }
        Err(StoreError(format!(
            "prefix space exhausted for stream {} after {REASSIGN_ATTEMPTS} attempts",
            stream.as_str()
        )))
    }
}

impl PresenceStore for SqlitePrefixRegistry {
    fn announce(
        &self,
        stream: &StreamId,
        hello: &MachineHello,
        now: i64,
        forget_before: i64,
    ) -> StoreResult<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO machine_presence(stream_id, machine_id, name, last_seen, interval_secs)
             VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(stream_id, machine_id) DO UPDATE SET
                 name = excluded.name,
                 last_seen = excluded.last_seen,
                 interval_secs = excluded.interval_secs",
            params![
                stream.as_str(),
                hello.machine_id,
                hello.name,
                now,
                hello.interval_secs as i64
            ],
        )?;
        conn.execute(
            "DELETE FROM machine_presence WHERE stream_id = ?1 AND last_seen < ?2",
            params![stream.as_str(), forget_before],
        )?;
        Ok(())
    }

    fn machines(
        &self,
        stream: &StreamId,
        seen_since: i64,
        limit: usize,
    ) -> StoreResult<MachinesPage> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT machine_id, name, last_seen, interval_secs FROM machine_presence
             WHERE stream_id = ?1 AND last_seen >= ?2
             ORDER BY last_seen DESC, machine_id ASC
             LIMIT ?3",
        )?;
        // One row past the limit is how "there were more" is known without a second query.
        let rows = stmt.query_map(
            params![stream.as_str(), seen_since, limit as i64 + 1],
            |r| {
                Ok(MachineRecord {
                    machine_id: r.get(0)?,
                    name: r.get(1)?,
                    last_seen: r.get(2)?,
                    interval_secs: r.get::<_, i64>(3)?.max(0) as u64,
                })
            },
        )?;
        let mut records = rows.collect::<Result<Vec<_>, _>>()?;
        let truncated = records.len() > limit;
        records.truncate(limit);
        Ok(MachinesPage { records, truncated })
    }
}

fn prefix_taken(conn: &Connection, stream: &StreamId, prefix: &str) -> StoreResult<bool> {
    let owner: Option<String> = conn
        .query_row(
            "SELECT replica_uuid FROM prefix_claims WHERE stream_id=?1 AND prefix=?2",
            params![stream.as_str(), prefix],
            |r| r.get(0),
        )
        .optional()?;
    Ok(owner.is_some())
}

fn claim(
    conn: &Connection,
    stream: &StreamId,
    prefix: &str,
    replica_uuid: &str,
) -> StoreResult<()> {
    conn.execute(
        "INSERT INTO prefix_claims(stream_id, prefix, replica_uuid) VALUES(?1,?2,?3)",
        params![stream.as_str(), prefix, replica_uuid],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream() -> StreamId {
        StreamId("s".into())
    }

    #[test]
    fn a_free_prefix_is_registered() {
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        assert_eq!(
            reg.register(&stream(), "aaaa", "uuid-A").unwrap(),
            RegisterOutcome::Registered
        );
    }

    #[test]
    fn re_registering_the_same_claim_is_idempotent() {
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        reg.register(&stream(), "aaaa", "uuid-A").unwrap();
        assert_eq!(
            reg.register(&stream(), "aaaa", "uuid-A").unwrap(),
            RegisterOutcome::Registered,
            "same (prefix, uuid) re-register stays Registered"
        );
    }

    #[test]
    fn a_colliding_prefix_from_a_different_replica_is_reassigned() {
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        reg.register(&stream(), "aaaa", "uuid-A").unwrap();

        let outcome = reg.register(&stream(), "aaaa", "uuid-B").unwrap();
        let RegisterOutcome::Reassigned { new_prefix } = outcome else {
            panic!("expected Reassigned, got {outcome:?}");
        };
        assert_ne!(
            new_prefix, "aaaa",
            "the colliding replica gets a different prefix"
        );
        // The reassigned prefix is now B's stable claim; A still owns "aaaa".
        assert_eq!(
            reg.register(&stream(), "aaaa", "uuid-A").unwrap(),
            RegisterOutcome::Registered,
            "A's original claim is untouched"
        );
    }

    #[test]
    fn a_reassigned_replica_gets_the_same_new_prefix_on_retry() {
        // Crash-safety: if the client is reassigned but crashes before adopting/persisting
        // the new prefix, re-registering the OLD prefix returns the SAME new prefix.
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        reg.register(&stream(), "aaaa", "uuid-A").unwrap();
        let first = reg.register(&stream(), "aaaa", "uuid-B").unwrap();
        let again = reg.register(&stream(), "aaaa", "uuid-B").unwrap();
        assert_eq!(first, again, "reassignment is stable across retries");
    }

    #[test]
    fn three_replicas_colliding_on_one_prefix_all_get_distinct_prefixes() {
        // The core no-aliasing guarantee at the registry level: no two replicas share a
        // prefix in a stream, even when all minted the same one offline.
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        let mut prefixes = std::collections::HashSet::new();
        for uuid in ["uuid-A", "uuid-B", "uuid-C"] {
            let out = reg.register(&stream(), "aaaa", uuid).unwrap();
            let p = match out {
                RegisterOutcome::Registered => "aaaa".to_string(),
                RegisterOutcome::Reassigned { new_prefix } => new_prefix,
            };
            assert!(prefixes.insert(p), "every replica's prefix is distinct");
        }
        assert_eq!(prefixes.len(), 3);
    }

    #[test]
    fn claims_are_isolated_per_stream() {
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        reg.register(&StreamId("a".into()), "aaaa", "uuid-A")
            .unwrap();
        // The same prefix is free in a different stream → Registered, not Reassigned.
        assert_eq!(
            reg.register(&StreamId("b".into()), "aaaa", "uuid-B")
                .unwrap(),
            RegisterOutcome::Registered
        );
    }

    #[test]
    fn candidate_prefixes_are_deterministic_and_uuid_distinct() {
        assert_eq!(candidate_prefix("uuid-A", 0), candidate_prefix("uuid-A", 0));
        assert_ne!(candidate_prefix("uuid-A", 0), candidate_prefix("uuid-B", 0));
        let p = candidate_prefix("uuid-A", 0);
        assert_eq!(p.len(), 4);
        assert!(p.chars().all(|c| ALPHABET.contains(&(c as u8))));
    }

    // ----- PresenceStore (6j6v.f0b5) --------------------------------------------------------

    fn hello(id: &str, name: &str) -> MachineHello {
        MachineHello {
            machine_id: id.into(),
            name: name.into(),
            interval_secs: 300,
        }
    }

    fn record(id: &str, name: &str, last_seen: i64) -> MachineRecord {
        MachineRecord {
            machine_id: id.into(),
            name: name.into(),
            last_seen,
            interval_secs: 300,
        }
    }

    /// Announce at `now`, forgetting nothing.
    fn seen_at(reg: &SqlitePrefixRegistry, stream: &StreamId, id: &str, name: &str, now: i64) {
        reg.announce(stream, &hello(id, name), now, i64::MIN)
            .unwrap();
    }

    fn all(reg: &SqlitePrefixRegistry, stream: &StreamId) -> Vec<MachineRecord> {
        reg.machines(stream, i64::MIN, 100).unwrap().records
    }

    #[test]
    fn an_announced_machine_is_listed_with_the_relay_s_time() {
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        seen_at(&reg, &stream(), "m1", "MacBook", 1_000);
        assert_eq!(all(&reg, &stream()), [record("m1", "MacBook", 1_000)]);
    }

    #[test]
    fn the_sqlite_registry_offers_its_presence_board() {
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        assert!(reg.presence().is_some());
    }

    #[test]
    fn a_machine_announcing_again_moves_its_one_entry_and_takes_its_new_name() {
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        seen_at(&reg, &stream(), "m1", "MacBook", 1_000);
        let mut renamed = hello("m1", "Work laptop");
        renamed.interval_secs = 20;
        reg.announce(&stream(), &renamed, 1_300, i64::MIN).unwrap();
        assert_eq!(
            all(&reg, &stream()),
            [MachineRecord {
                machine_id: "m1".into(),
                name: "Work laptop".into(),
                last_seen: 1_300,
                interval_secs: 20,
            }],
            "an upsert, never a second row per machine"
        );
    }

    #[test]
    fn machines_are_listed_most_recently_seen_first_ties_in_byte_order_and_the_limit_says_so() {
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        seen_at(&reg, &stream(), "old", "Old", 100);
        seen_at(&reg, &stream(), "new", "New", 300);
        seen_at(&reg, &stream(), "ma", "Tie lower", 200);
        seen_at(&reg, &stream(), "Mb", "Tie upper", 200);
        let page = |limit| {
            let page = reg.machines(&stream(), i64::MIN, limit).unwrap();
            let ids: Vec<String> = page.records.into_iter().map(|r| r.machine_id).collect();
            (ids, page.truncated)
        };
        // `Mb` before `ma`: byte order (`M` is 0x4D, `m` 0x6D), what a locale collation reverses.
        assert_eq!(
            page(100),
            (
                vec!["new".into(), "Mb".into(), "ma".into(), "old".into()],
                false
            )
        );
        assert_eq!(page(2), (vec!["new".into(), "Mb".into()], true));
        assert!(!page(4).1, "exactly as many as asked is not truncated");
    }

    #[test]
    fn a_machine_not_heard_from_within_the_window_is_not_listed_and_is_forgotten_on_the_next_announcement(
    ) {
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        seen_at(&reg, &stream(), "ghost", "Reinstalled laptop", 100);
        seen_at(&reg, &stream(), "live", "MacBook", 900);
        let listed: Vec<String> = reg
            .machines(&stream(), 500, 100)
            .unwrap()
            .records
            .into_iter()
            .map(|r| r.machine_id)
            .collect();
        assert_eq!(listed, ["live"], "not listed once past the window");
        assert_eq!(
            all(&reg, &stream()).len(),
            2,
            "still stored until an announcement prunes"
        );

        reg.announce(&stream(), &hello("live", "MacBook"), 1_000, 500)
            .unwrap();
        assert_eq!(
            all(&reg, &stream()),
            [record("live", "MacBook", 1_000)],
            "gone from the table itself"
        );
    }

    #[test]
    fn pruning_one_stream_leaves_the_others_alone() {
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        seen_at(&reg, &StreamId("a".into()), "m1", "MacBook", 100);
        reg.announce(&StreamId("b".into()), &hello("m2", "Mac mini"), 1_000, 500)
            .unwrap();
        assert_eq!(all(&reg, &StreamId("a".into())).len(), 1);
    }

    #[test]
    fn presence_is_isolated_per_stream() {
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        seen_at(&reg, &StreamId("a".into()), "m1", "MacBook", 1);
        assert!(all(&reg, &StreamId("b".into())).is_empty());
    }

    #[test]
    fn presence_survives_a_reopen_of_the_relay_file() {
        // The relay's default store is a file; presence must be in it like the prefix claims,
        // or every relay restart would make every machine look gone for a full window.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("relay.sqlite");
        let path = path.to_str().unwrap();
        seen_at(
            &SqlitePrefixRegistry::open(path).unwrap(),
            &stream(),
            "m1",
            "MacBook",
            7,
        );
        assert_eq!(
            all(&SqlitePrefixRegistry::open(path).unwrap(), &stream()),
            [record("m1", "MacBook", 7)]
        );
    }

    #[test]
    fn a_relay_file_from_before_presence_gains_the_table_and_keeps_its_claims() {
        // manufakt.io's upgrade path: a relay database that has only `prefix_claims` is opened by a
        // relay that knows presence. Nothing is migrated by hand; nothing is lost.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("relay.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE prefix_claims(
                     stream_id    TEXT NOT NULL,
                     prefix       TEXT NOT NULL,
                     replica_uuid TEXT NOT NULL,
                     PRIMARY KEY (stream_id, prefix),
                     UNIQUE (stream_id, replica_uuid)
                 );
                 INSERT INTO prefix_claims VALUES ('s', 'aaaa', 'uuid-A');",
            )
            .unwrap();
        }
        let reg = SqlitePrefixRegistry::open(path.to_str().unwrap()).unwrap();
        assert_eq!(
            reg.register(&stream(), "bbbb", "uuid-A").unwrap(),
            RegisterOutcome::Reassigned {
                new_prefix: "aaaa".into()
            },
            "the claim made before the upgrade is still the replica's"
        );
        seen_at(&reg, &stream(), "m1", "MacBook", 1);
        assert_eq!(all(&reg, &stream()).len(), 1);
    }

    #[test]
    fn the_replica_uuid_is_opaque_to_the_registry() {
        // Forward-compat invariant 2 (6j6v.xsf3): `replica_uuid` is today a random ULID, but
        // nothing may DEPEND on that shape — the E4 auth slice (6j6v.6aza) anchors a per-replica
        // Ed25519 keypair at this identity, and it must be able to do so ADDITIVELY, without
        // re-minting every replica that already exists. So the registry treats the uuid as an
        // opaque byte string: a key-shaped one registers, reassigns and stays idempotent exactly
        // like a ULID, and `candidate_prefix` is total over whatever it is handed.
        let reg = SqlitePrefixRegistry::open_in_memory().unwrap();
        let key_shaped = "ed25519:MCowBQYDK2VwAyEA6r7Vv0mQ3wLcC1xN9pKfZ8hQ2sT4uY6bW8aE0gJnHkI=";

        assert_eq!(
            reg.register(&stream(), "aaaa", key_shaped).unwrap(),
            RegisterOutcome::Registered
        );
        assert_eq!(
            reg.register(&stream(), "aaaa", key_shaped).unwrap(),
            RegisterOutcome::Registered,
            "the stable, idempotent answer does not depend on the uuid's shape"
        );
        // And it is still the identity that tells two replicas apart: a DIFFERENT uuid asking
        // for the same prefix is reassigned, key-shaped or not.
        let RegisterOutcome::Reassigned { new_prefix } = reg
            .register(&stream(), "aaaa", "01hrulid0000000000000000")
            .unwrap()
        else {
            panic!("a colliding claim from another replica must be reassigned");
        };
        assert_ne!(new_prefix, "aaaa");

        // `candidate_prefix` is total: it hashes bytes, so no uuid shape can panic it or push it
        // off the alphabet — including the degenerate ones.
        for uuid in ["", key_shaped, "λ🔑", &"x".repeat(4096)] {
            let p = candidate_prefix(uuid, 0);
            assert_eq!(p.len(), 4, "uuid {uuid:?}");
            assert!(p.chars().all(|c| ALPHABET.contains(&(c as u8))));
        }
    }
}
