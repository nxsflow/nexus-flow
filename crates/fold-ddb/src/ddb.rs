//! [`DynamoDbTable`]: one stream's partition of the views table in DynamoDB, and [`create_table`],
//! the table the contract in the crate doc describes.
//!
//! Every request names the stream's partition key — a `BatchGetItem` names it in every key; the
//! board's three index reads name an index. There is
//! no Scan anywhere in this file, and the `dynamodb-local` CI job records every request a fold
//! makes to prove it stays that way.
//!
//! The adapter is async and runs on the caller's runtime — a fold run in a Lambda awaits it
//! directly, unlike the relay's op store, which bridges the SDK into its synchronous traits.

use crate::deny::{AccountHash, DenyList, DenyTable, DENY_KEY, DENY_TABLE_ENV};
use crate::layout::{
    typed_partition, ACTIVE, ACTIVE_INDEX, DATED, DATED_AT, DATED_INDEX, PK, SK, TYPED, TYPED_INDEX,
};
use crate::purge::{PurgeError, StoreIdentity};
use crate::table::{
    Dated, KeyRow, Outcome, Row, Table, BATCH_GET_KEYS, BATCH_WRITE_KEYS, INDEX_REVISION_ATTRIBUTE,
};
use crate::write::Write;
use aws_sdk_dynamodb::config::AsyncSleep;
use aws_sdk_dynamodb::error::{DisplayErrorContext, ProvideErrorMetadata};
use aws_sdk_dynamodb::operation::create_table::builders::CreateTableFluentBuilder;
use aws_sdk_dynamodb::types::{
    AttributeDefinition, AttributeValue, BillingMode, DeleteRequest, GlobalSecondaryIndex,
    KeySchemaElement, KeyType, KeysAndAttributes, Projection, ProjectionType, ScalarAttributeType,
    TableDescription, WriteRequest,
};
use aws_sdk_dynamodb::Client;
use nxs_foundation::change::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// How often [`Table::get_many`] sends the keys a `BatchGetItem` left unprocessed again before it
/// gives up, waiting twice as long each time from [`UNPROCESSED_BACKOFF_MS`].
const UNPROCESSED_RETRIES: u32 = 8;
const UNPROCESSED_BACKOFF_MS: u64 = 25;

/// A request DynamoDB refused, or an item this crate cannot read.
#[derive(Debug)]
pub struct DdbError(pub String);

impl std::fmt::Display for DdbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DynamoDB: {}", self.0)
    }
}

impl std::error::Error for DdbError {}

fn err<E: std::error::Error + 'static>(e: E) -> DdbError {
    DdbError(DisplayErrorContext(e).to_string())
}

/// An SDK error as the purge and the deny list may print it: the operation and the AWS error CODE,
/// never the SDK's message, which can quote a key or a value (log-id v1 C10).
fn code_err<E: ProvideErrorMetadata>(operation: &str, e: &E) -> DdbError {
    DdbError(format!(
        "{operation}: {}",
        e.code().unwrap_or("UnknownError")
    ))
}

/// The views table of `name`, as the deploying stack must provision it: keys, the three sparse
/// indexes (`active`, `dated`, `typed`) projecting every attribute, on-demand billing. Production code never sends it; the
/// integration tests do, and a stack is checked against it.
pub fn create_table(client: &Client, name: &str) -> CreateTableFluentBuilder {
    let s = |n: &str| {
        AttributeDefinition::builder()
            .attribute_name(n)
            .attribute_type(ScalarAttributeType::S)
            .build()
            .expect("name and type are set")
    };
    let key = |n: &str, t: KeyType| {
        KeySchemaElement::builder()
            .attribute_name(n)
            .key_type(t)
            .build()
            .expect("name and type are set")
    };
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
            .expect("name, keys and projection are set")
    };
    client
        .create_table()
        .table_name(name)
        .billing_mode(BillingMode::PayPerRequest)
        .attribute_definitions(s(PK))
        .attribute_definitions(s(SK))
        .attribute_definitions(s(ACTIVE))
        .attribute_definitions(s(DATED))
        .attribute_definitions(s(DATED_AT))
        .attribute_definitions(s(TYPED))
        .key_schema(key(PK, KeyType::Hash))
        .key_schema(key(SK, KeyType::Range))
        .global_secondary_indexes(index(ACTIVE_INDEX, ACTIVE, SK))
        .global_secondary_indexes(index(DATED_INDEX, DATED, DATED_AT))
        .global_secondary_indexes(index(TYPED_INDEX, TYPED, SK))
}

/// One stream's partition of the views table `table`.
#[derive(Debug, Clone)]
pub struct DynamoDbTable {
    client: Client,
    table: String,
    stream: String,
}

