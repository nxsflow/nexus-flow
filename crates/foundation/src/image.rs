//! A store's IMAGE (6j6v.mxt2): its op log, and the views one product folded from that log — the
//! substrate half of a snapshot.
//!
//! **Why the log rides along.** A snapshot could carry the views alone and be smaller. It carries
//! the log too, because the log stays the source everywhere else: the history an item shows, the
//! timestamps it reports, the prefix remap and every refold read it, and a replica without it would
//! be a new kind of replica those paths know nothing about. With the log, a store that starts from
//! an image IS an ordinary replica that happens to have folded quickly — and an image whose views
//! cannot be trusted as they are (another engine wrote them) is simply folded again from the log it
//! brought, instead of being refused.
//!
//! **What never rides along.** Only [`Reducer::view_tables`](crate::reducer::Reducer::view_tables)
//! — the tables a reducer declares as pure folds of the log. A product keeps other state in the same
//! database (chat's session map and transcripts, a lease, a queue); that is this machine's own and
//! must not reach another replica.
//!
//! The substrate knows nothing about relays: where the log was pulled from, and how far, is the sync
//! layer's half (`nxs_sync::snapshot`).

use crate::schema;
use rusqlite::types::{Value, ValueRef};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The version of the engine that takes an image — the one whose reducers folded its views.
///
/// **The key a reader trusts views by.** A reducer can change what it produces from the same ops
/// without the schema moving (a fold rule fixed, an op shape newly understood), so a schema match
/// alone does not prove two engines would fold alike. Every release carries its own version, so an
/// equal version is the claim "these views are what I would have folded"; anything else is folded
/// again from the image's log. Two development builds between releases share a version — the one
/// place the claim rests on the developer rather than the number.
///
/// **What it names, precisely** (review of PR #487, Code Quality #2): the engine that EXPORTS — the
/// one serving those views from that database — not necessarily the one that folded every row. A
/// database carried across an upgrade serves the views an older engine folded unless the upgrade
/// refolded them, and the migration checklist makes that refold mandatory for any change to what a
/// fold produces (rule 1: no change to a view's meaning without a schema step — and a schema step
/// makes an import refold). So an importer ends with exactly the views its source serves; a fold
/// revision recorded beside the watermark, which would make that independent of the checklist, is
/// 6j6v.y3r4.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// One SQLite value, as an image carries it: JSON `null`, a number or a string. The four storage
/// classes the log and the views use; a BLOB appears in neither and is refused rather than guessed
/// at.
///
/// (De)serialized by hand rather than `#[serde(untagged)]`: an untagged enum buffers every value
/// and tries each variant in turn, and an image is tens of thousands of cells — measured on a real
/// board, that alone made starting from a snapshot barely faster than folding the history.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
}

impl Serialize for Cell {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Cell::Null => s.serialize_unit(),
            Cell::Integer(i) => s.serialize_i64(*i),
            Cell::Real(r) => s.serialize_f64(*r),
            Cell::Text(t) => s.serialize_str(t),
        }
    }
}

impl<'de> Deserialize<'de> for Cell {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Cell, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Cell;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("null, a number or a string")
            }
            fn visit_unit<E>(self) -> Result<Cell, E> {
                Ok(Cell::Null)
            }
            fn visit_none<E>(self) -> Result<Cell, E> {
                Ok(Cell::Null)
            }
            fn visit_i64<E>(self, v: i64) -> Result<Cell, E> {
                Ok(Cell::Integer(v))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Cell, E> {
                i64::try_from(v)
                    .map(Cell::Integer)
                    .map_err(|_| E::custom("an integer beyond SQLite's range"))
            }
            fn visit_f64<E>(self, v: f64) -> Result<Cell, E> {
                Ok(Cell::Real(v))
            }
            fn visit_str<E>(self, v: &str) -> Result<Cell, E> {
                Ok(Cell::Text(v.to_string()))
            }
            fn visit_string<E>(self, v: String) -> Result<Cell, E> {
                Ok(Cell::Text(v))
            }
        }
        d.deserialize_any(Visitor)
    }
}

impl Cell {
    fn to_value(&self) -> Value {
        match self {
            Cell::Null => Value::Null,
            Cell::Integer(i) => Value::Integer(*i),
            Cell::Real(r) => Value::Real(*r),
            Cell::Text(t) => Value::Text(t.clone()),
        }
    }
}

/// One table, whole: its column names and every row, in `rowid` order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}

