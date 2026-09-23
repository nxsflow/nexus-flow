//! Output rendering. The canonical JSON record IS the agent contract: every core
//! field, deterministic order, presentation-independent. Human presentation (which
//! columns a plugin shows) lives with the commands; this module never consults a plugin.
//!
//! Records pass through `serde_json::Value`, whose object keys serialize in stable
//! lexicographic order — that ordering is the determinism guarantee (an agent parses
//! by key, so the specific order is irrelevant as long as it never drifts). All
//! `Option` fields are always present (as `null` when absent), so the record shape is
//! fixed regardless of which fields are populated. `item_type` is renamed to `type`.

use nexus_flow_core::model::ItemRow;
use serde::Serialize;

/// The canonical item record: every core field, fixed declaration order.
#[derive(Debug, Serialize)]
pub struct CanonicalItem {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: Option<String>,
    pub title: Option<String>,
    pub completion_criterion: Option<String>,
    pub description: Option<String>,
    pub design: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    pub due: Option<String>,
    pub defer_until: Option<String>,
    pub assignee: Option<String>,
    pub belongs_to: Option<String>,
    pub closing_comment: Option<String>,
    pub deleted: Option<String>,
    pub archived: Option<String>,
    pub closed_at: Option<String>,
}

impl From<&ItemRow> for CanonicalItem {
    fn from(r: &ItemRow) -> Self {
        CanonicalItem {
            id: r.id.clone(),
            item_type: r.item_type.clone(),
            title: r.title.clone(),
            completion_criterion: r.completion_criterion.clone(),
            description: r.description.clone(),
            design: r.design.clone(),
            status: r.status.clone(),
            priority: r.priority.clone(),
            due: r.due.clone(),
            defer_until: r.defer_until.clone(),
            assignee: r.assignee.clone(),
            belongs_to: r.belongs_to.clone(),
            closing_comment: r.closing_comment.clone(),
            deleted: r.deleted.clone(),
            archived: r.archived.clone(),
            closed_at: r.closed_at.clone(),
        }
    }
}

/// Serialize one item as the canonical record.
///
/// The lexicographic key order that every `--json` consumer (and every golden doc example)
/// relies on comes from `serde_json::Value`'s default `BTreeMap` backing — round-tripping the
/// struct through a `Value` re-sorts the keys. Enabling the `serde_json/preserve_order` feature
/// anywhere in nxf's build graph would swap that for an `IndexMap` (declaration order) and
/// silently break the contract. It is deliberately NOT enabled (see xtask/Cargo.toml), and
/// `canonical_record_serializes_keys_in_lexicographic_order` pins it.
pub fn item_value(item: &ItemRow) -> serde_json::Value {
    serde_json::to_value(CanonicalItem::from(item)).expect("serialize canonical item")
}

/// Serialize a set of items as canonical records, deterministically ordered by id.
pub fn items_value(items: &[ItemRow]) -> serde_json::Value {
    let mut sorted: Vec<&ItemRow> = items.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    serde_json::Value::Array(sorted.into_iter().map(item_value).collect())
}

