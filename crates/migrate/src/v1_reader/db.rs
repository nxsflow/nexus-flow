//! `.nxs/db.sqlite`, opened so that nothing on disk changes.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use super::{Error, V1Op};

/// The database file in a v1 workspace directory.
pub(super) const FILE: &str = "db.sqlite";

/// Opens the database read-only and `immutable`: SQLite then takes no lock and creates no `-wal`,
/// `-shm` or `-journal` file beside it, so not one byte in the workspace changes.
pub(super) fn open(path: &Path) -> Result<Connection, Error> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Connection::open_with_flags(uri(path), flags).map_err(|source| Error::Sqlite {
        path: path.to_path_buf(),
        source,
    })
}

/// The database's `PRAGMA user_version`.
pub(super) fn schema(conn: &Connection) -> Result<i64, Error> {
    pragma(conn, "user_version")
}

/// Every op of the log in v1's order, `lamport` then `site`, with ties broken by `op_id`, compared
/// bytewise (SQLite's `BINARY` collation).
pub(super) fn ops(conn: &Connection) -> Result<Vec<V1Op>, Error> {
    let mut stmt = conn
        .prepare(
            "SELECT op_id, lamport, site, domain, target_kind, target_id, field, op_type, value,
                    author, wall_clock, key_id, sig, provenance
             FROM ops ORDER BY lamport, site, op_id",
        )
        .map_err(|source| sqlite(conn, source))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(V1Op {
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
                provenance: r.get(13)?,
            })
        })
        .map_err(|source| sqlite(conn, source))?;
    rows.collect::<Result<_, _>>()
        .map_err(|source| sqlite(conn, source))
}

fn pragma(conn: &Connection, name: &str) -> Result<i64, Error> {
    conn.pragma_query_value(None, name, |r| r.get(0))
        .map_err(|source| sqlite(conn, source))
}

fn sqlite(conn: &Connection, source: rusqlite::Error) -> Error {
    Error::Sqlite {
        path: conn.path().unwrap_or_default().into(),
        source,
    }
}

/// A `file:` URI for `path` with `immutable=1`. Every byte outside the unreserved set and `/` is
/// percent-encoded, so a path holding `?`, `#` or `%` still names the file.
fn uri(path: &Path) -> String {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = String::from("file:");
    let text = absolute.to_string_lossy().replace('\\', "/");
    if !text.starts_with('/') {
        out.push('/');
    }
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out.push_str("?immutable=1");
    out
}