impl DynamoDbTable {
    pub fn new(client: Client, table: &str, stream: &str) -> DynamoDbTable {
        DynamoDbTable {
            client,
            table: table.to_string(),
            stream: stream.to_string(),
        }
    }

    fn key(&self, sk: &str) -> HashMap<String, AttributeValue> {
        HashMap::from([
            (PK.to_string(), AttributeValue::S(self.stream.clone())),
            (SK.to_string(), AttributeValue::S(sk.to_string())),
        ])
    }

    /// Every page of a Query — the base table when `index` is `None`.
    async fn query_all(
        &self,
        index: Option<&str>,
        key_condition: &str,
        names: &[(&str, &str)],
        values: &[(&str, AttributeValue)],
        newest_first: bool,
        limit: Option<usize>,
    ) -> Result<Vec<Row>, DdbError> {
        let mut rows = Vec::new();
        let mut start: Option<HashMap<String, AttributeValue>> = None;
        loop {
            let mut q = self
                .client
                .query()
                .table_name(&self.table)
                .key_condition_expression(key_condition)
                .set_exclusive_start_key(start.take())
                .scan_index_forward(!newest_first);
            q = match index {
                Some(i) => q.index_name(i),
                // Only the base table reads strongly consistent; an index cannot.
                None => q.consistent_read(true),
            };
            for (p, n) in names {
                q = q.expression_attribute_names(*p, *n);
            }
            for (p, v) in values {
                q = q.expression_attribute_values(*p, v.clone());
            }
            if let Some(n) = limit {
                q = q.limit(i32::try_from(n.saturating_sub(rows.len()).max(1)).unwrap_or(i32::MAX));
            }
            let page = q.send().await.map_err(err)?;
            for item in page.items() {
                rows.push(from_item(item)?);
            }
            if limit.is_some_and(|n| rows.len() >= n) {
                rows.truncate(limit.unwrap_or(usize::MAX));
                break;
            }
            match page.last_evaluated_key() {
                Some(k) if !k.is_empty() => start = Some(k.clone()),
                _ => break,
            }
        }
        Ok(rows)
    }
}

fn to_attr(cell: &Cell) -> AttributeValue {
    match cell {
        Cell::Null => AttributeValue::Null(true),
        Cell::Text(v) => AttributeValue::S(v.clone()),
        Cell::Int(v) => AttributeValue::N(v.to_string()),
    }
}

fn from_item(item: &HashMap<String, AttributeValue>) -> Result<Row, DdbError> {
    item.iter()
        .map(|(name, value)| {
            let cell = match value {
                AttributeValue::Null(_) => Cell::Null,
                AttributeValue::S(v) => Cell::Text(v.clone()),
                AttributeValue::N(v) => Cell::Int(v.parse().map_err(|_| {
                    DdbError(format!("attribute {name:?} holds {v:?}, not an integer"))
                })?),
                other => {
                    return Err(DdbError(format!(
                        "attribute {name:?} holds {other:?}, which no view writes"
                    )))
                }
            };
            Ok((name.clone(), cell))
        })
        .collect()
}

impl Table for DynamoDbTable {
    type Error = DdbError;

    fn stream(&self) -> &str {
        &self.stream
    }

    async fn get(&self, sk: &str) -> Result<Option<Row>, DdbError> {
        let out = self
            .client
            .get_item()
            .table_name(&self.table)
            .set_key(Some(self.key(sk)))
            .consistent_read(true)
            .send()
            .await
            .map_err(err)?;
        out.item().map(from_item).transpose()
    }

    async fn get_many(&self, sks: &[String]) -> Result<BTreeMap<String, Row>, DdbError> {
        let keys: Vec<&String> = sks.iter().collect::<BTreeSet<_>>().into_iter().collect();
        let mut rows = BTreeMap::new();
        for chunk in keys.chunks(BATCH_GET_KEYS) {
            let mut pending: Vec<HashMap<String, AttributeValue>> =
                chunk.iter().map(|sk| self.key(sk)).collect();
            let mut retries = 0;
            while !pending.is_empty() {
                let request = KeysAndAttributes::builder()
                    .set_keys(Some(pending))
                    .consistent_read(true)
                    .build()
                    .map_err(err)?;
                let out = self
                    .client
                    .batch_get_item()
                    .request_items(&self.table, request)
                    .send()
                    .await
                    .map_err(err)?;
                for item in out
                    .responses()
                    .and_then(|r| r.get(&self.table))
                    .into_iter()
                    .flatten()
                {
                    let row = from_item(item)?;
                    if let Some(Cell::Text(sk)) = row.get(SK) {
                        rows.insert(sk.clone(), row.clone());
                    }
                }
                pending = out
                    .unprocessed_keys()
                    .and_then(|u| u.get(&self.table))
                    .map(|k| k.keys().to_vec())
                    .unwrap_or_default();
                if !pending.is_empty() {
                    // Throttled, or past the 16 MB a response carries: the rest again, after a
                    // pause that doubles, as DynamoDB asks of a caller.
                    if retries == UNPROCESSED_RETRIES {
                        return Err(DdbError(format!(
                            "BatchGetItem left {} keys unprocessed after {UNPROCESSED_RETRIES} \
                             retries",
                            pending.len()
                        )));
                    }
                    if let Some(sleep) = self.client.config().sleep_impl() {
                        sleep
                            .sleep(std::time::Duration::from_millis(
                                UNPROCESSED_BACKOFF_MS << retries,
                            ))
                            .await;
                    }
                    retries += 1;
                }
            }
        }
        Ok(rows)
    }

