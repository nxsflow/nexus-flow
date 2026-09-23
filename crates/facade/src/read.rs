//! The read compute layer — the `compute` half of the compute→render split (E5 #9t7.1 /
//! E9 #49g). Each read op (`prime`/`blocked`/`next`/`show`/`list`) computes a *typed
//! record* over the meaning-free core (`derive::*` + `Store`), with a canonical
//! [`to_value`](PrimeReport::to_value) projection that IS the `--json` shape. This projection
//! used to be, for every op including `prime`, a 1:1 structured view of the human layout
//! (nexus-flow-vux): the same sections, and `next` carries only the displayed fields in
//! canonical form — full records stay on `nxf next`. **Superseded for `prime` specifically,
//! 2026-08-28 (nxf xe2z, task 2):** its human (Markdown) rendering now shows only a CURATED
//! SUBSET of `to_value()`'s record, not a 1:1 mirror of it — the SessionStart hook `nxf prime`
//! backs has a hard host-side byte ceiling, so the human view keeps only what no `--help`/`nxf
//! guide` page can teach and drops the rest, while `to_value()` stays the full canonical record.
//! See [`PrimeReport`]'s own doc comment, and `crates/cli/src/commands/mod.rs::prime`'s `else`
//! branch, for the full accounting. Every other op's `to_value()` is unaffected and still
//! mirrors its human rendering 1:1. No `println!`, no clap, no human presentation lives here: a
//! consumer either serializes `to_value()` (the CLI `--json` path, the MCP read tools) or
//! renders the typed record through its own presentation (the CLI human view, an embedding
//! app's UI).
//!
//! `now` is an explicit parameter for the time-dependent ops (`next`/`prime`); the
//! caller decides whether that is the system clock or an override (the CLI's `--now`).

use crate::error::{NxfError, Result};
use crate::plugin::{Dir, Nulls, PluginConfig, RankKey};
use crate::record::{item_value, items_value, raw_field};
use nexus_flow_core::derive;
use nexus_flow_core::model::{ItemRow, LinkWeight, ThreadLink};
use nexus_flow_core::store::Store;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

// ---- schema (plugin-aware field + type-system introspection, 8qv.5 / y5j8) --------------------
//
// The active plugin's full field model AND type system as one record, shared by `nxf schema` and
// the `flow_schema` MCP tool so the two seams cannot drift (the read::prime pattern). Pure over the
// PluginConfig — no store. The `hierarchy` block (y5j8) is derived from RelationshipMatrix.

/// The static shape of one field in the create/update contract. The plugin-specific *label* and the
/// enum *values* are layered on at runtime in [`schema`]; `settable` is derived from
/// [`crate::write::UPDATABLE_FIELDS`] so the schema can never claim a field is settable when
/// `update` rejects it.
struct FieldSpec {
    field: &'static str,
    /// `text | longtext | date | priority | status | type | ref` — how the value is interpreted.
    kind: &'static str,
    /// Required when CREATING the item.
    required: bool,
    /// The `create` flag that sets it, if any.
    create_flag: Option<&'static str>,
    /// The `update --set` alias, when the agent-facing word differs from the column.
    update_alias: Option<&'static str>,
}

/// The create/update field contract `schema` reports. Order is create-call order: mandatory fields
/// first, then optional, then `--set`-only / special fields. Moved from the CLI (was 8qv.5) so the
/// facade owns the one field model both seams render.
const FIELD_SCHEMA: &[FieldSpec] = &[
    FieldSpec {
        field: "type",
        kind: "type",
        required: true,
        create_flag: Some("--type"),
        update_alias: None,
    },
    FieldSpec {
        field: "title",
        kind: "text",
        required: true,
        create_flag: Some("--title"),
        update_alias: None,
    },
    FieldSpec {
        field: "description",
        kind: "text",
        required: true,
        create_flag: Some("--description"),
        update_alias: None,
    },
    FieldSpec {
        field: "priority",
        kind: "priority",
        required: true,
        create_flag: Some("--priority"),
        update_alias: None,
    },
    FieldSpec {
        field: "design",
        kind: "text",
        required: false,
        create_flag: Some("--design"),
        update_alias: None,
    },
    FieldSpec {
        field: "completion_criterion",
        kind: "text",
        required: false,
        create_flag: Some("--dod"),
        update_alias: None,
    },
    FieldSpec {
        field: "due",
        kind: "date",
        required: false,
        create_flag: Some("--due"),
        update_alias: None,
    },
    FieldSpec {
        field: "defer_until",
        kind: "date",
        required: false,
        create_flag: Some("--defer"),
        update_alias: Some("defer"),
    },
    FieldSpec {
        field: "belongs_to",
        kind: "ref",
        required: false,
        create_flag: Some("--parent"),
        update_alias: Some("parent"),
    },
    FieldSpec {
        field: "status",
        kind: "status",
        required: false,
        create_flag: None,
        update_alias: None,
    },
    FieldSpec {
        field: "assignee",
        kind: "text",
        required: false,
        create_flag: None,
        update_alias: None,
    },
    FieldSpec {
        field: "closing_comment",
        kind: "text",
        required: false,
        create_flag: None,
        update_alias: None,
    },
];

/// Canonical field names whose `create --json` payload key DIVERGES from the field-column spelling
/// (3bw6): `(canonical, create-payload key)`. Keep in lockstep with the CLI's `CreateJson` — the
/// `field_schema_create_json_keys_match_the_payload` CLI test guards it against the report.
const CREATE_JSON_KEY_ALIASES: &[(&str, &str)] = &[
    ("completion_criterion", "dod"),
    ("defer_until", "defer"),
    ("belongs_to", "parent"),
];

/// The `create --json` payload key for a field: `None` for a non-createable field, else the payload
/// key — the field name unless it diverges (`CREATE_JSON_KEY_ALIASES`), e.g. `completion_criterion` → `dod`.
fn create_json_key(spec: &FieldSpec) -> Option<&'static str> {
    spec.create_flag?;
    Some(
        CREATE_JSON_KEY_ALIASES
            .iter()
            .find(|(canonical, _)| *canonical == spec.field)
            .map(|(_, payload_key)| *payload_key)
            .unwrap_or(spec.field),
    )
}

/// Core-default field labels — the bare-engine names, used when the plugin's `vocabulary.fields`
/// does not rename a field. `pub` because the CLI `show` path (8qv.8) renders headings from it too,
/// so the facade owns the ONE field-label vocabulary (no CLI duplicate).
pub fn default_field_label(key: &str) -> &str {
    match key {
        "description" => "Description",
        "completion_criterion" => "Definition of Done",
        "design" => "Design",
        "notes" => "Notes",
        "title" => "Title",
        "priority" => "Priority",
        "status" => "Status",
        "type" => "Type",
        "belongs_to" | "parent" => "Parent",
        "due" => "Due",
        "defer_until" => "Defer until",
        "assignee" => "Assignee",
        "closing_comment" => "Closing comment",
        other => other,
    }
}

/// The plugin's display label for a field (8qv.8), or the core default when unset. `pub` so the CLI
/// `show` path renders headings from this single source (the CLI copy is deleted in Task 3).
pub fn field_label(cfg: &PluginConfig, key: &str) -> String {
    cfg.vocabulary
        .fields
        .get(key)
        .cloned()
        .unwrap_or_else(|| default_field_label(key).to_string())
}

/// The display label of a plugin CUSTOM field (plugin-custom-fields §6): the declared `label`, or
/// the titleised field name when unset (`"file_uri"` → `"File Uri"`). `pub` so `schema` and the CLI
/// `show` custom-field rendering read the ONE definition and never drift.
pub fn custom_field_label(decl: &crate::plugin::FieldDecl, name: &str) -> String {
    decl.label.clone().unwrap_or_else(|| titleise(name))
}

/// Titleise a snake_case field name for a default label: split on `_`, capitalise each word's first
/// letter, join with a space. `"uri"` → `"Uri"`, `"file_uri"` → `"File Uri"`.
fn titleise(name: &str) -> String {
    name.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One field entry in the schema report.
pub struct SchemaField {
    pub field: String,
    pub label: String,
    pub kind: String,
    pub required: bool,
    pub settable: bool,
    pub create_flag: Option<String>,
    pub create_json_key: Option<String>,
    /// The accepted create values, spelled out ONLY for `priority` (the named labels, not the ordinal).
    pub create_json_values: Option<Vec<String>>,
    pub update_alias: Option<String>,
    /// `true` for a plugin-declared custom field (plugin-custom-fields §6), `false` for a canonical
    /// one. Gates the custom-only keys in [`to_value`](Self::to_value) so a canonical entry emits NO
    /// new keys and its `schema --json` object stays byte-identical.
    pub custom: bool,
    /// The item types a custom field applies to (its `[fields.<name>].on`); empty ⇒ global. Emitted
    /// only for a custom field.
    pub on: Vec<String>,
    /// The accepted values of a custom `enum` field. `Some` only for a custom enum; `None` otherwise.
    pub values: Option<Vec<String>>,
}

impl SchemaField {
    fn to_value(&self) -> Value {
        let mut o = json!({
            "field": self.field,
            "label": self.label,
            "kind": self.kind,
            "required": self.required,
            "settable": self.settable,
        });
        if let Some(f) = &self.create_flag {
            o["create_flag"] = json!(f);
        }
        if let Some(k) = &self.create_json_key {
            o["create_json_key"] = json!(k);
        }
        if let Some(v) = &self.create_json_values {
            o["create_json_values"] = json!(v);
        }
        if let Some(a) = &self.update_alias {
            o["update_alias"] = json!(a);
        }
        // Custom-field annotations (plugin-custom-fields §6) — emitted ONLY for a plugin custom field
        // so a canonical entry gains no keys (its object stays byte-identical, the existing `schema_*`
        // goldens/tests unchanged). `values` rides along only for a custom `enum`.
        if self.custom {
            o["custom"] = json!(true);
            o["on"] = json!(self.on);
            if let Some(v) = &self.values {
                o["values"] = json!(v);
            }
        }
        o
    }
}

/// One type's containment role (y5j8): whether it is a container, its allowed parent types, and its
/// parent cardinality. All derived from [`crate::matrix::RelationshipMatrix`].
pub struct TypeContainment {
    pub name: String,
    pub container: bool,
    pub parents: Vec<String>,
    pub cardinality: Option<crate::matrix::Cardinality>,
}

/// The plugin's type hierarchy: the per-type containment roles plus the global depth limit.
pub struct Hierarchy {
    pub max_depth: Option<usize>,
    pub types: Vec<TypeContainment>,
}

/// The active plugin's full field + type-system model. `to_value()` is the `nxf schema --json`
/// object AND the `flow_schema` structuredContent (one source, no drift).
pub struct SchemaReport {
    pub plugin: String,
    pub tagline: Option<String>,
    /// `(type, display-label)` sorted by type name — the plugin's `[types].list` is a `BTreeSet`,
    /// so this (like the `--json` `types` map it renders) is lexicographic, not declaration order.
    pub types: Vec<(String, String)>,
    /// The status vocabulary map (`stored → display`).
    pub statuses: BTreeMap<String, String>,
    /// Priority variant labels, index 0 highest.
    pub priorities: Vec<String>,
    pub fields: Vec<SchemaField>,
    pub hierarchy: Hierarchy,
}

/// Map a parent cardinality to its JSON form: `"single"` | `"many"` | `<n>`, or `null` for a type
/// with no parent rule (a root/container that may not itself be parented).
fn cardinality_value(c: Option<crate::matrix::Cardinality>) -> Value {
    use crate::matrix::Cardinality::*;
    match c {
        None => Value::Null,
        Some(Single) => json!("single"),
        Some(Many) => json!("many"),
        Some(Limited(n)) => json!(n),
    }
}

impl SchemaReport {
    /// The `nxf schema --json` / `flow_schema` object. Legacy keys byte-identical to 8qv.5; the
    /// `hierarchy` block is additive (y5j8).
    pub fn to_value(&self) -> Value {
        let types: serde_json::Map<String, Value> = self
            .types
            .iter()
            .map(|(t, label)| (t.clone(), Value::String(label.clone())))
            .collect();
        let priorities: serde_json::Map<String, Value> = self
            .priorities
            .iter()
            .enumerate()
            .map(|(i, label)| (i.to_string(), Value::String(label.clone())))
            .collect();
        let hierarchy_types: serde_json::Map<String, Value> = self
            .hierarchy
            .types
            .iter()
            .map(|t| {
                (
                    t.name.clone(),
                    json!({
                        "container": t.container,
                        "parents": t.parents,
                        "cardinality": cardinality_value(t.cardinality),
                    }),
                )
            })
            .collect();
        json!({
            "plugin": self.plugin,
            "tagline": self.tagline,
            "types": types,
            "statuses": self.statuses,
            "priorities": priorities,
            "fields": self.fields.iter().map(SchemaField::to_value).collect::<Vec<_>>(),
            "dependencies": {
                "create_flag": "--depends-on",
                "create_json_key": "depends_on",
                "repeatable": true,
                "command": "dep add <this> <id>",
            },
            "hierarchy": {
                "max_depth": self.hierarchy.max_depth,
                "types": hierarchy_types,
            },
        })
    }
}

/// Compute the schema record for the active plugin. Pure over `cfg` — no store. Shared by
/// `nxf schema` and the `flow_schema` MCP tool.
pub fn schema(cfg: &PluginConfig) -> SchemaReport {
    let types: Vec<(String, String)> = cfg
        .types
        .list
        .iter()
        .map(|t| {
            let label = cfg
                .vocabulary
                .types
                .get(t)
                .cloned()
                .unwrap_or_else(|| t.clone());
            (t.clone(), label)
        })
        .collect();
    let mut fields: Vec<SchemaField> = FIELD_SCHEMA
        .iter()
        .map(|spec| SchemaField {
            field: spec.field.to_string(),
            label: field_label(cfg, spec.field),
            kind: spec.kind.to_string(),
            required: spec.required,
            settable: crate::write::UPDATABLE_FIELDS.contains(&spec.field),
            create_flag: spec.create_flag.map(str::to_string),
            create_json_key: create_json_key(spec).map(str::to_string),
            create_json_values: (spec.field == "priority").then(|| cfg.priority.labels.clone()),
            update_alias: spec.update_alias.map(str::to_string),
            custom: false,
            on: Vec::new(),
            values: None,
        })
        .collect();
    // Append the plugin's declared custom fields (plugin-custom-fields §6) AFTER the canonical
    // entries. `cfg.fields` is a `BTreeMap`, so this is name-sorted and deterministic. Each carries
    // its declaration — `custom: true` (so `to_value` emits the annotation keys), the `on` scope, the
    // `required` guard, and (for an enum) its `values`; a custom field is always `--set`-settable and
    // has no create flag. A bundled plugin declares none, so this is empty and canonical `schema`
    // output is unchanged.
    fields.extend(cfg.fields.iter().map(|(name, decl)| {
        let is_enum = decl.field_type == crate::plugin::FieldType::Enum;
        SchemaField {
            field: name.clone(),
            label: custom_field_label(decl, name),
            kind: decl.field_type.as_str().to_string(),
            required: decl.required,
            settable: true,
            create_flag: None,
            create_json_key: None,
            create_json_values: None,
            update_alias: None,
            custom: true,
            on: decl.on.clone(),
            values: is_enum.then(|| decl.values.clone()),
        }
    }));
    let hierarchy_types: Vec<TypeContainment> = cfg
        .types
        .list
        .iter()
        .map(|t| TypeContainment {
            name: t.clone(),
            container: cfg.types.is_container(t),
            parents: cfg.types.allowed_parents(t),
            cardinality: cfg.types.cardinality(t),
        })
        .collect();
    SchemaReport {
        plugin: cfg.name.clone(),
        tagline: cfg.tagline.clone(),
        types,
        statuses: cfg.vocabulary.status.clone(),
        priorities: cfg.priority.labels.clone(),
        fields,
        hierarchy: Hierarchy {
            max_depth: cfg.types.max_depth,
            types: hierarchy_types,
        },
    }
}

// ---- shared dependency helpers ---------------------------------------------

/// Fetch an item addressable by explicit id or fail with `not_found`. Filters only the `deleted`
/// tombstone — an `archived` item is still returned (C1 #916.1: explicit lookup ≠ enumeration, and
/// archived is reversible via `unarchive`), so `show` can render it with its marker.
fn require_item(store: &Store, id: &str) -> Result<ItemRow> {
    store
        .get_item(id)?
        .filter(|i| i.deleted.as_deref() != Some("1"))
        .ok_or_else(|| NxfError::not_found(format!("no item '{id}'")))
}

/// Resolve `ids` to their live item rows, skipping any that are missing/deleted (mirroring the old
/// `filter_map(get_item)`), but surfacing a db error as `io` (#76u.14) instead of swallowing it to a
/// dropped row. Shared by the derived-set reads (`next`/`deferred`) whose ids come from `derive::*`.
fn resolve_items(store: &Store, ids: &[String]) -> Result<Vec<ItemRow>> {
    // Bulk-resolve in ONE query (xn8s): a per-id `get_item` re-materializes the `present_parent`
    // view each call, so the ready lane was O(n·edges) — the dominant `read::next` cost (~0.7s on a
    // 156-item board). `get_items` materializes it once; we then re-index by id to preserve the
    // caller's `ids` order and skip any id with no row. Identical to the old N× `get_item` loop for
    // the DISTINCT id sets the derivations feed it (`ready`/`in_progress` both emit unique ids); a
    // duplicate id — which they never produce — would now resolve once (`remove` consumes it) rather
    // than twice, an intentional, unreachable difference, not a behavior change for any real caller.
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut by_id: BTreeMap<String, ItemRow> = store
        .get_items(&id_refs)?
        .into_iter()
        .map(|i| (i.id.clone(), i))
        .collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

/// Each dependency of `id` paired with the live status of its target — the "why is this
/// blocked" view reused by `show`, `blocked`, and `prime` (nexus-flow-97b/bcj). Deterministic:
/// deps are id-sorted. A dep whose target is deleted/missing reports `None` (it no longer
/// blocks, mirroring the derivation, which ignores deleted blockers).
fn deps_with_status(store: &Store, id: &str) -> Result<Vec<(String, Option<String>)>> {
    let mut deps = store.deps_of(id)?;
    deps.sort();
    let mut out = Vec::with_capacity(deps.len());
    for d in deps {
        let status = store
            .get_item(&d)?
            .filter(|i| i.deleted.as_deref() != Some("1"))
            .and_then(|i| i.status);
        out.push((d, status));
    }
    Ok(out)
}

/// The OPEN blockers of `id`: deps whose target is still live and not closed — the concrete
/// reason `id` is not ready, as `(id, core-status)` pairs. Id-sorted, deterministic. A closed
/// or deleted dep is satisfied and never listed (consistent with `derive::ready`).
fn open_blockers(store: &Store, id: &str) -> Result<Vec<(String, String)>> {
    Ok(deps_with_status(store, id)?
        .into_iter()
        .filter_map(|(d, st)| match st {
            Some(s) if s != "closed" => Some((d, s)),
            _ => None,
        })
        .collect())
}

// ---- list ------------------------------------------------------------------

/// Every live item, optionally filtered by core `status` / `item_type`. Deleted and archived
/// items are excluded (C1 #916.1: `list` is an enumerating read). Ordered by `sort` — `None` is the
/// per-verb default ([`DEFAULT_SORT_LIST`], id, so `list --json` stays plugin-independent); the
/// order is preserved into the `--json` projection ([`items_in_order_value`]), so `--sort rank`
/// surfaces in the output too (C2 #916.2).
pub fn list(
    cfg: &PluginConfig,
    store: &Store,
    status: Option<&str>,
    item_type: Option<&str>,
    sort: Option<SortKey>,
) -> Result<Vec<ItemRow>> {
    let mut items: Vec<ItemRow> = store
        .list_items()?
        .into_iter()
        .filter(|i| i.deleted.as_deref() != Some("1"))
        .filter(|i| i.archived.is_none())
        .filter(|i| status.is_none_or(|s| i.status.as_deref() == Some(s)))
        .filter(|i| item_type.is_none_or(|t| i.item_type.as_deref() == Some(t)))
        .collect();
    order_by(cfg, &mut items, sort.unwrap_or(DEFAULT_SORT_LIST));
    Ok(items)
}

/// The canonical `--json` array for a derived/queried item set: records ordered by id.
pub fn items_to_value(items: &[ItemRow]) -> Value {
    items_value(items)
}

// ---- blocked ---------------------------------------------------------------

/// One blocked item: its canonical record plus the open blockers (id + core status) holding it
/// up — so a consumer sees *which* blocker without a follow-up call (nexus-flow-97b).
#[derive(Debug, Clone)]
pub struct BlockedItem {
    pub item: ItemRow,
    /// `(blocker_id, core_status)`, id-sorted; only live, non-closed deps.
    pub blockers: Vec<(String, String)>,
}

/// The blocked set: every blocked item with its open blockers, ordered by `sort` — `None` is the
/// per-verb default ([`DEFAULT_SORT_BLOCKED`], rank). C2 (#916.2) fixes the previous id default:
/// blocked work now ranks like `next`, so the highest-priority blocked item surfaces first.
pub fn blocked(
    cfg: &PluginConfig,
    store: &Store,
    sort: Option<SortKey>,
) -> Result<Vec<BlockedItem>> {
    let mut rows: Vec<BlockedItem> = Vec::new();
    for id in derive::blocked(store.connection())? {
        let Some(item) = store.get_item(&id)? else {
            continue;
        };
        let blockers = open_blockers(store, &item.id)?;
        rows.push(BlockedItem { item, blockers });
    }
    order_blocked(cfg, &mut rows, sort.unwrap_or(DEFAULT_SORT_BLOCKED));
    Ok(rows)
}

/// The `--json` array for `blocked`: each canonical record with its `blockers` list appended
/// (the top-level `id` is preserved for existing consumers — nexus-flow-97b).
pub fn blocked_to_value(rows: &[BlockedItem]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                let mut rec = item_value(&row.item);
                rec["blockers"] = Value::Array(
                    row.blockers
                        .iter()
                        .map(|(b, st)| json!({ "id": b, "status": st }))
                        .collect(),
                );
                rec
            })
            .collect(),
    )
}

