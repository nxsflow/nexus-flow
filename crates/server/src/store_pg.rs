//! [`PostgresOpStore`]: the durable production [`OpStore`] backend (spec §5/§10). A pure
//! swap behind the same SQL-free trait the relay already speaks — no protocol, wire, or
//! convergence change. SQLite proved convergence on a process mutex; Postgres makes the
//! same log durable across many connections.
//!
//! The one risk the trait names is the **atomic, per-stream, monotone sequence under
//! concurrent writers**. A process mutex does not generalise to a pooled, multi-connection
//! backend, so here a per-stream, transaction-scoped advisory lock
//! (`pg_advisory_xact_lock`) serialises *only the writers of one stream* — appends to
//! different streams stay concurrent — making `MAX(seq)+1` + INSERT one atomic step. The
//! observable result is identical to SQLite: per stream the sequence starts at 1, is
//! contiguous, and never gaps (an idempotent re-append is detected *before* any seq is
//! computed, so even a concurrent duplicate consumes nothing). That value-for-value
//! identity is exactly what the differential parity suite asserts against a real Postgres.
//!
//! Why `MAX(seq)+1` under the lock and **not** spec §5's `BIGINT GENERATED AS IDENTITY` /
//! a sequence: an IDENTITY/sequence is globally monotone, so per stream it would gap and
//! would not restart at 1 — neither matches the SQLite parity target (contiguous-from-1,
//! gap-free, idempotent re-append consumes nothing). The lock makes the recompute-from-MAX
//! atomic at the cost of serialising one stream's writers, which is the right trade here:
//! appends to a single stream are inherently ordered anyway.

use std::time::Duration;

use nxs_sync::protocol::{Cursor, StreamId};
use nxs_sync::wire::WireOp;
use postgres::Transaction;
use r2d2::Pool;
use r2d2_postgres::PostgresConnectionManager;

use crate::store::{decode_extra, encode_extra, page_limit, OpStore, StoreError, StoreResult};
use crate::tls_pg::make_tls;

pub(crate) type PgPool = Pool<PostgresConnectionManager<tokio_postgres_rustls::MakeRustlsConnect>>;

/// Owns the pool and guarantees it is **dropped** somewhere blocking is legal.
///
/// The half of the runtime problem that is easy to miss: [`off_runtime`] covers every *call*,
/// but a store outlives its calls and is dropped at shutdown — in the relay's case from the
/// async body of `main`. Dropping the pool closes its idle connections, each of which is a
/// synchronous `postgres::Client` that `block_on`s its internal runtime on the way out, so a
/// clean shutdown aborted the process with the same "runtime from within a runtime" panic —
/// this time inside a destructor, where Rust cannot unwind and the process gets SIGABRT.
///
/// `Deref` keeps every call site reading as `self.pool.get()`; the `Option` exists solely so
/// `Drop` can move the pool into the guarded closure, which is why it is never `None` outside
/// of that one call.
pub(crate) struct PoolHandle(Option<PgPool>);

impl std::ops::Deref for PoolHandle {
    type Target = PgPool;

    fn deref(&self) -> &PgPool {
        self.0
            .as_ref()
            .expect("pool is taken only in Drop, after which the handle is never read")
    }
}

impl Drop for PoolHandle {
    fn drop(&mut self) {
        if let Some(pool) = self.0.take() {
            off_runtime(move || drop(pool));
        }
    }
}

/// Advisory-lock **class** ids — the first arg of the two-int `pg_advisory_xact_lock`.
/// Advisory locks share ONE keyspace per database (not per table), and `main.rs` points
/// the op store and the prefix registry at the SAME database; locking both on the bare
/// stream-id hash would make an `append` and a `register` for one stream needlessly
/// contend despite touching disjoint tables. Distinct classes put each subsystem in its
/// own lock domain. (The two-int form is also a separate keyspace from any one-bigint
/// advisory lock taken elsewhere.)
pub const OPSTORE_LOCK_CLASS: i32 = 1;
pub const REGISTRY_LOCK_CLASS: i32 = 2;

