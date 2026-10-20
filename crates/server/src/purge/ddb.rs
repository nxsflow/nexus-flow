//! The DynamoDB purge store (design §3 DynamoDB).
//!
//! - **Identity gate**, before any read of a partition: `DescribeTable` on both tables, judged by
//!   the pure [`relay_tables_identity`] — the ops table exactly `stream_id` (S, HASH) + `seq` (N,
//!   RANGE), the registry table exactly `stream_id` (S, HASH) + `sk` (S, RANGE), neither with a GSI
//!   or an LSI, two different names and ARNs. A failed rule is `NotARelayStore` (78).
//! - **The plan**: strongly consistent Queries that project keys and the few attributes the report
//!   needs (never `value`) and follow `LastEvaluatedKey` to the end (a page can come back short at
//!   1 MB): the counter, then the ops `seq BETWEEN 1 AND next_seq`, the claims `seq < 0`, the
//!   registry partition (every `sk` classified by its prefix; an unknown prefix is `UnknownKind`,
//!   78), and exact-key Queries for `#due` and the stream lease.
//! - **The delete deletes the plan's keys**, `BatchWriteItem`s of 25, no Query in between: D1 ops,
//!   D2 claims (after the ops, so a late re-push is still turned back by its claim), D3 the counter
//!   (last on the ops table, so no append restarts the stream at seq 1 while ops are deleted), D4
//!   the registry, D5 `#due`. `UnprocessedItems` are sent again, 5 calls per batch in all; then
//!   exit 1. The lease rule runs before every batch; every key is checked to be the target's
//!   before it goes into a batch.
//! - Errors are carried as their DynamoDB error CODE only (log-id v1 C10).
//!
//! The IAM this needs (marked unverified) and its pass criterion are in the `purge` module doc.

use std::collections::HashMap;
use std::time::Duration;

use aws_sdk_dynamodb::types::{
    AttributeValue, DeleteRequest, KeyType, ScalarAttributeType, TableDescription, WriteRequest,
};
use aws_sdk_dynamodb::Client;
use nxs_sync::protocol::StreamId;
use serde_json::{json, Value};

use super::{
    name_unknown_prefix, parse_rfc3339, Deleted, OpsFacts, Physical, Plan, PlanDigest, PlanKey,
    PlannedKind, PurgeError, PurgeStore, RunCtx, StoreIdentity,
};
use crate::registry::StreamLease;
use crate::store::StoreError;
use crate::store_ddb::{
    block_on, build_client, ddb_code_err, lease_sk, DynamoDbPrefixRegistry, SWEEP_PARTITION,
};

/// `BatchWriteItem`'s ceiling.
pub const BATCH: usize = 25;
/// Calls per batch, the first included, before `UnprocessedItems` is a failure (exit 1).
pub const BATCH_ATTEMPTS: u32 = 5;
/// The partition of the `#due` items.
pub const DUE_PARTITION: &str = "#due";

/// The registry sort-key prefixes this build deletes (`PURGE_COVERS`' `registry:` entries).
const REGISTRY_PREFIXES: &[&str] = &["prefix#", "uuid#", "machine#"];

/// One item key of a delete batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemKey {
    pub partition: String,
    pub sort: SortKey,
}

/// A range key: a number on the ops table (`seq`), a string on the registry table (`sk`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SortKey {
    N(i64),
    S(String),
}

/// The seam the delete batches go through: the SDK in production, a fake in the unit tests
/// (`UnprocessedItems` on demand).
pub trait BatchDeleter: Send + Sync {
    /// One `BatchWriteItem` of up to 25 `DeleteRequest`s on `table`; answers the keys DynamoDB
    /// returned as unprocessed.
    fn delete_batch(&self, table: &str, keys: &[ItemKey]) -> Result<Vec<ItemKey>, StoreError>;
}

/// The SDK's `BatchWriteItem`.
pub struct SdkBatchDeleter {
    client: Client,
}

/// The item key as DynamoDB takes it: a number range key is the ops table's `seq`, a string one
/// the registry table's `sk`.
fn key_attributes(key: &ItemKey) -> HashMap<String, AttributeValue> {
    let mut map = HashMap::new();
    map.insert(
        "stream_id".to_string(),
        AttributeValue::S(key.partition.clone()),
    );
    let (name, value) = match &key.sort {
        SortKey::N(n) => ("seq", AttributeValue::N(n.to_string())),
        SortKey::S(s) => ("sk", AttributeValue::S(s.clone())),
    };
    map.insert(name.to_string(), value);
    map
}

