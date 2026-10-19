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
//! rows, then fold from relay position 0. The clear leaves a sentinel watermark naming the position
//! the refold has to reach, and readers answer `NotCurrent` / `Refolding` until it has.
//!
//! **No mixed fold versions.** `is_current` is equality, not order: a build with OLDER rules that
//! meets a stream folded by newer ones also finds it not current, and clears and refolds it back.
//! Two builds with different revisions folding the same streams — a rolling deploy, a rollback with
//! a newer build still running — would refold every stream back and forth. Roll a revision bump
//! out to every fold run at once, and roll back the same way (a fold run that builds anew beside
//! the served build, as one that keeps a namespace per build, ping-pongs builds instead).
//!
//! The rules this crate adds beside the reducers' rows — the board's index flags and the entries it
//! keeps under a ticket — have a revision of their own, [`INDEX_REVISION`], recorded in the same
//! watermark under [`INDEX_DOMAIN`]. A release that changes what the fold writes beside the rows
//! bumps it, and every stream folded before is no longer current and is folded again the same way.
//! Revision 1 (6j6v.t1ym) adds the `.ladj#`/`.tadj#` entries; a stream folded before it has none,
//! so its watermark names no index revision at all and the next run refolds it. Revision 2
//! (6j6v.15ed) adds the `.cadj#` entries, a ticket's outgoing `contributes_to` edges. Revision 3
//! (6j6v.1c5s) makes the `.adj#` entries self-describing — ends, kind, coordinate, presence — for
//! the derived ranking keys ([`crate::records::with_ranking`]), and sets the `typed` index
//! attribute on every live item row (6j6v.xkr7, [`crate::records::list_board`]). Both ship in one
//! release: no stream was ever folded by revision 3 without the attribute. Revision 4 (6j6v.9tx3)
//! adds the `.radj#` reverse-reference hints and their `.rsrc#` mirrors ([`board`], "Reverse
//! references"), kept as a function of the rows for every id-shaped custom value — what a hosted
//! `list --ref` reads ([`crate::records::referrers`]).
//!
//! # What the server cannot see
//!
//! A local replica refuses an op whose `(lamport, site)` another op already holds ("collided") and
//! folds only the first. The op table has no index on the coordinate, so the server folds both.
//! Registers still keep the first arrival — an equal version beats nothing — and only an add or a
//! note of the second op is extra. Such ops predate fc5p, which made a replica mint past a taken
//! coordinate.

use crate::board;
use crate::layout::{
    table_prefix, Layout, LayoutError, MAX_ITEM_BYTES, MAX_KEY_BYTES, MAX_PARTITION_BYTES, PK, SK,
    WATERMARK_KEY,
};
use crate::table::{text, Outcome, Table};
use crate::write::{plan, Cond, Write};
use nxs_foundation::change::{Cell, Change, Wins};
use nxs_foundation::model::Op;
use nxs_foundation::reducer::{folder_for, Reducer};
use std::collections::BTreeMap;

/// The key, among a watermark's revisions, of the rules this crate keeps beside the reducers' rows
/// (see the module doc). No reducer has this domain.
pub const INDEX_DOMAIN: &str = "index";

/// The watermark attribute [`Folder::clear`] sets: [`Watermark::refold_until`].
pub const REFOLD_UNTIL: &str = "refold_until";

