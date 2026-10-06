//! Described changes (6j6v.vvw6): what folding one op does to the views, written down as data
//! instead of as SQL.
//!
//! A [`Reducer`](crate::reducer::Reducer) maps an op to a list of [`Change`]s, and an *applier*
//! carries them out against a store. The mapping exists exactly once, in the reducer; this module
//! holds the SQLite applier the substrate folds through, and a server folds the same list into
//! DynamoDB with conditional writes. Neither applier knows a product's vocabulary — a table, its
//! key and its cells are all the change says.
//!
//! # The three effects
//!
//! Every fold in the platform is one of three shapes, and each is commutative and idempotent, so
//! the order ops arrive in does not decide the result:
//!
//! - [`Effect::Ensure`] — a row that is created once with its starting cells and never written
//!   again by this effect (memory's row seeded before its registers).
//! - [`Effect::Put`] — a row every cell of which is a pure function of its key: an OR-set add keyed
//!   by its op id, a tombstone, a note. Writing it again writes the same values, which is what lets
//!   a refold fill a column an older binary left empty.
//! - [`Effect::Register`] — keep-if-beats: the cells move together with a `(lamport, site)` version,
//!   and only a version that [`Wins`] against the stored one writes. A stored version that is NULL
//!   is a register nobody wrote, and loses to any version.

use rusqlite::{types::ToSql, Connection};

/// One cell value. The views hold text and integers only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    Null,
    Text(String),
    Int(i64),
}

impl From<&str> for Cell {
    fn from(v: &str) -> Self {
        Cell::Text(v.to_string())
    }
}

impl From<String> for Cell {
    fn from(v: String) -> Self {
        Cell::Text(v)
    }
}

impl From<&String> for Cell {
    fn from(v: &String) -> Self {
        Cell::Text(v.clone())
    }
}

impl From<i64> for Cell {
    fn from(v: i64) -> Self {
        Cell::Int(v)
    }
}

impl<T: Into<Cell>> From<Option<T>> for Cell {
    fn from(v: Option<T>) -> Self {
        v.map_or(Cell::Null, Into::into)
    }
}

impl ToSql for Cell {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        match self {
            Cell::Null => rusqlite::types::Null.to_sql(),
            Cell::Text(v) => v.to_sql(),
            Cell::Int(v) => v.to_sql(),
        }
    }
}

/// A column name and the value written into it. Column names come from the reducer's own fixed
/// lists — never from an op — which is what keeps the applier's generated SQL injection-safe.
pub type Cells = Vec<(String, Cell)>;

/// An op's coordinate, the version a register compares on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub lamport: i64,
    pub site: i64,
}

/// Which version keeps a register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wins {
    /// The highest `(lamport, site)` — last writer wins, every ordinary register.
    Higher,
    /// The lowest `(lamport, site)` — the first writer wins, by the causal order rather than by
    /// arrival: an insertion instant, a message whose id two ops claim.
    Lower,
}

/// What a change does to its row. See the module doc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Ensure {
        cells: Cells,
    },
    Put {
        cells: Cells,
    },
    Register {
        cells: Cells,
        /// The columns holding the register's version: `(lamport column, site column)`.
        version_columns: (String, String),
        version: Version,
        wins: Wins,
    },
}

/// One described change: a row of `table`, named by `key`, and what happens to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub table: &'static str,
    pub key: Cells,
    pub effect: Effect,
}

impl Change {
    /// An [`Effect::Ensure`] change.
    pub fn ensure(table: &'static str, key: Cells, cells: Cells) -> Change {
        Change {
            table,
            key,
            effect: Effect::Ensure { cells },
        }
    }

    /// An [`Effect::Put`] change.
    pub fn put(table: &'static str, key: Cells, cells: Cells) -> Change {
        Change {
            table,
            key,
            effect: Effect::Put { cells },
        }
    }

    /// An [`Effect::Register`] change.
    pub fn register(
        table: &'static str,
        key: Cells,
        cells: Cells,
        version_columns: (&str, &str),
        version: Version,
        wins: Wins,
    ) -> Change {
        Change {
            table,
            key,
            effect: Effect::Register {
                cells,
                version_columns: (version_columns.0.to_string(), version_columns.1.to_string()),
                version,
                wins,
            },
        }
    }
}

/// `[(column, value)]` from pairs — the shorthand the reducers build keys and cells with.
pub fn cells<const N: usize>(pairs: [(&str, Cell); N]) -> Cells {
    pairs.into_iter().map(|(c, v)| (c.to_string(), v)).collect()
}