    async fn write(&self, write: &Write) -> Result<Outcome, DdbError> {
        let e = write.expression();
        let mut req = self
            .client
            .update_item()
            .table_name(&self.table)
            .set_key(Some(self.key(&write.sk)));
        // Without an update expression an update still creates the item, with its keys alone.
        if !e.update.is_empty() {
            req = req.update_expression(&e.update);
        }
        if let Some(c) = &e.condition {
            req = req.condition_expression(c);
        }
        for (p, n) in &e.names {
            req = req.expression_attribute_names(p, n);
        }
        for (p, v) in &e.values {
            req = req.expression_attribute_values(p, to_attr(v));
        }
        match req.send().await {
            Ok(_) => Ok(Outcome::Written),
            Err(e)
                if e.as_service_error()
                    .is_some_and(|s| s.is_conditional_check_failed_exception()) =>
            {
                Ok(Outcome::Kept)
            }
            // "Item size has exceeded the maximum allowed size" / "Item size to update has
            // exceeded…": the row stays as it was, as with any refused write.
            Err(e)
                if e.code() == Some("ValidationException")
                    && e.message().is_some_and(|m| m.contains("size")) =>
            {
                Ok(Outcome::TooLarge)
            }
            Err(e) => Err(DdbError(format!(
                "{} ({})",
                DisplayErrorContext(&e),
                e.code().unwrap_or("no code")
            ))),
        }
    }

    async fn delete(&self, sk: &str) -> Result<(), DdbError> {
        self.client
            .delete_item()
            .table_name(&self.table)
            .set_key(Some(self.key(sk)))
            .send()
            .await
            .map_err(err)?;
        Ok(())
    }

    async fn query_prefix(&self, prefix: &str) -> Result<Vec<Row>, DdbError> {
        let partition = ("#pk", PK);
        let stream = (":pk", AttributeValue::S(self.stream.clone()));
        if prefix.is_empty() {
            return self
                .query_all(None, "#pk = :pk", &[partition], &[stream], false, None)
                .await;
        }
        self.query_all(
            None,
            "#pk = :pk AND begins_with(#sk, :prefix)",
            &[partition, ("#sk", SK)],
            &[stream, (":prefix", AttributeValue::S(prefix.to_string()))],
            false,
            None,
        )
        .await
    }

    async fn query_active(&self) -> Result<Vec<Row>, DdbError> {
        self.query_all(
            Some(ACTIVE_INDEX),
            "#a = :a",
            &[("#a", ACTIVE)],
            &[(":a", AttributeValue::S(self.stream.clone()))],
            false,
            None,
        )
        .await
    }

    async fn query_dated(&self, lane: Dated, limit: Option<usize>) -> Result<Vec<Row>, DdbError> {
        self.query_all(
            Some(DATED_INDEX),
            "#d = :d",
            &[("#d", DATED)],
            &[(":d", AttributeValue::S(lane.partition(&self.stream)))],
            true,
            limit,
        )
        .await
    }

    async fn query_typed(&self, item_type: &str) -> Result<Vec<Row>, DdbError> {
        self.query_all(
            Some(TYPED_INDEX),
            "#t = :t",
            &[("#t", TYPED)],
            &[(
                ":t",
                AttributeValue::S(typed_partition(&self.stream, item_type)),
            )],
            false,
            None,
        )
        .await
    }

