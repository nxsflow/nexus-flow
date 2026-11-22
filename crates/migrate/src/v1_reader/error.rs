//! Why a v1 workspace could not be read.

use std::fmt;
use std::path::PathBuf;

/// A workspace the move refuses (RFC 0008, "Workspaces the move refuses" and "The command"). A
/// refusal leaves the workspace as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// There is no v1 workspace: no such directory, or no `db.sqlite` in it.
    NothingToMigrate,
    /// A v1 process holds the database open, or the last one did not close it.
    V1InUse,
    /// The workspace's flow plugin is not `issue-tracker`.
    UnsupportedPlugin { plugin: String },
    /// The database is at a schema below 9.
    V1FormatOld { schema: i64 },
    /// The database is at a schema above 9.
    V1FormatUnknown { schema: i64 },
    /// The store started from a snapshot (`log_base`), so its log is not complete.
    V1BasedStore,
}

impl Refusal {
    /// The error code RFC 0008 gives the refusal.
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::NothingToMigrate => "nothing_to_migrate",
            Refusal::V1InUse => "v1_in_use",
            Refusal::UnsupportedPlugin { .. } => "unsupported_plugin",
            Refusal::V1FormatOld { .. } => "v1_format_old",
            Refusal::V1FormatUnknown { .. } => "v1_format_unknown",
            Refusal::V1BasedStore => "v1_based_store",
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.code();
        match self {
            Refusal::NothingToMigrate => write!(f, "{code}: there is no v1 workspace to move"),
            Refusal::V1InUse => write!(
                f,
                "{code}: a v1 process has the database open, or the last one did not close it; \
                 stop it, or open and close the workspace once with v1"
            ),
            Refusal::UnsupportedPlugin { plugin } => write!(
                f,
                "{code}: the workspace uses the plugin `{plugin}`; only `issue-tracker` moves, \
                 the workspace stays on v1"
            ),
            Refusal::V1FormatOld { schema } => write!(
                f,
                "{code}: the database is at schema {schema}; open it once with the last v1 \
                 release, which brings it to 9"
            ),
            Refusal::V1FormatUnknown { schema } => write!(
                f,
                "{code}: the database is at schema {schema}, which the move cannot read; it \
                 reads schema 9"
            ),
            Refusal::V1BasedStore => write!(
                f,
                "{code}: the store started from a snapshot and holds no history before it; \
                 move a replica that holds the full log"
            ),
        }
    }
}

/// Why a v1 workspace could not be read.
#[derive(Debug)]
pub enum Error {
    /// The move refuses the workspace.
    Refused(Refusal),
    /// A file of the workspace could not be read.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A TOML file of the workspace does not parse. The parser's message is not kept: it quotes
    /// the line it failed on, and `sync.toml`'s endpoint line may carry a password.
    Toml { path: PathBuf },
    /// The database could not be opened or read.
    Sqlite {
        path: PathBuf,
        source: rusqlite::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Refused(refusal) => refusal.fmt(f),
            Error::Io { path, source } => write!(f, "reading {}: {source}", path.display()),
            Error::Toml { path } => write!(f, "{} is not valid TOML", path.display()),
            Error::Sqlite { path, source } => write!(f, "reading {}: {source}", path.display()),
        }
    }
}

impl Error {
    /// The refusal, when the workspace was refused rather than unreadable.
    pub fn refusal(&self) -> Option<&Refusal> {
        match self {
            Error::Refused(refusal) => Some(refusal),
            _ => None,
        }
    }
}

impl std::error::Error for Error {}