fn item_key(map: &HashMap<String, AttributeValue>) -> Option<ItemKey> {
    let partition = map.get("stream_id")?.as_s().ok()?.clone();
    let sort = match (map.get("seq"), map.get("sk")) {
        (Some(AttributeValue::N(n)), _) => SortKey::N(n.parse().ok()?),
        (_, Some(AttributeValue::S(s))) => SortKey::S(s.clone()),
        _ => return None,
    };
    Some(ItemKey { partition, sort })
}

impl BatchDeleter for SdkBatchDeleter {
    fn delete_batch(&self, table: &str, keys: &[ItemKey]) -> Result<Vec<ItemKey>, StoreError> {
        let client = self.client.clone();
        let table = table.to_string();
        let requests: Vec<WriteRequest> = keys
            .iter()
            .map(|k| {
                WriteRequest::builder()
                    .delete_request(
                        DeleteRequest::builder()
                            .set_key(Some(key_attributes(k)))
                            .build()
                            .expect("a delete request with a key builds"),
                    )
                    .build()
            })
            .collect();
        block_on(async move {
            let out = client
                .batch_write_item()
                .request_items(&table, requests)
                .send()
                .await
                .map_err(ddb_code_err)?;
            let mut left = Vec::new();
            if let Some(unprocessed) = out.unprocessed_items().and_then(|m| m.get(&table)) {
                for request in unprocessed {
                    let key = request
                        .delete_request()
                        .map(|d| d.key())
                        .and_then(item_key)
                        .ok_or_else(|| {
                            StoreError("dynamodb error: an unprocessed item without a key".into())
                        })?;
                    left.push(key);
                }
            }
            Ok(left)
        })
    }
}

/// Delete `keys` from `table` in batches of 25, the lease rule and the target check before each
/// batch, `UnprocessedItems` sent again with exponential backoff, the seam after each batch
/// (`<step>-batch:<k>`) and after the step (`<step>`). Counts what went into `deleted` as it goes.
#[allow(clippy::too_many_arguments)]
pub fn delete_keys(
    deleter: &dyn BatchDeleter,
    table: &str,
    kind: &'static str,
    step: &str,
    keys: &[ItemKey],
    is_target: &dyn Fn(&ItemKey) -> bool,
    ctx: &mut RunCtx<'_>,
    deleted: &mut Deleted,
    backoff: &dyn Fn(u32) -> Duration,
) -> Result<(), PurgeError> {
    for (i, batch) in keys.chunks(BATCH).enumerate() {
        ctx.before_batch()?;
        // Defence in depth: a key of another stream in a batch is a bug; stop before sending.
        if batch.iter().any(|k| !is_target(k)) {
            return Err(PurgeError::Store(StoreError(format!(
                "a key outside the target was planned for {step}; nothing more is deleted"
            ))));
        }
        let mut pending: Vec<ItemKey> = batch.to_vec();
        let mut attempt = 0;
        while !pending.is_empty() {
            if attempt == BATCH_ATTEMPTS {
                return Err(PurgeError::Store(StoreError(format!(
                    "dynamodb error: UnprocessedItems after {BATCH_ATTEMPTS} calls in {step}"
                ))));
            }
            if attempt > 0 {
                std::thread::sleep(backoff(attempt));
            }
            let left = deleter.delete_batch(table, &pending)?;
            deleted.add(kind, (pending.len() - left.len()) as u64);
            pending = left;
            attempt += 1;
        }
        ctx.reached(&format!("{step}-batch:{}", i + 1))?;
    }
    ctx.reached(step)
}

/// The backoff between two calls of one batch: 50 ms doubled per attempt.
pub fn shipped_backoff(attempt: u32) -> Duration {
    Duration::from_millis(50 << attempt.min(6))
}

fn key_rule(
    desc: &TableDescription,
    which: &str,
    range: &str,
    range_type: ScalarAttributeType,
) -> Result<(), PurgeError> {
    let refuse = |rule: &str| {
        Err(PurgeError::NotARelayStore(format!(
            "the {which} table {rule}"
        )))
    };
    let keys = desc.key_schema();
    let hash = keys
        .iter()
        .find(|k| k.key_type() == &KeyType::Hash)
        .map(|k| k.attribute_name());
    let sort = keys
        .iter()
        .find(|k| k.key_type() == &KeyType::Range)
        .map(|k| k.attribute_name());
    if keys.len() != 2 || hash != Some("stream_id") || sort != Some(range) {
        return refuse(&format!(
            "does not have exactly the key stream_id (HASH) + {range} (RANGE)"
        ));
    }
    let type_of = |name: &str| {
        desc.attribute_definitions()
            .iter()
            .find(|a| a.attribute_name() == name)
            .map(|a| a.attribute_type().clone())
    };
    if type_of("stream_id") != Some(ScalarAttributeType::S) || type_of(range) != Some(range_type) {
        return refuse("has the right key names but not their types");
    }
    if !desc.global_secondary_indexes().is_empty() {
        return refuse("has a global secondary index (a relay table has none)");
    }
    if !desc.local_secondary_indexes().is_empty() {
        return refuse("has a local secondary index (a relay table has none)");
    }
    Ok(())
}

