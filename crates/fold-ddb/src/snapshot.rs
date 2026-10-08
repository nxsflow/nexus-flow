//! A snapshot is tables plus watermark, without the log (6j6v.vvw6 point 6) — only to start a fold
//! run, never instead of one.
//!
//! [`export`] reads the watermark FIRST and the tables after, with no lock: a fold that runs in
//! between may already have written ops past the watermark into the tables, and the run that starts
//! from the snapshot folds those ops again from the relay — which changes nothing, because every
//! change is idempotent. The order is what makes that safe: tables read before the watermark could
//! lack an op the watermark claims.
//!
//! [`import`] starts an EMPTY stream from one: it writes the rows, rebuilds the board's indexes from
//! them, and writes the watermark last, so a stream whose import died has no watermark and is
//! started again. The log stays where it is — on the relay — and the run then folds it from the
//! watermark on, in batches.
//!
//! [`from_sqlite`] makes one from a local replica's views, for a server that starts from a board
//! someone already has. A local replica's own snapshot (`nxs sync snapshot`, 6j6v.mxt2) keeps
//! carrying its log: a replica started from it is an ordinary replica that can refold.
//!
//! # Point 7
//!
//! E4 stands (owner, 2026-10-06): one relay position is the whole watermark, and no version vector
//! is kept. A snapshot is only taken of what is already on the relay — a local replica's export
//! refuses ops it has not pushed — so the position describes everything in it.

use crate::board;
use crate::fold::{Folder, Watermark};
use crate::layout::{row_key, table_prefix, LayoutError};
use crate::table::{Row, Table};
use crate::write::Write;
use nxs_foundation::change::Cell;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The snapshot format this build writes and reads.
pub const FORMAT: u32 = 1;

/// One cell in a snapshot: text, an integer, or null.
pub type Value = serde_json::Value;

/// Tables plus watermark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub format: u32,
    pub stream_id: String,
    pub folded_through: i64,
    /// Each reducer's fold revision, by domain: the rules the tables were folded by.
    pub revisions: BTreeMap<String, i64>,
    /// Every view table's rows: the cells each row holds, by column.
    pub tables: BTreeMap<String, Vec<BTreeMap<String, Value>>>,
}

/// Why a snapshot could not be loaded. Nothing was written, except where it says so.
#[derive(Debug)]
pub enum ImportError<E> {
    Table(E),
    Layout(LayoutError),
    /// It belongs to another stream.
    WrongStream {
        snapshot: String,
        table: String,
    },
    /// Its format is newer than this build reads.
    Format(u32),
    /// Its tables were folded by other rules than this build's: start from the relay instead.
    OtherRevisions {
        snapshot: BTreeMap<String, i64>,
        folder: BTreeMap<String, i64>,
    },
    /// The stream already has a watermark: clear it first.
    NotEmpty,
    /// A cell that is neither text, an integer nor null.
    BadCell {
        table: String,
        column: String,
    },
}

impl<E: std::fmt::Display> std::fmt::Display for ImportError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Table(e) => write!(f, "the views table refused a request: {e}"),
            ImportError::Layout(e) => write!(f, "{e}"),
            ImportError::WrongStream { snapshot, table } => {
                write!(
                    f,
                    "the snapshot is of stream {snapshot:?}, the table of {table:?}"
                )
            }
            ImportError::Format(v) => write!(f, "snapshot format {v} is newer than {FORMAT}"),
            ImportError::OtherRevisions { snapshot, folder } => write!(
                f,
                "the snapshot was folded by revisions {snapshot:?}, this build folds by {folder:?}"
            ),
            ImportError::NotEmpty => write!(f, "the stream is already folded; clear it first"),
            ImportError::BadCell { table, column } => {
                write!(
                    f,
                    "{table}.{column} holds a value that is no text, integer or null"
                )
            }
        }
    }
}

impl<E: std::error::Error> std::error::Error for ImportError<E> {}

fn to_value(cell: &Cell) -> Value {
    match cell {
        Cell::Null => Value::Null,
        Cell::Text(v) => Value::String(v.clone()),
        Cell::Int(v) => Value::from(*v),
    }
}

fn to_cell(value: &Value) -> Option<Cell> {
    match value {
        Value::Null => Some(Cell::Null),
        Value::String(v) => Some(Cell::Text(v.clone())),
        Value::Number(n) => n.as_i64().map(Cell::Int),
        _ => None,
    }
}

/// The view cells of a stored row — without the table's keys and the index attributes.
fn view_cells(row: &Row, columns: &[String]) -> BTreeMap<String, Value> {
    row.iter()
        .filter(|(a, _)| columns.contains(a))
        .map(|(a, c)| (a.clone(), to_value(c)))
        .collect()
}

