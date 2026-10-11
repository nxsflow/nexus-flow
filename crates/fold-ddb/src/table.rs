//! The requests a server fold makes of its table, as a trait: [`MemTable`](crate::mem::MemTable)
//! answers them in memory for the tests, `ddb::DynamoDbTable` (feature `dynamodb`) answers them with DynamoDB.
//! None of them is a Scan — every read names its partition, and the three index reads name an index.

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
}

/// A text cell of `row`, `None` when it is absent or NULL.
pub fn text<'a>(row: &'a Row, column: &str) -> Option<&'a str> {
    match row.get(column) {
        Some(Cell::Text(v)) => Some(v),
        _ => None,
    }
}
