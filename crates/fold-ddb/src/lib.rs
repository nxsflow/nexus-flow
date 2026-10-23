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
//! Beside the fold: [`deny`], the shared deny list a fold run asks before every chunk, and
//! [`purge`], which deletes one stream's partition for an account deletion ("Purge" below).
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
//! | sort key             | `sk`                     | S    | `<view table>#<key cells>`, `.adj#`/`.ladj#`/`.tadj#`/`.cadj#<ticket>#<tag>`, `.radj#<value>#<field>#<source>`, `.rsrc#<source>#<field>#<value>`, `.meta#watermark` |
//! | GSI `active`         | `nxf_active` / `sk`      | S/S  | sparse: active tickets and the present dep/parent edges touching one   |
//! | GSI `dated`          | `nxf_dated` / `nxf_dated_at` | S/S | sparse: `<stream>#closed` and `<stream>#archived`, by `<instant>␟<id>` |
//! | GSI `typed`          | `nxf_typed` / `sk`       | S/S  | sparse: `<stream>#<type>` on every live item row (not deleted; archived included) |
//!
//! The `.adj#<ticket>#<tag>` entries are plain rows in the stream's partition, on no index. Since
//! index revision 3 (6j6v.1c5s) each also carries its edge's `from_id`, `to_id`, `kind`, `lamport`,
//! `site` and a `removed` flag — attributes, not keys.
//!
//! The `.radj#<value>#<field>#<source>` reverse-reference hints and their
//! `.rsrc#<source>#<field>#<value>` mirrors (index revision 4, 6j6v.9tx3) are plain rows too, on
//! no index, one row per hinted value, each carrying `target_id`, `field` and `source_id`
//! ([`board`], "Reverse references"). Revision 4 (6j6v.9tx3) adds them for every id-shaped custom
//! value, kept as a function of the rows; a stream folded before is `NotCurrent` until its refold,
//! as revision 3 was — a full clear and refold by the fold run, every reader refused for its
//! duration, rolled out to every fold run at once; the stack changes nothing; the task fold
//! revision is unchanged, so a snapshot taken by the previous release imports.
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
//! two instants) plus two `GetItem`s — one before, to weigh the row with the index attributes it
//! would then carry against the 400 KB limit, one after, for the index — and one more write when
//! the ticket's lanes move.
//! A flip of a ticket's activity adds one Query and, per edge touching it, at most three reads and
//! one write. A label or thread link add, or its remove, adds two `GetItem`s and at most one write
//! for the entry under the ticket (`board`, "Who owns a label or a thread link"). Reading the
//! active tickets' records costs `5n + 1` requests for `n` tickets, plus one per parent outside
//! the selection, at most [`records::READ_CONCURRENCY`] tickets at a time ([`records`]). Folding an
//! edge remove reads the add and the remove and rewrites the entry under each end. The graph the
//! derived ranking keys read ([`records::with_ranking`]) costs one Query per active ticket and, per
//! inactive neighbour, up to three Queries, their rows read by `BatchGetItem` (100 keys a request).
//! A hosted `list` ([`records::list_board`]) costs one `typed` Query per listed type; one that is
//! not ranked keeps and cuts its rows first and reads four Queries per row of its page plus one
//! `BatchGetItem` for their timestamps, however many items the stream holds; a ranked one reads
//! four Queries per live item of its types — the scope its rank is computed over — and its point
//! reads by `BatchGetItem`. A hosted `list --ref` that is not ranked costs one Query per ref on
//! the hints, `⌈candidates / 100⌉` `BatchGetItem`s for the candidates' rows and the candidates'
//! own joins; a custom-field op costs one more Query for the place's reverse-reference reconcile,
//! and one `GetItem`, one Query and the delta's puts and deletes when an id-shaped value is or was
//! there (plugin-custom-fields §5.3, measured). No read caps its own requests: a server bounds one request by the
//! table it hands in (E4 spec §5.1). The `typed` index costs a fold at most one write per item
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
//! stream of an older reducer revision is. Every reader in [`records`] refuses it from the moment
//! the clear begins until the refold reaches the head the clear named ([`fold::Folder::clear`]):
//! `NotCurrent` while the watermark names no revision, `Refolding` after. Fold versions are not
//! mixed: a build with older rules meets a newer stream as not current, too (see [`fold`]).
//!
//! # Purge (6j6v.dbmd)
//!
//! The table keeps a complete derived copy of each stream's board, and board-fold ignores deletion
//! records, so an account deletion deletes the copy on its own: [`purge`] and its command line
//! `nxf-fold-purge`, the sibling of the relay's `nxf-relay purge` (6j6v.f6yb). **What goes:** the
//! stream's whole partition — every view row, every entry (`.adj#`, `.ladj#`, `.tadj#`, `.cadj#`,
//! `.radj#`, `.rsrc#`) and the watermark, the watermark last; the three indexes follow their base
//! items. A key of a kind this build does not know, or a watermark of a newer index revision, is
//! refused (78) before anything is deleted: run a build from the same pin as the deployed
//! board-fold (`cargo build -p nxs-fold-ddb --bin nxf-fold-purge --features dynamodb`).
//!
//! **Consumer contract v2.1** — ONE contract for the relay purge and this one (it supersedes f6yb's
//! "v1.4" and dbmd's "v2"; review round 4b, Integrity 8). The account sequence, steps A to 8 and
//! 8b, is written in one place, the relay operations runbook `docs/runbooks/relay-purge.md`; this
//! section restates the parts that concern this table. Binding: no breaking change without 14 days'
//! notice through nexflowit-revamp; additive changes come with a note on 6j6v.dbmd.
//!
//! - **The two passes.** Pass 1 (step 3): a dry run, then `--yes --plan <digest>`, right after the
//!   relay's `--yes` exited 0; its exit 3 does not block (a fold in flight can land after it). Pass
//!   2 (step 5): at least 25 h after the `finished_at` of the relay's confirm (step 4) — the
//!   DynamoDB stream keeps a record 24 h, an invocation runs at most 15 min — a confirm
//!   (`--dry-run --expect-empty --expect-identity <pin>`) that exits 0. Exit 3 there: a dry run, a
//!   bound `--yes`, step 0's proof again, then steps 4 and 5 again.
//! - **The fence is the deny list** (the "Deny table" section below). `nxf-fold-purge` refuses
//!   `--yes` and the confirm (78, `NotFenced`) unless the target's account is on it, and reports the
//!   one admission it read as `fence`. Step 0's proof, all three, before any destructive step:
//!   - (i) the deny item read back with a consistent `GetItem`; every board-fold function of the
//!     environment has `NXS_DENY_TABLE` naming the table whose `TableArn` equals the one the
//!     authorizer reports — compared by ARN, never by name — and the runner gives `nxf-fold-purge`
//!     its `NXS_DENY_TABLE` from the same source and checks the report's `deny.table_arn` against
//!     it. A function without it, or another ARN: stop.
//!   - (ii) the canary, run IN EACH ENVIRONMENT against the board-fold build that runs there, and
//!     again after every deploy of board-fold: a reserved `<h_canary>` (the first 32 hex of
//!     SHA-256(`nxs-canary:<environment>`) — `nxs-canary` is no issuer, so no account derives it)
//!     written to the deny table the same way; after its OWN write time + `deny_cache_ttl` + the
//!     largest function timeout, one message for `n-<h_canary>-1` through EVERY entry point that
//!     writes — a live-fold change-stream INSERT, a rescue message, a progress message and a
//!     backfill — is refused at `admission`, logged with `deny_list`, reaches the DLQ (or the
//!     backfill exits non-zero), and a Query of the canary's partition stays empty. Its claim holds
//!     for the entry points exercised, and the hand-over names them. It is foundations' to run and
//!     report (MF-5); until it has passed in an environment, step 8b applies there.
//!   - (iii) the wait: step 1, and so every destructive step, not before T_w (when the deny item's
//!     `PutItem` was acknowledged) + `deny_cache_ttl` (≤ 60 s) + the LARGEST timeout of any
//!     board-fold function that can write (live fold, rescue, progress, backfill; ≤ 15 min, 960 s in
//!     all on the staging probe). Every writing entry point is a Lambda invocation; a writer outside
//!     Lambda (a backfill from a CLI or a container) calls `admission` before every chunk too and
//!     adds its own longest chunk time to the wait, or is not run.
//! - **The fallback, step 8b**, only where (ii) has not yet passed: the queue window — wait `R_q`
//!   after the later of T_w + `deny_cache_ttl` + the timeout and pass 2's `finished_at`, then a
//!   fold confirm and a relay confirm. `R_q` is the queues' `MessageRetentionPeriod`: the larger of
//!   the two for standard queues, their SUM for FIFO (a FIFO message moved to its DLQ starts the
//!   period again).
//! - **Retention inputs** the deployer reports in the hand-over: the DynamoDB stream retention
//!   (24 h), board-fold's `MaximumRecordAgeInSeconds` and the timeout of EVERY writing function,
//!   both queues' `MessageRetentionPeriod` and type, the deny table's name and `TableArn`, the
//!   authorizer's `deny_cache_ttl`, the evidence for step 0 (i) and (ii) — the canary's entry points
//!   included — and for the authorizer's fail-closed clause (the "Deny table" section).
//! - **IAM — UNVERIFIED until kwv2's throwaway-account run.** Its own role, never board-fold's
//!   runtime role and never the relay purge's. On the board-fold table's ARN only (no index ARN):
//!   every mode `dynamodb:DescribeTable` and `dynamodb:Query`, a delete additionally
//!   `dynamodb:BatchWriteItem`; on the deny table's ARN `dynamodb:DescribeTable` and
//!   `dynamodb:GetItem`, in every mode, nothing else. No `GetItem`, `PutItem`, `UpdateItem`,
//!   `DeleteItem` or `Scan` on the board-fold table. **Pass criterion:** (i) a dry run, a bound
//!   `--yes` and a confirm (with `--expect-identity`) succeed under exactly these actions; (ii)
//!   without `BatchWriteItem` the dry run succeeds and `--yes` exits 1 with every `deleted` at 0;
//!   (iii) without `DescribeTable` on either table every mode exits 78 or 1 before any Query; (iv)
//!   CloudTrail shows no other DynamoDB action. Until that is reported, this list is a design, not a
//!   fact.
//! - **"Deleted" for the board-fold copy:** step 0's proof holds and the deny item is still in
//!   place (or 8b passed), pass 2 exited 0, the relay confirm after it exited 0, and step 8 found no
//!   new id. Outside it: this deployment's PITR, backups and logs (foundations' list), and the
//!   queues' messages, which carry the pseudonymous stream id and never content (≤ 14 d).
//!
//! # Deny table (6j6v.dbmd round 4b)
//!
//! board-fold's deletion fence, shared with the ticket authorizer: one DynamoDB table of the hosted
//! block, its NAME the deployer's, given to board-fold and `nxf-fold-purge` as `NXS_DENY_TABLE`
//! ([`deny::DENY_TABLE_ENV`]).
//!
//! | what | value |
//! |------|-------|
//! | key | `h` (S), the account's `<h>`: exactly 32 lowercase hex, the first 32 of SHA-256(`<issuer>:<sub>`), issuer byte for byte ([`deny::AccountHash::of_subject`], pinned by `contract/deny-derivation-vectors.json`); no sort key, no index (`ddb::create_deny_table`, checked by `ddb::deny_table_shape`) |
//! | item | `{"h": "<h>", "v": 1, "since": "<RFC 3339 UTC>"}`, one per deleted account; a reader judges PRESENCE only — any item under `<h>` is a denial, whatever `v` or other attributes it carries; no personal data, no expiry attribute (a TTL could unfence silently) |
//! | writer | kwv2 alone, `PutItem`, in step 0; T_w is when the `PutItem` was acknowledged |
//! | readers | the authorizer (`deny_cache_ttl` ≤ 60 s, hits and misses); board-fold before EVERY chunk, uncached ([`deny::admission`]); `nxf-fold-purge` once a run — all `GetItem` only |
//! | which ids | `u-<h>` exactly, and every id beginning `n-<h>-` ([`deny::account_of`]); the vector file `crates/fold-ddb/contract/deny-account-vectors.json` pins the mapping, and foundations run it against the authorizer |
//!
//! - **board-fold's rule** (owner decision 2026-10-23): `DynamoDbDenyList::from_env` at start — on
//!   `Err` it folds NOTHING; then `admission` before every chunk of the live fold, rescue, progress
//!   and backfill, never cached. It writes only on `Admitted` ([`deny::Admission::writes`]), and
//!   then only where `streams.admits` accepts too. `Denied`, `Unscoped` and an `Err` write nothing:
//!   the chunk errs (so it is redriven and lands in the DLQ) and logs the reason `deny_list` with the
//!   stream's `sid`. So a stack that runs board-fold with the deny table folds SCOPED ids only, and
//!   without it folds nothing at all.
//! - **Scoped ids only.** The fence can name only `u-<h>` and `n-<h>-…`. A product that promises
//!   deletion through this contract issues no other stream id, enforced where ids are issued
//!   (nexflow.it's `authorizeStream` does); "deleted" is defined for scoped ids only, and step A3
//!   and step 8 stop on any enumerated id outside the scoped forms.
//! - **`NXS_FOLD_STREAMS` is an operational allow-list only** (unset = every stream). No step of a
//!   deletion sets or reads it: Lambda's 4 KB of environment could never list every stream.
//! - **The authorizer fails closed** (consumer contract v2.1, binding on foundations' authorizer): a
//!   read error, or a deny table it cannot read or is not configured with, answers 503
//!   `deny_list_unavailable` and never allows; an error is never cached as a miss. The accepted
//!   consequence: a deny-table outage is an outage of all stream access. The 403 probe judges a 503
//!   as "inconclusive — stop", never as fenced. The hand-over carries the evidence: an authorizer
//!   test, or a kwv2 staging run with the table unreachable. The same holds here: a deny read that
//!   errs writes nothing.
//! - **Permanence.** The table is provisioned with deletion protection, a retain policy on stack
//!   removal and point-in-time recovery, and the stack documents its restore. An emptied, replaced
//!   or restored-too-early table would silently unfence every deleted account, and a table of the
//!   right shape passes `ddb::check_deny_table` however empty — so steps 5, 6 and 8 each read the
//!   deny item back (a consistent `GetItem`), and `nxf-fold-purge` reads it on every run.
//! - **Who deletes an item.** Only the unfence procedure of the runbook, under a break-glass role
//!   that alone has `DeleteItem` on the table, used by two people with the proof and the time
//!   recorded. Canary items are never deleted: they stay on the list for good.
//! - **An outage of the deny table** sends every chunk to the DLQ. Past the queues' retention those
//!   chunks are gone from the queues, not from the relay: the recovery is a backfill from the relay
//!   once the table answers again. (One denied account's chunk fails its whole change-stream batch
//!   until the rescue queue takes it; that one denied account delays no other for longer than that
//!   is foundations' to show, with a mixed-batch test.)
//! - **A deny item put between an `admission` and the write it admitted** is not seen by that chunk:
//!   that window is what step 0's wait covers (the largest function timeout), and the reason it is
//!   in the wait.

pub mod board;
pub mod chunks;
pub mod deny;
pub mod fold;
pub mod layout;
pub mod mem;
pub mod purge;
pub mod records;
pub mod snapshot;
pub mod table;
pub mod write;

#[cfg(feature = "dynamodb")]
pub mod ddb;