// ---- lane verbs (C3 #916.4) ------------------------------------------------
//
// The lanes partition the live item space as SETS: `archived ⊎ closed ⊎ blocked ⊎ deferred ⊎
// ready ⊎ in_progress = all live`. This module adds the three missing verbs — `deferred` (a graph
// derivation: it needs the blocker/cycle filter + `now`) and `closed`/`archived` (pure selections
// over the materialized view) — as facade methods, so the CLI and an in-process consumer share ONE
// lane-membership definition (seam invariant #9t7), not a client reconstruction.
//
// TWO lanes have no dedicated verb: `ready` and `in_progress`. The partition's `in_progress` set is
// the RAW status set (every `status='in_progress'` live item, blocked/deferred ones included), read
// via `list --status in_progress`. The in_progress work `next` folds in is NOT that lane — it is
// only the ACTIONABLE in_progress subset (unblocked, acyclic, not deferred). So a claimed-then-
// blocked item is reachable via `list`, not via `next`/`blocked`/`deferred` (those last two are
// `status='open'` only).
//
// `next` is NOT the ready lane either — not since the finish-first tiers made it recommend `ready ∪
// in_progress` (docs/specs/next-finish-first-tiers.md §2). It is a recommendation VIEW spanning two
// lanes, not a lane reader. The two sets it unions are disjoint by status, so a consumer that wants
// the ready LANE takes `next`'s `status='open'` rows (what `lanes.rs::ready_lane` does); one that
// wants a work list just uses `next` as-is.

/// The `deferred` lane: open, unblocked, acyclic work whose `defer_until` is still in the future,
/// ordered by `sort` (default [`DEFAULT_SORT_DEFERRED`], defer-date ascending — soonest first).
/// `now` is the defer boundary. Disjoint from `ready`/`blocked` by construction (`derive::deferred`).
pub fn deferred(
    cfg: &PluginConfig,
    store: &Store,
    now: &str,
    sort: Option<SortKey>,
) -> Result<Vec<ItemRow>> {
    let ids = derive::deferred(store.connection(), now)?;
    let mut items = resolve_items(store, &ids)?;
    order_by(cfg, &mut items, sort.unwrap_or(DEFAULT_SORT_DEFERRED));
    Ok(items)
}

/// The `closed` lane: closed items that are NOT archived (an archived item belongs to the
/// `archived` lane — archived takes precedence), deleted excluded. Ordered by `sort` (default
/// [`DEFAULT_SORT_CLOSED`], close-date descending — most recent first; a legacy item with no
/// `closed_at` sorts last, deterministically).
pub fn closed(cfg: &PluginConfig, store: &Store, sort: Option<SortKey>) -> Result<Vec<ItemRow>> {
    let mut items: Vec<ItemRow> = store
        .list_items()?
        .into_iter()
        .filter(|i| i.deleted.as_deref() != Some("1"))
        .filter(|i| i.archived.is_none())
        .filter(|i| i.status.as_deref() == Some("closed"))
        .collect();
    order_by(cfg, &mut items, sort.unwrap_or(DEFAULT_SORT_CLOSED));
    Ok(items)
}

/// The `archived` lane: every archived item, ANY status (archived is orthogonal to the lifecycle),
/// deleted excluded. Ordered by `sort` (default [`DEFAULT_SORT_ARCHIVED`], archive-date descending).
pub fn archived(cfg: &PluginConfig, store: &Store, sort: Option<SortKey>) -> Result<Vec<ItemRow>> {
    let mut items: Vec<ItemRow> = store
        .list_items()?
        .into_iter()
        .filter(|i| i.deleted.as_deref() != Some("1"))
        .filter(|i| i.archived.is_some())
        .collect();
    order_by(cfg, &mut items, sort.unwrap_or(DEFAULT_SORT_ARCHIVED));
    Ok(items)
}

/// The `recap` recency-recall surface (r4kb): the most-recently CLOSED items — "what got finished
/// lately" — as the reusable App-Layer query the embedding API, `prime`'s "Recently Closed" section,
/// and the `nxf recap` verb all share, so the three never drift.
///
/// DISTINCT from the [`closed`] LANE: recap INCLUDES archived-closed items (recency does not care
/// about the archived partition), where the lane EXCLUDES them (archived precedence). Both exclude
/// deleted and both order `closed_at` descending ([`DEFAULT_SORT_CLOSED`]).
///
/// - Included: `status == "closed"`, archived or not.
/// - Excluded: deleted; anything `status != "closed"` (a reopened item drops out).
/// - `limit`: `None` = every match, else the first N after ordering (the most recent).
/// - `since`: `None` = no floor, else keep only `closed_at >= since` — a plain string compare, the
///   same total order [`date_cmp`] uses (for ISO-8601, lexicographic == chronological, and a
///   `YYYY-MM-DD` floor compares correctly against a full RFC3339 instant). A match with no
///   `closed_at` cannot satisfy a floor, so it drops. Validated up front (loud rejection): a
///   malformed `since` is an `Err`, never a silent empty result.
///
/// The order is ALWAYS `closed_at` descending — deliberately no `sort` parameter (unlike the
/// general lane verbs [`closed`]/[`archived`]): "recap" IS the recency view, so a re-sort would
/// contradict its meaning. A consumer wanting another order reads the lane verbs instead.
///
/// Returns full, UNCAPPED [`ItemRow`]s (the whole `closing_comment`): presentation/truncation is the
/// CLI layer's job. This is the SemVer-contracted `crates/facade` embedding surface — apps call it
/// directly and decide their own rendering.
pub fn recap(
    cfg: &PluginConfig,
    store: &Store,
    limit: Option<usize>,
    since: Option<&str>,
) -> Result<Vec<ItemRow>> {
    // Validate the floor HERE, not only at the CLI seam: `recap` is a public facade fn embedders
    // call directly, and an unvalidated malformed `since` would silently filter every item out —
    // the opposite of the loud-rejection rule (validate.rs). The one shared ISO-date gate the
    // write paths use.
    if let Some(floor) = since {
        crate::validate::iso_date(floor)?;
    }
    let mut items: Vec<ItemRow> = store
        .list_items()?
        .into_iter()
        .filter(|i| i.deleted.as_deref() != Some("1"))
        .filter(|i| i.status.as_deref() == Some("closed"))
        .filter(|i| match since {
            Some(floor) => i.closed_at.as_deref().is_some_and(|c| c >= floor),
            None => true,
        })
        .collect();
    order_by(cfg, &mut items, DEFAULT_SORT_CLOSED);
    if let Some(n) = limit {
        items.truncate(n);
    }
    Ok(items)
}

/// The character cap a recap surface truncates a closing note to before appending `…`. The prime
/// "Recently Closed" section and `nxf recap` text share this one constant so their compact lines
/// can't diverge (r4kb: one query, one presentation, no drift).
pub const RECAP_NOTES_MAX: usize = 280;

/// How many recently-closed items `prime` shows (Top-3): the recency-recall glance, not the full
/// view — `nxf recap` / `nxf closed` are the full lists.
pub const PRIME_RECENTLY_CLOSED_LIMIT: usize = 3;

/// Collapse a note to a single compact line and cap it: every internal whitespace run (newlines
/// included) becomes one space, the ends are trimmed, then it is capped to `max` CHARACTERS —
/// appending `…` and reporting `true` when the source overran. Returns `(display, truncated)`. The
/// pure primitive the recap surfaces share (prime section text AND its JSON mirror, plus `nxf
/// recap`), so the compact note is identical everywhere. The `—` placeholder for a MISSING note is
/// [`recap_note`]'s job, not this one — an empty input yields `("", false)`.
pub fn truncate_notes(s: &str, max: usize) -> (String, bool) {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        (collapsed, false)
    } else {
        let kept: String = collapsed.chars().take(max).collect();
        (format!("{kept}…"), true)
    }
}

/// Resolve one item's closing note for a recap surface: [`truncate_notes`] over the comment, or the
/// `—` placeholder when the close left none (a `None`, empty, or whitespace-only comment). The
/// placeholder is never "truncated". Shared by the prime section build and `nxf recap` text so all
/// three surfaces render a note identically.
pub fn recap_note(comment: Option<&str>, max: usize) -> (String, bool) {
    match comment {
        Some(c) if !c.trim().is_empty() => truncate_notes(c, max),
        _ => ("—".to_string(), false),
    }
}

// ---- next ------------------------------------------------------------------

/// The `next` recommendation set (C4 #916.3): the actionable candidates — `ready ∪ in_progress`,
/// both unblocked, acyclic, not deferred — ordered by `sort`. Claimed work is **always** included
/// (the old `include_in_progress` flag is gone): hiding it is what let a finished-but-open epic
/// vanish from the default command. The two derived sets are disjoint (one `status='open'`, the
/// other `status='in_progress'`), so no item appears twice, and a board can still split
/// Ready/In-Progress on the raw `status` field.
///
/// Under the default `rank` order the result is **tiered** to push finishing over starting
/// ([`order_next_tiered`], `docs/specs/next-finish-first-tiers.md`); an explicit `--sort` bypasses
/// tiering for the flat order. Either way the result is in the chosen order — consumers must NOT
/// re-sort by id.
pub fn next(
    cfg: &PluginConfig,
    store: &Store,
    now: &str,
    sort: Option<SortKey>,
) -> Result<Vec<ItemRow>> {
    // ONE derivation for the whole lane: the candidate set AND its tier signals. Each `derive::*`
    // call re-walks the recursive cycle+suppression CTEs, so asking for ready/in_progress/the two
    // signals separately would run that walk four times over (the xn8s ratio guard below catches
    // precisely that). `next_candidates` IS `ready ∪ in_progress` by construction — same CTE, same
    // actionability — so the set is identical to the two-call form, at a quarter of the cost.
    let candidates = derive::next_candidates(store.connection(), now)?;
    let ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();
    let mut items = resolve_items(store, &ids)?;
    match sort.unwrap_or(DEFAULT_SORT_NEXT) {
        // The default: finishing beats starting.
        SortKey::Rank => order_next_tiered(cfg, &candidates, &mut items),
        // An explicit `--sort` is the escape hatch — the flat, untiered order (§4.2).
        key => order_by(cfg, &mut items, key),
    }
    Ok(items)
}

/// One page of a `next` result: the rows to show, plus how many there were before the cut.
///
/// The type exists so the total cannot be dropped on the way to a renderer. A limit that cuts
/// silently is worse than no limit at all — a list that shows 15 of 180 and looks like 15 of 15
/// tells the reader they are through (6j6v.8pf2).
///
/// `#[non_exhaustive]` (PR #363 review, Code Quality #1), for the same reason [`ShowRecord`] carries
/// it: the only constructor is [`truncate_next`], so nothing outside this crate builds a `NextPage`
/// via struct literal, and a later field — say the reason a row was cut — stays a genuinely
/// additive change instead of breaking every downstream that had spelled the literal out.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NextPage {
    /// The rows to render — already ordered, already filtered, already cut.
    pub items: Vec<ItemRow>,
    /// How many candidates there were BEFORE the cut. Equals `items.len()` when nothing was cut.
    pub total: usize,
}

impl NextPage {
    /// Whether the limit actually bit — the condition under which a view MUST disclose [`total`].
    ///
    /// [`total`]: NextPage::total
    pub fn truncated(&self) -> bool {
        self.items.len() < self.total
    }
}

/// Cut an ordered `next` result to `limit`, recording the untruncated total. `None` is "no limit"
/// — `nxf next`'s default; the caller that wants a cut is the one that names its size.
///
/// THE one truncation mechanism, for both callers that need one: `nxf next --limit <n>` and
/// [`prime`], which passes its own 15. It deliberately takes an already-ordered, already-filtered
/// set instead of running the query itself — the CLI applies `--label` between the ranking and the
/// cut, so the cut has to be the last step — and that is exactly what keeps it a single mechanism
/// rather than one per caller.
pub fn truncate_next(items: Vec<ItemRow>, limit: Option<usize>) -> NextPage {
    let total = items.len();
    let items = match limit {
        Some(n) => items.into_iter().take(n).collect(),
        None => items,
    };
    NextPage { items, total }
}

// ---- next: finish-first tiers (docs/specs/next-finish-first-tiers.md) ------
//
// `next` is the one verb that answers "what should I do NOW", so it is the one verb that tiers:
// `list --sort rank` and `blocked` keep the flat, field-based rank (the shared `order_by`/`sort_cmp`
// below are deliberately untouched). The tiers are composed HERE, in the facade, from two
// graph-derived core signals (`derive::finishable_in_progress`/`promoted_children`) and the
// UNCHANGED plugin rank: the core says which tier an item is in, the plugin says how items order
// within it. Nothing here reads plugin vocabulary — no "epic", no priority.

/// Tier 1 — closeable now: actionable claimed work with no live non-closed child (§2).
const TIER_FINISHABLE: u8 = 1;
/// Tier 2 — a started container and its actionable children, as one contiguous cluster (§2).
const TIER_STARTED: u8 = 2;
/// Tier 3 — the general ready backlog, exactly as ranked today (§2).
const TIER_BACKLOG: u8 = 3;
/// Within a Tier-2 cluster the header row sorts ahead of its children (§4.3).
const CLUSTER_HEADER: u8 = 0;
const CLUSTER_CHILD: u8 = 1;

