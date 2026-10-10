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
//! The rules this crate adds beside the reducers' rows — the board's index flags and the entries it
//! keeps under a ticket — have a revision of their own, [`INDEX_REVISION`], recorded in the same
//! watermark under [`INDEX_DOMAIN`]. A release that changes what the fold writes beside the rows
//! bumps it, and every stream folded before is no longer current and is folded again the same way.
//! Revision 1 (6j6v.t1ym) adds the `.ladj#`/`.tadj#` entries; a stream folded before it has none,
//! so its watermark names no index revision at all and the next run refolds it. Revision 2
//! (6j6v.15ed) adds the `.cadj#` entries, a ticket's outgoing `contributes_to` edges.
//!
//! # What the server cannot see
//!
//! A local replica refuses an op whose `(lamport, site)` another op already holds ("collided") and
//! folds only the first. The op table has no index on the coordinate, so the server folds both.
//! Registers still keep the first arrival — an equal version beats nothing — and only an add or a
//! note of the second op is extra. Such ops predate fc5p, which made a replica mint past a taken
//! coordinate.

use crate::board;
use crate::layout::{Layout, LayoutError, MAX_ITEM_BYTES, MAX_KEY_BYTES, WATERMARK_KEY};
use crate::table::{Outcome, Table};
use crate::write::{plan, Cond, Write};
use nxs_foundation::change::{Cell, Change, Wins};
use nxs_foundation::model::Op;
use nxs_foundation::reducer::{folder_for, Reducer};
use std::collections::BTreeMap;

/// The key, among a watermark's revisions, of the rules this crate keeps beside the reducers' rows
/// (see the module doc). No reducer has this domain.
pub const INDEX_DOMAIN: &str = "index";

/// The revision of those rules. 1: the entries under a ticket for its label and thread link adds
/// (6j6v.t1ym). 2: the entries under a ticket for its outgoing `contributes_to` edges (6j6v.15ed).
pub const INDEX_REVISION: i64 = 2;

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

/// What [`Folder::fold`] did with an op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Folded {
    Folded,
    /// No reducer folds it — a domain or shape this build does not know.
    NotFolded,
    /// The table cannot hold it, wholly or in part.
    Refused(Refusal),
}

/// An op the table could not hold, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub op_id: String,
    pub reason: String,
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

    /// Each reducer's fold revision, by domain, and this crate's own [`INDEX_REVISION`] under
    /// [`INDEX_DOMAIN`].
    pub fn revisions(&self) -> BTreeMap<String, i64> {
        let mut revisions = self.reducer_revisions();
        revisions.insert(INDEX_DOMAIN.to_string(), INDEX_REVISION);
        revisions
    }

    /// Each reducer's fold revision, by domain — what the rows themselves depend on.
    pub fn reducer_revisions(&self) -> BTreeMap<String, i64> {
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

    /// Fold one op: carry out its changes, then keep the board's indexes.
    ///
    /// An op the table cannot hold — a key past `MAX_KEY_BYTES`, a row past `MAX_ITEM_BYTES` — is
    /// [`Folded::Refused`] before it writes anything, rather than failing the batch for ever: the
    /// relay keeps it, as it keeps any op a server does not fold. A row that only outgrows the limit
    /// through earlier ops refuses the one write that would break it ([`Outcome::TooLarge`]); the
    /// op's other writes stand. That last case is the one place the result depends on the order ops
    /// arrive in — which op's cell made the row too large — and a local replica, which has no item
    /// limit, holds all of it.
    pub async fn fold<T: Table>(&self, table: &T, op: &Op) -> Result<Folded, FoldError<T::Error>> {
        let changes = self.changes(op);
        if changes.is_empty() {
            return Ok(Folded::NotFolded);
        }
        let writes = changes
            .iter()
            .map(|c| plan(&self.layout, c))
            .collect::<Result<Vec<_>, _>>()
            .map_err(FoldError::Layout)?;
        let refuse = |reason: String| {
            Folded::Refused(Refusal {
                op_id: op.op_id.clone(),
                reason,
            })
        };
        for key in writes
            .iter()
            .map(|w| &w.sk)
            .chain(&board::derived_keys(&changes))
        {
            if key.len() > MAX_KEY_BYTES {
                return Ok(refuse(format!(
                    "a key of {} bytes, past the {MAX_KEY_BYTES} a sort key may have",
                    key.len()
                )));
            }
        }
        if let Some(w) = writes.iter().find(|w| w.bytes() > MAX_ITEM_BYTES) {
            return Ok(refuse(format!(
                "a row of {} bytes, past the {MAX_ITEM_BYTES} an item may have",
                w.bytes()
            )));
        }
        let mut too_large = Vec::new();
        for write in &writes {
            if table.write(write).await.map_err(FoldError::Table)? == Outcome::TooLarge {
                too_large.push(write.sk.clone());
            }
        }
        board::after(table, &changes)
            .await
            .map_err(FoldError::Table)?;
        if too_large.is_empty() {
            Ok(Folded::Folded)
        } else {
            Ok(refuse(format!(
                "rows that would outgrow {MAX_ITEM_BYTES} bytes kept their state: {}",
                too_large.join(", ")
            )))
        }
    }

    /// Fold a batch of ops pulled from the relay — each with its relay position — and move the
    /// watermark to the highest position among them. Returns the ops it refused, for the caller to
    /// log; they do not hold the watermark back.
    pub async fn fold_batch<T: Table>(
        &self,
        table: &T,
        ops: &[(i64, Op)],
    ) -> Result<Vec<Refusal>, FoldError<T::Error>> {
        let mut refused = Vec::new();
        for (_, op) in ops {
            if let Folded::Refused(r) = self.fold(table, op).await? {
                refused.push(r);
            }
        }
        if let Some(through) = ops.iter().map(|(seq, _)| *seq).max() {
            self.advance(table, through).await?;
        }
        Ok(refused)
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

    /// Delete every row of the stream — the views, the adjacency entries and the watermark, the
    /// watermark LAST: a clear that dies half way leaves a stream whose watermark still names the old
    /// revisions, so the next run clears it again instead of taking the leftovers for a fresh stream
    /// (review of PR #24, Integrity #3).
    pub async fn clear<T: Table>(table: &T) -> Result<(), T::Error> {
        for row in table.query_prefix("").await? {
            if let Some(Cell::Text(sk)) = row.get(crate::layout::SK) {
                if sk != WATERMARK_KEY {
                    table.delete(sk).await?;
                }
            }
        }
        table.delete(WATERMARK_KEY).await
    }
}
