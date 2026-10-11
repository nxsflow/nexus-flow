//! Folding on a server (6j6v.vvw6): the reducers' [described changes](nxs_foundation::change)
//! carried out as conditional writes against one table per deployment, the board's `active` and
//! `dated` indexes kept beside the rows, next/blocked derived by the same library a local replica
//! uses, and snapshots that are tables plus watermark, without the log; a type's live items listed
//! by a third index, `typed`, for a ranked hosted `list`.
//!
//! The pieces, in the order a fold run uses them: [`fold::Folder`] maps each op through the
//! platform's reducers and [`write::plan`] turns every change into a [`write::Write`];
//! [`board`] keeps the indexes and reads the lanes; [`records`] reads the active tickets' records
//! for the facade's store-free `next`/`blocked`/`deferred`; [`chunks`] reads an item's live chunks;
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
//! | sort key             | `sk`                     | S    | `<view table>#<key cells>`, `.adj#`/`.ladj#`/`.tadj#<ticket>#<tag>`, `.meta#watermark` |
//! | GSI `active`         | `nxf_active` / `sk`      | S/S  | sparse: active tickets and the present dep/parent edges touching one   |
//! | GSI `dated`          | `nxf_dated` / `nxf_dated_at` | S/S | sparse: `<stream>#closed` and `<stream>#archived`, by `<instant>␟<id>` |
//! | GSI `typed`          | `nxf_typed` / `sk`       | S/S  | sparse: `<stream>#<type>` on every live item row (not deleted; archived included) |
//!
//! The `.adj#<ticket>#<tag>` entries are plain rows in the stream's partition, on no index. Since
//! index revision 3 (6j6v.1c5s) each also carries its edge's `from_id`, `to_id`, `kind`, `lamport`,
//! `site` and a `removed` flag — attributes, not keys.
//!
//! Every index projects ALL attributes, so a lane is one Query and no follow-up reads.
//!
//! **The `typed` index (6j6v.xkr7)** is the one change to this contract since `active` and `dated`
//! (docs/specs/derived-ranking-keys.md §6, owner decision §10.4): one Query lists the live items of
//! a type, the `list` scope a hosted `list` and the derived ranking keys rank over
//! ([`records::list_board`]). Its partition key `nxf_typed` is `<stream>#<type>`, the type escaped
//! as a key value is (`%` → `%25`, `#` → `%23`; [`layout::typed_partition`]); its sort key is the
//! table's `sk`. The fold sets it on every item row whose type is set and which is not deleted —
//! closed and archived ones included — and removes it on a delete, written after the row like the
//! other index attributes ([`board`], "`typed`"). The deploying stack adds the GSI FIRST: a fold
//! against a table without it still writes the attribute, which costs nothing and breaks nothing;
//! only the read — a `list` through [`records::list_board`], a `next` whose keys read a
//! `[ranking.list]` rank — needs the index. A type whose partition would pass DynamoDB's 2,048
//! bytes for an index key is refused at fold time ([`layout::MAX_PARTITION_BYTES`]). The lanes
//! need nothing else; the RECORDS a lane is shown with carry joins from other tables, which
//! [`records`] reads with a fixed number of requests per active ticket. Billing mode
//! is the deployment's call. No local secondary index: without one, DynamoDB sets no limit on the
//! size of a stream's partition (the 10 GB item-collection limit applies only to tables with an LSI).
//! What does bound a stream is the throughput of ONE partition key: a stream's `active` and `dated`
//! entries each share one index partition, its items of one type one `typed` partition — a
//! write-heavy stream is limited to that partition's throughput (1,000 writes per second).
//!
//! Every row of view table `t` carries its columns as attributes of the same name ([`layout`]); a
//! NULL cell is DynamoDB's NULL, an integer a number, text a string.
//!
//! # Freshness
//!
//! A row read (`GetItem`, a Query on the table) is strongly consistent. The index reads —
//! `board::select`, `board::dated` and the `typed` Query of [`records::list_board`] — are eventually
//! consistent, as every DynamoDB GSI read is:
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
//! one write. A label or thread link add, or its remove, adds two `GetItem`s and at most one write
//! for the entry under the ticket (`board`, "Who owns a label or a thread link"). Reading the
//! active tickets' records costs `4n + 1` requests for `n` tickets, plus one per parent outside
//! the selection, at most [`records::READ_CONCURRENCY`] tickets at a time ([`records`]). Folding an
//! edge remove reads the add and the remove and rewrites the entry under each end. The graph the
//! derived ranking keys read ([`records::with_ranking`]) costs one Query per active ticket and, per
//! inactive neighbour, one `GetItem` plus up to three Queries. A hosted `list`
//! ([`records::list_board`]) costs one `typed` Query per listed type and `4n + 1 + p` requests for
//! `n` listed items, as the records read. The `typed` index costs a fold at most one write per item
//! op, and only when the op gains, changes or loses the item's partition — a create, a retype, a
//! delete — in the same write as the item's other index attributes when they move too. No request
//! is a Scan: every read names its partition key, the board's through an index. The
//! `dynamodb-local` CI job counts the requests of a fold and of a records read, and proves it.
//!
//! # Rules beside the rows
//!
//! What this crate writes beside the reducers' rows — index flags, adjacency entries, the entries
//! under a ticket — has its own revision in the watermark ([`fold::INDEX_REVISION`]). A stream
//! folded by an older one is not current ([`fold::Folder::is_current`]) and is refolded the way a
//! stream of an older reducer revision is; [`records::active_board`] refuses it until then.

pub mod board;
pub mod chunks;
pub mod fold;
pub mod layout;
pub mod mem;
pub mod records;
pub mod snapshot;
pub mod table;
pub mod write;

#[cfg(feature = "dynamodb")]
pub mod ddb;
