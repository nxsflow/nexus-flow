//! DynamoDB `OpStore` + `PrefixRegistry` acceptance suite —
//! mirrors `postgres_parity.rs` in shape: mostly the same skip/hard-fail-under-CI harness
//! pattern, but the `OpStore` half doesn't have a second backend to diff against (that's
//! what `postgres_parity.rs` does), so those tests assert the DynamoDB backend's own
//! contract directly instead of a cross-backend trace comparison. The `PrefixRegistry` half
//! DOES diff against SQLite (`registry_parity_dynamodb_matches_sqlite`), because
//! `candidate_prefix` reuse only proves cross-backend parity if something actually compares
//! the two traces. The concurrency tests — `concurrent_appends_get_unique_contiguous_sequences`
//! and `registry_concurrent_collision_grants_exactly_one_dynamodb` /
//! `concurrent_registers_by_the_same_replica_converge_on_one_stable_answer_dynamodb`
//! — hammer the one invariant each trait names on the real service, no client-side mutex
//! hiding the question, the same risk `postgres_parity.rs`'s own concurrency tests hammer
//! for Postgres. `the_wired_dynamodb_backend_serves_the_real_http_surface` closes
//! the one gap the tests above leave: every test up to that point calls `OpStore`/
//! `PrefixRegistry` methods directly, never through `app()`'s axum handlers — the exact
//! composition `backends()` (`crates/server/src/main.rs`) performs at boot.
//!
//! ## Test harness decisions
//!
//! * Gated on the `dynamodb` cargo feature AND `NXF_RELAY_DDB_ENDPOINT`, same rationale as
//!   the Postgres suite's `DATABASE_URL` gate: `cargo test` stays green with zero AWS setup
//!   locally, but a dropped endpoint can never silently pass in CI (`CI` env set => hard
//!   failure on a missing endpoint, never a skip).
//! * Per-test isolation is a **unique table name per test** for EACH table (ops and
//!   registry have independent key schemas, so separate `CreateTable` shapes), created here
//!   and polled until `ACTIVE`. Production code (`store_ddb.rs`) never creates a table — the
//!   table is externally provisioned by the deploying stack (its required shape is
//!   documented at the top of `store_ddb.rs`) — so table lifecycle is the harness's job
//!   alone.
//! * DynamoDB Local accepts any credentials but the SDK still requires *something* to be
//!   present in the credential chain; a `Once` sets fake `AWS_*` env vars before the first
//!   client is built, exactly once, so parallel tests never race on process env.
#![cfg(feature = "dynamodb")]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Once};
use std::thread;
use std::time::Duration;

use aws_sdk_dynamodb::types::{
    AttributeDefinition, AttributeValue, BillingMode, KeySchemaElement, KeyType,
    ScalarAttributeType, TableStatus,
};
use aws_sdk_dynamodb::Client;

use nxs_server::app::app;
use nxs_server::presence::PresenceStore;
use nxs_server::registry::{PrefixRegistry, SqlitePrefixRegistry};
use nxs_server::store::OpStore;
use nxs_server::store_ddb::{DynamoDbOpStore, DynamoDbPrefixRegistry};
use nxs_sync::protocol::{
    Cursor, MachineHello, MachinesResponse, PullResponse, PushResponse, RegisterOutcome, StreamId,
};
use nxs_sync::wire::{WireOp, ENVELOPE_VERSION};

// ----- harness -----------------------------------------------------------------------

/// Returns the DynamoDB Local endpoint, or `None` (with a loud skip) when
/// `NXF_RELAY_DDB_ENDPOINT` is unset. See `postgres_parity.rs::base_url` for the identical
/// local-skip / CI-hard-fail rationale.
fn endpoint(test: &str) -> Option<String> {
    match std::env::var("NXF_RELAY_DDB_ENDPOINT") {
        Ok(url) => Some(url),
        Err(_) => {
            assert!(
                std::env::var("CI").is_err(),
                "{test}: NXF_RELAY_DDB_ENDPOINT must be set under --features dynamodb in CI — \
                 the DynamoDB Local suite must fail loud, not silently skip"
            );
            eprintln!(
                "SKIP {test}: NXF_RELAY_DDB_ENDPOINT unset (set it to run the DynamoDB Local suite)"
            );
            None
        }
    }
}

/// DynamoDB Local takes any credentials, but the SDK's credential chain still needs some
/// value present or client construction fails looking for a real provider. Set once, before
/// the first client is built in the process, so concurrent tests never race on process env.
fn ensure_fake_credentials() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::env::set_var("AWS_ACCESS_KEY_ID", "test");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "test");
        std::env::set_var("AWS_REGION", "us-east-1");
    });
}

/// A dedicated runtime for the harness's own admin calls (CreateTable/DescribeTable) —
/// separate from `store_ddb`'s internal bridge runtime, which backs the `DynamoDbOpStore`
/// under test. The two never interact; this one exists purely so plain `#[test]` fns (no
/// tokio context on the calling thread, matching how the relay calls into the store) can
/// still drive the async admin SDK calls needed to set up each test's table.
fn admin_rt() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("build admin runtime"))
}

fn admin_client(endpoint: &str) -> Client {
    ensure_fake_credentials();
    admin_rt().block_on(async {
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .endpoint_url(endpoint)
            .load()
            .await;
        Client::new(&config)
    })
}