/// Order the `ready ∪ in_progress` candidates into the finish-first tiers (§4.3), in place.
///
/// The sort key is `(tier, cluster, header-before-child)` with the plugin's [`rank_cmp`] as the
/// tiebreak — which is what ranks Tier 1 and Tier 3 internally, and orders a cluster's children.
/// Clusters themselves are ordered by their **header's** rank, so a cluster is contiguous and a
/// low-priority child never drags its epic up (nor a high-priority backlog item slice a cluster
/// apart). `rank_cmp` ends in an id tiebreak, so the whole composed order is total and
/// deterministic.
fn order_next_tiered(
    cfg: &PluginConfig,
    candidates: &[derive::NextCandidate],
    items: &mut [ItemRow],
) {
    // Split the one derivation into the three lookups the key needs. §3.3: the Tier-2 headers are
    // exactly `in_progress \ finishable` — the started items that still have a live non-closed
    // child; every promoted child's parent is one of them, so no cluster is ever headerless.
    let finishable: HashSet<&str> = candidates
        .iter()
        .filter(|c| c.status == "in_progress" && c.finishable)
        .map(|c| c.id.as_str())
        .collect();
    let started: HashSet<&str> = candidates
        .iter()
        .filter(|c| c.status == "in_progress")
        .map(|c| c.id.as_str())
        .collect();
    let promoted: HashMap<&str, &str> = candidates
        .iter()
        .filter_map(|c| c.promoter.as_deref().map(|p| (c.id.as_str(), p)))
        .collect();
    // The tiers must PARTITION the candidates: a promoted child is `ready` (open) by construction,
    // so it can never also be a header. A pure check — no logic, so `--release` behaves identically.
    debug_assert!(
        promoted.keys().all(|c| !started.contains(c)),
        "a promoted child is ready, never in_progress — the tiers would overlap"
    );

    let keys: HashMap<String, (u8, usize, u8)> = {
        let by_id: HashMap<&str, &ItemRow> = items.iter().map(|i| (i.id.as_str(), i)).collect();
        let mut headers: Vec<&str> = started
            .iter()
            .filter(|id| !finishable.contains(*id))
            .copied()
            .collect();
        headers.sort_by(|a, b| match (by_id.get(a), by_id.get(b)) {
            (Some(x), Some(y)) => rank_cmp(cfg, x, y),
            _ => a.cmp(b), // unreachable (a header is a candidate); stays deterministic anyway
        });
        let cluster_ord: HashMap<&str, usize> =
            headers.iter().enumerate().map(|(n, h)| (*h, n)).collect();

        items
            .iter()
            .map(|i| {
                let id = i.id.as_str();
                let key = if finishable.contains(id) {
                    (TIER_FINISHABLE, 0, CLUSTER_HEADER)
                } else if started.contains(id) {
                    // in_progress with a live non-closed child → its own cluster's header row.
                    let ord = cluster_ord.get(id).copied().unwrap_or(0);
                    (TIER_STARTED, ord, CLUSTER_HEADER)
                } else if let Some(parent) = promoted.get(id) {
                    // a ready child of a started container → a row inside that container's cluster.
                    let ord = cluster_ord.get(*parent).copied().unwrap_or(0);
                    (TIER_STARTED, ord, CLUSTER_CHILD)
                } else {
                    (TIER_BACKLOG, 0, CLUSTER_HEADER)
                };
                (i.id.clone(), key)
            })
            .collect()
    };

    items.sort_by(|a, b| {
        let ka = keys
            .get(&a.id)
            .copied()
            .unwrap_or((TIER_BACKLOG, 0, CLUSTER_HEADER));
        let kb = keys
            .get(&b.id)
            .copied()
            .unwrap_or((TIER_BACKLOG, 0, CLUSTER_HEADER));
        ka.cmp(&kb).then_with(|| rank_cmp(cfg, a, b))
    });
}

/// A resolved parent reference (#916.7): the live parent an item `belongs_to`, as id + title +
/// RAW core type (`project`/`task`, never a plugin label, so `--json` stays plugin-free). `next`
/// and `prime` attach it as a read-layer join — exactly the pattern `blocked` uses for its
/// `blockers` list — so the meaning-free core and the generic `item_value`/`CanonicalItem` record
/// stay untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentRef {
    pub id: String,
    pub title: Option<String>,
    pub item_type: Option<String>,
}

/// Resolve an item's `belongs_to` to its live parent, or `None` when there is no parent or it is
/// deleted/missing. Live-only: a deleted parent resolves to `None`; the raw `belongs_to` id stays
/// on the record regardless (this join is purely additive).
pub fn parent_of(store: &Store, item: &ItemRow) -> Result<Option<ParentRef>> {
    let Some(pid) = item.belongs_to.as_deref() else {
        return Ok(None);
    };
    let Some(p) = store
        .get_item(pid)?
        .filter(|i| i.deleted.as_deref() != Some("1"))
    else {
        return Ok(None);
    };
    Ok(Some(ParentRef {
        id: p.id,
        title: p.title,
        item_type: p.item_type,
    }))
}

/// The closed-parent reasons for the closed-mask (07a.3 §4/§5): when `item` is OPEN, has at least one
/// live parent, and EVERY live parent is closed, this is `(parent_id, closing_comment)` for all those
/// closed parents, id-sorted; empty otherwise (the item is not closed-masked). A read-layer join over
/// the full `parents_of` set — never a rewrite of the item's own `status` (the §4 review point: the
/// child keeps its own status; only the lane views treat it as effectively closed).
pub fn parent_closed_reason(
    store: &Store,
    item: &ItemRow,
) -> Result<Vec<(String, Option<String>)>> {
    if item.status.as_deref() != Some("open") {
        return Ok(Vec::new());
    }
    // Fallible parent read (07a.7): the effective-lane joins run on the long-lived MCP/embed server,
    // so a db error on the parent edge or a parent lookup maps to `io` (via `parents_of_result` +
    // fallible `get_item`) instead of `.expect()`-panicking and unwinding the handler.
    let mut live_parents: Vec<ItemRow> = Vec::new();
    for pid in store.parents_of_result(&item.id)? {
        if let Some(p) = store.get_item(&pid)? {
            if p.deleted.as_deref() != Some("1") {
                live_parents.push(p);
            }
        }
    }
    // Not closed-masked unless there is at least one live parent and none of them is non-closed.
    if live_parents.is_empty()
        || live_parents
            .iter()
            .any(|p| p.status.as_deref() != Some("closed"))
    {
        return Ok(Vec::new());
    }
    let mut out: Vec<(String, Option<String>)> = live_parents
        .into_iter()
        .map(|p| (p.id, p.closing_comment))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The `--json` value for a `parent_closed_reason` join: `[{parent_id, reason}]`, or `None` when the
/// item is not closed-masked (the field is then omitted — a sparse join, present only where it means
/// something).
fn parent_closed_reason_value(reasons: &[(String, Option<String>)]) -> Option<Value> {
    if reasons.is_empty() {
        return None;
    }
    Some(Value::Array(
        reasons
            .iter()
            .map(|(pid, reason)| json!({ "parent_id": pid, "reason": reason }))
            .collect(),
    ))
}

// ---- parent-pointer notice (6j6v.zvd0) -------------------------------------

/// Live (non-deleted, resolvable) parent ids of `item_id`, id-sorted (PR #255 review, Integrity
/// #2): mirrors the same live-only filter [`parent_closed_reason`] already applies to the same
/// `parents_of_result` edge set, so a tombstoned or missing parent is never surfaced as something
/// the reader is told to go read.
fn live_parents_of(store: &Store, item_id: &str) -> Result<Vec<String>> {
    let mut live = Vec::new();
    for pid in store.parents_of_result(item_id)? {
        if let Some(p) = store.get_item(&pid)? {
            if p.deleted.as_deref() != Some("1") {
                live.push(p.id);
            }
        }
    }
    Ok(live)
}

/// The loud, verbatim notice urging the reader to also read `parents` (6j6v.zvd0): `None` when
/// `parents` is empty, so a parentless item carries no notice at all. Lives here — not in the CLI
/// printer — so `nxf show` (human), `--json`, and every other consumer (MCP, embedders) render the
/// identical text off the identical engine computation.
fn parents_notice(parents: &[String]) -> Option<String> {
    if parents.is_empty() {
        return None;
    }
    Some(format!(
        "This item has the following parents: {}. URGENT RECOMMENDATION: ALSO READ THESE \
         ITEMS TO GET THE COMPLETE PICTURE!!!",
        parents.join(", ")
    ))
}

// ---- close(parent) sweep warning (07a.5) -----------------------------------

/// One child swept into the effective-`closed` lane by closing a parent (07a.5): the child's id and
/// title, plus its **other** parents (every live parent except the one just closed) with their
/// status, so the closer sees whether the child hangs anywhere still open. A swept child's own
/// `status` is never written — the sweep is pure derivation (the closed-mask, §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweptChild {
    pub id: String,
    pub title: Option<String>,
    /// `(parent_id, status)` for the child's other live parents, id-sorted.
    pub other_parents: Vec<(String, Option<String>)>,
}

/// The children that closing `parent_id` swept into the effective-`closed` lane (07a.5): the live
/// children of `parent_id` that are now closed-masked — still `open`, but with **no open parent
/// left** (every live parent closed, §4). Closing a parent can mask a multi-parent child without
/// writing to it, and the closer would not otherwise see it; this is the loud, derived list for the
/// close receipt. A child that still has another open parent is absent (not swept). Id-sorted.
pub fn swept_children(store: &Store, parent_id: &str) -> Result<Vec<SweptChild>> {
    // Fallible parent read (07a.7): `children_of_result` + `parents_of_result` + fallible `get_item`
    // so a db error on the close-receipt sweep (flow_close on the server) maps to `io`, not a panic.
    let mut out: Vec<SweptChild> = Vec::new();
    for cid in store.children_of_result(parent_id)? {
        let Some(c) = store.get_item(&cid)? else {
            continue;
        };
        if c.deleted.as_deref() == Some("1") {
            continue;
        }
        // Swept ⇔ the child is now closed-masked (open, ≥1 live parent, all live parents closed) —
        // the SAME predicate as `parent_closed_reason`, so the warning and the closed lane agree.
        if parent_closed_reason(store, &c)?.is_empty() {
            continue;
        }
        let mut other_parents: Vec<(String, Option<String>)> = Vec::new();
        for pid in store.parents_of_result(&c.id)? {
            if pid == parent_id {
                continue;
            }
            if let Some(p) = store.get_item(&pid)? {
                if p.deleted.as_deref() != Some("1") {
                    other_parents.push((p.id, p.status));
                }
            }
        }
        other_parents.sort_by(|a, b| a.0.cmp(&b.0));
        out.push(SweptChild {
            id: c.id,
            title: c.title,
            other_parents,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// The `--json` close receipt (07a.5): the closed item's canonical record plus a sparse
/// `swept_children` join — `[{id, title, other_parents:[{id, status}]}]` — appended only when the
/// close swept a child, exactly like `parent_closed_reason`, never a core field. Shared by the CLI
/// (`nxf close`) and the MCP (`flow_close`) so both receipts stay byte-identical.
pub fn close_receipt_value(store: &Store, item: &ItemRow) -> Result<Value> {
    let mut rec = item_value(item);
    let swept = swept_children(store, &item.id)?;
    if !swept.is_empty() {
        rec["swept_children"] = Value::Array(
            swept
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "title": c.title,
                        "other_parents": c.other_parents
                            .iter()
                            .map(|(pid, st)| json!({ "id": pid, "status": st }))
                            .collect::<Vec<_>>(),
                    })
                })
                .collect(),
        );
    }
    Ok(rec)
}

/// The canonical `parent` join value (#916.7): `{id,title,type}` for a live parent, else `null`.
fn parent_value(parent: Option<&ParentRef>) -> Value {
    match parent {
        Some(p) => json!({ "id": p.id, "title": p.title, "type": p.item_type }),
        None => Value::Null,
    }
}

/// The `--json` array for `next`: records in ranked order (NOT id-sorted), each with its resolved
/// `parent` join appended (#916.7) and a sparse `parent_closed_reason` join (07a.3 §5) for a
/// closed-masked child — both read-layer joins, never core fields. The store resolves the parents.
pub fn next_to_value(store: &Store, items: &[ItemRow]) -> Result<Value> {
    // One bulk label read for the whole lane instead of a `labels_of` per row (1w5v): `labels_by_id`
    // omits label-less items, so the sparse `labels` key appears iff an item has ≥1 label — the same
    // JSON as the old per-item `if !labels.is_empty()`, so `list`/`next --json` stays byte-identical
    // (the CLI↔handle parity gate) while the N read round-trips collapse to one.
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    let labels_by_id = store.labels_of_bulk(&ids)?;
    // nxf 6j6v.8dbe: one bulk read for the whole lane, same shape and same reason as the labels one
    // above. BEARING links only — a `passing` link must not make an item advertise "there are
    // conversations about this" in the work list; it is visible on the item itself (`show`).
    let conversations_by_id = store.bearing_thread_counts(&ids)?;
    let mut out = Vec::with_capacity(items.len());
    for i in items {
        let mut rec = item_value(i);
        rec["parent"] = parent_value(parent_of(store, i)?.as_ref());
        if let Some(labels) = labels_by_id.get(&i.id) {
            rec["labels"] = json!(labels);
        }
        if let Some(n) = conversations_by_id.get(&i.id) {
            rec["conversations"] = json!(n);
        }
        if let Some(reasons) = parent_closed_reason_value(&parent_closed_reason(store, i)?) {
            rec["parent_closed_reason"] = reasons;
        }
        out.push(rec);
    }
    let mut v = Value::Array(out);
    attach_timestamps_lane(store, &ids, &mut v)?; // 2kjy: sparse created_at/updated_at per record
    Ok(v)
}

/// The `--json` array for `list` (07a.3 §5): each record in the given order with a sparse
/// `parent_closed_reason` join appended for a closed-masked open child — the same read-layer join
/// `show`/`next` carry, never a core field. A non-masked record is byte-identical to
/// [`items_in_order_value`], so existing `list --json` output stays stable; only an open child of
/// all-closed parents gains the join (so an agent enumerating the board sees why it looks closed).
pub fn list_to_value(store: &Store, items: &[ItemRow]) -> Result<Value> {
    // One bulk label read for the whole lane (1w5v) — see [`next_to_value`]. Output is byte-identical
    // to the old per-item path, so the `list --json` parity + the sparse-key contract are preserved.
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    let labels_by_id = store.labels_of_bulk(&ids)?;
    let mut out = Vec::with_capacity(items.len());
    for i in items {
        let mut rec = item_value(i);
        if let Some(labels) = labels_by_id.get(&i.id) {
            rec["labels"] = json!(labels);
        }
        if let Some(reasons) = parent_closed_reason_value(&parent_closed_reason(store, i)?) {
            rec["parent_closed_reason"] = reasons;
        }
        out.push(rec);
    }
    let mut v = Value::Array(out);
    attach_timestamps_lane(store, &ids, &mut v)?; // 2kjy: sparse created_at/updated_at per record
    Ok(v)
}

// ---- plugin custom fields — the sparse `custom` read-layer join (6j6v.ekf5 §2.3/§6) -----------
//
// Custom values ride ONE additive, sparse `custom` key on the item record, exactly like the sparse
// `labels` key — narrowed to the ACTIVE plugin's declared fields (`cfg.fields`). A foreign /
// undeclared value (§7) is carried in the `custom_fields` view but never surfaced here. The
// `_with_custom` wrappers layer this over the label-bearing base projections, so the base fns
// (`next_to_value`/`list_to_value`/`show_value`) and the facade SemVer surface stay untouched
// (spec §8: additive, `facade changed` — not breaking).

/// The declared `custom` map for one item's set field→value pairs, or `None` when nothing declared
/// remains. Intersects the item's non-empty custom values (the core already excludes empties) with
/// `cfg.fields`, so an undeclared/foreign field is dropped. `None` ⇒ the caller omits the sparse
/// `custom` key entirely (exactly like a label-less item omits `labels`).
fn declared_custom_value(cfg: &PluginConfig, fields: &BTreeMap<String, String>) -> Option<Value> {
    let declared: serde_json::Map<String, Value> = fields
        .iter()
        .filter(|(name, _)| cfg.fields.contains_key(name.as_str()))
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect();
    (!declared.is_empty()).then_some(Value::Object(declared))
}

/// Attach the sparse, declared `custom` map to each record of a lane `--json` array, in place, via a
/// SINGLE bulk read (`custom_fields_of_bulk`) — no N+1, mirroring the `labels_of_bulk` lane join.
/// The array is the projection of `items` in the same order (both `next_to_value`/`list_to_value`
/// iterate `items` unshuffled), so record and item align by position.
fn attach_custom_lane(
    cfg: &PluginConfig,
    store: &Store,
    items: &[ItemRow],
    records: &mut Value,
) -> Result<()> {
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    attach_declared_custom(cfg, store, &ids, records)
}

/// Attach the sparse declared `custom` map to each record of `records` (an array), aligned by
/// POSITION to `ids`, via a SINGLE bulk read (`custom_fields_of_bulk`) — no N+1. The shared core of
/// every `_with_custom` lane builder (`list`/`next`/`blocked`/the bare lanes, 6j6v.bbq6): the caller
/// guarantees `records` is the projection of the same items, same order, so record and id align by
/// index. A record whose id has no declared custom value is left untouched (the sparse-key contract).
fn attach_declared_custom(
    cfg: &PluginConfig,
    store: &Store,
    ids: &[&str],
    records: &mut Value,
) -> Result<()> {
    let custom_by_id = store.custom_fields_of_bulk(ids)?;
    if let Some(rows) = records.as_array_mut() {
        // Index-alignment invariant: the caller passes `records` as the projection of the SAME
        // items in the SAME order as `ids`, so `zip` pairs each record with its own id. Assert it
        // in debug builds — a future length mismatch would otherwise silently cross-wire (or drop)
        // custom fields rather than fail.
        debug_assert_eq!(
            rows.len(),
            ids.len(),
            "attach_declared_custom: records/ids length mismatch ({} vs {})",
            rows.len(),
            ids.len(),
        );
        for (rec, id) in rows.iter_mut().zip(ids.iter()) {
            if let Some(fields) = custom_by_id.get(*id) {
                if let Some(custom) = declared_custom_value(cfg, fields) {
                    rec["custom"] = custom;
                }
            }
        }
    }
    Ok(())
}

// ---- op-log-derived read-surface timestamps (6j6v.2kjy) -----------------------
//
// created_at/updated_at ride the same ADDITIVE, SPARSE read-layer key mechanism as `labels`/`custom`
// (never a `CanonicalItem` field — "kanonische Keys unverändert"): derived from the op-log's
// `wall_clock` and attached per record iff stamped, so an un-stamped store is byte-identical to the
// pre-2kjy output. Unlike `custom` they are plugin-INDEPENDENT, so the attach runs unconditionally
// (never gated on `cfg.fields`). One bulk read per lane (`item_timestamps_of_bulk`) — no N+1.

/// Attach the sparse `created_at`/`updated_at` keys to each record of a lane `--json` array, in place,
/// aligned by POSITION to `ids` (the caller passes `records` as the projection of the same items in
/// the same order, exactly like [`attach_declared_custom`]). One bulk `item_timestamps_of_bulk` read.
fn attach_timestamps_lane(store: &Store, ids: &[&str], records: &mut Value) -> Result<()> {
    let ts = store.item_timestamps_of_bulk(ids)?;
    if let Some(rows) = records.as_array_mut() {
        // Index-alignment invariant (as in [`attach_declared_custom`]): the caller passes `records`
        // as the projection of the SAME items in the SAME order as `ids`, so `zip` pairs each record
        // with its own id. Assert it in debug builds — a future length mismatch would otherwise
        // silently cross-wire (or drop) timestamps rather than fail.
        debug_assert_eq!(
            rows.len(),
            ids.len(),
            "attach_timestamps_lane: records/ids length mismatch ({} vs {})",
            rows.len(),
            ids.len(),
        );
        for (rec, id) in rows.iter_mut().zip(ids.iter()) {
            if let Some((created, updated)) = ts.get(*id) {
                if let Some(c) = created {
                    rec["created_at"] = json!(c);
                }
                if let Some(u) = updated {
                    rec["updated_at"] = json!(u);
                }
            }
        }
    }
    Ok(())
}

