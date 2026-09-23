//! Substrate SQLite DDL + the schema-version spine. The `ops` log is the source of truth; each
//! product reducer creates and refreshes ITS OWN materialized views (the flow `items`/`edges`/
//! `notes` views live in the flow crate, not here — the foundation knows no product vocabulary).
//!
//! The version spine (spec §4.3) is the compatibility axis for the shared `nxs` substrate: every
//! consumer of one DB — `nxf`, the embedding app, sync peers — gates on this, NOT on product
//! versions.

use rusqlite::{Connection, OptionalExtension};

/// The current foundation schema version, persisted per database via `PRAGMA user_version`.
///
/// v4 (sp6.3) is the **first breaking migration**: flow's hierarchy parenthood moves from the
/// single-parent LWW `belongs_to` register onto a `parent` edge in the observed-remove OR-set. An
/// OR-set edge cannot converge with an old binary's LWW `belongs_to` writes on a concurrent
/// re-point, so this raises [`MIN_COMPATIBLE_SCHEMA_VERSION`] to lock out pre-v4 binaries by design
/// (checklist rule 4). The flow-semantic op-log rewrite that converts existing `belongs_to` data
/// into edges lives in the flow `Store` open path (data-shape gated), not here — the foundation
/// owns no product vocabulary.
///
/// v5 (6j6v.e0z6) gives memory's `memories` view classification + order. Purely additive, every
/// column with a status-quo DEFAULT (checklist rules 1–2), so the floor stays at 4.
///
/// v6 (6j6v.xbnh) gives it the `introduction` register — the ONE written line `nxs prime` replays
/// per memory. Additive and NULLABLE, and the NULL is load-bearing rather than a shortcut: a
/// memory written before this existed has no introduction, and "none written yet" has to be
/// distinguishable from "written as the empty string" so the bootstrap can SHOW the gap instead of
/// papering over it. Floor stays at 4 (checklist rules 1–2).
///
/// v7 (6j6v.pzkb) signs the log: every op gains its signature pair (`key_id`, `sig`, both NULL for
/// an unsigned op) and this replica's verdict on it (`provenance`, DEFAULT `'unsigned'` — the
/// honest verdict on a row an older binary writes without knowing the column). Beside them the
/// local trust list (`trusted_keys`) and the one definition of "an action may follow this op"
/// (`acting_ops`). Additive, every new column NULLable or DEFAULTed, so the floor stays at 4.
pub const SCHEMA_VERSION: i64 = 7;

/// The lowest schema version a peer must speak to safely touch a DB this binary writes — the
/// **compatibility floor** (spec §4.3). Persisted into the DB header (`PRAGMA application_id`,
/// repurposed) by [`migrate`], so a binary whose [`SCHEMA_VERSION`] is below the DB's stored
/// floor fail-louds instead of corrupting it. It stays at the baseline as long as every
/// migration is additive/alt-binary-safe (see the checklist below); only a truly breaking
/// change raises it, locking out older binaries by design.
///
/// Raised to 4 by sp6.3 (the first breaking migration): parenthood moved from the LWW `belongs_to`
/// register to an OR-set `parent` edge, which a pre-v4 binary's LWW writes cannot converge with
/// (checklist rule 4). flow and memory ship in one binary and fold the same op-log, so they upgrade
/// together — there is no flow-vs-memory skew, only the intended "use a v4+ nxs" gate.
pub const MIN_COMPATIBLE_SCHEMA_VERSION: i64 = 4;

// ── ALT-BINARY-SAFE MIGRATION CHECKLIST (spec §4.3) ─────────────────────────────────────
// CLI (`nxf`) and the embedding app share ONE SQLite file, so an older binary must keep
// reading+writing a DB a newer binary migrated up (degraded), or the floor below locks it
// out on purpose. Keep `MIN_COMPATIBLE_SCHEMA_VERSION` low by making every migration
// alt-binary-safe — ALL of these must hold:
//   1. Additive only — new TABLES/COLUMNS; never DROP/RENAME/repurpose a column or view an
//      older binary reads, and never change a view's semantics.
//   2. Every new column carries a DEFAULT, so an INSERT from a stale writer that does not
//      know the column still yields a valid row (e.g. `domain TEXT NOT NULL DEFAULT 'task'`).
//   3. A new product/reducer axis (`domain = fact | message`) ships DORMANT for flow: no
//      read/write path folds it; an unknown op shape is stored-not-folded (§7) and refolded
//      on upgrade, never dropped.
//   4. A TRULY breaking change (drop/rename/repurpose a read column or view, change view
//      semantics, add NOT NULL without DEFAULT) is the ONLY thing that may raise
//      `MIN_COMPATIBLE_SCHEMA_VERSION` — which fail-louds older binaries by design — and it
//      MUST bump `SCHEMA_VERSION` in the same step.
// Break any of 1–3 and you MUST raise the floor (4); otherwise leave the floor untouched.
// ────────────────────────────────────────────────────────────────────────────────────────

