//! The shipped defaults and their states (RFC 0006, "Installing the shipped defaults").
//!
//! `shipped.toml` lists, for each path nexus-flow ships, the SHA-256 of every version it has ever
//! shipped, oldest first; the last one is the current default. A path nexus-flow no longer ships
//! stays in the list with `retired = true`. Paths are relative to the repository's `factory/`:
//!
//! ```toml
//! [[file]]
//! path = "roles/coder.md"
//! sha256 = ["<oldest>", "<current>"]
//!
//! [[file]]
//! path = "roles/old.md"
//! sha256 = ["<last shipped>"]
//! retired = true
//! ```
//!
//! A hash may appear more than once for a path: a release that ships an earlier text again lists
//! its hash again, last, as the current default.
//!
//! A hash is taken of the text after CRLF is turned into LF, on the shipped side as on the
//! repository's, so a default checked out on Windows reads as unchanged.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// What `install` finds at a shipped path, and what it does there (RFC 0006's table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The file is missing: the current default is written.
    Written,
    /// The file is the current default: nothing is done.
    Current,
    /// The file is an earlier default: nothing is done without `--update`.
    OlderDefault,
    /// The file was an earlier default and `--update` replaces it with the current one.
    Updated,
    /// The team changed the file: it is never touched.
    Team,
    /// The file is present and its path is retired: it is never touched.
    Retired,
}

impl State {
    /// The state's name in `install`'s report.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Written => "written",
            State::Current => "current",
            State::OlderDefault => "older_default",
            State::Updated => "updated",
            State::Team => "team",
            State::Retired => "retired",
        }
    }
}

/// The lowercase hex SHA-256 of `bytes` after every CRLF in them is turned into LF.
pub fn sha256_lf(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    let mut rest = bytes;
    while let Some(at) = rest.windows(2).position(|w| w == b"\r\n") {
        hasher.update(&rest[..at]);
        hasher.update(b"\n");
        rest = &rest[at + 2..];
    }
    hasher.update(rest);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// One path of `shipped.toml`: its hashes, oldest first, and whether it is retired.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    sha256: Vec<String>,
    retired: bool,
}

/// A readable `shipped.toml`, its paths in bytewise order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shipped {
    files: BTreeMap<String, Entry>,
}

/// Why `shipped.toml` cannot be read. It is nexus-flow's own source, so each one is a bug in a
/// release, which the tests that read the embedded list catch before it ships.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShippedError {
    /// The text is not TOML, or holds a key or shape this format does not describe.
    Toml(String),
    /// The path is empty, absolute, or has an empty, `.` or `..` segment or a backslash.
    PathInvalid(String),
    /// The path is listed twice.
    PathDuplicate(String),
    /// The path lists no hash.
    NoHash(String),
    /// A hash of the path is not 64 lowercase hex digits.
    HashInvalid(String),
}

impl fmt::Display for ShippedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShippedError::Toml(e) => write!(f, "shipped.toml is not readable: {e}"),
            ShippedError::PathInvalid(p) => {
                write!(f, "shipped.toml: {p:?} is not a plain relative path")
            }
            ShippedError::PathDuplicate(p) => write!(f, "shipped.toml lists {p} twice"),
            ShippedError::NoHash(p) => write!(f, "shipped.toml lists no hash for {p}"),
            ShippedError::HashInvalid(p) => {
                write!(
                    f,
                    "shipped.toml: a hash of {p} is not 64 lowercase hex digits"
                )
            }
        }
    }
}

impl std::error::Error for ShippedError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawList {
    #[serde(default)]
    file: Vec<RawFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    path: String,
    sha256: Vec<String>,
    #[serde(default)]
    retired: bool,
}

fn plain_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && path.split('/').all(|s| !matches!(s, "" | "." | ".."))
}

fn hex_sha256(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl Shipped {
    /// Reads the text of a `shipped.toml`.
    pub fn parse(text: &str) -> Result<Shipped, ShippedError> {
        let raw: RawList = toml::from_str(text).map_err(|e| ShippedError::Toml(e.to_string()))?;
        let mut files = BTreeMap::new();
        for file in raw.file {
            if !plain_relative(&file.path) {
                return Err(ShippedError::PathInvalid(file.path));
            }
            if file.sha256.is_empty() {
                return Err(ShippedError::NoHash(file.path));
            }
            if !file.sha256.iter().all(|h| hex_sha256(h)) {
                return Err(ShippedError::HashInvalid(file.path));
            }
            if files.contains_key(&file.path) {
                return Err(ShippedError::PathDuplicate(file.path));
            }
            let entry = Entry {
                sha256: file.sha256,
                retired: file.retired,
            };
            files.insert(file.path, entry);
        }
        Ok(Shipped { files })
    }

    /// Every path the list names, retired ones included, in bytewise order.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// Whether the list names `path` as retired.
    pub fn is_retired(&self, path: &str) -> bool {
        self.files.get(path).is_some_and(|e| e.retired)
    }

    /// The hash of the current default at `path`: the last one listed.
    pub fn current(&self, path: &str) -> Option<&str> {
        self.files.get(path)?.sha256.last().map(String::as_str)
    }

    /// The state of the repository's file at `path`, given its bytes (`None` when missing) and
    /// whether `--update` was asked for. `None` when the list does not name `path`, or when the
    /// path is retired and the file is missing: there is nothing to write or report there.
    pub fn state(&self, path: &str, file: Option<&[u8]>, update: bool) -> Option<State> {
        let entry = self.files.get(path)?;
        let state = match file {
            None if entry.retired => return None,
            _ if entry.retired => State::Retired,
            None => State::Written,
            Some(bytes) => {
                let hash = sha256_lf(bytes);
                let (current, earlier) = entry.sha256.split_last()?;
                if hash == *current {
                    State::Current
                } else if earlier.contains(&hash) {
                    if update {
                        State::Updated
                    } else {
                        State::OlderDefault
                    }
                } else {
                    State::Team
                }
            }
        };
        Some(state)
    }
}