    /// One Query page of the partition (the purge's plan): `stream_id = :id`, the keys and the
    /// watermark's `folded_through` and `revision_index` projected, strongly consistent, `Limit`,
    /// from `ExclusiveStartKey (stream_id, after)`. DynamoDB's own `LastEvaluatedKey` is the cursor,
    /// so a page it cut at 1 MB is continued, not taken for the end. Errors carry the code only.
    async fn sort_keys_after(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<KeyRow>, Option<String>), DdbError> {
        let start = after.map(|sk| self.key(sk));
        let out = self
            .client
            .query()
            .table_name(&self.table)
            .key_condition_expression("#pk = :pk")
            .projection_expression("#pk, #sk, #ft, #ri")
            .expression_attribute_names("#pk", PK)
            .expression_attribute_names("#sk", SK)
            .expression_attribute_names("#ft", "folded_through")
            .expression_attribute_names("#ri", INDEX_REVISION_ATTRIBUTE)
            .expression_attribute_values(":pk", AttributeValue::S(self.stream.clone()))
            .consistent_read(true)
            .limit(i32::try_from(limit).unwrap_or(i32::MAX))
            .set_exclusive_start_key(start)
            .send()
            .await
            .map_err(|e| code_err("Query", &e))?;
        let mut keys = Vec::with_capacity(out.items().len());
        for item in out.items() {
            let s = |name: &str| item.get(name).and_then(|v| v.as_s().ok()).cloned();
            let n = |name: &str| {
                item.get(name)
                    .and_then(|v| v.as_n().ok())
                    .and_then(|v| v.parse::<i64>().ok())
            };
            let (Some(stream), Some(sk)) = (s(PK), s(SK)) else {
                return Err(DdbError("Query: an item without its keys".into()));
            };
            keys.push(KeyRow {
                stream,
                sk,
                folded_through: n("folded_through"),
                index_revision: n(INDEX_REVISION_ATTRIBUTE),
            });
        }
        let cursor = out
            .last_evaluated_key()
            .filter(|k| !k.is_empty())
            .and_then(|k| k.get(SK))
            .and_then(|v| v.as_s().ok())
            .cloned();
        Ok((keys, cursor))
    }

    /// One `BatchWriteItem` of up to 25 `DeleteRequest`s on this partition; the sort keys DynamoDB
    /// returned as unprocessed. Errors carry the code only.
    async fn delete_batch(&self, sks: &[String]) -> Result<Vec<String>, DdbError> {
        if sks.is_empty() {
            return Ok(Vec::new());
        }
        if sks.len() > BATCH_WRITE_KEYS {
            return Err(DdbError(format!(
                "BatchWriteItem: {} keys, past the {BATCH_WRITE_KEYS} one request names",
                sks.len()
            )));
        }
        let requests: Vec<WriteRequest> = sks
            .iter()
            .map(|sk| {
                WriteRequest::builder()
                    .delete_request(
                        DeleteRequest::builder()
                            .set_key(Some(self.key(sk)))
                            .build()
                            .expect("a delete request with a key builds"),
                    )
                    .build()
            })
            .collect();
        let out = self
            .client
            .batch_write_item()
            .request_items(&self.table, requests)
            .send()
            .await
            .map_err(|e| code_err("BatchWriteItem", &e))?;
        let mut left = Vec::new();
        for request in out
            .unprocessed_items()
            .and_then(|m| m.get(&self.table))
            .into_iter()
            .flatten()
        {
            let sk = request
                .delete_request()
                .and_then(|d| d.key().get(SK))
                .and_then(|v| v.as_s().ok())
                .ok_or_else(|| {
                    DdbError("BatchWriteItem: an unprocessed item without a key".into())
                })?;
            left.push(sk.clone());
        }
        Ok(left)
    }

    /// 50 ms doubled per attempt, on the client's own sleep.
    async fn backoff(&self, attempt: u32) {
        if let Some(sleep) = self.client.config().sleep_impl() {
            sleep
                .sleep(std::time::Duration::from_millis(50 << attempt.min(6)))
                .await;
        }
    }
}

// ----- the purge's table gate (6j6v.dbmd) -------------------------------------------------------

/// The key schema of one index or table: `(attribute, key type)` pairs.
fn keys_of(schema: &[KeySchemaElement]) -> Vec<(&str, &KeyType)> {
    schema
        .iter()
        .map(|k| (k.attribute_name(), k.key_type()))
        .collect()
}