/// Every table the SUBSTRATE owns in a workspace db — the log, the fold watermarks, and since
/// signing (6j6v.pzkb) the local trust list and the small local facts beside it. Everything else
/// in the file belongs to a product: its views, or its declared machine-local tables. One list, so
/// the products' "every table is classified" tests cannot each keep a copy that drifts.
pub const SUBSTRATE_TABLES: &[&str] = &["ops", "view_watermarks", "trusted_keys", "replica_meta"];

/// The PRAGMA user_version this database is stamped at (0 for a pre-spine legacy DB).
pub fn schema_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
}

/// Whether a base table named `name` exists. Used by [`migrate`] to guard a product-view ALTER
/// (the view may be created after migrate, or never, for a substrate-only / non-flow DB).
fn table_exists(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |_| Ok(()),
    )
    .optional()
    .map(|o| o.is_some())
}

/// Bring `conn` up to [`SCHEMA_VERSION`], applying each ordered migration step at most once,
/// then stamping the version. Idempotent and **forward-only**: every `Store` open path calls
/// it right after the baseline DDL.
///
/// No-Downgrade-Guard (spec §4.3): the stamp is `max(current, SCHEMA_VERSION)` and only this
/// binary's not-yet-applied steps run — a binary opening a higher-stamped DB (a newer peer
/// migrated the shared file) must NEVER roll `user_version` back, or the newer binary would
/// re-run an already-applied `ALTER` and hit "duplicate column". A naive
/// `pragma_update(user_version, SCHEMA_VERSION)` would do exactly that; this does not.
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    // Atomic against concurrent openers (#9e9t): take the write lock BEFORE reading the version so
    // the check and the DDL are one indivisible step. Two connections opening a freshly-created db —
    // the change watcher's re-arm racing a same-path recreate's own open, or `nxf init` racing a
    // watching app — would otherwise both read `user_version = 0`, both run `ALTER TABLE ops ADD
    // COLUMN domain`, and every loser hits "duplicate column". `busy_timeout` (set by `Store::open`)
    // makes the second `BEGIN IMMEDIATE` wait for the first to COMMIT; it then re-reads the bumped
    // version and skips the already-applied steps.
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = migrate_locked(conn);
    match &result {
        Ok(()) => conn.execute_batch("COMMIT")?,
        Err(_) => {
            // Deliberately discard a ROLLBACK failure (PR-review Integrity & Robustness #3): `result`
            // holds the ROOT-CAUSE migration error, which is what the caller must see — surfacing a
            // secondary ROLLBACK error instead would MASK it. Cleanup is best-effort and belt-and-
            // suspenders anyway: `Store::open` returns this `Err` without constructing a store, so the
            // `Connection` is dropped, and SQLite rolls back any still-open transaction on close.
            let _ = conn.execute_batch("ROLLBACK");
        }
    }
    result
}

