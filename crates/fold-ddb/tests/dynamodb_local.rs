//! The DynamoDB adapter against a real DynamoDB Local (6j6v.k7w7): the same fold written into
//! DynamoDB and into a [`MemTable`] leaves the same items, the board reads back through the indexes
//! with the same lanes and the same `typed` listings (6j6v.xkr7), the ranked list and the ranking
//! graph read back as from memory — a hub whose adjacency spans Query pages among them (review of
//! PR #55) — and every request the client sends is recorded — none is a Scan.
//!
//! Gated like `nxs-server`'s suite: on the `dynamodb` feature and `NXF_RELAY_DDB_ENDPOINT`, skipped
//! locally without one and a hard failure under CI. Each test creates its own table from
//! [`create_table`], the contract a deploying stack provisions.
#![cfg(feature = "dynamodb")]

mod common;

use aws_sdk_dynamodb::config::interceptors::BeforeTransmitInterceptorContextRef;
use aws_sdk_dynamodb::config::{
    BehaviorVersion, ConfigBag, Credentials, Intercept, Region, RuntimeComponents,
};
use aws_sdk_dynamodb::error::BoxError;
use aws_sdk_dynamodb::types::TableStatus;
use aws_sdk_dynamodb::Client;
use common::{by_key, delivered, merged, NOW, TYPES};
use nexus_flow_core::model::EdgeKind;
use nexus_flow_core::store::Store;
use nxs_fold_ddb::ddb::{create_table, DynamoDbTable};
use nxs_fold_ddb::fold::Folder;
use nxs_fold_ddb::mem::MemTable;
use nxs_fold_ddb::snapshot;
use nxs_fold_ddb::table::{text, Dated, Table};
use nxs_fold_ddb::{board, records};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn endpoint(test: &str) -> Option<String> {
    match std::env::var("NXF_RELAY_DDB_ENDPOINT") {
        Ok(url) => Some(url),
        Err(_) => {
            assert!(
                std::env::var("CI").is_err(),
                "{test}: NXF_RELAY_DDB_ENDPOINT must be set under --features dynamodb in CI"
            );
            eprintln!("SKIP {test}: NXF_RELAY_DDB_ENDPOINT unset");
            None
        }
    }
}

/// Every request the client sends, by operation name (`X-Amz-Target`).
#[derive(Debug, Clone, Default)]
struct Recorder(Arc<Mutex<Vec<String>>>);

impl Recorder {
    fn take(&self) -> BTreeMap<String, u64> {
        let mut counts = BTreeMap::new();
        for op in self.0.lock().unwrap().drain(..) {
            *counts.entry(op).or_insert(0) += 1;
        }
        counts
    }
}

impl Intercept for Recorder {
    fn name(&self) -> &'static str {
        "nxf-request-recorder"
    }

    fn read_before_transmit(
        &self,
        context: &BeforeTransmitInterceptorContextRef<'_>,
        _: &RuntimeComponents,
        _: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let target = context
            .request()
            .headers()
            .get("x-amz-target")
            .unwrap_or("unknown");
        let op = target.rsplit('.').next().unwrap_or(target).to_string();
        self.0.lock().unwrap().push(op);
        Ok(())
    }
}

fn client(endpoint: &str, recorder: &Recorder) -> Client {
    let config = aws_sdk_dynamodb::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .endpoint_url(endpoint)
        .credentials_provider(Credentials::new(
            "test",
            "test",
            None,
            None,
            "dynamodb-local",
        ))
        .interceptor(recorder.clone())
        .build();
    Client::from_conf(config)
}

static TABLES: AtomicU64 = AtomicU64::new(0);