/// A fresh, process-unique ops-table name so parallel tests never collide, then create it
/// (PAY_PER_REQUEST) and wait for `ACTIVE` before handing control back. Key schema per the
/// table contract at the top of `store_ddb.rs`: PK `stream_id` (S), SK `seq` (N).
fn unique_ops_table(endpoint: &str) -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let table = format!(
        "nxf_ddb_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    );
    let client = admin_client(endpoint);
    admin_rt().block_on(async {
        client
            .create_table()
            .table_name(&table)
            .billing_mode(BillingMode::PayPerRequest)
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name("stream_id")
                    .key_type(KeyType::Hash)
                    .build()
                    .expect("build hash key schema"),
            )
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name("seq")
                    .key_type(KeyType::Range)
                    .build()
                    .expect("build range key schema"),
            )
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name("stream_id")
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .expect("build stream_id attribute definition"),
            )
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name("seq")
                    .attribute_type(ScalarAttributeType::N)
                    .build()
                    .expect("build seq attribute definition"),
            )
            .send()
            .await
            .expect("create test ops table");

        loop {
            let resp = client
                .describe_table()
                .table_name(&table)
                .send()
                .await
                .expect("describe test ops table");
            if resp.table().and_then(|t| t.table_status()) == Some(&TableStatus::Active) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    table
}

fn opstore(endpoint: &str) -> DynamoDbOpStore {
    ensure_fake_credentials();
    let table = unique_ops_table(endpoint);
    DynamoDbOpStore::new(table, Some(endpoint.to_string()), None)
}

/// A fresh, process-unique registry-table name so parallel tests never collide, then create
/// it (PAY_PER_REQUEST) and wait for `ACTIVE`. Key schema per the table contract at the top
/// of `store_ddb.rs`: PK `stream_id` (S), SK `sk` (S) — a different range key from the ops
/// table's `seq`, and a different DynamoDB type, so this is its own
/// `CreateTable` shape rather than a reuse of `unique_ops_table`.
fn unique_registry_table(endpoint: &str) -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let table = format!(
        "nxf_ddb_reg_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    );
    let client = admin_client(endpoint);
    admin_rt().block_on(async {
        client
            .create_table()
            .table_name(&table)
            .billing_mode(BillingMode::PayPerRequest)
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name("stream_id")
                    .key_type(KeyType::Hash)
                    .build()
                    .expect("build hash key schema"),
            )
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name("sk")
                    .key_type(KeyType::Range)
                    .build()
                    .expect("build range key schema"),
            )
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name("stream_id")
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .expect("build stream_id attribute definition"),
            )
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name("sk")
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .expect("build sk attribute definition"),
            )
            .send()
            .await
            .expect("create test registry table");

        loop {
            let resp = client
                .describe_table()
                .table_name(&table)
                .send()
                .await
                .expect("describe test registry table");
            if resp.table().and_then(|t| t.table_status()) == Some(&TableStatus::Active) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    table
}

fn registry(endpoint: &str) -> DynamoDbPrefixRegistry {
    ensure_fake_credentials();
    let table = unique_registry_table(endpoint);
    DynamoDbPrefixRegistry::new(table, Some(endpoint.to_string()), None)
}

fn wire(op_id: &str) -> WireOp {
    WireOp {
        envelope_version: ENVELOPE_VERSION,
        op_id: op_id.into(),
        lamport: 1,
        site: 1,
        domain: "task".into(),
        target_kind: "item".into(),
        target_id: "ab12.0001".into(),
        field: "title".into(),
        op_type: "set".into(),
        value: Some("t".into()),
        author: "a".into(),
        wall_clock: String::new(),
        extra: Default::default(),
    }
}

// ----- tests ---------------------------------------------------------------------------

#[test]
fn append_read_since_round_trips_a_batch_in_order() {
    let Some(ep) = endpoint("append_read_since_round_trips_a_batch_in_order") else {
        return;
    };
    let store = opstore(&ep);
    let s = StreamId("s".into());
    for (i, id) in ["o1", "o2", "o3", "o4", "o5"].into_iter().enumerate() {
        let cursor = store.append(&s, &wire(id)).unwrap();
        assert_eq!(cursor, Cursor((i + 1) as i64));
    }
    let (ops, next) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
    assert_eq!(
        ops.iter().map(|o| o.op_id.as_str()).collect::<Vec<_>>(),
        ["o1", "o2", "o3", "o4", "o5"],
        "ops come back in seq order"
    );
    assert_eq!(next, Cursor(5));
}

#[test]
fn re_appending_the_same_op_id_stores_it_again_under_a_fresh_seq() {
    // PINS A DOCUMENTED DIVERGENCE, not a desirable property. SQLite and Postgres carry
    // UNIQUE (stream_id, op_id) and so make a re-pushed op idempotent: it returns its
    // EXISTING cursor and the log neither grows nor gaps (spec §4.4). DynamoDB has no
    // secondary unique constraint and this table has no op-id index, so the same push lands
    // twice. Convergence is unaffected (the engine folds an op-log idempotently by op_id),
    // but the log grows — see the module doc in store_ddb.rs and nexus-flow 4qjw.
    //
    // The test exists so the divergence cannot change silently in EITHER direction: if
    // dedupe is ever added, this fails and the doc + 4qjw must be updated with it.
    let Some(ep) = endpoint("re_appending_the_same_op_id_stores_it_again_under_a_fresh_seq") else {
        return;
    };
    let store = opstore(&ep);
    let s = StreamId("s".into());

    let first = store.append(&s, &wire("dup")).unwrap();
    let again = store.append(&s, &wire("dup")).unwrap();
    assert_eq!(first, Cursor(1));
    assert_eq!(
        again,
        Cursor(2),
        "no op-id dedupe on this backend: the re-push consumes a fresh seq"
    );

    let (ops, next) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
    assert_eq!(ops.len(), 2, "the op is stored twice, not once");
    assert!(
        ops.iter().all(|o| o.op_id == "dup"),
        "both stored items are the same op"
    );
    assert_eq!(next, Cursor(2));
}

