//! DynamoDB Local plumbing for the purge suites: tables per test, raw items, full dumps, and a
//! recording HTTP proxy that counts what a purge PROCESS sends (an in-process interceptor cannot
//! see a separate process — design §8 step 7).
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use aws_sdk_dynamodb::types::{
    AttributeDefinition, AttributeValue, BillingMode, GlobalSecondaryIndex, KeySchemaElement,
    KeyType, Projection, ProjectionType, ScalarAttributeType, TableStatus,
};
use aws_sdk_dynamodb::Client;

/// The DynamoDB Local endpoint, or `None` (a loud skip) when `NXF_RELAY_DDB_ENDPOINT` is unset; a
/// hard failure under `CI`, like the other DynamoDB suite.
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

pub fn ensure_fake_credentials() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::env::set_var("AWS_ACCESS_KEY_ID", "test");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "test");
        std::env::set_var("AWS_REGION", REGION);
    });
}

pub fn rt() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("admin runtime"))
}

pub fn client(endpoint: &str) -> Client {
    ensure_fake_credentials();
    rt().block_on(async {
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .endpoint_url(endpoint)
            .region(aws_sdk_dynamodb::config::Region::new(REGION))
            .load()
            .await;
        Client::new(&config)
    })
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

/// Create a table with `stream_id` (S) as hash key and `range` of `range_type`, optionally with a
/// GSI, and wait for ACTIVE.
pub fn create(
    c: &Client,
    table: &str,
    hash: &str,
    range: &str,
    range_type: ScalarAttributeType,
    gsi: bool,
) {
    rt().block_on(async {
        let mut req = c
            .create_table()
            .table_name(table)
            .billing_mode(BillingMode::PayPerRequest)
            .key_schema(key(hash, KeyType::Hash))
            .key_schema(key(range, KeyType::Range))
            .attribute_definitions(attr(hash, ScalarAttributeType::S))
            .attribute_definitions(attr(range, range_type));
        if gsi {
            req = req
                .attribute_definitions(attr("gsi_pk", ScalarAttributeType::S))
                .global_secondary_indexes(
                    GlobalSecondaryIndex::builder()
                        .index_name("by_gsi")
                        .key_schema(key("gsi_pk", KeyType::Hash))
                        .key_schema(key(range, KeyType::Range))
                        .projection(
                            Projection::builder()
                                .projection_type(ProjectionType::All)
                                .build(),
                        )
                        .build()
                        .unwrap(),
                );
        }
        req.send().await.expect("create table");
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

pub fn ops_table(c: &Client) -> String {
    let t = unique("nxf_purge_ops");
    create(c, &t, "stream_id", "seq", ScalarAttributeType::N, false);
    t
}

pub fn registry_table(c: &Client) -> String {
    let t = unique("nxf_purge_reg");
    create(c, &t, "stream_id", "sk", ScalarAttributeType::S, false);
    t
}

pub fn delete_table(c: &Client, table: &str) {
    rt().block_on(async {
        let _ = c.delete_table().table_name(table).send().await;
    });
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

pub fn delete_raw(c: &Client, table: &str, partition: &str, sk: &str) {
    rt().block_on(async {
        c.delete_item()
            .table_name(table)
            .key("stream_id", AttributeValue::S(partition.into()))
            .key("sk", AttributeValue::S(sk.into()))
            .send()
            .await
            .expect("delete raw item");
    });
}

fn render(v: &AttributeValue) -> String {
    match v {
        AttributeValue::S(s) => format!("S:{s}"),
        AttributeValue::N(n) => format!("N:{n}"),
        AttributeValue::Bool(b) => format!("BOOL:{b}"),
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

/// The dump without the items whose partition is `partition` (and without `extra` exact lines).
pub fn dump_without(c: &Client, table: &str, partition: &str) -> Vec<String> {
    let needle = format!("\"stream_id\": \"S:{partition}\"");
    dump(c, table)
        .into_iter()
        .filter(|row| !row.contains(&needle))
        .collect()
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