/// Whether two descriptions are a relay's ops and registry tables, and their identity: `TableArn`,
/// `TableId` and `CreationDateTime` of both. Pure, so every rule is unit-tested on constructed
/// descriptions, including states DynamoDB Local cannot produce.
pub fn relay_tables_identity(
    ops: &TableDescription,
    registry: &TableDescription,
) -> Result<StoreIdentity, PurgeError> {
    key_rule(ops, "ops", "seq", ScalarAttributeType::N)?;
    key_rule(registry, "registry", "sk", ScalarAttributeType::S)?;
    if ops.table_name() == registry.table_name() {
        return Err(PurgeError::NotARelayStore(
            "NXF_RELAY_DDB_TABLE and NXF_RELAY_DDB_REGISTRY_TABLE name the same table".into(),
        ));
    }
    if ops.table_arn().is_none() || ops.table_arn() == registry.table_arn() {
        return Err(PurgeError::NotARelayStore(
            "the two tables do not have two different ARNs".into(),
        ));
    }
    let created = |d: &TableDescription| {
        d.creation_date_time().and_then(|t| {
            t.fmt(aws_sdk_dynamodb::primitives::DateTimeFormat::DateTime)
                .ok()
        })
    };
    let mut locator = serde_json::Map::new();
    locator.insert("ops_table".into(), json!(ops.table_name()));
    locator.insert("registry_table".into(), json!(registry.table_name()));
    Ok(StoreIdentity {
        backend: "dynamodb",
        locator,
        identity: json!({
            "ops_table_arn": ops.table_arn(),
            "ops_table_id": ops.table_id(),
            "ops_created": created(ops),
            "registry_table_arn": registry.table_arn(),
            "registry_table_id": registry.table_id(),
            "registry_created": created(registry),
        }),
        populated: ops.item_count().unwrap_or(0) > 0,
    })
}

/// One relay store on DynamoDB.
pub struct DynamoDbPurgeStore {
    client: Client,
    ops_table: String,
    registry_table: String,
    region: String,
    endpoint: Option<String>,
    /// The registry the stream lease lives on (and the injection seam's way in).
    registry: DynamoDbPrefixRegistry,
    deleter: Box<dyn BatchDeleter>,
}

impl DynamoDbPurgeStore {
    /// Build the clients; no request leaves the process until `identity`.
    pub fn new(
        ops_table: String,
        registry_table: String,
        region: String,
        endpoint: Option<String>,
    ) -> DynamoDbPurgeStore {
        let client = block_on(build_client(endpoint.clone(), Some(region.clone())));
        let registry = DynamoDbPrefixRegistry::new(
            registry_table.clone(),
            endpoint.clone(),
            Some(region.clone()),
        );
        DynamoDbPurgeStore {
            deleter: Box::new(SdkBatchDeleter {
                client: client.clone(),
            }),
            client,
            ops_table,
            registry_table,
            region,
            endpoint,
            registry,
        }
    }
}

/// A Query that follows `LastEvaluatedKey`, page by page, through `ctx.plan_page()`.
struct Pages<'a> {
    client: &'a Client,
    table: &'a str,
    key_condition: &'a str,
    values: Vec<(&'a str, AttributeValue)>,
    names: Vec<(&'a str, &'a str)>,
    projection: &'a str,
    limit: Option<i32>,
}

impl Pages<'_> {
    fn read(
        &self,
        ctx: &mut RunCtx<'_>,
    ) -> Result<Vec<HashMap<String, AttributeValue>>, PurgeError> {
        let mut items = Vec::new();
        let mut start: Option<HashMap<String, AttributeValue>> = None;
        loop {
            let mut q = self
                .client
                .query()
                .table_name(self.table)
                .key_condition_expression(self.key_condition)
                .projection_expression(self.projection)
                .consistent_read(true)
                .set_limit(self.limit)
                .set_exclusive_start_key(start.take());
            for (k, v) in &self.values {
                q = q.expression_attribute_values(*k, v.clone());
            }
            for (k, v) in &self.names {
                q = q.expression_attribute_names(*k, *v);
            }
            let resp = block_on(q.send()).map_err(|e| PurgeError::Store(ddb_code_err(e)))?;
            items.extend(resp.items().iter().cloned());
            ctx.plan_page()?;
            match resp.last_evaluated_key() {
                Some(key) if !key.is_empty() => start = Some(key.clone()),
                _ => return Ok(items),
            }
        }
    }
}

