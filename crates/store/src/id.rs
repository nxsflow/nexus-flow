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
