//! [`DynamoDbTable`]: one stream's partition of the views table in DynamoDB, and [`create_table`],
//! the table the contract in the crate doc describes.
//!
//! Every request names the stream's partition key; the board's two reads name an index. There is
//! no Scan anywhere in this file, and the `dynamodb-local` CI job records every request a fold
//! makes to prove it stays that way.
//!
//! The adapter is async and runs on the caller's runtime — a fold run in a Lambda awaits it
//! directly, unlike the relay's op store, which bridges the SDK into its synchronous traits.

use crate::layout::{ACTIVE, ACTIVE_INDEX, DATED, DATED_AT, DATED_INDEX, PK, SK};
use crate::table::{Dated, Outcome, Row, Table};
use crate::write::Write;
use aws_sdk_dynamodb::error::{DisplayErrorContext, ProvideErrorMetadata};
use aws_sdk_dynamodb::operation::create_table::builders::CreateTableFluentBuilder;
use aws_sdk_dynamodb::types::{
    AttributeDefinition, AttributeValue, BillingMode, GlobalSecondaryIndex, KeySchemaElement,
    KeyType, Projection, ProjectionType, ScalarAttributeType,
};
use aws_sdk_dynamodb::Client;
use nxs_foundation::change::Cell;
use std::collections::HashMap;

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

/// The views table of `name`, as the deploying stack must provision it: keys, both sparse
/// indexes projecting every attribute, on-demand billing. Production code never sends it; the
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
        .key_schema(key(PK, KeyType::Hash))
        .key_schema(key(SK, KeyType::Range))
        .global_secondary_indexes(index(ACTIVE_INDEX, ACTIVE, SK))
        .global_secondary_indexes(index(DATED_INDEX, DATED, DATED_AT))
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
}
