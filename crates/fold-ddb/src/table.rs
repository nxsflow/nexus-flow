//! The requests a server fold makes of its table, as a trait: [`MemTable`](crate::mem::MemTable)
//! answers them in memory for the tests, `ddb::DynamoDbTable` (feature `dynamodb`) answers them with DynamoDB.
//! None of them is a Scan — every read names its partition, and the three index reads name an index.

use crate::layout::{PK, SK};
use crate::write::Write;
use nxs_foundation::change::Cell;
use std::collections::BTreeMap;
use std::future::Future;

/// A stored row: its attributes by name, the sort key and index attributes among them.
pub type Row = BTreeMap<String, Cell>;

/// The most keys one `BatchGetItem` names — DynamoDB's own limit ([`Table::get_many`]).
pub const BATCH_GET_KEYS: usize = 100;

/// What a write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Written,
    /// Its condition did not hold; the row is as it was.
    Kept,
    /// The row would outgrow what the table stores (`MAX_ITEM_BYTES`); the row is as it was.
    TooLarge,
}

/// A lane of the `dated` index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dated {
    /// Closed, not archived, not deleted — by `closed_at`.
    Closed,
    /// Archived, any status, not deleted — by `archived`.
    Archived,
}

impl Dated {
    /// The `dated` index partition of this lane in `stream`.
    pub fn partition(self, stream: &str) -> String {
        match self {
            Dated::Closed => format!("{stream}#closed"),
            Dated::Archived => format!("{stream}#archived"),
        }
    }
}

/// The most rows one `BatchWriteItem` names — DynamoDB's own limit ([`Table::delete_batch`]).
pub const BATCH_WRITE_KEYS: usize = 25;

/// One key of a stream's partition as the purge's plan reads it ([`Table::sort_keys_after`]): the
/// partition key the table answered with — the purge checks it is the target before a key goes into
/// a delete batch — the sort key, and the two attributes the plan needs from the watermark, read
/// from no other row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    pub stream: String,
    pub sk: String,
    /// `folded_through`, on the watermark.
    pub folded_through: Option<i64>,
    /// `revision_index` — the watermark's [`crate::fold::INDEX_REVISION`] entry.
    pub index_revision: Option<i64>,
}

impl KeyRow {
    /// The key row of a stored row; `None` for a row without its keys.
    pub fn of(row: &Row) -> Option<KeyRow> {
        let int = |name: &str| match row.get(name) {
            Some(Cell::Int(v)) => Some(*v),
            _ => None,
        };
        Some(KeyRow {
            stream: text(row, PK)?.to_string(),
            sk: text(row, SK)?.to_string(),
            folded_through: int("folded_through"),
            index_revision: int(INDEX_REVISION_ATTRIBUTE),
        })
    }
}

/// The watermark attribute that carries [`crate::fold::INDEX_REVISION`]: `revision_` and
/// [`crate::fold::INDEX_DOMAIN`].
pub const INDEX_REVISION_ATTRIBUTE: &str = "revision_index";

/// One stream's partition of the views table.
pub trait Table: Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// The stream this table holds.
    fn stream(&self) -> &str;

    /// One row, read strongly consistent.
    fn get(&self, sk: &str) -> impl Future<Output = Result<Option<Row>, Self::Error>> + Send;

    /// Several rows, read strongly consistent, by sort key — a key without a row is absent from the
    /// answer, and a key named twice is read once. DynamoDB answers it with `BatchGetItem`, at most
    /// [`BATCH_GET_KEYS`] keys a request, sending the keys a request left unprocessed again; so it
    /// costs `⌈keys / 100⌉` requests, more only when DynamoDB throttles or a response passes 16 MB.
    /// No keys, no request.
    fn get_many(
        &self,
        sks: &[String],
    ) -> impl Future<Output = Result<BTreeMap<String, Row>, Self::Error>> + Send;

    /// One conditional write.
    fn write(&self, write: &Write) -> impl Future<Output = Result<Outcome, Self::Error>> + Send;

    /// Delete one row.
    fn delete(&self, sk: &str) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Every row whose sort key starts with `prefix`, in sort-key order, read strongly consistent.
    fn query_prefix(
        &self,
        prefix: &str,
    ) -> impl Future<Output = Result<Vec<Row>, Self::Error>> + Send;

    /// Every row on the `active` index. An index is read eventually consistent: a fold that just
    /// finished may not show yet.
    fn query_active(&self) -> impl Future<Output = Result<Vec<Row>, Self::Error>> + Send;

    /// The rows of one `dated` lane, newest first, at most `limit` of them.
    fn query_dated(
        &self,
        lane: Dated,
        limit: Option<usize>,
    ) -> impl Future<Output = Result<Vec<Row>, Self::Error>> + Send;

    /// Every row on the `typed` index under `item_type` (6j6v.xkr7): the live items of that type,
    /// archived ones included, in sort-key (id) order. Eventually consistent, as every index read.
    fn query_typed(
        &self,
        item_type: &str,
    ) -> impl Future<Output = Result<Vec<Row>, Self::Error>> + Send;

    /// At most `limit` keys of this stream's partition, in sort-key order, after `after` (the
    /// purge's plan, 6j6v.dbmd); strongly consistent; keys and the watermark's two attributes only.
    /// The cursor is `None` once the partition is exhausted. DynamoDB may answer FEWER than `limit`
    /// with a cursor — a page it cut at 1 MB — so a caller follows the cursor, never the count.
    ///
    /// The default reads the whole partition through [`Table::query_prefix`] and cuts it, so an
    /// embedder's table keeps compiling; `ddb::DynamoDbTable` answers it with one Query a page.
    fn sort_keys_after(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> impl Future<Output = Result<(Vec<KeyRow>, Option<String>), Self::Error>> + Send {
        async move {
            let rows = self.query_prefix("").await?;
            let mut keys: Vec<KeyRow> = rows
                .iter()
                .filter_map(KeyRow::of)
                .filter(|k| after.is_none_or(|a| k.sk.as_str() > a))
                .collect();
            keys.sort_by(|a, b| a.sk.cmp(&b.sk));
            let more = keys.len() > limit;
            keys.truncate(limit);
            let cursor = if more {
                keys.last().map(|k| k.sk.clone())
            } else {
                None
            };
            Ok((keys, cursor))
        }
    }

    /// Delete these rows of this stream's partition in ONE request — at most
    /// [`BATCH_WRITE_KEYS`] — and answer the sort keys it left unprocessed, for the caller to send
    /// again (the purge, which counts what went). DynamoDB: one `BatchWriteItem` of
    /// `DeleteRequest`s. The default deletes each row and leaves nothing unprocessed.
    fn delete_batch(
        &self,
        sks: &[String],
    ) -> impl Future<Output = Result<Vec<String>, Self::Error>> + Send {
        async move {
            for sk in sks {
                self.delete(sk).await?;
            }
            Ok(Vec::new())
        }
    }

    /// Wait before the `attempt`-th call of a batch that came back partly unprocessed. The default
    /// does not wait; DynamoDB waits 50 ms doubled per attempt.
    fn backoff(&self, attempt: u32) -> impl Future<Output = ()> + Send {
        let _ = attempt;
        async {}
    }
}

/// A text cell of `row`, `None` when it is absent or NULL.
pub fn text<'a>(row: &'a Row, column: &str) -> Option<&'a str> {
    match row.get(column) {
        Some(Cell::Text(v)) => Some(v),
        _ => None,
    }
}
