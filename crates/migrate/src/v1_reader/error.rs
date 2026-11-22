//! Why a v1 workspace could not be read.

use std::fmt;
use std::path::PathBuf;

/// Why a v1 workspace could not be read.
#[derive(Debug)]
pub enum Error {
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
            Error::Io { path, source } => write!(f, "reading {}: {source}", path.display()),
            Error::Toml { path } => write!(f, "{} is not valid TOML", path.display()),
            Error::Sqlite { path, source } => write!(f, "reading {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for Error {}