/// Attach the sparse `created_at`/`updated_at` keys to ONE record (the single-item sibling, for
/// [`show_value`]).
fn attach_timestamps_one(store: &Store, id: &str, record: &mut Value) -> Result<()> {
    let (created, updated) = store.item_timestamps_of(id)?;
    if let Some(c) = created {
        record["created_at"] = json!(c);
    }
    if let Some(u) = updated {
        record["updated_at"] = json!(u);
    }
    Ok(())
}

/// [`items_in_order_value`] with the sparse declared `custom` map attached per record (6j6v.bbq6) —
/// the bare-lane analogue of [`list_to_value_with_custom`], for the `deferred`/`closed`/`archived`/
/// `search` reads. Byte-identical to `items_in_order_value` when the plugin declares no `[fields]`.
pub fn items_in_order_value_with_custom(
    cfg: &PluginConfig,
    store: &Store,
    items: &[ItemRow],
) -> Result<Value> {
    let mut v = items_in_order_value(items);
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    // Timestamps are plugin-independent, so they attach unconditionally (2kjy).
    attach_timestamps_lane(store, &ids, &mut v)?;
    // No declared fields ⇒ the custom read + attach can never add a key (ky26/ekf5): skip it.
    if !cfg.fields.is_empty() {
        attach_declared_custom(cfg, store, &ids, &mut v)?;
    }
    Ok(v)
}

/// [`blocked_to_value`] with the sparse declared `custom` map attached per record (6j6v.bbq6): the
/// `blocked` seam carries the same sparse `custom` join `list`/`next` do, alongside its `blockers`
/// list. Byte-identical to `blocked_to_value` when the plugin declares no `[fields]`.
pub fn blocked_to_value_with_custom(
    cfg: &PluginConfig,
    store: &Store,
    rows: &[BlockedItem],
) -> Result<Value> {
    let mut v = blocked_to_value(rows);
    let ids: Vec<&str> = rows.iter().map(|r| r.item.id.as_str()).collect();
    // Timestamps are plugin-independent, so they attach unconditionally (2kjy).
    attach_timestamps_lane(store, &ids, &mut v)?;
    if !cfg.fields.is_empty() {
        attach_declared_custom(cfg, store, &ids, &mut v)?;
    }
    Ok(v)
}

/// The sparse declared `custom` object per id (6j6v.bbq6): `id → {field: value, …}` for each id that
/// has ≥1 declared custom value, empty otherwise. For surfaces that don't build a `_with_custom`
/// array but still want the SAME projection (the `prime --json` `next` list, attached CLI-side so
/// `prime`'s public [`PrimeReport`] stays untouched). Empty map when no `[fields]` are declared.
pub fn declared_custom_by_id(
    cfg: &PluginConfig,
    store: &Store,
    ids: &[&str],
) -> Result<BTreeMap<String, Value>> {
    let mut out = BTreeMap::new();
    if cfg.fields.is_empty() {
        return Ok(out);
    }
    let custom_by_id = store.custom_fields_of_bulk(ids)?;
    for id in ids {
        if let Some(fields) = custom_by_id.get(*id) {
            if let Some(custom) = declared_custom_value(cfg, fields) {
                out.insert((*id).to_string(), custom);
            }
        }
    }
    Ok(out)
}

/// [`next_to_value`] with the sparse declared `custom` map attached per record (§6). Byte-identical
/// to `next_to_value` for a plugin declaring no fields (or an item with no declared custom value).
pub fn next_to_value_with_custom(
    cfg: &PluginConfig,
    store: &Store,
    items: &[ItemRow],
) -> Result<Value> {
    let mut v = next_to_value(store, items)?;
    // T4-review efficiency (ky26): a plugin with NO declared `[fields]` (issue-tracker/personal-todo)
    // can never surface a `custom` map, so skip the `custom_fields_of_bulk` read + attach entirely —
    // no guaranteed-empty query on every lane read. Behavior-preserving: `attach_custom_lane` already
    // adds no `custom` key when nothing declared is set, so the output is unchanged either way.
    if !cfg.fields.is_empty() {
        attach_custom_lane(cfg, store, items, &mut v)?;
    }
    Ok(v)
}

/// [`list_to_value`] with the sparse declared `custom` map attached per record (§6).
pub fn list_to_value_with_custom(
    cfg: &PluginConfig,
    store: &Store,
    items: &[ItemRow],
) -> Result<Value> {
    let mut v = list_to_value(store, items)?;
    // See [`next_to_value_with_custom`]: no declared fields ⇒ skip the custom read + attach (ky26).
    if !cfg.fields.is_empty() {
        attach_custom_lane(cfg, store, items, &mut v)?;
    }
    Ok(v)
}

/// Compare two candidates in rank order: the plugin's `ranking.next` keys in declared order, with a
/// final id tiebreaker so the order is always a total, deterministic one. Shared by the item and
/// blocked orderers ([`order_by`]/[`order_blocked`]), and the tiebreak inside each of `next`'s
/// finish-first tiers ([`order_next_tiered`]) — the tiers reorder candidates, they never re-rank
/// them. The status precedence that ranks `in_progress` ahead of `open` is no longer hardcoded here
/// (sp6.5): it is the first `ranking.next` key both bundled plugins declare, so ranking is entirely
/// plugin-driven — no command-code special-case undercuts the "all policy flows from the config" seam.
fn rank_cmp(cfg: &PluginConfig, a: &ItemRow, b: &ItemRow) -> std::cmp::Ordering {
    for key in &cfg.ranking.next.order {
        let c = cmp_rank_key(a, b, key);
        if c != std::cmp::Ordering::Equal {
            return c;
        }
    }
    a.id.cmp(&b.id)
}

// ---- ordering axis (C2 #916.2) ---------------------------------------------
//
// Selection (which items) and derivation (which lane) are settled elsewhere; ORDER is its own
// axis: a per-verb default plus an optional `--sort` override on every read verb. The orderer
// lives here, in the shared facade, so the CLI and an in-process consumer see the identical order
// (seam invariant #9t7). Later children (C3/C6) extend `SortKey` with the defer/closed/archive
// orders; the axis is established here and the `blocked` default is corrected from id to rank.

/// A `--sort` ordering key. C2 shipped `rank`/`id`; C3 (#916.4) adds the three lane orders so the
/// `--sort` axis is uniform across every read verb (each verb just picks a different default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    /// The active plugin's `next` ranking policy (in_progress first, then its keys, id tiebreak).
    Rank,
    /// Lexicographic by short-id — stable and plugin-independent.
    Id,
    /// `defer_until` ascending — the soonest-returning deferred item first (the `deferred` default).
    Defer,
    /// `closed_at` descending — most-recently-closed first (the `closed` default; recency).
    Closed,
    /// `archived` descending — most-recently-archived first (the `archived` default; recency).
    Archived,
}

impl SortKey {
    /// Parse a key name, or `None` if unknown. [`parse_sort`] wraps this with a loud error.
    pub fn parse(s: &str) -> Option<SortKey> {
        match s {
            "rank" => Some(SortKey::Rank),
            "id" => Some(SortKey::Id),
            "defer" => Some(SortKey::Defer),
            "closed" => Some(SortKey::Closed),
            "archived" => Some(SortKey::Archived),
            _ => None,
        }
    }

    /// The canonical key name.
    pub fn as_str(self) -> &'static str {
        match self {
            SortKey::Rank => "rank",
            SortKey::Id => "id",
            SortKey::Defer => "defer",
            SortKey::Closed => "closed",
            SortKey::Archived => "archived",
        }
    }

    /// Every accepted key name, for help text and validation messages.
    pub const NAMES: &'static [&'static str] = &["rank", "id", "defer", "closed", "archived"];
}

/// Per-verb default order, centralised so the CLI and the embed Engine resolve the SAME default.
/// `next` and `blocked` rank by default (the C2 fix is `blocked`, previously id-ordered); `list`
/// stays **id**-ordered so `list --json` remains byte-identical across plugins — the canonical
/// record is plugin-independent, and ranking is the one plugin-policy axis a `list` consumer opts
/// into with `--sort rank`. The lane verbs (C3) each default to their natural in-lane order.
pub const DEFAULT_SORT_LIST: SortKey = SortKey::Id;
pub const DEFAULT_SORT_NEXT: SortKey = SortKey::Rank;
pub const DEFAULT_SORT_BLOCKED: SortKey = SortKey::Rank;
pub const DEFAULT_SORT_DEFERRED: SortKey = SortKey::Defer;
pub const DEFAULT_SORT_CLOSED: SortKey = SortKey::Closed;
pub const DEFAULT_SORT_ARCHIVED: SortKey = SortKey::Archived;

/// Parse a `--sort` value, failing with a loud validation error that names the accepted keys.
pub fn parse_sort(s: &str) -> Result<SortKey> {
    SortKey::parse(s).ok_or_else(|| {
        NxfError::validation(format!(
            "unknown sort key '{s}'; valid: {}",
            SortKey::NAMES.join(", ")
        ))
    })
}

/// Compare two items on one date-valued field with `dir`, as the lane orders need it (C3 #916.4).
/// The values are opaque LWW strings a peer could even set malformed, so the comparison is a plain
/// **string** compare (lexicographic == chronological for ISO-8601) — a TOTAL order over ANY
/// string, never a parse that could panic or partial-order (PR #98 review, Integrity #3). Null
/// placement is fixed (missing dates sort LAST, independent of `dir`), and `id` is the final
/// tiebreak, so the whole order is total and deterministic even with malformed or absent instants.
fn date_cmp(a: &ItemRow, b: &ItemRow, field: &str, dir: Dir) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let cmp = match (raw_field(a, field), raw_field(b, field)) {
        (Some(x), Some(y)) => {
            let c = x.cmp(&y);
            match dir {
                Dir::Asc => c,
                Dir::Desc => c.reverse(),
            }
        }
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater, // a missing instant sorts last…
        (Some(_), None) => Ordering::Less,    // …regardless of direction
    };
    cmp.then_with(|| a.id.cmp(&b.id))
}

/// Compare two items by `key` — the one orderer every read verb shares (C2/C3). The lane keys
/// resolve to a [`date_cmp`] over the matching field; `rank`/`id` keep their C2 meaning.
fn sort_cmp(cfg: &PluginConfig, a: &ItemRow, b: &ItemRow, key: SortKey) -> std::cmp::Ordering {
    match key {
        SortKey::Rank => rank_cmp(cfg, a, b),
        SortKey::Id => a.id.cmp(&b.id),
        SortKey::Defer => date_cmp(a, b, "defer_until", Dir::Asc),
        SortKey::Closed => date_cmp(a, b, "closed_at", Dir::Desc),
        SortKey::Archived => date_cmp(a, b, "archived", Dir::Desc),
    }
}

/// Order `items` in place by `key` (the shared orderer behind `list`/`next`/the lane verbs).
fn order_by(cfg: &PluginConfig, items: &mut [ItemRow], key: SortKey) {
    items.sort_by(|a, b| sort_cmp(cfg, a, b, key));
}

/// Order blocked rows in place by `key`, applying the same orderer to each row's item.
fn order_blocked(cfg: &PluginConfig, rows: &mut [BlockedItem], key: SortKey) {
    rows.sort_by(|a, b| sort_cmp(cfg, &a.item, &b.item, key));
}

/// The `--json` array for an ALREADY-ORDERED item set: serialized in the given order (NOT
/// re-sorted by id). The bare-projection lane verbs (`deferred`/`closed`/`archived`/`search`) use it;
/// `list`/`next` instead take [`list_to_value`]/[`next_to_value`], which append the read-layer joins
/// (`parent_closed_reason`, and `parent` for `next`). [`items_value`] is the id-sorted projection.
pub fn items_in_order_value(items: &[ItemRow]) -> Value {
    Value::Array(items.iter().map(item_value).collect())
}

/// Compare two items on one ranking key. An ENUM-INDEX key (`precedence` set, sp6.5) orders by the
/// value's position in the precedence list — type `epic` before `bug`, status `in_progress` before
/// `open` — the same index-not-bytes idea as the priority ordinal, but for a named set. Otherwise a
/// FIELD key: priority is stored as its canonical ordinal (8qv.2), so the numeric-aware path orders
/// the variants by their rank (0 highest) — `later` (ordinal 2) never sorts before `now` (ordinal
/// 0). Direction applies to present values; null placement (`nulls first|last`) is independent of
/// direction.
fn cmp_rank_key(a: &ItemRow, b: &ItemRow, key: &RankKey) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    // Enum-index ordering: the precedence list IS the order (dir/nulls do not apply). A value absent
    // from the list sorts after every listed one; the id tiebreak in `rank_cmp` settles any tie.
    if !key.precedence.is_empty() {
        let oa = key.precedence_ordinal(raw_field(a, &key.field).as_deref());
        let ob = key.precedence_ordinal(raw_field(b, &key.field).as_deref());
        return oa.cmp(&ob);
    }
    match (raw_field(a, &key.field), raw_field(b, &key.field)) {
        (Some(x), Some(y)) => {
            // Numeric-aware: when BOTH values parse as integers, compare numerically so the
            // priority ordinal (and any future numeric field) doesn't mis-sort "10" before "2".
            // Otherwise (ISO dates, ids, any non-numeric label) fall back to lexicographic, which
            // is already the correct order for those (lexicographic == chronological for dates).
            let c = match (x.parse::<i64>(), y.parse::<i64>()) {
                (Ok(xi), Ok(yi)) => xi.cmp(&yi),
                _ => x.cmp(&y),
            };
            match key.dir {
                Dir::Asc => c,
                Dir::Desc => c.reverse(),
            }
        }
        (None, None) => Ordering::Equal,
        (None, Some(_)) => match key.nulls {
            Nulls::First => Ordering::Less,
            Nulls::Last => Ordering::Greater,
        },
        (Some(_), None) => match key.nulls {
            Nulls::First => Ordering::Greater,
            Nulls::Last => Ordering::Less,
        },
    }
}

// ---- lane-ranked search (C6 #916.6) ----------------------------------------
//
// `search` is a facade method (not CLI-only) so the CLI and an in-process consumer share ONE
// lane-ranked implementation — no client reconstructs lane membership to group matches (seam
// invariant #9t7; the MCP seam #76u benefits too). Selection is a substring match over the text
// fields; ORDER groups matches by lane priority (next+ip → blocked → deferred → closed [→
// archived]), each group in its natural in-lane order. Because the lanes partition the live space
// (C3), every match lands in exactly one group — no duplicates.

/// The options for [`search`] beyond the query string. `Default` is the everyday search: no
/// filters, archive excluded, lane-grouped order.
#[derive(Debug, Clone, Copy, Default)]
pub struct SearchArgs<'a> {
    /// Optional core-status selection filter (applied before grouping).
    pub status: Option<&'a str>,
    /// Optional core-type selection filter (`project`/`task`).
    pub item_type: Option<&'a str>,
    /// Append the archived group (lowest priority) instead of excluding it.
    pub include_archived: bool,
    /// Search ONLY the archive (overrides `include_archived`); the live lanes are skipped.
    pub archived_only: bool,
    /// Flatten the lane grouping into one order by this key (e.g. `id`); `None` keeps the groups.
    pub sort: Option<SortKey>,
}

/// Does `item` match `needle` (already lowercased) in its title, description, design, DoD, or any
/// note? The shared substring selector — the materialized projection, never the op-log (#25i).
fn item_matches(store: &Store, item: &ItemRow, needle: &str) -> Result<bool> {
    let hit = |o: &Option<String>| {
        o.as_deref()
            .is_some_and(|s| s.to_lowercase().contains(needle))
    };
    if hit(&item.title)
        || hit(&item.description)
        || hit(&item.design)
        || hit(&item.completion_criterion)
    {
        return Ok(true);
    }
    Ok(store
        .notes_of(&item.id)?
        .iter()
        .any(|(_, body)| body.to_lowercase().contains(needle)))
}

// ---- effective lanes (07a.3 §8) --------------------------------------------
//
// C3 (#916.4) partitioned the live space by RAW own-status lane. Under the parent↔child coupling
// (07a) that partition is restated over EFFECTIVE lanes: a child's lane is masked/suppressed by its
// ancestors (docs/specs/07a-parent-child-status-coupling.md §1/§2). The lane-ranked `search`
// grouping consumes the effective lanes so a suppressed/closed-masked child groups by its effective
// lane instead of vanishing (the gap shipped in #146). The child's STORED status is never rewritten
// — only its lane membership shifts (the §4 closed-mask precedent extended to every mask).

/// The EFFECTIVE lane of a live item (07a.3 §8) — its ancestor-coupled lane. `archived` outranks
/// everything (orthogonal to the lifecycle); then the closed-mask (all live parents closed, §4);
/// then ancestor suppression (a blocked ancestor → `Blocked`, a deferred-only ancestor → `Deferred`,
/// most-restrictive-wins §2); then the item's own lane, with the ready-mask (§1) showing an
/// in_progress child of an open/ready parent as `Ready`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    Archived,
    Closed,
    Blocked,
    Deferred,
    Ready,
    InProgress,
}

/// Precomputed membership sets backing [`effective_lane`], built once per `search` so classifying
/// each candidate is a set lookup rather than a re-derivation. Every set is a pure function of the
/// op-log (the `derive::*` queries).
struct EffectiveLaneSets {
    ready: std::collections::BTreeSet<String>,
    deferred_own: std::collections::BTreeSet<String>,
    suppressed_blocked: std::collections::BTreeSet<String>,
    suppressed_deferred_only: std::collections::BTreeSet<String>,
}

impl EffectiveLaneSets {
    fn compute(store: &Store, now: &str) -> Result<Self> {
        let conn = store.connection();
        Ok(Self {
            ready: derive::ready(conn, now)?.into_iter().collect(),
            deferred_own: derive::deferred(conn, now)?.into_iter().collect(),
            suppressed_blocked: derive::suppressed_by_blocked(conn)?.into_iter().collect(),
            suppressed_deferred_only: derive::suppressed_by_deferred_only(conn, now)?
                .into_iter()
                .collect(),
        })
    }
}

