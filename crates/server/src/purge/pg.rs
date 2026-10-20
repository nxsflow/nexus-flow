//! The Postgres purge store (design §3 Postgres).
//!
//! - **ONE connection of its own** from `NXF_RELAY_PG_URL`, made through [`crate::tls_pg`] exactly
//!   as the server's pools are (MF-7: CI's Postgres runs without TLS, so the TLS path is covered by
//!   the TLS job running `tests/purge_pg.rs` over `sslmode=require`, and by the connector unit test
//!   below). Never the server's pools and never `init_schema`: the purge creates nothing.
//! - **Connect directly, never through a transaction-pooling pooler** (MF-R4-2): the purge holds
//!   SESSION advisory locks across a transaction, which a pooler that hands each transaction to
//!   another server session does not keep.
//! - **P1**: the op store's and the registry's per-stream advisory locks as session locks, BEFORE
//!   `BEGIN` (`lock_timeout` 30 s, so a lock that never comes is exit 1, not a hang); `BEGIN` (READ
//!   COMMITTED); the unknown-table guard (`pg_catalog`, so a table the purge's role holds no
//!   privilege on is still seen); the plan read again and compared — any difference rolls back,
//!   exit 4, nothing deleted; plan-keyed `DELETE`s; COMMIT; unlock. A push that committed while
//!   the purge waited is in the re-read (4); one that waits commits after the unlock (residue, 3).
//! - **Physical erasure is not attempted**: dead tuples, TOAST and WAL until VACUUM, page reuse and
//!   recycling are the operator's (`physical: "not_attempted"`).

use std::sync::Mutex;

use nxs_sync::protocol::StreamId;
use serde_json::{json, Value};

use super::{
    name_unknown_table, parse_rfc3339, plan_digest, Deleted, OpsFacts, Physical, Plan, PlanDigest,
    PlanKey, PlannedKind, PurgeError, PurgeStore, RunCtx, StoreIdentity,
};
use crate::store::StoreError;
use crate::store_pg::{off_runtime, OPSTORE_LOCK_CLASS, REGISTRY_LOCK_CLASS};

/// How long the purge waits for a stream's lock before it gives up (exit 1).
const LOCK_TIMEOUT: &str = "30s";

/// A Postgres error as the report may carry it: the SQLSTATE and the constraint for a database
/// error (its message and DETAIL can quote values, log-id v1 C10), the driver's text otherwise.
fn pg_err(e: postgres::Error) -> PurgeError {
    let text = match e.as_db_error() {
        Some(db) => match db.constraint() {
            Some(c) => format!("postgres: SQLSTATE {} (constraint {c})", db.code().code()),
            None => format!("postgres: SQLSTATE {}", db.code().code()),
        },
        None => format!("postgres: {e}"),
    };
    PurgeError::Store(StoreError(text))
}

/// The connector the purge dials with: the server's own (MF-7).
pub fn connector() -> Result<tokio_postgres_rustls::MakeRustlsConnect, PurgeError> {
    crate::tls_pg::make_tls().map_err(PurgeError::Store)
}

/// One relay store on Postgres, through one connection of the purge's own.
pub struct PostgresPurgeStore {
    /// `Option` only so `Drop` can close it off any runtime, like the server's pools.
    client: Mutex<Option<postgres::Client>>,
    locator: serde_json::Map<String, Value>,
    /// The URL, kept only where the injection seam dials the server's own stores with it.
    #[cfg(feature = "test-seams")]
    url: String,
}

impl Drop for PostgresPurgeStore {
    fn drop(&mut self) {
        let client = self
            .client
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(client) = client {
            off_runtime(move || drop(client));
        }
    }
}

impl PostgresPurgeStore {
    /// Connect to `NXF_RELAY_PG_URL`. Nothing is created and no schema is initialised.
    pub fn connect(url: &str) -> Result<PostgresPurgeStore, PurgeError> {
        let config: postgres::Config = url.parse().map_err(|_| {
            PurgeError::Config("NXF_RELAY_PG_URL is not a valid Postgres URL".into())
        })?;
        let mut locator = serde_json::Map::new();
        let host = config.get_hosts().first().map(|h| match h {
            postgres::config::Host::Tcp(name) => name.clone(),
            #[cfg(unix)]
            postgres::config::Host::Unix(path) => path.to_string_lossy().into_owned(),
        });
        locator.insert("host".into(), json!(host));
        locator.insert("port".into(), json!(config.get_ports().first()));
        locator.insert("dbname".into(), json!(config.get_dbname()));
        locator.insert("user".into(), json!(config.get_user()));
        let tls = connector()?;
        let client = off_runtime(move || -> Result<postgres::Client, PurgeError> {
            let mut client = config.connect(tls).map_err(pg_err)?;
            client
                .batch_execute(&format!("SET lock_timeout = '{LOCK_TIMEOUT}'"))
                .map_err(pg_err)?;
            Ok(client)
        })?;
        Ok(PostgresPurgeStore {
            client: Mutex::new(Some(client)),
            locator,
            #[cfg(feature = "test-seams")]
            url: url.to_string(),
        })
    }