/// The ordered migration steps, run inside the write transaction [`migrate`] holds so the version
/// check and the DDL are atomic against a concurrent opener (#9e9t). Idempotent + forward-only.
fn migrate_locked(conn: &Connection) -> rusqlite::Result<()> {
    let from = schema_version(conn)?;

    // A negative stamp is a corrupt/garbage header, never something this engine wrote. Fail loud
    // rather than papering it over to SCHEMA_VERSION via `from.max(..)` below — benign with one
    // migration step today, a silent-data-loss foot-gun as steps accumulate (a `from < N` gate
    // would then run every step against a db that may not be at v0 at all).
    if from < 0 {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
            Some(format!(
                "corrupt schema header: PRAGMA user_version is {from} (< 0); the workspace db is damaged"
            )),
        ));
    }

    // Each step gates on `from < N` so a re-run (or a higher-stamped DB) never re-applies it.
    if from < 1 {
        // v0 -> v1: add the product/reducer `domain` axis. NOT NULL DEFAULT 'task' backfills
        // every pre-existing op as a flow op, so derivation stays byte-identical, and a stale
        // writer that omits the column still produces a valid row (checklist rule 2).
        conn.execute_batch("ALTER TABLE ops ADD COLUMN domain TEXT NOT NULL DEFAULT 'task';")?;
    }
    if from < 2 {
        // v1 -> v2 (C1, #916.1): flow's `items` view gains the `archived` tombstone — a nullable
        // ISO-8601 TIMESTAMP folded as an LWW cell, plus its `_v`/`_site` Lamport metadata (the
        // same shape `apply_flow_views` mints for a fresh DB). The view is product vocabulary, but
        // the version spine is foundation-wide and the migration is the one place it necessarily
        // leaks (cf. the `'task'` default above). Guarded on existence: `items` is created by flow
        // AFTER this runs (fresh DBs) or never (substrate-only / non-flow modules) — there it is a
        // clean no-op, and the fresh `items` already carries the column. An existing pre-C1 view
        // (the in-place upgrade) is ALTERed here. Nullable + additive ⇒ alt-binary-safe (checklist
        // rules 1–2) ⇒ MIN_COMPATIBLE_SCHEMA_VERSION stays put.
        if table_exists(conn, "items")? {
            conn.execute_batch(
                "ALTER TABLE items ADD COLUMN archived TEXT;
                 ALTER TABLE items ADD COLUMN archived_v INTEGER DEFAULT 0;
                 ALTER TABLE items ADD COLUMN archived_site INTEGER DEFAULT 0;",
            )?;
        }
    }
    if from < 3 {
        // v2 -> v3 (C3, #916.4): flow's `items` view gains the `closed_at` instant — a nullable
        // ISO-8601 TIMESTAMP folded as an LWW cell (+ its `_v`/`_site` Lamport metadata), set by
        // `close` so the `closed` lane can order by close-date descending. Modelled exactly like
        // `archived` above: the view is product vocabulary, but the version spine is foundation-
        // wide and the migration is where it necessarily leaks. Existence-guarded for the same
        // reasons (fresh DBs create `items` AFTER migrate already carrying the column; substrate-
        // only DBs never have it). Nullable + additive ⇒ alt-binary-safe (checklist rules 1–2) ⇒
        // MIN_COMPATIBLE_SCHEMA_VERSION stays put.
        if table_exists(conn, "items")? {
            conn.execute_batch(
                "ALTER TABLE items ADD COLUMN closed_at TEXT;
                 ALTER TABLE items ADD COLUMN closed_at_v INTEGER DEFAULT 0;
                 ALTER TABLE items ADD COLUMN closed_at_site INTEGER DEFAULT 0;",
            )?;
        }
    }

    // v3 -> v4 (sp6.3): the FIRST breaking migration. flow's hierarchy parenthood moves from the
    // single-parent LWW `belongs_to` register onto a `parent` edge in the OR-set. There is no
    // foundation DDL here — the breaking part is (a) the floor raise below (MIN_COMPATIBLE → 4,
    // which the floor-max logic applies, locking out pre-v4 binaries per checklist rule 4) and
    // (b) a flow-semantic op-log rewrite (`belongs_to` ops → `parent` edge ops) that lives in the
    // flow `Store` open path, gated by data shape (the foundation owns no product vocabulary, and
    // it migrates+stamps the version before flow runs, so flow cannot version-gate). The stale
    // `belongs_to` column on a migrated `items` view is left in place (harmless) and simply no
    // longer read — flow projects parenthood from the `present_parent` view instead.

    if from < 5 {
        // v4 -> v5 (6j6v.e0z6): memory's `memories` view gains **classification and order** —
        // `category`, `scope`, `refs` and `ordinal`, each an independently versioned register, plus
        // the `created_v`/`created_site` pair that carries insertion order. Same shape as the two
        // `items` steps above: the view is product vocabulary, but the version spine is
        // foundation-wide and a migration is the one place it necessarily leaks. Existence-guarded —
        // a fresh DB creates `memories` AFTER this runs (already carrying the columns), and a
        // flow-only / substrate-only DB never has the view at all.
        //
        // Every column carries a DEFAULT that describes what the memories in the file ALREADY do
        // (`scope='project'` — `nxs prime` replays them for this workspace; `category='unsorted'` —
        // reserved and therefore countable, the signal the later judging migration works from; no
        // refs; no explicit order). So the step introduces the fields WITHOUT changing any
        // behaviour, and an older binary that neither writes nor reads them keeps working
        // (checklist rules 1–2) ⇒ MIN_COMPATIBLE_SCHEMA_VERSION stays put.
        //
        // Kept in lockstep with `nexus_memory::schema::try_apply_memory_views`, which spells the
        // same shape for a fresh DB; that crate's `a_migrated_view_matches_a_freshly_created_one`
        // compares the two column for column, so this copy cannot drift unnoticed.
        if table_exists(conn, "memories")? {
            conn.execute_batch(
                "ALTER TABLE memories ADD COLUMN category TEXT NOT NULL DEFAULT 'unsorted';
                 ALTER TABLE memories ADD COLUMN category_v INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE memories ADD COLUMN category_site INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE memories ADD COLUMN scope TEXT NOT NULL DEFAULT 'project';
                 ALTER TABLE memories ADD COLUMN scope_v INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE memories ADD COLUMN scope_site INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE memories ADD COLUMN refs TEXT NOT NULL DEFAULT '';
                 ALTER TABLE memories ADD COLUMN refs_v INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE memories ADD COLUMN refs_site INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE memories ADD COLUMN ordinal INTEGER;
                 ALTER TABLE memories ADD COLUMN ordinal_v INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE memories ADD COLUMN ordinal_site INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE memories ADD COLUMN created_v INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE memories ADD COLUMN created_site INTEGER NOT NULL DEFAULT 0;",
            )?;
            // Backfill insertion order from the log rather than leaving it at the column DEFAULT.
            // The fact reducer folds `created_*` as the MINIMUM `(lamport, site)` over a key's ops,
            // so a refold would compute exactly this — writing it here is what keeps the migrated
            // view identical to a refolded one instead of only converging after the next refold.
            conn.execute_batch(
                "UPDATE memories SET
                     created_v = COALESCE((SELECT o.lamport FROM ops o
                                           WHERE o.domain='fact' AND o.target_kind='fact'
                                             AND o.target_id = memories.key
                                           ORDER BY o.lamport, o.site LIMIT 1), 0),
                     created_site = COALESCE((SELECT o.site FROM ops o
                                              WHERE o.domain='fact' AND o.target_kind='fact'
                                                AND o.target_id = memories.key
                                              ORDER BY o.lamport, o.site LIMIT 1), 0);",
            )?;
        }
    }

    if from < 6 {
        // v5 -> v6 (6j6v.xbnh): memory's `memories` view gains the **introduction** register — the
        // one written line the session bootstrap replays for a memory, at most 200 characters. Same
        // existence-guarded, additive shape as every step above, and kept in lockstep with
        // `nexus_memory::schema::try_apply_memory_views` (that crate's
        // `a_migrated_view_matches_a_freshly_created_one` compares the two column for column).
        //
        // NULLABLE, unlike the four v5 registers, and that is the whole point: those four could
        // carry a DEFAULT describing what an unclassified memory already did, while an
        // introduction is WRITTEN and there is no honest default for one nobody has written. NULL
        // is therefore "none yet", which `nxm prime` renders as a named placeholder — the lack has
        // to be visible, because a derived stand-in is exactly the mechanism this register
        // replaces.
        if table_exists(conn, "memories")? {
            conn.execute_batch(
                "ALTER TABLE memories ADD COLUMN introduction TEXT;
                 ALTER TABLE memories ADD COLUMN introduction_v INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE memories ADD COLUMN introduction_site INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
    }

    if from < 7 {
        // v6 -> v7 (6j6v.pzkb): the log is signed. See `SCHEMA_VERSION` for the columns, and
        // `crate::trust` for the table and the view.
        //
        // The pending mark is how the ops ALREADY in the log get their verdict: none of them was
        // signed, and those this replica wrote itself are its own — but which ones those are needs
        // the replica's site, which a migration does not know. `trust::seat_replica` does, runs on
        // every store open, and consumes the mark in the same transaction that stamps them, so the
        // stamp happens exactly once even when a raw `nxs migrate` ran this step first.
        //
        // Each column only where it is missing: a db stamped BELOW a version it already carries the
        // columns of — the way the suites build "a workspace the previous release wrote" out of a
        // current one — must migrate, not fail on a duplicate column.
        conn.execute_batch(crate::trust::TRUST_TABLES)?;
        for (column, ddl) in [
            ("key_id", "ALTER TABLE ops ADD COLUMN key_id TEXT;"),
            ("sig", "ALTER TABLE ops ADD COLUMN sig TEXT;"),
            (
                "provenance",
                "ALTER TABLE ops ADD COLUMN provenance TEXT NOT NULL DEFAULT 'unsigned';",
            ),
        ] {
            if !column_exists(conn, "ops", column)? {
                conn.execute_batch(ddl)?;
            }
        }
        conn.execute_batch(
            "INSERT OR REPLACE INTO replica_meta(key, value) VALUES('legacy_own_pending', '1');",
        )?;
    }

    // The signing schema's view and index (6j6v.pzkb). Not version-gated, for the index's reason
    // below: both are invisible to an older binary, and both are re-attempted on every open.
    crate::trust::ensure_acting_ops(conn)?;

    // The op-coordinate index (6j6v.fc5p). Deliberately NOT a version-gated step: it is an INDEX,
    // invisible to an older binary's reads, and its creation is CONDITIONAL on the log it finds
    // (see the helper) — so it must be re-attempted on every open rather than crossed off once.
    ensure_op_coordinate_index(conn)?;

    // The append-only guard (6j6v.hehx). Not version-gated either, for the index's reason: a
    // TRIGGER is invisible to an older binary, which never deletes an op or rewrites its identity
    // in the first place, so installing it on every open changes nothing any binary does legally.
    ensure_append_only_guard(conn)?;

    // No-Downgrade-Guard: stamp up only, never down.
    let stamped = from.max(SCHEMA_VERSION);
    if stamped != from {
        conn.pragma_update(None, "user_version", stamped)?;
    }

    // Persist the compatibility floor, raise-only (same no-downgrade discipline). At the
    // current additive floor this is a no-op (default `application_id` is already 0); the
    // mechanism is load-bearing for the first breaking migration, which raises
    // MIN_COMPATIBLE_SCHEMA_VERSION and thereby locks out older binaries (checklist rule 4).
    let floor_from = db_min_compatible(conn)?;
    let floor = floor_from.max(MIN_COMPATIBLE_SCHEMA_VERSION);
    if floor != floor_from {
        conn.pragma_update(None, "application_id", floor)?;
    }

    // Post-condition, mechanically guarding checklist rule 4: a migration must never leave the
    // floor above the version. `assess_open`'s `Current` short-circuit (`db_version <=
    // SCHEMA_VERSION` returns without consulting the floor) is sound ONLY because this holds —
    // so guard the linchpin here instead of trusting prose. (debug-only: a pure read of what
    // was just persisted, no release-build cost.)
    debug_assert!(
        db_min_compatible(conn)? <= schema_version(conn)?,
        "post-migrate invariant violated: compatibility floor exceeds the schema version"
    );
    Ok(())
}

/// Whether `table` has a column named `column`.
fn column_exists(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2",
        [table, column],
        |_| Ok(()),
    )
    .optional()
    .map(|o| o.is_some())
}