/// Default pool ceiling and connection-acquire timeout. Each writer holds a connection for
/// the whole append/register transaction, so a deployment with many concurrent hot-stream
/// writers may need a wider pool; both are env-overridable. The timeout means a saturated
/// pool fails with a `StoreError` after `NXF_RELAY_PG_ACQUIRE_TIMEOUT_SECS` rather than
/// waiting unboundedly (r2d2 defaults to 30 s; we make it explicit and tunable).
const DEFAULT_POOL_MAX_SIZE: u32 = 16;
const DEFAULT_ACQUIRE_TIMEOUT_SECS: u64 = 30;

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Map a backend driver error to the trait's opaque [`StoreError`] — keeps `postgres` /
/// `r2d2` types out of the trait signature, exactly as the SQLite impl keeps `rusqlite`
/// out (the seam must not bend toward any one backend).
pub(crate) fn pg_err<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError(e.to_string())
}

/// Build a pooled connection from a libpq-style URL. Pool size and acquire-timeout are
/// env-tunable (`NXF_RELAY_PG_POOL_MAX_SIZE`, `NXF_RELAY_PG_ACQUIRE_TIMEOUT_SECS`); both
/// clamp to at least 1 so a bogus value can't disable the pool or the timeout.
///
/// The manager is handed a **TLS-capable** connector (see [`crate::tls_pg`]), which is what
/// makes a managed Postgres — Supabase, Neon, RDS — reachable at all; with the previous
/// `NoTls` they refuse the connection outright. Whether TLS is actually used stays the
/// URL's decision via libpq `sslmode`: the default `prefer` negotiates it when the server
/// offers it and falls back to plaintext when it does not, so a local database and the CI
/// service container (SSL off) behave exactly as before, while `sslmode=require` makes it
/// mandatory.
///
/// `min_idle(Some(0))` makes the pool **lazy**: connections open on demand up to `max_size`
/// instead of r2d2's default (min_idle = max_size), which eagerly opens `max_size`
/// connections per pool at build time. Lazy is right both for production (a relay serving
/// many streams doesn't sit on `max_size` idle connections) and for the parity suite (each
/// test builds its own pool; eager-fill × parallel tests would exhaust `max_connections`).
/// Connectivity is still validated eagerly — `connect()` calls `init_schema` (a `get()`)
/// right after, so a bad URL fails at construction, not at first append.
pub(crate) fn build_pool(url: &str) -> StoreResult<PoolHandle> {
    let config: postgres::Config = url.parse().map_err(pg_err)?;
    if let Some(warning) = plaintext_to_remote_warning(&config) {
        eprintln!("nxf-relay: {warning}");
    }
    let manager = PostgresConnectionManager::new(config, make_tls()?);
    let max_size = env_parse("NXF_RELAY_PG_POOL_MAX_SIZE", DEFAULT_POOL_MAX_SIZE).max(1);
    let timeout_secs = env_parse(
        "NXF_RELAY_PG_ACQUIRE_TIMEOUT_SECS",
        DEFAULT_ACQUIRE_TIMEOUT_SECS,
    )
    .max(1);
    Pool::builder()
        .max_size(max_size)
        .min_idle(Some(0))
        .connection_timeout(Duration::from_secs(timeout_secs))
        .build(manager)
        .map(|pool| PoolHandle(Some(pool)))
        .map_err(pg_err)
}