    /// Run `f` with the connection, off any runtime.
    fn with<T: Send>(
        &self,
        f: impl FnOnce(&mut postgres::Client) -> Result<T, PurgeError> + Send,
    ) -> Result<T, PurgeError> {
        let mut guard = self.client.lock().unwrap_or_else(|e| e.into_inner());
        let client = guard.as_mut().expect("the client lives until Drop");
        off_runtime(move || f(client))
    }
}

/// Every table in the current schema, and those of them with a `stream_id` column — read from
/// `pg_catalog`, which shows tables the role holds no privilege on (information_schema does not).
fn tables(c: &mut impl postgres::GenericClient) -> Result<(Vec<String>, Vec<String>), PurgeError> {
    let all: Vec<String> = c
        .query(
            "SELECT c.relname::text FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = current_schema() AND c.relkind IN ('r', 'p')
             ORDER BY 1",
            &[],
        )
        .map_err(pg_err)?
        .iter()
        .map(|r| r.get(0))
        .collect();
    let with: Vec<String> = c
        .query(
            "SELECT c.relname::text FROM pg_catalog.pg_attribute a
             JOIN pg_catalog.pg_class c ON c.oid = a.attrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = current_schema() AND c.relkind IN ('r', 'p')
               AND a.attname = 'stream_id' AND NOT a.attisdropped AND a.attnum > 0
             ORDER BY 1",
            &[],
        )
        .map_err(pg_err)?
        .iter()
        .map(|r| r.get(0))
        .collect();
    Ok((all, with))
}

fn read_plan(c: &mut impl postgres::GenericClient, stream: &StreamId) -> Result<Plan, PurgeError> {
    let (_, with) = tables(c)?;
    let unknown: Vec<String> = with
        .iter()
        .filter(|t| !super::SQL_PURGE_TABLES.contains(&t.as_str()))
        .map(|t| name_unknown_table(t))
        .collect();
    let sid = stream.as_str();
    let mut ops = PlannedKind::new("ops");
    let mut facts = OpsFacts {
        value_bytes: Some(0),
        ..OpsFacts::default()
    };
    for r in c
        .query(
            "SELECT seq, op_id, wall_clock, COALESCE(octet_length(value), 0)::bigint
             FROM stream_ops WHERE stream_id = $1 ORDER BY seq",
            &[&sid],
        )
        .map_err(pg_err)?
    {
        let (seq, op_id, wall_clock, bytes): (i64, String, String, i64) =
            (r.get(0), r.get(1), r.get(2), r.get(3));
        ops.keys.push(PlanKey::Op { seq, op_id });
        match parse_rfc3339(&wall_clock) {
            Some(t) => {
                facts.newest_wall_clock = Some(facts.newest_wall_clock.map_or(t, |n| n.max(t)))
            }
            None => facts.wall_clock_unparseable += 1,
        }
        facts.value_bytes = facts.value_bytes.map(|v| v + bytes.max(0) as u64);
    }
    let mut registry = PlannedKind::new("registry");
    for r in c
        .query(
            "SELECT prefix, replica_uuid FROM prefix_claims WHERE stream_id = $1",
            &[&sid],
        )
        .map_err(pg_err)?
    {
        registry.keys.push(PlanKey::Row {
            table: "prefix_claims",
            key: r.get(0),
            extra: r.get(1),
        });
        *registry
            .by_prefix
            .entry("prefix_claims".into())
            .or_insert(0) += 1;
    }
    let mut newest_last_seen: Option<i64> = None;
    for r in c
        .query(
            "SELECT machine_id, last_seen FROM machine_presence WHERE stream_id = $1",
            &[&sid],
        )
        .map_err(pg_err)?
    {
        let last_seen: i64 = r.get(1);
        registry.keys.push(PlanKey::Row {
            table: "machine_presence",
            key: r.get(0),
            extra: String::new(),
        });
        *registry
            .by_prefix
            .entry("machine_presence".into())
            .or_insert(0) += 1;
        newest_last_seen = Some(newest_last_seen.map_or(last_seen, |n| n.max(last_seen)));
    }
    registry.keys.sort();
    Ok(Plan {
        kinds: vec![ops, registry],
        ops: facts,
        unknown,
        lease: None,
        newest_last_seen,
    })
}

/// The keys of one table in a plan, for a plan-keyed `DELETE`.
fn row_keys(plan: &Plan, table: &str) -> Vec<String> {
    plan.kind("registry")
        .into_iter()
        .flat_map(|k| k.keys.iter())
        .filter_map(|k| match k {
            PlanKey::Row { table: t, key, .. } if *t == table => Some(key.clone()),
            _ => None,
        })
        .collect()
}