/// Whether an index named `name` exists on this database.
fn index_exists(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1",
        [name],
        |_| Ok(()),
    )
    .optional()
    .map(|o| o.is_some())
}

/// Create the `(lamport, site)` index over `ops` — the op's **coordinate** in the CRDT order — as
/// UNIQUE where the log allows it (6j6v.fc5p). Called from [`migrate_locked`], inside the write
/// transaction, on every open.
///
/// It earns its keep twice:
///
/// 1. **The net under the clock.** Monotonicity PER SITE is an invariant, not a race outcome:
///    across sites a Lamport number says nothing (that is what Lamport clocks are for), but the
///    same number twice from the SAME stamp is something a correct implementation cannot produce.
///    A collision is therefore not bad luck but *proof* that a process wrote with a stale clock —
///    and `fold_lww`'s keep-if-beats (strict `>`) would swallow the second write in silence. With
///    the index the log refuses the coordinate instead of the fold discarding the value.
/// 2. **It pays for the refresh.** `lamport` leads the index, so the `MAX(lamport)` that
///    [`Store::emit`](crate::store::Store::emit) now reads before every local op is an index
///    lookup instead of a table scan. (`export`'s `ORDER BY lamport, site` is served by it too.)
///    The column order is therefore load-bearing — `(site, lamport)` would express the same
///    uniqueness and serve neither read.
///
/// **Conditional, because logs written before the fix carry the damage.** This is measured, not
/// feared: on the day it landed (2026-08-18), three of the twenty workspaces on this project's own
/// development machine held colliding pairs — the nexus-flow board itself among them, four pairs in
/// 9268 ops. `CREATE UNIQUE INDEX` over such a log fails, and a failure here fails `Store::open` —
/// i.e. an unconditional UNIQUE would brick exactly the boards that prove the bug. So a log that already contains duplicates gets the
/// same index NON-unique: every read path keeps the speedup, and only the uniqueness net is
/// missing — which is honest, because on that log the invariant is already broken and no index can
/// retroactively restore it. Which shape a database got is readable (`PRAGMA index_list(ops)`) and
/// pinned by a test. Duplicates never disappear from an append-only log, so the degraded state is
/// stable rather than a retry loop; the existence check short-circuits it after the first open.
pub(crate) fn ensure_op_coordinate_index(conn: &Connection) -> rusqlite::Result<()> {
    if index_exists(conn, "ops_lamport_site")? {
        return Ok(());
    }
    let colliding: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (SELECT 1 FROM ops GROUP BY lamport, site HAVING COUNT(*) > 1)",
        [],
        |r| r.get(0),
    )?;
    let unique = if colliding == 0 { "UNIQUE " } else { "" };
    conn.execute_batch(&format!(
        "CREATE {unique}INDEX ops_lamport_site ON ops(lamport, site);"
    ))
}