/// The SQLite applier: carry `changes` out on `conn`, in order, inside whatever transaction the
/// caller holds. One statement per change.
pub fn apply_sqlite(conn: &Connection, changes: &[Change]) -> rusqlite::Result<()> {
    for change in changes {
        apply_one(conn, change)?;
    }
    Ok(())
}

fn apply_one(conn: &Connection, change: &Change) -> rusqlite::Result<()> {
    let table = change.table;
    let key_columns: Vec<&str> = change.key.iter().map(|(c, _)| c.as_str()).collect();
    let conflict = key_columns.join(", ");
    let (sql, values): (String, Vec<&Cell>) = match &change.effect {
        Effect::Ensure { cells } => {
            let (columns, values) = row(&change.key, cells, None);
            (
                format!(
                    "INSERT OR IGNORE INTO {table}({}) VALUES({})",
                    columns.join(", "),
                    placeholders(columns.len())
                ),
                values,
            )
        }
        Effect::Put { cells } => {
            let (columns, values) = row(&change.key, cells, None);
            let sql = if cells.is_empty() {
                format!(
                    "INSERT OR IGNORE INTO {table}({}) VALUES({})",
                    columns.join(", "),
                    placeholders(columns.len())
                )
            } else {
                format!(
                    "INSERT INTO {table}({}) VALUES({}) ON CONFLICT({conflict}) DO UPDATE SET {}",
                    columns.join(", "),
                    placeholders(columns.len()),
                    assignments(cells.iter().map(|(c, _)| c.as_str()))
                )
            };
            (sql, values)
        }
        Effect::Register {
            cells,
            version_columns: (v, s),
            version,
            wins,
        } => {
            let lamport = Cell::Int(version.lamport);
            let site = Cell::Int(version.site);
            let (columns, values) = row(&change.key, cells, Some((v, s, &lamport, &site)));
            let beats = match wins {
                Wins::Higher => ">",
                Wins::Lower => "<",
            };
            let moved = cells
                .iter()
                .map(|(c, _)| c.as_str())
                .chain([v.as_str(), s.as_str()]);
            let sql = format!(
                "INSERT INTO {table}({}) VALUES({}) ON CONFLICT({conflict}) DO UPDATE SET {}
                 WHERE {table}.{v} IS NULL OR {table}.{s} IS NULL
                    OR (excluded.{v}, excluded.{s}) {beats} ({table}.{v}, {table}.{s})",
                columns.join(", "),
                placeholders(columns.len()),
                assignments(moved)
            );
            // `values` borrows the two version cells, which live only in this arm.
            return conn
                .execute(&sql, rusqlite::params_from_iter(values.iter()))
                .map(drop);
        }
    };
    conn.execute(&sql, rusqlite::params_from_iter(values.iter()))
        .map(drop)
}

/// The columns and values of a whole row: its key, its cells and, for a register, the version.
fn row<'a>(
    key: &'a Cells,
    cells: &'a Cells,
    version: Option<(&'a String, &'a String, &'a Cell, &'a Cell)>,
) -> (Vec<&'a str>, Vec<&'a Cell>) {
    let mut columns = Vec::new();
    let mut values = Vec::new();
    for (c, v) in key.iter().chain(cells.iter()) {
        columns.push(c.as_str());
        values.push(v);
    }
    if let Some((v, s, lamport, site)) = version {
        columns.extend([v.as_str(), s.as_str()]);
        values.extend([lamport, site]);
    }
    (columns, values)
}