/// Classify `item` into its [`Lane`] via the precomputed `sets`. Precedence follows §1/§2: archived,
/// then the closed-mask, then ancestor suppression (blocked before deferred-only), then the item's
/// own lane — with the ready-mask on an in_progress child that still has an open (non-gating) parent.
fn effective_lane(store: &Store, item: &ItemRow, sets: &EffectiveLaneSets) -> Result<Lane> {
    if item.archived.is_some() {
        return Ok(Lane::Archived);
    }
    if item.status.as_deref() == Some("closed") {
        return Ok(Lane::Closed);
    }
    // Closed-mask: a still-open child whose every live parent is closed (§4). The SAME condition as
    // the `parent_closed_reason` join, so the closed lane and `show`'s reason agree by construction.
    if item.status.as_deref() == Some("open") && !parent_closed_reason(store, item)?.is_empty() {
        return Ok(Lane::Closed);
    }
    if sets.suppressed_blocked.contains(&item.id) {
        return Ok(Lane::Blocked);
    }
    if sets.suppressed_deferred_only.contains(&item.id) {
        return Ok(Lane::Deferred);
    }
    match item.status.as_deref() {
        // An open, non-suppressed, non-masked item is in exactly one of ready/deferred/blocked by its
        // OWN state (the three derived sets partition the open space); `ready` is already effective.
        Some("open") => Ok(if sets.ready.contains(&item.id) {
            Lane::Ready
        } else if sets.deferred_own.contains(&item.id) {
            Lane::Deferred
        } else {
            Lane::Blocked
        }),
        // ready-mask (§1): any open parent left here is non-gating (a gating one would have
        // suppressed the child above), hence effectively ready — so the in_progress child is shown
        // `Ready`. Its stored status is untouched; only the displayed lane shifts (judgment call per
        // the 07a.3 note: effective-lane only, NOT a status rewrite, to keep the `next`
        // split-on-raw-status contract intact). Otherwise the child shows its own in_progress lane.
        Some("in_progress") if has_open_parent(store, item)? => Ok(Lane::Ready),
        _ => Ok(Lane::InProgress),
    }
}

/// Does `item` have at least one live (non-deleted) direct parent whose stored status is `open`? The
/// ready-mask predicate (§1), evaluated only after suppression is ruled out — so an open parent here
/// is necessarily a non-gating (ready) one.
fn has_open_parent(store: &Store, item: &ItemRow) -> Result<bool> {
    // Fallible parent read (07a.7): `parents_of_result` + fallible `get_item` so the ready-mask
    // probe on the long-lived search seam maps a db error to `io` instead of panicking.
    for pid in store.parents_of_result(&item.id)? {
        if let Some(p) = store.get_item(&pid)? {
            if p.deleted.as_deref() != Some("1") && p.status.as_deref() == Some("open") {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Lane-ranked substring search (C6 #916.6). Selection: a case-insensitive substring match over
/// title/description/design/DoD/notes, refined by the optional `status`/`item_type` filters;
/// `deleted` is always excluded. Order: by default the matches are grouped by lane priority
/// (`next`+in_progress → `blocked` → `deferred` → `closed`, archive excluded), each group in its
/// natural in-lane order — so every match appears in exactly one group. `--include-archived`
/// appends the archived group last; `--archived-only` searches only the archive. A `sort` override
/// flattens the grouping into one flat order by that key over all (selected) matches. `now` is the
/// defer boundary.
pub fn search(
    cfg: &PluginConfig,
    store: &Store,
    now: &str,
    query: &str,
    args: SearchArgs,
) -> Result<Vec<ItemRow>> {
    let needle = query.to_lowercase();
    // Fallible selector (#76u.13): the note-body match reads `notes_of`, which can surface a db
    // error. The `&&` chain keeps the original short-circuit — the notes read runs only after the
    // cheap deleted/status/type filters pass — so a non-matching item never touches the store.
    let selected = |i: &ItemRow| -> Result<bool> {
        Ok(i.deleted.as_deref() != Some("1")
            && args.status.is_none_or(|s| i.status.as_deref() == Some(s))
            && args
                .item_type
                .is_none_or(|t| i.item_type.as_deref() == Some(t))
            && item_matches(store, i, &needle)?)
    };

    // A `--sort` override drops the grouping: one flat order over all selected matches, honouring
    // the archive scope (only / include / exclude).
    if let Some(key) = args.sort {
        let candidates: Vec<ItemRow> = store
            .list_items()?
            .into_iter()
            .filter(|i| {
                if args.archived_only {
                    i.archived.is_some()
                } else if args.include_archived {
                    true
                } else {
                    i.archived.is_none()
                }
            })
            .collect();
        let mut items = try_retain(candidates, &selected)?;
        order_by(cfg, &mut items, key);
        return Ok(items);
    }

    // `--archived-only`: the archive is the whole result, in archive-recency order.
    if args.archived_only {
        return try_retain(archived(cfg, store, None)?, &selected);
    }

    // Lane-grouped over EFFECTIVE lanes (07a.3 §8): classify every matching live item by its
    // ancestor-coupled lane, then emit the lanes in priority order, each in its natural in-lane
    // order. The effective lanes partition the live space (every item in exactly one lane), so this
    // drops nothing and duplicates nothing — closing the gap where a suppressed/closed-masked child
    // vanished from the own-status lanes.
    let sets = EffectiveLaneSets::compute(store, now)?;
    let candidates: Vec<ItemRow> = store
        .list_items()?
        .into_iter()
        .filter(|i| args.include_archived || i.archived.is_none())
        .collect();
    let (mut next_ip, mut blk, mut def, mut cls, mut arc) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for item in try_retain(candidates, &selected)? {
        match effective_lane(store, &item, &sets)? {
            // NOTE: `search` folds Ready + InProgress into one group, so the ready-mask (§1) is
            // observationally inert HERE — it only distinguishes the two for a future consumer that
            // ranks them apart (`prime`/a board). Do not assume the mask is load-bearing for search.
            Lane::Ready | Lane::InProgress => next_ip.push(item),
            Lane::Blocked => blk.push(item),
            Lane::Deferred => def.push(item),
            Lane::Closed => cls.push(item),
            Lane::Archived => arc.push(item),
        }
    }
    // Each group in its natural in-lane order (the per-lane verb defaults).
    order_by(cfg, &mut next_ip, SortKey::Rank);
    order_by(cfg, &mut blk, DEFAULT_SORT_BLOCKED);
    order_by(cfg, &mut def, DEFAULT_SORT_DEFERRED);
    order_by(cfg, &mut cls, DEFAULT_SORT_CLOSED);
    order_by(cfg, &mut arc, DEFAULT_SORT_ARCHIVED);
    let mut out = next_ip;
    out.extend(blk);
    out.extend(def);
    out.extend(cls);
    out.extend(arc); // empty unless --include-archived (candidates excluded archived otherwise)
    Ok(out)
}

/// Retain the items for which a FALLIBLE predicate holds, short-circuiting on the first error — the
/// `Result`-aware analog of `Iterator::filter` the lane-ranked [`search`] needs now its match
/// predicate reads `notes_of` (#76u.13).
fn try_retain(
    items: Vec<ItemRow>,
    pred: &impl Fn(&ItemRow) -> Result<bool>,
) -> Result<Vec<ItemRow>> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if pred(&item)? {
            out.push(item);
        }
    }
    Ok(out)
}

// ---- show ------------------------------------------------------------------

/// The full detail of one item: the canonical record plus its dependencies (id + live status),
/// the items it contributes to (vf4 — never blocks, no status), and its notes (oldest first).
/// `#[non_exhaustive]` (PR #255 review, Integrity #1): the only constructor is [`show`] — nothing
/// outside this crate builds a `ShowRecord` via struct literal (readers only ever destructure with
/// `..` or access fields directly) — so growing this record with a new sparse read-layer join
/// (as `parent_closed_reason` and `parents`/`parents_notice` already have) is a genuine additive,
/// backward-compatible change, never a `facade: breaking` one.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ShowRecord {
    pub item: ItemRow,
    /// `(dep_id, live_status)`, id-sorted; status is `None` if the target is deleted/missing.
    pub deps: Vec<(String, Option<String>)>,
    /// Ids this item contributes to, sorted.
    pub contributes_to: Vec<String>,
    /// `(note_id, body)`, canonical oldest-first order.
    pub notes: Vec<(String, String)>,
    /// The closed-mask reasons (07a.3): `(parent_id, closing_comment)` for every closed parent when
    /// this open item's parents are ALL closed; empty otherwise. The item's own `status` is untouched.
    pub parent_closed_reason: Vec<(String, Option<String>)>,
    /// The item's belongs-to/containment parent ids, sorted (6j6v.zvd0); empty when it has none.
    /// `contributes_to` edges are NOT parents here — see [`parents_notice`] for the derived notice.
    pub parents: Vec<String>,
    /// The verbatim parent-pointer notice (6j6v.zvd0) when [`parents`](Self::parents) is
    /// non-empty; `None` for a parentless item.
    pub parents_notice: Option<String>,
}

impl ShowRecord {
    /// The `show` `--json` object: `{ item, deps, contributes_to, notes }`, plus a sparse
    /// `parent_closed_reason` join (`[{parent_id, reason}]`) only when the item is closed-masked,
    /// and sparse `parents`/`parents_notice` (6j6v.zvd0) only when the item has parent(s).
    pub fn to_value(&self) -> Value {
        let mut v = json!({
            "item": item_value(&self.item),
            "deps": self.deps
                .iter()
                .map(|(d, st)| json!({ "id": d, "status": st }))
                .collect::<Vec<_>>(),
            "contributes_to": self.contributes_to,
            "notes": self.notes
                .iter()
                .map(|(nid, body)| json!({ "id": nid, "body": body }))
                .collect::<Vec<_>>(),
        });
        if let Some(reasons) = parent_closed_reason_value(&self.parent_closed_reason) {
            v["parent_closed_reason"] = reasons;
        }
        if !self.parents.is_empty() {
            v["parents"] = json!(self.parents);
            v["parents_notice"] = json!(self.parents_notice);
        }
        v
    }
}

/// Detail of `id`, or `not_found` if it is missing/deleted.
pub fn show(store: &Store, id: &str) -> Result<ShowRecord> {
    let item = require_item(store, id)?;
    let deps = deps_with_status(store, id)?;
    let contributes_to = store.contributes_to_of(id)?;
    let notes = store.notes_of(id)?;
    let parent_closed_reason = parent_closed_reason(store, &item)?;
    let parents = live_parents_of(store, id)?;
    let notice = parents_notice(&parents);
    Ok(ShowRecord {
        item,
        deps,
        contributes_to,
        notes,
        parent_closed_reason,
        parents,
        parents_notice: notice,
    })
}

/// The `show` `--json` object with the item's user labels attached (h89s.2). A store-aware wrapper
/// over [`ShowRecord::to_value`] — labels are an OR-set view, not a `ShowRecord` field, so they ride
/// here exactly as they do in [`list_to_value`]/[`next_to_value`]. Additive: `ShowRecord` and its
/// `to_value` are untouched (the facade SemVer surface stays byte-compatible). Sparse: the `labels`
/// key is present only when the item has labels, so an unlabelled record is byte-identical to the
/// pre-labels output.
pub fn show_value(store: &Store, id: &str) -> Result<Value> {
    let mut v = show(store, id)?.to_value();
    let labels = store.labels_of(id)?;
    if !labels.is_empty() {
        v["labels"] = json!(labels);
    }
    // nxf 6j6v.8dbe: the chat threads about this item, sparse and additive exactly like `labels`
    // above — an unlinked item's output is byte-identical to the pre-link one. BOTH weights ride
    // here, unlike the lane reads: a caller that asked about THIS item by name has already narrowed
    // the field, so a link that only grazed it is signal rather than noise.
    let thread_links = store.thread_links_of_item(id)?;
    if !thread_links.is_empty() {
        v["conversations"] = thread_links_to_value(&thread_links);
    }
    // 2kjy: the item's op-log created_at/updated_at (sparse), plus each note's created_at — additive
    // read-layer keys, so `ShowRecord`/`to_value` (the canonical record) stay byte-stable.
    attach_timestamps_one(store, id, &mut v["item"])?;
    attach_note_created(store, id, &mut v)?;
    Ok(v)
}

/// Attach each note's sparse `created_at` (the note-add op's `wall_clock`) to the `notes` array of a
/// `show` `--json` object, matched by note id (2kjy). A note whose op carried no `wall_clock` keeps
/// exactly the `{id, body}` shape, so an un-stamped store is byte-identical to the pre-2kjy output.
fn attach_note_created(store: &Store, id: &str, v: &mut Value) -> Result<()> {
    let created: BTreeMap<String, String> = store
        .notes_with_created_of(id)?
        .into_iter()
        .filter_map(|(nid, _body, c)| c.map(|c| (nid, c)))
        .collect();
    if created.is_empty() {
        return Ok(());
    }
    if let Some(notes) = v.get_mut("notes").and_then(Value::as_array_mut) {
        for note in notes {
            if let Some(c) = note
                .get("id")
                .and_then(Value::as_str)
                .and_then(|nid| created.get(nid))
            {
                note["created_at"] = json!(c);
            }
        }
    }
    Ok(())
}

/// [`show_value`] with the sparse declared `custom` map attached (§2.3/§6) — the single-item sibling
/// of the lane `_with_custom` wrappers. Reads the item's set custom values once
/// (`custom_fields_of`), narrows them to the active plugin's declared fields, and attaches the
/// sparse `custom` key iff any remain. Byte-identical to `show_value` when nothing declared is set.
pub fn show_value_with_custom(cfg: &PluginConfig, store: &Store, id: &str) -> Result<Value> {
    let mut v = show_value(store, id)?;
    // T4-review efficiency (ky26): no declared `[fields]` ⇒ the `custom` key can never appear, so skip
    // the `custom_fields_of` read entirely (the single-item sibling of the lane short-circuit above).
    if cfg.fields.is_empty() {
        return Ok(v);
    }
    let fields = store.custom_fields_of(id)?;
    if let Some(custom) = declared_custom_value(cfg, &fields) {
        v["custom"] = custom;
    }
    Ok(v)
}

// ---- notes / mentions ------------------------------------------------------
//
// The `note list` / `mention list` reads as compute→render helpers so the CLI and the MCP seam
// share ONE definition (the #49g compute→render split) rather than each rebuilding the shape. Both
// take an already-resolved id (the caller resolves a bare suffix, exactly like `show`/`blocked`)
// and guard existence with `require_item`, so a list on a missing item is `not_found`, not empty.

/// An item's worklog notes as `(note_id, body)` pairs, in canonical oldest-first order (`notes_of`).
/// `not_found` if the item is missing/deleted (explicit lookup, not enumeration — as `show`).
pub fn notes(store: &Store, id: &str) -> Result<Vec<(String, String)>> {
    require_item(store, id)?;
    Ok(store.notes_of(id)?)
}

/// The `note list` `--json` array: `[{ id, body }]` in the same oldest-first order as [`notes`].
pub fn notes_to_value(notes: &[(String, String)]) -> Value {
    Value::Array(
        notes
            .iter()
            .map(|(nid, body)| json!({ "id": nid, "body": body }))
            .collect(),
    )
}

/// [`notes`] plus each note's op-log `created_at` (the note-add op's `wall_clock`, 2kjy) as
/// `(note_id, body, created_at)`. `not_found` if the item is missing/deleted, like [`notes`].
pub fn notes_with_created(
    store: &Store,
    id: &str,
) -> Result<Vec<(String, String, Option<String>)>> {
    require_item(store, id)?;
    Ok(store.notes_with_created_of(id)?)
}

/// The `note list` `--json` array WITH each note's sparse `created_at` — the same `{id, body,
/// created_at?}` note shape `show --json` carries (2kjy). A note whose op stamped no `wall_clock`
/// keeps the bare `{id, body}` shape, so the key is sparse exactly like the `show` note join.
pub fn notes_with_created_to_value(notes: &[(String, String, Option<String>)]) -> Value {
    Value::Array(
        notes
            .iter()
            .map(|(nid, body, created)| {
                let mut rec = json!({ "id": nid, "body": body });
                if let Some(c) = created {
                    rec["created_at"] = json!(c);
                }
                rec
            })
            .collect(),
    )
}

/// The short-ids `id` mentions (cites in its free text), sorted/deterministic
/// (`mentions_of_result`). `not_found` if the item is missing/deleted; a db fault maps to `io`
/// rather than unwinding — the fallible read seam, mirroring [`notes`] (#76u.14/07a.7).
pub fn mentions(store: &Store, id: &str) -> Result<Vec<String>> {
    require_item(store, id)?;
    Ok(store.mentions_of_result(id)?)
}

/// The `mention list` `--json` array: a plain array of the cited ids, in [`mentions`] order.
pub fn mentions_to_value(mentions: &[String]) -> Value {
    json!(mentions)
}

// ---- thread links (nxf 6j6v.8dbe) ------------------------------------------

/// The chat threads presently linked to `id`, sorted by thread id. `not_found` if the item is
/// missing/deleted (explicit lookup, as `show`); a db fault maps to `io`. Mirrors [`mentions`].
pub fn thread_links(store: &Store, id: &str) -> Result<Vec<ThreadLink>> {
    require_item(store, id)?;
    Ok(store.thread_links_of_item(id)?)
}

/// The board items `thread` is presently linked to, sorted by item id — the other direction, so a
/// thread can enumerate what it is about.
///
/// Deliberately does NOT validate the thread: flow does not own the chat id space (see
/// `write::thread_link_add`), and an unknown thread simply has no links, exactly as a linkless known
/// one does. A db fault maps to `io`.
pub fn thread_items(store: &Store, thread: &str) -> Result<Vec<ThreadLink>> {
    Ok(store.items_of_thread(thread)?)
}

/// The `thread list` / `show` `--json` array for a set of links: one object per link, in the
/// caller's order, carrying both endpoints and both attributes as canonical keys.
pub fn thread_links_to_value(links: &[ThreadLink]) -> Value {
    Value::Array(
        links
            .iter()
            .map(|l| {
                json!({
                    "thread_id": l.thread_id,
                    "item_id": l.item_id,
                    "relation": l.relation.as_str(),
                    "weight": l.weight.as_str(),
                })
            })
            .collect(),
    )
}

/// How many `bearing` threads are linked to `id` — the ONLY thread-link signal `next` carries.
///
/// The weight gate lives here rather than in the caller because it is the whole point of the weight:
/// a long, wandering conversation accumulates `passing` links to everything it touched, and if those
/// counted, every one of those items would advertise "there are conversations about this" — noise in
/// exactly the place a coding agent looks. A `passing` link is still visible on the item itself
/// (`show`), where the reader asked about that item specifically.
pub fn bearing_thread_count(store: &Store, id: &str) -> Result<usize> {
    Ok(store
        .thread_links_of_item(id)?
        .into_iter()
        .filter(|l| l.weight == LinkWeight::Bearing)
        .count())
}

/// The bulk sibling of [`bearing_thread_count`] (nxf 6j6v.8dbe): the bearing-thread count of MANY
/// items in ONE query, for decorating a whole lane without N per-item round-trips. Mirrors
/// [`labels_bulk`] — an id with no bearing links is simply absent from the map, and a db fault maps
/// to `io`.
pub fn bearing_thread_counts_bulk(store: &Store, ids: &[&str]) -> Result<BTreeMap<String, usize>> {
    Ok(store.bearing_thread_counts(ids)?)
}

// ---- labels (h89s.2) -------------------------------------------------------

/// An item's present user labels, sorted (`labels_of`). `not_found` if the item is missing/deleted
/// (explicit lookup, not enumeration — as `show`); a db fault maps to `io`. Mirrors [`mentions`].
pub fn labels(store: &Store, id: &str) -> Result<Vec<String>> {
    require_item(store, id)?;
    Ok(store.labels_of(id)?)
}

/// The bulk sibling of [`labels`] (1w5v): the present labels of MANY items in ONE query, for
/// decorating a whole lane (the app-bridge `with_labels` seam) without N per-item round-trips.
/// Returns `id → sorted labels` for every id carrying ≥1 label; an id with no labels — or a
/// stale/missing id — is simply absent from the map. Deliberately does NOT `require_item`/`not_found`
/// like singular [`labels`]: a lane hands ids it has already read, and a miss just means "no labels"
/// (the same value singular `labels` yields as an empty vec). A db fault maps to `io`.
pub fn labels_bulk(store: &Store, ids: &[&str]) -> Result<BTreeMap<String, Vec<String>>> {
    Ok(store.labels_of_bulk(ids)?)
}

/// The `label list` `--json` array: a plain array of the item's labels, in [`labels`] order.
pub fn labels_to_value(labels: &[String]) -> Value {
    json!(labels)
}

/// Filter an item set to those carrying `label` — the shared `--label` filter behind `list`/`next`
/// (kept out of their signatures so the v0.15 read surface stays additive). A db fault on the
/// per-item label read maps to `io` rather than unwinding.
pub fn with_label(store: &Store, items: Vec<ItemRow>, label: &str) -> Result<Vec<ItemRow>> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if store.labels_of(&item.id)?.iter().any(|l| l == label) {
            out.push(item);
        }
    }
    Ok(out)
}