#[test]
fn an_op_over_dynamodbs_400kb_item_ceiling_fails_loudly() {
    // The module doc claims an over-400 KB item is rejected by DynamoDB and surfaces as a
    // StoreError — a real Phase-1 constraint the SQLite and Postgres backends do not share
    // (app.rs accepts a push body up to MAX_PUSH_BYTES = 8 MB, far above this). Pin it, so
    // "fails loudly" is a tested claim rather than a comment: a silent truncation or a
    // swallowed error here would lose an op.
    let Some(ep) = endpoint("an_op_over_dynamodbs_400kb_item_ceiling_fails_loudly") else {
        return;
    };
    let store = opstore(&ep);
    let s = StreamId("s".into());

    let mut oversized = wire("huge");
    oversized.value = Some("x".repeat(420 * 1024));
    let err = store
        .append(&s, &oversized)
        .expect_err("an item past DynamoDB's 400 KB ceiling must not be silently accepted");
    assert!(
        err.to_string().to_lowercase().contains("size"),
        "the error names the size limit rather than being opaque: {err}"
    );

    // The failure is confined to the oversized op: the stream still works afterwards. (The
    // seq it consumed is gone — allocate_seq runs before the put — which is exactly the
    // "gaps are normal in production" property the module doc documents.)
    store.append(&s, &wire("after")).unwrap();
    let (ops, _) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
    assert_eq!(
        ops.iter().map(|o| o.op_id.as_str()).collect::<Vec<_>>(),
        ["after"],
        "the oversized op was never stored, and the stream is still usable"
    );
}

#[test]
fn opstore_preserves_envelope_verbatim_dynamodb() {
    // Server opacity: a future op (unknown op_type/target_kind, a
    // non-default envelope_version, a foreign domain, a null value) round-trips unchanged.
    let Some(ep) = endpoint("opstore_preserves_envelope_verbatim_dynamodb") else {
        return;
    };
    let store = opstore(&ep);
    let s = StreamId("s".into());
    let mut op = wire("o1");
    op.envelope_version = 999;
    op.op_type = "future_set".into();
    op.target_kind = "gizmo".into();
    op.domain = "fact".into();
    op.value = None;
    store.append(&s, &op).unwrap();
    let (ops, _) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
    assert_eq!(
        ops,
        vec![op],
        "an unknown-shape op is stored and returned verbatim"
    );
}

#[test]
fn an_unknown_envelope_field_survives_the_relay_dynamodb() {
    // 6j6v.5crb, per backend (the DoD names all three): the field a build does not know rides in
    // its own `extra` attribute here, omitted when there is nothing to carry. The test above
    // covers unknown VALUES in known attributes; this one covers an unknown FIELD, which used to
    // be dropped by every backend because each rebuilds the envelope from its own named columns.
    let Some(ep) = endpoint("an_unknown_envelope_field_survives_the_relay_dynamodb") else {
        return;
    };
    let store = opstore(&ep);
    let s = StreamId("s".into());

    let mut raw = serde_json::to_value(wire("future-1")).unwrap();
    raw["sig"] = serde_json::Value::String("ed25519:deadbeef".into());
    let from_a_newer_client: WireOp = serde_json::from_value(raw.clone()).unwrap();
    store.append(&s, &from_a_newer_client).unwrap();

    let (ops, _) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
    assert_eq!(ops, vec![from_a_newer_client]);
    assert_eq!(
        serde_json::to_value(&ops[0]).unwrap(),
        raw,
        "DynamoDB serves back the payload it was handed, unknown field and all"
    );

    // An op that carries nothing unknown stores no `extra` attribute at all, so a table written
    // before this landed and one written after are the same shape for the same op.
    store.append(&s, &wire("plain-1")).unwrap();
    let (ops, _) = store.read_since(&s, Cursor(1), 10).unwrap();
    assert!(ops[0].extra.is_empty(), "no attribute, no catch-all");
    assert_eq!(ops[0], wire("plain-1"));
}