/// A store's op log and one product's views, read in ONE transaction — see the module doc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Image {
    /// [`ENGINE_VERSION`] of the engine that took the image.
    pub engine: String,
    /// The schema the database stood at, and the compatibility floor it carried
    /// ([`schema::SCHEMA_VERSION`], [`schema::MIN_COMPATIBLE_SCHEMA_VERSION`]).
    pub schema_version: i64,
    pub min_compatible: i64,
    /// The whole log: `rowid` first, then every column of `ops`. The rowids are carried because a
    /// product's folded-through watermark is a rowid, and it has to mean the same op on both sides.
    pub ops: Table,
    /// Whose views these are — the `view_watermarks` key of the store that took the image — and how
    /// far that store had folded the log. `None` when the image carries no views.
    pub views_of: Option<String>,
    pub folded_through: i64,
    /// Every table the store's registered reducers declare as views.
    pub views: Vec<Table>,
}

impl Image {
    /// How many ops the image carries.
    pub fn op_count(&self) -> usize {
        self.ops.rows.len()
    }

    /// Whether the log could be loaded as it stands — every check `Store::load_image` makes on it
    /// before writing, without a store (see `typed_log`). Lets a caller refuse a bad image before it
    /// spends anything else, a network round-trip for instance.
    pub fn check(&self) -> Result<(), ImageError> {
        typed_log(&self.ops).map(|_| ())
    }

    /// The highest rowid the log carries (0 for an empty one) — where a replica started from this
    /// image stands, and so where its OWN ops begin.
    pub fn max_rowid(&self) -> i64 {
        let Some(at) = self.ops.columns.iter().position(|c| c == "rowid") else {
            return 0;
        };
        self.ops
            .rows
            .iter()
            .filter_map(|row| match row.get(at) {
                Some(Cell::Integer(r)) => Some(*r),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }

    /// The id of every op the image carries.
    pub fn op_ids(&self) -> HashSet<&str> {
        let Some(at) = self.ops.columns.iter().position(|c| c == "op_id") else {
            return HashSet::new();
        };
        self.ops
            .rows
            .iter()
            .filter_map(|row| match row.get(at) {
                Some(Cell::Text(id)) => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// What [`Store::load_image`](crate::store::Store::load_image) did with the image's views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    /// Taken as they were: the log went in, the views went in, nothing was folded.
    Views,
    /// The log went in and the views were folded again from it, for this reason.
    Refolded(RefoldReason),
}

/// Why an image's views were not taken as they were.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefoldReason {
    /// The image carries no views at all.
    NoViews,
    /// Another engine folded them — its reducers may not produce what this one does.
    OtherEngine { written_by: String },
    /// The database they came from stood at another schema.
    OtherSchema { written_at: i64 },
    /// They are another product's views, or a different set of tables than this store's reducers
    /// fold into.
    OtherViews,
}

impl std::fmt::Display for RefoldReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RefoldReason::NoViews => write!(f, "the snapshot carries no folded views"),
            RefoldReason::OtherEngine { written_by } => write!(
                f,
                "nxs {written_by} folded its views and this is nxs {ENGINE_VERSION}"
            ),
            RefoldReason::OtherSchema { written_at } => write!(
                f,
                "its views come from schema v{written_at} and this nxs folds into v{}",
                schema::SCHEMA_VERSION
            ),
            RefoldReason::OtherViews => {
                write!(f, "its views are not the ones this store folds into")
            }
        }
    }
}

/// Why an image could not be loaded at all. Nothing was written in any of these cases.
#[derive(Debug)]
pub enum ImageError {
    /// The store already holds ops. An image starts a FRESH replica; one that already holds ops
    /// syncs the ordinary way.
    NotEmpty {
        ops: i64,
    },
    /// The image came from a database whose compatibility floor is above this engine — a newer nxs
    /// made a change this one cannot fold correctly.
    TooNew {
        min_compatible: i64,
        ours: i64,
    },
    /// The image is not shaped the way an image is.
    Malformed(String),
    Storage(rusqlite::Error),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::NotEmpty { ops } => write!(
                f,
                "this store already holds {ops} ops — a snapshot starts a fresh replica, and one \
                 that already has a log syncs the ordinary way"
            ),
            ImageError::TooNew {
                min_compatible,
                ours,
            } => write!(
                f,
                "the snapshot needs an nxs that speaks at least schema v{min_compatible}; this one \
                 speaks v{ours} — upgrade nxs to start from it"
            ),
            ImageError::Malformed(why) => write!(f, "the snapshot is malformed: {why}"),
            ImageError::Storage(e) => write!(f, "loading the snapshot: {e}"),
        }
    }
}