/// The warning to print when this URL would send the op log to a **remote** host in plaintext,
/// or `None` when it would not.
///
/// libpq's default `sslmode` is `prefer`, which negotiates TLS *if offered* and silently falls
/// back to plaintext otherwise — including when an on-path attacker simply refuses the upgrade.
/// That default is deliberately kept (it is what lets a local cluster and the SSL-off CI service
/// container work unchanged), but a deployment pointed at a managed database is the case where
/// the fallback is dangerous and invisible. Both guides tell operators to write
/// `sslmode=require`; this is the same sentence at the one moment they can act on it, rather
/// than only in documentation they may never have read (PR #305 review, Integrity #1).
///
/// A warning, not a refusal: some deployments legitimately reach a database over a private
/// network or a sidecar, and turning a working relay into a non-booting one on an inference
/// about the host name would be the wrong trade.
fn plaintext_to_remote_warning(config: &postgres::Config) -> Option<String> {
    // `Require` mandates TLS; `Disable` is an explicit, informed choice — neither is a silent
    // fallback. Only `prefer` (the default) can degrade without anyone saying so.
    if config.get_ssl_mode() != postgres::config::SslMode::Prefer {
        return None;
    }
    let remote: Vec<&str> = config
        .get_hosts()
        .iter()
        .filter_map(|host| match host {
            postgres::config::Host::Tcp(name) => Some(name.as_str()),
            // A Unix socket never leaves the machine.
            #[cfg(unix)]
            postgres::config::Host::Unix(_) => None,
        })
        .filter(|name| !is_loopback(name))
        .collect();
    if remote.is_empty() {
        return None;
    }
    Some(format!(
        "WARNING: connecting to {} with sslmode=prefer — if the server does not offer TLS, \
         the op log is sent in PLAINTEXT and nothing will say so. Add `sslmode=require` to \
         NXF_RELAY_PG_URL (see `nxf guide running-a-relay`).",
        remote.join(", ")
    ))
}

/// Hostnames that cannot leave the machine. Deliberately a small literal set: resolving names
/// to decide would put a DNS lookup on the boot path, and a name that resolves to a loopback
/// address today may not tomorrow.
fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Run `f` on a thread where the **synchronous** `postgres` driver is allowed to block.
///
/// This is load-bearing, not defensive. The sync driver owns an internal Tokio runtime and
/// `block_on`s it on the calling thread; Tokio panics — "Cannot start a runtime from within a
/// runtime" — the moment that happens inside an async context. The relay is exactly that
/// context: `main` is `#[tokio::main]` and every axum handler runs on a runtime worker, so
/// **`NXF_RELAY_BACKEND=postgres` aborted at boot and no request could ever have been served**
/// (nexus-flow-6j6v.94gm). The parity suite never saw it because plain `#[test]` threads have
/// no runtime at all — `tests/postgres_in_runtime.rs` is what covers the real contexts now.
///
/// The three arms are the three contexts that exist, and each needs a different answer:
///
/// * **Multi-thread runtime** — what `#[tokio::main]` builds. `block_in_place` hands this
///   worker's queued tasks to a sibling for the duration, so a slow query blocks one database
///   call rather than every task that happened to share the worker. Same reasoning, and same
///   call, as the DynamoDB bridge (`store_ddb::block_on`).
/// * **Current-thread runtime** — what an embedder may build. `block_in_place` *panics*
///   there, so the work moves to a scoped OS thread, which by construction has no runtime
///   context. A panic inside `f` is re-raised on the caller rather than swallowed by `join`.
/// * **No runtime** — the parity suite and any plain caller. Nothing to arrange; call it.
pub(crate) fn off_runtime<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send,
    T: Send,
{
    match tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()) {
        Ok(tokio::runtime::RuntimeFlavor::MultiThread) => tokio::task::block_in_place(f),
        // `RuntimeFlavor` is `#[non_exhaustive]`; anything that is not the multi-thread
        // scheduler must not reach `block_in_place`, so the catch-all is deliberate.
        Ok(_) => std::thread::scope(|s| match s.spawn(f).join() {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }),
        Err(_) => f(),
    }
}

