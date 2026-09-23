//! [`OpStore`]: the relay's durable storage seam. SQL-free signature so the later
//! Postgres / DynamoDB backends (spec §5/§10) drop in behind it without bending the
//! contract toward SQL. Slice 1 carries one SQLite impl and proves convergence.

use std::fmt;
use std::sync::Mutex;

use nxs_sync::protocol::{Cursor, StreamId};
use nxs_sync::wire::WireOp;
use rusqlite::{params, Connection};

/// A backend-neutral storage failure. Deliberately opaque (no `rusqlite::Error` in the
/// signature) so the trait does not leak SQLite to its callers or to the other backends.
#[derive(Debug)]
pub struct StoreError(pub String);

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "op store: {}", self.0)
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> StoreError {
        StoreError(e.to_string())
    }
}

pub type StoreResult<T> = Result<T, StoreError>;

/// Hard ceiling on a single `read_since` page. Bounds the response and — crucially —
/// guards the `usize -> i64` LIMIT cast: a pathological `limit` near `usize::MAX` would
/// wrap to a negative SQLite `LIMIT` (which means "no limit") and silently defeat
/// pagination. Clamping keeps every page bounded regardless of the requested size.
pub(crate) const MAX_PAGE: usize = 10_000;

pub(crate) fn page_limit(limit: usize) -> i64 {
    limit.min(MAX_PAGE) as i64
}

/// The stored form of [`WireOp::extra`]: one JSON object, shared by all three backends so a
/// stream's ops read the same whichever one is under the relay (6j6v.5crb). A `BTreeMap` on the
/// envelope makes this deterministic — the same catch-all always encodes to the same text.
pub(crate) fn encode_extra(op: &WireOp) -> String {
    serde_json::to_string(&op.extra).expect("a map of JSON values serializes")
}

/// Parse a stored [`encode_extra`] blob back onto the envelope.
///
/// "Carried nothing" is spelled `{}`, never an empty cell: SQLite and Postgres default the column
/// to `'{}'` (which is also what the in-place upgrade backfills a pre-existing row to), and
/// DynamoDB omits the attribute entirely rather than storing an empty string — so the caller there
/// never reaches this at all. There is deliberately no empty-string fast path: it would be
/// unreachable code telling a story about a value this relay cannot write (PR #314 review, Code
/// Quality #2).
///
/// Anything that fails to parse is a tampered-with or corrupt table, not peer input — the ops the
/// relay serves must be exactly the ops it was given, so it says so rather than silently serving a
/// truncated envelope.
pub(crate) fn decode_extra(
    raw: &str,
) -> StoreResult<std::collections::BTreeMap<String, serde_json::Value>> {
    serde_json::from_str(raw).map_err(|e| {
        StoreError(format!(
            "stored envelope passthrough is not a JSON object: {e}"
        ))
    })
}

/// The durable op log behind the relay. Two operations, each with explicit write
/// semantics — the atomic, per-stream monotone sequence is the real backend risk (§5).
pub trait OpStore {
    /// Append one opaque op and assign it the next monotone sequence **for its stream**.
    /// Atomic under concurrent writers (the contract the durable backends must honour).
    fn append(&self, stream: &StreamId, op: &WireOp) -> StoreResult<Cursor>;

    /// One page of ops with `seq > cursor`, ordered by seq, at most `limit`. Never returns the
    /// whole stream at once.
    ///
    /// The returned cursor is the seq this call **scanned through** — `cursor` unchanged when
    /// there was nothing beyond it. Every backend today serves each op it scans, so that is also
    /// the last returned seq; the contract is deliberately worded on the SCAN because the E4 auth
    /// slice (`6j6v.6aza`) adds an ACL filter here, keyed on the authenticated identity. Under
    /// one, an implementation must still report the scan position: reporting the last VISIBLE seq
    /// would pin the client's cursor behind a run of ops it may not see and re-scan them forever,
    /// and an implementation that reports scan progress needs no client change at all
    /// (forward-compat invariant 4, `6j6v.xsf3` — the client's loop stops on a cursor that stands
    /// still, never on an empty or short page).
    fn read_since(
        &self,
        stream: &StreamId,
        cursor: Cursor,
        limit: usize,
    ) -> StoreResult<(Vec<WireOp>, Cursor)>;
}

