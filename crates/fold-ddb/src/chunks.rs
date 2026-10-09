//! The live chunks of an item's chunk field, read on a server (6j6v.c0kn) — what
//! `Store::chunks_of` answers on a replica: every chunk of the field that no supersede of the same
//! field names, in the canonical op order `(lamport, site, id)`.
//!
//! Two Queries, each on the item's own range and strongly consistent on its own (not as a pair: a
//! fold between them may show a chunk whose supersede landed just after the first read, which the
//! next read then hides): the field's replaced set, then the field's chunks. `chunks` and `chunk_superseded` key on `(item_id, field, …)`, so each is
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

fn int(row: &Row, column: &str) -> Option<i64> {
    match row.get(column) {
        Some(Cell::Int(v)) => Some(*v),
        _ => None,
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
        // Every row the fold writes carries its coordinate; one without it would sort nowhere
        // true, so it is left out rather than ordered as if it were the oldest.
        .filter_map(|row| {
            let id = text(row, "id")?;
            let (lamport, site) = (int(row, "lamport")?, int(row, "site")?);
            (!replaced.contains(id)).then(|| Chunk {
                id: id.to_string(),
                body: text(row, "body").unwrap_or_default().to_string(),
                author: text(row, "author").unwrap_or_default().to_string(),
                lamport,
                site,
                created_at: text(row, "created_at")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            })
        })
        .collect();
    live.sort_by(|a, b| (a.lamport, a.site, &a.id).cmp(&(b.lamport, b.site, &b.id)));
    Ok(live)
}