/// A fresh views table, created from the contract and polled until active.
async fn fresh_table(client: &Client, test: &str) -> String {
    let name = format!(
        "nxf_views_{test}_{}_{}",
        std::process::id(),
        TABLES.fetch_add(1, Ordering::SeqCst)
    );
    create_table(client, &name).send().await.unwrap();
    for _ in 0..100 {
        let d = client
            .describe_table()
            .table_name(&name)
            .send()
            .await
            .unwrap();
        if d.table().and_then(|t| t.table_status()) == Some(&TableStatus::Active) {
            return name;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("table {name} never became active");
}

/// An index read is eventually consistent: wait until `read` answers `want`.
async fn eventually<T: PartialEq + std::fmt::Debug, F: std::future::Future<Output = T>>(
    want: &T,
    mut read: impl FnMut() -> F,
) -> T {
    let mut got = read().await;
    for _ in 0..50 {
        if &got == want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        got = read().await;
    }
    got
}

#[tokio::test(flavor = "multi_thread")]
async fn a_board_folded_into_dynamodb_reads_back_as_from_memory_and_no_request_is_a_scan() {
    let Some(url) = endpoint("fold_and_read") else {
        return;
    };
    let recorder = Recorder::default();
    let client = client(&url, &recorder);
    let name = fresh_table(&client, "fold").await;
    let folder = Folder::platform().unwrap();

    for seed in [3, 8, 19, 27, 41, 55] {
        let s = merged(seed);
        let ops = delivered(&s, seed);
        let stream = format!("stream-{seed}");
        let ddb = DynamoDbTable::new(client.clone(), &name, &stream);
        let mem = MemTable::new(&stream);
        recorder.take();
        folder.fold_batch(&ddb, &ops).await.unwrap();
        let sent = recorder.take();
        folder.fold_batch(&mem, &ops).await.unwrap();

        // The same items, attribute for attribute — index attributes and adjacency included.
        let stored = by_key(ddb.query_prefix("").await.unwrap());
        assert_eq!(stored, mem.rows(), "seed {seed}");

        // The memory table's request counts are the real ones: what the crate doc's cost section
        // and the fan-out measurement below rest on.
        let counted = mem.requests();
        assert_eq!(
            sent.get("GetItem").copied().unwrap_or(0),
            counted.gets,
            "{sent:?}"
        );
        assert_eq!(
            sent.get("UpdateItem").copied().unwrap_or(0),
            counted.writes,
            "{sent:?}"
        );
        assert_eq!(
            sent.get("Query").copied().unwrap_or(0),
            counted.queries + counted.index_queries,
            "{sent:?}"
        );
        assert_eq!(
            sent.get("BatchGetItem").copied().unwrap_or(0),
            counted.batch_gets,
            "{sent:?}"
        );

        let want = board::select(&mem).await.unwrap().board.lanes(NOW);
        let ddb_ref = &ddb;
        let got = eventually(&want, || async move {
            board::select(ddb_ref).await.unwrap().board.lanes(NOW)
        })
        .await;
        assert_eq!(got, want, "lanes, seed {seed}");
        for lane in [Dated::Closed, Dated::Archived] {
            let ids = |rows: Vec<nxs_fold_ddb::table::Row>| -> Vec<String> {
                rows.iter()
                    .map(|r| text(r, "id").unwrap().to_string())
                    .collect()
            };
            for limit in [None, Some(1), Some(3)] {
                let want = ids(board::dated(&mem, lane, limit).await.unwrap());
                let got = eventually(&want, || async move {
                    ids(board::dated(ddb_ref, lane, limit).await.unwrap())
                })
                .await;
                assert_eq!(got, want, "{lane:?} limit {limit:?}, seed {seed}");
            }
        }
        // The `typed` index (6j6v.xkr7): each type's live items, in id order, through the SDK.
        for ty in TYPES {
            let ids = |rows: Vec<nxs_fold_ddb::table::Row>| -> Vec<String> {
                rows.iter()
                    .map(|r| text(r, "id").unwrap().to_string())
                    .collect()
            };
            let want = ids(mem.query_typed(ty).await.unwrap());
            let got = eventually(&want, || async move {
                ids(ddb_ref.query_typed(ty).await.unwrap())
            })
            .await;
            assert_eq!(got, want, "typed {ty}, seed {seed}");
        }
        // Two streams share the table without seeing each other: the second fold's partition
        // holds only its own rows (checked above), and its reads named its partition.
        let read = recorder.take();
        let all: Vec<&String> = sent.keys().chain(read.keys()).collect();
        assert!(
            all.iter()
                .all(|op| ["GetItem", "UpdateItem", "Query"].contains(&op.as_str())),
            "a fold and a read send only GetItem, UpdateItem and Query: {sent:?} {read:?}"
        );
        assert!(!all.iter().any(|op| op.as_str() == "Scan"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_a_ticket_with_many_dependents_costs_one_query_and_a_few_requests_per_edge() {
    let Some(url) = endpoint("fan_out") else {
        return;
    };
    let recorder = Recorder::default();
    let client = client(&url, &recorder);
    let name = fresh_table(&client, "fanout").await;
    let folder = Folder::platform().unwrap();

    let n = 40;
    let mut s = Store::open_in_memory(1);
    s.create_item("ab12.0000", "task", "Blocker", "u");
    for i in 1..=n {
        let id = format!("ab12.{i:04}");
        s.create_item(&id, "task", "Dependent", "u");
        s.add_edge(&id, "ab12.0000", EdgeKind::Dep, "u");
    }
    let ddb = DynamoDbTable::new(client.clone(), &name, "stream-fanout");
    let numbered = |ops: Vec<nxs_foundation::model::Op>, from: usize| -> Vec<(i64, _)> {
        ops.into_iter()
            .enumerate()
            .skip(from)
            .map(|(i, op)| (i as i64 + 1, op))
            .collect()
    };
    folder
        .fold_batch(&ddb, &numbered(s.export(), 0))
        .await
        .unwrap();
    let seen = s.export().len();
    s.set_field("ab12.0000", "status", Some("closed".into()), "u");
    recorder.take();
    folder
        .fold_batch(&ddb, &numbered(s.export(), seen))
        .await
        .unwrap();
    let sent = recorder.take();
    eprintln!("closing a ticket with {n} dependents sent {sent:?}");
    let count = |op: &str| sent.get(op).copied().unwrap_or(0);
    // The close: three changes and the watermark, one read to weigh the ticket's row first. The
    // ticket's refresh: one read, one write. The flip: one adjacency Query, and per edge three
    // reads — the edge, its remove, its first end, a dependent that is still active — and no
    // write, since that keeps the edge on the index.
    assert_eq!(count("Query"), 1, "{sent:?}");
    assert_eq!(count("GetItem"), 2 + 3 * n, "{sent:?}");
    assert_eq!(count("UpdateItem"), 3 + 1 + 1, "{sent:?}");
    assert_eq!(count("Scan"), 0);

    let ddb_ref = &ddb;
    let lanes = eventually(&(n as usize), || async move {
        board::select(ddb_ref)
            .await
            .unwrap()
            .board
            .lanes(NOW)
            .ready
            .len()
    })
    .await;
    assert_eq!(
        lanes, n as usize,
        "every dependent is ready once its blocker closed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_snapshot_taken_from_dynamodb_starts_another_stream_with_the_same_items() {
    let Some(url) = endpoint("snapshot") else {
        return;
    };
    let recorder = Recorder::default();
    let client = client(&url, &recorder);
    let name = fresh_table(&client, "snap").await;
    let folder = Folder::platform().unwrap();
    let s = merged(5);
    let ops = delivered(&s, 5);

    let source = DynamoDbTable::new(client.clone(), &name, "stream-a");
    folder.fold_batch(&source, &ops).await.unwrap();
    let mut snap = snapshot::export(&source, &folder).await.unwrap().unwrap();
    assert_eq!(snap.folded_through, ops.len() as i64);

    snap.stream_id = "stream-b".into();
    let started = DynamoDbTable::new(client.clone(), &name, "stream-b");
    snapshot::import(&started, &folder, &snap).await.unwrap();
    let strip = |rows: Vec<nxs_fold_ddb::table::Row>| {
        by_key(rows)
            .into_iter()
            .map(|(k, mut r)| {
                // The partition and the index attributes name their stream; everything else is the
                // same across the two.
                r.remove("stream_id");
                for attr in ["nxf_active", "nxf_dated", "nxf_typed"] {
                    if let Some(nxs_foundation::change::Cell::Text(v)) = r.get_mut(attr) {
                        *v = v
                            .replacen("stream-a", "<stream>", 1)
                            .replacen("stream-b", "<stream>", 1);
                    }
                }
                (k, r)
            })
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(
        strip(started.query_prefix("").await.unwrap()),
        strip(source.query_prefix("").await.unwrap())
    );
    assert!(!recorder.take().contains_key("Scan"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_board_larger_than_one_query_page_reads_back_whole() {
    let Some(url) = endpoint("pages") else { return };
    let recorder = Recorder::default();
    let client = client(&url, &recorder);
    let name = fresh_table(&client, "pages").await;
    let folder = Folder::platform().unwrap();
    // 300 active tickets with 8 KB descriptions: some 2.4 MB on the `active` index, where a Query
    // page stops at 1 MB — the reads must follow the pages to the end.
    let mut s = Store::open_in_memory(1);
    for i in 0..300 {
        let id = format!("ab12.{i:04}");
        s.create_item(&id, "task", "T", "u");
        s.set_field(&id, "description", Some("d".repeat(8 * 1024)), "u");
    }
    let ops: Vec<(i64, _)> = s
        .export()
        .into_iter()
        .enumerate()
        .map(|(i, op)| (i as i64 + 1, op))
        .collect();
    let ddb = DynamoDbTable::new(client.clone(), &name, "stream-pages");
    folder.fold_batch(&ddb, &ops).await.unwrap();
    recorder.take();
    let ddb_ref = &ddb;
    let ready = eventually(&300usize, || async move {
        board::select(ddb_ref)
            .await
            .unwrap()
            .board
            .lanes(NOW)
            .ready
            .len()
    })
    .await;
    assert_eq!(ready, 300);
    assert_eq!(
        ddb.query_prefix("items#").await.unwrap().len(),
        300,
        "the base table pages too"
    );
    let sent = recorder.take();
    assert!(
        sent.get("Query").copied().unwrap_or(0) >= 4,
        "more than one page per read: {sent:?}"
    );
    assert!(!sent.contains_key("Scan"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_records_read_back_as_from_memory_with_five_requests_per_ticket_and_no_scan() {
    // 6j6v.t1ym: `records::active_board` against DynamoDB — the key forms of the joins and the
    // entries under a ticket, read through the SDK — gives what it gives over the memory table,
    // and sends exactly the requests the crate doc counts.
    let Some(url) = endpoint("records") else {
        return;
    };
    let recorder = Recorder::default();
    let client = client(&url, &recorder);
    let name = fresh_table(&client, "records").await;
    let folder = Folder::platform().unwrap();

    for seed in [3, 19, 41] {
        let s = merged(seed);
        let ops = delivered(&s, seed);
        let stream = format!("stream-records-{seed}");
        let ddb = DynamoDbTable::new(client.clone(), &name, &stream);
        let mem = MemTable::new(&stream);
        folder.fold_batch(&ddb, &ops).await.unwrap();
        folder.fold_batch(&mem, &ops).await.unwrap();

        let want = records::active_board(&mem, board::select(&mem).await.unwrap())
            .await
            .unwrap();
        // The index is eventually consistent: wait for it to show the fold, then read once more
        // with the recorder clean.
        let want_ids: Vec<String> = want.tickets.keys().cloned().collect();
        let ddb_ref = &ddb;
        eventually(&want_ids, || async move {
            board::select(ddb_ref)
                .await
                .unwrap()
                .items
                .into_keys()
                .collect()
        })
        .await;
        let selection = board::select(&ddb).await.unwrap();
        let n = selection.items.len() as u64;
        let outside = selection
            .parents
            .values()
            .filter(|p| !selection.items.contains_key(*p))
            .collect::<std::collections::BTreeSet<_>>()
            .len() as u64;
        recorder.take();
        let got = records::active_board(&ddb, selection).await.unwrap();
        let sent = recorder.take();
        assert_eq!(got.tickets, want.tickets, "seed {seed}");

        let count = |op: &str| sent.get(op).copied().unwrap_or(0);
        assert_eq!(count("GetItem"), 1 + n + outside, "seed {seed}: {sent:?}");
        // Custom values, list entries (6j6v.4g6r), labels and links: one Query each per ticket.
        assert_eq!(count("Query"), 4 * n, "seed {seed}: {sent:?}");
        assert_eq!(count("Scan"), 0);
        assert!(
            sent.keys()
                .all(|op| ["GetItem", "Query"].contains(&op.as_str())),
            "a read sends only GetItem and Query: {sent:?}"
        );
    }
}

/// The reads of a ranked list of `types` (`None`: every type) whose keys follow every edge kind,
/// read labels and custom values, the `next` board and the `list` scope of `list_types`.
fn ranked_reads(types: Option<&[&str]>, list_types: &[&str]) -> nexus_flow_facade::read::ListReads {
    let mut graph = nexus_flow_facade::read::RankingReads::default();
    graph.kinds = [
        EdgeKind::Parent,
        EdgeKind::ContributesTo,
        EdgeKind::Mentions,
        EdgeKind::Dep,
    ]
    .into_iter()
    .collect();
    graph.labels = true;
    graph.custom = true;
    graph.list_types = list_types.iter().map(|t| t.to_string()).collect();
    let mut reads = nexus_flow_facade::read::ListReads::default();
    reads.types = types.map(|t| t.iter().map(|t| t.to_string()).collect());
    reads.next = true;
    reads.ranking = Some(graph);
    reads
}

/// What a list board holds, comparably.
type ListView = (
    BTreeMap<String, nexus_flow_facade::read::ListItem>,
    Option<nexus_flow_facade::read::RankingGraph>,
    Option<BTreeMap<String, nexus_flow_facade::read::ActiveTicket>>,
);

async fn list_view<T: Table>(t: &T, reads: &nexus_flow_facade::read::ListReads) -> ListView
where
    T::Error: std::fmt::Debug,
{
    let board = records::list_board(t, reads).await.unwrap();
    (board.items, board.ranking, board.active.map(|a| a.tickets))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_ranked_list_and_the_ranking_graph_read_back_as_from_memory() {
    // Review of PR #55, Integrity #7: `records::list_board` and `records::with_ranking` through the
    // SDK — the `typed` Query joined with base-table reads, `BatchGetItem` — give what they give
    // over the memory table, and send no Scan.
    let Some(url) = endpoint("ranked") else {
        return;
    };
    let recorder = Recorder::default();
    let client = client(&url, &recorder);
    let name = fresh_table(&client, "ranked").await;
    let folder = Folder::platform().unwrap();
    for seed in [3, 19, 41] {
        let s = merged(seed);
        let ops = delivered(&s, seed);
        let stream = format!("stream-ranked-{seed}");
        let ddb = DynamoDbTable::new(client.clone(), &name, &stream);
        let mem = MemTable::new(&stream);
        folder.fold_batch(&ddb, &ops).await.unwrap();
        folder.fold_batch(&mem, &ops).await.unwrap();
        let mut all = vec![ranked_reads(None, &[])];
        for ty in TYPES {
            let others: Vec<&str> = TYPES.into_iter().filter(|t| *t != ty).collect();
            all.push(ranked_reads(Some(&[ty]), &others));
        }
        for reads in &all {
            let want = list_view(&mem, reads).await;
            let ddb_ref = &ddb;
            let got = eventually(&want, || async move { list_view(ddb_ref, reads).await }).await;
            assert_eq!(got, want, "seed {seed}, {:?}", reads.types);
        }
        let graph = ranked_reads(None, &["epic"]).ranking.unwrap();
        let plain = records::active_board(&mem, board::select(&mem).await.unwrap())
            .await
            .unwrap();
        let want = records::with_ranking(&mem, plain.clone(), &graph)
            .await
            .unwrap()
            .ranking;
        let ddb_ref = &ddb;
        let (plain_ref, graph_ref) = (&plain, &graph);
        let got = eventually(&want, || async move {
            records::with_ranking(ddb_ref, plain_ref.clone(), graph_ref)
                .await
                .unwrap()
                .ranking
        })
        .await;
        assert_eq!(got, want, "seed {seed}: the ranking graph");
        let sent = recorder.take();
        assert!(sent.contains_key("BatchGetItem"), "{sent:?}");
        assert!(!sent.contains_key("Scan"), "{sent:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hub_whose_adjacency_spans_query_pages_reads_every_edge() {
    // Review of PR #55, Integrity #7: an item with more adjacency entries than one Query page holds
    // (1 MB) — 450 contributors with ids near the key limit, some 2.9 KB an entry — reads every edge
    // through the SDK, as from memory.
    let Some(url) = endpoint("hub") else { return };
    let recorder = Recorder::default();
    let client = client(&url, &recorder);
    let name = fresh_table(&client, "hub").await;
    let folder = Folder::platform().unwrap();
    let hub = format!("ab12.h{}", "h".repeat(890));
    let mut s = Store::open_in_memory(1);
    s.create_item(&hub, "epic", "Hub", "u");
    let n = 450;
    for i in 0..n {
        let id = format!("ab12.{i:04}{}", "n".repeat(890));
        s.create_item(&id, "task", "Contributor", "u");
        s.add_edge(&id, &hub, EdgeKind::ContributesTo, "u");
    }
    let ops: Vec<(i64, _)> = s
        .export()
        .into_iter()
        .enumerate()
        .map(|(i, op)| (i as i64 + 1, op))
        .collect();
    let ddb = DynamoDbTable::new(client.clone(), &name, "stream-hub");
    let mem = MemTable::new("stream-hub");
    assert!(folder.fold_batch(&ddb, &ops).await.unwrap().is_empty());
    folder.fold_batch(&mem, &ops).await.unwrap();

    recorder.take();
    let entries = ddb.query_prefix(&format!(".adj#{hub}#")).await.unwrap();
    let sent = recorder.take();
    assert_eq!(entries.len(), n);
    assert!(
        sent.get("Query").copied().unwrap_or(0) >= 2,
        "the hub's entries span pages: {sent:?}"
    );

    let mut graph = nexus_flow_facade::read::RankingReads::default();
    graph.kinds.insert(EdgeKind::ContributesTo);
    let plain = records::active_board(&mem, board::select(&mem).await.unwrap())
        .await
        .unwrap();
    let want = records::with_ranking(&mem, plain.clone(), &graph)
        .await
        .unwrap()
        .ranking
        .unwrap();
    assert_eq!(want.edges.len(), n);
    let got = records::with_ranking(&ddb, plain, &graph)
        .await
        .unwrap()
        .ranking
        .unwrap();
    assert_eq!(got, want);

    // The hub listed by its type: its edges come from the same paged entries.
    let reads = ranked_reads(Some(&["epic"]), &[]);
    let want = list_view(&mem, &reads).await;
    let ddb_ref = &ddb;
    let reads_ref = &reads;
    let got = eventually(&want, || async move { list_view(ddb_ref, reads_ref).await }).await;
    assert_eq!(got, want);
    assert_eq!(got.1.unwrap().edges.len(), n);
}

/// A neutral test plugin declaring the shared generator's `owner` and `members` as ref fields
/// (6j6v.9tx3), so a `list --ref` can be asked of a generated board.
const REFS: &str = r#"
name = "dynamodb-local-ref-fixture"
[description]
en = "x"
de = "y"
[priority]
labels = ["P0", "P1", "P2", "P3", "P4"]
[types]
list = ["task", "epic", "note"]
[vocabulary.status]
open = "open"
in_progress = "in progress"
closed = "closed"
[ranking.next]
order = [ { field = "priority", dir = "asc" } ]
[presentation.list]
columns = ["id"]
[fields.owner]
type = "ref"
targets = ["task", "epic", "note"]
on = ["task", "epic", "note"]
[fields.members]
type = "ref"
targets = ["task", "epic", "note"]
list = true
on = ["task", "epic", "note"]
"#;

inventory::submit!(nexus_flow_facade::plugin::PluginRegistration {
    name: "dynamodb-local-ref-fixture",
    toml: REFS,
    order: 143,
});

#[tokio::test(flavor = "multi_thread")]
async fn a_list_ref_query_reads_back_from_dynamodb_as_the_replica_answers_it() {
    // 6j6v.1w3m: the hints folded into DynamoDB, read by `records::referrers` (a base-table Query)
    // and the candidates' rows by `BatchGetItem` — strongly consistent, no index — answer
    // `list --ref` as the SQL path does.
    use nexus_flow_facade::read::{self, ListQuery};
    let Some(url) = endpoint("list_ref") else {
        return;
    };
    let recorder = Recorder::default();
    let client = client(&url, &recorder);
    let name = fresh_table(&client, "listref").await;
    let folder = Folder::platform().unwrap();
    let cfg = nexus_flow_facade::plugin::load("dynamodb-local-ref-fixture").unwrap();
    let mut answered = 0;
    for seed in [3, 19, 41] {
        let s = merged(seed);
        let ops = delivered(&s, seed);
        let stream = format!("stream-ref-{seed}");
        let ddb = DynamoDbTable::new(client.clone(), &name, &stream);
        folder.fold_batch(&ddb, &ops).await.unwrap();
        recorder.take();
        for i in 0..4 {
            let p = common::id(i);
            for q in [
                ListQuery::new().referencing("owner", &p),
                ListQuery::new()
                    .referencing("members", &p)
                    .referencing("owner", &p),
            ] {
                let want = read::list_page(&cfg, &s, &q).unwrap();
                let reads = read::list_reads(&cfg, &q);
                let board = records::list_board(&ddb, &reads).await.unwrap();
                let got = read::list_board_page(&cfg, &board, &q).unwrap();
                let ids = |items: &[nexus_flow_core::model::ItemRow]| -> Vec<String> {
                    items.iter().map(|i| i.id.clone()).collect()
                };
                assert_eq!(ids(&got.items), ids(&want.items), "seed {seed}, {p}");
                assert_eq!(got.total, want.total, "seed {seed}, {p}");
                answered += want.items.len();
            }
        }
        let sent = recorder.take();
        assert!(!sent.contains_key("Scan"), "{sent:?}");
    }
    assert!(answered > 0, "the generated boards hold references");
}
