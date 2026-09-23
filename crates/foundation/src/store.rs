//! The substrate. `ops` is the truth; every write appends one op and dispatches it to the reducer
//! registered for its `domain`, which folds it into that domain's materialized views with a
//! keep-if-beats rule. Merge re-applies foreign ops through the same path → convergence (validated
//! by the flow crate's differential oracle). Domain-agnostic: the substrate knows op-log mechanics
//! and the reducer registry, never any product's vocabulary.

use crate::model::Op;
use crate::reducer::Reducer;
use crate::schema;
use crate::signing::{Provenance, ReplicaKey};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// How a single op was taken into the log by [`Store::ingest`].
enum Ingest {
    /// New op, a reducer folded it into the views.
    Folded,
    /// New op, no reducer for its domain (or none that folds its shape) → persisted UNFOLDED
    /// (forward-compat, §7).
    Deferred,
    /// Op already in the log → no-op (idempotent union).
    AlreadySeen,
    /// The op's `(lamport, site)` coordinate is already held by a DIFFERENT op, so the log refused
    /// it (6j6v.fc5p) — see [`schema::ensure_op_coordinate_index`]. On [`Store::apply`] it means two
    /// replicas share a `site_id`. On the local path it means a RECEIVED op claims this replica's
    /// site on the very number the clock hands out next, above
    /// [`CLOCK_CEILING`](crate::model::CLOCK_CEILING) (6j6v.m19v); the local write then steps over
    /// it rather than losing the write — see `build_and_ingest`.
    Collided,
}

/// Switched on for every connection a store opens (review of PR #487, Integrity #6): an
/// `INSERT OR REPLACE` removes the row it replaces WITHOUT firing a delete trigger unless recursive
/// triggers are on — which would let a replace delete an op past the append-only guard
/// (`schema::ensure_append_only_guard`). A per-connection setting, so it cannot live in the schema.
const RECURSIVE_TRIGGERS: &str = "PRAGMA recursive_triggers = ON;";

/// The op-log substrate: the `ops` table + a reducer registry. Product views live in the reducers'
/// own tables, refreshed through [`Store::apply`]'s fold dispatch.
pub struct Store {
    conn: Connection,
    site: i64,
    clock: i64,
    /// The wall-clock timestamp stamped onto subsequently emitted LOCAL ops (see
    /// [`set_wall_clock`](Store::set_wall_clock)). Empty by default — display-only, never
    /// load-bearing for ordering.
    wall_clock: String,
    /// The reducer registry (spec §4.2): one reducer per domain, dispatched by `op.domain`.
    /// Empty by default — a product [`register_reducer`](Store::register_reducer)s its own.
    reducers: Vec<Box<dyn Reducer>>,
    /// The `view_watermarks.store_id` this store advances as it folds (aye.36). `None` for a raw
    /// substrate store (it tracks no product views); a product binds it at open via
    /// [`refold_if_behind`](Store::refold_if_behind) so every subsequent fold keeps the watermark
    /// current.
    views_key: Option<String>,
    /// Sibling of the db file (`<dir>/last-write`), touched on drop when this store emitted at
    /// least one LOCAL op. A neutral "this replica wrote at T" marker: the substrate knows
    /// nothing about sync, the sync daemon is merely its only consumer today. `None` for
    /// in-memory stores.
    write_marker: Option<PathBuf>,
    /// Whether this store originated a local op — gates the marker touch, so a read-only or
    /// pull-only process leaves no trace.
    emitted_local: bool,
    /// The schema version this database was stamped at **before** this open migrated it
    /// ([`migrated_from`](Store::migrated_from)).
    migrated_from: i64,
    /// This replica's signing key (6j6v.pzkb): every op [`emit`](Store::emit) appends is signed
    /// with it. Opened beside the db file, so every writer of one workspace signs with one key; an
    /// in-memory store gets one of its own.
    key: ReplicaKey,
}

impl Store {
    pub fn open_in_memory(site: i64) -> Store {
        let conn = Connection::open_in_memory().expect("open sqlite");
        conn.execute_batch(RECURSIVE_TRIGGERS)
            .expect("recursive triggers");
        schema::apply(&conn);
        schema::migrate(&conn).expect("migrate");
        let key = ReplicaKey::generate();
        crate::trust::seat_replica(&conn, site, key.key_id()).expect("seat the replica key");
        // A fresh in-memory DB is Current by construction; assert the skew gate agrees so this
        // open path is held to the same policy as `open` (spec §4.3).
        debug_assert!(matches!(
            schema::assess_open(&conn).expect("assess"),
            schema::SchemaCompat::Current
        ));
        Store {
            conn,
            site,
            clock: 0,
            wall_clock: String::new(),
            reducers: Vec::new(),
            views_key: None,
            write_marker: None,
            emitted_local: false,
            // A fresh in-memory db really was at 0 before `migrate` ran above. Reporting that
            // rather than [`schema::SCHEMA_VERSION`] keeps the field a fact about the FILE instead
            // of a hint about what the caller should do — and it costs a consumer nothing, because
            // an empty log makes the refold it might trigger a no-op.
            migrated_from: 0,
            key,
        }
    }

