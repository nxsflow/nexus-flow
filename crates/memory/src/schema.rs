//! memory's materialized view — the `memories` table the fact reducer folds into and reads serve
//! (spec §4). Created over the substrate's `ops` log; the substrate owns `ops` + the schema-version
//! spine, this module owns only the fact-vocabulary DDL.
//!
//! One row per memory `key`. The **body** is a single keep-if-beats LWW register (spec §3): `body`,
//! `author`, `updated` and `active` all move together with the winning op, versioned by `(v, site)`
//! = `(lamport, site)`. `body` is NULL when the winning op is a `forget`; `active` rows with `0` stay
//! in the table as reversible-tombstone carriers (revive) but are filtered out of every read.
//!
//! Since 6j6v.e0z6 the row also carries **classification and order**, each its own independently
//! versioned register so classifying a memory never disturbs its text (and vice versa):
//!
//! - `category`/`scope`/`refs` — keep-if-beats LWW, exactly like the body register.
//! - `ordinal` — keep-if-beats LWW, but NULL until the deliberate `reorder` verb writes one.
//! - `introduction` (6j6v.xbnh) — keep-if-beats LWW, NULL until somebody WRITES one. The one line
//!   the session bootstrap replays for this memory, and the NULL is a state rather than a gap in
//!   the DDL: a memory written before the register existed has none, and the bootstrap has to be
//!   able to say so.
//! - `created_v`/`created_site` — the (lamport, site) of the EARLIEST fact op for this key, folded
//!   as a **minimum**. Min is commutative, associative and idempotent, so insertion order is a pure
//!   function of the log no matter what order the ops are delivered or refolded in.
//!
//! Every classification column carries a DEFAULT that describes what a memory written before this
//! existed already did (`scope='project'`, `category='unsorted'`, no refs, no explicit order), which
//! is what lets the foundation's v4→v5 migration introduce them without changing any behaviour.

use rusqlite::Connection;

/// Apply memory's materialized view, panicking on failure. For in-memory stores and tests.
pub fn apply_memory_views(conn: &Connection) {
    try_apply_memory_views(conn).expect("memory views");
}

/// Apply memory's `memories` view (idempotent). `v`/`site` carry the LWW Lamport metadata of the
/// winning body op; each classification register carries its own `_v`/`_site` pair. Created over the
/// substrate's `ops` log; the fact reducer refreshes it.
///
/// **`introduction` comes last, matching the ALTER order of the v5→v6 step in the foundation** —
/// [`tests::a_migrated_view_matches_a_freshly_created_one`] compares the two shapes column for
/// column, position included.
///
/// The DDL below carries no SQL comments on purpose. SQLite keeps the statement verbatim in
/// `sqlite_master` and re-parses it on `ALTER TABLE … DROP COLUMN`; a trailing comment left with
/// nothing after it fails that re-parse with "incomplete input", which is a fixture-only paper cut
/// today and a genuine trap the day a migration drops a column here. The reasoning lives in this
/// doc comment instead, where nothing re-parses it.
///
/// **Kept in lockstep with the v4→v5 step in `nxs_foundation::schema::migrate`**, which ALTERs an
/// existing view into this same shape. The foundation owns no memory vocabulary, so the two
/// spellings cannot share code — [`tests::a_migrated_view_matches_a_freshly_created_one`] compares
/// them column for column instead, so the duplication cannot drift silently.
pub fn try_apply_memory_views(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memories(
             key           TEXT PRIMARY KEY,
             body          TEXT,
             author        TEXT,
             updated       TEXT,
             active        INTEGER NOT NULL DEFAULT 0,
             v             INTEGER NOT NULL DEFAULT 0,
             site          INTEGER NOT NULL DEFAULT 0,
             category      TEXT NOT NULL DEFAULT 'unsorted',
             category_v    INTEGER NOT NULL DEFAULT 0,
             category_site INTEGER NOT NULL DEFAULT 0,
             scope         TEXT NOT NULL DEFAULT 'project',
             scope_v       INTEGER NOT NULL DEFAULT 0,
             scope_site    INTEGER NOT NULL DEFAULT 0,
             refs          TEXT NOT NULL DEFAULT '',
             refs_v        INTEGER NOT NULL DEFAULT 0,
             refs_site     INTEGER NOT NULL DEFAULT 0,
             ordinal       INTEGER,
             ordinal_v     INTEGER NOT NULL DEFAULT 0,
             ordinal_site  INTEGER NOT NULL DEFAULT 0,
             created_v     INTEGER NOT NULL DEFAULT 0,
             created_site  INTEGER NOT NULL DEFAULT 0,
             introduction      TEXT,
             introduction_v    INTEGER NOT NULL DEFAULT 0,
             introduction_site INTEGER NOT NULL DEFAULT 0
         );",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// The `memories` columns in `PRAGMA table_info` order.
    fn columns(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info('memories')")
            .unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.map(Result::unwrap).collect()
    }

    #[test]
    fn apply_creates_the_memories_view() {
        let conn = Connection::open_in_memory().unwrap();
        apply_memory_views(&conn);
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memories'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "the memories view exists after apply");
    }

    #[test]
    fn apply_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        apply_memory_views(&conn);
        apply_memory_views(&conn); // second apply must not panic
    }

    #[test]
    fn a_migrated_view_matches_a_freshly_created_one() {
        // The drift gate for the one duplication this design cannot avoid: the foundation ALTERs an
        // existing `memories` view up to v5 with its own copy of the DDL (it owns no memory
        // vocabulary, so it cannot call the function above). Build the pre-6j6v.e0z6 shape by hand,
        // migrate it, and compare against a fresh create — if either spelling gains a column the
        // other lacks, this fails by name.
        let old = Connection::open_in_memory().unwrap();
        nxs_foundation::schema::try_apply(&old).unwrap();
        // Bring the substrate itself up to date first (the `ops` shape the backfill reads), THEN
        // stamp the db back to v4 and give it the pre-6j6v.e0z6 `memories` — the state a workspace
        // written by the previous release is actually in.
        nxs_foundation::schema::migrate(&old).unwrap();
        old.pragma_update(None, "user_version", 4).unwrap();
        old.execute_batch(
            "CREATE TABLE memories(
                 key     TEXT PRIMARY KEY,
                 body    TEXT,
                 author  TEXT,
                 updated TEXT,
                 active  INTEGER NOT NULL DEFAULT 0,
                 v       INTEGER NOT NULL DEFAULT 0,
                 site    INTEGER NOT NULL DEFAULT 0
             );",
        )
        .unwrap();
        nxs_foundation::schema::migrate(&old).unwrap();

        let fresh = Connection::open_in_memory().unwrap();
        apply_memory_views(&fresh);

        assert_eq!(
            columns(&old),
            columns(&fresh),
            "the migrated view and the freshly created one must be the same table"
        );
    }
}
