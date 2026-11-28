//! The marker that names a space's kind (RFC 0001, "The model", **One model, both kinds of
//! space.**; NEX-205): NoCentralDB spaces carry no type, so the store writes one `spaceKind`
//! object naming the kind in the change right after it creates a space. Every reader reads it,
//! a caller that opens the space by id alone through `Engine::open_space` included.
//!
//! A space created before the marker existed has none, and keeps the kind `nexus-flow.toml` gives
//! it. A space whose markers name more than one kind, which only a write outside the store can
//! make, is of no known kind.

use std::fmt;

use nocentraldb::{Db, Space, Value};

use crate::id::SpaceId;
use crate::opened::Opened;
use crate::space_kind::SpaceKind;

/// The marker's collection.
pub const COLLECTION: &str = "spaceKind";

/// The index that finds every marker of one space.
const BY_KIND: &str = "byKind";

/// What a space's markers say of its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Marked {
    /// The space holds no marker.
    None,
    /// Every marker names this kind.
    Kind(SpaceKind),
    /// The markers name more than one kind.
    Conflict,
}

/// The value of the marker's `kind` field for `kind`.
fn stored(kind: SpaceKind) -> &'static str {
    match kind {
        SpaceKind::Repository => "repository",
        SpaceKind::Coordination => "coordination",
        SpaceKind::AdHoc => "adHoc",
    }
}

/// Creates a space of kind `kind` in `db`, whose member becomes its first admin, and writes its
/// marker.
pub(crate) async fn create(db: &Db, kind: SpaceKind) -> Result<Space, nocentraldb::Error> {
    let space = db.spaces().create().await?;
    space
        .write(|tx| {
            tx.create(COLLECTION, [("kind", Value::from(stored(kind)))])
                .map(drop)
        })
        .await?;
    Ok(space)
}

/// What the markers of the open space `space` say of its kind; [`Marked::None`] when the
/// database holds none of the space.
pub(crate) async fn read(opened: &Opened, space: &SpaceId) -> Result<Marked, nocentraldb::Error> {
    let mut marked = Marked::None;
    for marker in opened.find(space, COLLECTION, BY_KIND, Vec::new()).await? {
        let kind = marker.get::<&str>("kind")?.and_then(|text| {
            SpaceKind::ALL
                .into_iter()
                .find(|&kind| stored(kind) == text)
        });
        marked = match (marked, kind) {
            (Marked::None, Some(kind)) => Marked::Kind(kind),
            (Marked::Kind(known), Some(kind)) if known == kind => marked,
            _ => Marked::Conflict,
        };
    }
    Ok(marked)
}

/// Why a write was refused: its collection is the marker, which the store writes only when it
/// creates a space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerWrite {
    /// The collection the write was for.
    pub collection: String,
}

impl fmt::Display for MarkerWrite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} names the space's kind and is written only when the space is created, so this write is refused",
            self.collection
        )
    }
}

impl std::error::Error for MarkerWrite {}
