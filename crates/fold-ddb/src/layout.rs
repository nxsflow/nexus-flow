//! Where a folded row lives in the table, and what a cell it never wrote reads as.
//!
//! A row of view table `t` with key cells `k1, k2, …` is the item whose sort key is
//! `t#k1#k2…` ([`row_key`]) in its stream's partition. Its cells are attributes named as the
//! columns, the key cells included, so a row reads back without parsing its sort key. A key value
//! has `%` and `#` escaped (`%25`, `%23`), so no value can reach into the next key cell.
//!
//! The defaults are SQLite's: a column a row never wrote reads as the `DEFAULT` the local schema
//! declares for it, and a register version compares against that default exactly as the local
//! `ON CONFLICT … WHERE` does. [`Layout::platform`] reads them from the very schema a local
//! replica creates, so the two cannot drift apart.

use nxs_foundation::change::Cell;
use rusqlite::Connection;
use std::collections::BTreeMap;

/// The table's partition key: the stream id.
pub const PK: &str = "stream_id";
/// The table's sort key.
pub const SK: &str = "sk";
/// The partition key of the `active` index: the stream id, present only on an active ticket and on
/// a present `dep`/`parent` edge with at least one active end.
pub const ACTIVE: &str = "nxf_active";
/// The partition key of the `dated` index: `<stream>#closed` or `<stream>#archived`, present only on
/// a ticket in that lane.
pub const DATED: &str = "nxf_dated";
/// The sort key of the `dated` index: the instant the ticket entered its lane.
pub const DATED_AT: &str = "nxf_dated_at";
/// The `active` index.
pub const ACTIVE_INDEX: &str = "active";
/// The `dated` index.
pub const DATED_INDEX: &str = "dated";

/// Attribute names no view column may take: the table's own keys and the index attributes.
pub const RESERVED: [&str; 5] = [PK, SK, ACTIVE, DATED, DATED_AT];

/// The fold's own bookkeeping, outside every view table: a view table's name is an identifier and
/// cannot start with `.`.
pub const WATERMARK_KEY: &str = ".meta#watermark";
const ADJACENCY: &str = ".adj#";

/// The sort key of a row of `table` with these key cells.
pub fn row_key(table: &str, key: &[(String, Cell)]) -> Result<String, LayoutError> {
    let mut sk = table_prefix(table);
    for (i, (column, cell)) in key.iter().enumerate() {
        if i > 0 {
            sk.push('#');
        }
        match cell {
            Cell::Text(v) => sk.push_str(&escape(v)),
            // SQLite's TEXT affinity stores an integer key as its decimal text: the same row.
            Cell::Int(v) => sk.push_str(&v.to_string()),
            Cell::Null => {
                return Err(LayoutError::NullKey {
                    table: table.to_string(),
                    column: column.clone(),
                })
            }
        }
    }
    Ok(sk)
}

/// The prefix every row of `table` starts with.
pub fn table_prefix(table: &str) -> String {
    format!("{table}#")
}

/// The adjacency entry that says edge `tag` touches ticket `item` — what lets a ticket whose
/// activity flips find its edges with one Query.
pub(crate) fn adjacency_key(item: &str, tag: &str) -> String {
    format!("{}{}", adjacency_prefix(item), escape(tag))
}

pub(crate) fn adjacency_prefix(item: &str) -> String {
    format!("{ADJACENCY}{}#", escape(item))
}

fn escape(v: &str) -> String {
    v.replace('%', "%25").replace('#', "%23")
}

/// One view table: its key columns in order, every column, and the defaults SQLite declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableLayout {
    pub key: Vec<String>,
    pub columns: Vec<String>,
    pub defaults: BTreeMap<String, Cell>,
}

impl TableLayout {
    /// What `column` reads as in a row that never wrote it.
    pub fn default_of(&self, column: &str) -> Cell {
        self.defaults.get(column).cloned().unwrap_or(Cell::Null)
    }
}

/// Every view table a fold writes, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    tables: BTreeMap<String, TableLayout>,
}

/// Why a layout could not be read, or a row has no place in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    Sqlite(String),
    /// A view table the schema does not have.
    UnknownTable(String),
    /// A column the table does not have.
    UnknownColumn {
        table: String,
        column: String,
    },
    /// A column named like one of the table's own attributes ([`RESERVED`]).
    ReservedColumn {
        table: String,
        column: String,
    },
    /// A `DEFAULT` that is not a plain integer, string or NULL.
    UnsupportedDefault {
        table: String,
        column: String,
        default: String,
    },
    /// A key cell that is NULL.
    NullKey {
        table: String,
        column: String,
    },
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutError::Sqlite(e) => write!(f, "reading the view schema: {e}"),
            LayoutError::UnknownTable(t) => write!(f, "no view table {t:?}"),
            LayoutError::UnknownColumn { table, column } => {
                write!(f, "view table {table:?} has no column {column:?}")
            }
            LayoutError::ReservedColumn { table, column } => {
                write!(f, "view table {table:?} names a column {column:?}, which the table reserves")
            }
            LayoutError::UnsupportedDefault { table, column, default } => write!(
                f,
                "view table {table:?} declares DEFAULT {default} on {column:?}, which is no plain value"
            ),
            LayoutError::NullKey { table, column } => {
                write!(f, "a row of {table:?} has a NULL key cell {column:?}")
            }
        }
    }
}

impl std::error::Error for LayoutError {}

