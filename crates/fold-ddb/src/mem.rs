//! [`MemTable`]: one stream's partition in memory, with DynamoDB's condition semantics — what the
//! tests fold into, and what an embedder's own tests can fold into without an endpoint.
//!
//! It refuses a row past `MAX_ITEM_BYTES`, counted as [`attribute_bytes`] counts — DynamoDB's own
//! count differs by a few bytes per attribute at most.
//!
//! It counts the requests it answers ([`Requests`]), so the cost of a fold — the flag fan-out above
//! all — is measured on the same code path the DynamoDB table runs.

use crate::layout::{ACTIVE, DATED, DATED_AT, MAX_ITEM_BYTES, PK, SK};
use crate::table::{Dated, Outcome, Row, Table};
use crate::write::{attribute_bytes, beats, Cond, Write};
use nxs_foundation::change::Cell;
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Mutex;

/// How many requests of each kind a table answered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Requests {
    pub gets: u64,
    pub writes: u64,
    pub deletes: u64,
    pub queries: u64,
    pub index_queries: u64,
}

/// One stream's partition, in memory.
#[derive(Debug, Default)]
pub struct MemTable {
    stream: String,
    rows: Mutex<BTreeMap<String, Row>>,
    requests: Mutex<Requests>,
}

impl MemTable {
    pub fn new(stream: &str) -> MemTable {
        MemTable {
            stream: stream.to_string(),
            ..MemTable::default()
        }
    }

    /// Every stored row, by sort key.
    pub fn rows(&self) -> BTreeMap<String, Row> {
        self.rows.lock().unwrap().clone()
    }

    /// The requests answered so far.
    pub fn requests(&self) -> Requests {
        *self.requests.lock().unwrap()
    }

    fn count(&self, f: impl FnOnce(&mut Requests)) {
        f(&mut self.requests.lock().unwrap());
    }
}

/// Whether `cond` holds over `row` as DynamoDB evaluates it: a comparison with an absent attribute,
/// or with one of another type, is false.
fn holds(cond: &Cond, row: Option<&Row>) -> bool {
    match cond {
        Cond::RowAbsent => row.is_none(),
        Cond::RowPresent => row.is_some(),
        Cond::Absent(a) => row.is_none_or(|r| !r.contains_key(a)),
        Cond::Beats {
            columns,
            theirs,
            wins,
        } => {
            let Some(row) = row else { return false };
            let stored: Option<Vec<i64>> = columns
                .iter()
                .map(|c| match row.get(c) {
                    Some(Cell::Int(v)) => Some(*v),
                    _ => None,
                })
                .collect();
            stored.is_some_and(|stored| !columns.is_empty() && beats(theirs, &stored, *wins))
        }
        Cond::Any(parts) => parts.iter().any(|p| holds(p, row)),
    }
}

impl Table for MemTable {
    type Error = Infallible;

    fn stream(&self) -> &str {
        &self.stream
    }

    async fn get(&self, sk: &str) -> Result<Option<Row>, Infallible> {
        self.count(|r| r.gets += 1);
        Ok(self.rows.lock().unwrap().get(sk).cloned())
    }

    async fn write(&self, write: &Write) -> Result<Outcome, Infallible> {
        self.count(|r| r.writes += 1);
        let mut rows = self.rows.lock().unwrap();
        if let Some(cond) = &write.condition {
            if !holds(cond, rows.get(&write.sk)) {
                return Ok(Outcome::Kept);
            }
        }
        let mut row = rows.get(&write.sk).cloned().unwrap_or_else(|| {
            Row::from([
                (PK.to_string(), Cell::Text(self.stream.clone())),
                (SK.to_string(), Cell::Text(write.sk.clone())),
            ])
        });
        for (a, c) in &write.set {
            row.insert(a.clone(), c.clone());
        }
        for a in &write.remove {
            row.remove(a);
        }
        if row
            .iter()
            .map(|(n, c)| attribute_bytes(n, c))
            .sum::<usize>()
            > MAX_ITEM_BYTES
        {
            return Ok(Outcome::TooLarge);
        }
        rows.insert(write.sk.clone(), row);
        Ok(Outcome::Written)
    }

    async fn delete(&self, sk: &str) -> Result<(), Infallible> {
        self.count(|r| r.deletes += 1);
        self.rows.lock().unwrap().remove(sk);
        Ok(())
    }

    async fn query_prefix(&self, prefix: &str) -> Result<Vec<Row>, Infallible> {
        self.count(|r| r.queries += 1);
        Ok(self
            .rows
            .lock()
            .unwrap()
            .range(prefix.to_string()..)
            .take_while(|(k, _)| k.starts_with(prefix))
            .map(|(_, r)| r.clone())
            .collect())
    }

    async fn query_active(&self) -> Result<Vec<Row>, Infallible> {
        self.count(|r| r.index_queries += 1);
        let partition = Cell::Text(self.stream.clone());
        Ok(self
            .rows
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.get(ACTIVE) == Some(&partition))
            .cloned()
            .collect())
    }

    async fn query_dated(&self, lane: Dated, limit: Option<usize>) -> Result<Vec<Row>, Infallible> {
        self.count(|r| r.index_queries += 1);
        let partition = Cell::Text(lane.partition(&self.stream));
        let mut rows: Vec<Row> = self
            .rows
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.get(DATED) == Some(&partition))
            .cloned()
            .collect();
        let at = |r: &Row| match r.get(DATED_AT) {
            Some(Cell::Text(v)) => v.clone(),
            _ => String::new(),
        };
        rows.sort_by_key(|r| std::cmp::Reverse(at(r)));
        rows.truncate(limit.unwrap_or(usize::MAX));
        Ok(rows)
    }
}

/// Run a future that never waits — every [`MemTable`] future — to completion on this thread.
pub fn ready<F: std::future::Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => panic!("a MemTable future waited"),
    }
}