impl PurgeStore for PostgresPurgeStore {
    fn backend(&self) -> &'static str {
        "postgres"
    }

    fn kinds(&self) -> &'static [&'static str] {
        &["ops", "registry"]
    }

    fn covers(&self) -> Vec<String> {
        vec![
            "ops".into(),
            "registry:prefix_claims".into(),
            "registry:machine_presence".into(),
        ]
    }

    fn identity(&self) -> Result<StoreIdentity, PurgeError> {
        let locator = self.locator.clone();
        self.with(move |c| {
            let row = c
                .query_one(
                    "SELECT current_database()::text, current_schema()::text",
                    &[],
                )
                .map_err(pg_err)?;
            let database: String = row.get(0);
            let schema: Option<String> = row.get(1);
            let schema = schema.ok_or_else(|| {
                PurgeError::NotARelayStore(
                    "the connection's search_path names no existing schema".into(),
                )
            })?;
            let (all, _) = tables(c)?;
            for table in super::SQL_PURGE_TABLES
                .iter()
                .chain(super::SQL_ALLOWED_TABLES)
            {
                if !all.iter().any(|t| t == table) {
                    return Err(PurgeError::NotARelayStore(format!(
                        "the schema has no {table} table (start the current relay once against \
                         it, then purge)"
                    )));
                }
            }
            let store_id: Option<String> = c
                .query_opt(
                    "SELECT store_id FROM relay_store_identity WHERE singleton = 1",
                    &[],
                )
                .map_err(pg_err)?
                .map(|r| r.get(0));
            let store_id = store_id.ok_or_else(|| {
                PurgeError::NotARelayStore(
                    "the schema has no relay_store_identity row (start the current relay once \
                     against it, then purge)"
                        .into(),
                )
            })?;
            let populated: bool = c
                .query_one("SELECT EXISTS (SELECT 1 FROM stream_ops)", &[])
                .map_err(pg_err)?
                .get(0);
            // MF-10: host and port are information, never identity — a failover or an endpoint
            // rename must not turn every pin into a 78.
            Ok(StoreIdentity {
                backend: "postgres",
                locator,
                identity: json!({
                    "database": database,
                    "schema": schema,
                    "store_id": store_id,
                }),
                populated,
            })
        })
    }

    fn plan(&self, stream: &StreamId, ctx: &mut RunCtx<'_>) -> Result<Plan, PurgeError> {
        let stream = stream.clone();
        let plan = self.with(move |c| {
            let mut tx = c
                .build_transaction()
                .isolation_level(postgres::IsolationLevel::RepeatableRead)
                .read_only(true)
                .start()
                .map_err(pg_err)?;
            let plan = read_plan(&mut tx, &stream)?;
            tx.commit().map_err(pg_err)?;
            Ok(plan)
        })?;
        ctx.plan_page()?;
        Ok(plan)
    }

    fn delete(
        &self,
        stream: &StreamId,
        _plan: &Plan,
        expected: &PlanDigest,
        identity: &Value,
        ctx: &mut RunCtx<'_>,
        deleted: &mut Deleted,
    ) -> Result<Physical, PurgeError> {
        // P0 comes before the locks: an injected write takes the same locks.
        ctx.reached("P0")?;
        let sid = stream.as_str().to_string();
        let sid_ref = sid.clone();
        self.with(move |c| {
            c.execute(
                "SELECT pg_advisory_lock($1, hashtext($2))",
                &[&OPSTORE_LOCK_CLASS, &sid_ref],
            )
            .map_err(pg_err)?;
            c.execute(
                "SELECT pg_advisory_lock($1, hashtext($2))",
                &[&REGISTRY_LOCK_CLASS, &sid_ref],
            )
            .map_err(pg_err)?;
            Ok(())
        })
        .inspect_err(|_| self.unlock(&sid))?;
        let result = self.delete_locked(stream, expected, identity, ctx);
        self.unlock(&sid);
        let (n_ops, n_registry) = result?;
        deleted.add("ops", n_ops);
        deleted.add("registry", n_registry);
        ctx.progress(&format!("deleted ops={n_ops} registry={n_registry}"));
        Ok(Physical::NotAttempted)
    }

    #[cfg(feature = "test-seams")]
    fn inject(&self, stream: &StreamId, write: super::InjectWrite) -> Result<(), StoreError> {
        use crate::presence::PresenceStore;
        use crate::registry::PrefixRegistry;
        use crate::registry_pg::PostgresPrefixRegistry;
        use crate::store::OpStore;
        use crate::store_pg::PostgresOpStore;
        // The injection dials the URL the purge was given, through the server's own stores.
        let url = self.url.clone();
        let stream = stream.clone();
        off_runtime(move || match write {
            super::InjectWrite::Op => PostgresOpStore::connect(&url)?
                .append(&stream, &super::injected_op())
                .map(|_| ()),
            super::InjectWrite::Machine => PostgresPrefixRegistry::connect(&url)?.announce(
                &stream,
                &super::injected_hello(),
                super::unix_secs(std::time::SystemTime::now()),
                i64::MIN,
            ),
            super::InjectWrite::Prefix => {
                let (prefix, uuid) = super::injected_claim();
                PostgresPrefixRegistry::connect(&url)?
                    .register(&stream, &prefix, &uuid)
                    .map(|_| ())
            }
            super::InjectWrite::Steal => Ok(()),
        })
    }
}

