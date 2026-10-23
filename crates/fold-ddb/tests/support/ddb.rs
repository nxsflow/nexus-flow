//! DynamoDB Local plumbing for the board-fold purge suites (6j6v.dbmd): tables per test, raw items,
//! full dumps, and a recording HTTP proxy that counts what a purge PROCESS sends — an in-process
//! interceptor cannot see a separate process. The proxy is the relay purge's (`nxs-server`'s
//! `tests/support/ddb.rs`), copied: a test helper, not a contract.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aws_sdk_dynamodb::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_dynamodb::types::{
    AttributeDefinition, AttributeValue, BillingMode, KeySchemaElement, KeyType,
    ScalarAttributeType, TableStatus,
};
use aws_sdk_dynamodb::Client;

/// The DynamoDB Local endpoint, or `None` (a loud skip) when `NXF_RELAY_DDB_ENDPOINT` is unset; a
/// hard failure under `CI`, like the other DynamoDB suites.
pub fn endpoint(test: &str) -> Option<String> {
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

pub const REGION: &str = "us-east-1";

pub fn rt() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("admin runtime"))
}

pub fn client(endpoint: &str) -> Client {
    let config = aws_sdk_dynamodb::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(REGION))
        .endpoint_url(endpoint)
        .credentials_provider(Credentials::new(
            "test",
            "test",
            None,
            None,
            "dynamodb-local",
        ))
        .build();
    Client::from_conf(config)
}

pub fn unique(prefix: &str) -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "{prefix}_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn key(name: &str, t: KeyType) -> KeySchemaElement {
    KeySchemaElement::builder()
        .attribute_name(name)
        .key_type(t)
        .build()
        .unwrap()
}

fn attr(name: &str, t: ScalarAttributeType) -> AttributeDefinition {
    AttributeDefinition::builder()
        .attribute_name(name)
        .attribute_type(t)
        .build()
        .unwrap()
}

/// A table with `stream_id` (S) as hash key and `range` of `range_type`, as the relay's tables are.
pub fn relay_table(c: &Client, table: &str, range: &str, range_type: ScalarAttributeType) {
    rt().block_on(async {
        c.create_table()
            .table_name(table)
            .billing_mode(BillingMode::PayPerRequest)
            .key_schema(key("stream_id", KeyType::Hash))
            .key_schema(key(range, KeyType::Range))
            .attribute_definitions(attr("stream_id", ScalarAttributeType::S))
            .attribute_definitions(attr(range, range_type))
            .send()
            .await
            .expect("create table");
    });
    wait_active(c, table);
}

/// A board-fold table from before the `typed` index (6j6v.xkr7): the keys and the `active` and
/// `dated` indexes as `ddb::create_table` makes them, no `typed`.
pub fn untyped_fold_table(c: &Client, table: &str) {
    use aws_sdk_dynamodb::types::{GlobalSecondaryIndex, Projection, ProjectionType};
    let index = |name: &str, hash: &str, range: &str| {
        GlobalSecondaryIndex::builder()
            .index_name(name)
            .key_schema(key(hash, KeyType::Hash))
            .key_schema(key(range, KeyType::Range))
            .projection(
                Projection::builder()
                    .projection_type(ProjectionType::All)
                    .build(),
            )
            .build()
            .unwrap()
    };
    rt().block_on(async {
        c.create_table()
            .table_name(table)
            .billing_mode(BillingMode::PayPerRequest)
            .key_schema(key("stream_id", KeyType::Hash))
            .key_schema(key("sk", KeyType::Range))
            .attribute_definitions(attr("stream_id", ScalarAttributeType::S))
            .attribute_definitions(attr("sk", ScalarAttributeType::S))
            .attribute_definitions(attr("nxf_active", ScalarAttributeType::S))
            .attribute_definitions(attr("nxf_dated", ScalarAttributeType::S))
            .attribute_definitions(attr("nxf_dated_at", ScalarAttributeType::S))
            .global_secondary_indexes(index("active", "nxf_active", "sk"))
            .global_secondary_indexes(index("dated", "nxf_dated", "nxf_dated_at"))
            .send()
            .await
            .expect("create the untyped fold table");
    });
    wait_active(c, table);
}

