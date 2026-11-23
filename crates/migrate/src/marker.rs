//! `.nxs/migrated.toml` and the raised header of `db.sqlite`, which together mark a v1 workspace as
//! moved (RFC 0008, "The marker").
//!
//! The move writes the marker before it replays anything, and raises `PRAGMA user_version` and the
//! compatibility floor in `PRAGMA application_id` to 10, so that every v1 binary refuses the
//! workspace on open. The two fields are written in place, four bytes each, and not through SQLite:
//! a write through SQLite also changes the file change counter and the version-valid-for number in
//! the header, and the move changes nothing of v1's data but the two fields. `undo` writes the old
//! values from the marker back and removes it.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::v1_reader::{self, Refusal};

/// The marker's file name in a v1 workspace directory.
pub const FILE: &str = "migrated.toml";

/// The schema and the floor a moved workspace's header is raised to: above every v1 binary's
/// schema, so every one of them refuses it.
pub const RAISED: i64 = 10;

/// The marker's format, its `format` key.
const FORMAT: i64 = 1;

/// The line the marker opens with.
const COMMENT: &str =
    "# .nxs/migrated.toml, written by nxs migrate. Remove only with nxs migrate --undo.\n";

/// The magic string every SQLite database file opens with.
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Where SQLite keeps `PRAGMA user_version` and `PRAGMA application_id` in the database header,
/// each a 4-byte big-endian integer.
const USER_VERSION_AT: u64 = 60;
const APPLICATION_ID_AT: u64 = 68;

/// What the run gives the marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// The NoCentralDB space the workspace moves into.
    pub space: String,
    /// The object id of the run's `migrationRun`.
    pub run: String,
    /// When the run marked the workspace, in RFC 3339.
    pub migrated_at: String,
    /// The binary that marked it: `nxs <version>`.
    pub by: String,
}

/// The two fields of the database header the move raises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    /// `PRAGMA user_version`, v1's schema version.
    pub user_version: i64,
    /// `PRAGMA application_id`, v1's compatibility floor.
    pub application_id: i64,
}

/// A marker, as `.nxs/migrated.toml` holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub space: String,
    pub run: String,
    pub migrated_at: String,
    pub by: String,
    /// The header's values before the move raised them.
    pub header: Header,
}

/// The file's layout, in the order of its keys.
#[derive(Serialize, Deserialize)]
struct Stored {
    format: i64,
    space: String,
    run: String,
    migrated_at: String,
    by: String,
    header: Header,
}

