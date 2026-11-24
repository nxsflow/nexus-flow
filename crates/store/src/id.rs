//! The ids the seam takes and returns (RFC 0001, "Public surfaces this RFC brings", library
//! seam), each a wrapper over NoCentralDB's own, so that no crate above the store names the SDK.

use std::fmt;
use std::str::FromStr;

/// A NoCentralDB space's id: 16 bytes, shown and read as 32 lowercase hex digits, as
/// `nexus-flow.toml` holds it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpaceId(pub(crate) nocentraldb::SpaceId);

impl fmt::Display for SpaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Debug for SpaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SpaceId({self})")
    }
}

impl FromStr for SpaceId {
    type Err = NotASpaceId;

    fn from_str(text: &str) -> Result<Self, NotASpaceId> {
        text.parse().map(SpaceId).map_err(|_| NotASpaceId {
            text: text.to_owned(),
        })
    }
}

/// A text that is not a space id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotASpaceId {
    pub text: String,
}

impl fmt::Display for NotASpaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} is not a space id: one is 32 lowercase hex digits",
            self.text
        )
    }
}

impl std::error::Error for NotASpaceId {}

/// A running session's id: the name of its folder under `sessions/` in the data directory
/// (RFC 0001, "The local data on disk"), as `NXS_SESSION` names it.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(String);

impl SessionId {
    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionId({})", self.0)
    }
}

impl FromStr for SessionId {
    type Err = NotASessionId;

    fn from_str(text: &str) -> Result<Self, NotASessionId> {
        if crate::data_dir::is_plain_name(text) {
            Ok(SessionId(text.to_owned()))
        } else {
            Err(NotASessionId {
                text: text.to_owned(),
            })
        }
    }
}

/// A text that is not a session id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotASessionId {
    pub text: String,
}

impl fmt::Display for NotASessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} is not a session id: one is a plain folder name, without a path separator",
            self.text
        )
    }
}

impl std::error::Error for NotASessionId {}