/// Whether `desc` is a board-fold table as [`create_table`] makes it, and its identity: key schema
/// exactly HASH `stream_id` (S) + RANGE `sk` (S); no local secondary index; the three global ones,
/// `active`, `dated` and `typed`, each with `create_table`'s key schema. Each GSI is judged by its
/// name and key schema and NEVER its `IndexStatus`, so a table whose indexes are still `CREATING`
/// right after it was made passes. The relay's ops table (RANGE `seq`, N) and registry table (no
/// GSI) are refused, and so is a board-fold table from before `typed`. Pure, so every rule is
/// unit-tested on constructed descriptions, states DynamoDB Local cannot produce included.
/// `region` is the report's only.
pub fn fold_table_identity(
    desc: &TableDescription,
    region: &str,
) -> Result<StoreIdentity, PurgeError> {
    let refuse = |rule: String| Err(PurgeError::NotAFoldTable(rule));
    let type_of = |name: &str| {
        desc.attribute_definitions()
            .iter()
            .find(|a| a.attribute_name() == name)
            .map(|a| a.attribute_type().clone())
    };
    let table = keys_of(desc.key_schema());
    if table != [(PK, &KeyType::Hash), (SK, &KeyType::Range)] {
        return refuse(format!(
            "the table's key is not exactly {PK} (HASH) + {SK} (RANGE)"
        ));
    }
    if type_of(PK) != Some(ScalarAttributeType::S) || type_of(SK) != Some(ScalarAttributeType::S) {
        return refuse(format!("{PK} and {SK} are not both strings (S)"));
    }
    if !desc.local_secondary_indexes().is_empty() {
        return refuse(
            "the table has a local secondary index (a board-fold table has none)".into(),
        );
    }
    for (index, hash, range) in [
        (ACTIVE_INDEX, ACTIVE, SK),
        (DATED_INDEX, DATED, DATED_AT),
        (TYPED_INDEX, TYPED, SK),
    ] {
        let Some(gsi) = desc
            .global_secondary_indexes()
            .iter()
            .find(|g| g.index_name() == Some(index))
        else {
            return refuse(format!("the global secondary index `{index}` is missing"));
        };
        if keys_of(gsi.key_schema()) != [(hash, &KeyType::Hash), (range, &KeyType::Range)] {
            return refuse(format!(
                "the global secondary index `{index}` is not {hash} (HASH) + {range} (RANGE)"
            ));
        }
    }
    let Some(arn) = desc.table_arn() else {
        return refuse("the table's description carries no TableArn".into());
    };
    let created = desc.creation_date_time().and_then(|t| {
        t.fmt(aws_sdk_dynamodb::primitives::DateTimeFormat::DateTime)
            .ok()
    });
    Ok(StoreIdentity {
        backend: "dynamodb",
        fold_table: desc.table_name().unwrap_or_default().to_string(),
        region: region.to_string(),
        table_arn: arn.to_string(),
        table_id: desc.table_id().map(str::to_string),
        created,
        populated: desc.item_count().unwrap_or(0) > 0,
    })
}

/// `DescribeTable` of `table`, or the purge's refusal: a missing table is `NotAFoldTable` (78) and
/// nothing is created; any other failure is `Table` with its code (1).
async fn describe(
    client: &Client,
    table: &str,
    missing: impl FnOnce() -> PurgeError,
) -> Result<TableDescription, PurgeError> {
    use aws_sdk_dynamodb::operation::describe_table::DescribeTableError;
    match client.describe_table().table_name(table).send().await {
        Ok(out) => out
            .table()
            .cloned()
            .ok_or_else(|| PurgeError::Table("DescribeTable: no description".into())),
        Err(e) => match e.into_service_error() {
            DescribeTableError::ResourceNotFoundException(_) => Err(missing()),
            other => Err(PurgeError::Table(code_err("DescribeTable", &other).0)),
        },
    }
}

/// `DescribeTable`, then [`fold_table_identity`]: the table gate of every purge mode, before any
/// read of a partition.
pub async fn check_fold_table(client: &Client, table: &str) -> Result<StoreIdentity, PurgeError> {
    let desc = describe(client, table, || {
        PurgeError::NotAFoldTable("the table NXF_FOLD_DDB_TABLE names does not exist".into())
    })
    .await?;
    let region = client
        .config()
        .region()
        .map(|r| r.as_ref().to_string())
        .unwrap_or_default();
    fold_table_identity(&desc, &region)
}

// ----- the deny table (6j6v.dbmd round 4b) ------------------------------------------------------

/// The deny table as the hosted block must provision it: HASH `h` (S) and nothing else, on-demand.
/// Production code never sends it; the tests do, and a stack is checked against it. (Deletion
/// protection, a retain policy and PITR are the stack's; the crate doc's "Deny table" section.)
pub fn create_deny_table(client: &Client, name: &str) -> CreateTableFluentBuilder {
    client
        .create_table()
        .table_name(name)
        .billing_mode(BillingMode::PayPerRequest)
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name(DENY_KEY)
                .attribute_type(ScalarAttributeType::S)
                .build()
                .expect("name and type are set"),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name(DENY_KEY)
                .key_type(KeyType::Hash)
                .build()
                .expect("name and type are set"),
        )
}

