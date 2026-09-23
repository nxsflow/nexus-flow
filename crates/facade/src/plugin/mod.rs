//! The declarative plugin seam. A `PluginConfig` is the single source of every
//! opinionated decision — vocabulary, priority range, `next` ordering, presentation —
//! over the meaning-free core. Two real configs ship embedded in the binary;
//! `personal-todo` exists so the seam is proven against an actual artifact, not just a
//! test fixture. No command code may hardcode any of this; it all flows from here.

use crate::error::{NxfError, Result};
use crate::matrix::RelationshipMatrix;
use serde::Deserialize;
use std::collections::BTreeMap;

pub mod registry;
pub use registry::PluginRegistration;

/// The registered plugin names in canonical order — the registry-driven replacement for the old
/// `PLUGIN_NAMES` const (r16h.1). A function, not a const, because a consumer plugin is collected at
/// runtime via `inventory`. The `init` chooser and `mcp install` read it, so a linked consumer
/// plugin appears without any OSS edit.
pub fn plugin_names() -> Vec<&'static str> {
    registry::registered().iter().map(|r| r.name).collect()
}

/// Deprecated: the OSS-bundled plugin names as a const. Superseded by [`plugin_names`], which is
/// registry-driven and therefore ALSO includes a consumer's own registered plugin (r16h.1) — this
/// const can only ever name the two shipped in this binary. Kept for one minor as the E5 §5
/// coexistence window for a stable facade item (rather than an outright removal); migrate pins to
/// `plugin_names()`.
#[deprecated(
    since = "0.16.0",
    note = "use plugin_names() — registry-driven, includes consumer-registered plugins"
)]
pub const PLUGIN_NAMES: &[&str] = &["issue-tracker", "personal-todo"];

/// Sort direction for a ranking key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    #[default]
    Asc,
    Desc,
}

/// Where nulls sort relative to values for a ranking key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Nulls {
    First,
    #[default]
    Last,
}

/// How the fold reconciles concurrent writes to a field (w213 — the per-field merge seam of the
/// note-body decision, 13uo). Declared per field in the plugin TOML `[merge]` table; *selecting*
/// the strategy is vocabulary here, the fold *behaviour* lives in the core reducer. `Lww` (the
/// default) is last-writer-wins on the op's Lamport clock — every bundled ticket field. `CrdtText`
/// is reserved for the collaborative note body: it folds LWW until the text-CRDT reducer lands
/// (4b39), so declaring it today is safe and lossless. `OrSet` is an observed-remove set (realized
/// per field by its own view, e.g. labels/h89s). Ticket fields stay `Lww` — no CRDT on ticket
/// fields (13uo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum MergeStrategy {
    #[default]
    Lww,
    CrdtText,
    OrSet,
}

/// The declared data type of a plugin custom field (plugin-custom-fields §3, TB-CF-3). Governs
/// value validation and presentation, never storage (a value is always a `TEXT` LWW cell). The v1
/// set: `text`/`longtext` are free UTF-8; `date` is ISO-8601 (the same check `due`/`defer` use);
/// `enum` is one of a declared `values` list. Richer types (`int`/`bool`/format-validated
/// uri/email/color) are additive follow-ups (§9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    Text,
    Longtext,
    Date,
    Enum,
}

impl FieldType {
    /// The declared type's wire spelling — the `kind` a custom field reports in `nxf schema --json`
    /// (plugin-custom-fields §6). The inverse of the `#[serde(rename_all = "lowercase")]` parse.
    pub fn as_str(&self) -> &'static str {
        match self {
            FieldType::Text => "text",
            FieldType::Longtext => "longtext",
            FieldType::Date => "date",
            FieldType::Enum => "enum",
        }
    }
}

/// One plugin custom field declaration — a `[fields.<name>]` sub-table (plugin-custom-fields §3).
/// The `<name>` key is the field name (validated by [`PluginConfig::validate_fields`]); this struct
/// carries its attributes. Modelled on the `[types.rules.<t>]` shape, all attributes but `type`
/// default so a minimal declaration is `type = "…"`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldDecl {
    /// The field's data type. `type` is a Rust keyword, so the TOML key `type` maps to `field_type`.
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// The enum members — required (non-empty) iff [`field_type`](Self::field_type) is
    /// [`FieldType::Enum`], forbidden otherwise (validated at load).
    #[serde(default)]
    pub values: Vec<String>,
    /// The item types this field applies to (their names in `[types].list`). Empty ⇒ the field is
    /// global (applies to every type) — TB-CF-2.
    #[serde(default)]
    pub on: Vec<String>,
    /// Whether the field must be set when an item of an applicable type is created (create-time
    /// guard only — TB-CF-7). Defaults to `false`.
    #[serde(default)]
    pub required: bool,
    /// The display label. `None` ⇒ the presentation layer falls back to the titleised field name
    /// (computed downstream, not here).
    #[serde(default)]
    pub label: Option<String>,
    /// How the fold reconciles concurrent writes to this field (w213). Only [`MergeStrategy::Lww`]
    /// (the default) and [`MergeStrategy::CrdtText`] are valid for a custom field in v1;
    /// [`MergeStrategy::OrSet`] is rejected at load (TB-CF-4).
    #[serde(default)]
    pub merge: MergeStrategy,
}