impl std::error::Error for ImageError {}

impl From<rusqlite::Error> for ImageError {
    fn from(e: rusqlite::Error) -> ImageError {
        ImageError::Storage(e)
    }
}

/// Read one table whole, in `rowid` order — with the rowid itself as the first column when asked.
/// `table` is always a name this engine chose (the log, or a reducer's declared view), never input.
pub(crate) fn dump(conn: &Connection, table: &str, with_rowid: bool) -> rusqlite::Result<Table> {
    let select = if with_rowid { "rowid, *" } else { "*" };
    let mut stmt = conn.prepare(&format!("SELECT {select} FROM {table} ORDER BY rowid"))?;
    let columns: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
    let width = columns.len();
    let rows = stmt
        .query_map([], |r| {
            (0..width)
                .map(|i| {
                    Ok(match r.get_ref(i)? {
                        ValueRef::Null => Cell::Null,
                        ValueRef::Integer(v) => Cell::Integer(v),
                        ValueRef::Real(v) => Cell::Real(v),
                        ValueRef::Text(t) => Cell::Text(String::from_utf8_lossy(t).into_owned()),
                        ValueRef::Blob(_) => {
                            return Err(rusqlite::Error::InvalidColumnType(
                                i,
                                format!("{table}.{}", columns[i]),
                                rusqlite::types::Type::Blob,
                            ))
                        }
                    })
                })
                .collect::<rusqlite::Result<Vec<Cell>>>()
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Table {
        name: table.to_string(),
        columns,
        rows,
    })
}

/// The columns `table` has in THIS database, each with its declared type, upper-cased (`rowid`
/// counts as one for a rowid table, typed `INTEGER`).
pub(crate) fn columns_of(
    conn: &Connection,
    table: &str,
) -> rusqlite::Result<std::collections::HashMap<String, String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut columns: std::collections::HashMap<String, String> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?.to_uppercase(),
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    columns.insert("rowid".to_string(), "INTEGER".to_string());
    Ok(columns)
}

/// Whether `cell` fits a column declared `declared`. SQLite would store almost anything almost
/// anywhere, and every reader of a view `.unwrap()`s the type the reducer wrote — so an image may
/// put only NULL or an integer into an `INTEGER` column and only NULL or text into a `TEXT` one.
fn fits(cell: &Cell, declared: &str) -> bool {
    match cell {
        Cell::Null => true,
        Cell::Integer(_) => !declared.contains("TEXT") && !declared.contains("CHAR"),
        Cell::Text(_) => !declared.contains("INT"),
        Cell::Real(_) => !declared.contains("INT") && !declared.contains("TEXT"),
    }
}

/// Insert a view table's rows into the table of that name. Column names come from the image — input,
/// not code — so only a name the target table really has is ever put into the statement; a column
/// this engine does not know is left behind (see `Store::load_image`), and every cell must fit its
/// column's declared type ([`fits`]).
pub(crate) fn insert_view(
    conn: &Connection,
    target: &str,
    table: &Table,
) -> Result<(), ImageError> {
    let known = columns_of(conn, target)?;
    let keep: Vec<(usize, &str)> = table
        .columns
        .iter()
        .enumerate()
        .filter_map(|(i, c)| known.get(c).map(|declared| (i, declared.as_str())))
        .collect();
    let names: Vec<String> = keep
        .iter()
        .map(|&(i, _)| format!("\"{}\"", table.columns[i]))
        .collect();
    let slots: Vec<String> = (1..=keep.len()).map(|n| format!("?{n}")).collect();
    let mut stmt = conn.prepare(&format!(
        "INSERT INTO {target} ({}) VALUES ({})",
        names.join(", "),
        slots.join(", ")
    ))?;
    for row in &table.rows {
        if row.len() != table.columns.len() {
            return Err(ImageError::Malformed(format!(
                "a row of {target} has {} cells for {} columns",
                row.len(),
                table.columns.len()
            )));
        }
        if let Some(&(i, declared)) = keep.iter().find(|&&(i, declared)| !fits(&row[i], declared)) {
            return Err(ImageError::Malformed(format!(
                "{target}.{} is {declared} and the snapshot puts {:?} into it",
                table.columns[i], row[i]
            )));
        }
        stmt.execute(rusqlite::params_from_iter(
            keep.iter().map(|&(i, _)| row[i].to_value()),
        ))?;
    }
    Ok(())
}

