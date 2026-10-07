//! flow's views answer without the op log (6j6v.vvw6).
//!
//! The parent projection, the thread-link projection, an item's created/updated instants and a
//! note's order and date used to be read by joining the views back onto `ops`. A server that folds
//! into DynamoDB keeps no log beside its tables, so each of those now lives in the folded rows. The
//! first test is the claim in its sharpest form: the views are folded into a database that HAS NO
//! `ops` table, and they answer exactly as the store's own do.

use nexus_flow_core::model::{EdgeKind, LinkRelation, LinkWeight};
use nexus_flow_core::reducer::Reducer;
use nexus_flow_core::schema;
use nexus_flow_core::store::Store;
use nexus_flow_core::task_reducer::TaskReducer;
use rusqlite::{Connection, OptionalExtension};

/// A board that exercises every projection that used to need the log: a re-pointed parent, a
/// thread link whose weight moved, notes on two items, a custom field and a redacted note, written
/// at changing wall clocks — and a second replica's ops merged in.
fn board() -> Store {
    let mut a = Store::open_in_memory(1);
    a.set_wall_clock("2026-10-01T08:00:00Z");
    a.create_item("ab12.0001", "epic", "Epic one", "u");
    a.create_item("ab12.0002", "epic", "Epic two", "u");
    a.create_item("ab12.0003", "task", "Child", "u");
    a.set_wall_clock("2026-10-02T08:00:00Z");
    a.set_parent("ab12.0003", "ab12.0001", "u").unwrap();
    a.add_thread_link(
        "t-1",
        "ab12.0003",
        LinkRelation::Cited,
        LinkWeight::Passing,
        "u",
    );
    a.add_note("ab12.0003", "first note", "u");
    a.set_wall_clock("2026-10-03T08:00:00Z");
    a.set_parent("ab12.0003", "ab12.0002", "u").unwrap();
    a.add_thread_link(
        "t-1",
        "ab12.0003",
        LinkRelation::WorkedOn,
        LinkWeight::Bearing,
        "u",
    );
    let redacted = a.add_note("ab12.0001", "a note to take back", "u");
    a.redact_note(&redacted, "u");
    a.add_note("ab12.0003", "second note", "u");
    a.set_wall_clock("2026-10-04T08:00:00Z");
    a.set_field("ab12.0002", "priority", Some("1".into()), "u");

    let mut b = Store::open_in_memory(2);
    b.set_wall_clock("2026-10-05T08:00:00Z");
    b.apply(&a.export());
    b.add_note("ab12.0001", "from the other replica", "v");
    b.add_edge("ab12.0003", "ab12.0001", EdgeKind::Dep, "v");
    a.apply(&b.export());
    a
}

/// Every projection the log used to feed, as rows a test can compare.
fn projections(conn: &Connection) -> Vec<Vec<String>> {
    let rows = |sql: &str| -> Vec<String> {
        let mut stmt = conn.prepare(sql).unwrap();
        stmt.query_map([], |r| {
            let n = r.as_ref().column_count();
            Ok((0..n)
                .map(|i| {
                    r.get::<_, Option<String>>(i)
                        .unwrap()
                        .unwrap_or_else(|| "NULL".into())
                })
                .collect::<Vec<_>>()
                .join("|"))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    };
    vec![
        rows("SELECT child_id, parent_id FROM present_parent ORDER BY child_id, parent_id"),
        rows(
            "SELECT thread_id, item_id, relation, weight FROM present_thread_links
             ORDER BY thread_id, item_id",
        ),
        rows("SELECT item_id, created_at, updated_at FROM item_timestamps ORDER BY item_id"),
        rows(
            "SELECT n.item_id, n.body, n.created_at FROM notes n
             WHERE n.id NOT IN (SELECT note_id FROM note_tombstones)
             ORDER BY n.item_id, n.lamport, n.site",
        ),
    ]
}

#[test]
fn the_flow_views_fold_and_answer_in_a_database_with_no_op_log() {
    let store = board();

    let bare = Connection::open_in_memory().unwrap();
    schema::apply_flow_views(&bare);
    let reducer = TaskReducer;
    for op in store.export() {
        if op.lamport_in_bound() && reducer.is_foldable(&op) {
            nxs_foundation::change::apply_sqlite(&bare, &reducer.changes(&op)).unwrap();
        }
    }
    let has_ops: bool = bare
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'ops')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!has_ops, "the bare database has no log to lean on");

    let theirs = projections(&bare);
    assert_eq!(theirs, projections(store.connection()));
    // Not vacuous: the board really exercises the moves the projections have to resolve.
    assert_eq!(
        theirs[0],
        vec!["ab12.0003|ab12.0002"],
        "the re-pointed parent"
    );
    assert_eq!(
        theirs[1],
        vec!["t-1|ab12.0003|worked_on|bearing"],
        "the link whose weight moved"
    );
}