/// The canonical item field names + aliases a custom field may not shadow (plugin-custom-fields §3,
/// TB-CF-6). A declared custom field whose name is in this set is a loud load error — it would
/// otherwise silently shadow a canonical field on the `--set`/`custom` surfaces.
const CANONICAL_FIELD_NAMES: &[&str] = &[
    "id",
    "title",
    "description",
    "status",
    "priority",
    "due",
    "defer",
    "defer_until",
    "assignee",
    "type",
    "parent",
    "belongs_to",
    "design",
    "dod",
    "completion_criterion",
    "closing_comment",
    "deleted",
    "archived",
    "closed_at",
    "labels",
    "notes",
];

/// True iff `name` matches `^[a-z][a-z0-9_]*$` — a safe `--set` key and JSON key. Checked manually
/// (no regex dependency): non-empty, a lowercase-letter first char, then lowercase letters, digits,
/// or underscores.
fn is_valid_field_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    if !matches!(bytes.next(), Some(b'a'..=b'z')) {
        return false;
    }
    bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// One ordering key in the `next` ranking policy. Two modes: a FIELD key (`dir`/`nulls` over the
/// stored value — numeric-aware for the priority ordinal, lexicographic else) and, when
/// `precedence` is non-empty, an ENUM-INDEX key (sp6.5): items sort by the position of their
/// `field` value in `precedence`, e.g. type `epic` before `bug` or status `in_progress` before
/// `open`. The list order IS the sort order, so a precedence key ignores `dir`/`nulls`; a value
/// absent from the list sorts after every listed one (the deterministic `id` tiebreak then settles
/// it). This is how a plugin expresses type/status precedence without any hardcoding in the engine.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RankKey {
    pub field: String,
    #[serde(default)]
    pub dir: Dir,
    #[serde(default)]
    pub nulls: Nulls,
    #[serde(default)]
    pub precedence: Vec<String>,
}