/// Take a per-stream, transaction-scoped advisory lock in `class`. `hashtext` maps the
/// stream id to the int4 `objid`; the lock releases on commit/rollback, so a panicking
/// writer can never wedge the stream. Shared by both backends so the lock scheme — and its
/// namespacing — lives in one place.
pub(crate) fn advisory_xact_lock_stream(
    tx: &mut Transaction<'_>,
    class: i32,
    stream_id: &str,
) -> StoreResult<()> {
    tx.execute(
        "SELECT pg_advisory_xact_lock($1, hashtext($2))",
        &[&class, &stream_id],
    )
    .map_err(pg_err)?;
    Ok(())
}

/// Postgres-backed [`OpStore`]. Connections are pooled, so concurrent appends genuinely
/// race on independent sessions — the atomicity is the database's, not a client mutex's.
pub struct PostgresOpStore {
    pool: PoolHandle,
}

impl PostgresOpStore {
    /// Connect using a libpq URL (e.g. `postgres://user@host:5432/db`) and ensure the
    /// schema exists. `CREATE TABLE IF NOT EXISTS` is idempotent, so repeated boots and
    /// the parity harness's per-test schemas are both safe.
    pub fn connect(url: &str) -> StoreResult<PostgresOpStore> {
        // `init_schema` opens the first real connection, so construction is already a
        // blocking driver call — and it is the one the relay makes from its async `main`.
        off_runtime(|| {
            let store = PostgresOpStore {
                pool: build_pool(url)?,
            };
            store.init_schema()?;
            Ok(store)
        })
    }

    /// Same shape as the SQLite table — `(stream_id, seq)` PK is the only index
    /// `read_since` needs (a forward range scan); `UNIQUE (stream_id, op_id)` makes append
    /// an idempotent union (§4.4). Op fields are stored opaquely; no constraint references
    /// `target_kind` / `op_type` / `envelope_version` (server opacity, §4.1).
    ///
    /// `extra` carries every envelope field this build does not know (`WireOp::extra`,
    /// 6j6v.5crb). The separate `ADD COLUMN IF NOT EXISTS` upgrades a relay database created
    /// before it existed, where `CREATE TABLE IF NOT EXISTS` alone is a no-op; both statements are
    /// idempotent, so repeated boots and the parity harness's per-test schemas stay safe.
    fn init_schema(&self) -> StoreResult<()> {
        let mut client = self.pool.get().map_err(pg_err)?;
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS stream_ops(
                     stream_id        TEXT    NOT NULL,
                     seq              BIGINT  NOT NULL,
                     envelope_version INTEGER NOT NULL,
                     op_id            TEXT    NOT NULL,
                     lamport          BIGINT  NOT NULL,
                     site             BIGINT  NOT NULL,
                     domain           TEXT    NOT NULL DEFAULT 'task',
                     target_kind      TEXT    NOT NULL,
                     target_id        TEXT    NOT NULL,
                     field            TEXT    NOT NULL,
                     op_type          TEXT    NOT NULL,
                     value            TEXT,
                     author           TEXT    NOT NULL,
                     wall_clock       TEXT    NOT NULL,
                     extra            TEXT    NOT NULL DEFAULT '{}',
                     PRIMARY KEY (stream_id, seq),
                     UNIQUE (stream_id, op_id)
                 );
                 ALTER TABLE stream_ops
                     ADD COLUMN IF NOT EXISTS extra TEXT NOT NULL DEFAULT '{}';",
            )
            .map_err(pg_err)?;
        Ok(())
    }
}

impl OpStore for PostgresOpStore {
    // Both trait methods are reached from axum handlers — i.e. from inside the runtime — so
    // each hands its blocking driver work to `off_runtime`. The SQL itself is untouched, one
    // level down in the `*_blocking` methods.
    fn append(&self, stream: &StreamId, op: &WireOp) -> StoreResult<Cursor> {
        off_runtime(|| self.append_blocking(stream, op))
    }

    fn read_since(
        &self,
        stream: &StreamId,
        cursor: Cursor,
        limit: usize,
    ) -> StoreResult<(Vec<WireOp>, Cursor)> {
        off_runtime(|| self.read_since_blocking(stream, cursor, limit))
    }
}