/// The Lamport bound, re-exported where the snapshot work first introduced it (review of PR #487,
/// Integrity #1). It now belongs to the op itself — see [`crate::model::MAX_LAMPORT`].
pub use crate::model::MAX_LAMPORT;

/// One op of an image's log, typed: every column an op has, of the type [`Op`](crate::model::Op)
/// reads it as.
pub(crate) struct LogRow {
    pub(crate) rowid: i64,
    pub(crate) op_id: String,
    pub(crate) lamport: i64,
    pub(crate) site: i64,
    pub(crate) domain: String,
    pub(crate) target_kind: String,
    pub(crate) target_id: String,
    pub(crate) field: String,
    pub(crate) op_type: String,
    pub(crate) value: Option<String>,
    pub(crate) author: String,
    pub(crate) wall_clock: String,
    pub(crate) key_id: Option<String>,
    pub(crate) sig: Option<String>,
}

impl LogRow {
    /// The op this row is.
    pub(crate) fn op(&self) -> crate::model::Op {
        crate::model::Op {
            op_id: self.op_id.clone(),
            lamport: self.lamport,
            site: self.site,
            domain: self.domain.clone(),
            target_kind: self.target_kind.clone(),
            target_id: self.target_id.clone(),
            field: self.field.clone(),
            op_type: self.op_type.clone(),
            value: self.value.clone(),
            author: self.author.clone(),
            wall_clock: self.wall_clock.clone(),
            key_id: self.key_id.clone(),
            sig: self.sig.clone(),
        }
    }
}

/// The image's log as typed rows, or the reason it is not one (review of PR #487, Integrity #1).
///
/// **A load must be total over its input.** An image is a file — handed over by an operator, read
/// from an app's bucket — and anything the log accepted here is read back later through code that
/// `.unwrap()`s the type an op has: the clock seed, every export, every refold. A lamport stored as
/// text or a NULL author would sail through the insert and panic after the commit, leaving a store
/// no open can read and no retry can replace. So every column an op needs must be present with its
/// type — text where `Op` has a `String` (`value` alone may be NULL), integers for the coordinate
/// and the rowid, the rowids strictly rising and the op ids unique.
///
/// A lamport past [`MAX_LAMPORT`] is NOT refused (6j6v.m19v). It used to be, because it would have
/// seeded the clock where the next `+ 1` overflows; the clock is now seeded within the bound
/// everywhere, and the wire path keeps such an op inert — so a store may hold one, its image has to
/// load, and the loading store holds it inert again. A column this engine does not know is left
/// behind, as sync leaves behind an envelope field it does not know.
///
/// The signature pair (`key_id`, `sig`) is optional — an image taken before ops were signed has
/// neither column — and `provenance` is never read at all (6j6v.pzkb): it is the SOURCE's verdict,
/// and a verdict is never taken from outside. The loading store verifies every op itself.
pub(crate) fn typed_log(ops: &Table) -> Result<Vec<LogRow>, ImageError> {
    let at = |name: &str| {
        ops.columns
            .iter()
            .position(|c| c == name)
            .ok_or_else(|| ImageError::Malformed(format!("the log carries no {name} column")))
    };
    let [rowid, op_id, lamport, site, domain, target_kind, target_id, field, op_type, value, author, wall_clock] =
        [
            "rowid",
            "op_id",
            "lamport",
            "site",
            "domain",
            "target_kind",
            "target_id",
            "field",
            "op_type",
            "value",
            "author",
            "wall_clock",
        ]
        .map(at);
    let (rowid, op_id, lamport, site, domain, target_kind) =
        (rowid?, op_id?, lamport?, site?, domain?, target_kind?);
    let (target_id, field, op_type, value, author, wall_clock) =
        (target_id?, field?, op_type?, value?, author?, wall_clock?);
    let optional = |name: &str| ops.columns.iter().position(|c| c == name);
    let (key_id, sig) = (optional("key_id"), optional("sig"));
    let mut seen = HashSet::new();
    let mut last_rowid = 0;
    let mut rows = Vec::with_capacity(ops.rows.len());
    for (n, row) in ops.rows.iter().enumerate() {
        if row.len() != ops.columns.len() {
            return Err(ImageError::Malformed(format!(
                "op #{n} has {} cells for {} columns",
                row.len(),
                ops.columns.len()
            )));
        }
        let bad = |column: &str, cell: &Cell| {
            ImageError::Malformed(format!("op #{n}: {column} cannot be {cell:?}"))
        };
        let text = |i: usize, column: &str| match &row[i] {
            Cell::Text(t) => Ok(t.clone()),
            cell => Err(bad(column, cell)),
        };
        let int = |i: usize, column: &str| match &row[i] {
            Cell::Integer(v) => Ok(*v),
            cell => Err(bad(column, cell)),
        };
        let nullable_text = |i: Option<usize>, column: &str| match i.map(|i| &row[i]) {
            None | Some(Cell::Null) => Ok(None),
            Some(Cell::Text(t)) => Ok(Some(t.clone())),
            Some(cell) => Err(bad(column, cell)),
        };
        let log_row = LogRow {
            rowid: int(rowid, "rowid")?,
            op_id: text(op_id, "op_id")?,
            lamport: int(lamport, "lamport")?,
            site: int(site, "site")?,
            domain: text(domain, "domain")?,
            target_kind: text(target_kind, "target_kind")?,
            target_id: text(target_id, "target_id")?,
            field: text(field, "field")?,
            op_type: text(op_type, "op_type")?,
            value: match &row[value] {
                Cell::Null => None,
                Cell::Text(t) => Some(t.clone()),
                cell => return Err(bad("value", cell)),
            },
            author: text(author, "author")?,
            wall_clock: text(wall_clock, "wall_clock")?,
            key_id: nullable_text(key_id, "key_id")?,
            sig: nullable_text(sig, "sig")?,
        };
        if log_row.rowid <= last_rowid {
            return Err(ImageError::Malformed(format!(
                "op #{n}: rowid {} does not rise above {last_rowid}",
                log_row.rowid
            )));
        }
        if !seen.insert(log_row.op_id.clone()) {
            return Err(ImageError::Malformed(format!(
                "op #{n}: op id {} appears twice",
                log_row.op_id
            )));
        }
        last_rowid = log_row.rowid;
        rows.push(log_row);
    }
    Ok(rows)
}