/// A snapshot of `table`: the watermark first, then the tables.
pub async fn export<T: Table>(table: &T, folder: &Folder) -> Result<Option<Snapshot>, T::Error> {
    let Some(mark) = Folder::watermark(table).await? else {
        return Ok(None);
    };
    let mut tables = BTreeMap::new();
    for (name, layout) in folder.layout().tables() {
        let rows = table.query_prefix(&table_prefix(name)).await?;
        tables.insert(
            name.to_string(),
            rows.iter()
                .map(|r| view_cells(r, &layout.columns))
                .collect(),
        );
    }
    Ok(Some(Snapshot {
        format: FORMAT,
        stream_id: table.stream().to_string(),
        folded_through: mark.folded_through,
        revisions: mark.revisions,
        tables,
    }))
}

/// Start the empty stream `table` from `snapshot`.
pub async fn import<T: Table>(
    table: &T,
    folder: &Folder,
    snapshot: &Snapshot,
) -> Result<(), ImportError<T::Error>> {
    if snapshot.format > FORMAT {
        return Err(ImportError::Format(snapshot.format));
    }
    if snapshot.stream_id != table.stream() {
        return Err(ImportError::WrongStream {
            snapshot: snapshot.stream_id.clone(),
            table: table.stream().to_string(),
        });
    }
    if snapshot.revisions != folder.revisions() {
        return Err(ImportError::OtherRevisions {
            snapshot: snapshot.revisions.clone(),
            folder: folder.revisions(),
        });
    }
    if Folder::watermark(table)
        .await
        .map_err(ImportError::Table)?
        .is_some()
    {
        return Err(ImportError::NotEmpty);
    }
    // Every row is checked before the first is written.
    let mut writes = Vec::new();
    for (name, rows) in &snapshot.tables {
        let layout = folder.layout().table(name).map_err(ImportError::Layout)?;
        for row in rows {
            let mut set = Vec::new();
            for (column, value) in row {
                if !layout.columns.contains(column) {
                    return Err(ImportError::Layout(LayoutError::UnknownColumn {
                        table: name.clone(),
                        column: column.clone(),
                    }));
                }
                let cell = to_cell(value).ok_or_else(|| ImportError::BadCell {
                    table: name.clone(),
                    column: column.clone(),
                })?;
                set.push((column.clone(), cell));
            }
            let key: Vec<(String, Cell)> = layout
                .key
                .iter()
                .map(|k| {
                    let cell = set
                        .iter()
                        .find(|(c, _)| c == k)
                        .map(|(_, v)| v.clone())
                        .unwrap_or(Cell::Null);
                    (k.clone(), cell)
                })
                .collect();
            let sk = row_key(name, &key).map_err(ImportError::Layout)?;
            let (set, remove) = crate::write::null_as_absent(layout, set);
            writes.push(Write {
                remove,
                ..Write::put(sk, set)
            });
        }
    }
    for write in &writes {
        table.write(write).await.map_err(ImportError::Table)?;
    }
    board::reindex(table).await.map_err(ImportError::Table)?;
    // A NEW watermark, not an advance: the stream had none.
    folder
        .advance(table, snapshot.folded_through)
        .await
        .map_err(|e| match e {
            crate::fold::FoldError::Table(e) => ImportError::Table(e),
            crate::fold::FoldError::Layout(e) => ImportError::Layout(e),
        })?;
    Ok(())
}

/// A snapshot of a local replica's views, reaching relay position `folded_through`.
///
/// The caller vouches for the position: it is the replica's `pulled_through`, and the replica holds
/// no op it has not pushed — what `nxs sync snapshot` checks before it writes one. `conn` is a store
/// this build opened, so its views are folded by this build's revisions.
pub fn from_sqlite(
    conn: &rusqlite::Connection,
    folder: &Folder,
    stream_id: &str,
    folded_through: i64,
) -> rusqlite::Result<Snapshot> {
    let mut tables = BTreeMap::new();
    for (name, layout) in folder.layout().tables() {
        // A replica of one product has no other product's views: none of their rows either.
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [name],
            |r| r.get(0),
        )?;
        if !exists {
            continue;
        }
        let mut stmt = conn.prepare(&format!("SELECT * FROM {name}"))?;
        let columns: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
        let rows = stmt
            .query_map([], |r| {
                let mut row = BTreeMap::new();
                for (i, c) in columns.iter().enumerate() {
                    if !layout.columns.contains(c) {
                        continue;
                    }
                    let value = match r.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => Value::Null,
                        rusqlite::types::ValueRef::Integer(v) => Value::from(v),
                        rusqlite::types::ValueRef::Text(t) => {
                            Value::String(String::from_utf8_lossy(t).into_owned())
                        }
                        other => {
                            return Err(rusqlite::Error::InvalidColumnType(
                                i,
                                c.clone(),
                                other.data_type(),
                            ))
                        }
                    };
                    row.insert(c.clone(), value);
                }
                Ok(row)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        tables.insert(name.to_string(), rows);
    }
    Ok(Snapshot {
        format: FORMAT,
        stream_id: stream_id.to_string(),
        folded_through,
        revisions: folder.revisions(),
        tables,
    })
}

/// The watermark a snapshot starts a stream at.
pub fn watermark(snapshot: &Snapshot) -> Watermark {
    Watermark {
        folded_through: snapshot.folded_through,
        revisions: snapshot.revisions.clone(),
    }
}