impl PostgresOpStore {
    fn append_blocking(&self, stream: &StreamId, op: &WireOp) -> StoreResult<Cursor> {
        let sid = stream.as_str();
        let mut client = self.pool.get().map_err(pg_err)?;
        let mut tx = client.transaction().map_err(pg_err)?;

        // Serialise writers of THIS stream (cross-stream appends stay concurrent), in the
        // op-store lock domain so an append never contends with a register on the same DB.
        advisory_xact_lock_stream(&mut tx, OPSTORE_LOCK_CLASS, sid)?;

        // Idempotent union (§4.4): a retried/duplicated push returns its EXISTING seq and
        // consumes no new sequence value, so the log neither grows nor gaps. Detected under
        // the lock and BEFORE MAX(seq)+1, so even a concurrent duplicate leaves no gap —
        // matching SQLite's "the wasted seq left no gap" property exactly.
        if let Some(row) = tx
            .query_opt(
                "SELECT seq FROM stream_ops WHERE stream_id = $1 AND op_id = $2",
                &[&sid, &op.op_id],
            )
            .map_err(pg_err)?
        {
            let existing: i64 = row.get(0);
            tx.commit().map_err(pg_err)?;
            return Ok(Cursor(existing));
        }

        let seq: i64 = tx
            .query_one(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM stream_ops WHERE stream_id = $1",
                &[&sid],
            )
            .map_err(pg_err)?
            .get(0);

        // envelope_version is a u16 on the wire; Postgres has no u16, so it rides in an
        // INTEGER column and converts back losslessly on read (16 bits fit in i32).
        let envelope_version = op.envelope_version as i32;
        let extra = encode_extra(op);
        tx.execute(
            "INSERT INTO stream_ops
                 (stream_id, seq, envelope_version, op_id, lamport, site, domain,
                  target_kind, target_id, field, op_type, value, author, wall_clock, extra)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
            &[
                &sid,
                &seq,
                &envelope_version,
                &op.op_id,
                &op.lamport,
                &op.site,
                &op.domain,
                &op.target_kind,
                &op.target_id,
                &op.field,
                &op.op_type,
                &op.value,
                &op.author,
                &op.wall_clock,
                &extra,
            ],
        )
        .map_err(pg_err)?;
        tx.commit().map_err(pg_err)?;
        Ok(Cursor(seq))
    }