#[test]
fn an_op_carrying_nothing_unknown_writes_no_extra_attribute_at_all_dynamodb() {
    // At the ATTRIBUTE level, not through the round trip (PR #314 review, Test Quality #2). Read
    // back through `OpStore`, an omitted attribute and a stored `"{}"` are indistinguishable —
    // `decode_extra` turns both into an empty map — so the round-trip test above would stay green
    // if this regressed to writing `extra` on every op. It matters because that is what keeps an
    // item written by a pre-`extra` build and one written now byte-identical for the same op, and
    // because every stored attribute counts against DynamoDB's 400 KB item ceiling.
    let Some(ep) = endpoint("an_op_carrying_nothing_unknown_writes_no_extra_attribute_at_all")
    else {
        return;
    };
    ensure_fake_credentials();
    let table = unique_ops_table(&ep);
    let store = DynamoDbOpStore::new(table.clone(), Some(ep.clone()), None);
    let s = StreamId("s".into());
    store.append(&s, &wire("plain-1")).unwrap();

    let mut raw = serde_json::to_value(wire("signed-1")).unwrap();
    raw["sig"] = serde_json::Value::String("ed25519:deadbeef".into());
    store
        .append(&s, &serde_json::from_value::<WireOp>(raw).unwrap())
        .unwrap();

    let admin = admin_client(&ep);
    let item = |seq: &str| {
        admin_rt().block_on(async {
            admin
                .get_item()
                .table_name(&table)
                .key("stream_id", AttributeValue::S("s".into()))
                .key("seq", AttributeValue::N(seq.into()))
                .send()
                .await
                .expect("get_item")
                .item
                .expect("the op item exists")
        })
    };
    assert!(
        !item("1").contains_key("extra"),
        "an op with nothing unknown writes no `extra` attribute — not an empty one"
    );
    assert_eq!(
        item("2").get("extra").and_then(|v| v.as_s().ok()),
        Some(&r#"{"sig":"ed25519:deadbeef"}"#.to_string()),
        "and an op that DOES carry something stores exactly it"
    );
}

#[test]
fn read_since_returns_only_ops_after_the_cursor_and_paginates() {
    let Some(ep) = endpoint("read_since_returns_only_ops_after_the_cursor_and_paginates") else {
        return;
    };
    let store = opstore(&ep);
    let s = StreamId("s".into());
    for id in ["o1", "o2", "o3"] {
        store.append(&s, &wire(id)).unwrap();
    }
    let (ops, next) = store.read_since(&s, Cursor(1), 10).unwrap();
    assert_eq!(
        ops.iter().map(|o| o.op_id.as_str()).collect::<Vec<_>>(),
        ["o2", "o3"],
        "only ops after the cursor come back"
    );
    assert_eq!(next, Cursor(3));

    // Now paginate two more streams' worth of ops, two at a time, following next-cursor.
    for id in ["o4", "o5"] {
        store.append(&s, &wire(id)).unwrap();
    }
    let mut cursor = Cursor::BEGINNING;
    let mut seen = Vec::new();
    loop {
        let (ops, next) = store.read_since(&s, cursor, 2).unwrap();
        if ops.is_empty() {
            break;
        }
        assert!(ops.len() <= 2, "page never exceeds the limit");
        seen.extend(ops.iter().map(|o| o.op_id.clone()));
        cursor = next;
    }
    assert_eq!(seen, ["o1", "o2", "o3", "o4", "o5"]);
}

#[test]
fn read_since_on_exhausted_stream_returns_empty_and_unchanged_cursor() {
    let Some(ep) = endpoint("read_since_on_exhausted_stream_returns_empty_and_unchanged_cursor")
    else {
        return;
    };
    let store = opstore(&ep);
    let s = StreamId("s".into());
    store.append(&s, &wire("o1")).unwrap();
    let (ops, next) = store.read_since(&s, Cursor(1), 10).unwrap();
    assert!(ops.is_empty());
    assert_eq!(next, Cursor(1), "no ops -> cursor does not move");
}

#[test]
fn sequences_are_independent_per_stream_dynamodb() {
    let Some(ep) = endpoint("sequences_are_independent_per_stream_dynamodb") else {
        return;
    };
    let store = opstore(&ep);
    let a = StreamId("a".into());
    let b = StreamId("b".into());
    assert_eq!(store.append(&a, &wire("a1")).unwrap(), Cursor(1));
    assert_eq!(store.append(&a, &wire("a2")).unwrap(), Cursor(2));
    // A different stream starts its own sequence at 1, independent of stream a's cursor.
    assert_eq!(store.append(&b, &wire("b1")).unwrap(), Cursor(1));
}

#[test]
fn value_some_empty_string_round_trips_as_some_empty_not_none() {
    // An empty string is a legitimate Some(""); it must not collapse to None on either
    // the write side (attribute omission) or the read side (absent-attribute handling).
    let Some(ep) = endpoint("value_some_empty_string_round_trips_as_some_empty_not_none") else {
        return;
    };
    let store = opstore(&ep);
    let s = StreamId("s".into());
    let mut op = wire("o1");
    op.value = Some(String::new());
    store.append(&s, &op).unwrap();
    let (ops, _) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
    assert_eq!(ops[0].value, Some(String::new()));
}

#[test]
fn read_since_fills_a_page_across_dynamodbs_1mb_query_cap() {
    // DynamoDB stops a `Query` at 1 MB of READ DATA and hands back fewer items than `Limit`
    // plus a `LastEvaluatedKey` — SQL `LIMIT n` returns exactly n rows when n match, so this
    // is the one place the backends can diverge observably. It matters because the client's
    // pull loop (`crates/sync/src/engine.rs`) reads a short page as "stream exhausted" and
    // stops: drop the key and a whole sync pass silently under-delivers. Ops this size are
    // the expected case, not a pathology — the prose body rides the same LWW-longtext
    // substrate field. 40 x ~30 KB is ~1.2 MB: comfortably over the cap, and each item is
    // well under DynamoDB's 400 KB per-item ceiling.
    let Some(ep) = endpoint("read_since_fills_a_page_across_dynamodbs_1mb_query_cap") else {
        return;
    };
    let store = opstore(&ep);
    let s = StreamId("s".into());
    const OPS: usize = 40;
    let big = "x".repeat(30 * 1024);
    for i in 0..OPS {
        let mut op = wire(&format!("o{i}"));
        op.value = Some(big.clone());
        store.append(&s, &op).unwrap();
    }

    let (ops, next) = store.read_since(&s, Cursor::BEGINNING, OPS).unwrap();
    assert_eq!(
        ops.len(),
        OPS,
        "a requested page must be filled even when DynamoDB's 1 MB Query cap splits it \
         across several round-trips"
    );
    assert_eq!(
        ops.iter().map(|o| o.op_id.as_str()).collect::<Vec<_>>(),
        (0..OPS).map(|i| format!("o{i}")).collect::<Vec<_>>(),
        "and the continued pages stay in seq order with no gaps or repeats"
    );
    assert_eq!(next, Cursor(OPS as i64));
}

// ----- atomic per-stream seq under concurrent writers --------------------------------

#[test]
fn concurrent_appends_get_unique_contiguous_sequences() {
    // THE named risk (store.rs:45-48): atomic, per-stream, monotone seq under concurrent
    // writers — on the real DynamoDB Local service, no client-side mutex hiding the
    // question. Shape copied from
    // postgres_parity.rs::opstore_concurrent_appends_get_unique_contiguous_sequences_postgres.
    let Some(ep) = endpoint("concurrent_appends_get_unique_contiguous_sequences") else {
        return;
    };
    let store = Arc::new(opstore(&ep));
    let s = StreamId("s".into());
    const THREADS: i64 = 8;
    const PER: i64 = 50;

    let mut handles = Vec::new();
    for t in 0..THREADS {
        let store = Arc::clone(&store);
        let s = s.clone();
        handles.push(thread::spawn(move || {
            (0..PER)
                .map(|i| store.append(&s, &wire(&format!("o{t}-{i}"))).unwrap().0)
                .collect::<Vec<_>>()
        }));
    }
    let mut all: Vec<i64> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    all.sort_unstable();
    let expected: Vec<i64> = (1..=THREADS * PER).collect();
    // CONTIGUITY IS NOT A BACKEND GUARANTEE — do not read this assertion as a contract. The
    // backend guarantees uniqueness and monotonicity; density is a happy accident of the test
    // environment. `allocate_seq`'s `UpdateItem` + `ADD` is not idempotent, so any retry of
    // that call — including one the SDK performs internally after a throttle or a lost
    // response — burns a seq that no op will ever occupy. Gaps are therefore normal against
    // real AWS. This holds here only because DynamoDB Local never throttles, and asserting
    // the stronger property is deliberate: it is the sharpest available detector of the
    // duplicate/overwrite bug this test exists for.
    assert_eq!(
        all, expected,
        "sequences are unique and contiguous under contention"
    );

    // The returned cursors are only half the claim — prove what actually landed in the
    // table agrees: read the whole stream back and check the same three properties
    // (contiguous, gap-free, duplicate-free) plus no duplicate op_id, so a bug where
    // `append` reports a cursor that doesn't match what got written would still be caught.
    let mut cursor = Cursor::BEGINNING;
    let mut seqs = Vec::new();
    let mut op_ids = std::collections::HashSet::new();
    loop {
        let (ops, next) = store.read_since(&s, cursor, 64).unwrap();
        if ops.is_empty() {
            break;
        }
        for op in &ops {
            assert!(
                op_ids.insert(op.op_id.clone()),
                "duplicate op_id read back: {}",
                op.op_id
            );
        }
        seqs.push(next.0);
        cursor = next;
    }
    assert_eq!(
        op_ids.len(),
        (THREADS * PER) as usize,
        "400 distinct ops stored"
    );
    // `read_since` only hands back the last seq of each page, but pages advance strictly
    // (each `next` is greater than the last) precisely because storage order is seq order.
    let mut prev = 0;
    for seq in seqs {
        assert!(seq > prev, "seq must strictly increase page over page");
        prev = seq;
    }
    assert_eq!(prev, THREADS * PER, "the stream ends at exactly seq 400");
}

#[test]
fn concurrent_appends_to_different_streams_keep_independent_sequences() {
    // Independent per-stream state: hammering two DIFFERENT streams concurrently must not
    // let one stream's allocations bleed into the other's — each still reaches a
    // contiguous, gap-free 1..=N of its own.
    let Some(ep) = endpoint("concurrent_appends_to_different_streams_keep_independent_sequences")
    else {
        return;
    };
    let store = Arc::new(opstore(&ep));
    let stream_a = StreamId("stream-a".into());
    let stream_b = StreamId("stream-b".into());
    const THREADS: i64 = 4;
    const PER: i64 = 20;

    let spawn_for = |stream: StreamId| {
        (0..THREADS)
            .map(|t| {
                let store = Arc::clone(&store);
                let stream = stream.clone();
                thread::spawn(move || {
                    (0..PER)
                        .map(|i| {
                            store
                                .append(&stream, &wire(&format!("{}-{t}-{i}", stream.as_str())))
                                .unwrap()
                                .0
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>()
    };

    // Interleave both streams' threads so the race is genuinely concurrent, not
    // stream-a-then-stream-b.
    let handles_a = spawn_for(stream_a);
    let handles_b = spawn_for(stream_b);

    let collect = |handles: Vec<thread::JoinHandle<Vec<i64>>>| {
        let mut all: Vec<i64> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        all.sort_unstable();
        all
    };
    let expected: Vec<i64> = (1..=THREADS * PER).collect();
    assert_eq!(
        collect(handles_a),
        expected,
        "stream a reaches its own contiguous 1..=N"
    );
    assert_eq!(
        collect(handles_b),
        expected,
        "stream b reaches its own contiguous 1..=N, independent of stream a"
    );
}

// ----- DynamoDB PrefixRegistry — claim-if-free ---------------------------------------

#[test]
fn a_free_prefix_is_registered_dynamodb() {
    let Some(ep) = endpoint("a_free_prefix_is_registered_dynamodb") else {
        return;
    };
    let reg = registry(&ep);
    assert_eq!(
        reg.register(&StreamId("s".into()), "aaaa", "uuid-A")
            .unwrap(),
        RegisterOutcome::Registered
    );
}

#[test]
fn re_registering_the_same_claim_is_idempotent_dynamodb() {
    let Some(ep) = endpoint("re_registering_the_same_claim_is_idempotent_dynamodb") else {
        return;
    };
    let reg = registry(&ep);
    let s = StreamId("s".into());
    reg.register(&s, "aaaa", "uuid-A").unwrap();
    assert_eq!(
        reg.register(&s, "aaaa", "uuid-A").unwrap(),
        RegisterOutcome::Registered,
        "same (prefix, uuid) re-register stays Registered"
    );
}

#[test]
fn a_colliding_prefix_from_a_different_replica_is_reassigned_dynamodb() {
    let Some(ep) = endpoint("a_colliding_prefix_from_a_different_replica_is_reassigned_dynamodb")
    else {
        return;
    };
    let reg = registry(&ep);
    let s = StreamId("s".into());
    reg.register(&s, "aaaa", "uuid-A").unwrap();

    let outcome = reg.register(&s, "aaaa", "uuid-B").unwrap();
    let RegisterOutcome::Reassigned { new_prefix } = outcome else {
        panic!("expected Reassigned, got {outcome:?}");
    };
    assert_ne!(
        new_prefix, "aaaa",
        "the colliding replica gets a different prefix"
    );
    // The reassigned prefix is now B's stable claim; A still owns "aaaa".
    assert_eq!(
        reg.register(&s, "aaaa", "uuid-A").unwrap(),
        RegisterOutcome::Registered,
        "A's original claim is untouched"
    );
}

#[test]
fn a_reassigned_replica_gets_the_same_new_prefix_on_retry_dynamodb() {
    // Crash-safety: if the client is reassigned but crashes before adopting/persisting the
    // new prefix, re-registering the OLD prefix returns the SAME new prefix.
    let Some(ep) = endpoint("a_reassigned_replica_gets_the_same_new_prefix_on_retry_dynamodb")
    else {
        return;
    };
    let reg = registry(&ep);
    let s = StreamId("s".into());
    reg.register(&s, "aaaa", "uuid-A").unwrap();
    let first = reg.register(&s, "aaaa", "uuid-B").unwrap();
    let again = reg.register(&s, "aaaa", "uuid-B").unwrap();
    assert_eq!(first, again, "reassignment is stable across retries");
}

#[test]
fn registry_claims_are_isolated_per_stream_dynamodb() {
    let Some(ep) = endpoint("registry_claims_are_isolated_per_stream_dynamodb") else {
        return;
    };
    let reg = registry(&ep);
    reg.register(&StreamId("a".into()), "aaaa", "uuid-A")
        .unwrap();
    // The same prefix is free in a different stream → Registered, not Reassigned.
    assert_eq!(
        reg.register(&StreamId("b".into()), "aaaa", "uuid-B")
            .unwrap(),
        RegisterOutcome::Registered
    );
}

#[test]
fn registry_concurrent_collision_grants_exactly_one_dynamodb() {
    // claim-if-free under contention: N replicas race on ONE prefix; exactly one keeps it
    // (Registered), the rest Reassigned, and every resulting prefix is distinct (no
    // aliasing). This is THE risk the one `TransactWriteItems` exists to close: without it,
    // two racers can both observe the prefix as free and both "win".
    let Some(ep) = endpoint("registry_concurrent_collision_grants_exactly_one_dynamodb") else {
        return;
    };
    let reg = Arc::new(registry(&ep));
    let s = StreamId("s".into());
    const REPLICAS: usize = 8;

    let mut handles = Vec::new();
    for r in 0..REPLICAS {
        let reg = Arc::clone(&reg);
        let s = s.clone();
        handles.push(thread::spawn(move || {
            match reg.register(&s, "aaaa", &format!("uuid-{r}")).unwrap() {
                RegisterOutcome::Registered => "aaaa".to_string(),
                RegisterOutcome::Reassigned { new_prefix } => new_prefix,
            }
        }));
    }
    let prefixes: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    let registered = prefixes.iter().filter(|p| p.as_str() == "aaaa").count();
    assert_eq!(
        registered, 1,
        "exactly one replica keeps (Registered) the contested prefix"
    );
    let distinct: std::collections::HashSet<&String> = prefixes.iter().collect();
    assert_eq!(
        distinct.len(),
        REPLICAS,
        "every replica ends with a distinct prefix"
    );
}

#[test]
fn concurrent_registers_by_the_same_replica_converge_on_one_stable_answer_dynamodb() {
    // The guarded race `try_claim`/`resolve_uuid_race` exist for: several in-flight register
    // calls for the SAME (prefix, replica_uuid) — e.g. a client retrying a slow request. Only
    // one call's `TransactWriteItems` can land; every loser's uuid item ALSO fails its
    // condition (not just the prefix item, because both racers are asking for the identical
    // key pair), which is exactly the case `try_claim` must not treat as "try another
    // candidate" — that would wrongly reassign this one replica a second, aliasing prefix.
    let Some(ep) =
        endpoint("concurrent_registers_by_the_same_replica_converge_on_one_stable_answer_dynamodb")
    else {
        return;
    };
    let reg = Arc::new(registry(&ep));
    let s = StreamId("s".into());
    const RACERS: usize = 8;

    let mut handles = Vec::new();
    for _ in 0..RACERS {
        let reg = Arc::clone(&reg);
        let s = s.clone();
        handles.push(thread::spawn(move || {
            reg.register(&s, "aaaa", "uuid-A").unwrap()
        }));
    }
    let outcomes: Vec<RegisterOutcome> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    for outcome in &outcomes {
        assert_eq!(
            *outcome,
            RegisterOutcome::Registered,
            "every racer for the SAME replica converges on the one winning claim"
        );
    }
}

/// Deterministic register script; returns each outcome as a stable string. Copied from
/// `postgres_parity.rs::registry_trace` (integration-test binaries are separate compilation
/// units, so there is no shared crate to hold one copy) — kept byte-for-byte identical so a
/// divergence in either backend's `candidate_prefix` reuse shows up as a trace mismatch, not
/// a subtly different script.
fn registry_trace(reg: &dyn PrefixRegistry) -> Vec<String> {
    let s = StreamId("s".into());
    let show = |o: RegisterOutcome| match o {
        RegisterOutcome::Registered => "registered".to_string(),
        RegisterOutcome::Reassigned { new_prefix } => format!("reassigned:{new_prefix}"),
    };
    let mut trace = Vec::new();
    for (prefix, uuid) in [
        ("aaaa", "uuid-A"),
        ("aaaa", "uuid-B"), // collides → reassigned
        ("aaaa", "uuid-C"), // collides → reassigned (distinct from B)
        ("aaaa", "uuid-A"), // idempotent → registered
        ("aaaa", "uuid-B"), // idempotent → same reassignment
        ("bbbb", "uuid-A"), // A already owns aaaa → reassigned to its existing prefix
    ] {
        trace.push(format!(
            "{prefix} {uuid} -> {}",
            show(reg.register(&s, prefix, uuid).unwrap())
        ));
    }
    trace
}

#[test]
fn registry_parity_dynamodb_matches_sqlite() {
    let Some(ep) = endpoint("registry_parity_dynamodb_matches_sqlite") else {
        return;
    };
    let sqlite_trace = registry_trace(&SqlitePrefixRegistry::open_in_memory().unwrap());
    let ddb_trace = registry_trace(&registry(&ep));
    assert_eq!(
        sqlite_trace, ddb_trace,
        "DynamoDB registry must be observably identical to SQLite — proves candidate_prefix \
         reuse is value-for-value, not just shape-for-shape"
    );
}

// ----- presence (6j6v.f0b5) --------------------------------------------------------------

/// Copied byte for byte from `postgres_parity.rs::presence_trace` (separate test binaries, no
/// shared crate), so all three backends are held to one trace. The byte-order tie (`Mb` before
/// `ma`) is the line this backend could get wrong: its key orders by id, and the read sorts.
fn presence_trace(store: &dyn PresenceStore) -> Vec<String> {
    let a = StreamId("stream-a".into());
    let b = StreamId("stream-b".into());
    let hello = |id: &str, name: &str, interval_secs| MachineHello {
        machine_id: id.into(),
        name: name.into(),
        interval_secs,
    };
    fn read(
        store: &dyn PresenceStore,
        stream: &StreamId,
        since: i64,
        limit: usize,
        trace: &mut Vec<String>,
    ) {
        let page = store.machines(stream, since, limit).unwrap();
        let rows: Vec<String> = page
            .records
            .iter()
            .map(|r| {
                format!(
                    "{}={}@{}/{}",
                    r.machine_id, r.name, r.last_seen, r.interval_secs
                )
            })
            .collect();
        let more = if page.truncated { " +more" } else { "" };
        trace.push(format!(
            "{} since {since} limit {limit}: [{}]{more}",
            stream.as_str(),
            rows.join(", ")
        ));
    }
    let mut trace = Vec::new();
    store
        .announce(&a, &hello("ma", "MacBook", 300), 100, 0)
        .unwrap();
    store
        .announce(&a, &hello("Mb", "Mac mini", 300), 100, 0)
        .unwrap();
    store
        .announce(&a, &hello("z9", "Studio", 20), 50, 0)
        .unwrap();
    store
        .announce(&b, &hello("ma", "MacBook", 300), 70, 0)
        .unwrap();
    read(store, &a, 0, 100, &mut trace);
    read(store, &a, 0, 2, &mut trace);
    read(store, &b, 0, 100, &mut trace);
    // A machine last seen before the window is not listed.
    read(store, &a, 60, 100, &mut trace);
    // The same machine again: one row, everything it said replaced.
    store
        .announce(&a, &hello("z9", "Studio (renamed)", 600), 400, 0)
        .unwrap();
    read(store, &a, 0, 100, &mut trace);
    // An announcement that forgets what is past the window (SQLite and Postgres delete it, DynamoDB
    // filters it) — read within the window, which is what every backend answers alike.
    store
        .announce(&a, &hello("new", "Workstation", 300), 500, 150)
        .unwrap();
    read(store, &a, 150, 100, &mut trace);
    read(store, &StreamId("stream-none".into()), 0, 100, &mut trace);
    trace
}

const EXPECTED_PRESENCE_TRACE: [&str; 7] = [
    "stream-a since 0 limit 100: [Mb=Mac mini@100/300, ma=MacBook@100/300, z9=Studio@50/20]",
    "stream-a since 0 limit 2: [Mb=Mac mini@100/300, ma=MacBook@100/300] +more",
    "stream-b since 0 limit 100: [ma=MacBook@70/300]",
    "stream-a since 60 limit 100: [Mb=Mac mini@100/300, ma=MacBook@100/300]",
    "stream-a since 0 limit 100: [z9=Studio (renamed)@400/600, Mb=Mac mini@100/300, ma=MacBook@100/300]",
    "stream-a since 150 limit 100: [new=Workstation@500/300, z9=Studio (renamed)@400/600]",
    "stream-none since 0 limit 100: []",
];

#[test]
fn presence_parity_dynamodb_matches_sqlite() {
    let Some(ep) = endpoint("presence_parity_dynamodb_matches_sqlite") else {
        return;
    };
    let sqlite_trace = presence_trace(&SqlitePrefixRegistry::open_in_memory().unwrap());
    assert_eq!(sqlite_trace, EXPECTED_PRESENCE_TRACE);
    let ddb_trace = presence_trace(&registry(&ep));
    assert_eq!(
        sqlite_trace, ddb_trace,
        "DynamoDB presence must be observably identical to SQLite"
    );
}

#[test]
fn presence_items_share_the_registry_table_without_disturbing_a_claim_dynamodb() {
    // Presence is a THIRD item kind in the registry table (`machine#<id>`), beside `prefix#` and
    // `uuid#` — so a deployment provisions nothing new. It must not read as a claim, and a claim
    // must not read as a machine, even when a machine id happens to equal a replica uuid.
    let Some(ep) =
        endpoint("presence_items_share_the_registry_table_without_disturbing_a_claim_dynamodb")
    else {
        return;
    };
    let reg = registry(&ep);
    let s = StreamId("s".into());
    reg.announce(
        &s,
        &MachineHello {
            machine_id: "uuid-A".into(),
            name: "MacBook".into(),
            interval_secs: 300,
        },
        1,
        0,
    )
    .unwrap();
    assert_eq!(
        reg.register(&s, "aaaa", "uuid-A").unwrap(),
        RegisterOutcome::Registered
    );
    assert!(matches!(
        reg.register(&s, "aaaa", "uuid-B").unwrap(),
        RegisterOutcome::Reassigned { .. }
    ));
    let machines = reg.machines(&s, 0, 100).unwrap().records;
    assert_eq!(machines.len(), 1, "{machines:?}");
    assert_eq!(machines[0].machine_id, "uuid-A");
}

#[test]
fn a_presence_item_carries_when_it_leaves_the_window_for_an_optional_ttl_dynamodb() {
    // DynamoDB does not prune on announce; a deployment MAY enable TTL on `expires_at` instead. The
    // attribute must be exactly where the window ends, or a TTL would drop live machines early.
    let Some(ep) =
        endpoint("a_presence_item_carries_when_it_leaves_the_window_for_an_optional_ttl_dynamodb")
    else {
        return;
    };
    ensure_fake_credentials();
    let table = unique_registry_table(&ep);
    let reg = DynamoDbPrefixRegistry::new(table.clone(), Some(ep.clone()), None);
    let now = 1_790_000_000;
    let window = nxs_sync::presence::RETENTION_SECS;
    reg.announce(
        &StreamId("s".into()),
        &MachineHello {
            machine_id: "m1".into(),
            name: "MacBook".into(),
            interval_secs: 300,
        },
        now,
        now - window,
    )
    .unwrap();
    let client = admin_client(&ep);
    let item = admin_rt()
        .block_on(
            client
                .get_item()
                .table_name(&table)
                .key("stream_id", AttributeValue::S("s".into()))
                .key("sk", AttributeValue::S("machine#m1".into()))
                .consistent_read(true)
                .send(),
        )
        .unwrap()
        .item()
        .cloned()
        .expect("the presence item exists");
    assert_eq!(
        item.get("expires_at")
            .and_then(|v| v.as_n().ok())
            .map(String::as_str),
        Some((now + window).to_string().as_str())
    );

    // And the read filters by the window whether or not a TTL ever ran.
    assert!(reg
        .machines(&StreamId("s".into()), now + window + 1, 100)
        .unwrap()
        .records
        .is_empty());
}

// ----- boot smoke test: the wired backend serves the real HTTP surface -----------------

/// Mirrors `relay_http.rs::spawn_relay`, but composes `app()` from a DynamoDB-backed
/// `OpStore` + `PrefixRegistry` (each on its own harness-created table) instead of the
/// in-memory SQLite pair — the same composition `backends()` performs at boot. Duplicated
/// rather than shared with `relay_http.rs`: integration-test binaries are separate
/// compilation units (see `registry_trace`'s doc above for the same rationale).
fn spawn_ddb_relay(endpoint: &str) -> String {
    let ops = Arc::new(opstore(endpoint));
    let reg = Arc::new(registry(endpoint));
    let router = app(ops, reg);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, router).await.unwrap();
        });
    });
    format!("http://{addr}")
}

fn ddb_push(base: &str, stream: &str, ops: &[WireOp]) -> PushResponse {
    let body = serde_json::json!({ "ops": ops });
    let resp = ureq::post(&format!("{base}/streams/{stream}/ops"))
        .send_json(body)
        .unwrap();
    resp.into_json().unwrap()
}

fn ddb_pull(base: &str, stream: &str, since: i64, limit: usize) -> PullResponse {
    let resp = ureq::get(&format!("{base}/streams/{stream}/ops"))
        .query("since", &since.to_string())
        .query("limit", &limit.to_string())
        .call()
        .unwrap();
    resp.into_json().unwrap()
}

fn ddb_register(base: &str, stream: &str, prefix: &str, uuid: &str) -> RegisterOutcome {
    let body = serde_json::json!({ "prefix": prefix, "replica_uuid": uuid });
    let resp = ureq::post(&format!("{base}/streams/{stream}/register"))
        .send_json(body)
        .unwrap();
    resp.into_json().unwrap()
}

#[test]
fn the_wired_dynamodb_backend_serves_presence_over_http() {
    // The composition `backends()` performs, driven over the new route: an announcement goes in
    // through the handler, comes back out through it, and a malformed one is refused before it
    // reaches the table.
    let Some(ep) = endpoint("the_wired_dynamodb_backend_serves_presence_over_http") else {
        return;
    };
    let base = spawn_ddb_relay(&ep);
    let announce =
        |body: serde_json::Value| match ureq::post(&format!("{base}/streams/s1/machines"))
            .send_json(body)
        {
            Ok(resp) => resp.status(),
            Err(ureq::Error::Status(status, _)) => status,
            Err(e) => panic!("transport error: {e}"),
        };
    assert_eq!(
        announce(serde_json::json!({"machine_id": "m1", "name": "MacBook", "interval_secs": 300})),
        204
    );
    assert_eq!(
        announce(serde_json::json!({"machine_id": "m2", "name": "", "interval_secs": 300})),
        400
    );
    let listed: MachinesResponse = ureq::get(&format!("{base}/streams/s1/machines"))
        .call()
        .unwrap()
        .into_json()
        .unwrap();
    assert_eq!(listed.machines.len(), 1, "{listed:?}");
    assert_eq!(listed.machines[0].machine_id, "m1");
    assert!(
        listed.machines[0].age_secs < 60,
        "stamped with the relay's own clock"
    );
}

#[test]
fn the_wired_dynamodb_backend_serves_the_real_http_surface() {
    // `backends()` (crates/server/src/main.rs) composes a DynamoDbOpStore +
    // DynamoDbPrefixRegistry behind app() at boot exactly like it composes the SQLite pair
    // — this test performs the identical composition and drives it over real HTTP, so a
    // regression in that wiring (not just in the store/registry methods themselves, which
    // every other test above already covers directly) would show up here.
    let Some(ep) = endpoint("the_wired_dynamodb_backend_serves_the_real_http_surface") else {
        return;
    };
    let base = spawn_ddb_relay(&ep);

    let batch = [wire("o1"), wire("o2"), wire("o3")];
    let appended = ddb_push(&base, "s1", &batch);
    assert_eq!(appended.appended, 3);

    let page = ddb_pull(&base, "s1", 0, 100);
    assert_eq!(
        page.ops
            .iter()
            .map(|o| o.op_id.as_str())
            .collect::<Vec<_>>(),
        ["o1", "o2", "o3"],
        "ops pushed over HTTP come back in order over HTTP"
    );
    assert_eq!(page.next.0, 3);

    assert_eq!(
        ddb_register(&base, "s1", "aaaa", "uuid-A"),
        RegisterOutcome::Registered,
        "a free prefix registers over HTTP against the DynamoDB registry"
    );
}