impl Layout {
    /// The layout of `tables` as `conn`'s schema declares them.
    pub fn from_sqlite(conn: &Connection, tables: &[&str]) -> Result<Layout, LayoutError> {
        let sqlite = |e: rusqlite::Error| LayoutError::Sqlite(e.to_string());
        let mut out = BTreeMap::new();
        for &table in tables {
            let mut stmt = conn
                .prepare("SELECT name, dflt_value, pk FROM pragma_table_info(?1)")
                .map_err(sqlite)?;
            let rows = stmt
                .query_map([table], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })
                .map_err(sqlite)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sqlite)?;
            if rows.is_empty() {
                return Err(LayoutError::UnknownTable(table.to_string()));
            }
            let mut key: Vec<(i64, String)> = Vec::new();
            let mut columns = Vec::new();
            let mut defaults = BTreeMap::new();
            for (name, default, pk) in rows {
                if RESERVED.contains(&name.as_str()) {
                    return Err(LayoutError::ReservedColumn {
                        table: table.to_string(),
                        column: name,
                    });
                }
                if pk > 0 {
                    key.push((pk, name.clone()));
                }
                if let Some(text) = default {
                    match parse_default(&text) {
                        Some(Cell::Null) => {}
                        Some(cell) => {
                            defaults.insert(name.clone(), cell);
                        }
                        None => {
                            return Err(LayoutError::UnsupportedDefault {
                                table: table.to_string(),
                                column: name,
                                default: text,
                            })
                        }
                    }
                }
                columns.push(name);
            }
            key.sort();
            out.insert(
                table.to_string(),
                TableLayout {
                    key: key.into_iter().map(|(_, c)| c).collect(),
                    columns,
                    defaults,
                },
            );
        }
        Ok(Layout { tables: out })
    }

    /// The platform's views — the board's, the chat's and the memory's — read from the schema a
    /// local replica creates.
    pub fn platform() -> Result<Layout, LayoutError> {
        let conn = Connection::open_in_memory().map_err(|e| LayoutError::Sqlite(e.to_string()))?;
        platform_schema(&conn).map_err(|e| LayoutError::Sqlite(e.to_string()))?;
        Layout::from_sqlite(&conn, &crate::fold::platform_view_tables())
    }

    /// One table's layout.
    pub fn table(&self, name: &str) -> Result<&TableLayout, LayoutError> {
        self.tables
            .get(name)
            .ok_or_else(|| LayoutError::UnknownTable(name.to_string()))
    }

    /// Every table, by name.
    pub fn tables(&self) -> impl Iterator<Item = (&str, &TableLayout)> {
        self.tables.iter().map(|(n, t)| (n.as_str(), t))
    }
}

/// The local schema of every platform view, on `conn`.
pub(crate) fn platform_schema(conn: &Connection) -> rusqlite::Result<()> {
    nxs_foundation::schema::try_apply(conn)?;
    nexus_flow_core::schema::try_apply_flow_views(conn)?;
    nexus_chat::schema::try_apply_chat_views(conn)?;
    nexus_memory::schema::try_apply_memory_views(conn)?;
    Ok(())
}

/// A `DEFAULT` as `pragma_table_info` spells it: an integer, a single-quoted string, or NULL.
fn parse_default(text: &str) -> Option<Cell> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("null") {
        return Some(Cell::Null);
    }
    if let Ok(v) = text.parse::<i64>() {
        return Some(Cell::Int(v));
    }
    let inner = text.strip_prefix('\'')?.strip_suffix('\'')?;
    // Inside the quotes a quote is doubled; a lone one would have ended the literal.
    (!inner.replace("''", "").contains('\'')).then(|| Cell::Text(inner.replace("''", "'")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_value_cannot_reach_into_the_next_key_cell() {
        let k = |a: &str, b: &str| {
            row_key(
                "t",
                &[
                    ("a".into(), Cell::Text(a.into())),
                    ("b".into(), Cell::Text(b.into())),
                ],
            )
            .unwrap()
        };
        assert_ne!(k("x#y", "z"), k("x", "y#z"));
        assert_ne!(k("x%23", "y"), k("x#", "y"));
        assert_eq!(k("x#y", "z"), "t#x%23y#z");
        assert!(row_key("t", &[("a".into(), Cell::Null)]).is_err());
    }

    #[test]
    fn defaults_parse_as_sqlite_spells_them() {
        assert_eq!(parse_default("0"), Some(Cell::Int(0)));
        assert_eq!(parse_default("-3"), Some(Cell::Int(-3)));
        assert_eq!(parse_default("''"), Some(Cell::Text(String::new())));
        assert_eq!(parse_default("'it''s'"), Some(Cell::Text("it's".into())));
        assert_eq!(parse_default("NULL"), Some(Cell::Null));
        assert_eq!(parse_default("CURRENT_TIMESTAMP"), None);
    }

    #[test]
    fn the_platform_layout_reads_every_view_with_its_key_and_defaults() {
        let layout = Layout::platform().unwrap();
        let items = layout.table("items").unwrap();
        assert_eq!(items.key, vec!["id"]);
        assert_eq!(items.default_of("status_v"), Cell::Int(0));
        assert_eq!(items.default_of("status"), Cell::Null);
        let custom = layout.table("custom_fields").unwrap();
        assert_eq!(custom.key, vec!["item_id", "field"]);
        let stamps = layout.table("item_timestamps").unwrap();
        assert_eq!(stamps.default_of("created_v"), Cell::Null);
        let memories = layout.table("memories").unwrap();
        assert_eq!(
            memories.default_of("category"),
            Cell::Text("unsorted".into())
        );
        assert!(layout.table("messages").is_ok());
    }
}