    /// File-backed store (opens the same shape across process runs). Fallible: a missing-parent
    /// path, a corrupt file, or a non-sqlite file returns the sqlite error so callers can surface
    /// a handled error instead of panicking.
    pub fn open(path: &str, site: i64) -> rusqlite::Result<Store> {
        let conn = Connection::open(path)?;
        // Wait-on-lock rather than fail instantly with "database is locked" when another opener
        // momentarily holds the WAL writer — e.g. the nexflow.it app, or the MCP server's own
        // auto-init materializing the substrate on the SHARED App-Data-Home board (#76u.7). The
        // product `open_store` sets this, but the substrate open path (`ensure_substrate_db`) goes
        // straight through here, so set it at the root with the same 5s budget every store uses.
        conn.execute_batch("PRAGMA busy_timeout=5000;")?;
        conn.execute_batch(RECURSIVE_TRIGGERS)?;
        // Skew gate (spec §4.3): classify the (possibly foreign-stamped) file BEFORE touching it.
        // A newer nxs that raised the compatibility floor above us → fail-loud: refuse to open,
        // no schema apply, no migrate, no clock recovery, no writes. `assess_open` reads only the
        // header pragmas — no tables needed — so this runs ahead of `try_apply` and the refusal
        // path is provably zero-write even for a FOREIGN sqlite file.
        if let Some(msg) = schema::assess_open(&conn)?.refusal() {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                Some(msg),
            ));
        }
        schema::try_apply(&conn)?;
        // Read the stamp BEFORE migrating: this is the only moment it is knowable, and a product
        // whose view gained a column in one of the steps below needs it to decide between a cheap
        // `refold_if_behind` and an unconditional [`force_refold`] (nxf 6j6v.xbnh, review of PR
        // #381). See [`migrated_from`](Store::migrated_from) for why the distinction matters.
        let migrated_from = schema::schema_version(&conn)?;
        schema::migrate(&conn)?;
        // The replica key lives beside the db (6j6v.pzkb): whoever opens this file signs with it.
        let dir = Path::new(path).parent().unwrap_or(Path::new("."));
        let key = ReplicaKey::load_or_create(dir).map_err(|e| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                Some(format!("opening this replica's signing key: {e}")),
            )
        })?;
        crate::trust::seat_replica(&conn, site, key.key_id())?;
        // The opening seed of the Lamport clock. It used to be the ONLY time this handle looked at
        // the log's high-water mark, which is what 6j6v.fc5p was: a sibling process writing to the
        // same file was never observed again. Every local write now re-reads it under the write
        // lock ([`refresh_clock`](Self::refresh_clock)), so this is the starting value a read-only
        // handle reports through [`clock`](Self::clock) rather than the basis of any mint.
        let clock = high_water_mark(&conn)?;
        Ok(Store {
            conn,
            site,
            clock,
            wall_clock: String::new(),
            reducers: Vec::new(),
            views_key: None,
            write_marker: Path::new(path).parent().map(|p| p.join("last-write")),
            emitted_local: false,
            migrated_from,
            key,
        })
    }

    /// The schema version this database carried **before** this open migrated it — `0` for a file
    /// that had never been stamped, and equal to [`schema::SCHEMA_VERSION`] for the steady-state
    /// reopen that migrated nothing.
    ///
    /// **What it is for, and why a product cannot compute it itself** (nxf 6j6v.xbnh, review of
    /// PR #381). A migration step that adds a COLUMN to a product view retroactively makes a class
    /// of op foldable that this database's earlier binary stored-don't-folded (§7) — and the
    /// watermark has long since advanced past those ops, so [`refold_if_behind`](Store::refold_if_behind)
    /// will never revisit them. Only an unconditional [`force_refold`](Store::force_refold) brings
    /// them back, and a product must know whether THIS open is the one that crossed that line.
    ///
    /// It cannot work that out after the fact: `open` migrates before it hands the connection over,
    /// so by the time a product inspects its own view the column is already there and the upgrade
    /// is invisible. flow and chat sidestep this because the tables they gate on
    /// (`custom_fields`, the pre-M2 `threads` shape) are created by their OWN view DDL, which runs
    /// after the migration and can therefore see the before-state; memory's `introduction` column
    /// is added by the foundation step itself, which is exactly the case that has no such tell.
    pub fn migrated_from(&self) -> i64 {
        self.migrated_from
    }

    /// Register a [`Reducer`] for its domain (spec §4.2). A product registers its own reducer over
    /// the substrate store it holds; the substrate ships none.
    pub fn register_reducer(&mut self, reducer: Box<dyn Reducer>) {
        self.reducers.push(reducer);
    }

    /// The reducer that folds `op`, if any: the one registered for its domain, when it folds this
    /// shape. An op whose domain has no reducer, or whose shape that reducer cannot fold, is
    /// stored-not-folded (§7) — the substrate never invents a fold for an unknown domain. So is an
    /// op past [`MAX_LAMPORT`](crate::model::MAX_LAMPORT) (6j6v.m19v), whatever its shape: every
    /// fold dispatch — the one at ingest and the one in [`refold`](Self::refold) — asks here.
    fn folder_for(&self, op: &Op) -> Option<&dyn Reducer> {
        if !op.lamport_in_bound() {
            return None;
        }
        self.reducers
            .iter()
            .map(Box::as_ref)
            .find(|r| r.domain() == op.domain)
            .filter(|r| r.is_foldable(op))
    }

    /// Set the wall-clock timestamp stamped onto subsequently emitted LOCAL ops. The op's
    /// `wall_clock` is **display-only** — history/attribution, never ordering, which is the
    /// Lamport/site pair. It defaults to blank and stays blank until a consumer with an explicit
    /// reference time sets it. Foreign ops merged via [`apply`](Store::apply) keep their own
    /// `wall_clock` untouched — this only affects ops this replica *originates*.
    pub fn set_wall_clock(&mut self, now: &str) {
        self.wall_clock = now.to_string();
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// The current Lamport clock (the highest lamport this replica has emitted/observed). Exposed
    /// for the flow crate's tests, which assert op-emission advances it.
    pub fn clock(&self) -> i64 {
        self.clock
    }

    /// The next local Lamport number. Checked, never wrapping (6j6v.m19v): the clock only ever
    /// takes numbers within [`MAX_LAMPORT`](crate::model::MAX_LAMPORT), so reaching `i64::MAX` from
    /// here would take `2^62` local writes — unreachable, which is why there is no test that gets
    /// here. It is the backstop behind that bound: a wrap would make every later local write lose
    /// every compare, silently, so a broken bound has to be loud instead.
    fn tick(&mut self) -> i64 {
        self.clock = self
            .clock
            .checked_add(1)
            .expect("the Lamport clock overflowed — no clock may leave MAX_LAMPORT's range");
        self.clock
    }

    /// Re-read the log's high-water mark into this handle's clock (6j6v.fc5p).
    ///
    /// The clock used to be seeded ONCE, at [`open`](Self::open), and afterwards moved only by this
    /// handle's own [`tick`](Self::tick) and by foreign ops passed through [`apply`](Self::apply).
    /// Neither observes a SIBLING PROCESS writing to the same file — and that is the ordinary shape
    /// this platform ships in: several agents in one workspace, a long-lived `Engine` handle beside
    /// short-lived `nxc`/`nxf` subprocesses, all on one `.nxs/db.sqlite` in WAL mode precisely so
    /// they can. Two such writers seeded at the same value mint the SAME `(lamport, site)` — same
    /// device, so the same site — and `fold_lww`'s keep-if-beats (strict `>`) then drops the second
    /// one without a word: exit 0, no error, the board still showing the old value.
    ///
    /// So the clock is refreshed **at emit** rather than only at open. That closes the same-replica
    /// axis inside the substrate with no change to the synced payload, and it is what makes
    /// [`schema::ensure_op_coordinate_index`]'s uniqueness a net rather than a tripwire.
    ///
    /// **Correct only under the write lock.** The caller must already hold `BEGIN IMMEDIATE`:
    /// read-then-write without it is the classic lost update — a sibling can commit between the
    /// `MAX` and the `INSERT`, and both writers still land on the same number.
    ///
    /// The read is a `MAX` on the leading column of `ops_lamport_site`, so it is an index lookup
    /// per local op, not the table scan it would be without that index — the two halves of this fix
    /// pay for each other.
    fn refresh_clock(&mut self) {
        let observed = high_water_mark(&self.conn).unwrap();
        if observed > self.clock {
            self.clock = observed;
        }
    }

    /// This replica's key id — `ed25519:<base64url public key>`, the string another replica puts on
    /// its trust list to believe this one (6j6v.pzkb).
    pub fn key_id(&self) -> &str {
        self.key.key_id()
    }

    /// Build + persist + materialize one LOCAL op in `domain`. Public so a product's write layer
    /// appends its domain's ops through the substrate (flow passes its `task` domain). Returns the
    /// minted op id.
    #[allow(clippy::too_many_arguments)]
    pub fn emit(
        &mut self,
        domain: &str,
        target_kind: &str,
        target_id: &str,
        field: &str,
        op_type: &str,
        value: Option<String>,
        author: &str,
    ) -> String {
        let (op_id, ingest) = self.build_and_ingest(
            domain,
            target_kind,
            target_id,
            field,
            op_type,
            value,
            author,
        );
        debug_assert!(
            matches!(ingest, Ingest::Folded),
            "emit must only build fresh, foldable ops (is the domain's reducer registered?)"
        );
        op_id
    }

    /// Build + persist one LOCAL op that this build deliberately does **not** fold into any view —
    /// store-don't-fold (§7) on purpose rather than by accident.
    ///
    /// The distinction [`emit`](Self::emit) cannot make: its assert exists to catch a product
    /// writing through an unregistered reducer, and it is right to be loud about that. But a durable
    /// fact that must ride the sync wire while materializing NOTHING is a legitimate shape — the
    /// judging migration's once-per-stream mark (nxf 6j6v.9yaj) is one: every device on the stream
    /// has to see that the run happened, and no reader may ever see it as a row. Writing it through
    /// `emit` would trip the assert; relaxing that assert would blind the case it was built for.
    ///
    /// So this is the same write with the **mirror** assert: the op must be one no registered reducer
    /// folds. A caller that reaches for this and then teaches a reducer to fold that shape fails here
    /// instead of silently growing a view row nobody expects.
    #[allow(clippy::too_many_arguments)]
    pub fn emit_unfolded(
        &mut self,
        domain: &str,
        target_kind: &str,
        target_id: &str,
        field: &str,
        op_type: &str,
        value: Option<String>,
        author: &str,
    ) -> String {
        let (op_id, ingest) = self.build_and_ingest(
            domain,
            target_kind,
            target_id,
            field,
            op_type,
            value,
            author,
        );
        debug_assert!(
            matches!(ingest, Ingest::Deferred),
            "emit_unfolded must build an op no registered reducer folds — this one materialized"
        );
        op_id
    }

    /// The shared body of the two local-write entries: mint the op, mark this store as an originator,
    /// ingest it, and report what happened. Ingestion runs in EVERY build profile — the side effect
    /// must not live inside a `debug_assert!`, which the compiler strips from release (that bug once
    /// made release binaries silently drop all writes); only the two callers' expectations are
    /// asserted.
    #[allow(clippy::too_many_arguments)]
    fn build_and_ingest(
        &mut self,
        domain: &str,
        target_kind: &str,
        target_id: &str,
        field: &str,
        op_type: &str,
        value: Option<String>,
        author: &str,
    ) -> (String, Ingest) {
        // Forward-compat invariant 1 (6j6v.xsf3): author identity is stamped on EVERY op at
        // append. A blank author is not a small blemish — it is the one defect this log cannot
        // repair, because attribution on an append-only history can never be backfilled, and the
        // E4 auth slice (6j6v.6aza) authenticates exactly this field. A hard `assert!` (not
        // `debug_assert!`) so release binaries hold the line too; the same class as
        // `create_item`'s SEP-in-id guard. Callers resolve the identity through
        // `model::resolve_author`, which makes a set-but-empty `$USER`/`*_ACTOR` unreachable
        // here — so tripping this is a programming error, not a user's environment.
        //
        // "Blank" is `model::is_attributable`'s definition, shared with the embedder-facing
        // `validate_author` and with the reducers that fold an author into an identity column: an
        // author made only of characters that render as nothing (a zero-width space, a BOM) names
        // no one either, and `str::trim` does not strip them.
        assert!(
            crate::model::is_attributable(author),
            "op author must not be blank (domain={domain}, target_kind={target_kind}, field={field})"
        );
        // The write lock is taken HERE, not inside `ingest_in_txn`, because the lamport is minted
        // from a read of the log (6j6v.fc5p): `refresh_clock` + `tick` + the INSERT have to be one
        // indivisible step, or a sibling process commits in between and both mint the same
        // coordinate — the very lost update this exists to close. IMMEDIATE for the same reason
        // `migrate` uses it: a DEFERRED transaction takes the write lock only at its first WRITE,
        // by which time the `MAX` it read may be stale.
        self.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        self.refresh_clock();
        let mut op = Op {
            op_id: ulid::Ulid::new().to_string(),
            lamport: 0,
            site: self.site,
            domain: domain.to_string(),
            target_kind: target_kind.to_string(),
            target_id: target_id.to_string(),
            field: field.to_string(),
            op_type: op_type.to_string(),
            value,
            author: author.to_string(),
            // Display-only timestamp: blank unless a consumer set an explicit `now` via
            // `set_wall_clock`. Never load-bearing for ordering (Lamport/site).
            wall_clock: self.wall_clock.clone(),
            key_id: None,
            sig: None,
        };
        let op_id = op.op_id.clone();
        // Gate the drop-time marker touch: this store originated a local op, as opposed to only
        // ever merging foreign ops via `apply` (a pull), which must never set it (loop-freedom).
        self.emitted_local = true;
        // Mint the coordinate, sign the op, append it — and step over a coordinate somebody else
        // already holds. `refresh_clock` just read the high-water mark under this write lock, so
        // no op THIS replica wrote can sit above it (6j6v.fc5p). A RECEIVED op can, since the clock
        // stops following received ops at `CLOCK_CEILING` (6j6v.m19v): one that claims this
        // replica's site on a number above the ceiling squats the very coordinate `tick` hands out
        // next. The log refuses the collision (the unique `(lamport, site)` index), and the clock
        // jumps past the whole squatted run in ONE query ([`first_free_coordinate`]) rather than
        // one number per attempt — a writer who can reach the relay can plant any number of such
        // ops, and a retry per op would hold this exclusive lock for as long as they liked (review
        // of PR #489, Integrity #1). So the second attempt is free by construction, and that is
        // asserted rather than hoped: a third attempt would mean the jump is wrong. Never on an
        // honest log, where the first attempt is always free.
        let mut attempts = 0;
        let ingest = loop {
            attempts += 1;
            op.lamport = self.tick();
            self.key.seal(&mut op);
            match self.ingest_in_txn(&op, Provenance::Own) {
                Ingest::Collided => {
                    assert!(
                        attempts < 2,
                        "local op collided again at (lamport={}, site={}) right after the jump to \
                         the first free coordinate — the jump is wrong",
                        op.lamport,
                        self.site
                    );
                    self.clock =
                        first_free_coordinate(&self.conn, self.site, op.lamport).unwrap() - 1;
                }
                taken => break taken,
            }
        };
        self.conn.execute_batch("COMMIT").unwrap();
        (op_id, ingest)
    }

    /// [`ingest_in_txn`](Self::ingest_in_txn) with its own `BEGIN IMMEDIATE`/`COMMIT` around it —
    /// the entry for a FOREIGN op, whose coordinate arrives already minted. The local path opens
    /// its own transaction earlier (in [`build_and_ingest`](Self::build_and_ingest), so the clock
    /// read that mints the coordinate is inside it) and calls the inner form directly. Raw
    /// transaction control (not rusqlite's `Transaction` guard) because the guard borrows the
    /// connection.
    fn ingest(&mut self, op: &Op, provenance: Provenance) -> Ingest {
        self.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let ingest = self.ingest_in_txn(op, provenance);
        self.conn.execute_batch("COMMIT").unwrap();
        ingest
    }

    /// Persist an op to the log (idempotent union) and, if a registered reducer folds its shape,
    /// fold it into the views. Unknown / malformed shapes are kept in the log UNFOLDED
    /// (`Ingest::Deferred`) rather than dropped, so a newer peer's op is never lost and `refold`
    /// can resurface it later (§7). **The caller holds the write transaction** — see [`ingest`].
    ///
    /// `provenance` is this replica's verdict on where the op came from (6j6v.pzkb), recorded with
    /// it and never taken from anything the op carries. An op already in the log keeps the verdict
    /// it was first recorded with: a re-delivery changes nothing, its verdict included.
    fn ingest_in_txn(&mut self, op: &Op, provenance: Provenance) -> Ingest {
        let inserted = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO ops
                 (op_id, lamport, site, domain, target_kind, target_id, field, op_type, value, author,
                  wall_clock, key_id, sig, provenance)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![
                    op.op_id, op.lamport, op.site, op.domain, op.target_kind, op.target_id,
                    op.field, op.op_type, op.value, op.author, op.wall_clock, op.key_id, op.sig,
                    provenance.as_str()
                ],
            )
            .unwrap();
        if inserted == 0 {
            // `OR IGNORE` skips a row for EITHER uniqueness constraint on the log, and the two mean
            // opposite things (6j6v.fc5p): the primary key means "this exact op is already here",
            // the honest no-op of an idempotent union; `ops_lamport_site` means "a DIFFERENT op
            // already holds this coordinate", which is a broken invariant. Telling them apart is
            // what keeps the second from being reported as the first — the silence this item is
            // about. One keyed lookup, and only on the path that already decided not to fold.
            let known: bool = self
                .conn
                .query_row("SELECT 1 FROM ops WHERE op_id=?1", [&op.op_id], |_| Ok(()))
                .optional()
                .unwrap()
                .is_some();
            return if known {
                Ingest::AlreadySeen
            } else {
                Ingest::Collided
            };
        }
        // Capture the op's rowid NOW, before the fold runs its own INSERTs into the views (which
        // would move `last_insert_rowid`). This is the snapshot boundary the watermark tracks.
        let op_rowid = self.conn.last_insert_rowid();
        // Dispatch to the reducer registered for this op's domain (§4.2). An op whose domain has
        // no reducer, or whose shape that reducer cannot fold, is stored-not-folded (§7).
        let folded = match self.folder_for(op) {
            Some(r) => {
                r.fold(&self.conn, op);
                true
            }
            None => false,
        };
        // Advance this store's folded-through watermark to the op just taken in — folded or
        // deferred alike: from THIS store's standpoint the log is processed through here (a
        // deferred op of a foreign domain is one it will never fold; a sibling store with its own
        // views_key tracks its own progress). Keeps the single-device reopen O(1) — no refold
        // (aye.36).
        self.advance_watermark(op_rowid);
        if folded {
            Ingest::Folded
        } else {
            Ingest::Deferred
        }
    }

    pub fn op_count(&self) -> i64 {
        self.conn
            .query_row("SELECT COUNT(*) FROM ops", [], |r| r.get(0))
            .unwrap()
    }

    /// SQLite's `PRAGMA data_version` for this connection. Unchanged across commits on *this*
    /// connection, but bumps whenever ANY OTHER connection — including another process — commits.
    /// A host that polls it learns when an external writer mutated the workspace, with no daemon.
    pub fn data_version(&self) -> rusqlite::Result<i64> {
        self.conn
            .pragma_query_value(None, "data_version", |r| r.get(0))
    }

    /// Export the full op log (the sync payload).
    pub fn export(&self) -> Vec<Op> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT op_id, lamport, site, domain, target_kind, target_id, field, op_type,
                        value, author, wall_clock, key_id, sig FROM ops ORDER BY lamport, site",
            )
            .unwrap();
        stmt.query_map([], |r| {
            Ok(Op {
                op_id: r.get(0)?,
                lamport: r.get(1)?,
                site: r.get(2)?,
                domain: r.get(3)?,
                target_kind: r.get(4)?,
                target_id: r.get(5)?,
                field: r.get(6)?,
                op_type: r.get(7)?,
                value: r.get(8)?,
                author: r.get(9)?,
                wall_clock: r.get(10)?,
                key_id: r.get(11)?,
                sig: r.get(12)?,
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }

    /// Merge foreign ops: advance the Lamport clock past each, fold it in via its domain's reducer.
    /// Ops with an unknown / malformed shape (or unknown domain) are **stored but not folded**
    /// (forward-compat, §7) and returned so the caller can see what was deferred; a deferred op
    /// never aborts the batch and is never dropped (a later version can resurface it via
    /// [`refold`]).
    ///
    /// **The returned vec also carries an op the log REFUSED** (6j6v.fc5p): one whose
    /// `(lamport, site)` coordinate is already held by a different op. That is a broken
    /// precondition rather than a forward-compat case — two replicas minting the same coordinate
    /// means they share a `site_id`, which 63 bits of ULID entropy make unreachable between real
    /// replicas (`workspace::fresh_replica`) and which the prefix registry already treats as a
    /// detectable, repairable misconfiguration. It rides the same channel because the honest
    /// statement about both is identical — *this store did not take this op into its views* — and
    /// because the alternative, letting `INSERT OR IGNORE` skip it as though it were a re-delivery,
    /// is precisely the silence this item exists to remove. Nothing is lost globally: the op still
    /// stands in its origin's log and on the relay, and `pulled_through` is a scan cursor, so a
    /// re-pull redelivers it once the site collision is repaired.
    ///
    /// **Clock semantics**: after each foreign op the clock is advanced to `max(clock, op.lamport)`;
    /// the next local write then calls `tick()` (`+1`). This is the standard Lamport rule — the pair
    /// `(lamport, site)` is a total order consistent with causality, NOT wall-clock chronology.
    /// Two bounds make it safe against a writer who picks the number (6j6v.m19v): an op past
    /// [`MAX_LAMPORT`](crate::model::MAX_LAMPORT) is kept and returned as not folded, but neither
    /// folds nor moves the clock; and no received op moves the clock past
    /// [`CLOCK_CEILING`](crate::model::CLOCK_CEILING). The constants' docs say why both.
    ///
    /// **Trust semantics** (6j6v.pzkb): every merged op is VERIFIED — its signature checked against
    /// the key its `key_id` names — and the verdict recorded beside it
    /// ([`Provenance`]: `verified`, `invalid` or `unsigned`). The verdict decides nothing here: an
    /// op is stored and folded exactly the same whatever it is, because the board's convergence
    /// must not depend on trust. It decides only whether an ACTION may follow the op
    /// ([`acts_on`](Self::acts_on)). Its [`author`](Op::author) is still whatever its origin wrote
    /// and is neither checked nor rewritten — the name is for display, authority is the key's.
    pub fn apply(&mut self, ops: &[Op]) -> Vec<Op> {
        let mut deferred = Vec::new();
        for op in ops {
            if op.lamport_in_bound() {
                let observed = op.lamport.min(crate::model::CLOCK_CEILING);
                if observed > self.clock {
                    self.clock = observed;
                }
            }
            match self.ingest(op, Provenance::of_received(op)) {
                Ingest::Deferred | Ingest::Collided => deferred.push(op.clone()),
                Ingest::Folded | Ingest::AlreadySeen => {}
            }
        }
        deferred
    }

    /// Re-seal ops an in-place rewrite changed (6j6v.pzkb) — the prefix remap (T-remap §8) rewrites
    /// ids and free text inside the log, which leaves each op's signature describing bytes the log
    /// no longer holds. This replica's OWN ops are signed again with its key; every other one is
    /// verified again, which for a signed op means `invalid` from here on — honest, since it is no
    /// longer what its author signed. `ops` are the ops as they now stand.
    ///
    /// Runs inside the caller's transaction (it opens none), so the rewrite and its re-seal commit
    /// together.
    pub fn reseal(&self, ops: &[Op]) {
        for op in ops {
            let provenance: Option<String> = self
                .conn
                .query_row(
                    "SELECT provenance FROM ops WHERE op_id = ?1",
                    [&op.op_id],
                    |r| r.get(0),
                )
                .optional()
                .unwrap();
            match provenance.as_deref().and_then(Provenance::parse) {
                None => {}
                Some(Provenance::Own) => {
                    let mut sealed = op.clone();
                    self.key.seal(&mut sealed);
                    self.conn
                        .execute(
                            "UPDATE ops SET key_id = ?2, sig = ?3 WHERE op_id = ?1",
                            params![sealed.op_id, sealed.key_id, sealed.sig],
                        )
                        .unwrap();
                }
                Some(_) => {
                    self.conn
                        .execute(
                            "UPDATE ops SET provenance = ?2 WHERE op_id = ?1",
                            params![op.op_id, Provenance::of_received(op).as_str()],
                        )
                        .unwrap();
                }
            }
        }
    }

    /// Resurface (§7): re-fold every foldable op in the durable log into the materialized views.
    /// Idempotent — the fold is keep-if-beats / observed-remove / grow-only, so re-applying an
    /// already-folded op is a no-op. After an upgrade that understands an op shape previously
    /// stored-but-skipped, this materializes those ops. The views are a pure fold of the log.
    pub fn refold(&mut self) {
        let ops = self.export();
        self.conn.execute_batch("BEGIN").unwrap();
        for op in &ops {
            if let Some(r) = self.folder_for(op) {
                r.fold(&self.conn, op);
            }
        }
        self.conn.execute_batch("COMMIT").unwrap();
    }

    /// The highest `ops.rowid` the `store_id` product store has folded into its views (0 if it has
    /// never folded). The snapshot boundary against which [`refold_if_behind`] decides whether the
    /// log advanced out-of-band (aye.36).
    pub fn folded_through(&self, store_id: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT folded_through FROM view_watermarks WHERE store_id = ?1",
                params![store_id],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }

    /// The current snapshot boundary of the durable log: its max `ops.rowid` (0 when empty).
    fn max_op_rowid(&self) -> i64 {
        self.conn
            .query_row("SELECT COALESCE(MAX(rowid), 0) FROM ops", [], |r| r.get(0))
            .unwrap()
    }

    /// Raise the bound store's folded-through watermark to `rowid` (monotonic: never lowers it).
    /// A no-op for a raw substrate store, which binds no `views_key`.
    fn advance_watermark(&self, rowid: i64) {
        if let Some(key) = &self.views_key {
            self.conn
                .execute(
                    "INSERT INTO view_watermarks(store_id, folded_through) VALUES(?1, ?2)
                     ON CONFLICT(store_id)
                       DO UPDATE SET folded_through = MAX(folded_through, excluded.folded_through)",
                    params![key, rowid],
                )
                .unwrap();
        }
    }

    /// Open hook (aye.36): bind this store to its `store_id` watermark, then — if the shared log
    /// advanced beyond it (foreign ops landed out-of-band, e.g. an nxs sync pull that moved the
    /// op-log without folding this product's views) — [`refold`] once and advance the watermark to
    /// the log's boundary. When already current it does nothing, so reads stay O(1) (no needless
    /// refold). Idempotent: the fold is keep-if-beats, so a refold can never change the converged
    /// value (guarded by the differential oracle). The product store calls this right after
    /// registering its reducer.
    pub fn refold_if_behind(&mut self, store_id: &str) {
        self.views_key = Some(store_id.to_string());
        let boundary = self.max_op_rowid();
        if boundary > self.folded_through(store_id) {
            self.refold();
            self.advance_watermark(boundary);
        }
    }

    /// This store's IMAGE (6j6v.mxt2): the whole log, the views of every registered reducer, and
    /// how far this store had folded — read in ONE transaction, so the three describe one state of
    /// the file even while another process writes to it. See [`crate::image`].
    pub fn image(&self) -> rusqlite::Result<crate::image::Image> {
        use crate::image;
        // A read transaction: the first SELECT fixes the snapshot every later one reads from.
        // Dropped rather than committed — it wrote nothing.
        let tx = self.conn.unchecked_transaction()?;
        let ops = image::dump(&tx, "ops", true)?;
        let mut views = Vec::new();
        for reducer in &self.reducers {
            for table in reducer.view_tables() {
                views.push(image::dump(&tx, table, false)?);
            }
        }
        let folded_through = match &self.views_key {
            Some(key) => self.folded_through(key),
            None => 0,
        };
        let schema_version = schema::schema_version(&tx)?;
        let min_compatible = schema::db_min_compatible(&tx)?;
        drop(tx);
        Ok(image::Image {
            engine: image::ENGINE_VERSION.to_string(),
            schema_version,
            min_compatible,
            ops,
            views_of: self.views_key.clone().filter(|_| !views.is_empty()),
            folded_through,
            views,
        })
    }

    /// Start this store from `image` (6j6v.mxt2): take its log, and take its views as they are when
    /// this engine would have folded exactly those — otherwise fold them again from the log it
    /// brought. The store must be EMPTY; nothing is written when this returns an error.
    ///
    /// **Views are taken only on a full match**: the same engine version (see
    /// [`ENGINE_VERSION`](crate::image::ENGINE_VERSION) for why a schema match is not enough), the
    /// same schema, the same product, the same tables. Anything else is [`Loaded::Refolded`] with the
    /// reason — never a view read as something it is not.
    ///
    /// **The log keeps its rowids and its shape.** The rowids come across unchanged, so the
    /// folded-through watermark names the same op here as where the image was taken. And the
    /// `(lamport, site)` index is rebuilt for the log that arrived: a log written before 6j6v.fc5p
    /// may hold two ops on one coordinate, which a fresh store's UNIQUE index would refuse — so the
    /// index takes the shape `ensure_op_coordinate_index` gives that log on any open, and this
    /// replica holds exactly what its source held.
    ///
    /// [`Loaded::Refolded`]: crate::image::Loaded::Refolded
    pub fn load_image(
        &mut self,
        image: &crate::image::Image,
    ) -> Result<crate::image::Loaded, crate::image::ImageError> {
        use crate::image::{self, ImageError, Loaded, RefoldReason};
        if image.min_compatible > schema::SCHEMA_VERSION {
            return Err(ImageError::TooNew {
                min_compatible: image.min_compatible,
                ours: schema::SCHEMA_VERSION,
            });
        }
        // Everything the log will be read back as is checked BEFORE anything is written — see
        // `image::typed_log` for why a load must be total over its input.
        let log = image::typed_log(&image.ops)?;
        // Verified before the write lock is taken: the checks are the slow part, and nothing they
        // read can change under a sibling process.
        let verdicts = image::verdicts(&log);
        let held = self.op_count();
        if held > 0 {
            return Err(ImageError::NotEmpty { ops: held });
        }
        let ours: HashSet<&str> = self
            .reducers
            .iter()
            .flat_map(|r| r.view_tables().iter().copied())
            .collect();
        let theirs: HashSet<&str> = image.views.iter().map(|t| t.name.as_str()).collect();
        let verdict = if image.views.is_empty() || image.views_of.is_none() {
            Err(RefoldReason::NoViews)
        } else if image.engine != image::ENGINE_VERSION {
            Err(RefoldReason::OtherEngine {
                written_by: image.engine.clone(),
            })
        } else if image.schema_version != schema::SCHEMA_VERSION {
            Err(RefoldReason::OtherSchema {
                written_at: image.schema_version,
            })
        } else if image.views_of != self.views_key
            || theirs != ours
            || !self.views_cover_every_column(image)?
        {
            Err(RefoldReason::OtherViews)
        } else {
            Ok(())
        };

        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match self.write_image(image, &log, &verdicts, verdict.is_ok()) {
            Ok(()) => self.conn.execute_batch("COMMIT")?,
            Err(e) => {
                // The root cause is what the caller must see; a failed ROLLBACK would mask it, and
                // SQLite rolls back an open transaction on close regardless.
                let _ = self.conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
        // The clock from the rows just checked, not from a read of the file: nothing that runs
        // after the commit may be able to fail on what the image put there. An image brings only
        // RECEIVED ops (the store was empty), so within the bound and under the ceiling, as for
        // any received op (6j6v.m19v).
        if let Some(highest) = log
            .iter()
            .map(|r| r.lamport)
            .filter(|&l| l <= crate::model::MAX_LAMPORT)
            .max()
        {
            self.clock = self.clock.max(highest.min(crate::model::CLOCK_CEILING));
        }

        match verdict {
            Ok(()) => {
                // Views that were behind their own log where the image was taken (another product
                // appended after this one last folded) catch up now, exactly as they would on the
                // next open — rather than leaving this handle behind until then.
                if let Some(key) = self.views_key.clone() {
                    let boundary = self.max_op_rowid();
                    if boundary > self.folded_through(&key) {
                        self.refold();
                        self.advance_watermark(boundary);
                    }
                }
                Ok(Loaded::Views)
            }
            Err(reason) => {
                self.refold();
                self.advance_watermark(self.max_op_rowid());
                Ok(Loaded::Refolded(reason))
            }
        }
    }

    /// The writing half of [`load_image`](Self::load_image), inside the transaction it holds: the
    /// log, the coordinate index in the shape that log calls for, and — when `with_views` — the
    /// views and the folded-through watermark.
    fn write_image(
        &self,
        image: &crate::image::Image,
        log: &[crate::image::LogRow],
        verdicts: &[Provenance],
        with_views: bool,
    ) -> Result<(), crate::image::ImageError> {
        use crate::image;
        // Asked again under the write lock: the check in `load_image` fails fast, this one is the
        // one a sibling process writing in between cannot slip past.
        let held = self.op_count();
        if held > 0 {
            return Err(image::ImageError::NotEmpty { ops: held });
        }
        // Without the index while the log goes in (faster, and a pre-fc5p duplicate coordinate must
        // not be refused), then rebuilt in the shape this log calls for. A rollback puts the
        // dropped index back with everything else.
        self.conn
            .execute_batch("DROP INDEX IF EXISTS ops_lamport_site;")?;
        image::insert_log(&self.conn, log, verdicts)?;
        schema::ensure_op_coordinate_index(&self.conn)?;
        if with_views {
            for table in &image.views {
                image::insert_view(&self.conn, &table.name, table)?;
            }
            if let Some(key) = &self.views_key {
                self.conn.execute(
                    "INSERT INTO view_watermarks(store_id, folded_through) VALUES(?1, ?2)
                     ON CONFLICT(store_id) DO UPDATE SET folded_through = excluded.folded_through",
                    params![key, image.folded_through],
                )?;
            }
        }
        Ok(())
    }

    /// Whether every column this store's view tables have is one the image carries.
    ///
    /// **Not equality, on purpose.** A database created long ago keeps columns its engine no longer
    /// writes — the additive migration rule never drops one; the real manufakt.io board still has
    /// `items.belongs_to` from before parenthood moved onto edges — and the engine that took the
    /// image never reads such a column, so leaving it behind loses nothing. A column THIS store has
    /// and the image lacks is the other case: one this engine reads and the image cannot fill.
    fn views_cover_every_column(&self, image: &crate::image::Image) -> rusqlite::Result<bool> {
        for table in &image.views {
            let carried: HashSet<&str> = table.columns.iter().map(String::as_str).collect();
            let needed = crate::image::columns_of(&self.conn, &table.name)?;
            if needed
                .keys()
                .any(|c| c != "rowid" && !carried.contains(c.as_str()))
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Force a one-time refold + watermark rebind after THIS product's **view schema changed** — it
    /// now folds an op shape a prior binary stored-but-skipped. Unlike [`refold_if_behind`] it
    /// refolds **unconditionally**: the behind-check is not enough here, because the prior binary
    /// refolded everything else and advanced the folded-through watermark **past** the skipped op's
    /// rowid, so `max_op_rowid == folded_through` and `refold_if_behind` would be a no-op while the
    /// op stays unfolded forever. The caller gates this on an actual schema change (a migration that
    /// added a column), so steady-state reopens still take the O(1) [`refold_if_behind`] path.
    /// Idempotent: the fold is keep-if-beats, so forcing a refold never changes a converged value.
    pub fn force_refold(&mut self, store_id: &str) {
        self.views_key = Some(store_id.to_string());
        self.refold();
        self.advance_watermark(self.max_op_rowid());
    }
}

/// The first Lamport number at or above `taken` that no op on `site` holds, given that `taken` is
/// held — the end of the squatted run that starts there, plus one (6j6v.m19v, review of PR #489).
/// One query however long the run: among the run's ops, the lowest one whose successor number is
/// free on this site.
fn first_free_coordinate(conn: &Connection, site: i64, taken: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT MIN(o.lamport) + 1 FROM ops o
          WHERE o.site = ?1 AND o.lamport >= ?2
            AND NOT EXISTS(SELECT 1 FROM ops n WHERE n.site = ?1 AND n.lamport = o.lamport + 1)",
        [site, taken],
        |r| r.get(0),
    )
}

/// The log's high-water mark — the one number a clock is seeded and refreshed from (6j6v.m19v):
/// the highest Lamport number within [`MAX_LAMPORT`](crate::model::MAX_LAMPORT), capped at
/// [`CLOCK_CEILING`](crate::model::CLOCK_CEILING) as any received op is, or this replica's OWN
/// highest op if that lies above it.
///
/// The second term is what keeps a replica writing once a received op has pushed it to the
/// ceiling: its own ops then climb past the ceiling, and the next write — in this handle or in a
/// sibling process — must mint above THEM, not at the ceiling again. It reads `provenance = 'own'`
/// rather than `site`, because a received op can claim this replica's site but never its
/// provenance, which only a local append records (6j6v.pzkb). Both terms are index lookups: a
/// range on the leading column of `ops_lamport_site`, and the partial `ops_own_lamport`.
fn high_water_mark(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT MAX(
             COALESCE((SELECT MIN(MAX(lamport), ?2) FROM ops WHERE lamport <= ?1), 0),
             COALESCE((SELECT MAX(lamport) FROM ops WHERE provenance = 'own' AND lamport <= ?1), 0))",
        [crate::model::MAX_LAMPORT, crate::model::CLOCK_CEILING],
        |r| r.get(0),
    )
}

impl Drop for Store {
    /// Touch the local-write marker ONCE per process, and only when something was actually
    /// emitted. Best-effort by design: the marker is a scheduling hint for the sync daemon,
    /// never a correctness input — a failed touch costs at most a slightly later push pass, so
    /// it must never turn a successful write into an error.
    fn drop(&mut self) {
        if !self.emitted_local {
            return;
        }
        if let Some(path) = &self.write_marker {
            touch_marker_no_follow(path);
        }
    }
}

/// Best-effort empty-file touch that REFUSES to follow a symlink at `path` (review finding
/// Integrity #1, PR #263): plain `std::fs::write` follows symlinks, so a symlink pre-placed at
/// `<dir>/last-write` (e.g. by a cloned repo) would turn every ordinary `nxf`/`nxm`/`nxc` write
/// into a truncate of whatever it points at — an arbitrary-file-write primitive triggered by
/// routine use. `write_atomic` (`crates/nxs::workspaces`) is the usual fix for this class of bug,
/// but this marker is written from `crates/foundation`, which `nxs` depends ON — reaching back
/// into `nxs` for it would invert the crate layering. The marker is a single always-empty file
/// touched from `Drop` (no meaningful content, no atomic-rename dance needed), so `O_NOFOLLOW` on
/// a direct open is the proportionate fix, mirroring `daemon.rs`'s `DaemonLock::acquire` (same
/// "hold/touch a fixed path directly" shape as opposed to the temp+rename writers).
///
/// `cfg(unix)` only: `O_NOFOLLOW` has no portable non-unix equivalent, so the fallback there is
/// the previous unguarded `fs::write` (no regression on those targets, which had no protection
/// before this fix either).
#[cfg(unix)]
fn touch_marker_no_follow(path: &Path) {
    use std::os::unix::fs::OpenOptionsExt;
    // Best-effort: any failure (including "it's a symlink", which is the attack this guards
    // against) is swallowed, exactly like the `fs::write` it replaces — see the rationale on
    // `Drop::drop` above.
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .custom_flags(nofollow::O_NOFOLLOW)
        .open(path);
}

#[cfg(not(unix))]
fn touch_marker_no_follow(path: &Path) {
    let _ = std::fs::write(path, b"");
}

/// The `O_NOFOLLOW` raw flag value, per-platform. Not exposed by `std`, and `libc` is not (and
/// should not become, for one flag) a dependency of this crate — these are the values every libc
/// implementation defines in `<fcntl.h>` for the given platform, stable ABI constants rather than
/// something that needs a crate to look up.
#[cfg(unix)]
mod nofollow {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub const O_NOFOLLOW: i32 = 0o400_000;
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    pub const O_NOFOLLOW: i32 = 0x0100;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A test reducer that folds any op of its domain into a `demo` table and counts folds — a
    /// minimal product over the substrate, enough to exercise the registry + op-log mechanics.
    struct DemoReducer {
        folds: Arc<AtomicUsize>,
    }
    impl Reducer for DemoReducer {
        fn domain(&self) -> &'static str {
            "demo"
        }
        fn is_foldable(&self, op: &Op) -> bool {
            op.target_kind == "demo"
        }
        fn fold(&self, conn: &Connection, op: &Op) {
            conn.execute_batch("CREATE TABLE IF NOT EXISTS demo(id TEXT PRIMARY KEY)")
                .unwrap();
            conn.execute("INSERT OR IGNORE INTO demo(id) VALUES(?1)", [&op.target_id])
                .unwrap();
            self.folds.fetch_add(1, Ordering::SeqCst);
        }
        fn clear_views(&self, conn: &Connection) {
            conn.execute_batch("DROP TABLE IF EXISTS demo").unwrap();
        }
    }

    fn demo_op(op_id: &str, kind: &str, domain: &str) -> Op {
        Op {
            op_id: op_id.into(),
            lamport: 1,
            site: 9,
            domain: domain.into(),
            target_kind: kind.into(),
            target_id: "d.1".into(),
            field: "x".into(),
            op_type: "set".into(),
            value: None,
            author: "u".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        }
    }

    /// The log is append-only BY CONSTRUCTION, not by the absence of a path that deletes (nxf
    /// 6j6v.hehx #3). A test over today's maintenance paths cannot see tomorrow's — "a later
    /// compaction job would pass every test today" — so the database refuses the deletion itself,
    /// whoever issues it.
    #[test]
    fn the_log_refuses_to_lose_an_op_whoever_asks() {
        let mut s = Store::open_in_memory(1);
        s.apply(std::slice::from_ref(&demo_op("o1", "demo", "demo")));
        let err = s
            .connection()
            .execute("DELETE FROM ops WHERE op_id='o1'", [])
            .expect_err("deleting an op must be refused");
        assert!(err.to_string().contains("append-only"), "{err}");
        // …and a REPLACE, which removes the row it replaces without a delete trigger unless the
        // connection has recursive triggers on (review of PR #487, Integrity #6).
        let err = s
            .connection()
            .execute(
                "INSERT OR REPLACE INTO ops(op_id, lamport, site, target_kind, target_id, field,
                                            op_type) VALUES ('o1', 99, 9, 'demo', 'x', 'x', 'set')",
                [],
            )
            .expect_err("replacing an op must be refused");
        assert!(err.to_string().contains("append-only"), "{err}");
        assert_eq!(s.op_count(), 1, "and the op is still there");
    }

    /// What an op IS never changes: its id (the dedup key), its `(lamport, site)` coordinate (its
    /// place in the CRDT order) and its author (attribution cannot be rewritten after the fact).
    /// The two maintenance rewrites that exist — the prefix remap and the legacy `belongs_to`
    /// migration — touch only the target and value columns, and must keep working.
    #[test]
    fn an_ops_identity_coordinate_and_author_never_change() {
        let mut s = Store::open_in_memory(1);
        s.apply(std::slice::from_ref(&demo_op("o1", "demo", "demo")));
        for set in ["op_id='o2'", "lamport=7", "site=8", "author='mallory'"] {
            let err = s
                .connection()
                .execute(&format!("UPDATE ops SET {set} WHERE op_id='o1'"), [])
                .expect_err(set);
            assert!(err.to_string().contains("never change"), "{set}: {err}");
        }
        s.connection()
            .execute(
                "UPDATE ops SET target_kind='edge', target_id='x', field='present', \
                 op_type='add', value=NULL WHERE op_id='o1'",
                [],
            )
            .expect("the shape a maintenance rewrite changes stays writable");
    }

    #[test]
    fn appends_and_dedups_ops_in_the_log() {
        let mut s = Store::open_in_memory(1);
        let op = demo_op("o1", "demo", "demo");
        s.apply(std::slice::from_ref(&op));
        s.apply(std::slice::from_ref(&op)); // idempotent union: same op_id, no growth
        assert_eq!(s.op_count(), 1, "the log is an idempotent union by op_id");
    }

    #[test]
    fn dispatches_fold_to_the_reducer_registered_for_the_op_domain() {
        let mut s = Store::open_in_memory(1);
        let folds = Arc::new(AtomicUsize::new(0));
        s.register_reducer(Box::new(DemoReducer {
            folds: folds.clone(),
        }));
        let deferred = s.apply(std::slice::from_ref(&demo_op("o1", "demo", "demo")));
        assert!(deferred.is_empty(), "the demo reducer folded its op");
        assert_eq!(folds.load(Ordering::SeqCst), 1, "fold reached the reducer");
    }

    #[test]
    fn an_op_with_no_registered_reducer_is_stored_not_folded() {
        // §7: an op in a domain the substrate has no reducer for is kept in the log (resurface
        // stays possible) but not materialized — never dropped, never panicked on.
        let mut s = Store::open_in_memory(1);
        let deferred = s.apply(std::slice::from_ref(&demo_op("o1", "demo", "nobody")));
        assert_eq!(deferred.len(), 1, "unknown domain defers");
        assert_eq!(s.op_count(), 1, "but the op IS persisted");
    }

    #[test]
    fn emit_appends_a_local_op_and_advances_the_clock() {
        let mut s = Store::open_in_memory(1);
        let folds = Arc::new(AtomicUsize::new(0));
        s.register_reducer(Box::new(DemoReducer {
            folds: folds.clone(),
        }));
        s.emit("demo", "demo", "d.1", "x", "set", None, "u");
        assert_eq!(s.clock(), 1, "lamport advanced once per emitted op");
        assert_eq!(folds.load(Ordering::SeqCst), 1, "the local op was folded");
    }

    #[test]
    fn every_emitted_op_carries_an_author() {
        // Invariant 1 (6j6v.xsf3): attribution is stamped at append, so it is readable off the
        // log itself — the property the E4 auth slice authenticates and the audit trail rests on.
        let mut s = Store::open_in_memory(1);
        s.register_reducer(Box::new(DemoReducer {
            folds: Arc::new(AtomicUsize::new(0)),
        }));
        s.emit("demo", "demo", "d.1", "x", "set", None, "alice");
        let ops = s.export();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].author, "alice", "the author rode the append");
    }

    #[test]
    #[should_panic(expected = "op author must not be blank")]
    fn emitting_a_blank_author_is_refused() {
        // Invariant 1 (6j6v.xsf3): an unattributed op is unrepairable — the log is append-only,
        // so there is no later write that can say who did this. Fail at the append instead.
        let mut s = Store::open_in_memory(1);
        s.register_reducer(Box::new(DemoReducer {
            folds: Arc::new(AtomicUsize::new(0)),
        }));
        s.emit("demo", "demo", "d.1", "x", "set", None, "   ");
    }

    #[test]
    fn merging_a_foreign_op_never_rewrites_or_refuses_its_author() {
        // The mirror of the append guard: `apply` is the MERGE path, and §7 forbids dropping a
        // foreign op. So the guard deliberately does not extend here — a peer's op keeps the
        // author the peer stamped, verbatim, whatever it is. Rejecting on merge would trade a
        // forward-compat guarantee for an enforcement the merge path cannot make anyway: until
        // E4 signs ops, `author` is self-declared either way, so refusing a blank one would buy
        // nothing and lose an op.
        let mut s = Store::open_in_memory(1);
        let mut op = demo_op("o1", "demo", "nobody");
        op.author = "peer-with-its-own-rules".into();
        s.apply(std::slice::from_ref(&op));
        assert_eq!(s.export()[0].author, "peer-with-its-own-rules");

        // Including a BLANK one — the case the local append asserts against. It is stored, not
        // refused (review finding Test Quality #4, PR #311: the comment claimed this, nothing
        // asserted it).
        let mut blank = demo_op("o2", "demo", "nobody");
        // Its own coordinate: `demo_op` stamps every op `(1, 9)`, and two ops from ONE site cannot
        // share one — the log refuses that outright since 6j6v.fc5p. The fixture, not the case
        // under test: what is being pinned here is the AUTHOR the merge path keeps.
        blank.lamport = 2;
        blank.author = "   ".into();
        s.apply(std::slice::from_ref(&blank));
        let stored = s.export();
        assert_eq!(stored.len(), 2, "the blank-authored foreign op was kept");
        assert_eq!(
            stored.iter().find(|o| o.op_id == "o2").unwrap().author,
            "   ",
            "verbatim — the merge path neither rewrites nor rejects a peer's author"
        );
    }

    #[test]
    fn data_version_is_stable_across_own_commits() {
        let mut s = Store::open_in_memory(1);
        s.register_reducer(Box::new(DemoReducer {
            folds: Arc::new(AtomicUsize::new(0)),
        }));
        let v0 = s.data_version().unwrap();
        s.emit("demo", "demo", "d.1", "x", "set", None, "u");
        assert_eq!(
            s.data_version().unwrap(),
            v0,
            "own commits never bump data_version"
        );
    }

    #[test]
    fn apply_advances_the_clock_past_foreign_ops() {
        let mut s = Store::open_in_memory(1);
        let mut op = demo_op("o1", "demo", "nobody");
        op.lamport = 42;
        s.apply(std::slice::from_ref(&op));
        // A subsequent local op must beat the merged-in lamport.
        s.register_reducer(Box::new(DemoReducer {
            folds: Arc::new(AtomicUsize::new(0)),
        }));
        s.emit("demo", "demo", "d.2", "x", "set", None, "u");
        assert_eq!(s.clock(), 43, "local op beats the highest observed lamport");
    }

    // ---- 6j6v.m19v: a foreign lamport past the bound is kept, but inert ------------------------

    /// A foreign op whose lamport sits past [`MAX_LAMPORT`](crate::model::MAX_LAMPORT) — the
    /// shape a hostile writer at a relay that authenticates nobody can plant — used to park the
    /// clock where the next local `+ 1` overflowed: a panic in a debug build, a wrap to `i64::MIN`
    /// in a release build, after which every local write lost every compare. §7 forbids dropping
    /// the op, so it is KEPT — in the log, reported like any other op this store did not fold —
    /// and it neither folds nor moves the clock.
    #[test]
    fn a_foreign_op_past_the_lamport_bound_is_kept_but_neither_folds_nor_moves_the_clock() {
        let folds = Arc::new(AtomicUsize::new(0));
        let mut s = Store::open_in_memory(1);
        s.register_reducer(Box::new(DemoReducer {
            folds: folds.clone(),
        }));
        s.emit("demo", "demo", "d.1", "x", "set", None, "u");
        for (n, lamport) in [crate::model::MAX_LAMPORT + 1, i64::MAX]
            .into_iter()
            .enumerate()
        {
            let mut hostile = demo_op(&format!("hostile-{n}"), "demo", "demo");
            hostile.target_id = format!("d.hostile.{n}");
            hostile.lamport = lamport;
            let not_folded = s.apply(std::slice::from_ref(&hostile));
            assert_eq!(
                not_folded,
                vec![hostile],
                "reported as not taken into the views"
            );
        }
        assert_eq!(s.op_count(), 3, "both are kept in the log (§7)");
        assert_eq!(folds.load(Ordering::SeqCst), 1, "only the local op folded");
        assert_eq!(demo_count(&s), 1, "no view row for either");
        assert_eq!(s.clock(), 1, "and the clock did not move");

        s.emit("demo", "demo", "d.2", "x", "set", None, "u");
        assert_eq!(
            s.clock(),
            2,
            "the next local write neither panics nor wraps"
        );

        s.refold();
        assert_eq!(demo_count(&s), 2, "a refold does not fold them either");
    }

    /// The attack moved to the bound: an op ON [`MAX_LAMPORT`](crate::model::MAX_LAMPORT) folds, and
    /// if it moved the clock there, every later local op would land above the bound and be inert — a
    /// frozen board. So a received op moves the clock only to
    /// [`CLOCK_CEILING`](crate::model::CLOCK_CEILING), and local writes go on folding from there.
    #[test]
    fn an_op_on_the_bound_folds_but_moves_the_clock_only_to_the_ceiling() {
        use crate::model::{CLOCK_CEILING, MAX_LAMPORT};
        let (mut s, _) = tally_store(1);
        let edge = tally_op("edge", "t.edge", "planted", MAX_LAMPORT, 9);
        assert!(s.apply(std::slice::from_ref(&edge)).is_empty(), "folded");
        assert_eq!(s.clock(), CLOCK_CEILING);
        s.emit(
            "tally",
            "tally",
            "t.mine",
            "v",
            "set",
            Some("mine".into()),
            "u",
        );
        assert_eq!(s.clock(), CLOCK_CEILING + 1);
        assert!(
            tally_rows(&s)
                .iter()
                .any(|r| r.0 == "t.mine" && r.1 == "mine"),
            "the local write after it folded: the board is not frozen"
        );
    }

    /// The price of the ceiling, pinned so it stays a stated one: an op between the ceiling and the
    /// bound keeps winning its ONE field against later local writes. It can pin that field — it
    /// cannot stop any other write from landing.
    #[test]
    fn an_op_between_the_ceiling_and_the_bound_pins_its_one_field() {
        use crate::model::MAX_LAMPORT;
        let (mut s, _) = tally_store(1);
        s.apply(&[tally_op("pin", "t.1", "planted", MAX_LAMPORT, 9)]);
        s.emit(
            "tally",
            "tally",
            "t.1",
            "v",
            "set",
            Some("mine".into()),
            "u",
        );
        s.emit(
            "tally",
            "tally",
            "t.2",
            "v",
            "set",
            Some("mine".into()),
            "u",
        );
        let rows = tally_rows(&s);
        assert_eq!(
            rows[0].1, "planted",
            "the pinned field keeps the planted value"
        );
        assert_eq!(rows[1].1, "mine", "every other field takes local writes");
    }

    /// A received op may claim this replica's site, on the very coordinate `tick` hands out next
    /// once the clock sits at the ceiling. The local write steps over it instead of colliding — the
    /// log would refuse the coordinate, and the write must not be lost or panic.
    #[test]
    fn a_received_op_squatting_the_next_local_coordinate_is_stepped_over() {
        use crate::model::CLOCK_CEILING;
        let (mut s, _) = tally_store(1);
        s.apply(&[
            tally_op("push", "t.9", "x", CLOCK_CEILING, 9),
            tally_op("squat-1", "t.8", "x", CLOCK_CEILING + 1, 1),
            tally_op("squat-2", "t.8", "x", CLOCK_CEILING + 2, 1),
        ]);
        assert_eq!(s.clock(), CLOCK_CEILING);
        s.emit(
            "tally",
            "tally",
            "t.1",
            "v",
            "set",
            Some("mine".into()),
            "u",
        );
        let mine = s
            .export()
            .into_iter()
            .find(|o| o.target_id == "t.1")
            .unwrap();
        assert_eq!((mine.lamport, mine.site), (CLOCK_CEILING + 3, 1));
        assert_eq!(coordinate_collisions(&s), 0);
    }

    /// However LONG the squatted run, the local write jumps past it in one step (review of PR #489,
    /// Integrity #1): a relay-level writer can plant any number of ops on this replica's site, and a
    /// retry per op would hold the exclusive write lock for as long as it liked. The run here is
    /// placed straight into the log — building it through `apply` would verify thousands of
    /// signatures just to set the stage. What is counted is the insert attempts on `ops` the write
    /// PREPARES (SQLite's authorizer runs once per statement prepared, and each attempt prepares its
    /// own): two — the collision and the free coordinate — however long the run, where a retry per
    /// number would prepare one per squatted op.
    #[test]
    fn a_long_squatted_run_is_jumped_in_one_step() {
        use crate::model::CLOCK_CEILING;
        const RUN: i64 = 5_000;
        let (mut s, _) = tally_store(1);
        s.apply(&[tally_op("push", "t.9", "x", CLOCK_CEILING, 9)]);
        let tx = s.connection().unchecked_transaction().unwrap();
        for n in 1..=RUN {
            tx.execute(
                "INSERT INTO ops(op_id, lamport, site, domain, target_kind, target_id, field,
                                 op_type, value, author, wall_clock)
                 VALUES (?1, ?2, 1, 'tally', 'tally', 't.8', 'v', 'set', 'x', 'u', '')",
                params![format!("squat-{n}"), CLOCK_CEILING + n],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&attempts);
        s.connection()
            .authorizer(Some(move |ctx: rusqlite::hooks::AuthContext<'_>| {
                if matches!(
                    ctx.action,
                    rusqlite::hooks::AuthAction::Insert { table_name: "ops" }
                ) {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
                rusqlite::hooks::Authorization::Allow
            }));
        s.emit(
            "tally",
            "tally",
            "t.1",
            "v",
            "set",
            Some("mine".into()),
            "u",
        );
        s.connection().authorizer(
            None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>,
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "one collision, one jump"
        );
        let mine = s
            .export()
            .into_iter()
            .find(|o| o.target_id == "t.1")
            .unwrap();
        assert_eq!((mine.lamport, mine.site), (CLOCK_CEILING + RUN + 1, 1));
        s.emit(
            "tally",
            "tally",
            "t.1",
            "v",
            "set",
            Some("again".into()),
            "u",
        );
        assert_eq!(
            s.clock(),
            CLOCK_CEILING + RUN + 2,
            "and the clock stays past it"
        );
    }

    /// Once past the ceiling, the next write mints above this replica's OWN ops — in a sibling
    /// handle too, which only has the log to go by. A received op claiming this replica's site
    /// high above does not drag it along: the own high-water mark reads `provenance = 'own'`, which
    /// only a local append records.
    #[test]
    fn past_the_ceiling_the_clock_follows_this_replicas_own_ops_and_nothing_that_claims_them() {
        use crate::model::{CLOCK_CEILING, MAX_LAMPORT};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        let path = path.to_str().unwrap();
        let mut a = Store::open(path, 1).unwrap();
        a.connection()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS tally(id TEXT PRIMARY KEY, v TEXT, v_l INTEGER, v_s INTEGER)",
            )
            .unwrap();
        a.register_reducer(Box::new(TallyReducer {
            folds: Arc::new(AtomicUsize::new(0)),
        }));
        a.apply(&[
            tally_op("push", "t.9", "x", MAX_LAMPORT, 9),
            tally_op("claims-our-site", "t.8", "x", MAX_LAMPORT - 5, 1),
        ]);
        a.emit("tally", "tally", "t.1", "v", "set", Some("1".into()), "u");
        a.emit("tally", "tally", "t.1", "v", "set", Some("2".into()), "u");
        assert_eq!(a.clock(), CLOCK_CEILING + 2);

        let b = Store::open(path, 1).unwrap();
        assert_eq!(
            b.clock(),
            CLOCK_CEILING + 2,
            "a sibling seeds from this replica's own ops"
        );
    }

    /// A log that already holds such an op — pulled by a build before this fix, or by any process
    /// before this handle opened — does not seed the clock with it either: the seed at open and the
    /// refresh under the write lock both read the high-water mark of the FOLDABLE log.
    #[test]
    fn a_reopened_log_holding_a_hostile_lamport_still_mints_above_its_real_ops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        let path = path.to_str().unwrap();
        {
            let mut s = Store::open(path, 1).unwrap();
            s.register_reducer(Box::new(DemoReducer {
                folds: Arc::new(AtomicUsize::new(0)),
            }));
            s.emit("demo", "demo", "d.1", "x", "set", None, "u");
            let mut hostile = demo_op("hostile", "demo", "demo");
            hostile.lamport = i64::MAX;
            s.apply(std::slice::from_ref(&hostile));
        }
        let mut s = Store::open(path, 1).unwrap();
        assert_eq!(s.clock(), 1, "the seed at open skips it");
        s.register_reducer(Box::new(DemoReducer {
            folds: Arc::new(AtomicUsize::new(0)),
        }));
        s.emit("demo", "demo", "d.2", "x", "set", None, "u");
        assert_eq!(s.clock(), 2, "and so does the refresh at emit");
    }

    /// A store that took such an op over the wire can still be imaged, and the image loads: the
    /// image keeps what the wire keeps, and holds it just as inert. Refusing it instead (as the
    /// image did while only its own path was bounded) would let one planted op make every snapshot
    /// of a board unloadable.
    #[test]
    fn a_store_holding_a_hostile_lamport_images_and_loads_with_the_op_inert() {
        let (mut src, _) = tally_store(1);
        src.emit("tally", "tally", "t.1", "x", "set", Some("1".into()), "u");
        let mut hostile = src.export().pop().unwrap();
        hostile.op_id = "hostile".into();
        hostile.site = 9;
        hostile.value = Some("planted".into());
        hostile.lamport = i64::MAX;
        src.apply(std::slice::from_ref(&hostile));
        let image = src.image().unwrap();

        let (mut dst, _) = tally_store(2);
        dst.load_image(&image).expect("the image loads");
        assert_eq!(dst.op_count(), 2, "the op came across with the log");
        assert_eq!(tally_rows(&src)[0].1, "1", "the source never folded it");
        assert_eq!(
            tally_rows(&dst),
            tally_rows(&src),
            "and neither did the load"
        );
        assert_eq!(dst.clock(), 1, "nor did it seed the clock");
        dst.emit("tally", "tally", "t.1", "x", "set", Some("2".into()), "u");
        assert_eq!(dst.clock(), 2);
    }

    // ---- folded-through watermark + refold-when-behind (aye.36) ----------------

    /// Number of rows the demo reducer has materialized (0 if it never folded — its table may not
    /// exist yet).
    fn demo_count(s: &Store) -> i64 {
        s.connection()
            .query_row("SELECT COUNT(*) FROM demo", [], |r| r.get(0))
            .unwrap_or(0)
    }

    /// Append a demo op straight into the log, BYPASSING `ingest` — i.e. it lands unfolded and
    /// without advancing any watermark. Models a foundation-only sync pull that moved the shared
    /// op-log but did not fold this product's views (aye.36).
    fn append_out_of_band(s: &Store, op_id: &str, target_id: &str) {
        s.connection()
            .execute(
                "INSERT INTO ops
                   (op_id, lamport, site, domain, target_kind, target_id, field, op_type, value, author, wall_clock)
                 VALUES (?1, 1, 9, 'demo', 'demo', ?2, 'x', 'set', NULL, 'u', '')",
                params![op_id, target_id],
            )
            .unwrap();
    }

    #[test]
    fn refold_if_behind_folds_ops_that_landed_out_of_band() {
        // The multi-device case: a pull appended ops to the shared log without this store's reducer
        // folding them. Opening the product store refolds the log into the views once — the
        // folded-through watermark sits behind the log's max rowid, so the open detects it.
        let mut s = Store::open_in_memory(1);
        let folds = Arc::new(AtomicUsize::new(0));
        s.register_reducer(Box::new(DemoReducer {
            folds: folds.clone(),
        }));
        s.refold_if_behind("demo"); // bind the views key; an empty log has nothing to refold
        assert_eq!(s.folded_through("demo"), 0, "fresh log → watermark at 0");

        append_out_of_band(&s, "o1", "d.1");
        assert_eq!(demo_count(&s), 0, "the out-of-band op is not yet folded");

        s.refold_if_behind("demo");
        assert_eq!(
            demo_count(&s),
            1,
            "refold-on-open materialized the out-of-band op"
        );
        assert_eq!(
            folds.load(Ordering::SeqCst),
            1,
            "the reducer folded it once"
        );
        assert_eq!(
            s.folded_through("demo"),
            1,
            "the watermark advanced to the log's max rowid"
        );
    }

    #[test]
    fn force_refold_folds_a_skipped_op_the_watermark_already_passed() {
        // The view-schema-bump case (nexus-chat M2 §7): a prior binary could not fold an op shape,
        // so it stayed unfolded — but the prior binary refolded everything else and advanced the
        // watermark PAST it. Now the reducer understands the shape, yet refold_if_behind is a no-op
        // (caught up). force_refold must still resurface it.
        let mut s = Store::open_in_memory(1);
        // Phase 1 — "prior binary": no reducer registered, so the op is unfoldable. The out-of-band
        // op lands, then refold_if_behind advances the watermark to the log boundary WITHOUT folding.
        append_out_of_band(&s, "o1", "d.1");
        s.refold_if_behind("demo");
        assert_eq!(
            s.folded_through("demo"),
            1,
            "watermark advanced past the op"
        );
        assert_eq!(
            demo_count(&s),
            0,
            "…but the op is unfolded (no reducer yet)"
        );

        // Phase 2 — "upgraded binary": the reducer now understands the shape.
        let folds = Arc::new(AtomicUsize::new(0));
        s.register_reducer(Box::new(DemoReducer {
            folds: folds.clone(),
        }));
        s.refold_if_behind("demo");
        assert_eq!(
            demo_count(&s),
            0,
            "refold_if_behind is a no-op — max rowid == folded_through, the bug M2 §7 warns about"
        );

        s.force_refold("demo");
        assert_eq!(
            demo_count(&s),
            1,
            "force_refold resurfaces the previously-skipped op"
        );
        assert_eq!(
            s.folded_through("demo"),
            1,
            "the watermark is (re)bound at the log boundary"
        );
    }

    #[test]
    fn emit_unfolded_writes_a_durable_op_that_materializes_nothing() {
        // The shape the judging migration's once-per-stream mark needs (nxf 6j6v.9yaj): durable in
        // the log, carried over the sync wire, and a row in nobody's view.
        let mut s = Store::open_in_memory(1);
        let folds = Arc::new(AtomicUsize::new(0));
        s.register_reducer(Box::new(DemoReducer {
            folds: folds.clone(),
        }));
        s.emit_unfolded(
            "demo",
            "not-demo",
            "d.1",
            "mark",
            "set",
            Some("v".into()),
            "alice",
        );

        assert_eq!(folds.load(Ordering::SeqCst), 0, "no reducer folded it");
        assert_eq!(s.export().len(), 1, "and it is in the durable log");
        assert_eq!(s.export()[0].value.as_deref(), Some("v"));
        // A refold does not resurface it either — this build understands the shape and declines it.
        s.refold();
        assert_eq!(folds.load(Ordering::SeqCst), 0);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "emit_unfolded must build an op no registered reducer folds")]
    fn emit_unfolded_refuses_an_op_a_reducer_actually_folds() {
        // PR #289 review, Test Quality #3: the misuse guard was never exercised. It is the mirror
        // of `emit`'s own assert, and it is what makes the "materializes nothing" claim above a
        // guarantee rather than a coincidence of today's `is_foldable`.
        let mut s = Store::open_in_memory(1);
        s.register_reducer(Box::new(DemoReducer {
            folds: Arc::new(AtomicUsize::new(0)),
        }));
        s.emit_unfolded("demo", "demo", "d.1", "x", "set", None, "alice");
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "emit must only build fresh, foldable ops")]
    fn emit_refuses_an_op_no_reducer_folds() {
        // The other direction of the same pair — a product writing through an unregistered reducer
        // is the case `emit`'s assert has always been for, and it stays loud.
        let mut s = Store::open_in_memory(1);
        s.emit("demo", "demo", "d.1", "x", "set", None, "alice");
    }

    #[test]
    fn refold_if_behind_is_a_noop_once_caught_up() {
        // "otherwise reads stay O(1)": a re-open with no new ops must NOT refold.
        let mut s = Store::open_in_memory(1);
        let folds = Arc::new(AtomicUsize::new(0));
        s.register_reducer(Box::new(DemoReducer {
            folds: folds.clone(),
        }));
        append_out_of_band(&s, "o1", "d.1");
        s.refold_if_behind("demo");
        assert_eq!(folds.load(Ordering::SeqCst), 1, "first open catches up");

        s.refold_if_behind("demo");
        s.refold_if_behind("demo");
        assert_eq!(
            folds.load(Ordering::SeqCst),
            1,
            "no needless refold while the watermark is current"
        );
    }

    #[test]
    fn the_folded_through_watermark_is_per_store_id() {
        // flow and memory fold the SAME shared log into different views; each tracks its own
        // progress, so a store that is caught up must not stop a sibling store from refolding.
        let mut s = Store::open_in_memory(1);
        let folds = Arc::new(AtomicUsize::new(0));
        s.register_reducer(Box::new(DemoReducer {
            folds: folds.clone(),
        }));
        append_out_of_band(&s, "o1", "d.1");

        s.refold_if_behind("flow");
        let after_flow = folds.load(Ordering::SeqCst);
        assert_eq!(
            after_flow, 1,
            "the first store id refolds the out-of-band op"
        );
        s.refold_if_behind("flow");
        assert_eq!(
            folds.load(Ordering::SeqCst),
            after_flow,
            "flow is now current"
        );

        // A DIFFERENT store id carries its own (still-zero) watermark → it is behind and refolds.
        s.refold_if_behind("memory");
        assert!(
            folds.load(Ordering::SeqCst) > after_flow,
            "a second store id tracks its own watermark, independent of the first"
        );
        assert_eq!(
            s.folded_through("flow"),
            s.folded_through("memory"),
            "both ids end caught up to the same log boundary"
        );
    }

    #[test]
    fn a_locally_emitted_op_advances_the_bound_watermark() {
        // emit folds inline; the bound watermark must advance with it, so the next open sees itself
        // current (the single-device path stays O(1), no refold).
        let mut s = Store::open_in_memory(1);
        let folds = Arc::new(AtomicUsize::new(0));
        s.register_reducer(Box::new(DemoReducer {
            folds: folds.clone(),
        }));
        s.refold_if_behind("demo"); // bind
        s.emit("demo", "demo", "d.1", "x", "set", None, "u");
        s.emit("demo", "demo", "d.2", "x", "set", None, "u");
        assert_eq!(
            folds.load(Ordering::SeqCst),
            2,
            "both local ops folded inline"
        );
        assert_eq!(
            s.folded_through("demo"),
            2,
            "the watermark tracks the local appends"
        );

        s.refold_if_behind("demo");
        assert_eq!(
            folds.load(Ordering::SeqCst),
            2,
            "locally-emitted ops already advanced the watermark — no refold on the next open"
        );
    }

    #[test]
    fn applied_foreign_ops_advance_the_bound_watermark() {
        // The flow store that nxs sync drives applies foreign ops; its watermark must advance so a
        // later flow open does not refold what it already folded on the pull.
        let mut s = Store::open_in_memory(1);
        let folds = Arc::new(AtomicUsize::new(0));
        s.register_reducer(Box::new(DemoReducer {
            folds: folds.clone(),
        }));
        s.refold_if_behind("demo"); // bind
        s.apply(std::slice::from_ref(&demo_op("o1", "demo", "demo")));
        assert_eq!(
            folds.load(Ordering::SeqCst),
            1,
            "the applied op folded inline"
        );

        s.refold_if_behind("demo");
        assert_eq!(
            folds.load(Ordering::SeqCst),
            1,
            "apply advanced the watermark — no refold on the next open"
        );
    }

    #[test]
    fn concurrent_opens_of_a_fresh_db_do_not_race_the_migration() {
        // #9e9t: two+ connections opening a freshly-created db AT THE SAME TIME (the change
        // watcher's re-arm racing a same-path recreate's own open, or `nxf init` racing a watching
        // app) must all succeed. Without an atomic migrate, each reads `user_version = 0` before any
        // has stamped, so each runs `ALTER TABLE ops ADD COLUMN domain` and every loser hits
        // "duplicate column name: domain". Repeated over fresh dbs to make the narrow window reliable.
        use std::sync::{Arc, Barrier};
        let tmp = tempfile::TempDir::new().unwrap();
        for i in 0..25 {
            let path = tmp.path().join(format!("db{i}.sqlite"));
            let p = path.to_string_lossy().into_owned();
            let n = 6;
            let barrier = Arc::new(Barrier::new(n));
            let handles: Vec<_> = (0..n)
                .map(|_| {
                    let p = p.clone();
                    let b = Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        b.wait(); // release all opens together, into the migration
                        Store::open(&p, 1).map(|_| ())
                    })
                })
                .collect();
            for h in handles {
                h.join()
                    .unwrap()
                    .expect("a concurrent open must not race the schema migration");
            }
        }
    }

    // ---- local-write marker (n4dn) -----------------------------------------------------

    #[test]
    fn a_local_emit_touches_the_write_marker_when_the_store_is_dropped() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("db.sqlite");
        let marker = tmp.path().join("last-write");
        {
            let mut s = Store::open(db.to_str().unwrap(), 1).unwrap();
            s.register_reducer(Box::new(DemoReducer {
                folds: Arc::new(AtomicUsize::new(0)),
            }));
            s.emit("demo", "demo", "d.1", "x", "set", None, "u");
            assert!(!marker.exists(), "touched on drop, not on every emit");
        }
        assert!(marker.exists(), "the daemon's write signal");
    }

    #[test]
    fn a_store_that_only_merged_foreign_ops_leaves_no_marker() {
        // A pull folds through `apply`, never `emit` — so the daemon's own pass cannot nudge
        // itself into a loop.
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("db.sqlite");
        let marker = tmp.path().join("last-write");
        {
            let mut s = Store::open(db.to_str().unwrap(), 1).unwrap();
            s.register_reducer(Box::new(DemoReducer {
                folds: Arc::new(AtomicUsize::new(0)),
            }));
            s.apply(std::slice::from_ref(&demo_op("o1", "demo", "demo")));
        }
        assert!(!marker.exists());
    }

    #[test]
    fn an_in_memory_store_has_no_marker_to_touch() {
        let mut s = Store::open_in_memory(1);
        s.register_reducer(Box::new(DemoReducer {
            folds: Arc::new(AtomicUsize::new(0)),
        }));
        s.emit("demo", "demo", "d.1", "x", "set", None, "u");
        drop(s); // must not panic and must not write anywhere
    }

    #[cfg(unix)]
    #[test]
    fn drop_refuses_a_pre_placed_symlink_at_the_write_marker_and_leaves_its_target_untouched() {
        // Review finding Integrity #1 (PR #263): a symlink planted at `<dir>/last-write` (e.g. by
        // a cloned repo) must not be followed by the drop-time marker touch — mirrors
        // `daemon.rs`'s `the_lock_refuses_a_pre_placed_symlink_and_leaves_its_target_untouched`,
        // which is the same "direct open on a fixed path" shape (as opposed to the
        // `write_atomic` temp+rename writers' symlink tests).
        use std::os::unix::fs::symlink;
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("db.sqlite");
        let marker = tmp.path().join("last-write");

        let outside = tmp.path().join("precious.txt");
        std::fs::write(&outside, b"PRECIOUS").unwrap();
        symlink(&outside, &marker).unwrap();

        {
            let mut s = Store::open(db.to_str().unwrap(), 1).unwrap();
            s.register_reducer(Box::new(DemoReducer {
                folds: Arc::new(AtomicUsize::new(0)),
            }));
            s.emit("demo", "demo", "d.1", "x", "set", None, "u");
        } // drop attempts the touch here

        assert_eq!(
            std::fs::read(&outside).unwrap(),
            b"PRECIOUS",
            "the symlink target is never opened, truncated, or written through"
        );
        assert!(
            std::fs::symlink_metadata(&marker).unwrap().is_symlink(),
            "the symlink itself is left exactly as planted, not replaced"
        );
    }

    // ---- 6j6v.fc5p: the clock is the replica's, not the handle's -------------------------------

    /// The unit-level statement of the bug's mechanism: a SECOND handle over the same file writes,
    /// and the first handle — already open, never told — must still mint above it. Two handles in
    /// one process are the same divergence two processes have (each `Store` owns its own connection
    /// and its own in-memory clock); the two-process proof this cannot fake lives in
    /// `crates/cli/tests/lamport_clock_two_process.rs`.
    #[test]
    fn a_handle_mints_above_what_a_sibling_handle_wrote_after_it_opened() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("db.sqlite");
        let path = path.to_str().unwrap();

        let folds = Arc::new(AtomicUsize::new(0));
        let mut long_lived = Store::open(path, 7).unwrap();
        long_lived.register_reducer(Box::new(DemoReducer {
            folds: Arc::clone(&folds),
        }));
        long_lived.emit("demo", "demo", "d.1", "x", "set", None, "u");
        let first = long_lived.clock();

        // A sibling handle over the SAME file, same replica — the shape a `nxc reply` subprocess
        // has beside a long-lived `Engine`. It writes three ops the long-lived handle never sees.
        {
            let mut sibling = Store::open(path, 7).unwrap();
            sibling.register_reducer(Box::new(DemoReducer {
                folds: Arc::clone(&folds),
            }));
            for i in 0..3 {
                sibling.emit("demo", "demo", &format!("s.{i}"), "x", "set", None, "u");
            }
            assert_eq!(sibling.clock(), first + 3);
        }

        long_lived.emit("demo", "demo", "d.2", "x", "set", None, "u");
        assert_eq!(
            long_lived.clock(),
            first + 4,
            "the long-lived handle mints ABOVE the sibling's ops, not on top of them"
        );
        assert_eq!(
            coordinate_collisions(&long_lived),
            0,
            "and no two ops share a (lamport, site) coordinate"
        );
    }

    /// The net under the clock: the log itself refuses a coordinate a different op already holds,
    /// and `apply` reports the refusal instead of letting `INSERT OR IGNORE` pass it off as a
    /// re-delivery. Reachable only when two replicas share a `site_id` — forged here directly,
    /// because 63 bits of ULID entropy make it unreachable between real replicas.
    #[test]
    fn a_foreign_op_on_a_taken_coordinate_is_refused_and_reported() {
        let mut s = Store::open_in_memory(1);
        s.register_reducer(Box::new(DemoReducer {
            folds: Arc::new(AtomicUsize::new(0)),
        }));
        s.emit("demo", "demo", "d.1", "x", "set", None, "u");
        let mine = s.export().pop().unwrap();

        // Same coordinate, different op — what a replica sharing our site would ship.
        let impostor = Op {
            op_id: "forged-op-id".into(),
            target_id: "d.2".into(),
            ..mine.clone()
        };
        let not_taken = s.apply(std::slice::from_ref(&impostor));
        assert_eq!(
            not_taken.len(),
            1,
            "the refusal is reported, not swallowed as a re-delivery"
        );
        assert_eq!(not_taken[0].op_id, "forged-op-id");
        assert_eq!(s.op_count(), 1, "and the log kept the coordinate's owner");

        // A genuine re-delivery of an op already held stays the silent no-op it should be.
        assert!(
            s.apply(std::slice::from_ref(&mine)).is_empty(),
            "the idempotent union is untouched by the distinction"
        );
    }

    /// A fresh log gets the coordinate index as UNIQUE. Pinned because everything above rests on
    /// it: without the uniqueness there is no net, and without `lamport` LEADING it the per-op
    /// `MAX(lamport)` would be a table scan.
    #[test]
    fn a_fresh_log_gets_a_unique_coordinate_index_led_by_lamport() {
        let s = Store::open_in_memory(1);
        assert_eq!(coordinate_index_shape(s.connection()), Some(true));
        let cols: Vec<String> = s
            .connection()
            .prepare("SELECT name FROM pragma_index_info('ops_lamport_site') ORDER BY seqno")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(cols, ["lamport", "site"]);
    }

    /// A log that ALREADY carries colliding coordinates — every board written before this fix may —
    /// still opens. It gets the same index without the uniqueness, because `CREATE UNIQUE INDEX`
    /// over such a log fails and a failure in the open path would brick exactly the workspaces that
    /// prove the bug. Degraded, not refused, and not silently identical either.
    #[test]
    fn a_log_that_already_collides_opens_with_the_index_degraded_not_unique() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("db.sqlite");
        let path = path.to_str().unwrap();
        {
            // Write the damage the way a pre-fix binary did: two ops, one coordinate. Done against
            // a raw connection because the fixed `Store` can no longer produce it.
            let store = Store::open(path, 1).unwrap();
            let conn = store.connection();
            conn.execute_batch("DROP INDEX ops_lamport_site").unwrap();
            for op_id in ["op-a", "op-b"] {
                conn.execute(
                    "INSERT INTO ops(op_id, lamport, site, domain, target_kind, target_id, field,
                                     op_type, value, author, wall_clock)
                     VALUES (?1, 51, 1, 'demo', 'demo', 'd.1', 'x', 'set', NULL, 'u', '')",
                    [op_id],
                )
                .unwrap();
            }
        }

        let reopened = Store::open(path, 1).expect("a damaged log still opens");
        assert_eq!(
            coordinate_index_shape(reopened.connection()),
            Some(false),
            "the index is there for the reads, without a uniqueness it cannot honestly claim"
        );
        assert_eq!(reopened.op_count(), 2, "and neither op was dropped");
        // The clock still recovers from the damaged log, so writing continues from above it.
        assert_eq!(reopened.clock(), 51);
    }

    /// How many `(lamport, site)` coordinates are held by more than one op — 0 is the invariant.
    fn coordinate_collisions(store: &Store) -> i64 {
        store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM (SELECT 1 FROM ops GROUP BY lamport, site HAVING COUNT(*) > 1)",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// `Some(true)` when `ops_lamport_site` exists and is UNIQUE, `Some(false)` when it exists
    /// degraded, `None` when it is absent.
    fn coordinate_index_shape(conn: &Connection) -> Option<bool> {
        conn.prepare(
            "SELECT \"unique\" FROM pragma_index_list('ops') WHERE name='ops_lamport_site'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .next()
        .map(|v| v.unwrap() == 1)
    }

    #[test]
    fn open_sets_a_busy_timeout_so_a_locked_db_waits_not_fails() {
        // Review (#76u.7): the substrate open path (used by auto-init's `ensure_substrate_db`) must
        // set a busy_timeout, so a first start that momentarily races another writer on the SHARED
        // App-Data-Home board waits rather than failing instantly with "database is locked".
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("db.sqlite");
        let store = Store::open(path.to_str().unwrap(), 1).unwrap();
        let ms: i64 = store
            .connection()
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ms, 5000, "Store::open sets the 5s busy_timeout");
    }

    // ---- images (6j6v.mxt2) ------------------------------------------------------------------

    /// A product in miniature for the image tests: one LWW register per id, keep-if-beats on
    /// `(lamport, site)` like every real register, a table created up front (a real product creates
    /// its views at open) and a fold counter — the counter is how a test tells "the views were
    /// taken" from "the views were folded again".
    struct TallyReducer {
        folds: Arc<AtomicUsize>,
    }
    impl Reducer for TallyReducer {
        fn domain(&self) -> &'static str {
            "tally"
        }
        fn is_foldable(&self, op: &Op) -> bool {
            op.target_kind == "tally"
        }
        fn fold(&self, conn: &Connection, op: &Op) {
            conn.execute(
                "INSERT INTO tally(id, v, v_l, v_s) VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET v=excluded.v, v_l=excluded.v_l, v_s=excluded.v_s
                 WHERE (excluded.v_l, excluded.v_s) > (v_l, v_s)",
                params![op.target_id, op.value, op.lamport, op.site],
            )
            .unwrap();
            self.folds.fetch_add(1, Ordering::SeqCst);
        }
        fn view_tables(&self) -> &'static [&'static str] {
            &["tally"]
        }
    }

    fn tally_store(site: i64) -> (Store, Arc<AtomicUsize>) {
        let mut s = Store::open_in_memory(site);
        s.connection()
            .execute_batch(
                "CREATE TABLE tally(id TEXT PRIMARY KEY, v TEXT, v_l INTEGER, v_s INTEGER)",
            )
            .unwrap();
        let folds = Arc::new(AtomicUsize::new(0));
        s.register_reducer(Box::new(TallyReducer {
            folds: folds.clone(),
        }));
        s.refold_if_behind("tally");
        (s, folds)
    }

    fn tally_op(op_id: &str, id: &str, value: &str, lamport: i64, site: i64) -> Op {
        Op {
            op_id: op_id.into(),
            lamport,
            site,
            domain: "tally".into(),
            target_kind: "tally".into(),
            target_id: id.into(),
            field: "v".into(),
            op_type: "set".into(),
            value: Some(value.into()),
            author: "u".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        }
    }

    fn tally_rows(s: &Store) -> Vec<(String, String, i64, i64)> {
        let mut stmt = s
            .connection()
            .prepare("SELECT id, v, v_l, v_s FROM tally ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn rowids(s: &Store) -> Vec<(i64, String)> {
        let mut stmt = s
            .connection()
            .prepare("SELECT rowid, op_id FROM ops ORDER BY rowid")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// A source with history: a register overwritten, one written late into the past, a foreign
    /// op of another domain the tally reducer does not fold.
    fn source() -> Store {
        let (mut s, _) = tally_store(1);
        s.apply(&[
            tally_op("t1", "a", "one", 1, 1),
            tally_op("t2", "b", "two", 2, 2),
            tally_op("t3", "a", "three", 5, 2),
            tally_op("t4", "a", "late", 3, 1), // loses to t3: written into the past
            demo_op("d1", "demo", "demo"),
        ]);
        s
    }

    #[test]
    fn an_image_carries_the_log_with_its_rowids_and_the_views_without_a_single_fold() {
        let mut src = source();
        let image = src.image().unwrap();
        assert_eq!(image.op_count(), 5);
        assert_eq!(image.views_of.as_deref(), Some("tally"));

        let (mut dst, folds) = tally_store(9);
        let loaded = dst.load_image(&image).unwrap();
        assert_eq!(loaded, crate::image::Loaded::Views);
        assert_eq!(
            folds.load(Ordering::SeqCst),
            0,
            "the views were TAKEN, not refolded"
        );
        assert_eq!(dst.export(), src.export(), "the same log");
        assert_eq!(rowids(&dst), rowids(&src), "…in the same rowid order");
        assert_eq!(tally_rows(&dst), tally_rows(&src), "the same views");
        assert_eq!(dst.folded_through("tally"), src.folded_through("tally"));
        assert_eq!(
            dst.clock(),
            5,
            "the clock stands past every op it now holds"
        );

        // …and it is an ordinary replica from here on: a later op folds on top as it would have
        // at the source, and a refold of its own log changes nothing.
        let later = tally_op("t5", "b", "five", 6, 3);
        dst.apply(std::slice::from_ref(&later));
        src.apply(std::slice::from_ref(&later));
        assert_eq!(tally_rows(&dst), tally_rows(&src));
        let before = tally_rows(&dst);
        dst.refold();
        assert_eq!(tally_rows(&dst), before);
    }

    #[test]
    fn an_image_another_engine_folded_is_folded_again_from_the_log_it_brought() {
        let src = source();
        let mut image = src.image().unwrap();
        image.engine = "0.0.1".into();
        let (mut dst, folds) = tally_store(9);
        let loaded = dst.load_image(&image).unwrap();
        assert_eq!(
            loaded,
            crate::image::Loaded::Refolded(crate::image::RefoldReason::OtherEngine {
                written_by: "0.0.1".into()
            })
        );
        assert!(folds.load(Ordering::SeqCst) > 0, "folded from the log");
        assert_eq!(tally_rows(&dst), tally_rows(&src), "to the same state");
        assert_eq!(dst.folded_through("tally"), dst.max_op_rowid());
    }

    /// The other reasons views are not taken — each one a real way two stores differ.
    #[test]
    fn views_are_taken_only_on_a_full_match() {
        use crate::image::{Loaded, RefoldReason};
        let src = source();
        type Tamper = Box<dyn Fn(&mut crate::image::Image)>;
        let cases: [(Tamper, RefoldReason); 6] = [
            (
                Box::new(|i| i.schema_version += 1),
                RefoldReason::OtherSchema {
                    written_at: schema::SCHEMA_VERSION + 1,
                },
            ),
            (
                Box::new(|i| {
                    let mut extra = i.views[0].clone();
                    extra.name = "tally_extra".into();
                    i.views.push(extra);
                }),
                RefoldReason::OtherViews,
            ),
            (
                Box::new(|i| i.schema_version -= 1),
                RefoldReason::OtherSchema {
                    written_at: schema::SCHEMA_VERSION - 1,
                },
            ),
            (
                Box::new(|i| i.views_of = Some("flow".into())),
                RefoldReason::OtherViews,
            ),
            (
                Box::new(|i| i.views[0].name = "other".into()),
                RefoldReason::OtherViews,
            ),
            (
                Box::new(|i| {
                    i.views.clear();
                    i.views_of = None;
                }),
                RefoldReason::NoViews,
            ),
        ];
        for (tamper, reason) in cases {
            let mut image = src.image().unwrap();
            tamper(&mut image);
            let (mut dst, _) = tally_store(9);
            assert_eq!(dst.load_image(&image).unwrap(), Loaded::Refolded(reason));
            assert_eq!(tally_rows(&dst), tally_rows(&src));
        }
    }

    #[test]
    fn a_store_that_already_holds_ops_refuses_an_image_and_keeps_what_it_had() {
        let image = source().image().unwrap();
        let (mut dst, _) = tally_store(9);
        dst.apply(&[tally_op("mine", "z", "own", 1, 9)]);
        let err = dst.load_image(&image).unwrap_err();
        assert!(
            matches!(err, crate::image::ImageError::NotEmpty { ops: 1 }),
            "{err}"
        );
        assert_eq!(dst.op_count(), 1);
        assert_eq!(tally_rows(&dst).len(), 1);
    }

    #[test]
    fn an_image_from_a_database_a_newer_nxs_locked_is_refused_by_name() {
        let mut image = source().image().unwrap();
        image.min_compatible = schema::SCHEMA_VERSION + 1;
        let (mut dst, _) = tally_store(9);
        let err = dst.load_image(&image).unwrap_err();
        assert!(err.to_string().contains("upgrade nxs"), "{err}");
        assert_eq!(dst.op_count(), 0, "nothing was written");
    }

    /// A log written before 6j6v.fc5p can hold two ops on one coordinate. A fresh store's UNIQUE
    /// index would refuse the second — so the image rebuilds the index in the shape that log calls
    /// for, and the replica holds exactly what its source held.
    #[test]
    fn an_image_whose_log_holds_a_duplicate_coordinate_loads_whole() {
        let (mut src, _) = tally_store(1);
        src.connection()
            .execute_batch(
                "DROP INDEX ops_lamport_site; CREATE INDEX ops_lamport_site ON ops(lamport, site);",
            )
            .unwrap();
        src.apply(&[
            tally_op("x1", "a", "first", 4, 1),
            tally_op("x2", "b", "second", 4, 1),
        ]);
        assert_eq!(src.op_count(), 2, "the pre-fc5p shape, reproduced");
        let (mut dst, _) = tally_store(9);
        dst.load_image(&src.image().unwrap()).unwrap();
        assert_eq!(dst.export(), src.export());
        let unique: i64 = dst
            .connection()
            .query_row(
                "SELECT \"unique\" FROM pragma_index_list('ops') WHERE name='ops_lamport_site'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unique, 0, "the index took the shape its log calls for");

        // And a clean log keeps the net: its index comes back UNIQUE.
        let (mut clean, _) = tally_store(8);
        clean.load_image(&source().image().unwrap()).unwrap();
        let unique: i64 = clean
            .connection()
            .query_row(
                "SELECT \"unique\" FROM pragma_index_list('ops') WHERE name='ops_lamport_site'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unique, 1);
    }

    /// Views that were behind their own log where the image was taken catch up on load, rather
    /// than leaving this handle behind until its next open.
    #[test]
    fn views_behind_their_log_catch_up_as_they_load() {
        let src = source();
        let mut image = src.image().unwrap();
        image.folded_through = 0;
        image.views[0].rows.clear();
        let (mut dst, _) = tally_store(9);
        dst.load_image(&image).unwrap();
        assert_eq!(tally_rows(&dst), tally_rows(&src));
        assert_eq!(dst.folded_through("tally"), dst.max_op_rowid());
    }

    /// A database created long ago keeps columns its engine no longer writes — the additive rule
    /// never drops one, and the real manufakt.io board still carries `items.belongs_to` from before
    /// parenthood moved onto edges. The engine that took the image never reads such a column, so a
    /// store of the same engine takes the views without it. The other way round is not safe: a
    /// column the TARGET has and the image lacks is one this engine reads and the image cannot fill,
    /// so those views are folded again.
    #[test]
    fn a_column_the_engine_no_longer_writes_is_left_behind_and_a_missing_one_means_refold() {
        use crate::image::{Cell, Loaded, RefoldReason};
        let src = source();
        let mut legacy = src.image().unwrap();
        legacy.views[0].columns.push("belongs_to".into());
        for row in &mut legacy.views[0].rows {
            row.push(Cell::Text("stale".into()));
        }
        let (mut dst, folds) = tally_store(9);
        assert_eq!(dst.load_image(&legacy).unwrap(), Loaded::Views);
        assert_eq!(folds.load(Ordering::SeqCst), 0);
        assert_eq!(tally_rows(&dst), tally_rows(&src));

        let mut short = src.image().unwrap();
        let at = short.views[0]
            .columns
            .iter()
            .position(|c| c == "v_s")
            .unwrap();
        short.views[0].columns.remove(at);
        for row in &mut short.views[0].rows {
            row.remove(at);
        }
        let (mut dst, _) = tally_store(9);
        assert_eq!(
            dst.load_image(&short).unwrap(),
            Loaded::Refolded(RefoldReason::OtherViews)
        );
        assert_eq!(tally_rows(&dst), tally_rows(&src));
    }

    /// Every way a crafted (or buggy) image could put into the log what the engine later reads back
    /// as something else (review of PR #487, Integrity #1). Each is refused before a single row is
    /// written — the alternative was a panic AFTER the commit, and a store no open could read.
    ///
    /// A lamport outside `0..=MAX_LAMPORT` is no longer among them (6j6v.m19v): it reads back as
    /// the integer it is, and the store holds such an op inert instead of folding it or seeding the
    /// clock with it — the same thing the wire path does, see
    /// `a_store_holding_a_hostile_lamport_images_and_loads_with_the_op_inert`.
    #[test]
    fn an_image_the_log_could_not_be_read_back_from_is_refused_whole() {
        use crate::image::{Cell, ImageError};
        let good = source().image().unwrap();
        let col = |name: &str| good.ops.columns.iter().position(|c| c == name).unwrap();
        type Break = Box<dyn Fn(&mut crate::image::Image)>;
        let (lamport, author, op_id, rowid) =
            (col("lamport"), col("author"), col("op_id"), col("rowid"));
        let cases: [(&str, Break); 5] = [
            (
                "a lamport stored as text",
                Box::new(move |i| i.ops.rows[0][lamport] = Cell::Text("7".into())),
            ),
            (
                "a NULL author",
                Box::new(move |i| i.ops.rows[0][author] = Cell::Null),
            ),
            (
                "an op id twice",
                Box::new(move |i| {
                    let first = i.ops.rows[0][op_id].clone();
                    i.ops.rows[1][op_id] = first;
                }),
            ),
            (
                "rowids that do not rise",
                Box::new(move |i| i.ops.rows[1][rowid] = Cell::Integer(1)),
            ),
            (
                "a missing column",
                Box::new(move |i| {
                    i.ops.columns.remove(author);
                    for row in &mut i.ops.rows {
                        row.remove(author);
                    }
                }),
            ),
        ];
        for (what, crafted) in cases {
            let mut image = good.clone();
            crafted(&mut image);
            let (mut dst, _) = tally_store(9);
            let err = dst.load_image(&image).expect_err(what);
            assert!(matches!(err, ImageError::Malformed(_)), "{what}: {err}");
            assert_eq!(dst.op_count(), 0, "{what}: nothing written");
            assert_eq!(dst.export(), vec![], "{what}: and the store still reads");
        }

        // A view cell that does not fit its column's declared type — every reader `.unwrap()`s
        // the type the reducer wrote.
        let mut image = good.clone();
        let v_l = image.views[0]
            .columns
            .iter()
            .position(|c| c == "v_l")
            .unwrap();
        image.views[0].rows[0][v_l] = Cell::Text("nine".into());
        let (mut dst, _) = tally_store(9);
        let err = dst
            .load_image(&image)
            .expect_err("a text lamport in a view");
        assert!(matches!(err, ImageError::Malformed(_)), "{err}");
        assert_eq!(dst.op_count(), 0, "rolled back whole");
    }

    /// A failure in the middle of writing — here a view row the table's key refuses — takes the
    /// whole load back, the dropped coordinate index included.
    #[test]
    fn a_load_that_fails_midway_leaves_nothing_behind_not_even_a_missing_index() {
        let mut image = source().image().unwrap();
        let duplicate = image.views[0].rows[0].clone();
        image.views[0].rows.push(duplicate);
        let (mut dst, _) = tally_store(9);
        let err = dst.load_image(&image).expect_err("a view key twice");
        assert!(matches!(err, crate::image::ImageError::Storage(_)), "{err}");
        assert_eq!(dst.op_count(), 0);
        let unique: i64 = dst
            .connection()
            .query_row(
                "SELECT \"unique\" FROM pragma_index_list('ops') WHERE name='ops_lamport_site'",
                [],
                |r| r.get(0),
            )
            .expect("the index is back");
        assert_eq!(unique, 1, "and it is the UNIQUE one an empty log gets");
        dst.load_image(&source().image().unwrap())
            .expect("and a good image loads afterwards");
    }

    /// Rowids with gaps — a real log has them — come across as they are, and the watermark still
    /// names the same op on both sides.
    #[test]
    fn rowids_with_gaps_come_across_as_they_are() {
        use crate::image::Cell;
        let src = source();
        let mut image = src.image().unwrap();
        for row in &mut image.ops.rows {
            if let Cell::Integer(r) = &mut row[0] {
                *r *= 10;
            }
        }
        image.folded_through *= 10;
        let (mut dst, folds) = tally_store(9);
        assert_eq!(dst.load_image(&image).unwrap(), crate::image::Loaded::Views);
        assert_eq!(
            folds.load(Ordering::SeqCst),
            0,
            "no refold: the watermark matched"
        );
        let ids: Vec<i64> = rowids(&dst).into_iter().map(|(r, _)| r).collect();
        assert_eq!(ids, vec![10, 20, 30, 40, 50]);
        assert_eq!(dst.folded_through("tally"), 50);
        assert_eq!(tally_rows(&dst), tally_rows(&src));
    }

    /// On the refold path the image's views are never written, so a view row no op backs cannot
    /// survive the refold (keep-if-beats would not clear it).
    #[test]
    fn views_that_are_refolded_leave_no_row_of_their_own_behind() {
        use crate::image::Cell;
        let src = source();
        let mut image = src.image().unwrap();
        image.engine = "0.0.1".into();
        let mut ghost = image.views[0].rows[0].clone();
        ghost[0] = Cell::Text("ghost".into());
        image.views[0].rows.push(ghost);
        let (mut dst, _) = tally_store(9);
        dst.load_image(&image).unwrap();
        assert_eq!(tally_rows(&dst), tally_rows(&src), "no ghost row");
    }

    /// Column names are input: a name the target table does not have never reaches a statement.
    /// Here it also takes the place of a column the table needs, so the views cannot be taken —
    /// they are folded again from the log, and the log is untouched.
    #[test]
    fn a_view_column_name_reaches_no_statement_unless_the_table_has_it() {
        use crate::image::{Loaded, RefoldReason};
        let src = source();
        let mut image = src.image().unwrap();
        image.views[0].columns[1] = "v); DROP TABLE ops; --".into();
        let (mut dst, _) = tally_store(9);
        assert_eq!(
            dst.load_image(&image).unwrap(),
            Loaded::Refolded(RefoldReason::OtherViews)
        );
        assert_eq!(dst.export(), src.export(), "the log went in whole");
        assert_eq!(tally_rows(&dst), tally_rows(&src));
    }
}