impl PostgresPurgeStore {
    /// P1 under the held session locks: BEGIN, guard, re-read and compare, plan-keyed DELETEs,
    /// COMMIT. Returns the counts deleted.
    fn delete_locked(
        &self,
        stream: &StreamId,
        expected: &PlanDigest,
        identity: &Value,
        ctx: &mut RunCtx<'_>,
    ) -> Result<(u64, u64), PurgeError> {
        let stream = stream.clone();
        let identity = identity.clone();
        let expected = *expected;
        // Everything up to the deletes, in one transaction left open for the P1-tx point.
        let mut guard = self.client.lock().unwrap_or_else(|e| e.into_inner());
        let client = guard.as_mut().expect("the client lives until Drop");
        let outcome = off_runtime(|| -> Result<(u64, u64), PurgeError> {
            client.batch_execute("BEGIN").map_err(pg_err)?;
            let run = (|| -> Result<(u64, u64), PurgeError> {
                let fresh = read_plan(client, &stream)?;
                if !fresh.unknown.is_empty() {
                    return Err(PurgeError::UnknownKind(fresh.unknown));
                }
                if plan_digest(&identity, &fresh) != expected {
                    return Err(PurgeError::PlanChanged);
                }
                let seqs: Vec<i64> = fresh
                    .kind("ops")
                    .into_iter()
                    .flat_map(|k| k.keys.iter())
                    .filter_map(|k| match k {
                        PlanKey::Op { seq, .. } => Some(*seq),
                        _ => None,
                    })
                    .collect();
                let sid = stream.as_str();
                let n_ops = client
                    .execute(
                        "DELETE FROM stream_ops WHERE stream_id = $1 AND seq = ANY($2)",
                        &[&sid, &seqs],
                    )
                    .map_err(pg_err)?;
                let n_claims = client
                    .execute(
                        "DELETE FROM prefix_claims WHERE stream_id = $1 AND prefix = ANY($2)",
                        &[&sid, &row_keys(&fresh, "prefix_claims")],
                    )
                    .map_err(pg_err)?;
                let n_machines = client
                    .execute(
                        "DELETE FROM machine_presence WHERE stream_id = $1 AND machine_id = ANY($2)",
                        &[&sid, &row_keys(&fresh, "machine_presence")],
                    )
                    .map_err(pg_err)?;
                Ok((n_ops, n_claims + n_machines))
            })();
            if run.is_err() {
                let _ = client.batch_execute("ROLLBACK");
            }
            run
        });
        let counts = outcome?;
        if let Err(e) = ctx.reached("P1-tx") {
            let _ = off_runtime(|| client.batch_execute("ROLLBACK"));
            return Err(e);
        }
        off_runtime(|| client.batch_execute("COMMIT")).map_err(pg_err)?;
        Ok(counts)
    }

    /// Release both session locks, on every path (the session ends with the process anyway).
    fn unlock(&self, sid: &str) {
        let sid = sid.to_string();
        let _ = self.with(move |c| {
            let _ = c.execute(
                "SELECT pg_advisory_unlock($1, hashtext($2))",
                &[&REGISTRY_LOCK_CLASS, &sid],
            );
            let _ = c.execute(
                "SELECT pg_advisory_unlock($1, hashtext($2))",
                &[&OPSTORE_LOCK_CLASS, &sid],
            );
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_purge_dials_through_the_servers_tls_connector() {
        // MF-7: CI's Postgres runs without TLS, so the TLS path of the purge's connection is
        // pinned here — the connector is the one `tls_pg` builds for the server — and exercised
        // for real by the TLS job, which runs `tests/purge_pg.rs` over `sslmode=require`.
        assert!(connector().is_ok());
    }

    #[test]
    fn a_url_that_does_not_parse_is_a_configuration_error_that_never_echoes_it() {
        match PostgresPurgeStore::connect("postgres://user:s3cret@[bad") {
            Err(PurgeError::Config(m)) => assert!(!m.contains("s3cret"), "{m}"),
            Err(other) => panic!("expected Config, got {other:?}"),
            Ok(_) => panic!("expected Config, got a store"),
        }
    }
}
