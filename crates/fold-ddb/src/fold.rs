//! [`Folder`]: the server's fold run over one stream (6j6v.vvw6) — the reducers a local replica
//! registers, the same dispatch rule ([`nxs_foundation::reducer::folder_for`]), and every change
//! they describe carried out as a conditional write.
//!
//! # The watermark
//!
//! `.meta#watermark` holds `folded_through` — the relay position the fold has reached, the one
//! number E4 decided a server needs (no version vector, owner 2026-10-06) — and the fold revision of
//! every reducer. A batch folds its ops first and moves the watermark after, and only forward: a
//! run that dies in between folds part of the batch again next time, which changes nothing, because
//! every change is idempotent. The relay may also hand the same op twice (the DynamoDB op store
//! does not deduplicate); folding it twice is just as harmless.
//!
//! When the recorded revisions differ from the reducers' — a release changed what the same ops fold
//! to — [`Folder::is_current`] says so, and the run starts over: [`Folder::clear`] the stream's
//! rows, then fold from relay position 0.
//!
//! # What the server cannot see
//!
//! A local replica refuses an op whose `(lamport, site)` another op already holds ("collided") and
//! folds only the first. The op table has no index on the coordinate, so the server folds both.
//! Registers still keep the first arrival — an equal version beats nothing — and only an add or a
//! note of the second op is extra. Such ops predate fc5p, which made a replica mint past a taken
//! coordinate.

use crate::board;
use crate::layout::{Layout, LayoutError, WATERMARK_KEY};
use crate::table::{Outcome, Table};
use crate::write::{plan, Cond, Write};
use nxs_foundation::change::{Cell, Change, Wins};
use nxs_foundation::model::Op;
use nxs_foundation::reducer::{folder_for, Reducer};
use std::collections::BTreeMap;

/// The platform's reducers — the board's, the chat's and the memory's.
pub fn platform_reducers() -> Vec<Box<dyn Reducer>> {
    vec![
        Box::new(nexus_flow_core::task_reducer::TaskReducer),
        Box::new(nexus_chat::message_reducer::MessageReducer),
        Box::new(nexus_memory::fact_reducer::FactReducer),
    ]
}

/// Every view table of the platform's reducers.
pub fn platform_view_tables() -> Vec<&'static str> {
    platform_reducers()
        .iter()
        .flat_map(|r| r.view_tables().iter().copied())
        .collect()
}

/// How far a stream is folded, and by which rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Watermark {
    /// The relay position of the last op folded.
    pub folded_through: i64,
    /// Each reducer's fold revision, by domain.
    pub revisions: BTreeMap<String, i64>,
}

/// Why a fold stopped. Nothing after the failing op was written.
#[derive(Debug)]
pub enum FoldError<E> {
    /// The table refused a request.
    Table(E),
    /// A reducer described a change the layout has no place for — a build whose reducers and
    /// layout disagree.
    Layout(LayoutError),
}

impl<E: std::fmt::Display> std::fmt::Display for FoldError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FoldError::Table(e) => write!(f, "the views table refused a request: {e}"),
            FoldError::Layout(e) => write!(f, "{e}"),
        }
    }
}

impl<E: std::error::Error> std::error::Error for FoldError<E> {}

/// The reducers and the layout their changes are written in.
pub struct Folder {
    reducers: Vec<Box<dyn Reducer>>,
    layout: Layout,
}

impl Folder {
    pub fn new(reducers: Vec<Box<dyn Reducer>>, layout: Layout) -> Folder {
        Folder { reducers, layout }
    }

    /// The platform's reducers over the platform's layout.
    pub fn platform() -> Result<Folder, LayoutError> {
        Ok(Folder::new(platform_reducers(), Layout::platform()?))
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Each reducer's fold revision, by domain.
    pub fn revisions(&self) -> BTreeMap<String, i64> {
        self.reducers
            .iter()
            .map(|r| (r.domain().to_string(), r.fold_revision()))
            .collect()
    }

    /// What folding `op` changes — nothing for an op no reducer folds, which is stored on the relay
    /// and folded once a later release understands it.
    pub fn changes(&self, op: &Op) -> Vec<Change> {
        folder_for(self.reducers.iter().map(Box::as_ref), op)
            .map(|r| r.changes(op))
            .unwrap_or_default()
    }

    /// Fold one op: carry out its changes, then keep the board's indexes. Returns whether a reducer
    /// folded it.
    pub async fn fold<T: Table>(&self, table: &T, op: &Op) -> Result<bool, FoldError<T::Error>> {
        let changes = self.changes(op);
        if changes.is_empty() {
            return Ok(false);
        }
        let writes = changes
            .iter()
            .map(|c| plan(&self.layout, c))
            .collect::<Result<Vec<_>, _>>()
            .map_err(FoldError::Layout)?;
        for write in &writes {
            table.write(write).await.map_err(FoldError::Table)?;
        }
        board::after(table, &changes)
            .await
            .map_err(FoldError::Table)?;
        Ok(true)
    }

    /// Fold a batch of ops pulled from the relay — each with its relay position — and move the
    /// watermark to the highest position among them.
    pub async fn fold_batch<T: Table>(
        &self,
        table: &T,
        ops: &[(i64, Op)],
    ) -> Result<(), FoldError<T::Error>> {
        for (_, op) in ops {
            self.fold(table, op).await?;
        }
        if let Some(through) = ops.iter().map(|(seq, _)| *seq).max() {
            self.advance(table, through).await?;
        }
        Ok(())
    }

    /// Move the watermark to `through` — never backwards — with this folder's revisions.
    pub async fn advance<T: Table>(
        &self,
        table: &T,
        through: i64,
    ) -> Result<Outcome, FoldError<T::Error>> {
        let mut set = vec![("folded_through".to_string(), Cell::Int(through))];
        set.extend(
            self.revisions()
                .into_iter()
                .map(|(domain, rev)| (format!("revision_{domain}"), Cell::Int(rev))),
        );
        table
            .write(&Write {
                sk: WATERMARK_KEY.to_string(),
                set,
                remove: Vec::new(),
                condition: Some(Cond::Any(vec![
                    Cond::RowAbsent,
                    Cond::Beats {
                        columns: vec!["folded_through".to_string()],
                        theirs: vec![through],
                        wins: Wins::Higher,
                    },
                ])),
            })
            .await
            .map_err(FoldError::Table)
    }

    /// The stream's watermark, `None` before its first batch.
    pub async fn watermark<T: Table>(table: &T) -> Result<Option<Watermark>, T::Error> {
        Ok(table.get(WATERMARK_KEY).await?.map(|row| {
            let mut mark = Watermark::default();
            for (attr, cell) in row {
                match (attr.as_str(), cell) {
                    ("folded_through", Cell::Int(v)) => mark.folded_through = v,
                    (a, Cell::Int(v)) if a.starts_with("revision_") => {
                        mark.revisions.insert(a["revision_".len()..].to_string(), v);
                    }
                    _ => {}
                }
            }
            mark
        }))
    }

    /// Whether `mark` was folded by this folder's rules. A stream that is not must be cleared and
    /// folded again from position 0.
    pub fn is_current(&self, mark: &Watermark) -> bool {
        mark.revisions == self.revisions()
    }

    /// Delete every row of the stream — the views, the adjacency entries and the watermark.
    pub async fn clear<T: Table>(table: &T) -> Result<(), T::Error> {
        for row in table.query_prefix("").await? {
            if let Some(Cell::Text(sk)) = row.get(crate::layout::SK) {
                table.delete(sk).await?;
            }
        }
        Ok(())
    }
}