pub fn wait_active(c: &Client, table: &str) {
    rt().block_on(async {
        loop {
            let resp = c.describe_table().table_name(table).send().await.unwrap();
            if resp.table().and_then(|t| t.table_status()) == Some(&TableStatus::Active) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
}

pub fn delete_table(c: &Client, table: &str) {
    rt().block_on(async {
        let _ = c.delete_table().table_name(table).send().await;
    });
}

pub fn table_names(c: &Client) -> Vec<String> {
    rt().block_on(async {
        let mut names = Vec::new();
        let mut start = None;
        loop {
            let out = c
                .list_tables()
                .set_exclusive_start_table_name(start.take())
                .send()
                .await
                .unwrap();
            names.extend(out.table_names().iter().cloned());
            match out.last_evaluated_table_name() {
                Some(n) => start = Some(n.to_string()),
                None => break,
            }
        }
        names.sort();
        names
    })
}

/// Put one raw item with string attributes (numbers given as `#N:<n>`).
pub fn put_raw(c: &Client, table: &str, attrs: &[(&str, &str)]) {
    rt().block_on(async {
        let mut put = c.put_item().table_name(table);
        for (k, v) in attrs {
            let value = match v.strip_prefix("#N:") {
                Some(n) => AttributeValue::N(n.to_string()),
                None => AttributeValue::S(v.to_string()),
            };
            put = put.item(*k, value);
        }
        put.send().await.expect("put raw item");
    });
}

fn render(v: &AttributeValue) -> String {
    match v {
        AttributeValue::S(s) => format!("S:{s}"),
        AttributeValue::N(n) => format!("N:{n}"),
        AttributeValue::Bool(b) => format!("BOOL:{b}"),
        AttributeValue::Null(_) => "NULL".to_string(),
        other => format!("{other:?}"),
    }
}

/// Every item of a table, as sorted text lines (a full Scan, strongly consistent).
pub fn dump(c: &Client, table: &str) -> Vec<String> {
    rt().block_on(async {
        let mut rows = Vec::new();
        let mut start = None;
        loop {
            let resp = c
                .scan()
                .table_name(table)
                .consistent_read(true)
                .set_exclusive_start_key(start.take())
                .send()
                .await
                .expect("scan");
            for item in resp.items() {
                let sorted: BTreeMap<&String, String> =
                    item.iter().map(|(k, v)| (k, render(v))).collect();
                rows.push(format!("{sorted:?}"));
            }
            match resp.last_evaluated_key() {
                Some(k) if !k.is_empty() => start = Some(k.clone()),
                _ => break,
            }
        }
        rows.sort();
        rows
    })
}

/// The dump without the items whose partition is `partition`.
pub fn dump_without(c: &Client, table: &str, partition: &str) -> Vec<String> {
    let needle = format!("\"stream_id\": \"S:{partition}\"");
    dump(c, table)
        .into_iter()
        .filter(|row| !row.contains(&needle))
        .collect()
}

/// The dump of one partition only.
pub fn dump_of(c: &Client, table: &str, partition: &str) -> Vec<String> {
    let needle = format!("\"stream_id\": \"S:{partition}\"");
    dump(c, table)
        .into_iter()
        .filter(|row| row.contains(&needle))
        .collect()
}

/// The sort keys of one partition, by a consistent Query.
pub fn partition_keys(
    c: &Client,
    table: &str,
    partition: &str,
) -> std::collections::BTreeSet<String> {
    rt().block_on(async {
        let mut keys = std::collections::BTreeSet::new();
        let mut start = None;
        loop {
            let out = c
                .query()
                .table_name(table)
                .key_condition_expression("#pk = :pk")
                .expression_attribute_names("#pk", "stream_id")
                .expression_attribute_values(":pk", AttributeValue::S(partition.into()))
                .consistent_read(true)
                .set_exclusive_start_key(start.take())
                .send()
                .await
                .expect("query");
            for item in out.items() {
                if let Some(AttributeValue::S(sk)) = item.get("sk") {
                    keys.insert(sk.clone());
                }
            }
            match out.last_evaluated_key() {
                Some(k) if !k.is_empty() => start = Some(k.clone()),
                _ => break,
            }
        }
        keys
    })
}

/// How many items an index partition holds (eventually consistent, as every index read).
pub fn index_count(c: &Client, table: &str, index: &str, attribute: &str, value: &str) -> usize {
    rt().block_on(async {
        let out = c
            .query()
            .table_name(table)
            .index_name(index)
            .key_condition_expression("#a = :a")
            .expression_attribute_names("#a", attribute)
            .expression_attribute_values(":a", AttributeValue::S(value.into()))
            .send()
            .await
            .expect("index query");
        out.items().len()
    })
}

// ----- the recording proxy ------------------------------------------------------------------------

/// A TCP proxy in front of DynamoDB Local that records the `X-Amz-Target` of every request, in
/// order. Bytes pass through unchanged in both directions.
pub struct Proxy {
    pub url: String,
    log: Arc<Mutex<Vec<String>>>,
}

impl Proxy {
    pub fn start(upstream: &str) -> Proxy {
        let upstream = upstream
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_string();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let log_accept = log.clone();
        std::thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let Ok(server) = TcpStream::connect(&upstream) else {
                    continue;
                };
                let log = log_accept.clone();
                let (c2, s2) = (client.try_clone().unwrap(), server.try_clone().unwrap());
                std::thread::spawn(move || pump_requests(client, server, log));
                std::thread::spawn(move || pump(s2, c2));
            }
        });
        Proxy { url, log }
    }

    /// The operations seen so far, in order (`Query`, `BatchWriteItem`, …).
    pub fn ops(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.log.lock().unwrap().clear();
    }
}