/// Pure: the key schema is exactly HASH `h` (S) — no RANGE key, no GSI, no LSI. `Err` names the
/// rule.
pub fn deny_table_shape(desc: &TableDescription) -> Result<(), &'static str> {
    let keys = keys_of(desc.key_schema());
    if keys.iter().any(|(_, t)| **t == KeyType::Range) {
        return Err("a deny table has no RANGE key");
    }
    if keys != [(DENY_KEY, &KeyType::Hash)] {
        return Err("a deny table's only key is h (HASH)");
    }
    let h_type = desc
        .attribute_definitions()
        .iter()
        .find(|a| a.attribute_name() == DENY_KEY)
        .map(|a| a.attribute_type());
    if h_type != Some(&ScalarAttributeType::S) {
        return Err("a deny table's key h is a string (S)");
    }
    if !desc.global_secondary_indexes().is_empty() {
        return Err("a deny table has no global secondary index");
    }
    if !desc.local_secondary_indexes().is_empty() {
        return Err("a deny table has no local secondary index");
    }
    Ok(())
}

/// Why the deny list could not be opened. Board-fold folds NOTHING on any of them; the purge exits
/// 78 (`NotADenyTable`) on the first three and 1 on `Aws`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenyTableError {
    /// `NXS_DENY_TABLE` is unset or empty.
    Unset,
    /// The table it names does not exist (nothing is created).
    Missing,
    /// The table it names is not a deny table: the rule it broke.
    NotADenyTable(&'static str),
    /// `DescribeTable` failed otherwise: the operation and the AWS code.
    Aws(String),
}

impl std::fmt::Display for DenyTableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DenyTableError::Unset => write!(
                f,
                "{DENY_TABLE_ENV} is not set; without the deny table nothing is folded or purged"
            ),
            DenyTableError::Missing => write!(f, "the table {DENY_TABLE_ENV} names does not exist"),
            DenyTableError::NotADenyTable(rule) => {
                write!(
                    f,
                    "the table {DENY_TABLE_ENV} names is not a deny table: {rule}"
                )
            }
            DenyTableError::Aws(code) => write!(f, "{code}"),
        }
    }
}

impl std::error::Error for DenyTableError {}

impl From<DenyTableError> for PurgeError {
    fn from(e: DenyTableError) -> PurgeError {
        match e {
            DenyTableError::Aws(code) => PurgeError::Table(code),
            other => PurgeError::NotADenyTable(other.to_string()),
        }
    }
}

/// `DescribeTable`, then [`deny_table_shape`]; the table's `TableArn` on success. A missing table
/// is an error; nothing is ever created.
pub async fn check_deny_table(
    client: &Client,
    table: &str,
) -> Result<Option<String>, DenyTableError> {
    use aws_sdk_dynamodb::operation::describe_table::DescribeTableError;
    let desc = match client.describe_table().table_name(table).send().await {
        Ok(out) => out
            .table()
            .cloned()
            .ok_or_else(|| DenyTableError::Aws("DescribeTable: no description".into()))?,
        Err(e) => {
            return Err(match e.into_service_error() {
                DescribeTableError::ResourceNotFoundException(_) => DenyTableError::Missing,
                other => DenyTableError::Aws(code_err("DescribeTable", &other).0),
            })
        }
    };
    deny_table_shape(&desc).map_err(DenyTableError::NotADenyTable)?;
    Ok(desc.table_arn().map(str::to_string))
}

/// The shared deny list in DynamoDB, as board-fold and `nxf-fold-purge` read it.
#[derive(Debug, Clone)]
pub struct DynamoDbDenyList {
    client: Client,
    table: DenyTable,
}

impl DynamoDbDenyList {
    /// `NXS_DENY_TABLE` required and non-empty, then [`check_deny_table`]. Board-fold calls it at
    /// start and folds NOTHING on `Err`.
    pub async fn from_env(client: Client) -> Result<DynamoDbDenyList, DenyTableError> {
        DynamoDbDenyList::open(client, std::env::var(DENY_TABLE_ENV).ok().as_deref()).await
    }

    /// [`DynamoDbDenyList::from_env`] over the variable's value (`None` when unset), for a caller
    /// that reads its environment itself.
    pub async fn open(
        client: Client,
        table: Option<&str>,
    ) -> Result<DynamoDbDenyList, DenyTableError> {
        let name = table
            .filter(|t| !t.is_empty())
            .ok_or(DenyTableError::Unset)?;
        let table_arn = check_deny_table(&client, name).await?;
        Ok(DynamoDbDenyList {
            client,
            table: DenyTable {
                name: name.to_string(),
                table_arn,
            },
        })
    }
}

/// `GetItem { h: <h> }`, `ConsistentRead`, `ProjectionExpression #h`: an item present is a denial,
/// whatever else it carries. No cache, neither for a hit nor for a miss. An error carries the code
/// only.
impl DenyList for DynamoDbDenyList {
    type Error = DdbError;