/// Make the log append-only BY CONSTRUCTION (6j6v.hehx #3): the database refuses to delete an op,
/// and refuses to change what an op IS — its `op_id` (the dedup key), its `(lamport, site)`
/// coordinate (its place in the CRDT order) and its `author` (attribution cannot be backfilled, so
/// it cannot be rewritten either). Called from [`migrate_locked`] on every open.
///
/// **Why a trigger and not a test.** The review of PR #482 found that nothing asserted the log
/// never shrinks: "a later compaction job would pass every test today". A test over the maintenance
/// paths that exist cannot see the one somebody writes next — and a snapshot (6j6v.mxt2) is
/// precisely a job of that family. With the guard, such a path fails the first time it runs, in a
/// test or in production, and giving it up becomes a decision somebody takes in a migration rather
/// than a side effect nobody noticed.
///
/// **What stays writable, on purpose.** The two in-place rewrites the engine performs — the prefix
/// remap (`target_id`/`value`) and the legacy `belongs_to` → `parent` edge migration (the op's
/// shape) — keep an op's identity, coordinate and author, so they pass. `wall_clock` is display
/// only and `domain` has no rewrite at all; neither is guarded, because guarding a column nothing
/// writes only moves the day somebody has to reason about it.
fn ensure_append_only_guard(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS ops_append_only_no_delete BEFORE DELETE ON ops
         BEGIN
             SELECT RAISE(ABORT, 'the op log is append-only: an op is never deleted');
         END;
         CREATE TRIGGER IF NOT EXISTS ops_append_only_identity
         BEFORE UPDATE OF op_id, lamport, site, author ON ops
         BEGIN
             SELECT RAISE(ABORT, 'an op''s id, (lamport, site) coordinate and author never change');
         END;",
    )
}