fn placeholders(n: usize) -> String {
    (1..=n)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn assignments<'a>(columns: impl Iterator<Item = &'a str>) -> String {
    columns
        .map(|c| format!("{c}=excluded.{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE reg(id TEXT PRIMARY KEY, v TEXT, v_l INTEGER DEFAULT 0, v_s INTEGER DEFAULT 0,
                              first TEXT, first_l INTEGER, first_s INTEGER);
             CREATE TABLE seed(id TEXT PRIMARY KEY, label TEXT NOT NULL DEFAULT '');
             CREATE TABLE pair(a TEXT NOT NULL, b TEXT NOT NULL, note TEXT, PRIMARY KEY(a, b));",
        )
        .unwrap();
        conn
    }

    fn at(lamport: i64, site: i64) -> Version {
        Version { lamport, site }
    }

    fn set(id: &str, value: &str, version: Version, wins: Wins) -> Change {
        let (cell, columns) = match wins {
            Wins::Higher => ("v", ("v_l", "v_s")),
            Wins::Lower => ("first", ("first_l", "first_s")),
        };
        Change::register(
            "reg",
            cells([("id", id.into())]),
            cells([(cell, value.into())]),
            columns,
            version,
            wins,
        )
    }

    fn read(conn: &Connection, column: &str) -> Option<String> {
        conn.query_row(&format!("SELECT {column} FROM reg WHERE id='x'"), [], |r| {
            r.get(0)
        })
        .unwrap()
    }

    #[test]
    fn a_higher_register_keeps_the_highest_version_whatever_the_order() {
        let writes = [
            set("x", "a", at(1, 1), Wins::Higher),
            set("x", "c", at(3, 1), Wins::Higher),
            set("x", "b", at(3, 0), Wins::Higher),
        ];
        for order in [[0, 1, 2], [2, 1, 0], [1, 0, 2], [1, 2, 0]] {
            let conn = db();
            for i in order {
                apply_sqlite(&conn, std::slice::from_ref(&writes[i])).unwrap();
            }
            assert_eq!(read(&conn, "v").as_deref(), Some("c"), "order {order:?}");
        }
    }

    #[test]
    fn a_lower_register_keeps_the_lowest_version_whatever_the_order() {
        let writes = [
            set("x", "late", at(5, 1), Wins::Lower),
            set("x", "early", at(2, 9), Wins::Lower),
            set("x", "tie-loser", at(2, 10), Wins::Lower),
        ];
        for order in [[0, 1, 2], [2, 1, 0], [1, 0, 2], [0, 2, 1]] {
            let conn = db();
            for i in order {
                apply_sqlite(&conn, std::slice::from_ref(&writes[i])).unwrap();
            }
            assert_eq!(
                read(&conn, "first").as_deref(),
                Some("early"),
                "order {order:?}"
            );
        }
    }

    #[test]
    fn a_lower_register_on_a_row_another_register_created_is_unwritten_not_lost() {
        // The row exists because a HIGHER register created it; the lower register's version
        // columns are NULL there, which must read as "nobody wrote this", not as a winner.
        let conn = db();
        apply_sqlite(&conn, &[set("x", "a", at(1, 1), Wins::Higher)]).unwrap();
        apply_sqlite(&conn, &[set("x", "first", at(7, 1), Wins::Lower)]).unwrap();
        assert_eq!(read(&conn, "first").as_deref(), Some("first"));
    }

    #[test]
    fn ensure_creates_once_and_never_overwrites() {
        let conn = db();
        let seed = |label: &str| {
            Change::ensure(
                "seed",
                cells([("id", "x".into())]),
                cells([("label", label.into())]),
            )
        };
        apply_sqlite(&conn, &[seed("first"), seed("second")]).unwrap();
        let label: String = conn
            .query_row("SELECT label FROM seed WHERE id='x'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(label, "first");
    }

    #[test]
    fn put_fills_a_cell_a_row_written_without_it_lacks() {
        // The case Put exists for: a row an older binary inserted without a column this one writes.
        let conn = db();
        conn.execute("INSERT INTO pair(a, b) VALUES('x', 'y')", [])
            .unwrap();
        let put = Change::put(
            "pair",
            cells([("a", "x".into()), ("b", "y".into())]),
            cells([("note", "filled".into())]),
        );
        apply_sqlite(&conn, &[put.clone(), put]).unwrap();
        let note: Option<String> = conn
            .query_row("SELECT note FROM pair", [], |r| r.get(0))
            .unwrap();
        assert_eq!(note.as_deref(), Some("filled"));
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM pair", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn a_put_with_no_cells_is_a_set_element() {
        let conn = db();
        conn.execute_batch("CREATE TABLE tags(tag TEXT PRIMARY KEY)")
            .unwrap();
        let add = Change::put("tags", cells([("tag", "t1".into())]), Vec::new());
        apply_sqlite(&conn, &[add.clone(), add]).unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn a_null_cell_is_written_as_null() {
        let conn = db();
        apply_sqlite(
            &conn,
            &[Change::register(
                "reg",
                cells([("id", "x".into())]),
                cells([("v", Cell::from(None::<&str>))]),
                ("v_l", "v_s"),
                at(1, 1),
                Wins::Higher,
            )],
        )
        .unwrap();
        assert_eq!(read(&conn, "v"), None);
    }
}