    fn table(&self) -> Option<&DenyTable> {
        Some(&self.table)
    }

    async fn is_denied(&self, account: &AccountHash) -> Result<bool, DdbError> {
        let out = self
            .client
            .get_item()
            .table_name(&self.table.name)
            .key(DENY_KEY, AttributeValue::S(account.as_str().to_string()))
            .consistent_read(true)
            .projection_expression("#h")
            .expression_attribute_names("#h", DENY_KEY)
            .send()
            .await
            .map_err(|e| code_err("GetItem", &e))?;
        Ok(out.item().is_some_and(|item| !item.is_empty()))
    }
}

#[cfg(test)]
mod gate_tests {
    //! The two table gates on constructed `TableDescription`s (6j6v.dbmd round 3, CQ 8): every rule,
    //! including states DynamoDB Local cannot produce — a GSI still `CREATING`, an LSI.

    use super::*;
    use aws_sdk_dynamodb::types::{
        GlobalSecondaryIndexDescription, IndexStatus, LocalSecondaryIndexDescription,
    };

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

    fn gsi(
        name: &str,
        hash: &str,
        range: &str,
        status: IndexStatus,
    ) -> GlobalSecondaryIndexDescription {
        GlobalSecondaryIndexDescription::builder()
            .index_name(name)
            .key_schema(key(hash, KeyType::Hash))
            .key_schema(key(range, KeyType::Range))
            .index_status(status)
            .build()
    }

    /// A table as `create_table` makes it, right after the request: every GSI `CREATING`.
    fn fold_table() -> aws_sdk_dynamodb::types::builders::TableDescriptionBuilder {
        let mut b = TableDescription::builder()
            .table_name("board_fold")
            .table_arn("arn:aws:dynamodb:eu-central-1:123456789012:table/board_fold")
            .table_id("1b2c3d4e-0000-4000-8000-000000000001")
            .item_count(3)
            .key_schema(key(PK, KeyType::Hash))
            .key_schema(key(SK, KeyType::Range));
        for a in [PK, SK, ACTIVE, DATED, DATED_AT, TYPED] {
            b = b.attribute_definitions(attr(a, ScalarAttributeType::S));
        }
        b.global_secondary_indexes(gsi(ACTIVE_INDEX, ACTIVE, SK, IndexStatus::Creating))
            .global_secondary_indexes(gsi(DATED_INDEX, DATED, DATED_AT, IndexStatus::Creating))
            .global_secondary_indexes(gsi(TYPED_INDEX, TYPED, SK, IndexStatus::Creating))
    }

    fn refused(desc: &TableDescription) -> String {
        match fold_table_identity(desc, "eu-central-1") {
            Err(PurgeError::NotAFoldTable(rule)) => rule,
            other => panic!("refused, got {other:?}"),
        }
    }

    #[test]
    fn a_table_as_create_table_makes_it_passes_while_its_indexes_are_creating() {
        let identity = fold_table_identity(&fold_table().build(), "eu-central-1").unwrap();
        assert_eq!(identity.fold_table, "board_fold");
        assert_eq!(
            identity.table_id.as_deref(),
            Some("1b2c3d4e-0000-4000-8000-000000000001")
        );
        assert!(identity.populated);
        assert_eq!(identity.region, "eu-central-1");
    }

    #[test]
    fn a_missing_or_misshapen_index_refuses_naming_it() {
        for missing in [ACTIVE_INDEX, DATED_INDEX, TYPED_INDEX] {
            let mut desc = fold_table().build();
            let kept: Vec<_> = desc
                .global_secondary_indexes()
                .iter()
                .filter(|g| g.index_name() != Some(missing))
                .cloned()
                .collect();
            desc = TableDescription::builder()
                .set_table_name(desc.table_name().map(str::to_string))
                .set_table_arn(desc.table_arn().map(str::to_string))
                .set_key_schema(Some(desc.key_schema().to_vec()))
                .set_attribute_definitions(Some(desc.attribute_definitions().to_vec()))
                .set_global_secondary_indexes(Some(kept))
                .build();
            let rule = refused(&desc);
            assert!(rule.contains(missing) && rule.contains("missing"), "{rule}");
        }
        let wrong = fold_table()
            .set_global_secondary_indexes(Some(vec![
                gsi(ACTIVE_INDEX, ACTIVE, SK, IndexStatus::Active),
                gsi(DATED_INDEX, DATED, SK, IndexStatus::Active),
                gsi(TYPED_INDEX, TYPED, SK, IndexStatus::Active),
            ]))
            .build();
        assert!(refused(&wrong).contains("`dated` is not"));
    }