/// The read this replaced, kept as the oracle: the first and last `wall_clock` over the canonical
/// op order of every op whose target is the item.
fn timestamps_from_the_log(store: &Store, id: &str) -> (Option<String>, Option<String>) {
    let (created, updated): (Option<String>, Option<String>) = store
        .connection()
        .query_row(
            "SELECT FIRST_VALUE(wall_clock) OVER w, LAST_VALUE(wall_clock) OVER w
             FROM ops WHERE target_id = ?1
             WINDOW w AS (ORDER BY lamport, site
                          ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING)
             LIMIT 1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap()
        .unwrap_or((None, None));
    let norm = |v: Option<String>| v.filter(|s| !s.is_empty());
    (norm(created), norm(updated))
}

#[test]
fn folded_timestamps_equal_the_window_over_the_log_whatever_the_delivery_order() {
    let source = board();
    let mut ops = source.export();
    // An unstamped op on an item that is otherwise stamped: the window took '' as its first or
    // last value and read it as "none" — the folded register must do the same.
    let mut unstamped = Store::open_in_memory(3);
    unstamped.apply(&ops);
    unstamped.set_field("ab12.0001", "priority", Some("2".into()), "w");
    ops = unstamped.export();

    for reversed in [false, true] {
        let mut replica = Store::open_in_memory(4);
        let mut delivery = ops.clone();
        if reversed {
            delivery.reverse();
        }
        for op in &delivery {
            replica.apply(std::slice::from_ref(op));
        }
        for id in ["ab12.0001", "ab12.0002", "ab12.0003", "ab12.9999"] {
            assert_eq!(
                replica.item_timestamps_of(id).unwrap(),
                timestamps_from_the_log(&replica, id),
                "{id}, delivered {}",
                if reversed { "newest first" } else { "in order" }
            );
        }
    }
}

#[test]
fn a_workspace_written_at_schema_v7_opens_with_its_views_moved_off_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v7.db");
    let p = path.to_str().unwrap();
    let expected = {
        let mut s = Store::open(p, 1).unwrap();
        s.apply(&board().export());
        projections(s.connection())
    };
    assert!(!expected[0].is_empty());
    // Put the file back to what a v7 binary left: the views without the v8 columns and table,
    // the two projections joining the log, and no fold revision recorded.
    {
        let conn = Connection::open(p).unwrap();
        conn.execute_batch(
            "DROP VIEW present_parent;
             DROP VIEW present_thread_links;
             DROP TABLE item_timestamps;
             ALTER TABLE edge_adds DROP COLUMN lamport;
             ALTER TABLE edge_adds DROP COLUMN site;
             ALTER TABLE thread_link_adds DROP COLUMN lamport;
             ALTER TABLE thread_link_adds DROP COLUMN site;
             ALTER TABLE notes DROP COLUMN lamport;
             ALTER TABLE notes DROP COLUMN site;
             ALTER TABLE notes DROP COLUMN created_at;
             ALTER TABLE view_watermarks DROP COLUMN fold_revision;
             ALTER TABLE view_watermarks DROP COLUMN revision_through;
             CREATE VIEW present_parent AS
                 SELECT a.from_id AS child_id, a.to_id AS parent_id
                 FROM edge_adds a JOIN ops o ON o.op_id = a.tag
                 WHERE a.kind = 'parent'
                   AND NOT EXISTS (SELECT 1 FROM edge_removes r WHERE r.tag = a.tag)
                   AND NOT EXISTS (
                       SELECT 1 FROM edge_adds a2 JOIN ops o2 ON o2.op_id = a2.tag
                       WHERE a2.from_id = a.from_id AND a2.kind = 'parent'
                         AND NOT EXISTS (SELECT 1 FROM edge_removes r2 WHERE r2.tag = a2.tag)
                         AND (o2.lamport, o2.site, a2.to_id) > (o.lamport, o.site, a.to_id));
             CREATE VIEW present_thread_links AS
                 SELECT a.thread_id, a.item_id, a.relation, a.weight
                 FROM thread_link_adds a JOIN ops o ON o.op_id = a.tag
                 WHERE NOT EXISTS (SELECT 1 FROM thread_link_removes r WHERE r.tag = a.tag);",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 7).unwrap();
    }

    let s = Store::open(p, 1).unwrap();
    assert_eq!(
        projections(s.connection()),
        expected,
        "every projection is answered from the folded rows, filled on the open"
    );
    for view in ["present_parent", "present_thread_links"] {
        let sql: String = s
            .connection()
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='view' AND name=?1",
                [view],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !sql.contains("ops"),
            "{view} no longer reads the log: {sql}"
        );
    }
}