/// Raw (unmapped) value of a presentable field — pure canonical-field access over an
/// [`ItemRow`], independent of any plugin vocabulary. Shared by the read compute layer (the
/// `next` ranking sorts on it) and the CLI's human renderer (which then maps it through the
/// plugin's labels). The plugin display mapping itself stays consumer-side, never here.
pub fn raw_field(item: &ItemRow, field: &str) -> Option<String> {
    match field {
        "id" => Some(item.id.clone()),
        "type" => item.item_type.clone(),
        "title" => item.title.clone(),
        "completion_criterion" => item.completion_criterion.clone(),
        "status" => item.status.clone(),
        "priority" => item.priority.clone(),
        "due" => item.due.clone(),
        "defer_until" | "defer" => item.defer_until.clone(),
        "assignee" => item.assignee.clone(),
        "belongs_to" => item.belongs_to.clone(),
        "closing_comment" => item.closing_comment.clone(),
        // C3 (#916.4): the lane recency keys — `archived` (archived lane, desc) and `closed_at`
        // (closed lane, desc) — exposed here so the shared orderer can sort on them like any field.
        "archived" => item.archived.clone(),
        "closed_at" => item.closed_at.clone(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> ItemRow {
        ItemRow {
            id: id.to_string(),
            item_type: Some("task".into()),
            title: Some("t".into()),
            completion_criterion: None,
            description: None,
            design: None,
            status: Some("open".into()),
            priority: Some("2".into()),
            due: None,
            defer_until: None,
            assignee: None,
            belongs_to: None,
            closing_comment: None,
            deleted: None,
            archived: None,
            closed_at: None,
        }
    }

    #[test]
    fn canonical_record_has_exactly_the_core_fields() {
        // Test the path the commands actually use: through `item_value` (a Value).
        let v = item_value(&row("c1.A"));
        let obj = v.as_object().expect("record is a json object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut expected = [
            "id",
            "type", // renamed from item_type
            "title",
            "completion_criterion",
            "description",
            "design",
            "status",
            "priority",
            "due",
            "defer_until",
            "assignee",
            "belongs_to",
            "closing_comment",
            "deleted",
            "archived",
            "closed_at",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected, "record must carry exactly the core fields");
        assert_eq!(obj["title"], serde_json::json!("t"));
        // Absent fields are present as null, fixing the record shape.
        assert_eq!(obj["due"], serde_json::Value::Null);
    }

    #[test]
    fn canonical_record_serializes_keys_in_lexicographic_order() {
        // The `--json` contract and every golden doc example depend on lexicographic key order,
        // which is an emergent property of `serde_json::Value`'s default BTreeMap backing.
        // `canonical_record_has_exactly_the_core_fields` sorts keys before comparing, so it
        // CANNOT catch an insertion-order regression (e.g. someone enabling `preserve_order`).
        // This pins the contract on the SERIALIZED bytes — the surface that hits the wire — so a
        // flip fails here, at the cause, not only deep in the trycmd golden corpus.
        let mut item = row("ab12.0001");
        // Populate every Option so all keys are present in the serialized object.
        item.due = Some("2026-12-31".into());
        item.defer_until = Some("2026-07-01".into());
        item.assignee = Some("dev".into());
        item.belongs_to = Some("ab12.0002".into());
        item.closing_comment = Some("done".into());
        item.completion_criterion = Some("c".into());
        item.description = Some("why + goal".into());
        item.design = Some("path to goal".into());
        item.deleted = Some("0".into());
        item.archived = Some("2026-06-23T10:00:00Z".into());
        item.closed_at = Some("2026-06-22T09:00:00Z".into());

        let json = serde_json::to_string(&item_value(&item)).unwrap();
        let keys = [
            "archived",
            "assignee",
            "belongs_to",
            "closed_at",
            "closing_comment",
            "completion_criterion",
            "defer_until",
            "deleted",
            "description",
            "design",
            "due",
            "id",
            "priority",
            "status",
            "title",
            "type",
        ];
        // `keys` is already in lexicographic order; assert each `"key":` appears after the prior
        // one in the serialized string (strictly increasing byte offsets ⇒ keys emitted sorted).
        let offsets: Vec<usize> = keys
            .iter()
            .map(|k| {
                json.find(&format!("\"{k}\":"))
                    .unwrap_or_else(|| panic!("key `{k}` missing in {json}"))
            })
            .collect();
        let mut ascending = offsets.clone();
        ascending.sort_unstable();
        assert_eq!(
            offsets, ascending,
            "canonical record keys must serialize in lexicographic order: {json}"
        );
    }

    #[test]
    fn record_is_deterministic() {
        // The contract is byte-stability of the serialized form across runs.
        let a = serde_json::to_string(&item_value(&row("c1.A"))).unwrap();
        let b = serde_json::to_string(&item_value(&row("c1.A"))).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn items_value_is_sorted_by_id() {
        let v = items_value(&[row("c1.C"), row("c1.A"), row("c1.B")]);
        let ids: Vec<&str> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["c1.A", "c1.B", "c1.C"]);
    }
}