// ---- prime -----------------------------------------------------------------

/// The Core Rules surfaced at session start (nexus-flow-6pu, regrouped in 0b4): the tracker
/// rule first (track all work in nexus-flow at all), then the order an agent works — find work,
/// claim/close, the machine contract, the stabilization convention, the citation rule. These
/// are issue-tracker *politics* and belong in a plugin config eventually (see E3).
const CORE_RULES: &[&str] = &[
    "Track all work in nexus-flow itself: open an item for every task rather than keeping a \
     separate TODO list or scratch notes — the board is the single source of truth.",
    "Find what to work on next: `nxf next`.",
    "Claim work before starting it: `nxf claim <id>`.",
    "Close with a reason: `nxf close <id> --reason <text>`.",
    "Use `--json` everywhere for deterministic, machine-readable output.",
    // 07a.4: now that `parent` gates a child at its container's lifecycle, the edge choice is
    // load-bearing — teach it in one tight principle (the close-time warning + runtime lanes catch
    // the rest). One worked one-liner is enough (docs/specs/07a-parent-child-status-coupling.md §3).
    "Choose the containment edge deliberately: `parent` is gating — a child rests when its \
     container rests (a deferred, blocked, or closed parent propagates down and hides or masks the \
     child). For a loose association that must NOT gate the child, use `contributes_to` (e.g. cream \
     belongs to the shopping list but only contributes to the birthday plan, so deferring the \
     birthday never hides the cream).",
    // oxmu: the defer-vs-WAIT-chore convention — keep the deferred lane honest (only real dates
    // defer) and give cross-workspace waits a first-class local anchor (there are no cross-workspace
    // deps). The guide topic `deferring-and-waiting` carries the full worked example.
    "Defer only for a real calendar date — a day before which the item genuinely cannot start \
     (`--defer <date>` on create, or `nxf update <id> --set defer=<date>`); a placeholder date for \
     \"someday, once X ships\" is an anti-pattern that hides the item on a false promise. To wait \
     on an external DELIVERY instead — there are no cross-workspace dependencies — model the \
     delivery as an open WAIT chore in this workspace (title it `WAIT: <what ships>`, e.g. `WAIT: \
     acme-api v2`), have the dependents `nxf dep add <id> <wait-chore>` onto it, and CLOSE the \
     chore — with the delivered version in the reason — to release the whole chain. Each workspace \
     keeps its own anchor; see `nxf guide deferring-and-waiting`.",
    // 8qv.6: the stabilization convention — correct the field model once, then append knowledge.
    // 8qv.9: plus the *why* in one sentence — preserving the original intent is what lets us learn
    // from the intent-vs-outcome pair (the guide carries the full principle).
    "Correct an item's fields once shortly after creating it (e.g. to fold in a review); after \
     that keep the fields stable and record what you learn while working as append-only notes \
     (`nxf note add <id> <text>`), not field edits. The original title and description are \
     preserved on purpose: paired with the closing comment they form the intent-vs-outcome pair \
     you learn from, so when an item no longer fits, open a new one and close the old with a \
     reason instead of rewriting it past recognition.",
    "When you cite a task's short-id in free text (body or note), also record the \
     reference: `nxf mention add <this-item> <cited-id>`. It keeps the citation resolvable \
     if ids are remapped on sync, and never blocks (it is not a dependency).",
];

/// How many `next` rows `prime` shows before truncating (nexus-flow-e2z). The full list is
/// always one `nxf next` away; the header never hides the real total. Raised 7 → 15 with the
/// finish-first tiers: a started epic with ~10 children must not be truncated mid-cluster, or an
/// agent decides on a partial list.
///
/// A VALUE, not a mechanism (6j6v.8pf2): the cutting itself is [`truncate_next`], which
/// `nxf next --limit` reaches through too — `prime` is simply the caller that names this size.
const PRIME_NEXT_LIMIT: usize = 15;

/// The predicate after "nexus-flow is …" when the active plugin declares no `tagline`
/// (bare engine / future plugins) — so `prime` never renders a dangling "nexus-flow is ."
/// (nexus-flow-6pu).
const DEFAULT_TAGLINE: &str = "a tracker for projects and tasks";

/// The invariant half of the purpose paragraph: how the core derives the work list. Plugin-
/// independent and stable (the plugin-specific predicate is prepended from `tagline`). Begins
/// from `next`/`blocked` and names `priority`; keeps "rather than stored" (nexus-flow-6pu).
const PRIME_DERIVATION: &str = "You record items, the dependencies between them, due/defer \
    dates, and priority; `next` and `blocked` are then derived deterministically from that \
    graph rather than stored, so the work list is always consistent.";

/// The context-recovery note (nexus-flow-0b4, adapted from bd's): what `prime` is for and that
/// the host re-runs it automatically. Rendered as a blockquote in the human view; a plain string
/// field in `--json`.
const PRIME_CONTEXT_RECOVERY: &str = "Run `nxs prime` after a context compaction, /clear, or a \
    new session — hosts auto-call it in Claude Code when a nexus-flow workspace is resolved.";

/// The escaping-free long-text input pointer for the `create` section (nexus-flow-7s6 notes):
/// agents bootstrap via `prime` and won't read `--help`, so 95d's input modes are surfaced here.
const PRIME_LONGTEXT_HINT: &str = "Long text without shell escaping: pipe a field via STDIN \
    (`--description -`), read it from a file (`--description-file <path>`), or pipe the whole \
    item as JSON (`nxf create --json -`).";

/// The plugin-determined predicate alone — the active plugin's `tagline`, or [`DEFAULT_TAGLINE`]
/// when it declares none (nxf xe2z). Split out of [`prime_purpose`] so `nxf prime`'s human view
/// (rendered CLI-side, `crates/cli/src/commands/mod.rs`) can open the SAME plugin-declared
/// predicate with a different grammatical subject (`` `nxf` is … ``) than `--json`'s `purpose`
/// field, whose "nexus-flow is …" wording is a stable contract other consumers (the MCP server,
/// embedding apps) already read and is therefore left untouched by xe2z — only the human view's
/// fixed prose shrank; the shared canonical record did not.
pub fn prime_tagline(cfg: &PluginConfig) -> &str {
    cfg.tagline.as_deref().unwrap_or(DEFAULT_TAGLINE)
}

/// The purpose paragraph: the plugin-determined predicate ("nexus-flow is …") followed by the
/// invariant derivation sentence (nexus-flow-6pu). The predicate comes from the active plugin's
/// `tagline`, with [`DEFAULT_TAGLINE`] as the core fallback so it is never empty.
fn prime_purpose(cfg: &PluginConfig) -> String {
    format!("nexus-flow is {}. {PRIME_DERIVATION}", prime_tagline(cfg))
}

/// The active plugin's declared types joined for a `--type` example, e.g.
/// `bug|chore|decision|epic|feature` for issue-tracker or `project|termin|todo` for personal-todo.
/// Reads the authoritative `[types].list` (sp6.4), applying any sparse `vocabulary.types` display
/// override. The `BTreeSet` iterates in sorted order, so the output stays deterministic.
fn type_choices(cfg: &PluginConfig) -> String {
    cfg.types
        .list
        .iter()
        .map(|t| cfg.vocabulary.types.get(t).unwrap_or(t).as_str())
        .collect::<Vec<_>>()
        .join("|")
}

/// The dedicated `create` example for `prime` (nexus-flow-wt1): a full, copy-pasteable command
/// carrying the active plugin's type vocabulary and named priority variants (8qv.2), plus a
/// one-sentence recommendation to always set a priority (an item without one ranks last in
/// `next`). Every label is read from `cfg.priority`, never hardcoded.
fn create_example(cfg: &PluginConfig) -> (String, String) {
    let types = type_choices(cfg);
    let prios = cfg.priority.labels.join("|");
    let example = format!("nxf create --type <{types}> --title \"...\" --priority <{prios}>");
    let first = cfg
        .priority
        .labels
        .first()
        .map(String::as_str)
        .unwrap_or("");
    let last = cfg.priority.labels.last().map(String::as_str).unwrap_or("");
    let recommendation = format!(
        "Always set --priority (named variants, highest first: {first} … {last}); \
         an item created without a priority ranks last in `next`."
    );
    (example, recommendation)
}

/// One labelled group of the `prime` command reference (nexus-flow-0b4). `items` are
/// `(name, summary)` pairs; `name` is the bare subcommand (no `nxf ` prefix), so a consumer that
/// renders it prefixes it itself and keeps the canonical token. **Superseded 2026-08-28 (nxf
/// xe2z, task 2):** this used to name nxf's OWN CLI human view as that consumer ("the human view
/// adds it") — true when written, because `prime`'s old `## Essential Commands` section rendered
/// exactly this shape (`- \`nxf {name}\` — {summary}`). nxf's human view no longer renders
/// `commands` at all — it survives only under `--json` (still exactly this bare-name shape,
/// still read by the MCP server and embedding apps, which remain the "a consumer" this comment
/// means).
#[derive(Debug, Clone)]
pub struct CommandGroup {
    pub group: String,
    pub items: Vec<(String, String)>,
}

/// One multi-step recipe in the `prime` Common Workflows section (nexus-flow-0b4): a name and an
/// ordered list of command lines. **Superseded 2026-08-28 (nxf xe2z, task 2):** "rendered as a
/// `bash` block in the human view" was true when written — `prime`'s old `## Common Workflows`
/// section rendered each recipe's steps as 4-space-indented lines (a `bash`-flavored block,
/// deliberately NOT a fenced ` ```bash ` block: an embedded ``` fence would collide with the
/// golden test harness's own ` ```console ` fence around the whole example). nxf's human view no
/// longer renders `workflows` at all — it survives only under `--json`, in this same shape,
/// still read by the MCP server and embedding apps.
#[derive(Debug, Clone)]
pub struct Workflow {
    pub name: String,
    pub steps: Vec<String>,
}

fn group(name: &str, items: &[(&str, &str)]) -> CommandGroup {
    CommandGroup {
        group: name.into(),
        items: items
            .iter()
            .map(|(n, s)| ((*n).to_string(), (*s).to_string()))
            .collect(),
    }
}

/// The command reference shown at session start, grouped by task (nexus-flow-0b4, regrouped from
/// the flat wt1 list). Order within "Finding work" keeps `next` (the recommendation) first.
/// `create` is only a short pointer here — its full, vocabulary-aware example lives in the
/// create section. The `dep` entry spells out direction unambiguously (`A depends on B` ⇒ `B
/// blocks A`).
fn command_reference() -> Vec<CommandGroup> {
    vec![
        group(
            "Finding work",
            &[
                (
                    "next",
                    "what to work on (start here): work you can finish now, then started epics with their children, then the backlog",
                ),
                ("blocked", "list blocked work"),
                // 6j6v.jpcj: the deferred lane was documented only in the Core Rules prose, so an
                // agent scanning the command index never saw it. The summary also surfaces the
                // event-vs-date practice (WAIT: gate + dep) that likewise lived only in prose.
                (
                    "deferred",
                    "list deferred items (open, unblocked, future defer date); defer needs a real date — to wait on an event/delivery use a WAIT: chore dependents dep on, not a placeholder date (nxf guide deferring-and-waiting)",
                ),
                ("show <id>", "item detail with deps and notes"),
            ],
        ),
        group(
            "Creating & updating",
            &[
                ("create", "create an item — see the `create` section above"),
                ("update <id> --set k=v", "edit fields"),
                ("claim <id>", "mark in progress"),
                ("close <id> --reason", "close with a comment"),
                (
                    "schema",
                    "introspect this plugin's field model (--json) before create/update",
                ),
            ],
        ),
        group(
            "Dependencies & references",
            &[
                (
                    "dep add <from> <to>",
                    "<from> depends on <to> (so <to> blocks <from> and must close first)",
                ),
                (
                    "mention add <from> <to>",
                    "record a free-text short-id reference",
                ),
            ],
        ),
        group(
            "Notes & search",
            &[
                ("note add <id> <text>", "append a worklog note"),
                (
                    "search <query>",
                    "search title/description/design/DoD/notes",
                ),
            ],
        ),
    ]
}

/// The Common Workflows recipes (nexus-flow-0b4): nxf-native multi-step sequences. Deliberately
/// sync-free — sync guidance lives in the bound-only `sync` field and `session_close`'s
/// bound-only step (nexus-flow-smz; rendered as the human view's `## Sync` section and
/// `## Session close` bracket UNTIL nxf xe2z, task 2, retired both from that view — they still
/// carry this data under `--json`, unchanged), so single-device output carries no `nxs sync run`
/// noise. Placeholders stay vocabulary-GENERIC by design (`--type <type>`, not `<epic|issue>`):
/// these illustrate the *shape* of each workflow, while the copy-pasteable, vocabulary-aware
/// command lives in `create_example` — so the recipes need no `cfg`.
fn common_workflows() -> Vec<Workflow> {
    let recipe = |name: &str, steps: &[&str]| Workflow {
        name: name.into(),
        steps: steps.iter().map(|s| (*s).to_string()).collect(),
    };
    vec![
        recipe(
            "Starting work",
            &["nxf next", "nxf show <id>", "nxf claim <id>"],
        ),
        recipe(
            "Completing work",
            &[
                "nxf close <id> --reason \"...\"",
                "nxf next   # pick up the next unblocked item",
            ],
        ),
        recipe(
            "Creating dependent work",
            &[
                "nxf create --type <type> --title \"...\" --priority <P…>",
                "nxf create --type <type> --title \"...\" --priority <P…>",
                "nxf dep add <child> <prereq>   # child depends on prereq; prereq must close first",
            ],
        ),
    ]
}

/// The session-close lifecycle bracket (nexus-flow-smz, hybrid in 0b4): nxf steps, plus the
/// bound-only sync step, plus one non-prescriptive version-control reminder (NOT bd's literal
/// git commands — the engine stays out of the agent/git lane).
fn session_close(bound: bool) -> Vec<String> {
    let mut steps = vec![
        "Capture unfinished work as a note so the next session has the context: \
         `nxf note add <id> <text>`."
            .to_string(),
        "Close finished items with the reason they're done: `nxf close <id> --reason <text>`."
            .to_string(),
    ];
    if bound {
        steps.push(
            "Sync so your work reaches the server before you stop: `nxs sync run`.".to_string(),
        );
    }
    steps.push(
        "If the project is under version control, commit and push your code changes.".to_string(),
    );
    steps
}

/// The bound-only sync paragraph — `Some` only when a stream is bound (nexus-flow-smz): no sync
/// noise for single-device use. Feeds `--json`'s `sync` field; rendered as the human view's
/// `## Sync` section UNTIL nxf xe2z (task 2) retired that section — the field and this function
/// are unchanged, only the human rendering of it is gone.
fn sync_hint(bound: bool) -> Option<String> {
    bound.then(|| {
        "This workspace is bound to a sync stream — others' work arrives, and yours becomes \
         durable, only through `nxs sync run`. Pull at the start of a session and run it again \
         before you stop; the server is the durable truth."
            .to_string()
    })
}

/// One blocked blocker for `prime`: a blocker id, its status, and its direct fan-out
/// (`blocks_count` — how many items it holds up).
#[derive(Debug, Clone)]
pub struct Blocker {
    pub id: String,
    pub status: String,
    pub blocks_count: usize,
}

/// One blocked item for `prime`: its id and its open blockers, each annotated for leverage.
#[derive(Debug, Clone)]
pub struct BlockedEntry {
    pub id: String,
    pub blockers: Vec<Blocker>,
}

/// The blocked snapshot for `prime`: every blocked item with its open blockers + each blocker's
/// fan-out, ordered so the highest-leverage blocker surfaces first (then by id, deterministic).
/// Pure visibility — it never changes the `next` ranking (nexus-flow-bcj).
fn prime_blocked_entries(store: &Store) -> Result<Vec<BlockedEntry>> {
    let mut entries: Vec<BlockedEntry> = Vec::new();
    for id in derive::blocked(store.connection())? {
        let mut blockers = Vec::new();
        for (bid, status) in open_blockers(store, &id)? {
            blockers.push(Blocker {
                blocks_count: derive::dependents_count(store.connection(), &bid)?,
                id: bid,
                status,
            });
        }
        entries.push(BlockedEntry { id, blockers });
    }
    // Highest-leverage blocker first (descending max fan-out), id as the stable tiebreaker.
    entries.sort_by(|a, b| {
        let max = |e: &BlockedEntry| e.blockers.iter().map(|b| b.blocks_count).max().unwrap_or(0);
        max(b).cmp(&max(a)).then_with(|| a.id.cmp(&b.id))
    });
    Ok(entries)
}

/// The lean `next` projection for `prime` (nexus-flow-vux): only the displayed columns, in
/// CANONICAL form (ordinal priority, core status/type tokens) so `--json` stays
/// plugin-independent. The full canonical record stays on `nxf next`.
fn prime_next_value(row: &NextRow) -> Value {
    json!({
        "id": row.item.id,
        "type": row.item.item_type,
        "status": row.item.status,
        "priority": row.item.priority,
        "title": row.item.title,
        // #916.7: the same `parent` join shape as `nxf next` — one form, one contract.
        "parent": parent_value(row.parent.as_ref()),
    })
}

