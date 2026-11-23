//! The member a command runs as (RFC 0001, "Reading and writing"): the session's member when
//! `NXS_SESSION` names a running session, otherwise the device's member.
//!
//! A session's folder, `sessions/<session id>/` in the data directory, holds its member's
//! database in `db/` and that member's keys in the file `member.keys`; a session agent has no key
//! store of its own. A session runs while both exist: `nxs session start` creates them and
//! `nxs session end` removes them. Every session's member is opened as a sub-agent.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use crate::data_dir::DataDir;
use crate::id::{NotASessionId, SessionId};
use crate::open::DeviceKeys;

/// The variable that names the session a command runs as.
pub const NXS_SESSION: &str = "NXS_SESSION";

/// The member whose database a command opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunAs {
    /// The device's member, with its keys where they live.
    Device(DeviceKeys),
    /// A running session's member.
    Session(SessionId),
}

impl RunAs {
    /// The session `NXS_SESSION` names, reading variables through `var`; `None` when it is unset
    /// or empty, so the command runs as the device's member. Touches no file.
    pub fn session_from(
        var: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Option<SessionId>, SessionVarError> {
        let Some(value) = var(NXS_SESSION).filter(|v| !v.is_empty()) else {
            return Ok(None);
        };
        let text = value.to_string_lossy();
        text.parse()
            .map(Some)
            .map_err(|error| SessionVarError { error })
    }

    /// The session `NXS_SESSION` names in the process's environment. Touches no file.
    pub fn session_from_env() -> Result<Option<SessionId>, SessionVarError> {
        Self::session_from(|name| std::env::var_os(name))
    }
}

/// `NXS_SESSION` holds no session id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionVarError {
    pub error: NotASessionId,
}

impl fmt::Display for SessionVarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{NXS_SESSION}: {}", self.error)
    }
}

impl std::error::Error for SessionVarError {}

/// Where a session's member keeps its database and keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFiles {
    /// The session's folder.
    pub folder: PathBuf,
    /// The member's NoCentralDB database.
    pub database: PathBuf,
    /// The member's key file.
    pub keys: PathBuf,
}

impl SessionFiles {
    /// The files of the session `id` in `data`. Touches no file.
    pub fn of(data: &DataDir, id: &SessionId) -> Self {
        let folder = data
            .session(id.as_str())
            .expect("a session id is a plain folder name");
        SessionFiles {
            database: folder.join("db"),
            keys: folder.join("member.keys"),
            folder,
        }
    }

    /// Whether the session is running: its database and keys both exist.
    pub fn running(&self) -> bool {
        self.database.is_dir() && self.keys.is_file()
    }
}