#[test]
fn a_row_an_older_binary_adds_after_the_upgrade_neither_doubles_a_parent_nor_stays_unfilled() {
    // Review of PR #22, Code Quality #1. An older binary still writing the file adds an edge row
    // without the coordinate. Until this build opens the file again the projection must still give
    // the child ONE parent — a NULL in the compare used to make every row a winner — and the open
    // then fills the row and decides by the real coordinates.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("w.db");
    let p = path.to_str().unwrap();
    {
        let mut s = Store::open(p, 1).unwrap();
        s.create_item("ab12.0001", "epic", "One", "u");
        s.create_item("ab12.0002", "epic", "Two", "u");
        s.create_item("ab12.0003", "task", "Child", "u");
        s.add_parent("ab12.0003", "ab12.0001", "u");
    }
    // What the older binary does for a second parent edge: the op into the log, the row without
    // the coordinate, and nothing to the revision's mark.
    {
        let conn = Connection::open(p).unwrap();
        let next: i64 = conn
            .query_row("SELECT MAX(lamport) + 1 FROM ops", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO ops(op_id, lamport, site, domain, target_kind, target_id, field, op_type,
                             value, author, wall_clock)
             VALUES('older-binary-op', ?1, 1, 'task', 'edge',
                    'ab12.0003' || char(31) || 'ab12.0002' || char(31) || 'parent',
                    'present', 'add', NULL, 'u', '')",
            [next],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO edge_adds(from_id, to_id, kind, tag)
             VALUES('ab12.0003', 'ab12.0002', 'parent', 'older-binary-op')",
            [],
        )
        .unwrap();
        let parents: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM present_parent WHERE child_id='ab12.0003'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(parents, 1, "one present parent while the row is unfilled");
    }
    let s = Store::open(p, 1).unwrap();
    let unfilled: i64 = s
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM edge_adds WHERE lamport IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unfilled, 0, "the open filled the older binary's row");
    let winner: String = s
        .connection()
        .query_row(
            "SELECT parent_id FROM present_parent WHERE child_id='ab12.0003'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        winner, "ab12.0002",
        "the later add wins by its real coordinate"
    );
}