/// One `prime`/`next` row with its resolved parent join (#916.7). The item carries the displayed
/// fields; `parent` is the live `belongs_to` target (id + title + raw type), or `None`.
#[derive(Debug, Clone)]
pub struct NextRow {
    pub item: ItemRow,
    pub parent: Option<ParentRef>,
}

/// One entry in prime's "Recently Closed" section (r4kb): the capped, drift-free record the human
/// text and the `--json` view both render from a SINGLE source, so they cannot diverge.
/// `closing_notes` is ALREADY collapsed + capped to [`RECAP_NOTES_MAX`] (or the `—` placeholder when
/// the close carried no comment); `notes_truncated` says whether the cap bit. This is prime's
/// ginned-down mirror — the full comment is a `nxf show <id>` (or `nxf recap --json`) away.
#[derive(Debug, Clone)]
pub struct RecentClosed {
    pub id: String,
    /// The raw title (the human view escapes it at render; `--json` carries it verbatim, `null` when
    /// absent — matching the canonical `title` contract, never coerced to `""`).
    pub title: Option<String>,
    /// The collapsed + capped closing note, or `—` when the close left none.
    pub closing_notes: String,
    /// Whether [`RECAP_NOTES_MAX`] truncated the note.
    pub notes_truncated: bool,
    pub closed_at: Option<String>,
    /// Whether the item is also archived (recap includes archived-closed items).
    pub archived: bool,
}

/// The full `prime` session-bootstrap record (nexus-flow-0b4/vux): the plugin-determined
/// purpose, the context-recovery note, the Core Rules, the real `next` recommendation (truncated
/// to [`PRIME_NEXT_LIMIT`], with the true total), a leverage-aware blocked snapshot, the
/// recently-closed recall glance, the create section, grouped commands, the common-workflow
/// recipes, the session-close bracket, and — only when the workspace is bound — the sync hint.
/// **Superseded 2026-08-28 (nxf xe2z, task 2):** "the human and `--json` views render the same
/// record 1:1" was true when written and stays true for `to_value()` itself — every field above
/// is still there, unchanged. It stopped being true of the HUMAN (Markdown) rendering: `nxf
/// prime`'s SessionStart hook has a hard host-side byte ceiling, so the CLI's human view (see
/// `crates/cli/src/commands/mod.rs::prime`'s `else` branch) now renders a CURATED SUBSET of this
/// struct — the purpose predicate (reworded, not the `purpose` string verbatim), the workspace
/// prefix, a compact command cheatsheet, and 3 of the 9 `rules` — and drops `context_recovery`,
/// `commands`, `workflows`, `session_close`, `sync`, `create_recommendation`, and
/// `create_long_text_hint` entirely from that view. `to_value()` below is unaffected: it still
/// serializes every field, and is the stable canonical record the MCP server and embedding apps
/// read.
#[derive(Debug, Clone)]
pub struct PrimeReport {
    pub purpose: String,
    pub context_recovery: &'static str,
    pub rules: &'static [&'static str],
    /// The top `next` rows (ranked), truncated to [`PRIME_NEXT_LIMIT`], each with its parent join.
    pub next: Vec<NextRow>,
    /// The untruncated count of `next` candidates (the header shows "showing N of total").
    pub next_total: usize,
    pub blocked: Vec<BlockedEntry>,
    /// The most-recently closed items (Top-[`PRIME_RECENTLY_CLOSED_LIMIT`], recency order), each a
    /// capped mirror of the compact recall line — "what got finished lately" (r4kb).
    pub recently_closed: Vec<RecentClosed>,
    pub create_example: String,
    pub create_recommendation: String,
    pub create_long_text_hint: &'static str,
    pub commands: Vec<CommandGroup>,
    pub workflows: Vec<Workflow>,
    pub session_close: Vec<String>,
    /// The sync hint — `Some` only when the workspace is bound to a stream.
    pub sync: Option<String>,
}