/// The compatibility floor stamped into this database's header. Stored in `PRAGMA
/// application_id` (repurposed as the foundation's min-compatible axis, distinct from
/// `user_version` which is the schema *version*). 0 for any DB no raised-floor writer has
/// touched. See [`MIN_COMPATIBLE_SCHEMA_VERSION`] and [`assess_open`].
pub fn db_min_compatible(conn: &Connection) -> rusqlite::Result<i64> {
    conn.pragma_query_value(None, "application_id", |r| r.get(0))
}

/// The verdict of opening a (possibly foreign-stamped) database with THIS binary — the local
/// consumer/sync skew gate (spec §4.3). CLI and embedding app share one file, so an open must
/// classify the DB before touching it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaCompat {
    /// The DB is at or below this binary's schema version: proceed; [`migrate`] upgrades it
    /// if it is behind.
    Current,
    /// The DB is newer than this binary but the writer kept the floor at/below us: keep
    /// working, reading + writing the axes we know. Op shapes we do not understand stay
    /// store-don't-fold (§7) and refold on a later upgrade.
    Degraded { db_version: i64, ours: i64 },
    /// The DB's compatibility floor is above this binary — a newer nxs made a breaking change.
    /// Refuse to touch it (fail-loud); [`SchemaCompat::refusal`] carries the message.
    Incompatible {
        db_version: i64,
        db_min_compatible: i64,
        ours: i64,
    },
}

impl SchemaCompat {
    /// The actionable message for the fail-loud case; `None` when the DB is usable
    /// (`Current`/`Degraded`).
    pub fn refusal(&self) -> Option<String> {
        match self {
            SchemaCompat::Incompatible {
                db_version,
                db_min_compatible,
                ours,
            } => Some(format!(
                "this nxs workspace is at schema v{db_version} (min-compatible v{db_min_compatible}) \
                 but this binary speaks schema v{ours}; it was written by a newer nxs — \
                 upgrade nxs to open this workspace"
            )),
            _ => None,
        }
    }
}

/// Classify a DB against this binary's [`SCHEMA_VERSION`]/[`MIN_COMPATIBLE_SCHEMA_VERSION`]
/// for the skew gate (spec §4.3). Pure: reads the stamps, mutates nothing — so a fail-loud
/// DB is never written. Every `Store` open path runs this.
pub fn assess_open(conn: &Connection) -> rusqlite::Result<SchemaCompat> {
    let db_version = schema_version(conn)?;
    if db_version <= SCHEMA_VERSION {
        // We are at or ahead of the DB; migrate() upgrades it if it is behind.
        return Ok(SchemaCompat::Current);
    }
    // The DB was written by a newer binary. We may still operate iff our SCHEMA_VERSION meets
    // the floor that newer writer declared — i.e. it changed only additive/alt-binary-safe axes.
    let db_min = db_min_compatible(conn)?;
    if SCHEMA_VERSION >= db_min {
        Ok(SchemaCompat::Degraded {
            db_version,
            ours: SCHEMA_VERSION,
        })
    } else {
        Ok(SchemaCompat::Incompatible {
            db_version,
            db_min_compatible: db_min,
            ours: SCHEMA_VERSION,
        })
    }
}

/// Apply the substrate schema, panicking on failure. For in-memory stores and tests where a
/// failure is a bug, not a recoverable condition. File-backed opens use [`try_apply`].
pub fn apply(conn: &Connection) {
    try_apply(conn).expect("schema");
}

