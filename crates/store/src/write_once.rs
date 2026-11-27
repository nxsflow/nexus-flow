//! Write-once collections (RFC 0003, "Append-only"): an object of such a collection takes its
//! fields in the change that creates it, and no change after it. The store refuses, for every
//! member and every caller, a field set, a delete or a restore of an existing object of a
//! collection [`WRITE_ONCE`] names; a record is corrected by a new one, never rewritten.
//!
//! The guard is generic: each module that keeps a write-once record declares its collections in
//! a table of its own, and [`WRITE_ONCE`] lists those tables. The decisions module declares its
//! four collections (`crate::decisions::guard`), the move its records (`crate::migration`), the
//! status module its three (`crate::status`). Once NoCentralDB declares write-once collections in the model (R12),
//! every member parks such a change, and this guard stays as the check it already is.

use std::fmt;

/// Every table of write-once collections, one per module that keeps write-once records.
pub const WRITE_ONCE: &[&[&str]] = &[
    crate::decisions::guard::WRITE_ONCE,
    crate::migration::WRITE_ONCE,
    crate::status::WRITE_ONCE,
];

/// A change to an existing object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Change {
    /// A write of one or more of its fields.
    Set,
    /// Its deletion.
    Delete,
    /// The restore of it after a deletion.
    Restore,
}

impl Change {
    fn name(self) -> &'static str {
        match self {
            Change::Set => "a field set",
            Change::Delete => "a delete",
            Change::Restore => "a restore",
        }
    }
}

/// Why a change to an existing object was refused: its collection is write-once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOnce {
    /// The collection of the object.
    pub collection: String,
    /// The change that was refused.
    pub change: Change,
}

impl fmt::Display for WriteOnce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is write-once: an existing object takes no change, so {} is refused; record a new object instead",
            self.collection,
            self.change.name()
        )
    }
}

impl std::error::Error for WriteOnce {}

/// Whether an existing object of `collection` may take `change`, by [`WRITE_ONCE`].
pub fn check(collection: &str, change: Change) -> Result<(), WriteOnce> {
    check_in(WRITE_ONCE, collection, change)
}

/// Whether an existing object of `collection` may take `change`, by `tables`.
pub fn check_in(tables: &[&[&str]], collection: &str, change: Change) -> Result<(), WriteOnce> {
    if tables.iter().any(|table| table.contains(&collection)) {
        Err(WriteOnce {
            collection: collection.to_owned(),
            change,
        })
    } else {
        Ok(())
    }
}
