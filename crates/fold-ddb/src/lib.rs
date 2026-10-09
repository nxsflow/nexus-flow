//! Folding on a server (6j6v.vvw6): the reducers' [described changes](nxs_foundation::change)
//! carried out as conditional writes against one table per deployment, the board's `active` and
//! `dated` indexes kept beside the rows, next/blocked derived by the same library a local replica
//! uses, and snapshots that are tables plus watermark, without the log.
//!
//! The pieces, in the order a fold run uses them: [`fold::Folder`] maps each op through the
//! platform's reducers and [`write::plan`] turns every change into a [`write::Write`];
//! [`board`] keeps the indexes and reads the lanes; [`chunks`] reads an item's live chunks;
//! [`snapshot`] starts a stream without folding its whole history. [`table::Table`] is the seam
//! to storage: [`mem::MemTable`] in memory, and with the `dynamodb` feature `ddb::DynamoDbTable`.
//!
//! # The table contract (point 5)
//!
//! Provisioned by the deploying stack; production code here never creates a table.
//! `ddb::create_table` (feature `dynamodb`) builds the request the integration tests send, and the
//! stack has to match it:
//!
//! | what                 | name                     | type | role                                                                   |
//! |----------------------|--------------------------|------|------------------------------------------------------------------------|
//! | partition key        | `stream_id`              | S    | one partition per stream                                               |
//! | sort key             | `sk`                     | S    | `<view table>#<key cells>`, `.adj#<ticket>#<tag>`, `.meta#watermark`   |
//! | GSI `active`         | `nxf_active` / `sk`      | S/S  | sparse: active tickets and the present dep/parent edges touching one   |
//! | GSI `dated`          | `nxf_dated` / `nxf_dated_at` | S/S | sparse: `<stream>#closed` and `<stream>#archived`, by `<instant>␟<id>` |
//!
//! Both indexes project ALL attributes, so a lane is one Query and no follow-up reads. Billing mode
//! is the deployment's call. No local secondary index: without one, DynamoDB sets no limit on the
//! size of a stream's partition (the 10 GB item-collection limit applies only to tables with an LSI).
//! What does bound a stream is the throughput of ONE partition key: a stream's `active` and `dated`
//! entries each share one index partition — a write-heavy stream is limited to that partition's
//! throughput (1,000 writes per second).
//!
//! Every row of view table `t` carries its columns as attributes of the same name ([`layout`]); a
//! NULL cell is DynamoDB's NULL, an integer a number, text a string.
//!
//! # Freshness
//!
//! A row read (`GetItem`, a Query on the table) is strongly consistent. The two index reads —
//! `board::select` and `board::dated` — are eventually consistent, as every DynamoDB GSI read is:
//! right after a fold they may answer the state before it, for as long as the index takes to catch
//! up (typically well under a second). The lanes are the local lanes of THAT state. A reader that
//! must see its own write reads the ticket's row instead.
//!
//! # Limits
//!
//! An op the table cannot hold — a sort key past 1,024 bytes, a row past 400 KB — is refused
//! (`fold::Folded::Refused`) and reported by `Folder::fold_batch`, instead of failing every run at
//! the same op; the relay keeps it. Every request is awaited in sequence, so a fold run sets its
//! own deadline: a client operation timeout on the SDK client it hands in, and a bound on the batch
//! it folds. An op's work is proportional to what it says — one write per change, one per edge of a
//! ticket whose activity flips — and is not capped here, because a capped fold would no longer be
//! the local fold.
//!
//! # Cost
//!
//! A change is one `UpdateItem`. An item field op makes three changes (the cell and the ticket's
//! two instants) plus one `GetItem` for the index, and one more write when the ticket's lanes move.
//! A flip of a ticket's activity adds one Query and, per edge touching it, at most three reads and
//! one write. No request is a Scan: every read names its partition key, the board's through an
//! index. The `dynamodb-local` CI job counts the requests of a fold and proves it.

pub mod board;
pub mod chunks;
pub mod fold;
pub mod layout;
pub mod mem;
pub mod snapshot;
pub mod table;
pub mod write;

#[cfg(feature = "dynamodb")]
pub mod ddb;
