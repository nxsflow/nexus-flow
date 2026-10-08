//! The live chunks of an item's chunk field, read on a server (6j6v.c0kn) — what
//! `Store::chunks_of` answers on a replica: every chunk of the field that no supersede of the same
//! field names, in the canonical op order `(lamport, site, id)`.
//!
//! Two Queries, both on the item's own range and strongly consistent: the field's chunks and the
//! field's replaced set. `chunks` and `chunk_superseded` key on `(item_id, field, …)`, so each is
//! one `begins_with` on the sort key and reads nothing of any other item or field. The engine
//! never reads a chunk's body.

use std::collections::BTreeSet;

use nexus_flow_core::model::Chunk;
use nxs_foundation::change::Cell;

use crate::layout::row_key;
use crate::table::{text, Row, Table};

/// The sort-key prefix of `table`'s rows for one item's chunk field: the row key of the leading
/// two key cells, closed by the separator, so `a#b#` never matches field `bc`.
fn field_prefix(table: &str, item: &str, field: &str) -> String {
    let key = [
        ("item_id".to_string(), Cell::Text(item.to_string())),
        ("field".to_string(), Cell::Text(field.to_string())),
    ];
    // Both cells are text, so the key cannot be NULL and row_key cannot fail.
    let mut prefix = row_key(table, &key).expect("text key cells always form a row key");
    prefix.push('#');
    prefix
}

fn int(row: &Row, column: &str) -> i64 {
    match row.get(column) {
        Some(Cell::Int(v)) => *v,
        _ => 0,
    }
}

/// The live chunks of `item`'s chunk field `field`, in canonical op order.
pub async fn chunks<T: Table>(table: &T, item: &str, field: &str) -> Result<Vec<Chunk>, T::Error> {
    let replaced: BTreeSet<String> = table
        .query_prefix(&field_prefix("chunk_superseded", item, field))
        .await?
        .iter()
        .filter_map(|row| text(row, "chunk_id").map(str::to_string))
        .collect();
    let mut live: Vec<Chunk> = table
        .query_prefix(&field_prefix("chunks", item, field))
        .await?
        .iter()
        .filter_map(|row| {
            let id = text(row, "id")?;
            (!replaced.contains(id)).then(|| Chunk {
                id: id.to_string(),
                body: text(row, "body").unwrap_or_default().to_string(),
                author: text(row, "author").unwrap_or_default().to_string(),
                lamport: int(row, "lamport"),
                site: int(row, "site"),
                created_at: text(row, "created_at")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            })
        })
        .collect();
    live.sort_by(|a, b| (a.lamport, a.site, &a.id).cmp(&(b.lamport, b.site, &b.id)));
    Ok(live)
}