/// The revision of those rules. 1: the entries under a ticket for its label and thread link adds
/// (6j6v.t1ym). 2: the entries under a ticket for its outgoing `contributes_to` edges (6j6v.15ed).
/// 3: the adjacency entries copy their edge's ends, kind and coordinate and say whether it is
/// removed (6j6v.1c5s), and every live item row carries its `typed` index partition (6j6v.xkr7).
/// 4 (6j6v.9tx3): the `.radj#` reverse-reference hints and their `.rsrc#` mirrors, one row per
/// hinted value, kept as a function of the rows for every custom value `ref_place_ok` admits.
pub const INDEX_REVISION: i64 = 4;

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
    /// Set by [`Folder::clear`]: the relay position a refold has to reach before the stream is
    /// served again — readers answer `RecordsError::Refolding` while `folded_through` is below it
    /// (review of PR #55, Integrity #2). `None` on a stream never cleared.
    pub refold_until: Option<i64>,
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
    /// An op the table cannot hold — a key past `MAX_KEY_BYTES`, a type whose `typed` index
    /// partition is past `MAX_PARTITION_BYTES`, a row past `MAX_ITEM_BYTES`, an item row that would
    /// pass it once it carries its index attributes (`active`, `typed`, `dated`; one `GetItem` per
    /// item row the op writes, to weigh it) — is
    /// [`Folded::Refused`] before it writes anything, rather than failing the batch for ever: the
    /// relay keeps it, as it keeps any op a server does not fold. An item row that only outgrows
    /// the limit through earlier ops is caught by that weighing too, so the whole op is refused;
    /// any other row refuses the one write that would break it ([`Outcome::TooLarge`]), and the
    /// op's other writes stand. Those cases are the one place the result depends on the order ops
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
        if let Some(p) = board::typed_partitions(table.stream(), &changes)
            .into_iter()
            .find(|p| p.len() > MAX_PARTITION_BYTES)
        {
            return Ok(refuse(format!(
                "a type whose `typed` index partition has {} bytes, past the {MAX_PARTITION_BYTES} \
                 a partition key may have",
                p.len()
            )));
        }
        if let Some(w) = writes.iter().find(|w| w.bytes() > MAX_ITEM_BYTES) {
            return Ok(refuse(format!(
                "a row of {} bytes, past the {MAX_ITEM_BYTES} an item may have",
                w.bytes()
            )));
        }
        // An item row takes its index attributes in a write of their own, after the row
        // (`board::refresh_item`): one the row has no room for would leave the item off an index
        // in silence. So the item rows this op writes are weighed first — stored row, this op's
        // writes, and the index attributes it would then carry — and the op is refused before it
        // writes anything (review of PR #55, Integrity #4).
        for (id, bytes) in self.item_bytes(table, &writes).await? {
            if bytes > MAX_ITEM_BYTES {
                return Ok(refuse(format!(
                    "item {id} would weigh {bytes} bytes with its index attributes, past the \
                     {MAX_ITEM_BYTES} an item may have"
                )));
            }
        }
        let mut too_large = Vec::new();
        for write in &writes {
            if table.write(write).await.map_err(FoldError::Table)? == Outcome::TooLarge {
                too_large.push(write.sk.clone());
            }
        }
        // Past the check above only where DynamoDB counts a few bytes more than `attribute_bytes`:
        // reported, never dropped.
        too_large.extend(
            board::after(table, &changes)
                .await
                .map_err(FoldError::Table)?,
        );
        if too_large.is_empty() {
            Ok(Folded::Folded)
        } else {
            Ok(refuse(format!(
                "rows that would outgrow {MAX_ITEM_BYTES} bytes kept their state: {}",
                too_large.join(", ")
            )))
        }
    }

    /// What each item row `writes` touch would weigh after them, with the index attributes it
    /// would then carry ([`board::indexed_bytes`]): one `GetItem` per such row. Every write is
    /// counted as applied, its condition aside — so a write that would be kept can only make the
    /// estimate larger, never let a row past the limit.
    async fn item_bytes<T: Table>(
        &self,
        table: &T,
        writes: &[Write],
    ) -> Result<Vec<(String, usize)>, FoldError<T::Error>> {
        let prefix = table_prefix("items");
        let mut rows: BTreeMap<&str, crate::table::Row> = BTreeMap::new();
        for write in writes.iter().filter(|w| w.sk.starts_with(&prefix)) {
            if !rows.contains_key(write.sk.as_str()) {
                let stored = table
                    .get(&write.sk)
                    .await
                    .map_err(FoldError::Table)?
                    .unwrap_or_else(|| {
                        crate::table::Row::from([
                            (PK.to_string(), Cell::Text(table.stream().to_string())),
                            (SK.to_string(), Cell::Text(write.sk.clone())),
                        ])
                    });
                rows.insert(write.sk.as_str(), stored);
            }
            if let Some(row) = rows.get_mut(write.sk.as_str()) {
                for (a, c) in &write.set {
                    row.insert(a.clone(), c.clone());
                }
                for a in &write.remove {
                    row.remove(a);
                }
            }
        }
        Ok(rows
            .values()
            .map(|row| {
                let id = text(row, "id").unwrap_or_default().to_string();
                let bytes = board::indexed_bytes(table.stream(), &id, row);
                (id, bytes)
            })
            .collect())
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

    /// A watermark that names no revision — so no reader serves the stream ([`Folder::clear`],
    /// `snapshot::import`) — at `folded_through`, with the position a refold must reach.
    pub(crate) fn sentinel(folded_through: i64, until: Option<i64>) -> Write {
        let mut set = vec![("folded_through".to_string(), Cell::Int(folded_through))];
        set.extend(until.map(|u| (REFOLD_UNTIL.to_string(), Cell::Int(u))));
        Write {
            sk: WATERMARK_KEY.to_string(),
            set,
            remove: Vec::new(),
            condition: None,
        }
    }

    /// This folder's watermark at `through`, written whatever the stored one says, and without a
    /// refold position — what `snapshot::import` ends with.
    pub(crate) fn stamp(&self, through: i64) -> Write {
        let mut set = vec![("folded_through".to_string(), Cell::Int(through))];
        set.extend(
            self.revisions()
                .into_iter()
                .map(|(domain, rev)| (format!("revision_{domain}"), Cell::Int(rev))),
        );
        Write {
            sk: WATERMARK_KEY.to_string(),
            set,
            remove: vec![REFOLD_UNTIL.to_string()],
            condition: None,
        }
    }

    /// The stream's watermark, `None` before its first batch.
    pub async fn watermark<T: Table>(table: &T) -> Result<Option<Watermark>, T::Error> {
        Ok(table.get(WATERMARK_KEY).await?.map(|row| {
            let mut mark = Watermark::default();
            for (attr, cell) in row {
                match (attr.as_str(), cell) {
                    ("folded_through", Cell::Int(v)) => mark.folded_through = v,
                    (REFOLD_UNTIL, Cell::Int(v)) => mark.refold_until = Some(v),
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

    /// Delete every row of the stream — the views and the adjacency entries — for a refold from
    /// position 0 that has to reach relay position `until` (the stream's head when the refold
    /// begins) before the stream is served again.
    ///
    /// The watermark is not deleted: it is rewritten FIRST, as a sentinel that names no revision,
    /// `folded_through` 0 and `refold_until` = `until`. So readers answer `NotCurrent` from the
    /// moment the clear begins (review of PR #55, Integrity #2) — a cleared, half-cleared or
    /// half-refolded stream is never served as if whole — and a clear that dies half way leaves a
    /// watermark that is not current, so the next run clears it again instead of taking the
    /// leftovers for a fresh stream (review of PR #24, Integrity #3). The refold's first batch
    /// writes this build's revisions ([`Folder::advance`]), and from then on the readers answer
    /// `Refolding` until `folded_through` reaches `until`; nothing has to be written for that, so a
    /// refold that dies and is resumed, or folds a batch twice, ends served all the same. `until`
    /// 0 serves the stream from its first refolded batch on, as before revisions were gated.
    pub async fn clear<T: Table>(table: &T, until: i64) -> Result<(), T::Error> {
        let rows = table.query_prefix("").await?;
        let stale: Vec<String> = rows
            .iter()
            .find(|r| text(r, crate::layout::SK) == Some(WATERMARK_KEY))
            .map(|mark| {
                mark.keys()
                    .filter(|a| a.starts_with("revision_"))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let mut sentinel = Folder::sentinel(0, Some(until));
        sentinel.remove = stale;
        table.write(&sentinel).await?;
        for row in rows {
            if let Some(sk) = text(&row, crate::layout::SK) {
                if sk != WATERMARK_KEY {
                    table.delete(sk).await?;
                }
            }
        }
        Ok(())
    }
}