fn pump(mut from: TcpStream, mut to: TcpStream) {
    let mut buf = [0u8; 64 * 1024];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => {
                let _ = to.shutdown(std::net::Shutdown::Write);
                return;
            }
            Ok(n) => {
                if to.write_all(&buf[..n]).is_err() {
                    return;
                }
            }
        }
    }
}

/// Client → server: forward, and pick each request's target out of its headers. A header line
/// starts after CRLF; a JSON body never carries a raw CRLF, so a scan of the raw bytes is exact.
fn pump_requests(mut from: TcpStream, mut to: TcpStream, log: Arc<Mutex<Vec<String>>>) {
    let needle = b"\r\nx-amz-target: dynamodb_20120810.";
    let mut buf = [0u8; 64 * 1024];
    let mut tail: Vec<u8> = Vec::new();
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => {
                let _ = to.shutdown(std::net::Shutdown::Write);
                return;
            }
            Ok(n) => {
                tail.extend_from_slice(&buf[..n]);
                let lower: Vec<u8> = tail.iter().map(u8::to_ascii_lowercase).collect();
                let mut consumed = 0;
                while let Some(pos) = lower[consumed..]
                    .windows(needle.len())
                    .position(|w| w == needle)
                {
                    let start = consumed + pos + needle.len();
                    let Some(end) = lower[start..].iter().position(|b| *b == b'\r') else {
                        break;
                    };
                    let name = String::from_utf8_lossy(&tail[start..start + end]).to_string();
                    log.lock().unwrap().push(name);
                    consumed = start + end;
                }
                // Keep a little of the end, in case a header straddles two reads.
                let keep = tail.len().saturating_sub(consumed).min(256);
                tail.drain(..tail.len() - keep);
                if to.write_all(&buf[..n]).is_err() {
                    return;
                }
            }
        }
    }
}