impl RankKey {
    /// The value's sort ordinal for an enum-index (`precedence`) key: its position in the list, or
    /// `precedence.len()` when absent/none (so unlisted values sort after every listed one). Only
    /// meaningful when [`precedence`](Self::precedence) is non-empty.
    pub fn precedence_ordinal(&self, value: Option<&str>) -> usize {
        value
            .and_then(|v| self.precedence.iter().position(|p| p == v))
            .unwrap_or(self.precedence.len())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RankSpec {
    pub order: Vec<RankKey>,
}

impl RankSpec {
    /// Reject a `precedence` (enum-index) key that ALSO sets a non-default `dir`/`nulls` (Code
    /// review #2). A precedence key orders strictly by the value's position in its list and ignores
    /// `dir`/`nulls`, so setting them is a silent no-op that signals a misunderstanding — caught
    /// loudly at load, like the matrix's cardinality-`0` rejection, rather than quietly dropped.
    pub fn validate(&self) -> std::result::Result<(), String> {
        for key in &self.order {
            if !key.precedence.is_empty() && (key.dir != Dir::Asc || key.nulls != Nulls::Last) {
                return Err(format!(
                    "ranking key '{}' sets a precedence list together with dir/nulls; a precedence \
                     key orders by the list's index and ignores dir/nulls — drop the dir/nulls",
                    key.field
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Ranking {
    pub next: RankSpec,
}

/// Priority as named variants (8qv.2): `labels` is the ordered set of variant names — index 0
/// is the highest priority — and is the single source of both the valid set and the ranking
/// order. The agent sets, reads, and sees the label itself (e.g. `P1`, `now`); there is no
/// integer indirection. Analogous to `vocabulary.status`, but ordered.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Priority {
    pub labels: Vec<String>,
}

impl Priority {
    /// The variant's rank ordinal (its index in `labels`), or `None` if `label` is not a known
    /// variant. Lower ordinal = higher priority. This is both the validity check (`Some` ⇒ valid)
    /// and the canonical value stored on the item; `next` ranking sorts on it, never on bytes.
    pub fn ordinal(&self, label: &str) -> Option<usize> {
        self.labels.iter().position(|l| l == label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Vocabulary {
    pub status: BTreeMap<String, String>,
    /// Optional, sparse DISPLAY labels for the plugin's item types (`type → label`), keyed by the
    /// canonical type declared in `[types]`. A missing key falls back to the type name itself, so a
    /// plugin only overrides the types it wants to render differently. Both bundled plugins omit it
    /// — their type names (`epic`/`bug`, `project`/`todo`) already are their agent-facing labels.
    /// The authoritative valid type SET is `[types].list` (sp6.4), not this map.
    #[serde(default)]
    pub types: BTreeMap<String, String>,
    /// Display labels for the item's fields — the headings `show` prints (8qv.8) and the
    /// names `schema` reports, in the plugin's own language (e.g. issue-tracker
    /// `completion_criterion = "Definition of Done"`, personal-todo `description = "Why"`).
    /// Optional and sparse: a missing key falls back to a core default, so a plugin only
    /// overrides the labels it wants to rename.
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ListView {
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Presentation {
    pub list: ListView,
}

/// A one-line, human-facing description of a plugin in both shipped languages. It is the
/// single source the `init` chooser (and `guide plugins`, sibling PR) reads, so an agent
/// or operator can make a conscious choice instead of accepting a silent default.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginDescription {
    pub en: String,
    pub de: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginConfig {
    pub name: String,
    pub description: PluginDescription,
    /// The predicate `prime` renders after "nexus-flow is …" — the agent-facing answer to
    /// *what kind of tracker is this* (e.g. "a software issue tracker for epics and issues").
    /// Distinct from `description` (the operator chooser one-liner). Absent ⇒ `prime` falls back
    /// to a core default, so it never renders a dangling "nexus-flow is ." (nexus-flow-6pu).
    /// Today only the two in-house plugins set it; later it becomes a consumer-overridable knob.
    #[serde(default)]
    pub tagline: Option<String>,
    pub priority: Priority,
    /// The plugin's type system (sp6.4, TOML `[types]`): the valid type list, the allowed
    /// parent→child pairs, the parent cardinality per type, and the hierarchy depth limit. The
    /// write seam enforces it (see [`crate::matrix`]). A config that omits `[types]` falls back to
    /// the pre-`[types]` default (parent must be `project`, single-parent, unlimited depth).
    #[serde(default)]
    pub types: RelationshipMatrix,
    pub vocabulary: Vocabulary,
    pub ranking: Ranking,
    pub presentation: Presentation,
    /// Per-field merge strategy declarations (w213), `field name → strategy`. Optional and sparse:
    /// an unlisted field defaults to [`MergeStrategy::Lww`], so both bundled plugins omit the table
    /// and keep byte-identical LWW folding. A plugin (e.g. the notes plugin, r16h) declares only
    /// the fields whose fold differs — `description = "crdt-text"` for a collaborative body.
    #[serde(default)]
    pub merge: BTreeMap<String, MergeStrategy>,
    /// Plugin custom field declarations (plugin-custom-fields §3), `field name → declaration`.
    /// Optional and sparse: a plugin adds its own item fields on top of the 16 canonical ones —
    /// a `file`'s `uri`, a `person`'s `email` — each with a data type, `on` scope, merge policy,
    /// and `required`/`label`. Both bundled plugins omit the table (empty map, no behaviour change).
    /// Validated fail-closed at load by [`PluginConfig::validate_fields`].
    #[serde(default)]
    pub fields: BTreeMap<String, FieldDecl>,
}

impl PluginConfig {
    /// The declared merge strategy for `field`, or [`MergeStrategy::Lww`] when the plugin's
    /// `[merge]` table does not mention it (the default the whole schema is built around).
    pub fn merge_strategy(&self, field: &str) -> MergeStrategy {
        self.merge.get(field).copied().unwrap_or_default()
    }

    /// Validate the `[merge]` table (w213): an `lww`/`crdt-text` field must name a real folded item
    /// field (the core LWW whitelist), else a typo silently no-ops at fold time. An `or-set` field
    /// is view-backed (e.g. labels/h89s), deliberately absent from that whitelist, so it is exempt.
    /// Returns the offending field name on failure (the caller wraps it in a plugin-named error).
    fn validate_merge(&self) -> std::result::Result<(), String> {
        for (field, strat) in &self.merge {
            if !matches!(strat, MergeStrategy::OrSet)
                && !nexus_flow_core::schema::ITEM_LWW_FIELDS.contains(&field.as_str())
            {
                return Err(format!("'{field}' is not a known item field"));
            }
        }
        Ok(())
    }

    /// Validate the `[fields]` table (plugin-custom-fields §3) — fail-closed at load, the same loud
    /// posture as [`validate_merge`](Self::validate_merge)/`[types]`/`[ranking]`. Each rule returns
    /// the offending field name + reason (the caller wraps it in a plugin-named error):
    ///
    /// - **name collision** — must not shadow a canonical field or alias ([`CANONICAL_FIELD_NAMES`]);
    /// - **charset** — must match `^[a-z][a-z0-9_]*$` (a safe `--set`/JSON key);
    /// - **`on` types** — every type in `on` must be declared in `[types].list`;
    /// - **enum** — an `enum` needs a non-empty `values` with no empty/duplicate member; a non-enum
    ///   must not carry `values`;
    /// - **merge** — only `lww`/`crdt-text`; `or-set` is a v1 non-goal (TB-CF-4).
    fn validate_fields(&self) -> std::result::Result<(), String> {
        for (name, decl) in &self.fields {
            if CANONICAL_FIELD_NAMES.contains(&name.as_str()) {
                return Err(format!(
                    "custom field '{name}' collides with a canonical field or alias — pick a name \
                     outside the canonical set"
                ));
            }
            if !is_valid_field_name(name) {
                return Err(format!(
                    "custom field name '{name}' is invalid — a field name must match \
                     ^[a-z][a-z0-9_]*$ (a lowercase letter first, then lowercase letters, digits, \
                     or underscores)"
                ));
            }
            for ty in &decl.on {
                if !self.types.list.contains(ty) {
                    return Err(format!(
                        "custom field '{name}' lists an unknown type '{ty}' in `on` — add it to \
                         [types].list"
                    ));
                }
            }
            match decl.field_type {
                FieldType::Enum if decl.values.is_empty() => {
                    return Err(format!(
                        "custom field '{name}' is an enum but declares no `values` — an enum field \
                         needs a non-empty `values` list"
                    ));
                }
                FieldType::Enum => {
                    // An empty enum member would collide with the "empty value = clear/unset"
                    // convention (§4.2) once the write/read paths land, so reject it at load. A
                    // duplicate member is a declaration bug — a value can only rank/validate once.
                    let mut seen = std::collections::BTreeSet::new();
                    for v in &decl.values {
                        if v.is_empty() {
                            return Err(format!(
                                "custom field '{name}' has an empty enum member in `values` — an \
                                 empty value is reserved for clearing a field and cannot be an \
                                 enum member"
                            ));
                        }
                        if !seen.insert(v.as_str()) {
                            return Err(format!(
                                "custom field '{name}' lists a duplicate enum member '{v}' in \
                                 `values`"
                            ));
                        }
                    }
                }
                _ if !decl.values.is_empty() => {
                    return Err(format!(
                        "custom field '{name}' declares `values` but is not an enum — `values` is \
                         only valid for an `enum` field"
                    ));
                }
                _ => {}
            }
            if matches!(decl.merge, MergeStrategy::OrSet) {
                return Err(format!(
                    "custom field '{name}' declares merge = \"or-set\" — or-set is not supported \
                     for custom fields in v1 (only lww or crdt-text); a set-valued custom field \
                     needs its own view + fold"
                ));
            }
        }
        Ok(())
    }
}

/// Load a registered plugin config by name. An unknown name is a loud validation error naming the
/// registered set (registry-driven, r16h.1) — never a silently-wrong vocabulary.
pub fn load(name: &str) -> Result<PluginConfig> {
    let reg = registry::find(name)
        // A duplicate registration is ambiguous (link-order-dependent) — a loud error, not a
        // silent first-wins pick.
        .map_err(NxfError::validation)?
        .ok_or_else(|| {
            NxfError::validation(format!(
                "unknown plugin '{name}'; registered: {}. It may be a consumer plugin not linked \
                 into this binary — check the build, or fix .nxs/config.toml.",
                plugin_names().join(", ")
            ))
        })?;
    load_registration(reg)
}

/// Parse + validate one registration's TOML. The descriptor `name` is the lookup key, so the TOML's
/// own `name` must agree — a mismatch is a registration bug, caught here. Pure over its input, so the
/// name-agreement + validation gates are unit-testable without `inventory`.
fn load_registration(reg: &PluginRegistration) -> Result<PluginConfig> {
    let cfg: PluginConfig = toml::from_str(reg.toml)
        .map_err(|e| NxfError::io(format!("parsing plugin '{}': {e}", reg.name)))?;
    // The descriptor name is the lookup key; a TOML whose own `name` disagrees would silently load
    // the wrong vocabulary, so it is a loud registration bug.
    if cfg.name != reg.name {
        return Err(NxfError::validation(format!(
            "plugin registration name '{}' disagrees with its TOML name '{}'",
            reg.name, cfg.name
        )));
    }
    // The declared type system must be internally consistent — every parent→child rule may only
    // reference types in `[types].list`. A dangling reference is a loud validation error, not a
    // silently unreachable rule.
    cfg.types.validate().map_err(|e| {
        NxfError::validation(format!("plugin '{}' has an invalid [types]: {e}", reg.name))
    })?;
    cfg.ranking.next.validate().map_err(|e| {
        NxfError::validation(format!(
            "plugin '{}' has an invalid [ranking.next]: {e}",
            reg.name
        ))
    })?;
    // w213: a mis-declared `[merge]` field is caught at load (a typo would otherwise silently
    // no-op at fold time), the same loud posture as the `[types]`/`[ranking]` checks above. Now on
    // the registration path, so a consumer/external plugin gets the same [merge] validation.
    cfg.validate_merge().map_err(|e| {
        NxfError::validation(format!("plugin '{}' has an invalid [merge]: {e}", reg.name))
    })?;
    // plugin-custom-fields §3: the `[fields]` declaration table is validated fail-closed at load —
    // collision/charset/`on`-type/enum/merge — the same loud posture as `[types]`/`[ranking]`/
    // `[merge]` above. Also on the registration path, so a consumer/external plugin gets it too.
    cfg.validate_fields().map_err(|e| {
        NxfError::validation(format!(
            "plugin '{}' has an invalid [fields]: {e}",
            reg.name
        ))
    })?;
    Ok(cfg)
}

/// Every registered plugin's config, in `plugin_names()` order. The init chooser and the non-TTY
/// validation error both render from this, so the options + their descriptions are always exactly
/// what is registered.
///
/// Fallible (review): once a plugin can be registered from a CONSUMER crate (r16h.1), a single bad
/// third-party TOML (or a duplicate-name collision) is a real user condition — it must surface as a
/// loud error, not panic `nxf init`/`mcp install` for everyone including the bundled plugins.
pub fn available() -> Result<Vec<PluginConfig>> {
    plugin_names().iter().map(|name| load(name)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_tracker_config_parses_with_expected_schema() {
        let c = load("issue-tracker").unwrap();
        assert_eq!(c.name, "issue-tracker");
        // Both descriptions are present and non-empty (the chooser/guide read them).
        assert!(!c.description.en.is_empty());
        assert!(!c.description.de.is_empty());
        // prime's plugin-determined predicate (nexus-flow-6pu).
        assert_eq!(
            c.tagline.as_deref(),
            Some("a software issue tracker for epics and issues")
        );
        assert_eq!(c.priority.labels, ["P0", "P1", "P2", "P3", "P4"]);
        // The ordered labels are both the valid set and the ranking order (8qv.2): a known label
        // resolves to its ordinal, an unknown one to None (which is how validation rejects it).
        assert_eq!(c.priority.ordinal("P0"), Some(0));
        assert_eq!(c.priority.ordinal("P4"), Some(4));
        assert_eq!(c.priority.ordinal("nope"), None);
        assert_eq!(c.priority.ordinal("5"), None);
        assert_eq!(c.vocabulary.status["in_progress"], "in progress");
        // sp6.4: the type set + rules are declared in `[types]`, not the old 2-role `vocabulary.types`.
        assert!(c.vocabulary.types.is_empty(), "no display-label overrides");
        assert_eq!(
            c.types.list,
            ["bug", "chore", "decision", "epic", "feature"]
                .into_iter()
                .map(String::from)
                .collect()
        );
        assert_eq!(c.types.max_depth, Some(2));
        // only `epic` is a container: every non-epic may have a single `epic` parent; epic has no rule.
        assert!(c.types.pair_allowed("epic", "bug"));
        assert!(
            !c.types.pair_allowed("bug", "feature"),
            "non-epic is not a container"
        );
        assert_eq!(
            c.types.cardinality("bug"),
            Some(crate::matrix::Cardinality::Single)
        );
        assert_eq!(
            c.types.cardinality("epic"),
            None,
            "an epic cannot be parented"
        );
        // ranking (sp6.5): status → priority → type → due → id, with status/type as enum-index
        // precedence keys (the list order IS the order) and priority/due as field keys.
        let order = &c.ranking.next.order;
        assert_eq!(order.len(), 5);
        assert_eq!(order[0].field, "status");
        assert_eq!(order[0].precedence, ["in_progress", "open"]);
        assert_eq!(order[1].field, "priority");
        assert_eq!(order[1].dir, Dir::Asc);
        assert_eq!(order[2].field, "type");
        assert_eq!(
            order[2].precedence,
            ["epic", "bug", "decision", "feature", "chore"]
        );
        assert_eq!(order[3].field, "due");
        assert_eq!(order[3].nulls, Nulls::Last);
        assert_eq!(order[4].field, "id");
        assert_eq!(
            c.presentation.list.columns,
            ["id", "priority", "status", "type", "title"]
        );
        // Field labels the `show` headings render (8qv.8).
        assert_eq!(
            c.vocabulary.fields["completion_criterion"],
            "Definition of Done"
        );
        assert_eq!(c.vocabulary.fields["description"], "Description");
    }

    #[test]
    fn personal_todo_config_is_a_vocabulary_remap() {
        let c = load("personal-todo").unwrap();
        assert_eq!(c.name, "personal-todo");
        assert!(!c.description.en.is_empty());
        assert!(!c.description.de.is_empty());
        // distinct vocabulary AND a distinct hierarchy shape from issue-tracker.
        assert_eq!(
            c.tagline.as_deref(),
            Some("a personal to-do list for projects and todos")
        );
        assert_eq!(c.vocabulary.status["open"], "todo");
        assert_eq!(c.priority.labels[0], "now");
        assert_eq!(c.priority.labels.last().unwrap(), "icebox");
        // ranking is field-based with NO type precedence; the leading status precedence preserves
        // the in_progress-first behaviour (sp6.5). Order: status → priority → id.
        let order = &c.ranking.next.order;
        assert_eq!(order.len(), 3);
        assert_eq!(order[0].field, "status");
        assert_eq!(order[0].precedence, ["in_progress", "open"]);
        assert_eq!(order[1].field, "priority");
        assert_eq!(order[2].field, "id");
        assert!(
            order.iter().all(|k| k.field != "type"),
            "personal-todo does not rank by type"
        );
        // sp6.4 seam proof: its parent cardinality is `Many` (a todo may belong to any number of
        // projects) — the behavioural difference from issue-tracker's single parent.
        assert_eq!(
            c.types.list,
            ["project", "termin", "todo"]
                .into_iter()
                .map(String::from)
                .collect()
        );
        assert_eq!(
            c.types.cardinality("todo"),
            Some(crate::matrix::Cardinality::Many)
        );
        assert_eq!(
            c.types.cardinality("termin"),
            Some(crate::matrix::Cardinality::Many)
        );
        assert!(c.types.pair_allowed("project", "todo"));
        assert_eq!(
            c.types.cardinality("project"),
            None,
            "a project cannot be parented"
        );
    }

    #[test]
    fn unknown_plugin_is_validation_error() {
        let err = load("nope").unwrap_err();
        assert_eq!(err.kind, crate::error::ErrorKind::Validation);
    }

    #[test]
    fn unknown_plugin_error_lists_the_registered_names() {
        let err = load("nope").unwrap_err();
        assert_eq!(err.kind, crate::error::ErrorKind::Validation);
        // Registry-driven wording (distinct from the old "available:" message).
        assert!(
            err.msg.contains("registered:"),
            "registry wording: {}",
            err.msg
        );
        assert!(
            err.msg.contains("not linked"),
            "consumer-plugin hint: {}",
            err.msg
        );
        assert!(
            err.msg.contains("issue-tracker") && err.msg.contains("personal-todo"),
            "names the registered set: {}",
            err.msg
        );
    }

    #[test]
    fn load_registration_rejects_a_descriptor_toml_name_mismatch() {
        // A registration whose descriptor name disagrees with its TOML `name` is a loud bug — the
        // descriptor name is the lookup key, so a mismatch would silently load the wrong vocabulary.
        let reg = registry::PluginRegistration {
            name: "claims-alpha",
            toml: "name = \"actually-beta\"\n[description]\nen=\"x\"\nde=\"y\"\n[priority]\n\
                   labels=[\"P1\"]\n[vocabulary.status]\nopen=\"open\"\nin_progress=\"in progress\"\n\
                   closed=\"closed\"\n[ranking.next]\norder=[{ field=\"id\", dir=\"asc\" }]\n\
                   [presentation.list]\ncolumns=[\"id\"]\n",
            order: 0,
        };
        let err = load_registration(&reg).unwrap_err();
        assert_eq!(err.kind, crate::error::ErrorKind::Validation);
        assert!(
            err.msg.contains("claims-alpha") && err.msg.contains("actually-beta"),
            "names both sides of the mismatch: {}",
            err.msg
        );
    }

    #[test]
    fn load_registration_rejects_unparseable_toml() {
        // The load path a consumer plugin travels must fail closed on garbage TOML — a loud error
        // naming the plugin, never a panic (review Test Quality #3).
        let reg = registry::PluginRegistration {
            name: "broken",
            toml: "this is [not valid toml",
            order: 0,
        };
        let err = load_registration(&reg).unwrap_err();
        assert_eq!(err.kind, crate::error::ErrorKind::Io);
        assert!(err.msg.contains("broken"), "names the plugin: {}", err.msg);
    }

    #[test]
    fn plugin_names_lists_registered_plugins_in_order() {
        assert_eq!(plugin_names(), vec!["issue-tracker", "personal-todo"]);
    }

    #[test]
    fn available_lists_every_plugin_with_descriptions() {
        let plugins = available().unwrap();
        let names: Vec<&str> = plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, plugin_names());
        for p in &plugins {
            assert!(!p.description.en.is_empty(), "{} has EN desc", p.name);
            assert!(!p.description.de.is_empty(), "{} has DE desc", p.name);
        }
    }

    #[test]
    fn ranking_rejects_a_precedence_key_with_dir_or_nulls() {
        // Code review #2: a precedence key orders by list index and ignores dir/nulls, so combining
        // them is a loud load-time rejection instead of a silently dropped dir.
        let prec = |dir, nulls| RankSpec {
            order: vec![RankKey {
                field: "type".into(),
                dir,
                nulls,
                precedence: vec!["epic".into(), "bug".into()],
            }],
        };
        assert!(
            prec(Dir::Desc, Nulls::Last).validate().is_err(),
            "non-default dir is rejected"
        );
        assert!(
            prec(Dir::Asc, Nulls::First).validate().is_err(),
            "non-default nulls is rejected"
        );
        // The form both bundled plugins use — precedence with default dir/nulls — is accepted.
        assert!(prec(Dir::Asc, Nulls::Last).validate().is_ok());
        // A plain field key may set dir/nulls freely (no precedence list).
        let field = RankSpec {
            order: vec![RankKey {
                field: "due".into(),
                dir: Dir::Desc,
                nulls: Nulls::First,
                precedence: Vec::new(),
            }],
        };
        assert!(field.validate().is_ok());
    }

    #[test]
    fn merge_strategy_defaults_to_lww_for_bundled_plugins() {
        // w213: no bundled plugin declares `[merge]`, so every field folds LWW exactly as before —
        // the seam adds vocabulary, not a behaviour change for issue-tracker/personal-todo.
        let it = load("issue-tracker").unwrap();
        assert_eq!(it.merge_strategy("description"), MergeStrategy::Lww);
        assert_eq!(it.merge_strategy("title"), MergeStrategy::Lww);
        assert_eq!(it.merge_strategy("anything-unknown"), MergeStrategy::Lww);
        let td = load("personal-todo").unwrap();
        assert_eq!(td.merge_strategy("description"), MergeStrategy::Lww);
    }

    #[test]
    fn merge_strategy_is_declarable_per_field() {
        // The r16h seam: a plugin declares a per-field merge strategy (notes body = crdt-text,
        // a labels-style field = or-set). Undeclared fields still default to LWW.
        let cfg: PluginConfig = toml::from_str(
            r#"
            name = "notes-fixture"
            [description]
            en = "x"
            de = "y"
            [priority]
            labels = ["P1"]
            [vocabulary.status]
            open = "open"
            in_progress = "in progress"
            closed = "closed"
            [ranking.next]
            order = [ { field = "id", dir = "asc" } ]
            [presentation.list]
            columns = ["id"]
            [merge]
            description = "crdt-text"
            labels = "or-set"
            title = "lww"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.merge_strategy("description"), MergeStrategy::CrdtText);
        assert_eq!(cfg.merge_strategy("labels"), MergeStrategy::OrSet);
        assert_eq!(cfg.merge_strategy("title"), MergeStrategy::Lww);
        assert_eq!(cfg.merge_strategy("undeclared"), MergeStrategy::Lww);
    }

    /// A minimal parseable [`PluginConfig`] carrying just the given `[merge]` body — the fixture the
    /// `validate_merge` tests vary.
    fn cfg_with_merge(merge_body: &str) -> PluginConfig {
        toml::from_str(&format!(
            r#"
            name = "merge-fixture"
            [description]
            en = "x"
            de = "y"
            [priority]
            labels = ["P1"]
            [vocabulary.status]
            open = "open"
            in_progress = "in progress"
            closed = "closed"
            [ranking.next]
            order = [ {{ field = "id", dir = "asc" }} ]
            [presentation.list]
            columns = ["id"]
            [merge]
            {merge_body}
            "#
        ))
        .unwrap()
    }

    #[test]
    fn validate_merge_rejects_an_unknown_lww_or_crdt_text_field() {
        // w213 review: a typo'd item field with an lww/crdt-text strategy would silently no-op at
        // fold time (the fold defaults to LWW for a field it never matches) — validate_merge rejects
        // it at load, the same loud posture as [types]/[ranking].
        assert!(cfg_with_merge(r#"descriptoin = "crdt-text""#)
            .validate_merge()
            .is_err());
        assert!(cfg_with_merge(r#"nope = "lww""#).validate_merge().is_err());
        // A real item field is accepted.
        assert!(cfg_with_merge(r#"description = "crdt-text""#)
            .validate_merge()
            .is_ok());
        // The bundled plugins declare no [merge], so they validate clean through `load`.
        assert!(load("issue-tracker").unwrap().validate_merge().is_ok());
    }

    #[test]
    fn validate_merge_exempts_an_or_set_field_from_the_item_whitelist() {
        // An or-set field is view-backed (labels/h89s), deliberately not a folded item field, so it
        // is not checked against the LWW whitelist — a plugin may name any field for `or-set`.
        assert!(cfg_with_merge(r#"labels = "or-set""#)
            .validate_merge()
            .is_ok());
    }

    #[test]
    fn facade_merge_vocabulary_matches_the_core_fold_names() {
        // The facade `[merge]` vocabulary and the core fold enum are deliberately SEPARATE types —
        // the facade adds `or-set` (a view strategy the core does not fold, model.rs:§w213). But the
        // strategy rides an op's `op_type` across the crate boundary, so their SHARED variants must
        // agree on the wire name — a rename in one enum that silently drifts from the other would
        // break the op-carried-strategy round-trip. Pin it here rather than merge the two enums.
        use nexus_flow_core::model::MergeStrategy as Core;
        for (core, expect) in [
            (Core::Lww, MergeStrategy::Lww),
            (Core::CrdtText, MergeStrategy::CrdtText),
        ] {
            let parsed =
                cfg_with_merge(&format!(r#"title = "{}""#, core.as_str())).merge_strategy("title");
            assert_eq!(
                parsed,
                expect,
                "core fold name '{}' must deserialize to the matching facade variant",
                core.as_str()
            );
        }
    }

    #[test]
    fn merge_strategy_rejects_unknown_value() {
        // An unrecognised strategy is a loud parse error, not a silent fallback (like the rest of
        // the plugin schema, a typo fails at load rather than degrading behaviour).
        let err = toml::from_str::<PluginConfig>(
            r#"
            name = "bad"
            [description]
            en = "x"
            de = "y"
            [priority]
            labels = ["P1"]
            [vocabulary.status]
            open = "open"
            in_progress = "in progress"
            closed = "closed"
            [ranking.next]
            order = [ { field = "id", dir = "asc" } ]
            [presentation.list]
            columns = ["id"]
            [merge]
            description = "three-way"
            "#,
        );
        assert!(err.is_err(), "unknown merge strategy must fail to parse");
    }

    /// A minimal parseable [`PluginConfig`] whose `[types].list` is `file`/`project`/`person` and
    /// whose `[fields]` body is `fields_body` — the fixture the `validate_fields` tests vary. The
    /// `on`-validation tests rely on the three declared types being present.
    fn cfg_with_fields(fields_body: &str) -> PluginConfig {
        toml::from_str(&format!(
            r#"
            name = "fields-fixture"
            [description]
            en = "x"
            de = "y"
            [priority]
            labels = ["P1"]
            [types]
            list = ["file", "project", "person"]
            [vocabulary.status]
            open = "open"
            in_progress = "in progress"
            closed = "closed"
            [ranking.next]
            order = [ {{ field = "id", dir = "asc" }} ]
            [presentation.list]
            columns = ["id"]

            {fields_body}
            "#
        ))
        .unwrap()
    }

    #[test]
    fn fields_block_parses_and_round_trips_every_attribute() {
        // Happy path (§3): a valid `[fields]` block parses, validates, and every attribute
        // round-trips — type, values, on, required, label, merge, including the defaults.
        let cfg = cfg_with_fields(
            r#"
            [fields.uri]
            type = "text"
            on = ["file"]
            required = true
            label = "File URI"
            merge = "lww"

            [fields.stage]
            type = "enum"
            values = ["backlog", "active", "done"]
            on = ["project"]
            merge = "crdt-text"
            "#,
        );
        assert!(cfg.validate_fields().is_ok());

        let uri = &cfg.fields["uri"];
        assert_eq!(uri.field_type, FieldType::Text);
        assert!(uri.values.is_empty(), "a text field has no enum values");
        assert_eq!(uri.on, ["file"]);
        assert!(uri.required);
        assert_eq!(uri.label.as_deref(), Some("File URI"));
        assert_eq!(uri.merge, MergeStrategy::Lww);

        let stage = &cfg.fields["stage"];
        assert_eq!(stage.field_type, FieldType::Enum);
        assert_eq!(stage.values, ["backlog", "active", "done"]);
        assert_eq!(stage.on, ["project"]);
        assert!(!stage.required, "required defaults to false");
        assert_eq!(stage.label, None, "label defaults to None");
        assert_eq!(stage.merge, MergeStrategy::CrdtText);
    }

    #[test]
    fn validate_fields_rejects_a_canonical_name_collision() {
        // §3 / TB-CF-6: a custom field must not shadow a canonical field or alias — a loud load
        // error, never a silently-shadowing declaration.
        assert!(
            cfg_with_fields("[fields.status]\ntype = \"text\"")
                .validate_fields()
                .is_err(),
            "'status' collides with a canonical field"
        );
        assert!(
            cfg_with_fields("[fields.due]\ntype = \"date\"")
                .validate_fields()
                .is_err(),
            "'due' collides with a canonical field"
        );
        // A non-canonical name in the same shape is accepted.
        assert!(cfg_with_fields("[fields.uri]\ntype = \"text\"")
            .validate_fields()
            .is_ok());
    }

    #[test]
    fn validate_fields_rejects_an_invalid_charset() {
        // §3: the name must match ^[a-z][a-z0-9_]*$ (a safe `--set` key and JSON key).
        assert!(
            cfg_with_fields("[fields.Uri]\ntype = \"text\"")
                .validate_fields()
                .is_err(),
            "uppercase first char is rejected"
        );
        assert!(
            cfg_with_fields("[fields.1x]\ntype = \"text\"")
                .validate_fields()
                .is_err(),
            "leading digit is rejected"
        );
        assert!(
            cfg_with_fields("[fields.a-b]\ntype = \"text\"")
                .validate_fields()
                .is_err(),
            "hyphen is rejected"
        );
        // A valid snake_case name with a trailing digit and underscore is accepted.
        assert!(cfg_with_fields("[fields.field_2]\ntype = \"text\"")
            .validate_fields()
            .is_ok());
    }

    #[test]
    fn validate_fields_rejects_an_unknown_on_type() {
        // §3: every type in `on` must be declared in [types].list.
        assert!(
            cfg_with_fields("[fields.uri]\ntype = \"text\"\non = [\"nope\"]")
                .validate_fields()
                .is_err(),
            "an undeclared `on` type is rejected"
        );
        // A declared type is accepted, and an empty/omitted `on` (global) is accepted.
        assert!(
            cfg_with_fields("[fields.uri]\ntype = \"text\"\non = [\"file\"]")
                .validate_fields()
                .is_ok()
        );
        assert!(cfg_with_fields("[fields.uri]\ntype = \"text\"")
            .validate_fields()
            .is_ok());
    }

    #[test]
    fn validate_fields_enforces_enum_values_coupling() {
        // §3: an enum needs a non-empty `values`; a non-enum must NOT carry `values`.
        assert!(
            cfg_with_fields("[fields.stage]\ntype = \"enum\"")
                .validate_fields()
                .is_err(),
            "an enum with no values is rejected"
        );
        assert!(
            cfg_with_fields("[fields.stage]\ntype = \"enum\"\nvalues = []")
                .validate_fields()
                .is_err(),
            "an enum with empty values is rejected"
        );
        assert!(
            cfg_with_fields("[fields.uri]\ntype = \"text\"\nvalues = [\"a\", \"b\"]")
                .validate_fields()
                .is_err(),
            "a non-enum with values is a declaration error"
        );
        assert!(
            cfg_with_fields("[fields.stage]\ntype = \"enum\"\nvalues = [\"a\", \"b\"]")
                .validate_fields()
                .is_ok(),
            "an enum with non-empty values is accepted"
        );
    }

    #[test]
    fn validate_fields_rejects_the_id_field() {
        // Review: `id` is canonical key #1 of the record — a `[fields.id]` declaration must be
        // rejected as a collision, exactly like `title`/`status`, even though it passes the charset.
        assert!(
            cfg_with_fields("[fields.id]\ntype = \"text\"")
                .validate_fields()
                .is_err(),
            "'id' collides with the canonical record key"
        );
    }

    #[test]
    fn validate_fields_rejects_empty_and_duplicate_enum_members() {
        // Review: an empty enum member would collide with the "empty value = clear/unset" convention
        // (§4.2) once the write/read paths land; a duplicate member is a declaration bug. Both fail
        // closed at load.
        assert!(
            cfg_with_fields("[fields.stage]\ntype = \"enum\"\nvalues = [\"\", \"a\"]")
                .validate_fields()
                .is_err(),
            "an empty enum member is rejected"
        );
        assert!(
            cfg_with_fields("[fields.stage]\ntype = \"enum\"\nvalues = [\"a\", \"a\"]")
                .validate_fields()
                .is_err(),
            "a duplicate enum member is rejected"
        );
    }

    #[test]
    fn field_decl_rejects_an_unknown_attribute() {
        // Review: `deny_unknown_fields` makes a typo'd attribute in a `[fields.<name>]` table fail
        // closed at parse (e.g. `requird` for `required`) instead of silently defaulting.
        assert!(
            toml::from_str::<FieldDecl>("type = \"text\"\nrequird = true").is_err(),
            "a typo'd attribute is a loud parse error, not a silent default"
        );
        assert!(
            toml::from_str::<FieldDecl>("type = \"text\"\nrequired = true").is_ok(),
            "the correctly-spelled attribute parses"
        );
    }

    #[test]
    fn validate_fields_rejects_or_set_merge() {
        // §3 / TB-CF-4: or-set custom fields need their own view+fold — a v1 non-goal, so declaring
        // one is a loud load error. lww and crdt-text (and the omitted default) are accepted.
        assert!(
            cfg_with_fields("[fields.uri]\ntype = \"text\"\nmerge = \"or-set\"")
                .validate_fields()
                .is_err(),
            "or-set is rejected for a custom field in v1"
        );
        assert!(
            cfg_with_fields("[fields.uri]\ntype = \"text\"\nmerge = \"crdt-text\"")
                .validate_fields()
                .is_ok()
        );
        assert!(cfg_with_fields("[fields.uri]\ntype = \"text\"")
            .validate_fields()
            .is_ok());
    }

    #[test]
    fn bundled_plugins_have_no_custom_fields_and_load_clean() {
        // Neither bundled plugin declares `[fields]`, so the map is empty and `load` (which runs
        // `validate_fields`) stays clean — the seam adds vocabulary, no behaviour change.
        for name in ["issue-tracker", "personal-todo"] {
            let cfg = load(name).unwrap();
            assert!(
                cfg.fields.is_empty(),
                "{name} declares no custom fields yet"
            );
            assert!(cfg.validate_fields().is_ok(), "{name} validates clean");
        }
    }
}