/// This store's verdict on every op of an image's log, in order (6j6v.pzkb) — each one verified
/// as any received op is, because an image is received: handed over by an operator, read from a
/// bucket.
///
/// **In parallel across the machine's cores.** A signature check costs tens of microseconds, and
/// a snapshot exists to make a new replica's start FAST (6j6v.mxt2 measured 0.2 s against 1.35 s
/// on a real board); checking a fully signed log of that size one op at a time would give most of
/// that back. The checks are independent, so the log is cut into one chunk per core.
pub(crate) fn verdicts(rows: &[LogRow]) -> Vec<crate::signing::Provenance> {
    use crate::signing::Provenance;
    let judge = |chunk: &[LogRow]| -> Vec<Provenance> {
        chunk
            .iter()
            .map(|r| Provenance::of_received(&r.op()))
            .collect()
    };
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    if cores < 2 || rows.len() < 512 {
        return judge(rows);
    }
    let size = rows.len().div_ceil(cores);
    std::thread::scope(|scope| {
        let parts: Vec<_> = rows
            .chunks(size)
            .map(|chunk| scope.spawn(move || judge(chunk)))
            .collect();
        parts
            .into_iter()
            .flat_map(|part| part.join().expect("a signature check does not panic"))
            .collect()
    })
}

/// Insert the typed log, rowids and all, through one fixed statement — each op with the verdict
/// [`verdicts`] gave it, in the same order.
pub(crate) fn insert_log(
    conn: &Connection,
    rows: &[LogRow],
    verdicts: &[crate::signing::Provenance],
) -> rusqlite::Result<()> {
    debug_assert_eq!(rows.len(), verdicts.len());
    let mut stmt = conn.prepare(
        "INSERT INTO ops (rowid, op_id, lamport, site, domain, target_kind, target_id, field,
                          op_type, value, author, wall_clock, key_id, sig, provenance)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
    )?;
    for (r, verdict) in rows.iter().zip(verdicts) {
        stmt.execute(rusqlite::params![
            r.rowid,
            r.op_id,
            r.lamport,
            r.site,
            r.domain,
            r.target_kind,
            r.target_id,
            r.field,
            r.op_type,
            r.value,
            r.author,
            r.wall_clock,
            r.key_id,
            r.sig,
            verdict.as_str()
        ])?;
    }
    Ok(())
}