    fn read_since_blocking(
        &self,
        stream: &StreamId,
        cursor: Cursor,
        limit: usize,
    ) -> StoreResult<(Vec<WireOp>, Cursor)> {
        let sid = stream.as_str();
        // Clamp the cursor to >= 0 and the page to MAX_PAGE — same guards as SQLite, shared
        // via `page_limit`, so a negative cursor or overflowing limit can never mean "scan
        // everything" on either backend.
        let since = cursor.0.max(0);
        let limit = page_limit(limit);
        let mut client = self.pool.get().map_err(pg_err)?;
        let rows = client
            .query(
                "SELECT seq, envelope_version, op_id, lamport, site, domain, target_kind, target_id,
                        field, op_type, value, author, wall_clock, extra
                 FROM stream_ops
                 WHERE stream_id = $1 AND seq > $2
                 ORDER BY seq
                 LIMIT $3",
                &[&sid, &since, &limit],
            )
            .map_err(pg_err)?;

        let mut ops = Vec::with_capacity(rows.len());
        let mut next = cursor;
        for r in rows {
            let seq: i64 = r.get(0);
            next = Cursor(seq);
            ops.push(WireOp {
                envelope_version: r.get::<_, i32>(1) as u16,
                op_id: r.get(2),
                lamport: r.get(3),
                site: r.get(4),
                domain: r.get(5),
                target_kind: r.get(6),
                target_id: r.get(7),
                field: r.get(8),
                op_type: r.get(9),
                value: r.get(10),
                author: r.get(11),
                wall_clock: r.get(12),
                extra: decode_extra(r.get(13))?,
            });
        }
        Ok((ops, next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `off_runtime` is the guard that makes the whole backend usable inside the relay, and
    // every one of these cases is a context it can actually be called from. None of them
    // needs a database: the contract under test is "runs the closure, returns its value, does
    // not panic", which is exactly what was broken before (a panic, not a wrong answer).
    // Deliberately mirrors the DynamoDB bridge's own flavor tests in `store_ddb`.

    fn warning_for(url: &str) -> Option<String> {
        plaintext_to_remote_warning(&url.parse::<postgres::Config>().expect("parse url"))
    }

    #[test]
    fn a_remote_host_on_the_default_sslmode_is_warned_about() {
        let warning = warning_for("postgres://user@db.example.com:5432/nxf")
            .expect("prefer + remote host is exactly the silent-plaintext case");
        assert!(
            warning.contains("db.example.com") && warning.contains("sslmode=require"),
            "the warning must name the host and the fix: {warning}"
        );
    }

    #[test]
    fn requiring_tls_is_never_warned_about() {
        assert_eq!(
            warning_for("postgres://user@db.example.com:5432/nxf?sslmode=require"),
            None,
            "sslmode=require cannot fall back, so a warning would be noise that teaches \
             operators to ignore the real one"
        );
    }

    #[test]
    fn an_explicit_disable_is_not_second_guessed() {
        assert_eq!(
            warning_for("postgres://user@db.example.com:5432/nxf?sslmode=disable"),
            None,
            "disable is an informed choice, not a silent degradation"
        );
    }

    #[test]
    fn a_local_database_is_not_warned_about() {
        for url in [
            "postgres://user@localhost:5432/nxf",
            "postgres://user@127.0.0.1:5432/nxf",
            "postgres://user@[::1]:5432/nxf",
        ] {
            assert_eq!(
                warning_for(url),
                None,
                "{url} never leaves the machine — warning here would fire on every \
                 contributor's `cargo test` and on the CI service container"
            );
        }
    }

    #[test]
    fn runs_directly_when_there_is_no_runtime() {
        assert_eq!(off_runtime(|| 7u32), 7);
    }

    #[test]
    fn runs_inside_a_multi_thread_runtime() {
        // The relay itself: `#[tokio::main]` is multi-thread, and this is the async context
        // where the driver's internal `block_on` used to abort the process.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("multi-thread runtime");
        assert_eq!(rt.block_on(async { off_runtime(|| 7u32) }), 7);
    }

    #[test]
    fn runs_inside_a_current_thread_runtime() {
        // `block_in_place` panics on this flavor, so the scoped-thread arm is what keeps the
        // guard honest for an embedder that builds a current-thread runtime.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        assert_eq!(rt.block_on(async { off_runtime(|| 7u32) }), 7);
    }

    #[test]
    fn runs_on_a_blocking_pool_thread() {
        // The subtle one. Inside `spawn_blocking` the runtime context is still set, so the
        // guard sees `MultiThread` and reaches for `block_in_place` — from a thread that is
        // NOT a runtime worker. Pinned because an embedder may well call the store that way,
        // and the failure would be a panic in production rather than a wrong value here.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("multi-thread runtime");
        let got = rt.block_on(async {
            tokio::task::spawn_blocking(|| off_runtime(|| 7u32))
                .await
                .expect("blocking task")
        });
        assert_eq!(got, 7);
    }

    #[test]
    fn a_panic_inside_the_closure_reaches_the_caller() {
        // The scoped-thread arm joins, and a swallowed panic there would turn a real failure
        // into a silent hang or a wrong value. `resume_unwind` must re-raise it.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rt.block_on(async { off_runtime(|| panic!("boom")) })
        }))
        .expect_err("the panic must propagate, not be swallowed by join()");
        let msg = err
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| err.downcast_ref::<String>().map(|s| s.as_str()));
        assert_eq!(msg, Some("boom"), "the original panic payload must survive");
    }
}