/// Why a workspace could not be marked, read or unmarked.
#[derive(Debug)]
pub enum Error {
    /// The reader refuses the workspace, or could not read it.
    Reader(v1_reader::Error),
    /// The workspace already carries a marker.
    Marked,
    /// `undo` found no marker.
    NotMarked,
    /// `undo` found the header at other values than the move raised it to, so it was changed
    /// since, and the marker's values may no longer be the ones to restore.
    HeaderChanged { found: Header },
    /// The marker is of a format this binary does not know.
    UnknownFormat { format: i64 },
    /// `db.sqlite` is not a SQLite database.
    NotADatabase { path: PathBuf },
    /// The marker does not parse, or holds a header value no SQLite header can.
    Toml { path: PathBuf },
    /// A file could not be read or written.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl Error {
    /// The refusal, when the reader refused the workspace.
    pub fn refusal(&self) -> Option<&Refusal> {
        match self {
            Error::Reader(err) => err.refusal(),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Reader(err) => err.fmt(f),
            Error::Marked => write!(f, "the workspace is already marked as moved"),
            Error::NotMarked => write!(f, "the workspace is not marked as moved"),
            Error::HeaderChanged { found } => write!(
                f,
                "the database header is at user_version {} and application_id {}, not at the {} \
                 the move raised it to; it was changed since, so it is not restored",
                found.user_version, found.application_id, RAISED
            ),
            Error::UnknownFormat { format } => {
                write!(
                    f,
                    "the marker is of format {format}, which this nxs does not know"
                )
            }
            Error::NotADatabase { path } => {
                write!(f, "{} is not a SQLite database", path.display())
            }
            Error::Toml { path } => write!(f, "{} is not a valid marker", path.display()),
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for Error {}

impl From<v1_reader::Error> for Error {
    fn from(err: v1_reader::Error) -> Self {
        Error::Reader(err)
    }
}

/// Reads the marker in the workspace directory `dir`, or `None` when it carries none.
pub fn read(dir: &Path) -> Result<Option<Marker>, Error> {
    let path = dir.join(FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(Error::Io { path, source }),
    };
    let format = toml::from_str::<toml::Table>(&text)
        .map_err(|_| Error::Toml { path: path.clone() })?
        .get("format")
        .and_then(toml::Value::as_integer);
    match format {
        Some(FORMAT) => {}
        Some(format) => return Err(Error::UnknownFormat { format }),
        None => return Err(Error::Toml { path }),
    }
    let stored: Stored = toml::from_str(&text).map_err(|_| Error::Toml { path })?;
    Ok(Some(Marker {
        space: stored.space,
        run: stored.run,
        migrated_at: stored.migrated_at,
        by: stored.by,
        header: stored.header,
    }))
}

/// Writes the marker into `dir` and raises the header of its database, the marker first, so that
/// the old values are on disk before the header changes. The caller has checked that no v1
/// process holds the database.
pub(crate) fn write(dir: &Path, mark: &Mark) -> Result<Marker, Error> {
    let db = dir.join(v1_reader::DB_FILE);
    let header = read_header(&db)?;
    let marker = Marker {
        space: mark.space.clone(),
        run: mark.run.clone(),
        migrated_at: mark.migrated_at.clone(),
        by: mark.by.clone(),
        header,
    };
    let path = dir.join(FILE);
    write_marker(&path, &marker)?;
    let raised = Header {
        user_version: RAISED,
        application_id: RAISED,
    };
    if let Err(err) = write_header(&db, raised) {
        let _ = std::fs::remove_file(&path);
        return Err(err);
    }
    Ok(marker)
}

/// Restores the header of the database in `dir` from its marker, then removes the marker, and
/// returns the restored values. Refuses while a v1 process holds the database, and when the header
/// is no longer at the values the move raised it to.
pub fn undo(dir: &Path) -> Result<Header, Error> {
    let marker = read(dir)?.ok_or(Error::NotMarked)?;
    let db = dir.join(v1_reader::DB_FILE);
    v1_reader::refuse_in_use(&db)?;
    let found = read_header(&db)?;
    if found.user_version != RAISED || found.application_id != RAISED {
        return Err(Error::HeaderChanged { found });
    }
    write_header(&db, marker.header)?;
    let path = dir.join(FILE);
    std::fs::remove_file(&path).map_err(|source| Error::Io { path, source })?;
    Ok(marker.header)
}

fn write_marker(path: &Path, marker: &Marker) -> Result<(), Error> {
    let stored = Stored {
        format: FORMAT,
        space: marker.space.clone(),
        run: marker.run.clone(),
        migrated_at: marker.migrated_at.clone(),
        by: marker.by.clone(),
        header: marker.header,
    };
    let body = toml::to_string(&stored).expect("a marker always serialises");
    let io = |source| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                Error::Marked
            } else {
                io(e)
            }
        })?;
    file.write_all(COMMENT.as_bytes()).map_err(io)?;
    file.write_all(body.as_bytes()).map_err(io)?;
    file.sync_all().map_err(io)
}

fn read_header(db: &Path) -> Result<Header, Error> {
    let io = |source| Error::Io {
        path: db.to_path_buf(),
        source,
    };
    let mut bytes = [0u8; 72];
    File::open(db)
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                Error::NotADatabase {
                    path: db.to_path_buf(),
                }
            } else {
                io(e)
            }
        })?;
    if &bytes[..16] != SQLITE_MAGIC {
        return Err(Error::NotADatabase {
            path: db.to_path_buf(),
        });
    }
    let field = |at: u64| {
        let at = at as usize;
        i64::from(i32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()))
    };
    Ok(Header {
        user_version: field(USER_VERSION_AT),
        application_id: field(APPLICATION_ID_AT),
    })
}

fn write_header(db: &Path, header: Header) -> Result<(), Error> {
    let field = |value: i64| {
        i32::try_from(value)
            .map(i32::to_be_bytes)
            .map_err(|_| Error::Toml {
                path: db.with_file_name(FILE),
            })
    };
    let user_version = field(header.user_version)?;
    let application_id = field(header.application_id)?;
    let io = |source| Error::Io {
        path: db.to_path_buf(),
        source,
    };
    let mut file = OpenOptions::new().write(true).open(db).map_err(io)?;
    for (at, bytes) in [
        (USER_VERSION_AT, user_version),
        (APPLICATION_ID_AT, application_id),
    ] {
        file.seek(SeekFrom::Start(at)).map_err(io)?;
        file.write_all(&bytes).map_err(io)?;
    }
    file.sync_all().map_err(io)
}