/// Apply the substrate schema (the `ops` log + its index), returning any sqlite error (e.g.
/// "file is not a database"). File-backed `Store::open` uses this so a corrupt/foreign db
/// surfaces as a handled error rather than a panic. Product views are created by each reducer,
/// not here.
pub fn try_apply(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ops(
             op_id TEXT PRIMARY KEY,
             lamport INTEGER NOT NULL,
             site INTEGER NOT NULL,
             target_kind TEXT NOT NULL,
             target_id TEXT NOT NULL,
             field TEXT NOT NULL,
             op_type TEXT NOT NULL,
             value TEXT,
             author TEXT,
             wall_clock TEXT
         );
         CREATE INDEX IF NOT EXISTS ops_target ON ops(target_id);
         -- Per-store folded-through watermark (aye.36): the highest `ops.rowid` a given product
         -- store has folded into ITS views. `ops.rowid` is the local-append order — the snapshot
         -- boundary. A product store opens behind when the log advanced out-of-band (a sync pull
         -- that moved the shared log without folding this product's views) and refolds once. Keyed
         -- by store id because flow (`task`) and memory (`fact`) fold the SAME log into different
         -- views and advance independently. Additive baseline DDL (CREATE IF NOT EXISTS, like
         -- `ops`): an older binary simply never reads it, so no version-spine step / floor change.
         CREATE TABLE IF NOT EXISTS view_watermarks(
             store_id TEXT PRIMARY KEY,
             folded_through INTEGER NOT NULL DEFAULT 0
         );",
    )?;
    conn.execute_batch(crate::trust::TRUST_TABLES)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh substrate db has the `ops` log and is stamped at the current version.
    #[test]
    fn fresh_substrate_has_ops_and_is_at_current_version() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn);
        migrate(&conn).unwrap();
        let has_ops: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='ops'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_ops, 1, "the substrate creates the ops log");
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn schema_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn);
        apply(&conn); // must not panic on second apply
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn);
        migrate(&conn).unwrap();
        migrate(&conn).unwrap(); // second run is a clean no-op — no "duplicate column"
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn migrate_upgrades_a_legacy_v0_database() {
        // A pre-spine DB: the v0 `ops` shape (no `domain`), user_version 0, one op already in.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE ops(
                 op_id TEXT PRIMARY KEY, lamport INTEGER NOT NULL, site INTEGER NOT NULL,
                 target_kind TEXT NOT NULL, target_id TEXT NOT NULL, field TEXT NOT NULL,
                 op_type TEXT NOT NULL, value TEXT, author TEXT, wall_clock TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ops(op_id,lamport,site,target_kind,target_id,field,op_type,value,author,wall_clock)
             VALUES('01','1','1','item','t.A','title','set','A','u','')",
            [],
        )
        .unwrap();
        assert_eq!(schema_version(&conn).unwrap(), 0, "legacy db starts at v0");

        migrate(&conn).unwrap();

        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        let domain: String = conn
            .query_row("SELECT domain FROM ops WHERE op_id='01'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(domain, "task", "pre-existing ops backfill as flow ops");
    }

    /// All column names on a table (for asserting an ALTER landed).
    fn columns(conn: &Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn migrate_v1_runs_every_item_view_step_to_current() {
        // C1 (#916.1) + C3 (#916.4): an older flow binary left an `items` view with only the
        // `deleted` tombstone, stamped at v1. Bringing it current must run BOTH additive ALTERs —
        // `archived` (v1→v2) and `closed_at` (v2→v3) — each nullable with LWW metadata, WITHOUT
        // raising the compatibility floor.
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn);
        conn.execute_batch(
            "CREATE TABLE items(
                 id TEXT PRIMARY KEY,
                 deleted TEXT, deleted_v INTEGER DEFAULT 0, deleted_site INTEGER DEFAULT 0
             );",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();

        migrate(&conn).unwrap();

        assert_eq!(
            schema_version(&conn).unwrap(),
            SCHEMA_VERSION,
            "version brought current"
        );
        assert_eq!(SCHEMA_VERSION, 7);
        assert_eq!(
            MIN_COMPATIBLE_SCHEMA_VERSION, 4,
            "the v4 breaking migration (parenthood → OR-set parent edge) raised the floor; the \
             additive v5 step (memory's classification + order), the additive v6 step (its \
             written introduction) and the additive v7 step (the signed log) left it exactly there"
        );
        let cols = columns(&conn, "items");
        for expected in [
            "archived",
            "archived_v",
            "archived_site",
            "closed_at",
            "closed_at_v",
            "closed_at_site",
        ] {
            assert!(
                cols.contains(&expected.to_string()),
                "missing {expected}: {cols:?}"
            );
        }
        // A pre-existing row reads each new column as NULL (not archived, not close-stamped).
        conn.execute("INSERT INTO items(id) VALUES('x')", [])
            .unwrap();
        let (archived, closed_at): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT archived, closed_at FROM items WHERE id='x'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(archived, None, "pre-existing rows are not archived");
        assert_eq!(closed_at, None, "pre-existing rows carry no close instant");

        // Idempotent: a second migrate (from = current) skips every step — no "duplicate column".
        migrate(&conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn migrate_v2_to_v3_adds_the_closed_at_column_to_an_existing_items_view() {
        // C3 (#916.4): the `closed` lane sorts by close-date descending, which needs a stored
        // close instant. An older flow binary left `items` WITH `archived` (v2) but WITHOUT
        // `closed_at`. This binary ALTERs the nullable, additive `closed_at` cell (+ its LWW
        // metadata) in so the `closed` lane can order by it — WITHOUT raising the floor.
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn);
        conn.execute_batch(
            "CREATE TABLE items(
                 id TEXT PRIMARY KEY,
                 deleted TEXT, deleted_v INTEGER DEFAULT 0, deleted_site INTEGER DEFAULT 0,
                 archived TEXT, archived_v INTEGER DEFAULT 0, archived_site INTEGER DEFAULT 0
             );",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();

        migrate(&conn).unwrap();

        assert_eq!(
            schema_version(&conn).unwrap(),
            SCHEMA_VERSION,
            "version brought current (the v2→v3 closed_at ALTER + the v4 breaking step)"
        );
        assert_eq!(SCHEMA_VERSION, 7);
        assert_eq!(
            MIN_COMPATIBLE_SCHEMA_VERSION, 4,
            "the v4 breaking migration raised the floor; the additive v5, v6 and v7 steps left it there"
        );
        let cols = columns(&conn, "items");
        for expected in ["closed_at", "closed_at_v", "closed_at_site"] {
            assert!(
                cols.contains(&expected.to_string()),
                "missing {expected}: {cols:?}"
            );
        }
        // A pre-existing row reads the new column as NULL (not closed-stamped).
        conn.execute("INSERT INTO items(id) VALUES('x')", [])
            .unwrap();
        let v: Option<String> = conn
            .query_row("SELECT closed_at FROM items WHERE id='x'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, None, "pre-existing rows carry no close instant");

        // Idempotent: a second migrate (from = current) skips the step — no "duplicate column".
        migrate(&conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn migrate_to_v3_is_a_clean_noop_when_no_items_view_exists() {
        // The fresh-flow and substrate-only paths: `items` is created AFTER migrate (or never).
        // The item-view ALTER steps (v1→v2 archived, v2→v3 closed_at) must guard on existence
        // and not error when `items` is absent.
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn); // ops only, no flow views
        migrate(&conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        assert!(!columns(&conn, "items").iter().any(|c| c == "archived"));
        assert!(!columns(&conn, "items").iter().any(|c| c == "closed_at"));
    }

    #[test]
    fn migrate_does_not_downgrade_a_higher_stamped_db() {
        // No-Downgrade-Guard (aye.1.3): a binary whose SCHEMA_VERSION is LOWER than the DB's
        // stamp must NOT roll user_version back, and must NOT re-run an already-applied step
        // (which would raise "duplicate column"). migrate() stamps max(current, mine) and
        // only applies its own not-yet-applied steps.
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn);
        migrate(&conn).unwrap(); // brings us to SCHEMA_VERSION, `domain` present
        let higher = SCHEMA_VERSION + 1;
        conn.pragma_update(None, "user_version", higher).unwrap();

        migrate(&conn).unwrap();
        assert_eq!(
            schema_version(&conn).unwrap(),
            higher,
            "lower binary must not stamp the version back down"
        );
        migrate(&conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), higher);
    }

    #[test]
    fn migrate_persists_the_compatibility_floor() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn);
        migrate(&conn).unwrap();
        assert_eq!(
            db_min_compatible(&conn).unwrap(),
            MIN_COMPATIBLE_SCHEMA_VERSION
        );
    }

    #[test]
    fn degraded_when_db_is_newer_but_within_the_compatibility_floor() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn);
        migrate(&conn).unwrap();
        let newer = SCHEMA_VERSION + 1;
        conn.pragma_update(None, "user_version", newer).unwrap();

        match assess_open(&conn).unwrap() {
            SchemaCompat::Degraded { db_version, ours } => {
                assert_eq!(db_version, newer);
                assert_eq!(ours, SCHEMA_VERSION);
            }
            other => panic!("expected Degraded, got {other:?}"),
        }
        assert!(
            assess_open(&conn).unwrap().refusal().is_none(),
            "a degraded workspace stays usable — no fail-loud"
        );
    }

    #[test]
    fn fail_loud_when_the_db_floor_exceeds_this_binary() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn);
        migrate(&conn).unwrap();
        let newer = SCHEMA_VERSION + 1;
        conn.pragma_update(None, "user_version", newer).unwrap();
        conn.pragma_update(None, "application_id", newer).unwrap(); // floor raised above us

        match assess_open(&conn).unwrap() {
            SchemaCompat::Incompatible {
                db_version,
                db_min_compatible,
                ours,
            } => {
                assert_eq!(db_version, newer);
                assert_eq!(db_min_compatible, newer);
                assert_eq!(ours, SCHEMA_VERSION);
            }
            other => panic!("expected Incompatible, got {other:?}"),
        }
        let msg = assess_open(&conn)
            .unwrap()
            .refusal()
            .expect("incompatible must carry a refusal message");
        assert!(
            msg.contains("upgrade"),
            "message must tell the user what to do: {msg}"
        );
    }
}