    #[test]
    fn an_lsi_a_numeric_range_key_and_another_hash_key_refuse() {
        let lsi = fold_table()
            .local_secondary_indexes(
                LocalSecondaryIndexDescription::builder()
                    .index_name("by_x")
                    .key_schema(key(PK, KeyType::Hash))
                    .key_schema(key("x", KeyType::Range))
                    .build(),
            )
            .build();
        assert!(refused(&lsi).contains("local secondary index"));

        let numeric = fold_table()
            .set_attribute_definitions(Some(vec![
                attr(PK, ScalarAttributeType::S),
                attr(SK, ScalarAttributeType::N),
            ]))
            .build();
        assert!(refused(&numeric).contains("strings"));

        let relay_ops = fold_table()
            .set_key_schema(Some(vec![
                key(PK, KeyType::Hash),
                key("seq", KeyType::Range),
            ]))
            .build();
        assert!(refused(&relay_ops).contains("key is not exactly"));

        let other_hash = fold_table()
            .set_key_schema(Some(vec![
                key("pk", KeyType::Hash),
                key(SK, KeyType::Range),
            ]))
            .build();
        assert!(refused(&other_hash).contains("key is not exactly"));

        let no_arn = fold_table().set_table_arn(None).build();
        assert!(refused(&no_arn).contains("TableArn"));
    }

    #[test]
    fn the_identity_changes_with_the_table_id_and_the_creation_time() {
        let a = fold_table_identity(&fold_table().build(), "r").unwrap();
        let b = fold_table_identity(
            &fold_table()
                .table_id("1b2c3d4e-0000-4000-8000-000000000002")
                .build(),
            "r",
        )
        .unwrap();
        assert_ne!(
            a.digest(),
            b.digest(),
            "a re-created table has a new TableId"
        );
        let c = fold_table_identity(
            &fold_table()
                .creation_date_time(aws_sdk_dynamodb::primitives::DateTime::from_secs(
                    1_790_000_000,
                ))
                .build(),
            "r",
        )
        .unwrap();
        assert_ne!(a.digest(), c.digest());
        assert_eq!(c.created.as_deref(), Some("2026-09-21T14:13:20Z"));
        // The name and the region are the report's only: never digested.
        let d =
            fold_table_identity(&fold_table().table_name("renamed").build(), "elsewhere").unwrap();
        assert_eq!(a.digest(), d.digest());
    }

    fn deny_table() -> aws_sdk_dynamodb::types::builders::TableDescriptionBuilder {
        TableDescription::builder()
            .table_name("deny")
            .table_arn("arn:aws:dynamodb:eu-central-1:123456789012:table/deny")
            .key_schema(key(DENY_KEY, KeyType::Hash))
            .attribute_definitions(attr(DENY_KEY, ScalarAttributeType::S))
    }

    #[test]
    fn the_deny_table_gate_passes_create_deny_table_and_names_every_refusal() {
        assert_eq!(deny_table_shape(&deny_table().build()), Ok(()));
        let range = deny_table()
            .key_schema(key("since", KeyType::Range))
            .attribute_definitions(attr("since", ScalarAttributeType::S))
            .build();
        assert_eq!(
            deny_table_shape(&range),
            Err("a deny table has no RANGE key")
        );
        let other_hash = TableDescription::builder()
            .key_schema(key("account", KeyType::Hash))
            .attribute_definitions(attr("account", ScalarAttributeType::S))
            .build();
        assert_eq!(
            deny_table_shape(&other_hash),
            Err("a deny table's only key is h (HASH)")
        );
        let numeric = TableDescription::builder()
            .key_schema(key(DENY_KEY, KeyType::Hash))
            .attribute_definitions(attr(DENY_KEY, ScalarAttributeType::N))
            .build();
        assert_eq!(
            deny_table_shape(&numeric),
            Err("a deny table's key h is a string (S)")
        );
        let with_gsi = deny_table()
            .global_secondary_indexes(gsi("by_since", "since", DENY_KEY, IndexStatus::Active))
            .build();
        assert_eq!(
            deny_table_shape(&with_gsi),
            Err("a deny table has no global secondary index")
        );
        let with_lsi = deny_table()
            .local_secondary_indexes(
                LocalSecondaryIndexDescription::builder()
                    .index_name("by_since")
                    .key_schema(key(DENY_KEY, KeyType::Hash))
                    .key_schema(key("since", KeyType::Range))
                    .build(),
            )
            .build();
        assert_eq!(
            deny_table_shape(&with_lsi),
            Err("a deny table has no local secondary index")
        );
        // The board-fold table and the relay's ops table are both refused as deny tables.
        assert_eq!(
            deny_table_shape(&fold_table().build()),
            Err("a deny table has no RANGE key")
        );
    }
}