fn n(item: &HashMap<String, AttributeValue>, name: &str) -> Option<i64> {
    item.get(name)?.as_n().ok()?.parse().ok()
}

fn s<'a>(item: &'a HashMap<String, AttributeValue>, name: &str) -> Option<&'a str> {
    item.get(name)?.as_s().ok().map(String::as_str)
}

fn malformed(what: &str) -> PurgeError {
    PurgeError::Store(StoreError(format!(
        "dynamodb error: a {what} item without its key"
    )))
}

impl DynamoDbPurgeStore {
    fn describe(&self, table: &str, variable: &str) -> Result<TableDescription, PurgeError> {
        use aws_sdk_dynamodb::operation::describe_table::DescribeTableError;
        let request = self.client.describe_table().table_name(table).send();
        match block_on(request) {
            Ok(out) => out.table().cloned().ok_or_else(|| {
                PurgeError::NotARelayStore(format!("the table {variable} names has no description"))
            }),
            Err(e) => match e.into_service_error() {
                DescribeTableError::ResourceNotFoundException(_) => {
                    Err(PurgeError::NotARelayStore(format!(
                        "the table {variable} names does not exist"
                    )))
                }
                other => Err(PurgeError::Store(ddb_code_err(other))),
            },
        }
    }
}

impl PurgeStore for DynamoDbPurgeStore {
    fn backend(&self) -> &'static str {
        "dynamodb"
    }

    fn kinds(&self) -> &'static [&'static str] {
        &["ops", "claims", "counter", "registry", "due"]
    }

    fn covers(&self) -> Vec<String> {
        super::PURGE_COVERS.iter().map(|c| c.to_string()).collect()
    }

    fn identity(&self) -> Result<StoreIdentity, PurgeError> {
        let ops = self.describe(&self.ops_table, "NXF_RELAY_DDB_TABLE")?;
        let registry = self.describe(&self.registry_table, "NXF_RELAY_DDB_REGISTRY_TABLE")?;
        let mut identity = relay_tables_identity(&ops, &registry)?;
        identity.locator.insert("region".into(), json!(self.region));
        if let Some(endpoint) = &self.endpoint {
            identity.locator.insert("endpoint".into(), json!(endpoint));
        }
        Ok(identity)
    }

    fn plan(&self, stream: &StreamId, ctx: &mut RunCtx<'_>) -> Result<Plan, PurgeError> {
        let sid = AttributeValue::S(stream.as_str().to_string());
        let page = Some(ctx.page().min(i32::MAX as usize) as i32);
        let started = super::unix_secs(ctx.plan_started());

        // The counter first: it bounds the ops plan.
        let counter_items = Pages {
            client: &self.client,
            table: &self.ops_table,
            key_condition: "stream_id = :sid AND #seq = :zero",
            values: vec![
                (":sid", sid.clone()),
                (":zero", AttributeValue::N("0".into())),
            ],
            names: vec![("#seq", "seq"), ("#next", "next_seq")],
            projection: "stream_id, #seq, #next",
            limit: None,
        }
        .read(ctx)?;
        let next_seq = counter_items.first().and_then(|i| n(i, "next_seq"));
        let mut counter = PlannedKind::new("counter");
        if !counter_items.is_empty() {
            counter.keys.push(PlanKey::Seq(0));
        }

        let (ops_condition, mut ops_values) = match next_seq {
            Some(_) => (
                "stream_id = :sid AND #seq BETWEEN :one AND :next",
                vec![
                    (":sid", sid.clone()),
                    (":one", AttributeValue::N("1".into())),
                ],
            ),
            None => (
                "stream_id = :sid AND #seq >= :one",
                vec![
                    (":sid", sid.clone()),
                    (":one", AttributeValue::N("1".into())),
                ],
            ),
        };
        if let Some(next) = next_seq {
            ops_values.push((":next", AttributeValue::N(next.to_string())));
        }
        let ops_items = Pages {
            client: &self.client,
            table: &self.ops_table,
            key_condition: ops_condition,
            values: ops_values,
            names: vec![("#seq", "seq"), ("#op", "op_id"), ("#wc", "wall_clock")],
            projection: "#seq, #op, #wc",
            limit: page,
        }
        .read(ctx)?;
        let mut ops = PlannedKind::new("ops");
        let mut facts = OpsFacts::default();
        for item in &ops_items {
            let seq = n(item, "seq").ok_or_else(|| malformed("op"))?;
            let op_id = s(item, "op_id").unwrap_or_default().to_string();
            ops.keys.push(PlanKey::Op { seq, op_id });
            match s(item, "wall_clock").and_then(parse_rfc3339) {
                Some(t) => {
                    facts.newest_wall_clock = Some(facts.newest_wall_clock.map_or(t, |x| x.max(t)))
                }
                None => facts.wall_clock_unparseable += 1,
            }
        }

        let claim_items = Pages {
            client: &self.client,
            table: &self.ops_table,
            key_condition: "stream_id = :sid AND #seq < :zero",
            values: vec![
                (":sid", sid.clone()),
                (":zero", AttributeValue::N("0".into())),
            ],
            names: vec![("#seq", "seq")],
            projection: "#seq",
            limit: page,
        }
        .read(ctx)?;
        let mut claims = PlannedKind::new("claims");
        for item in &claim_items {
            claims.keys.push(PlanKey::Seq(
                n(item, "seq").ok_or_else(|| malformed("claim"))?,
            ));
        }

        let registry_items = Pages {
            client: &self.client,
            table: &self.registry_table,
            key_condition: "stream_id = :sid",
            values: vec![(":sid", sid.clone())],
            names: vec![
                ("#sk", "sk"),
                ("#exp", "expires_at"),
                ("#seen", "last_seen"),
            ],
            projection: "#sk, #exp, #seen",
            limit: page,
        }
        .read(ctx)?;
        let mut registry = PlannedKind::new("registry");
        let mut unknown = Vec::new();
        let mut newest_last_seen: Option<i64> = None;
        for item in &registry_items {
            let sk = s(item, "sk")
                .ok_or_else(|| malformed("registry"))?
                .to_string();
            let Some(prefix) = REGISTRY_PREFIXES.iter().find(|p| sk.starts_with(**p)) else {
                let name = name_unknown_prefix(&sk);
                if !unknown.contains(&name) {
                    unknown.push(name);
                }
                continue;
            };
            *registry.by_prefix.entry(prefix.to_string()).or_insert(0) += 1;
            if let Some(seen) = n(item, "last_seen") {
                newest_last_seen = Some(newest_last_seen.map_or(seen, |x| x.max(seen)));
            }
            // An item DynamoDB's TTL may already have removed is outside the digest (CQ 3).
            if n(item, "expires_at").is_some_and(|e| e < started) {
                registry.expired.push(PlanKey::Sk(sk));
            } else {
                registry.keys.push(PlanKey::Sk(sk));
            }
        }
        registry.keys.sort();

        let due_items = Pages {
            client: &self.client,
            table: &self.registry_table,
            key_condition: "stream_id = :p AND #sk = :id",
            values: vec![
                (":p", AttributeValue::S(DUE_PARTITION.into())),
                (":id", sid.clone()),
            ],
            names: vec![("#sk", "sk")],
            projection: "#sk",
            limit: None,
        }
        .read(ctx)?;
        let mut due = PlannedKind::new("due");
        if !due_items.is_empty() {
            // Never the stream id itself in the digest: the item is named by its kind.
            due.keys.push(PlanKey::Sk(DUE_PARTITION.into()));
        }

        let lease = self.registry.read(stream).map_err(PurgeError::Store)?;

        Ok(Plan {
            kinds: vec![ops, claims, counter, registry, due],
            ops: facts,
            unknown,
            lease,
            newest_last_seen,
        })
    }

    fn delete(
        &self,
        stream: &StreamId,
        plan: &Plan,
        _expected: &PlanDigest,
        _identity: &Value,
        ctx: &mut RunCtx<'_>,
        deleted: &mut Deleted,
    ) -> Result<Physical, PurgeError> {
        let target = stream.as_str().to_string();
        let ops_key = |sort: SortKey| ItemKey {
            partition: target.clone(),
            sort,
        };
        let keys_of = |kind: &str| -> Vec<ItemKey> {
            plan.kind(kind)
                .into_iter()
                .flat_map(|k| k.all_keys())
                .filter_map(|k| match k {
                    PlanKey::Op { seq, .. } | PlanKey::Seq(seq) => Some(ops_key(SortKey::N(*seq))),
                    PlanKey::Sk(sk) => Some(ops_key(SortKey::S(sk.clone()))),
                    PlanKey::Row { .. } => None,
                })
                .collect()
        };
        let in_target = |k: &ItemKey| k.partition == target;
        let due_key =
            |k: &ItemKey| k.partition == DUE_PARTITION && k.sort == SortKey::S(target.clone());
        let backoff = shipped_backoff;
        for (kind, step) in [("ops", "D1"), ("claims", "D2"), ("counter", "D3")] {
            delete_keys(
                self.deleter.as_ref(),
                &self.ops_table,
                kind,
                step,
                &keys_of(kind),
                &in_target,
                ctx,
                deleted,
                &backoff,
            )?;
        }
        delete_keys(
            self.deleter.as_ref(),
            &self.registry_table,
            "registry",
            "D4",
            &keys_of("registry"),
            &in_target,
            ctx,
            deleted,
            &backoff,
        )?;
        let due: Vec<ItemKey> = if plan.kind("due").is_some_and(|k| k.found() > 0) {
            vec![ItemKey {
                partition: DUE_PARTITION.into(),
                sort: SortKey::S(target.clone()),
            }]
        } else {
            Vec::new()
        };
        delete_keys(
            self.deleter.as_ref(),
            &self.registry_table,
            "due",
            "D5",
            &due,
            &due_key,
            ctx,
            deleted,
            &backoff,
        )?;
        Ok(Physical::Provider)
    }

    fn stream_lease(&self) -> Option<&dyn StreamLease> {
        Some(&self.registry)
    }

    #[cfg(feature = "test-seams")]
    fn inject(&self, stream: &StreamId, write: super::InjectWrite) -> Result<(), StoreError> {
        use crate::presence::PresenceStore;
        use crate::registry::PrefixRegistry;
        use crate::store::OpStore;
        use crate::store_ddb::DynamoDbOpStore;
        // The injected writes may go straight to DynamoDB, past the recording proxy a request
        // budget test puts in front of the purge's own endpoint.
        let endpoint = std::env::var("NXF_RELAY_TEST_PURGE_INJECT_ENDPOINT")
            .ok()
            .or_else(|| self.endpoint.clone());
        let region = Some(self.region.clone());
        let registry = || {
            DynamoDbPrefixRegistry::new(
                self.registry_table.clone(),
                endpoint.clone(),
                region.clone(),
            )
        };
        match write {
            super::InjectWrite::Op => {
                DynamoDbOpStore::new(self.ops_table.clone(), endpoint.clone(), region.clone())
                    .append(stream, &super::injected_op())
                    .map(|_| ())
            }
            super::InjectWrite::Machine => registry().announce(
                stream,
                &super::injected_hello(),
                super::unix_secs(std::time::SystemTime::now()),
                i64::MIN,
            ),
            super::InjectWrite::Prefix => {
                let (prefix, uuid) = super::injected_claim();
                registry().register(stream, &prefix, &uuid).map(|_| ())
            }
            super::InjectWrite::Steal => {
                // Overwrite the stream lease with another holder, unconditionally.
                let client = self.client.clone();
                let table = self.registry_table.clone();
                let sk = lease_sk(stream.as_str());
                let until = super::unix_secs(std::time::SystemTime::now()) + 3600;
                block_on(async move {
                    client
                        .put_item()
                        .table_name(&table)
                        .item("stream_id", AttributeValue::S(SWEEP_PARTITION.into()))
                        .item("sk", AttributeValue::S(sk))
                        .item("holder", AttributeValue::S("test-seam-thief".into()))
                        .item("expires_at", AttributeValue::N(until.to_string()))
                        .send()
                        .await
                        .map(|_| ())
                        .map_err(ddb_code_err)
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_dynamodb::types::{
        AttributeDefinition, GlobalSecondaryIndexDescription, KeySchemaElement,
        LocalSecondaryIndexDescription,
    };
    use std::sync::Mutex;

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

    fn table(name: &str, range: &str, range_type: ScalarAttributeType) -> TableDescription {
        TableDescription::builder()
            .table_name(name)
            .table_arn(format!(
                "arn:aws:dynamodb:eu-central-1:123456789012:table/{name}"
            ))
            .table_id(format!("id-{name}"))
            .creation_date_time(aws_sdk_dynamodb::primitives::DateTime::from_secs(
                1_756_720_800,
            ))
            .item_count(3)
            .key_schema(key("stream_id", KeyType::Hash))
            .key_schema(key(range, KeyType::Range))
            .attribute_definitions(attr("stream_id", ScalarAttributeType::S))
            .attribute_definitions(attr(range, range_type))
            .build()
    }

    fn ops() -> TableDescription {
        table("nxf_stream_ops", "seq", ScalarAttributeType::N)
    }

    fn registry() -> TableDescription {
        table("nxf_prefix_claims", "sk", ScalarAttributeType::S)
    }

    fn refused(ops: &TableDescription, registry: &TableDescription) -> String {
        match relay_tables_identity(ops, registry) {
            Err(PurgeError::NotARelayStore(rule)) => rule,
            other => panic!("expected NotARelayStore, got {other:?}"),
        }
    }

    #[test]
    fn a_relay_pair_passes_with_its_identity() {
        let id = relay_tables_identity(&ops(), &registry()).unwrap();
        assert_eq!(
            id.identity,
            json!({
                "ops_table_arn": "arn:aws:dynamodb:eu-central-1:123456789012:table/nxf_stream_ops",
                "ops_table_id": "id-nxf_stream_ops",
                "ops_created": "2025-09-01T10:00:00Z",
                "registry_table_arn": "arn:aws:dynamodb:eu-central-1:123456789012:table/nxf_prefix_claims",
                "registry_table_id": "id-nxf_prefix_claims",
                "registry_created": "2025-09-01T10:00:00Z",
            })
        );
        assert!(id.populated);
    }

    #[test]
    fn swapped_tables_are_refused() {
        assert!(refused(&registry(), &ops()).contains("ops table"));
    }

    #[test]
    fn the_same_table_twice_is_refused() {
        assert!(refused(&ops(), &ops()).contains("registry table"));
        let mut twin = registry();
        twin = TableDescription::builder()
            .set_table_name(twin.table_name().map(str::to_string))
            .set_key_schema(Some(twin.key_schema().to_vec()))
            .set_attribute_definitions(Some(twin.attribute_definitions().to_vec()))
            .set_table_arn(ops().table_arn().map(str::to_string))
            .build();
        assert!(refused(&ops(), &twin).contains("ARN"));
        let same_name = TableDescription::builder()
            .table_name("nxf_stream_ops")
            .set_key_schema(Some(registry().key_schema().to_vec()))
            .set_attribute_definitions(Some(registry().attribute_definitions().to_vec()))
            .table_arn("arn:other")
            .build();
        assert!(refused(&ops(), &same_name).contains("same table"));
    }

    #[test]
    fn a_table_with_a_secondary_index_is_refused() {
        let gsi = GlobalSecondaryIndexDescription::builder()
            .index_name("active")
            .build();
        let with_gsi = TableDescription::builder()
            .table_name("fold")
            .table_arn("arn:fold")
            .set_key_schema(Some(registry().key_schema().to_vec()))
            .set_attribute_definitions(Some(registry().attribute_definitions().to_vec()))
            .global_secondary_indexes(gsi)
            .build();
        assert!(refused(&ops(), &with_gsi).contains("global secondary index"));
        let lsi = LocalSecondaryIndexDescription::builder()
            .index_name("by_x")
            .build();
        let with_lsi = TableDescription::builder()
            .table_name("ops2")
            .table_arn("arn:ops2")
            .set_key_schema(Some(ops().key_schema().to_vec()))
            .set_attribute_definitions(Some(ops().attribute_definitions().to_vec()))
            .local_secondary_indexes(lsi)
            .build();
        assert!(refused(&with_lsi, &registry()).contains("local secondary index"));
    }

    #[test]
    fn the_board_fold_tables_shape_is_refused_as_the_registry() {
        // nxs_fold_ddb::ddb::create_table: pk/sk strings plus three GSIs.
        let fold = TableDescription::builder()
            .table_name("board")
            .table_arn("arn:board")
            .key_schema(key("pk", KeyType::Hash))
            .key_schema(key("sk", KeyType::Range))
            .attribute_definitions(attr("pk", ScalarAttributeType::S))
            .attribute_definitions(attr("sk", ScalarAttributeType::S))
            .build();
        assert!(refused(&ops(), &fold).contains("stream_id"));
    }

    #[test]
    fn the_right_names_with_the_wrong_types_are_refused() {
        let seq_as_string = table("ops_s", "seq", ScalarAttributeType::S);
        assert!(refused(&seq_as_string, &registry()).contains("types"));
        let sk_as_number = table("reg_n", "sk", ScalarAttributeType::N);
        assert!(refused(&ops(), &sk_as_number).contains("types"));
    }

    #[test]
    fn an_empty_ops_table_is_not_populated_and_missing_ids_are_null() {
        let empty = TableDescription::builder()
            .table_name("e")
            .table_arn("arn:e")
            .item_count(0)
            .set_key_schema(Some(ops().key_schema().to_vec()))
            .set_attribute_definitions(Some(ops().attribute_definitions().to_vec()))
            .build();
        let id = relay_tables_identity(&empty, &registry()).unwrap();
        assert!(!id.populated);
        assert_eq!(id.identity["ops_table_id"], Value::Null);
        assert_eq!(id.identity["ops_created"], Value::Null);
    }

    /// A fake `BatchWriteItem`: the first `unprocessed_for` calls of every batch leave half of it
    /// unprocessed.
    struct FakeDeleter {
        unprocessed_for: u32,
        calls: Mutex<Vec<usize>>,
        current: Mutex<(Vec<ItemKey>, u32)>,
    }

    impl FakeDeleter {
        fn new(unprocessed_for: u32) -> FakeDeleter {
            FakeDeleter {
                unprocessed_for,
                calls: Mutex::new(Vec::new()),
                current: Mutex::new((Vec::new(), 0)),
            }
        }
    }

    impl BatchDeleter for FakeDeleter {
        fn delete_batch(&self, _t: &str, keys: &[ItemKey]) -> Result<Vec<ItemKey>, StoreError> {
            self.calls.lock().unwrap().push(keys.len());
            let mut current = self.current.lock().unwrap();
            // A new batch starts when the keys are not a remainder of the last call's.
            if !keys.iter().all(|k| current.0.contains(k)) {
                *current = (keys.to_vec(), 0);
            }
            current.1 += 1;
            if current.1 <= self.unprocessed_for && keys.len() > 1 {
                let left = keys[keys.len() / 2..].to_vec();
                current.0 = left.clone();
                Ok(left)
            } else if current.1 <= self.unprocessed_for {
                Ok(keys.to_vec())
            } else {
                current.0.clear();
                Ok(Vec::new())
            }
        }
    }

    fn keys(n: usize) -> Vec<ItemKey> {
        (1..=n as i64)
            .map(|seq| ItemKey {
                partition: "n-h-1".into(),
                sort: SortKey::N(seq),
            })
            .collect()
    }

    fn run(deleter: &FakeDeleter, keys: &[ItemKey]) -> (Result<(), PurgeError>, Deleted) {
        let seams = crate::purge::PurgeSeams::shipped();
        let clock = std::time::SystemTime::now;
        let mut sink = |_: &str| {};
        let mut ctx = RunCtx::for_test(&seams, &clock, &mut sink);
        let mut deleted = Deleted::default();
        let result = delete_keys(
            deleter,
            "ops",
            "ops",
            "D1",
            keys,
            &|k| k.partition == "n-h-1",
            &mut ctx,
            &mut deleted,
            &|_| Duration::ZERO,
        );
        (result, deleted)
    }

    #[test]
    fn unprocessed_items_for_four_calls_still_complete() {
        let deleter = FakeDeleter::new(4);
        let (result, deleted) = run(&deleter, &keys(60));
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(deleted.get("ops"), 60);
        // 3 batches (25, 25, 10), 5 calls each.
        assert_eq!(deleter.calls.lock().unwrap().len(), 15);
    }

    #[test]
    fn unprocessed_items_for_five_calls_fail_with_exactly_what_went_counted() {
        let deleter = FakeDeleter::new(5);
        let (result, deleted) = run(&deleter, &keys(60));
        assert!(matches!(result, Err(PurgeError::Store(_))), "{result:?}");
        // Of the first batch of 25: halves processed in calls 1-5: 12 + 7 + 3 + 2 + 0 = 24? The
        // fake leaves keys[len/2..]: 25 → 13 left (12 went), 13 → 7 (6), 7 → 4 (3), 4 → 2 (2),
        // 2 → 1 (1): 24 went, 1 left, and the step stops.
        assert_eq!(deleted.get("ops"), 24);
        assert_eq!(deleter.calls.lock().unwrap().len(), 5);
    }

    #[test]
    fn batches_are_never_larger_than_twenty_five() {
        let deleter = FakeDeleter::new(0);
        let (result, deleted) = run(&deleter, &keys(101));
        assert!(result.is_ok());
        assert_eq!(deleted.get("ops"), 101);
        assert_eq!(*deleter.calls.lock().unwrap(), vec![25, 25, 25, 25, 1]);
    }

    #[test]
    fn a_key_of_another_stream_stops_the_step_before_it_is_sent() {
        let deleter = FakeDeleter::new(0);
        let mut bad = keys(30);
        bad[27].partition = "n-h-10".into();
        let (result, deleted) = run(&deleter, &bad);
        assert!(matches!(result, Err(PurgeError::Store(_))));
        assert_eq!(
            deleted.get("ops"),
            25,
            "the first batch went, the second never left"
        );
        assert_eq!(deleter.calls.lock().unwrap().len(), 1);
    }
}