impl PrimeReport {
    /// The `prime` `--json` object: the full canonical record, one key per [`PrimeReport`]
    /// field (nexus-flow-vux). `sync` is present only when bound. **Superseded 2026-08-28 (nxf
    /// xe2z, task 2):** this used to describe itself as "a 1:1 structured view of the human
    /// layout… mirroring the conditional `## Sync` section" — true when written, no longer true
    /// of the human view specifically (see [`PrimeReport`]'s doc comment for why); this method's
    /// OWN output is unaffected by that change and still serializes every field below.
    pub fn to_value(&self) -> Value {
        let mut v = json!({
            "purpose": self.purpose,
            "context_recovery": self.context_recovery,
            "rules": self.rules,
            "next": self.next.iter().map(prime_next_value).collect::<Vec<_>>(),
            "next_total": self.next_total,
            "blocked": self.blocked
                .iter()
                .map(|e| json!({
                    "id": e.id,
                    "blockers": e.blockers.iter().map(|b| json!({
                        "id": b.id, "status": b.status, "blocks_count": b.blocks_count,
                    })).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
            // r4kb: the capped 1:1 mirror of the human "Recently Closed" section — same `closing_notes`
            // string the text shows, so the two views can't drift. Full data is `nxf recap --json`.
            "recently_closed": self.recently_closed
                .iter()
                .map(|r| json!({
                    "id": r.id,
                    "title": r.title,
                    "closing_notes": r.closing_notes,
                    "notes_truncated": r.notes_truncated,
                    "closed_at": r.closed_at,
                    "archived": r.archived,
                }))
                .collect::<Vec<_>>(),
            "create": {
                "example": self.create_example,
                "recommendation": self.create_recommendation,
                "long_text_hint": self.create_long_text_hint,
            },
            "commands": self.commands
                .iter()
                .map(|g| json!({
                    "group": g.group,
                    "items": g.items
                        .iter()
                        .map(|(name, summary)| json!({ "name": name, "summary": summary }))
                        .collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
            "workflows": self.workflows
                .iter()
                .map(|w| json!({ "name": w.name, "steps": w.steps }))
                .collect::<Vec<_>>(),
            "session_close": self.session_close,
        });
        if let Some(sync) = &self.sync {
            v["sync"] = json!(sync);
        }
        v
    }
}

/// Compute the `prime` session-bootstrap record. `bound` (does `.nexusflow/sync.toml` exist?) is
/// an input because the facade can't see the workspace filesystem; the caller passes it
/// (nexus-flow-vux) — it gates the sync hint and the session-close sync step.
pub fn prime(cfg: &PluginConfig, store: &Store, now: &str, bound: bool) -> Result<PrimeReport> {
    let purpose = prime_purpose(cfg);
    // prime's snapshot is the tiered `next` default (C4 #916.3): finishable work first, then the
    // started clusters, then the backlog — started work included, as `next` now always is.
    // The cut goes through the SHARED mechanism (6j6v.8pf2), the same one `nxf next --limit` uses:
    // `prime` is only the caller that names the size. It is not a second, private truncation.
    let page = truncate_next(next(cfg, store, now, None)?, Some(PRIME_NEXT_LIMIT));
    let next_total = page.total;
    // Resolve each shown row's parent now (#916.7), while the store is in hand, so the report can
    // render the join in both views without re-touching the store.
    let mut next_rows: Vec<NextRow> = Vec::new();
    for item in page.items {
        let parent = parent_of(store, &item)?;
        next_rows.push(NextRow { item, parent });
    }
    let blocked = prime_blocked_entries(store)?;
    // The recency-recall glance (r4kb): the Top-N most-recently closed, each capped to the compact
    // line the human view and `--json` mirror both render — computed ONCE here from the shared
    // `recap` query, so the section can't drift from `nxf recap`.
    let recently_closed = recap(cfg, store, Some(PRIME_RECENTLY_CLOSED_LIMIT), None)?
        .iter()
        .map(|i| {
            let (closing_notes, notes_truncated) =
                recap_note(i.closing_comment.as_deref(), RECAP_NOTES_MAX);
            RecentClosed {
                id: i.id.clone(),
                title: i.title.clone(),
                closing_notes,
                notes_truncated,
                closed_at: i.closed_at.clone(),
                archived: i.archived.is_some(),
            }
        })
        .collect();
    let (create_example, create_recommendation) = create_example(cfg);
    Ok(PrimeReport {
        purpose,
        context_recovery: PRIME_CONTEXT_RECOVERY,
        rules: CORE_RULES,
        next: next_rows,
        next_total,
        blocked,
        recently_closed,
        create_example,
        create_recommendation,
        create_long_text_hint: PRIME_LONGTEXT_HINT,
        commands: command_reference(),
        workflows: common_workflows(),
        session_close: session_close(bound),
        sync: sync_hint(bound),
    })
}

/// The distinct item types present in the store that the active plugin does NOT declare (#yod):
/// a pre-plugin workspace can hold legacy core-enum types (`project`/`task`) that no longer resolve
/// to the active plugin's set, so they leak through the display and block parent-edge writes. Sorted
/// and deduplicated; empty when every live item's type is declared. Drives the non-silent retype
/// hint — pure compute, the consumer renders it.
pub fn undeclared_types(cfg: &PluginConfig, store: &Store) -> Result<Vec<String>> {
    let mut found = std::collections::BTreeSet::new();
    for item in store.list_items()? {
        if item.deleted.as_deref() == Some("1") {
            continue;
        }
        if let Some(ty) = &item.item_type {
            if !cfg.types.is_type(ty) {
                found.insert(ty.clone());
            }
        }
    }
    Ok(found.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_flow_core::model::EdgeKind;
    use nexus_flow_core::store::Store;

    // ---- next: finish-first tiers (docs/specs/next-finish-first-tiers.md §4) ----

    const T_NOW: &str = "2026-07-16T00:00:00Z";

    fn it_cfg() -> PluginConfig {
        crate::plugin::load("issue-tracker").unwrap()
    }

    /// Create an item with a type + priority (the two keys `issue-tracker`'s `next` policy ranks on
    /// after status), open by default.
    fn mk(s: &mut Store, id: &str, ty: &str, priority: &str) {
        s.create_item(id, ty, id, "x");
        s.set_field(id, "priority", Some(priority.into()), "x");
    }

    fn claim(s: &mut Store, id: &str) {
        s.set_field(id, "status", Some("in_progress".into()), "x");
    }

    fn next_ids(cfg: &PluginConfig, s: &Store, sort: Option<SortKey>) -> Vec<String> {
        next(cfg, s, T_NOW, sort)
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect()
    }

    #[test]
    fn next_tiers_finish_first_then_started_then_backlog() {
        // §2: the whole point — (1) closeable-now claimed work, (2) the started epic's cluster,
        // (3) the general backlog. Priorities are chosen so the FLAT plugin rank would invert this:
        // the Tier-3 item and the Tier-2 child are both P0, the Tier-1 leaf only P2.
        let mut s = Store::open_in_memory(1);
        mk(&mut s, "t.leaf", "bug", "2"); // Tier 1: claimed, childless
        claim(&mut s, "t.leaf");
        mk(&mut s, "t.epic", "epic", "1"); // Tier 2: started, one open child
        claim(&mut s, "t.epic");
        mk(&mut s, "t.child", "feature", "0");
        s.add_parent("t.child", "t.epic", "x");
        mk(&mut s, "t.backlog", "bug", "0"); // Tier 3: highest priority, not started

        assert_eq!(
            next_ids(&it_cfg(), &s, None),
            ["t.leaf", "t.epic", "t.child", "t.backlog"],
            "finishing beats starting: the P0 backlog bug sinks below the started work"
        );
    }

    #[test]
    fn next_tier1_surfaces_an_all_done_epic_that_used_to_vanish() {
        // §1 root cause 2 (the `a0gw` bug): an in-progress epic whose children are ALL closed was
        // absent from plain `next` (in-progress was opt-in), so it never got closed. Now it leads.
        let mut s = Store::open_in_memory(1);
        mk(&mut s, "t.a0gw", "epic", "1");
        claim(&mut s, "t.a0gw");
        for c in ["t.c1", "t.c2"] {
            mk(&mut s, c, "chore", "2");
            s.add_parent(c, "t.a0gw", "x");
            s.set_field(c, "status", Some("closed".into()), "x");
        }
        mk(&mut s, "t.other", "bug", "0"); // higher priority, but only Tier 3

        assert_eq!(
            next_ids(&it_cfg(), &s, None),
            ["t.a0gw", "t.other"],
            "the closeable epic surfaces first instead of vanishing"
        );
    }

    #[test]
    fn next_tier2_cluster_is_header_then_children_by_priority() {
        // §2 worked example (board 41j0): the started epic `cd6k` heads its own cluster and its four
        // open children follow it in plugin-rank order — instead of scattering across the backlog
        // behind an unrelated, higher-priority ready item.
        let mut s = Store::open_in_memory(1);
        mk(&mut s, "t.cd6k", "epic", "2");
        claim(&mut s, "t.cd6k");
        for (c, p) in [
            ("t.9z2b", "3"),
            ("t.nwh3", "1"),
            ("t.sbek", "2"),
            ("t.zj10", "3"),
        ] {
            mk(&mut s, c, "feature", p);
            s.add_parent(c, "t.cd6k", "x");
        }
        mk(&mut s, "t.p0", "bug", "0"); // unrelated P0: outranked everything before

        assert_eq!(
            next_ids(&it_cfg(), &s, None),
            ["t.cd6k", "t.nwh3", "t.sbek", "t.9z2b", "t.zj10", "t.p0"],
            "header, then children P1→P2→P3 (id breaking the P3 tie), then the backlog"
        );
    }

    #[test]
    fn next_orders_clusters_by_their_parents_rank() {
        // §4.3: clusters are ordered by the PARENT's rank (not by the children's) — a lower-priority
        // epic's cluster never interleaves with a higher-priority epic's.
        let mut s = Store::open_in_memory(1);
        // E1 (P1) has only a P3 child; E2 (P2) has a P0 child. Flat rank would put E2's P0 child on
        // top; clustered, E1's whole group comes first.
        mk(&mut s, "t.e1", "epic", "1");
        claim(&mut s, "t.e1");
        mk(&mut s, "t.e1c", "feature", "3");
        s.add_parent("t.e1c", "t.e1", "x");
        mk(&mut s, "t.e2", "epic", "2");
        claim(&mut s, "t.e2");
        mk(&mut s, "t.e2c", "feature", "0");
        s.add_parent("t.e2c", "t.e2", "x");

        assert_eq!(
            next_ids(&it_cfg(), &s, None),
            ["t.e1", "t.e1c", "t.e2", "t.e2c"],
            "each cluster stays contiguous, ordered by its parent's rank"
        );
    }

    #[test]
    fn next_puts_an_in_progress_child_in_tier_1_above_its_own_parent() {
        // §7 edge case, end-to-end through the composed order (the core pins the signals; this pins
        // what `next` actually emits): a CLAIMED child is not "open", so it is no Tier-2 cluster row
        // — it is a Tier-1 candidate in its own right, and therefore sorts ABOVE the very parent
        // whose cluster it belongs to. That reads right: finish the child, and the parent becomes
        // finishable next. The parent stays a lone Tier-2 header meanwhile (its child is live).
        let mut s = Store::open_in_memory(1);
        mk(&mut s, "t.epic", "epic", "0"); // P0 — outranks the child on the flat rank
        claim(&mut s, "t.epic");
        mk(&mut s, "t.child", "feature", "3"); // P3, claimed → Tier 1 despite the lower priority
        s.add_parent("t.child", "t.epic", "x");
        claim(&mut s, "t.child");

        assert_eq!(
            next_ids(&it_cfg(), &s, None),
            ["t.child", "t.epic"],
            "the claimed child is Tier 1, ahead of its own Tier-2 header parent"
        );
    }

    #[test]
    fn next_shows_a_lone_header_when_no_child_is_actionable() {
        // §2/§7: a started epic whose only open child is blocked appears as a lone header — visible
        // (so it can be finished/re-planned), without "do this now" child noise.
        let mut s = Store::open_in_memory(1);
        mk(&mut s, "t.epic", "epic", "1");
        claim(&mut s, "t.epic");
        mk(&mut s, "t.child", "feature", "0");
        s.add_parent("t.child", "t.epic", "x");
        mk(&mut s, "t.blocker", "chore", "3");
        s.add_edge("t.child", "t.blocker", EdgeKind::Dep, "x"); // child blocked by its own dep

        assert_eq!(
            next_ids(&it_cfg(), &s, None),
            ["t.epic", "t.blocker"],
            "the epic is a lone Tier-2 header; its blocked child is not a row"
        );
    }

    #[test]
    fn next_keeps_a_blocked_parents_child_out_of_every_tier() {
        // 07a invariant #07a.2 regression (§0/§3.3): a blocked (open) parent suppresses its whole
        // subtree into the `blocked` lane. Tiering is selection+ordering ONLY — it must never
        // resurrect a suppressed child.
        let mut s = Store::open_in_memory(1);
        mk(&mut s, "t.parent", "epic", "0");
        mk(&mut s, "t.blocker", "chore", "3");
        s.add_edge("t.parent", "t.blocker", EdgeKind::Dep, "x"); // parent blocked
        mk(&mut s, "t.child", "feature", "0");
        s.add_parent("t.child", "t.parent", "x");

        assert_eq!(
            next_ids(&it_cfg(), &s, None),
            ["t.blocker"],
            "neither the blocked parent nor its suppressed child appears in any tier"
        );
    }

    #[test]
    fn next_includes_started_work_by_default() {
        // §5: in-progress is no longer opt-in — the flag is gone and claimed work is always there.
        let mut s = Store::open_in_memory(1);
        mk(&mut s, "t.open", "bug", "0");
        mk(&mut s, "t.claimed", "bug", "2");
        claim(&mut s, "t.claimed");

        assert_eq!(
            next_ids(&it_cfg(), &s, None),
            ["t.claimed", "t.open"],
            "claimed work is present by default (Tier 1), ahead of the ready backlog"
        );
    }

    #[test]
    fn next_sort_id_bypasses_tiering() {
        // §4.2: an explicit non-rank `--sort` is the escape hatch — flat order, no tiers.
        let mut s = Store::open_in_memory(1);
        mk(&mut s, "t.b_epic", "epic", "1");
        claim(&mut s, "t.b_epic");
        mk(&mut s, "t.c_child", "feature", "3");
        s.add_parent("t.c_child", "t.b_epic", "x");
        mk(&mut s, "t.a_backlog", "bug", "0");

        assert_eq!(
            next_ids(&it_cfg(), &s, Some(SortKey::Id)),
            ["t.a_backlog", "t.b_epic", "t.c_child"],
            "--sort id gives the flat id order, tiering bypassed"
        );
    }

    #[test]
    fn next_tiering_is_deterministic() {
        // The composed (tier, cluster, rank) order is total — repeated calls agree.
        let mut s = Store::open_in_memory(1);
        mk(&mut s, "t.epic", "epic", "1");
        claim(&mut s, "t.epic");
        for c in ["t.x", "t.y", "t.z"] {
            mk(&mut s, c, "feature", "2");
            s.add_parent(c, "t.epic", "x");
        }
        let cfg = it_cfg();
        let first = next_ids(&cfg, &s, None);
        assert_eq!(first, next_ids(&cfg, &s, None));
        assert_eq!(first, ["t.epic", "t.x", "t.y", "t.z"]);
    }

    #[test]
    fn undeclared_types_lists_legacy_enum_values_not_in_the_active_plugin() {
        // #yod: a pre-plugin workspace holds the old core-enum types; only those the active plugin
        // does not declare are flagged (sorted, deduplicated). Declared types and deleted items are
        // not flagged.
        let mut s = Store::open_in_memory(1);
        s.create_item("a", "project", "A", "x"); // legacy, undeclared
        s.create_item("b", "task", "B", "x"); // legacy, undeclared
        s.create_item("c", "epic", "C", "x"); // declared by issue-tracker
        let cfg = crate::plugin::load("issue-tracker").unwrap();
        assert_eq!(
            undeclared_types(&cfg, &s).unwrap(),
            vec!["project".to_string(), "task".to_string()]
        );
        s.delete_item("a", "x");
        assert_eq!(
            undeclared_types(&cfg, &s).unwrap(),
            vec!["task".to_string()],
            "a deleted legacy item is not flagged — nothing to repair"
        );
    }

    #[test]
    fn list_excludes_archived_by_default_but_show_still_returns_it() {
        // C1 (#916.1): archived items drop out of the enumerating `list`, but an explicit `show`
        // lookup still resolves them (carrying the marker) — unlike `deleted`, which 404s.
        let mut s = Store::open_in_memory(1);
        s.create_item("c1.A", "task", "A", "x");
        s.create_item("c1.B", "task", "B", "x");
        s.set_field("c1.B", "archived", Some("2026-06-23T00:00:00Z".into()), "x");

        let cfg = crate::plugin::load("issue-tracker").unwrap();
        let ids: Vec<String> = list(&cfg, &s, None, None, None)
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(
            ids,
            vec!["c1.A".to_string()],
            "the archived item is excluded from list by default"
        );

        let rec = show(&s, "c1.B").expect("show resolves an archived item (no not_found)");
        assert_eq!(
            rec.item.archived.as_deref(),
            Some("2026-06-23T00:00:00Z"),
            "show carries the archived marker timestamp"
        );

        // A deleted item is still not_found via show — `archived` is a softer tombstone.
        s.delete_item("c1.A", "x");
        assert!(
            show(&s, "c1.A").is_err(),
            "a deleted item is still not_found"
        );
    }

    /// PR #363 review, Test Quality #1 and #2: the cut mechanism itself, at every boundary. It
    /// was covered only indirectly before — through `prime`'s truncation and the CLI's `--limit`
    /// integration tests — and the one boundary that decides whether a view DISCLOSES anything,
    /// `limit == total`, was never hit exactly by either.
    #[test]
    fn truncate_next_cuts_to_the_limit_and_always_reports_the_untruncated_total() {
        let five = || {
            (0..5)
                .map(|n| item(&format!("a{n}"), None))
                .collect::<Vec<_>>()
        };

        // No limit is the default: everything, and nothing to disclose.
        let all = truncate_next(five(), None);
        assert_eq!((all.items.len(), all.total), (5, 5));
        assert!(!all.truncated());

        // THE boundary: a limit at exactly the length is not a cut. `truncated()` is
        // `len < total`, so an off-by-one here would head an intact list with "showing 5 of 5".
        let exact = truncate_next(five(), Some(5));
        assert_eq!((exact.items.len(), exact.total), (5, 5));
        assert!(!exact.truncated(), "limit == total is not a truncation");
        assert!(
            !truncate_next(five(), Some(50)).truncated(),
            "nor is a wider one"
        );

        // One below: the first real cut, taking the HEAD of the given order — the caller has
        // already ranked and filtered, so the cut must not reorder.
        let cut = truncate_next(five(), Some(4));
        assert_eq!((cut.items.len(), cut.total), (4, 5));
        assert!(cut.truncated());
        assert_eq!(cut.items[0].id, "a0", "order preserved");

        // Zero is an honest count-only answer, not an empty board: no rows, but the total says
        // there are five, so the view still discloses rather than reading as "nothing to do".
        let zero = truncate_next(five(), Some(0));
        assert!(zero.items.is_empty());
        assert_eq!(zero.total, 5);
        assert!(zero.truncated());

        // An empty set cannot be made to look cut by any limit.
        assert!(!truncate_next(Vec::new(), Some(3)).truncated());
    }

    fn item(id: &str, priority: Option<&str>) -> ItemRow {
        ItemRow {
            id: id.into(),
            item_type: Some("task".into()),
            title: None,
            completion_criterion: None,
            description: None,
            design: None,
            status: Some("open".into()),
            priority: priority.map(str::to_string),
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
    fn priority_ordinal_ranks_numerically_not_lexicographically() {
        // Priority is stored as its canonical ordinal (8qv.2), so a plugin with a two-digit
        // range must not sort "10" before "2" — the numeric path keeps variant rank order.
        let prio = RankKey {
            field: "priority".into(),
            dir: Dir::Asc,
            nulls: Nulls::Last,
            precedence: Vec::new(),
        };
        assert_eq!(
            cmp_rank_key(&item("a", Some("2")), &item("b", Some("10")), &prio),
            std::cmp::Ordering::Less,
            "ordinal 2 ranks before 10 (numeric, not lexicographic)"
        );
        assert_eq!(
            cmp_rank_key(&item("a", Some("0")), &item("b", Some("3")), &prio),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn cmp_rank_key_keeps_non_numeric_fields_lexicographic() {
        // ids and ISO dates must still compare as strings (lexicographic == chronological for
        // dates), so the numeric fast-path must not disturb them.
        let id = RankKey {
            field: "id".into(),
            dir: Dir::Asc,
            nulls: Nulls::Last,
            precedence: Vec::new(),
        };
        assert_eq!(
            cmp_rank_key(&item("ab12.0002", None), &item("ab12.0010", None), &id),
            "ab12.0002".cmp("ab12.0010"),
            "ids keep lexicographic order"
        );
    }

    #[test]
    fn enum_index_key_orders_by_precedence_not_bytes() {
        // sp6.5: a `precedence` key sorts by the value's position in the list, not lexicographically.
        // For `type`, `epic` (index 0) must rank before `bug` (index 1) even though "bug" < "epic".
        let typed = |id: &str, ty: &str| {
            let mut i = item(id, Some("1"));
            i.item_type = Some(ty.into());
            i
        };
        let key = RankKey {
            field: "type".into(),
            dir: Dir::Asc,
            nulls: Nulls::Last,
            precedence: ["epic", "bug", "decision", "feature", "chore"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        assert_eq!(
            cmp_rank_key(&typed("a", "epic"), &typed("b", "bug"), &key),
            std::cmp::Ordering::Less,
            "epic (index 0) ranks before bug (index 1), not by bytes"
        );
        assert_eq!(
            cmp_rank_key(&typed("a", "decision"), &typed("b", "chore"), &key),
            std::cmp::Ordering::Less,
            "decision (index 2) ranks before chore (index 4)"
        );
        // A value absent from the list sorts after every listed one.
        assert_eq!(
            cmp_rank_key(&typed("a", "chore"), &typed("b", "mystery"), &key),
            std::cmp::Ordering::Less,
            "an unlisted type sorts last"
        );
    }

    #[test]
    fn status_precedence_ranks_in_progress_before_open() {
        // The status tier is now a declarative precedence key (sp6.5), not hardcoded: in_progress
        // (index 0) outranks open (index 1) purely from the config.
        let status = |id: &str, st: &str| {
            let mut i = item(id, Some("1"));
            i.status = Some(st.into());
            i
        };
        let key = RankKey {
            field: "status".into(),
            dir: Dir::Asc,
            nulls: Nulls::Last,
            precedence: vec!["in_progress".to_string(), "open".to_string()],
        };
        assert_eq!(
            cmp_rank_key(&status("a", "in_progress"), &status("b", "open"), &key),
            std::cmp::Ordering::Less,
            "in_progress ranks ahead of open"
        );
    }

    #[test]
    fn effective_lane_ready_masks_an_in_progress_child_of_an_open_parent() {
        // 07a.3 ready-mask (§1): an in_progress child under an open (non-gating, ready) parent has an
        // EFFECTIVE lane of `Ready` — surfacing it as in_progress would over-state progress at the
        // not-yet-started container. Judgment call (07a.3 note): effective-lane only — the child's
        // STORED status stays in_progress, so the `next` split-on-raw-status contract is untouched.
        // A claimed item WITHOUT an open parent shows its own in_progress lane.
        let mut s = Store::open_in_memory(1);
        s.create_item("p", "task", "open parent", "t"); // open, no deps → ready
        s.create_item("c", "task", "claimed child", "t");
        s.add_parent("c", "p", "t");
        s.set_field("c", "status", Some("in_progress".into()), "t");
        s.create_item("solo", "task", "claimed solo", "t"); // in_progress, no parent
        s.set_field("solo", "status", Some("in_progress".into()), "t");

        let sets = EffectiveLaneSets::compute(&s, "2026-06-17T00:00:00Z").unwrap();
        let child = s.get_item("c").unwrap().unwrap();
        assert_eq!(
            effective_lane(&s, &child, &sets).unwrap(),
            Lane::Ready,
            "ready-masked by the open parent"
        );
        assert_eq!(
            child.status.as_deref(),
            Some("in_progress"),
            "the stored status is untouched — only the effective lane shifts"
        );
        let solo = s.get_item("solo").unwrap().unwrap();
        assert_eq!(
            effective_lane(&s, &solo, &sets).unwrap(),
            Lane::InProgress,
            "no open parent → own in_progress lane"
        );
    }

    #[test]
    fn effective_lane_closed_mask_outranks_suppression_through_a_closed_parent() {
        // §2: the closed-mask (every live parent closed) takes precedence over ancestor suppression.
        // C's only parent P is closed → C is closed-masked, even though a blocked grandparent G gates
        // the chain (C would otherwise be suppressed-by-blocked). All-parents-closed wins → `Closed`.
        let mut s = Store::open_in_memory(1);
        s.create_item("g", "task", "blocked grandparent", "t");
        s.create_item("x", "task", "G's open blocker", "t");
        s.add_edge("g", "x", nexus_flow_core::model::EdgeKind::Dep, "t"); // G blocked
        s.create_item("p", "task", "parent", "t");
        s.add_parent("p", "g", "t");
        s.create_item("c", "task", "child", "t");
        s.add_parent("c", "p", "t");
        s.set_field("p", "status", Some("closed".into()), "t"); // C's only parent is closed

        let sets = EffectiveLaneSets::compute(&s, "2026-06-17T00:00:00Z").unwrap();
        let c = s.get_item("c").unwrap().unwrap();
        assert_eq!(
            effective_lane(&s, &c, &sets).unwrap(),
            Lane::Closed,
            "all-parents-closed masks C as closed, ahead of the blocked-grandparent suppression"
        );
    }

    #[test]
    fn prime_purpose_uses_the_plugin_tagline_when_present() {
        let cfg = crate::plugin::load("issue-tracker").unwrap();
        let p = prime_purpose(&cfg);
        assert!(
            p.starts_with("nexus-flow is a software issue tracker for epics and issues."),
            "{p}"
        );
    }

    #[test]
    fn prime_tagline_matches_the_predicate_inside_prime_purpose() {
        // nxf xe2z: `prime_tagline` is the SAME predicate `prime_purpose` wraps as "nexus-flow is
        // …" for `--json` — split out so `nxf prime`'s human view (CLI-side) can wrap it with a
        // different subject (`` `nxf` is … ``) without duplicating the tagline-or-default lookup.
        let cfg = crate::plugin::load("issue-tracker").unwrap();
        let tagline = prime_tagline(&cfg);
        assert_eq!(tagline, "a software issue tracker for epics and issues");
        assert_eq!(
            prime_purpose(&cfg),
            format!("nexus-flow is {tagline}. {PRIME_DERIVATION}"),
            "prime_purpose is exactly prime_tagline wrapped, nothing more"
        );
    }

    #[test]
    fn prime_tagline_falls_back_to_the_core_default_without_a_tagline() {
        let mut cfg = crate::plugin::load("issue-tracker").unwrap();
        cfg.tagline = None;
        assert_eq!(prime_tagline(&cfg), DEFAULT_TAGLINE);
    }

    #[test]
    fn prime_purpose_falls_back_to_core_default_without_a_tagline() {
        // nexus-flow-6pu: a plugin (or bare engine) with no tagline must never render a
        // dangling "nexus-flow is ." — the core default fills the predicate.
        let mut cfg = crate::plugin::load("issue-tracker").unwrap();
        cfg.tagline = None;
        let p = prime_purpose(&cfg);
        assert!(
            p.starts_with("nexus-flow is a tracker for projects and tasks."),
            "core default predicate: {p}"
        );
        assert!(!p.contains("nexus-flow is ."), "no dangling predicate: {p}");
    }

    #[test]
    fn schema_report_exposes_hierarchy_for_the_bundled_plugins() {
        // issue-tracker: `epic` is the container (non-epics parent under it, single), depth 2.
        let it = crate::plugin::load("issue-tracker").unwrap();
        let r = schema(&it);
        let epic = r.hierarchy.types.iter().find(|t| t.name == "epic").unwrap();
        assert!(epic.container, "epic is a container");
        assert!(epic.parents.is_empty(), "epic is a root");
        assert_eq!(epic.cardinality, None, "a root has no parent rule");
        let bug = r.hierarchy.types.iter().find(|t| t.name == "bug").unwrap();
        assert!(!bug.container, "bug is a leaf");
        assert_eq!(bug.parents, vec!["epic".to_string()]);
        assert_eq!(bug.cardinality, Some(crate::matrix::Cardinality::Single));
        assert_eq!(r.hierarchy.max_depth, Some(2));

        // personal-todo: `project` is the container; `todo` has `many` cardinality.
        let td = crate::plugin::load("personal-todo").unwrap();
        let r = schema(&td);
        assert!(
            r.hierarchy
                .types
                .iter()
                .find(|t| t.name == "project")
                .unwrap()
                .container
        );
        let todo = r.hierarchy.types.iter().find(|t| t.name == "todo").unwrap();
        assert_eq!(todo.cardinality, Some(crate::matrix::Cardinality::Many));
    }

    #[test]
    fn schema_to_value_carries_legacy_keys_and_the_additive_hierarchy() {
        let it = crate::plugin::load("issue-tracker").unwrap();
        let v = schema(&it).to_value();
        // Legacy keys unchanged (a flat {type: label} map, keyed {ordinal: label} priorities).
        assert_eq!(v["plugin"], serde_json::json!("issue-tracker"));
        assert_eq!(v["types"]["epic"], serde_json::json!("epic"));
        assert_eq!(v["priorities"]["0"], serde_json::json!("P0"));
        assert_eq!(
            v["statuses"]["in_progress"],
            serde_json::json!("in progress")
        );
        assert_eq!(
            v["dependencies"]["create_flag"],
            serde_json::json!("--depends-on")
        );
        let dod = v["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["field"] == serde_json::json!("completion_criterion"))
            .unwrap();
        assert_eq!(dod["create_flag"], serde_json::json!("--dod"));
        // Additive hierarchy block.
        assert_eq!(v["hierarchy"]["max_depth"], serde_json::json!(2));
        assert_eq!(
            v["hierarchy"]["types"]["epic"]["container"],
            serde_json::json!(true)
        );
        assert_eq!(
            v["hierarchy"]["types"]["bug"]["container"],
            serde_json::json!(false)
        );
        assert_eq!(
            v["hierarchy"]["types"]["bug"]["parents"],
            serde_json::json!(["epic"])
        );
        assert_eq!(
            v["hierarchy"]["types"]["bug"]["cardinality"],
            serde_json::json!("single")
        );
        assert_eq!(
            v["hierarchy"]["types"]["epic"]["cardinality"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn schema_exposes_a_parsed_container_fixture() {
        // A consumer-shaped plugin: a `context` container (max_depth 2), non-container types parented
        // under it with single cardinality. Proves container exposure independent of the bundled set.
        let cfg: PluginConfig = toml::from_str(
            r#"
            name = "notes-fixture"
            [description]
            en = "x"
            de = "y"
            [priority]
            labels = ["P1"]
            [types]
            list = ["context", "project", "note", "person", "goal"]
            max_depth = 2
            [types.rules.note]
            parents = ["context"]
            max = 1
            [types.rules.person]
            parents = ["context"]
            max = 1
            [types.rules.goal]
            parents = ["context"]
            max = 1
            [vocabulary.status]
            open = "open"
            in_progress = "in progress"
            closed = "closed"
            [ranking.next]
            order = [ { field = "id", dir = "asc" } ]
            [presentation.list]
            columns = ["id"]
        "#,
        )
        .unwrap();
        let v = schema(&cfg).to_value();
        assert_eq!(
            v["hierarchy"]["types"]["context"]["container"],
            serde_json::json!(true)
        );
        assert_eq!(
            v["hierarchy"]["types"]["note"]["container"],
            serde_json::json!(false)
        );
        assert_eq!(
            v["hierarchy"]["types"]["note"]["parents"],
            serde_json::json!(["context"])
        );
        assert_eq!(
            v["hierarchy"]["types"]["note"]["cardinality"],
            serde_json::json!("single")
        );
        // `project` here is declared but no type parents under it → not a container.
        assert_eq!(
            v["hierarchy"]["types"]["project"]["container"],
            serde_json::json!(false)
        );
    }

    #[test]
    fn field_schema_covers_every_settable_field() {
        // Drift guard (moved from the CLI): `schema` derives `settable` from UPDATABLE_FIELDS, so every
        // settable field MUST appear in the report — else the schema silently omits a field update accepts.
        let r = schema(&crate::plugin::load("issue-tracker").unwrap());
        for f in crate::write::UPDATABLE_FIELDS {
            assert!(
                r.fields.iter().any(|s| &s.field == f && s.settable),
                "settable field '{f}' is missing/not-settable in the schema report"
            );
        }
    }
}