/// SQLite-backed [`OpStore`]. The connection sits behind a `Mutex`, which serializes
/// writers and so makes the per-stream `MAX(seq)+1` assignment atomic — the SQLite
/// stand-in for Postgres' `IDENTITY` / DynamoDB's conditional counter.
pub struct SqliteOpStore {
    conn: Mutex<Connection>,
}

impl SqliteOpStore {
    pub fn open_in_memory() -> StoreResult<SqliteOpStore> {
        let conn = Connection::open_in_memory()?;
        Self::from_conn(conn)
    }

    /// File-backed durable store (the relay's default).
    pub fn open(path: &str) -> StoreResult<SqliteOpStore> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        Self::from_conn(conn)
    }

    fn from_conn(conn: Connection) -> StoreResult<SqliteOpStore> {
        // (stream_id, seq) is the PK and the only index read_since needs (a forward range
        // scan). UNIQUE(stream_id, op_id) makes append idempotent: a retried/duplicated
        // push is ignored rather than stored under a fresh seq, so the authoritative log
        // stays an idempotent union (§4.4) and does not grow without bound. The op fields
        // are stored opaquely — no constraint references target_kind/op_type/version.
        //
        // `extra` is the passthrough for every envelope field THIS BUILD does not know
        // (`WireOp::extra`, 6j6v.5crb): one JSON object per op, stored and returned verbatim.
        // Without it the relay is opaque to unknown values but not to unknown fields, and an
        // additive `sig` from a newer client would be stripped here.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS stream_ops(
                 stream_id        TEXT    NOT NULL,
                 seq              INTEGER NOT NULL,
                 envelope_version INTEGER NOT NULL,
                 op_id            TEXT    NOT NULL,
                 lamport          INTEGER NOT NULL,
                 site             INTEGER NOT NULL,
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
             );",
        )?;
        Self::add_extra_column_if_missing(&conn)?;
        Ok(SqliteOpStore {
            conn: Mutex::new(conn),
        })
    }

    /// In-place upgrade of a relay db created before `extra` existed. `CREATE TABLE IF NOT EXISTS`
    /// leaves an existing table untouched, so the column has to be ALTERed in — and SQLite has no
    /// `ADD COLUMN IF NOT EXISTS`. The `NOT NULL DEFAULT '{}'` backfills every stored op as
    /// "carried nothing unknown", which is exactly true of ops appended by the older build.
    ///
    /// `BEGIN IMMEDIATE` makes the check and the DDL one indivisible step (mirrors
    /// `nxs_foundation::schema::migrate`): two relay processes opening the same file would
    /// otherwise both see the column missing, both ALTER, and the loser would fail on "duplicate
    /// column name". `busy_timeout` (set in [`open`](Self::open)) makes the second wait for the
    /// first to commit and then find the column already there.
    fn add_extra_column_if_missing(conn: &Connection) -> StoreResult<()> {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> StoreResult<()> {
            let present: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('stream_ops') WHERE name = 'extra'",
                [],
                |r| r.get(0),
            )?;
            if present == 0 {
                conn.execute_batch(
                    "ALTER TABLE stream_ops ADD COLUMN extra TEXT NOT NULL DEFAULT '{}';",
                )?;
            }
            Ok(())
        })();
        match &result {
            Ok(()) => conn.execute_batch("COMMIT")?,
            // Discard a ROLLBACK failure deliberately: `result` holds the root-cause error, which
            // is what the caller must see. The connection is dropped on the error path anyway, and
            // SQLite rolls back an open transaction on close.
            Err(_) => {
                let _ = conn.execute_batch("ROLLBACK");
            }
        }
        result
    }

    /// Lock the connection, recovering from poisoning. A panic while another request held
    /// the lock must not wedge the relay for every subsequent request: the op log is
    /// append-only and each op is a single statement, so the data stays consistent.
    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl OpStore for SqliteOpStore {
    fn append(&self, stream: &StreamId, op: &WireOp) -> StoreResult<Cursor> {
        // Holding the mutex makes the read-max + insert one atomic step: no two appends
        // can observe the same MAX(seq) and collide on the (stream_id, seq) PK.
        let conn = self.lock();
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM stream_ops WHERE stream_id = ?1",
            [stream.as_str()],
            |r| r.get(0),
        )?;
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO stream_ops
                 (stream_id, seq, envelope_version, op_id, lamport, site, domain,
                  target_kind, target_id, field, op_type, value, author, wall_clock, extra)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            params![
                stream.as_str(),
                seq,
                op.envelope_version,
                op.op_id,
                op.lamport,
                op.site,
                op.domain,
                op.target_kind,
                op.target_id,
                op.field,
                op.op_type,
                op.value,
                op.author,
                op.wall_clock,
                encode_extra(op),
            ],
        )?;
        if inserted == 0 {
            // Already appended (idempotent union, §4.4): return the op's EXISTING cursor so
            // a retried/duplicated push neither grows the log nor reports a phantom seq.
            // The computed `seq` was not consumed, so the next distinct op reuses it — no
            // gap in the sequence.
            let existing: i64 = conn.query_row(
                "SELECT seq FROM stream_ops WHERE stream_id = ?1 AND op_id = ?2",
                params![stream.as_str(), op.op_id],
                |r| r.get(0),
            )?;
            return Ok(Cursor(existing));
        }
        Ok(Cursor(seq))
    }

    fn read_since(
        &self,
        stream: &StreamId,
        cursor: Cursor,
        limit: usize,
    ) -> StoreResult<(Vec<WireOp>, Cursor)> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT seq, envelope_version, op_id, lamport, site, domain, target_kind, target_id,
                    field, op_type, value, author, wall_clock, extra
             FROM stream_ops
             WHERE stream_id = ?1 AND seq > ?2
             ORDER BY seq
             LIMIT ?3",
        )?;
        // Clamp the cursor to >= 0 and the page to MAX_PAGE: a negative cursor or an
        // overflowing limit must not turn into "scan everything".
        let rows = stmt.query_map(
            params![stream.as_str(), cursor.0.max(0), page_limit(limit)],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    WireOp {
                        envelope_version: r.get(1)?,
                        op_id: r.get(2)?,
                        lamport: r.get(3)?,
                        site: r.get(4)?,
                        domain: r.get(5)?,
                        target_kind: r.get(6)?,
                        target_id: r.get(7)?,
                        field: r.get(8)?,
                        op_type: r.get(9)?,
                        value: r.get(10)?,
                        author: r.get(11)?,
                        wall_clock: r.get(12)?,
                        extra: Default::default(),
                    },
                    // Decoded outside the closure: a malformed blob is a StoreError, not a
                    // rusqlite::Error, and this closure can only produce the latter.
                    r.get::<_, String>(13)?,
                ))
            },
        )?;
        let mut ops = Vec::new();
        let mut next = cursor;
        for row in rows {
            let (seq, mut op, raw_extra) = row?;
            op.extra = decode_extra(&raw_extra)?;
            next = Cursor(seq);
            ops.push(op);
        }
        Ok((ops, next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nxs_sync::protocol::{Cursor, StreamId};
    use nxs_sync::wire::{WireOp, ENVELOPE_VERSION};

    /// [`wire`], as a payload from a build that knows a field this one does not — parsed from JSON
    /// on purpose, since that is the only way an unknown field can enter (mirrors the Postgres
    /// suite's helper of the same name).
    fn wire_from_a_newer_client(op_id: &str) -> WireOp {
        let mut raw = serde_json::to_value(wire(op_id)).expect("a wire op serializes");
        raw["sig"] = serde_json::Value::String("ed25519:deadbeef".into());
        serde_json::from_value(raw).expect("an unknown field parses into the catch-all")
    }

    fn wire(op_id: &str) -> WireOp {
        WireOp {
            envelope_version: ENVELOPE_VERSION,
            op_id: op_id.into(),
            lamport: 1,
            site: 1,
            domain: "task".into(),
            target_kind: "item".into(),
            target_id: "ab12.0001".into(),
            field: "title".into(),
            op_type: "set".into(),
            value: Some("t".into()),
            author: "a".into(),
            wall_clock: String::new(),
            extra: Default::default(),
        }
    }

    #[test]
    fn relay_round_trips_a_foreign_op_domain_opaquely() {
        // aye.1.4 / §4.1: the relay is opaque — it stores every field, including `domain`, and
        // interprets none. A foreign `fact` op pushed through must come back with its domain
        // intact, not relabelled `task`; otherwise forward-compat holds locally but breaks over
        // sync (the whole point of carrying `domain` on the envelope).
        let store = SqliteOpStore::open_in_memory().unwrap();
        let s = StreamId("s".into());
        let mut fact = wire("f1");
        fact.domain = "fact".into();
        store.append(&s, &fact).unwrap();

        let (ops, _) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(
            ops[0].domain, "fact",
            "the relay round-trips the op's domain verbatim"
        );
    }

    #[test]
    fn the_relay_preserves_an_ops_canonical_bytes_exactly() {
        // Forward-compat invariant 3 (6j6v.xsf3): the E4 auth slice signs an op at its author
        // and verifies it at the far end, so the bytes the signature covers must survive the
        // hop through the relay UNCHANGED — every field, byte for byte. A field the relay
        // normalized (an empty string read back as NULL, a `None` collapsed into `Some("")`,
        // a non-ASCII body mangled) would only surface in E4 as "signatures fail sometimes",
        // by which point re-plumbing the storage layer is the fix. Prove it here instead,
        // against the awkward values rather than the happy one.
        let store = SqliteOpStore::open_in_memory().unwrap();
        let s = StreamId("s".into());

        let mut awkward = Vec::new();
        for (n, tweak) in [
            (|o: &mut WireOp| o.value = None) as fn(&mut WireOp),
            |o: &mut WireOp| o.value = Some(String::new()),
            |o: &mut WireOp| o.value = Some("λ — ünïcode\n\t\"quoted\"\u{1f}sep".into()),
            |o: &mut WireOp| o.wall_clock = "2026-08-08T21:00:00Z".into(),
            |o: &mut WireOp| o.domain = "message".into(),
            |o: &mut WireOp| o.envelope_version = 999,
            |o: &mut WireOp| o.author = "nxsflow/nexus-flow/PmAgent".into(),
        ]
        .into_iter()
        .enumerate()
        {
            let mut op = wire(&format!("awkward-{n}"));
            tweak(&mut op);
            awkward.push(op);
        }
        for op in &awkward {
            store.append(&s, op).unwrap();
        }

        let (back, _) = store.read_since(&s, Cursor::BEGINNING, 100).unwrap();
        assert_eq!(back.len(), awkward.len());
        for (sent, got) in awkward.iter().zip(back.iter()) {
            assert_eq!(
                sent.canonical_bytes(),
                got.canonical_bytes(),
                "canonical bytes changed crossing the relay: {sent:?} -> {got:?}"
            );
            assert_eq!(sent, got, "and the envelope itself round-trips whole");
        }
    }

    #[test]
    fn an_unknown_envelope_field_survives_the_relay() {
        // 6j6v.5crb, the whole point of the ticket: the sync crate can only prove the envelope
        // TYPE carries an unknown field; this proves the part that actually matters — that it
        // survives a real store — by pushing a JSON payload carrying a field this build does not
        // know through append + read_since. Until this landed the drop was durable here (the
        // relay reserializes from its own named columns), so a `sig` from a newer client pushed
        // through an older relay came back unsigned: a silent downgrade on exactly the path
        // signing exists to protect (6j6v.6aza, point 3).
        let store = SqliteOpStore::open_in_memory().unwrap();
        let s = StreamId("s".into());

        let mut raw = serde_json::to_value(wire("future-1")).unwrap();
        raw["sig"] = serde_json::Value::String("ed25519:deadbeef".into());
        let from_a_newer_client: WireOp = serde_json::from_value(raw.clone()).unwrap();
        store.append(&s, &from_a_newer_client).unwrap();

        let (back, _) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(
            serde_json::to_value(&back[0]).unwrap(),
            raw,
            "the relay serves back the payload it was handed, unknown field and all"
        );
        assert_eq!(
            back[0], from_a_newer_client,
            "and the envelope round-trips whole, not just its known half"
        );
    }

    #[test]
    fn a_corrupt_passthrough_blob_is_reported_not_served_as_a_truncated_envelope() {
        // The error path `decode_extra` deliberately implements (PR #314 review, Test Quality #3).
        // The relay's whole claim is that it serves back exactly the op it was handed; if the
        // stored passthrough is unreadable, the honest answer is to fail the read, NOT to serve an
        // envelope silently missing whatever it held — which is indistinguishable, at the far end,
        // from an op that never carried a signature at all.
        let store = SqliteOpStore::open_in_memory().unwrap();
        let s = StreamId("s".into());
        store.append(&s, &wire("o1")).unwrap();
        store
            .lock()
            .execute("UPDATE stream_ops SET extra = 'not json'", [])
            .unwrap();

        let err = store
            .read_since(&s, Cursor::BEGINNING, 10)
            .expect_err("a corrupt passthrough must surface, not be swallowed");
        assert!(
            err.to_string().contains("passthrough"),
            "the error names what is corrupt rather than being opaque: {err}"
        );
    }

    #[test]
    fn a_relay_db_written_before_the_passthrough_column_upgrades_in_place() {
        // The in-place upgrade path (`add_extra_column_if_missing`): `CREATE TABLE IF NOT EXISTS`
        // leaves an existing table alone, so a relay file written by a build that predates `extra`
        // would keep serving without the column and fail every INSERT. Reconstruct exactly that
        // db, then open it with the current code.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("relay.sqlite");
        let old = Connection::open(&path).unwrap();
        old.execute_batch(
            "CREATE TABLE stream_ops(
                 stream_id        TEXT    NOT NULL,
                 seq              INTEGER NOT NULL,
                 envelope_version INTEGER NOT NULL,
                 op_id            TEXT    NOT NULL,
                 lamport          INTEGER NOT NULL,
                 site             INTEGER NOT NULL,
                 domain           TEXT    NOT NULL DEFAULT 'task',
                 target_kind      TEXT    NOT NULL,
                 target_id        TEXT    NOT NULL,
                 field            TEXT    NOT NULL,
                 op_type          TEXT    NOT NULL,
                 value            TEXT,
                 author           TEXT    NOT NULL,
                 wall_clock       TEXT    NOT NULL,
                 PRIMARY KEY (stream_id, seq),
                 UNIQUE (stream_id, op_id)
             );
             INSERT INTO stream_ops VALUES
                 ('s',1,1,'old-1',1,1,'task','item','ab12.0001','title','set','t','a','');",
        )
        .unwrap();
        drop(old);

        let store = SqliteOpStore::open(path.to_str().unwrap()).unwrap();
        let s = StreamId("s".into());
        let (back, _) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
        assert_eq!(back.len(), 1, "the pre-existing op is still served");
        assert!(
            back[0].extra.is_empty(),
            "a row written before the column reads as 'carried nothing unknown'"
        );

        // And the upgraded db carries an unknown field from here on.
        let mut raw = serde_json::to_value(wire("future-1")).unwrap();
        raw["sig"] = serde_json::Value::String("ed25519:deadbeef".into());
        store
            .append(&s, &serde_json::from_value(raw).unwrap())
            .unwrap();
        let (back, _) = store.read_since(&s, Cursor(1), 10).unwrap();
        assert_eq!(
            back[0].extra.get("sig").and_then(|v| v.as_str()),
            Some("ed25519:deadbeef")
        );

        // Re-opening an ALREADY-migrated db must be a no-op, not a "duplicate column" failure.
        drop(store);
        SqliteOpStore::open(path.to_str().unwrap()).unwrap();
    }

    #[test]
    fn concurrent_openers_of_a_pre_upgrade_db_all_succeed() {
        // The claim `add_extra_column_if_missing`'s doc makes, pinned rather than asserted (PR #314
        // review, Test Quality #4). Two relay processes pointed at one file — a restart overlapping
        // its predecessor, or a relay next to an operator's own `nxf-relay` — would otherwise both
        // see the column missing, both ALTER, and every loser would fail on "duplicate column
        // name". `BEGIN IMMEDIATE` + `busy_timeout` is what makes the check and the DDL one step.
        use std::sync::Arc;
        use std::thread;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("relay.sqlite");
        let old = Connection::open(&path).unwrap();
        old.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE stream_ops(
                 stream_id        TEXT    NOT NULL,
                 seq              INTEGER NOT NULL,
                 envelope_version INTEGER NOT NULL,
                 op_id            TEXT    NOT NULL,
                 lamport          INTEGER NOT NULL,
                 site             INTEGER NOT NULL,
                 domain           TEXT    NOT NULL DEFAULT 'task',
                 target_kind      TEXT    NOT NULL,
                 target_id        TEXT    NOT NULL,
                 field            TEXT    NOT NULL,
                 op_type          TEXT    NOT NULL,
                 value            TEXT,
                 author           TEXT    NOT NULL,
                 wall_clock       TEXT    NOT NULL,
                 PRIMARY KEY (stream_id, seq),
                 UNIQUE (stream_id, op_id)
             );",
        )
        .unwrap();
        drop(old);

        // A barrier so the openers genuinely overlap instead of politely queueing.
        let path = Arc::new(path.to_str().unwrap().to_string());
        let start = Arc::new(std::sync::Barrier::new(8));
        let opened: Vec<_> = (0..8)
            .map(|_| {
                let path = Arc::clone(&path);
                let start = Arc::clone(&start);
                thread::spawn(move || {
                    start.wait();
                    SqliteOpStore::open(&path).map_err(|e| e.to_string())
                })
            })
            .collect();
        for (i, h) in opened.into_iter().enumerate() {
            let store = h
                .join()
                .unwrap()
                .unwrap_or_else(|e| panic!("opener {i} lost the migration race: {e}"));
            // And every opener really got the migrated schema, not merely a non-error.
            let s = StreamId(format!("s{i}"));
            store
                .append(&s, &wire_from_a_newer_client(&format!("future-{i}")))
                .unwrap();
            let (back, _) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
            assert_eq!(
                back[0].extra.get("sig").and_then(|v| v.as_str()),
                Some("ed25519:deadbeef")
            );
        }
    }

    #[test]
    fn append_assigns_monotone_sequence_per_stream() {
        let store = SqliteOpStore::open_in_memory().unwrap();
        let s = StreamId("s".into());
        assert_eq!(store.append(&s, &wire("o1")).unwrap(), Cursor(1));
        assert_eq!(store.append(&s, &wire("o2")).unwrap(), Cursor(2));
        assert_eq!(store.append(&s, &wire("o3")).unwrap(), Cursor(3));
    }

    #[test]
    fn appending_the_same_op_twice_is_idempotent() {
        // §4.4 server-side: a retried/duplicated push neither grows the log nor reports a
        // phantom seq. UNIQUE(stream_id, op_id) + INSERT OR IGNORE makes append idempotent.
        let store = SqliteOpStore::open_in_memory().unwrap();
        let s = StreamId("s".into());
        let first = store.append(&s, &wire("o1")).unwrap();
        let again = store.append(&s, &wire("o1")).unwrap();
        assert_eq!(
            first, again,
            "re-appending the same op returns its existing cursor"
        );

        let (ops, _) = store.read_since(&s, Cursor::BEGINNING, 100).unwrap();
        assert_eq!(ops.len(), 1, "the op is stored exactly once");

        // The wasted seq left no gap: a distinct op still gets the next contiguous seq.
        assert_eq!(store.append(&s, &wire("o2")).unwrap(), Cursor(2));
    }

    #[test]
    fn page_limit_is_clamped_to_max_page() {
        // Guards the usize -> i64 cast: usize::MAX must not wrap to a negative LIMIT.
        assert_eq!(page_limit(10), 10);
        assert_eq!(page_limit(usize::MAX), MAX_PAGE as i64);
        assert!(page_limit(usize::MAX) > 0, "never a negative SQLite LIMIT");
    }

    #[test]
    fn sequences_are_independent_per_stream() {
        let store = SqliteOpStore::open_in_memory().unwrap();
        let a = StreamId("a".into());
        let b = StreamId("b".into());
        assert_eq!(store.append(&a, &wire("o1")).unwrap(), Cursor(1));
        assert_eq!(store.append(&a, &wire("o2")).unwrap(), Cursor(2));
        // A different stream starts its own sequence at 1.
        assert_eq!(store.append(&b, &wire("o3")).unwrap(), Cursor(1));
    }

    #[test]
    fn read_since_returns_only_ops_after_the_cursor() {
        let store = SqliteOpStore::open_in_memory().unwrap();
        let s = StreamId("s".into());
        for id in ["o1", "o2", "o3"] {
            store.append(&s, &wire(id)).unwrap();
        }
        let (ops, next) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
        assert_eq!(
            ops.iter().map(|o| o.op_id.as_str()).collect::<Vec<_>>(),
            ["o1", "o2", "o3"]
        );
        assert_eq!(next, Cursor(3));

        let (ops, next) = store.read_since(&s, Cursor(2), 10).unwrap();
        assert_eq!(
            ops.iter().map(|o| o.op_id.as_str()).collect::<Vec<_>>(),
            ["o3"]
        );
        assert_eq!(next, Cursor(3));
    }

    #[test]
    fn read_since_paginates_via_limit_and_next_cursor() {
        let store = SqliteOpStore::open_in_memory().unwrap();
        let s = StreamId("s".into());
        for id in ["o1", "o2", "o3", "o4", "o5"] {
            store.append(&s, &wire(id)).unwrap();
        }
        // Walk the stream two ops at a time, following next-cursor until exhausted.
        let mut cursor = Cursor::BEGINNING;
        let mut seen = Vec::new();
        loop {
            let (ops, next) = store.read_since(&s, cursor, 2).unwrap();
            if ops.is_empty() {
                break;
            }
            assert!(ops.len() <= 2, "page never exceeds the limit");
            seen.extend(ops.iter().map(|o| o.op_id.clone()));
            cursor = next;
        }
        assert_eq!(seen, ["o1", "o2", "o3", "o4", "o5"]);
    }

    #[test]
    fn read_since_on_exhausted_stream_returns_empty_and_unchanged_cursor() {
        let store = SqliteOpStore::open_in_memory().unwrap();
        let s = StreamId("s".into());
        store.append(&s, &wire("o1")).unwrap();
        let (ops, next) = store.read_since(&s, Cursor(1), 10).unwrap();
        assert!(ops.is_empty());
        assert_eq!(next, Cursor(1), "no ops → cursor does not move");
    }

    #[test]
    fn stored_envelope_is_preserved_verbatim() {
        // Server opacity: every field round-trips through storage unchanged, including
        // a value and a non-default envelope_version.
        let store = SqliteOpStore::open_in_memory().unwrap();
        let s = StreamId("s".into());
        let mut op = wire("o1");
        op.envelope_version = 999;
        op.op_type = "future_set".into();
        op.value = None;
        store.append(&s, &op).unwrap();
        let (ops, _) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
        assert_eq!(ops[0], op);
    }

    #[test]
    fn concurrent_appends_get_unique_contiguous_sequences() {
        use std::sync::Arc;
        use std::thread;
        let store = Arc::new(SqliteOpStore::open_in_memory().unwrap());
        let s = StreamId("s".into());
        const THREADS: i64 = 8;
        const PER: i64 = 50;
        let mut handles = Vec::new();
        for t in 0..THREADS {
            let store = Arc::clone(&store);
            let s = s.clone();
            handles.push(thread::spawn(move || {
                let mut cursors = Vec::new();
                for i in 0..PER {
                    let c = store.append(&s, &wire(&format!("o{t}-{i}"))).unwrap();
                    cursors.push(c.0);
                }
                cursors
            }));
        }
        let mut all: Vec<i64> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        all.sort_unstable();
        let expected: Vec<i64> = (1..=THREADS * PER).collect();
        assert_eq!(
            all, expected,
            "sequences are unique and contiguous under contention"
        );
    }
}
