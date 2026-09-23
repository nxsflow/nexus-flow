//! The write compute layer — the mutation half of the shared seam (E5 #9t7.5). Where
//! [`crate::read`] computes typed records over the meaning-free core, this applies the core ops
//! that mutate it: `create`/`update`/`claim`/`close`/`dep`/`note`/`mention`/`contributes`. It is
//! the one write surface the CLI (the agent seam), the MCP server, and an embedding app (the
//! Tauri host) all share — there is deliberately no second write path, so every seam emits
//! identical ops and the same derivation can never diverge (the App↔CLI parity differential,
//! #9t7.6, pins exactly that).
//!
//! Two values are **explicit parameters** of every write, never read from ambient process state:
//!
//! - `now` — the wall-clock timestamp stamped on the emitted ops (display-only; the caller
//!   chooses system clock vs. an override). A long-lived host must control its own clock rather
//!   than have the engine reach for `OffsetDateTime::now()` on each call.
//! - `actor` — the identity recorded on the ops (`op.author`, for history/attribution and E4
//!   sync). A host process serving many users must attribute each write, not write everything
//!   under one ambient `$USER` (E9 #zxj).
//!
//! Validation is loud and up front: every rejection carries a [`crate::error::ErrorKind`] from
//! the closed set, and anything invalid is caught *before* the first op is written, so a rejected
//! write leaves the store untouched. Dependency cycles are rejected at write time so the
//! derivation never has to cope with an invalid edge.

use crate::error::{NxfError, Result};
use crate::matrix::{MatrixViolation, ParentWriteMode, RelationshipMatrix};
use crate::plugin::{FieldType, MergeStrategy, PluginConfig};
use crate::validate;
use nexus_flow_core::id::ReplicaIds;
use nexus_flow_core::model::{
    EdgeKind, ItemRow, LinkRelation, LinkWeight, MergeStrategy as CoreMergeStrategy,
};
use nexus_flow_core::store::Store;
use std::collections::{HashMap, HashSet};

/// The relationship matrix the write seam enforces — the active plugin's declared type system
/// (sp6.4, TOML `[types]`): allowed parent→child pairs, parent cardinality per type, and the depth
/// limit. A config without a `[types]` table falls back to [`RelationshipMatrix::default`] (parent
/// must be `project`, single-parent, unlimited depth) — the pre-`[types]` behaviour.
fn active_matrix(cfg: &PluginConfig) -> RelationshipMatrix {
    cfg.types.clone()
}

/// The fields a `create` carries (8qv.3): `description` + `priority` are mandatory; the rest are
/// optional. `depends_on` is the repeatable dependency list (this item depends on each id). All
/// values are raw strings as the caller supplied them.
///
/// **`priority` accepts the plugin's LABEL or the canonical ordinal KEY (ee2h, additive over the
/// earlier label-only 3bw6 rule).** Pass a named variant (`"P0".."P4"` for issue-tracker,
/// `"now".."icebox"` for personal-todo) — the value `schema` advertises as
/// `priority.create_json_values` — OR the read-form ordinal that `show`/`list`/`next --json` EMIT
/// (`"1"`). Either way the write path stores the canonical ordinal, so machine output round-trips
/// as machine input without an ordinal→label translation at the caller's seam. A label match wins,
/// so a plugin whose labels are themselves integers keeps label semantics.
pub struct NewItem<'a> {
    pub description: &'a str,
    pub priority: &'a str,
    pub design: Option<&'a str>,
    pub dod: Option<&'a str>,
    pub due: Option<&'a str>,
    pub defer: Option<&'a str>,
    pub parent: Option<&'a str>,
    pub depends_on: &'a [String],
    /// Create-time plugin custom-field assignments (plugin-custom-fields §5), each a `name=value`
    /// string mirroring `--set name=value`. Canonical fields keep their dedicated fields above;
    /// `custom` carries ONLY the plugin's declared custom fields, validated against the resolved
    /// `type` (`on`-scope + declared data type) and enforcing `required` at create time. Empty for
    /// a plugin with no `[fields]` (both bundled plugins).
    pub custom: &'a [String],
}

/// Fields settable via `update`. `deleted`/`id` are excluded (internal). `type` IS settable (#yod /
/// #5qp.7 — the retype primitive), but it is NOT validated through the generic [`prepare_set`] path:
/// [`update`] intercepts it to resolve against the plugin's declared type set and re-validate the
/// relationship edges before appending the op. It is listed here so the CLI's `schema` introspection
/// reports it `settable` from the same single source the write path accepts — never claiming a field
/// settable that `update` rejects. The long-form `description`/`design`/`completion_criterion` are
/// ordinary LWW cells set through here — there is no separate body field (#25i).
pub const UPDATABLE_FIELDS: &[&str] = &[
    "type",
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
];

/// The optional single-value fields an `update` clears when handed an EMPTY value (`--set defer=`,
/// 6j6v.68ke): the LWW cell is set to `NULL`, dropping the item out of the deferred lane / clearing
/// the due date or assignee — no stale value, no `--unset` flag. Scoped deliberately: `due`/`defer`
/// are date cells the generic `iso_date` validator would otherwise reject an empty value for, and
/// `assignee` a free cell where empty must mean unassigned (not the empty string). Validated fields
/// (`status`/`priority`) and required cells stay out, so an empty value there still fails loudly; the
/// long-text cells already accept an empty value verbatim (stored as `""`) and need no clear path.
const CLEARABLE_ON_EMPTY: &[&str] = &["due", "defer_until", "assignee"];

// ---- shared helpers --------------------------------------------------------

/// Fetch a live (non-tombstoned) item or fail with `not_found`.
fn require_item(store: &Store, id: &str) -> Result<ItemRow> {
    store
        .get_item(id)?
        .filter(|i| i.deleted.as_deref() != Some("1"))
        .ok_or_else(|| NxfError::not_found(format!("no item '{id}'")))
}

/// Require that `id` names a live item, else a `not_found` naming the `role` it was given as
/// (`parent` / `dependency`). Shared by `create`'s reference checks.
fn require_live(store: &Store, id: &str, role: &str) -> Result<()> {
    if store
        .get_item(id)?
        .filter(|i| i.deleted.as_deref() != Some("1"))
        .is_none()
    {
        return Err(NxfError::not_found(format!("{role} '{id}' does not exist")));
    }
    Ok(())
}

/// The longest chain of present `parent` edges from `id` up to a root, counting `id` itself
/// (so a parentless item is depth 1). Used only when the matrix sets a depth limit.
fn depth_above(store: &Store, id: &str) -> Result<usize> {
    longest_chain(id, &mut HashMap::new(), &mut HashSet::new(), &|n| {
        Ok(store.parents_of_result(n)?)
    })
}

/// The height of `id`'s subtree over present `parent` edges, counting `id` itself (a childless item
/// is height 1). The downward mirror of [`depth_above`].
fn height_below(store: &Store, id: &str) -> Result<usize> {
    longest_chain(id, &mut HashMap::new(), &mut HashSet::new(), &|n| {
        Ok(store.children_of_result(n)?)
    })
}

/// Longest chain length from `id` following `next` (parents for depth, children for height),
/// counting `id` itself. Memoized over the shared DAG so a diamond — a node reachable by several
/// paths — folds once per NODE, not once per path: without the memo a wide merge-delivered parent
/// DAG is O(2^depth) (Integrity review #1), with it the walk is linear in the present edges. A cycle
/// is only reachable from merge-delivered data (the write-time parent-cycle check is sp6.8); the
/// `on_stack` set detects the back-edge and contributes 0 there, keeping the walk total and bounded.
/// On a genuine DAG (the only valid shape) the memo is exact; on already-invalid cyclic data it is a
/// bounded approximation the convergence-time `invariant` backstop still catches.
fn longest_chain(
    id: &str,
    memo: &mut HashMap<String, usize>,
    on_stack: &mut HashSet<String>,
    next: &dyn Fn(&str) -> Result<Vec<String>>,
) -> Result<usize> {
    if let Some(&h) = memo.get(id) {
        return Ok(h);
    }
    if !on_stack.insert(id.to_string()) {
        return Ok(0); // cycle — stop contributing
    }
    let mut tallest_child = 0;
    for n in next(id)? {
        tallest_child = tallest_child.max(longest_chain(&n, memo, on_stack, next)?);
    }
    let h = 1 + tallest_child;
    on_stack.remove(id);
    memo.insert(id.to_string(), h);
    Ok(h)
}

/// Validate a candidate `child → parent` edge against the relationship matrix at write time (sp6.6),
/// returning how it should land ([`ParentWriteMode`]). Replaces the hardcoded "parent must be a
/// project" rule: a missing/tombstoned parent is `not_found`; a self-parent, a disallowed
/// parent→child type pair, an exceeded cardinality, or an exceeded depth limit are each a loud
/// `validation` reject — before any write. The convergence-time backstop in `invariant` keeps the
/// purely *structural* checks (parent exists, not deleted); the *type/cardinality/depth* policy is
/// the plugin's, enforced only here.
fn check_parent(
    matrix: &RelationshipMatrix,
    store: &Store,
    child_id: &str,
    child_type: &str,
    parent_id: &str,
) -> Result<ParentWriteMode> {
    if parent_id == child_id {
        return Err(NxfError::validation(format!(
            "an item cannot be its own parent ('{child_id}')"
        )));
    }
    let parent = store
        .get_item(parent_id)?
        .filter(|i| i.deleted.as_deref() != Some("1"))
        .ok_or_else(|| NxfError::not_found(format!("parent '{parent_id}' does not exist")))?;
    // sp6.8 (②d): reject a parent CYCLE at the source — `child → parent` would close a loop iff
    // `parent` already reaches `child` over parent edges. A loud `cycle` envelope, analogous to the
    // dep-cycle guard, instead of a silently-accepted invalid edge the derivation would have to cope
    // with. (Structural — runs before the plugin's type/cardinality/depth policy below.)
    if reaches_via_parent(store, parent_id, child_id)? {
        return Err(NxfError::cycle(format!(
            "'{child_id}' → '{parent_id}' would create a parent cycle"
        )));
    }
    let parent_type = parent.item_type.as_deref().unwrap_or_default();
    // Existing parent count, excluding the candidate (re-adding the same parent is idempotent).
    let existing = store
        .parents_of_result(child_id)?
        .into_iter()
        .filter(|p| p != parent_id)
        .count();
    // Depth is only consulted when the matrix caps it — skip the walk otherwise.
    let depth = if matrix.max_depth.is_some() {
        depth_above(store, parent_id)? + height_below(store, child_id)?
    } else {
        0
    };
    matrix
        .validate_parent(parent_type, child_type, existing, depth)
        .map_err(|v| matrix_error(matrix, child_id, parent_id, parent_type, child_type, v))
}

/// Render a [`MatrixViolation`] as the shared, self-explaining `validation` envelope. Extracted from
/// [`check_parent`] so the retype primitive ([`revalidate_type_change`]) reports edge violations the
/// same way the create/re-parent paths do.
fn matrix_error(
    matrix: &RelationshipMatrix,
    child_id: &str,
    parent_id: &str,
    parent_type: &str,
    child_type: &str,
    v: MatrixViolation,
) -> NxfError {
    match v {
        MatrixViolation::Pair => {
            let allowed = matrix.allowed_parents(child_type);
            if allowed.is_empty() {
                // No parent is allowed for this child type. Distinguish the two ways that happens
                // (Integrity review #3) so the message self-explains instead of a bare "allowed
                // parent types: (none)": either the type is top-level by design, or — on
                // cross-plugin / merge-delivered data — it is not a type this plugin declares.
                let reason = if matrix.is_type(child_type) {
                    format!("a '{child_type}' is a top-level type and cannot have a parent")
                } else {
                    format!(
                        "'{child_type}' is not a type the active plugin declares, \
                         so it cannot be parented here"
                    )
                };
                NxfError::validation(format!(
                    "'{child_id}' cannot be parented under '{parent_id}': {reason}"
                ))
            } else {
                NxfError::validation(format!(
                    "a '{child_type}' may not have a '{parent_type}' parent; \
                     allowed parent types: {}",
                    allowed.join(", ")
                ))
            }
        }
        MatrixViolation::Cardinality => NxfError::validation(format!(
            "'{child_id}' already has the maximum {} parent(s) allowed for a '{child_type}'",
            matrix.parent_limit(child_type).unwrap_or(0)
        )),
        MatrixViolation::Depth => NxfError::validation(format!(
            "parenting '{child_id}' under '{parent_id}' would exceed the maximum hierarchy \
             depth of {}",
            matrix.max_depth.unwrap_or(0)
        )),
    }
}

/// Re-validate `id`'s relationship edges if its type becomes `new_type` (#yod / #5qp.7): a retype may
/// not introduce a NEW matrix violation among types the active plugin DECLARES. Both directions are
/// checked against the pure matrix with the NEW type supplied explicitly (the store still holds the
/// old type) — every parent edge must allow `new_type` under the parent's type (pair + cardinality),
/// and every child must still be allowed under `new_type`.
///
/// Two deliberate exclusions, both from the 5qp.7 spec:
/// - **Edges whose OTHER end carries an UNDECLARED (legacy/foreign) type are SKIPPED.** A fully
///   legacy `{project, task}` board is otherwise a chicken-and-egg deadlock — the child can't be
///   retyped while its parent is still `project`, and the parent can't be retyped while a child is
///   still `task`. Skipping the not-yet-migrated end lets the board migrate item by item, order-
///   independently; once both ends are declared the edge is validated normally.
/// - **Depth is NOT re-checked.** A retype leaves the tree shape unchanged, so it can never worsen
///   depth — and re-checking would wrongly block migrating a legacy hierarchy that already exceeds
///   the active plugin's limit.
///
/// Returns the first violation as a `validation` error; on success the caller appends the type op.
fn revalidate_type_change(
    matrix: &RelationshipMatrix,
    store: &Store,
    id: &str,
    new_type: &str,
) -> Result<()> {
    // Fallible edge reads (07a.7): retype revalidation runs on the long-lived server
    // (`flow_update type=…` / embed `Engine::update`), so the parent/child edge reads surface a db
    // error as `io` instead of `.expect()`-panicking — the same de-panic as the effective-lane joins.
    // id as a child of each of its parents.
    let parents = store.parents_of_result(id)?;
    for parent_id in &parents {
        let parent_type = store
            .get_item(parent_id)?
            .and_then(|p| p.item_type)
            .unwrap_or_default();
        // Skip an edge to a not-yet-migrated (undeclared) parent — see the doc comment.
        if !matrix.is_type(&parent_type) {
            continue;
        }
        let existing = parents.len().saturating_sub(1);
        matrix
            .validate_parent(&parent_type, new_type, existing, 0)
            .map_err(|v| matrix_error(matrix, id, parent_id, &parent_type, new_type, v))?;
    }
    // id as a parent of each of its children.
    for child_id in store.children_of_result(id)? {
        let child_type = store
            .get_item(&child_id)?
            .and_then(|c| c.item_type)
            .unwrap_or_default();
        // Skip an edge to a not-yet-migrated (undeclared) child — see the doc comment.
        if !matrix.is_type(&child_type) {
            continue;
        }
        let existing = store.parents_of_result(&child_id)?.len().saturating_sub(1);
        matrix
            .validate_parent(new_type, &child_type, existing, 0)
            .map_err(|v| matrix_error(matrix, &child_id, id, new_type, &child_type, v))?;
    }
    Ok(())
}

/// Write a validated parent edge in the mode the matrix chose: re-point (cardinality 1) or add
/// (cardinality n).
fn write_parent(
    store: &mut Store,
    mode: ParentWriteMode,
    child_id: &str,
    parent_id: &str,
    actor: &str,
) -> Result<()> {
    match mode {
        // `set_parent` reads the current parents to clear them first — fallible (#76u.14).
        ParentWriteMode::Replace => store.set_parent(child_id, parent_id, actor)?,
        ParentWriteMode::Add => store.add_parent(child_id, parent_id, actor),
    }
    Ok(())
}

/// Validate the caller-supplied wall-clock pin and stamp it onto the store for the writes that
/// follow. `now` is an explicit parameter at every write seam, so a malformed value fails loudly
/// here — matching the CLI's `--now`/`NXF_NOW` validation — instead of silently landing an
/// arbitrary string in `op.wall_clock` when an embedding host passes one in directly. Coupling
/// the check to the stamp makes it impossible to emit a local op under an unvalidated `now`.
fn stamp_now(store: &mut Store, now: &str) -> Result<()> {
    validate::iso_date(now)?;
    store.set_wall_clock(now);
    Ok(())
}

/// Read back a just-mutated item, or an `io` error if it vanished (a should-never-happen the
/// caller still surfaces as a handled error rather than panicking).
fn read_back(store: &Store, id: &str) -> Result<ItemRow> {
    store
        .get_item(id)?
        .ok_or_else(|| NxfError::io(format!("item '{id}' could not be read back")))
}

/// Resolve a caller-supplied `--type` to the canonical type string, bidirectionally with the plugin
/// seam: a type the active plugin DECLARES (a `[types]` list member, sp6.4) is accepted directly and
/// stored verbatim; a plugin display label (an optional `vocabulary.types` override) is
/// reverse-looked-up to its declared type. No vocabulary is hardcoded — it all comes from the config.
fn resolve_item_type(cfg: &PluginConfig, ty: &str) -> Result<String> {
    // A type the plugin DECLARES in its `[types]` list is the canonical value, stored verbatim. The
    // set is the plugin's, not a fixed core enum — this is the sp6.2/sp6.4 seam.
    if cfg.types.is_type(ty) {
        return Ok(ty.to_string());
    }
    // …or an optional display label (sparse `vocabulary.types` override), reverse-looked-up to its
    // declared type. Both bundled plugins omit it — their type names already are their labels.
    if let Some((core, _)) = cfg
        .vocabulary
        .types
        .iter()
        .find(|(_, label)| label.as_str() == ty)
    {
        if cfg.types.is_type(core) {
            return Ok(core.clone());
        }
    }
    // Neither a declared type nor a known label: a loud validation error listing the declared types
    // an agent can self-correct to.
    let mut accepted: Vec<&str> = cfg.types.list.iter().map(String::as_str).collect();
    accepted.sort_unstable();
    Err(NxfError::validation(format!(
        "unknown type '{ty}'; expected one of the active plugin's types ({})",
        accepted.join(", ")
    )))
}

/// Resolve a named priority variant to the canonical ordinal stored on the item (8qv.2): the
/// caller sets a label (`P1`), but the record keeps the plugin-independent ordinal (`"1"`) so the
/// `--json` contract stays byte-identical across plugins. An unknown label is a validation error
/// listing the accepted labels, so an agent self-corrects without a schema round-trip.
///
/// Resolve a caller-supplied priority to the canonical ordinal stored on the item, accepting
/// BOTH forms (ee2h): the named label (`P1`, the plugin's display vocabulary) and the canonical
/// ordinal key (`1`, the exact form `show/list/next --json` emit). Accepting the ordinal key
/// repairs the round-trip — machine output was previously rejected as machine input (3bw6 was
/// deliberately label-only) — while staying additive: a label match takes precedence, so the only
/// new inputs accepted are plain in-range integers that are not themselves labels. The stored value
/// is always the ordinal, so the `--json` contract and `next` ranking are unchanged.
fn resolve_priority(cfg: &PluginConfig, value: &str) -> Result<String> {
    // A named label (`P1`) → its ordinal. The primary vocabulary; wins over the ordinal-key path
    // below so a plugin whose labels are themselves integers keeps label semantics.
    if let Some(i) = cfg.priority.ordinal(value) {
        return Ok(i.to_string());
    }
    // …or the canonical ordinal key itself (`1`), as long as it names a variant in range.
    if let Ok(i) = value.parse::<usize>() {
        if i < cfg.priority.labels.len() {
            return Ok(i.to_string());
        }
    }
    Err(NxfError::validation(format!(
        "unknown priority '{value}'; expected a label ({}) or its ordinal key (0..{})",
        cfg.priority.labels.join(", "),
        cfg.priority.labels.len().saturating_sub(1)
    )))
}

/// Canonicalize a user-facing field alias to its core column name. The aliases mirror the
/// `create` flags so the same word works on both verbs (8qv.4): `parent` → `belongs_to`,
/// `defer` → `defer_until`.
fn normalize_field(field: &str) -> &str {
    match field {
        "defer" => "defer_until",
        "parent" => "belongs_to",
        other => other,
    }
}

/// Whitelist + per-field validation for one `update` assignment, returning the CANONICAL value to
/// store. Most fields store their value verbatim; `priority` resolves the named variant to its
/// ordinal (8qv.2); `status` is checked against the plugin's vocabulary; dates are validated.
fn prepare_set(cfg: &PluginConfig, field: &str, value: &str) -> Result<String> {
    if !UPDATABLE_FIELDS.contains(&field) {
        // A declared custom field is intercepted by the caller BEFORE this generic whitelist, so
        // reaching here means the name is neither canonical nor custom — but list the plugin's custom
        // fields too, so an agent that mistyped (or reached for a custom field) sees both options.
        let custom = custom_field_names(cfg);
        let custom_hint = if custom.is_empty() {
            String::new()
        } else {
            format!("; custom fields: {custom:?}")
        };
        return Err(NxfError::validation(format!(
            "field '{field}' is not updatable; settable fields: {UPDATABLE_FIELDS:?} \
             (aliases: parent→belongs_to, defer→defer_until){custom_hint}"
        )));
    }
    match field {
        "status" => {
            if !cfg.vocabulary.status.contains_key(value) {
                return Err(NxfError::validation(format!(
                    "unknown status '{value}'; expected one of {:?}",
                    cfg.vocabulary.status.keys().collect::<Vec<_>>()
                )));
            }
            Ok(value.to_string())
        }
        "priority" => resolve_priority(cfg, value),
        "due" | "defer_until" => {
            validate::iso_date(value)?;
            Ok(value.to_string())
        }
        // `type` is settable, but only via the retype path in `update` (resolve + edge
        // re-validation) — it must never reach this generic value validator. `update` intercepts it
        // before this is called; this arm is the defensive backstop that keeps an unvalidated type
        // from ever being stored verbatim if a future caller routes it here.
        "type" => Err(NxfError::validation(
            "type is changed via the retype path (resolve + edge re-validation), not a generic set",
        )),
        _ => Ok(value.to_string()),
    }
}

/// The active plugin's declared custom-field names, sorted — for the "not updatable"/"not a custom
/// field" errors so an agent sees the custom options alongside the canonical ones.
fn custom_field_names(cfg: &PluginConfig) -> Vec<&str> {
    let mut names: Vec<&str> = cfg.fields.keys().map(String::as_str).collect();
    names.sort_unstable();
    names
}

/// Map a custom field's plugin-declared merge strategy to the core strategy carried on its
/// `field set` op (plugin-custom-fields §4.2). `OrSet` is impossible here — T2 rejects an `or-set`
/// custom field at plugin load — so it is `unreachable!`.
fn custom_merge_strategy(merge: MergeStrategy) -> CoreMergeStrategy {
    match merge {
        MergeStrategy::Lww => CoreMergeStrategy::Lww,
        MergeStrategy::CrdtText => CoreMergeStrategy::CrdtText,
        MergeStrategy::OrSet => unreachable!(
            "an or-set custom field is rejected at plugin load (T2); it can never reach the write path"
        ),
    }
}

/// Validate one `--set name=value` assignment against a DECLARED custom field (plugin-custom-fields
/// §5). The caller guarantees `name` names a declared field (`cfg.fields[name]` exists); this
/// enforces the field's `on`-scope against `item_type` and validates `value` against the declared
/// data type, returning the canonical value to store + the core merge strategy the `field set` op
/// carries. An **empty** value is always accepted as a CLEAR (§4.2) — it bypasses type validation so
/// a `date`/`enum` field can be cleared. `required` is NOT enforced here: it is a create-time-only
/// guard (§5, TB-CF-7) applied in [`create`], never on `update`.
fn prepare_custom_set(
    cfg: &PluginConfig,
    item_type: &str,
    name: &str,
    value: &str,
) -> Result<(String, CoreMergeStrategy)> {
    let decl = cfg
        .fields
        .get(name)
        .expect("prepare_custom_set is only called for a declared custom field");
    // Type scope (TB-CF-2): `on` empty ⇒ global; else `item_type` must be listed.
    if !decl.on.is_empty() && !decl.on.iter().any(|t| t == item_type) {
        return Err(NxfError::validation(format!(
            "custom field '{name}' does not apply to type '{item_type}'; it applies to {:?}",
            decl.on
        )));
    }
    // Per-type value validation — skipped for an empty value, which is the clear sentinel (§4.2).
    if !value.is_empty() {
        match decl.field_type {
            FieldType::Date => {
                validate::iso_date(value)?;
            }
            FieldType::Enum => {
                if !decl.values.iter().any(|v| v == value) {
                    return Err(NxfError::validation(format!(
                        "'{value}' is not a valid '{name}' value; expected one of {:?}",
                        decl.values
                    )));
                }
            }
            FieldType::Text | FieldType::Longtext => {}
        }
    }
    Ok((value.to_string(), custom_merge_strategy(decl.merge)))
}

/// Parse + validate the create/update custom-field `--set name=value` pairs (plugin-custom-fields
/// §5): each must name a declared custom field (else a loud `validation` error listing the declared
/// set) applicable to `item_type`, with its value validated against the declared type. Returns the
/// staged `(name, canonical value, strategy)` writes AND the set of supplied names (for the
/// create-time `required` check) — nothing is written by the caller until every pair validates.
fn prepare_custom_sets(
    cfg: &PluginConfig,
    item_type: &str,
    pairs: &[(String, String)],
) -> Result<Vec<(String, String, CoreMergeStrategy)>> {
    let mut writes = Vec::with_capacity(pairs.len());
    for (name, value) in pairs {
        if !cfg.fields.contains_key(name) {
            let custom = custom_field_names(cfg);
            return Err(NxfError::validation(format!(
                "'{name}' is not a declared custom field of the active plugin (declared: {custom:?}); \
                 canonical fields have their own create flags"
            )));
        }
        let (canonical, strategy) = prepare_custom_set(cfg, item_type, name, value)?;
        writes.push((name.clone(), canonical, strategy));
    }
    Ok(writes)
}

/// Split the `name=value` custom-set strings (mirroring `--set`) into `(name, value)` pairs, with
/// the `parent`/`defer` aliases normalized away so a canonical alias supplied via `--set` is caught
/// as a non-custom name rather than silently aliased. A pair with no `=` is a loud `validation`
/// error (the same shape `update` uses).
fn split_custom_pairs(raw: &[String]) -> Result<Vec<(String, String)>> {
    raw.iter()
        .map(|s| {
            let (name, value) = s
                .split_once('=')
                .ok_or_else(|| NxfError::validation(format!("expected field=value, got '{s}'")))?;
            Ok((normalize_field(name).to_string(), value.to_string()))
        })
        .collect()
}

/// Every `required` custom field that applies to `item_type` must be among `supplied` — the
/// create-time-only guard (plugin-custom-fields §5, TB-CF-7). The first missing one is a loud
/// `validation` error naming it; checked BEFORE any write so a rejected create writes nothing.
fn check_required_custom_fields(
    cfg: &PluginConfig,
    item_type: &str,
    supplied: &HashSet<&str>,
) -> Result<()> {
    for (name, decl) in &cfg.fields {
        let applies = decl.on.is_empty() || decl.on.iter().any(|t| t == item_type);
        if decl.required && applies && !supplied.contains(name.as_str()) {
            return Err(NxfError::validation(format!(
                "custom field '{name}' is required for type '{item_type}' but was not set"
            )));
        }
    }
    Ok(())
}

/// Does `from` reach `target` by following `dep` edges? Used to reject a cycle at write time.
/// Fallible (#76u.13): the `deps_of` walk surfaces a db error rather than unwinding, so a cycle
/// check that hits a failing store reports `io` instead of panicking.
fn reaches(store: &Store, from: &str, target: &str) -> Result<bool> {
    let mut stack = vec![from.to_string()];
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if node == target {
            return Ok(true);
        }
        if !seen.insert(node.clone()) {
            continue;
        }
        stack.extend(store.deps_of(&node)?);
    }
    Ok(false)
}

/// Does `from` reach `target` by following `parent` edges (walking up the ancestor chain)? Used to
/// reject a parent CYCLE at write time (sp6.8, ②d): a candidate edge `child → parent` would close a
/// cycle iff `parent` already reaches `child` over parent edges. The bounded visited set terminates
/// even on a transient merge-delivered loop, exactly like [`reaches`] does for deps.
fn reaches_via_parent(store: &Store, from: &str, target: &str) -> Result<bool> {
    let mut stack = vec![from.to_string()];
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if node == target {
            return Ok(true);
        }
        if !seen.insert(node.clone()) {
            continue;
        }
        // Fallible parent read (#76u.14): mirrors the dep-cycle `reaches` walk so a db error on this
        // long-lived write-validation path maps to `io` instead of `.expect()`-unwinding the handler.
        stack.extend(store.parents_of_result(&node)?);
    }
    Ok(false)
}

// ---- create ----------------------------------------------------------------

/// Create an item: resolve vocabulary, validate every field, mint a unique id under `prefix`,
/// and write the full field model. Returns the materialized record. Nothing is written if any
/// validation fails or a parent/dependency reference is dangling.
#[allow(clippy::too_many_arguments)]
pub fn create(
    store: &mut Store,
    cfg: &PluginConfig,
    prefix: &str,
    now: &str,
    actor: &str,
    ty: &str,
    title: &str,
    new: NewItem,
) -> Result<ItemRow> {
    let actor = validate::actor(actor)?;
    // Validate everything before mutating, so a rejected create writes nothing. The named
    // priority variant is resolved to its canonical ordinal here.
    let item_type = resolve_item_type(cfg, ty)?;
    let priority = resolve_priority(cfg, new.priority)?;
    let due = new.due.map(validate::iso_date).transpose()?;
    let defer = new.defer.map(validate::iso_date).transpose()?;

    // Custom fields (plugin-custom-fields §5): validate every `--set name=value` against the
    // declared type + `on`-scope, then enforce that every applicable `required` field is supplied —
    // all BEFORE minting/writing, so a rejected create writes nothing.
    let custom_pairs = split_custom_pairs(new.custom)?;
    let custom_writes = prepare_custom_sets(cfg, &item_type, &custom_pairs)?;
    let supplied: HashSet<&str> = custom_writes.iter().map(|(n, _, _)| n.as_str()).collect();
    check_required_custom_fields(cfg, &item_type, &supplied)?;

    // Every dependency target must exist (and be live) before we mint anything — a dangling
    // reference fails loudly and writes nothing, mirroring `dep_add`'s existence check.
    for dep in new.depends_on {
        require_live(store, dep, "dependency")?;
    }

    // Production mints a random suffix; under the golden-docs determinism switch
    // (nexus-flow-4oa.2) it mints the next free sequential id so example output is stable. Both
    // go through the same local-collision check, so uniqueness is identical either way.
    let minter = ReplicaIds::new(prefix);
    // The local-collision probe is a one-shot check on the just-folded local store; the `is_taken`
    // closure can't be fallible (the minter takes `FnMut(&str) -> bool`), so a db error degrades to
    // "candidate free" exactly as the pre-#76u.14 `get_item` swallow did — behaviour-preserving on
    // this create path (a genuine db fault here is a local bug, not the contended-server case).
    let is_taken = |cand: &str| store.get_item(cand).map(|o| o.is_some()).unwrap_or(false);
    let id = if crate::workspace::deterministic_ids_enabled() {
        minter.mint_sequential(is_taken)
    } else {
        minter.mint_unique(is_taken)
    }
    .ok_or_else(|| {
        NxfError::io("could not mint a unique id; the replica's suffix space may be exhausted")
    })?;

    // The parent (if any) is validated against the relationship matrix now — after minting (it needs
    // the child id, which has no parents/children yet) but BEFORE any write, so a rejected parent
    // still writes nothing. Minting is pure (emits no op), so this ordering is side-effect-free.
    let parent_mode = match new.parent {
        Some(parent) => Some(check_parent(
            &active_matrix(cfg),
            store,
            &id,
            &item_type,
            parent,
        )?),
        None => None,
    };

    stamp_now(store, now)?;
    store.create_item(&id, &item_type, title, actor);
    // Mandatory fields: description (free text) + priority (canonical ordinal, resolved above).
    store.set_field(&id, "description", Some(new.description.to_string()), actor);
    store.set_field(&id, "priority", Some(priority), actor);
    // Optional fields.
    if let Some(d) = new.design {
        store.set_field(&id, "design", Some(d.to_string()), actor);
    }
    if let Some(d) = new.dod {
        store.set_field(&id, "completion_criterion", Some(d.to_string()), actor);
    }
    if let Some(d) = due {
        store.set_field(&id, "due", Some(d), actor);
    }
    if let Some(d) = defer {
        store.set_field(&id, "defer_until", Some(d), actor);
    }
    if let (Some(parent), Some(mode)) = (new.parent, parent_mode) {
        // sp6.3/sp6.6: parenthood is a `parent` OR-set edge; the matrix decided re-point vs add.
        write_parent(store, mode, &id, parent, actor)?;
    }
    // Dependencies: a directed `this -> dep` edge each (this depends on dep). No cycle check is
    // needed — the item is brand new, so nothing reaches it yet; the edge can never close a loop.
    for dep in new.depends_on {
        store.add_edge(&id, dep, EdgeKind::Dep, actor);
    }
    // Custom fields (validated above): each rides a strategy-tagged `field set` op — the first
    // writer through the now-live w213 path, folding into the `custom_fields` view (§4).
    for (name, value, strategy) in &custom_writes {
        store.set_custom_field_merge(&id, name, Some(value.clone()), actor, *strategy);
    }

    read_back(store, &id)
}

// ---- update / claim / close ------------------------------------------------

/// Update item fields. Every `field=value` is parsed and validated *before* any is written, so a
/// partially-bad batch leaves the item untouched. Returns the materialized record.
pub fn update(
    store: &mut Store,
    cfg: &PluginConfig,
    now: &str,
    actor: &str,
    id: &str,
    sets: &[String],
) -> Result<ItemRow> {
    let actor = validate::actor(actor)?;
    let item = require_item(store, id)?;
    let current_type = item.item_type.clone().unwrap_or_default();
    let matrix = active_matrix(cfg);

    let mut parsed = Vec::with_capacity(sets.len());
    // `type` (the retype primitive, #yod / #5qp.7) and `belongs_to`/`parent` (the edge) are
    // relationship-bearing — held aside from the plain LWW cells and validated against the matrix
    // before any write.
    let mut type_write: Option<String> = None;
    let mut belongs_to_write: Option<String> = None;
    // Declared custom fields (plugin-custom-fields §5), held aside as raw `(name, value)` pairs and
    // validated below against the EFFECTIVE type (a same-call retype may change it), mirroring how
    // the parent edge is validated against `effective_type`.
    let mut raw_custom: Vec<(String, String)> = Vec::new();
    for raw in sets {
        let (field, value) = raw
            .split_once('=')
            .ok_or_else(|| NxfError::validation(format!("expected field=value, got '{raw}'")))?;
        let field = normalize_field(field);
        // `type` can't go through the generic `prepare_set` value validator: it is resolved against
        // the plugin's DECLARED set (so a retype can only ever land on a valid type — the #yod
        // repair never introduces a new undeclared value), then its edges are re-validated below.
        if field == "type" {
            type_write = Some(resolve_item_type(cfg, value)?);
            continue;
        }
        // A declared custom field is intercepted BEFORE the canonical `prepare_set` whitelist, so a
        // custom name is not rejected as "not updatable"; it is validated against the effective type
        // below (a custom name never collides with a canonical field/alias — the load-time guard).
        if cfg.fields.contains_key(field) {
            raw_custom.push((field.to_string(), value.to_string()));
            continue;
        }
        // An empty value CLEARS an optional cell (6j6v.68ke) — set to NULL, bypassing the per-field
        // value validator (which would reject an empty date). Only the clearable optionals; a
        // validated/required field still runs `prepare_set` and fails loudly on an empty value.
        if value.is_empty() && CLEARABLE_ON_EMPTY.contains(&field) {
            parsed.push((field, None));
            continue;
        }
        let value = prepare_set(cfg, field, value)?;
        if field == "belongs_to" {
            belongs_to_write = Some(value);
        } else {
            parsed.push((field, Some(value)));
        }
    }

    // The item's type after this update — a concurrent retype changes it, so the edge check uses it.
    let effective_type = type_write.as_deref().unwrap_or(&current_type);
    // Custom fields validate against the effective type (`on`-scope + declared type). `required` is
    // deliberately NOT re-enforced on update (TB-CF-7): clearing a required field is allowed.
    let custom_writes = prepare_custom_sets(cfg, effective_type, &raw_custom)?;
    // A retype re-validates the item's EXISTING edges under the new type (no silently-broken
    // hierarchy). NB: combining a retype with a re-parent in one call validates the old edges under
    // the new type; split the two calls if that ever conflicts (the #yod repair retypes alone).
    if let Some(new_type) = &type_write {
        revalidate_type_change(&matrix, store, id, new_type)?;
    }
    // Re-pointing `parent` is validated against the relationship matrix (sp6.6) — before any write —
    // exactly like `create --parent`, under the effective (possibly just-changed) child type.
    let parent_write = match &belongs_to_write {
        Some(parent) => Some((
            parent.clone(),
            check_parent(&matrix, store, id, effective_type, parent)?,
        )),
        None => None,
    };

    stamp_now(store, now)?;
    // The retype op-log append (LWW), before the plain cells.
    if let Some(new_type) = type_write {
        store.set_field(id, "type", Some(new_type), actor);
    }
    // Capture a status transition before consuming `parsed`, so closed_at can be kept in sync.
    // `status` is not clearable, so its parsed value is always `Some` — `and_then` unwraps it.
    let new_status = parsed
        .iter()
        .find(|(f, _)| *f == "status")
        .and_then(|(_, v)| v.clone());
    for (field, value) in parsed {
        // `value` is `None` for an empty-value clear (68ke), `Some(_)` for a normal set.
        store.set_field(id, field, value, actor);
    }
    // Custom fields (validated above): each rides a strategy-tagged `field set` op (§4). An empty
    // value is a clear (§4.2) — stored as an empty-value set, read back as unset.
    for (name, value, strategy) in &custom_writes {
        store.set_custom_field_merge(id, name, Some(value.clone()), actor, *strategy);
    }
    if let Some((parent, mode)) = parent_write {
        // sp6.3/sp6.6: `parent` writes the OR-set parent edge; the matrix chose re-point vs add.
        write_parent(store, mode, id, &parent, actor)?;
    }
    // Keep `closed_at` in sync with the status transition (C5 review #3): present iff status ==
    // closed. A move INTO closed stamps the instant; a move OUT of closed clears a stale one.
    if let Some(status) = new_status {
        if status == "closed" {
            store.set_field(id, "closed_at", Some(now.to_string()), actor);
        } else if item.closed_at.is_some() {
            store.set_field(id, "closed_at", None, actor);
        }
    }
    read_back(store, id)
}

/// Claim an item: mark it in progress (and optionally assign it). `in_progress` is a canonical
/// core status, so no plugin validation is needed.
pub fn claim(
    store: &mut Store,
    now: &str,
    actor: &str,
    id: &str,
    assignee: Option<&str>,
) -> Result<ItemRow> {
    let actor = validate::actor(actor)?;
    let item = require_item(store, id)?;
    stamp_now(store, now)?;
    store.set_field(id, "status", Some("in_progress".to_string()), actor);
    if let Some(a) = assignee {
        store.set_field(id, "assignee", Some(a.to_string()), actor);
    }
    // Reopening out of `closed` clears the close instant, mirroring how `unarchive` clears
    // `archived` — keeping the invariant `closed_at` is present iff status == closed (C5 review #3).
    if item.closed_at.is_some() {
        store.set_field(id, "closed_at", None, actor);
    }
    // 07a.1: claim-up — propagate `in_progress` UP the full parent chain. Every OPEN ancestor (the
    // transitive `parents_of` set, not the single `belongs_to` projection) becomes `in_progress` so
    // a container reflects that work has started inside it. The one *stored* write of epic 07a; the
    // rest of the parent↔child coupling (suppression, masks) is pure derivation
    // (docs/specs/07a-parent-child-status-coupling.md).
    claim_up(store, actor, id)?;
    read_back(store, id)
}

/// Propagate `in_progress` up the full parent chain of `start` (07a.1). A depth-first walk (a
/// `Vec`-as-stack `pop`/`extend`) over the transitive `parents_of` set, bounded by a visited set so a
/// convergence-delivered transient parent loop terminates (the write seam already rejects parent
/// cycles, sp6.8) — the order is irrelevant since every open ancestor is flipped regardless. An
/// ancestor is flipped ONLY
/// when its stored status is `open`: a `closed` ancestor is left closed (a descendant claim never
/// revives a deliberately-closed container), and an already-`in_progress` one is a no-op. A
/// blocked/deferred ancestor is stored `open`, so it is flipped too — the existing
/// "claimed-then-blocked" state (reachable via `list`, not `next`/`blocked`). The wall clock is
/// already stamped by the caller, so every emitted op shares the claim's `now`.
fn claim_up(store: &mut Store, actor: &str, start: &str) -> Result<()> {
    // Fallible parent walk (07a.7): the claim-up walk runs on the long-lived server (flow_claim), so
    // it reads the parent edge via `parents_of_result` + fallible `get_item` — a db error maps to
    // `io` instead of `.expect()`-panicking and unwinding the handler.
    let mut seen = HashSet::new();
    seen.insert(start.to_string());
    let mut frontier = store.parents_of_result(start)?;
    while let Some(pid) = frontier.pop() {
        if !seen.insert(pid.clone()) {
            continue; // already visited (or `start`) — cycle/diamond guard
        }
        let Some(parent) = store
            .get_item(&pid)?
            .filter(|i| i.deleted.as_deref() != Some("1"))
        else {
            continue; // a deleted/missing parent is not part of the live chain
        };
        if parent.status.as_deref() == Some("open") {
            store.set_field(&pid, "status", Some("in_progress".to_string()), actor);
        }
        // Walk past every ancestor (even a closed/in_progress one) — a closed link may still sit
        // under an open grandparent that the full-chain claim-up reaches.
        frontier.extend(store.parents_of_result(&pid)?);
    }
    Ok(())
}

/// Close an item with a mandatory closing comment. Closing is the only way an item leaves the
/// board (no hard delete), so a reason is always required (nexus-flow-ohh): it is the "so we
/// closed it this way" half of the intent-vs-outcome pair. The missing-reason check runs before
/// any write, so a rejected close leaves the item untouched.
pub fn close(
    store: &mut Store,
    now: &str,
    actor: &str,
    id: &str,
    reason: Option<&str>,
) -> Result<ItemRow> {
    let actor = validate::actor(actor)?;
    let reason = reason.ok_or_else(|| {
        NxfError::validation("close requires a reason: pass a closing comment (closing is final)")
    })?;
    require_item(store, id)?;
    stamp_now(store, now)?;
    store.set_field(id, "status", Some("closed".to_string()), actor);
    store.set_field(id, "closing_comment", Some(reason.to_string()), actor);
    // C3 (#916.4): stamp the close instant so the `closed` lane can order by close-date descending.
    // `now` is the validated, caller-injected wall clock (the same one `stamp_now` pinned).
    store.set_field(id, "closed_at", Some(now.to_string()), actor);
    read_back(store, id)
}

// ---- dependency / reference edges ------------------------------------------

/// Add a `from -> to` dependency (from depends on to). Rejected at write time if it would create
/// a cycle, so the derivation never has to cope with an invalid edge. Both endpoints must exist.
pub fn dep_add(store: &mut Store, now: &str, actor: &str, from: &str, to: &str) -> Result<()> {
    let actor = validate::actor(actor)?;
    require_item(store, from)?;
    require_item(store, to)?;
    // A new edge from->to closes a cycle iff `to` already reaches `from` (and a self-edge is the
    // degenerate cycle).
    if from == to || reaches(store, to, from)? {
        return Err(NxfError::cycle(format!(
            "'{from}' -> '{to}' would create a dependency cycle"
        )));
    }
    stamp_now(store, now)?;
    store.add_edge(from, to, EdgeKind::Dep, actor);
    Ok(())
}

/// Remove the `from -> to` dependency (observed-remove). Both endpoints are validated like
/// `dep_add`, so a typo is a loud `not_found` rather than a silently-successful no-op.
pub fn dep_remove(store: &mut Store, now: &str, actor: &str, from: &str, to: &str) -> Result<()> {
    let actor = validate::actor(actor)?;
    require_item(store, from)?;
    require_item(store, to)?;
    stamp_now(store, now)?;
    store.remove_edge(from, to, EdgeKind::Dep, actor);
    Ok(())
}

/// Record a `from -> to` reference edge: `from`'s free text cites the short-id `to`. Pure
/// reference, no blocking semantics (it is not a `dep`); a self-reference is intentionally
/// permitted (it forms no cycle and cannot block). Both endpoints must exist.
pub fn mention_add(store: &mut Store, now: &str, actor: &str, from: &str, to: &str) -> Result<()> {
    let actor = validate::actor(actor)?;
    require_item(store, from)?;
    require_item(store, to)?;
    stamp_now(store, now)?;
    store.add_edge(from, to, EdgeKind::Mentions, actor);
    Ok(())
}

/// Remove a `from -> to` reference edge (observed-remove).
pub fn mention_remove(
    store: &mut Store,
    now: &str,
    actor: &str,
    from: &str,
    to: &str,
) -> Result<()> {
    let actor = validate::actor(actor)?;
    require_item(store, from)?;
    require_item(store, to)?;
    stamp_now(store, now)?;
    store.remove_edge(from, to, EdgeKind::Mentions, actor);
    Ok(())
}

/// Record a `from -> to` contributes-to edge: `from` contributes to `to` (n:m, nexus-flow-vf4).
/// Like a `mention` it never blocks (forms no cycle, never appears in `blocked`), so a self-edge
/// is harmless and permitted; both endpoints must exist.
pub fn contributes_add(
    store: &mut Store,
    now: &str,
    actor: &str,
    from: &str,
    to: &str,
) -> Result<()> {
    let actor = validate::actor(actor)?;
    require_item(store, from)?;
    require_item(store, to)?;
    stamp_now(store, now)?;
    store.add_edge(from, to, EdgeKind::ContributesTo, actor);
    Ok(())
}

/// Remove a `from -> to` contributes-to edge (observed-remove).
pub fn contributes_remove(
    store: &mut Store,
    now: &str,
    actor: &str,
    from: &str,
    to: &str,
) -> Result<()> {
    let actor = validate::actor(actor)?;
    require_item(store, from)?;
    require_item(store, to)?;
    stamp_now(store, now)?;
    store.remove_edge(from, to, EdgeKind::ContributesTo, actor);
    Ok(())
}

// ---- labels (h89s.2): user labels as an observed-remove OR-set --------------
//
// A label is opaque user vocabulary attached to an item — distinct from the plugin's display type.
// The write seam validates the label through the shared [`validate::label`] (trimmed, non-empty,
// separator-free) — the SAME normalization the beads migration reuses — and requires the item to
// exist, so a typo is a loud error rather than a silent no-op; the core folds it into its own
// `present_labels` view (h89s.1). Additive: no existing write path changes.

/// Attach a user label to an item (OR-set add). The item must exist; the label is trimmed and must
/// be non-empty. Idempotent in effect — re-adding a present label leaves one present label.
pub fn label_add(store: &mut Store, now: &str, actor: &str, id: &str, label: &str) -> Result<()> {
    let actor = validate::actor(actor)?;
    require_item(store, id)?;
    let label = validate::label(label)?;
    stamp_now(store, now)?;
    store.add_label(id, &label, actor);
    Ok(())
}

/// Detach a label from an item (observed-remove). The item must exist and the label is validated
/// identically to [`label_add`], so a padded/blank spelling is rejected rather than silently
/// missing the stored (trimmed) form.
pub fn label_remove(
    store: &mut Store,
    now: &str,
    actor: &str,
    id: &str,
    label: &str,
) -> Result<()> {
    let actor = validate::actor(actor)?;
    require_item(store, id)?;
    let label = validate::label(label)?;
    stamp_now(store, now)?;
    store.remove_label(id, &label, actor);
    Ok(())
}

// ---- thread links (nxf 6j6v.8dbe) -------------------------------------------
//
// A chat thread as an endpoint of a link to a board item, with two independent attributes. Only the
// ITEM endpoint is existence-checked: the thread lives in the chat store's id space, which flow
// does not own and cannot read (`crates/chat` depends on the shared substrate alone and holds no
// flow vocabulary, spec §4.4 — the separation runs both ways). The engine supplies where the link
// lives and how it is addressed; judging whether a given thread is worth linking is the caller's.

/// Attach `thread` to `item` with a relation and a weight.
///
/// Idempotent in effect and re-callable at any point in the thread's life: re-attaching the same
/// pair with different attributes is how a link firms up (`passing` → `bearing`), not an error, and
/// nothing forces the link to exist early. See `Store::add_thread_link` for the convergence story.
pub fn thread_link_add(
    store: &mut Store,
    now: &str,
    actor: &str,
    thread: &str,
    item: &str,
    relation: LinkRelation,
    weight: LinkWeight,
) -> Result<()> {
    let actor = validate::actor(actor)?;
    require_item(store, item)?;
    let thread = validate::thread_id(thread)?;
    stamp_now(store, now)?;
    store.add_thread_link(&thread, item, relation, weight, actor);
    Ok(())
}

/// Detach `thread` from `item` (observed-remove), whatever attributes the link currently carries.
/// The item is validated exactly as in [`thread_link_add`], so a typo is a loud `not_found` rather
/// than a silently-successful no-op (mirrors [`dep_remove`]).
pub fn thread_link_remove(
    store: &mut Store,
    now: &str,
    actor: &str,
    thread: &str,
    item: &str,
) -> Result<()> {
    let actor = validate::actor(actor)?;
    require_item(store, item)?;
    let thread = validate::thread_id(thread)?;
    stamp_now(store, now)?;
    store.remove_thread_link(&thread, item, actor);
    Ok(())
}

// ---- archive / unarchive (C5 #916.5) ---------------------------------------
//
// Two cascading mutations over the `archived` tombstone (C1). `archive` cascades DOWN a fully-
// closed subtree (put a finished epic away with its children); `unarchive` cascades UP only
// (surface an item by resurfacing its ancestor chain, never its siblings/children) so making
// something visible is deterministic. Both are partial batches — atomic PER ROOT (a root that
// fails its precondition writes nothing of its subtree), independent BETWEEN roots — and both
// take `now`/`actor` explicitly and write through the shared store, so the file-change watch
// (#9t7.3) notifies subscribers exactly as for any other write (the "change event").

/// Why a root in an archive/unarchive batch could not be processed — a closed reason-code set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveReason {
    /// No such live item.
    NotFound,
    /// `archive`: the root is not closed (open/in_progress) — close it before archiving.
    NotClosed,
    /// `archive`: the root is closed but a descendant is still open — close the subtree first.
    HasOpenDescendants,
    /// `unarchive`: the item is not archived, so there is nothing to surface.
    NotArchived,
}

impl ArchiveReason {
    /// The stable, machine-readable reason code (the `--json` value).
    pub fn code(self) -> &'static str {
        match self {
            ArchiveReason::NotFound => "not-found",
            ArchiveReason::NotClosed => "not-closed",
            ArchiveReason::HasOpenDescendants => "has-open-descendants",
            ArchiveReason::NotArchived => "not-archived",
        }
    }
}

/// One input root's outcome. `ok` is the per-root success; on failure `reason` carries why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootOutcome {
    pub id: String,
    pub ok: bool,
    pub reason: Option<ArchiveReason>,
}

impl RootOutcome {
    fn ok(id: &str) -> RootOutcome {
        RootOutcome {
            id: id.to_string(),
            ok: true,
            reason: None,
        }
    }
    fn fail(id: &str, reason: ArchiveReason) -> RootOutcome {
        RootOutcome {
            id: id.to_string(),
            ok: false,
            reason: Some(reason),
        }
    }
}

/// The result of an `archive`/`unarchive` batch: one [`RootOutcome`] per input id (in input
/// order), plus `affected` — the full set of ids this call actually (un)archived, INCLUDING the
/// cascaded ones, id-sorted and deduped (already-archived/already-visible items are not re-stamped
/// and so are absent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchResult {
    pub outcomes: Vec<RootOutcome>,
    pub affected: Vec<String>,
}

/// A live item is "closed" for the archive precondition if it is `status='closed'` OR already
/// archived (already put away counts as done — re-archiving its ancestor is idempotent).
fn closed_for_archive(item: &ItemRow) -> bool {
    item.status.as_deref() == Some("closed") || item.archived.is_some()
}

/// Every live descendant of `root` via the `belongs_to` hierarchy (recursive, root excluded).
/// Built from a single `list_items()` scan so the traversal is one pass regardless of fan-out;
/// deleted items are skipped (they are not part of the live subtree).
fn live_descendants(store: &Store, root: &str) -> Result<Vec<ItemRow>> {
    let live: Vec<ItemRow> = store
        .list_items()?
        .into_iter()
        .filter(|i| i.deleted.as_deref() != Some("1"))
        .collect();
    let mut out = Vec::new();
    let mut frontier = vec![root.to_string()];
    let mut seen = HashSet::new();
    seen.insert(root.to_string());
    while let Some(parent) = frontier.pop() {
        for child in live
            .iter()
            .filter(|i| i.belongs_to.as_deref() == Some(&parent))
        {
            if seen.insert(child.id.clone()) {
                frontier.push(child.id.clone());
                out.push(child.clone());
            }
        }
    }
    Ok(out)
}

/// The live ancestor chain of `start` via `belongs_to` (recursive upward, `start` excluded).
/// Stops at the first missing/deleted link or a cycle (defensive — `belongs_to` is acyclic by the
/// write-time guards, but convergence can transiently deliver a loop).
fn live_ancestors(store: &Store, start: &ItemRow) -> Result<Vec<ItemRow>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    seen.insert(start.id.clone());
    let mut cur = start.belongs_to.clone();
    while let Some(pid) = cur {
        if !seen.insert(pid.clone()) {
            break; // cycle guard
        }
        let Some(parent) = store
            .get_item(&pid)?
            .filter(|i| i.deleted.as_deref() != Some("1"))
        else {
            break;
        };
        cur = parent.belongs_to.clone();
        out.push(parent);
    }
    Ok(out)
}

/// `archive <ids…>` — cascade DOWN a fully-closed subtree (C5 #916.5). Per root: the root must be
/// closed and every descendant closed (already-archived counts as closed); then the root and every
/// not-yet-archived descendant are stamped `archived = now`. Atomic per root (a failing root writes
/// nothing of its subtree), independent between roots. Idempotent: already-archived items keep
/// their original instant and are not reported in `affected`.
pub fn archive(store: &mut Store, now: &str, actor: &str, ids: &[String]) -> Result<BatchResult> {
    let actor = validate::actor(actor)?;
    stamp_now(store, now)?;
    let mut outcomes = Vec::with_capacity(ids.len());
    let mut affected = HashSet::new();
    for id in ids {
        let Some(root) = store
            .get_item(id)?
            .filter(|i| i.deleted.as_deref() != Some("1"))
        else {
            outcomes.push(RootOutcome::fail(id, ArchiveReason::NotFound));
            continue;
        };
        if !closed_for_archive(&root) {
            outcomes.push(RootOutcome::fail(id, ArchiveReason::NotClosed));
            continue;
        }
        let descendants = live_descendants(store, id)?;
        if !descendants.iter().all(closed_for_archive) {
            outcomes.push(RootOutcome::fail(id, ArchiveReason::HasOpenDescendants));
            continue;
        }
        // Preconditions hold — stamp the root + every not-yet-archived descendant.
        for item in std::iter::once(&root).chain(descendants.iter()) {
            if item.archived.is_none() {
                store.set_field(&item.id, "archived", Some(now.to_string()), actor);
                affected.insert(item.id.clone());
            }
        }
        outcomes.push(RootOutcome::ok(id));
    }
    let mut affected: Vec<String> = affected.into_iter().collect();
    affected.sort();
    Ok(BatchResult { outcomes, affected })
}

/// `unarchive <ids…>` — cascade UP only (C5 #916.5). Per root: the item must be archived, then it
/// and every archived ancestor have their `archived` cell cleared so the item can become visible.
/// No downward/sideways cascade — siblings and children stay archived. Same partial-batch shape as
/// [`archive`]; reasons `not-found`/`not-archived`.
pub fn unarchive(store: &mut Store, now: &str, actor: &str, ids: &[String]) -> Result<BatchResult> {
    let actor = validate::actor(actor)?;
    stamp_now(store, now)?;
    let mut outcomes = Vec::with_capacity(ids.len());
    let mut affected = HashSet::new();
    for id in ids {
        let Some(item) = store
            .get_item(id)?
            .filter(|i| i.deleted.as_deref() != Some("1"))
        else {
            outcomes.push(RootOutcome::fail(id, ArchiveReason::NotFound));
            continue;
        };
        if item.archived.is_none() {
            outcomes.push(RootOutcome::fail(id, ArchiveReason::NotArchived));
            continue;
        }
        // Clear the item + every archived ancestor (skip ancestors that are already visible).
        let ancestors = live_ancestors(store, &item)?;
        for target in std::iter::once(&item).chain(ancestors.iter()) {
            if target.archived.is_some() {
                store.set_field(&target.id, "archived", None, actor);
                affected.insert(target.id.clone());
            }
        }
        outcomes.push(RootOutcome::ok(id));
    }
    let mut affected: Vec<String> = affected.into_iter().collect();
    affected.sort();
    Ok(BatchResult { outcomes, affected })
}

// ---- notes -----------------------------------------------------------------

/// Append an immutable worklog note to an item. Returns the minted note id.
pub fn note_add(store: &mut Store, now: &str, actor: &str, id: &str, text: &str) -> Result<String> {
    let actor = validate::actor(actor)?;
    require_item(store, id)?;
    stamp_now(store, now)?;
    Ok(store.add_note(id, text, actor))
}

#[cfg(test)]
mod tests {
    //! Relationship-matrix enforcement (sp6.6, ②b) — `check_parent` against a Store with a TEST
    //! matrix, covering the store-side axes the pure `matrix::validate_parent` test cannot reach:
    //! the existing-parent count and the hierarchy-depth walk. The per-plugin matrix DECLARATION is
    //! sp6.4; here the matrix is supplied directly, proving the core enforcement is matrix-driven.
    use super::*;
    use crate::error::ErrorKind;
    use crate::matrix::{Cardinality, ParentRule};
    use std::collections::{BTreeMap, BTreeSet};

    fn seeded(items: &[(&str, &str)]) -> Store {
        let mut s = Store::open_in_memory(1);
        for (id, ty) in items {
            s.create_item(id, ty, id, "t");
        }
        s
    }

    /// Build a matrix from `(child, allowed_parents, max_parents)` tuples — `max` of 1 ⇒ `Single`,
    /// `n` ⇒ `Limited(n)`. `list` is derived from every type the rules mention so it is self-consistent.
    fn matrix(rules: &[(&str, &[&str], usize)], max_depth: Option<usize>) -> RelationshipMatrix {
        let mut list = BTreeSet::new();
        let rules = rules
            .iter()
            .map(|(child, parents, max)| {
                list.insert(child.to_string());
                list.extend(parents.iter().map(|s| s.to_string()));
                (
                    child.to_string(),
                    ParentRule {
                        parents: parents.iter().map(|s| s.to_string()).collect(),
                        max: if *max == 1 {
                            Cardinality::Single
                        } else {
                            Cardinality::Limited(*max)
                        },
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        RelationshipMatrix {
            list,
            rules,
            max_depth,
        }
    }

    #[test]
    fn check_parent_enforces_cardinality_n_against_the_store() {
        // A `todo` may have up to TWO `project` parents — the n-cardinality seam (vs the bundled
        // plugins' single-parent default). The third is a loud `validation` reject.
        let mut s = seeded(&[
            ("p.1", "project"),
            ("p.2", "project"),
            ("p.3", "project"),
            ("t.1", "todo"),
        ]);
        let m = matrix(&[("todo", &["project"], 2)], None);

        assert_eq!(
            check_parent(&m, &s, "t.1", "todo", "p.1").unwrap(),
            ParentWriteMode::Add,
            "n-cardinality adds rather than replaces"
        );
        s.add_parent("t.1", "p.1", "t");
        assert_eq!(
            check_parent(&m, &s, "t.1", "todo", "p.2").unwrap(),
            ParentWriteMode::Add,
            "second parent still under the limit"
        );
        s.add_parent("t.1", "p.2", "t");
        assert_eq!(
            check_parent(&m, &s, "t.1", "todo", "p.3").unwrap_err().kind,
            ErrorKind::Validation,
            "a third parent exceeds the cardinality limit"
        );
    }

    #[test]
    fn check_parent_enforces_the_depth_limit_against_the_store() {
        // `project → project` chains, capped at depth 2: a third level is rejected.
        let mut s = seeded(&[("p.1", "project"), ("p.2", "project"), ("p.3", "project")]);
        let m = matrix(&[("project", &["project"], 1)], Some(2));

        // p.1 → p.2 is depth 2 — allowed (and a re-point, since project cardinality is 1).
        assert_eq!(
            check_parent(&m, &s, "p.2", "project", "p.1").unwrap(),
            ParentWriteMode::Replace
        );
        s.set_parent("p.2", "p.1", "t").unwrap();
        // p.1 → p.2 → p.3 would be depth 3 — rejected.
        assert_eq!(
            check_parent(&m, &s, "p.3", "project", "p.2")
                .unwrap_err()
                .kind,
            ErrorKind::Validation,
            "a third hierarchy level exceeds max_depth=2"
        );
    }

    #[test]
    fn check_parent_rejects_disallowed_pair_self_and_missing() {
        let s = seeded(&[("p.1", "project"), ("t.1", "task"), ("t.2", "task")]);
        let m = RelationshipMatrix::default(); // parent must be a project (pre-matrix behaviour)

        assert_eq!(
            check_parent(&m, &s, "t.1", "task", "t.2").unwrap_err().kind,
            ErrorKind::Validation,
            "a task is not an allowed parent type"
        );
        assert_eq!(
            check_parent(&m, &s, "t.1", "task", "t.1").unwrap_err().kind,
            ErrorKind::Validation,
            "an item cannot be its own parent"
        );
        assert_eq!(
            check_parent(&m, &s, "t.1", "task", "nope")
                .unwrap_err()
                .kind,
            ErrorKind::NotFound,
            "a missing parent is not_found"
        );
        assert_eq!(
            check_parent(&m, &s, "t.1", "task", "p.1").unwrap(),
            ParentWriteMode::Replace,
            "a valid project parent re-points (single-parent default)"
        );
    }

    #[test]
    fn depth_walk_folds_a_diamond_dag_once_per_node_not_once_per_path() {
        // Integrity review #1: build a wide diamond DAG — each level `l` has two nodes, both
        // pointing at BOTH nodes of level `l-1`, so there are 2^N distinct root→leaf paths. The
        // memoized walk folds each of the 2·N+2 nodes once (linear); the old clone-per-branch
        // recursion was O(2^N) and would not finish here. Also a correctness check: the longest
        // parent chain from a top node is exactly N+1 levels.
        const N: usize = 30;
        let mut s = Store::open_in_memory(1);
        s.create_item("a0", "task", "a0", "t");
        s.create_item("b0", "task", "b0", "t");
        for l in 1..=N {
            for node in [format!("a{l}"), format!("b{l}")] {
                s.create_item(&node, "task", &node, "t");
                s.add_parent(&node, &format!("a{}", l - 1), "t");
                s.add_parent(&node, &format!("b{}", l - 1), "t");
            }
        }
        assert_eq!(
            depth_above(&s, &format!("a{N}")).unwrap(),
            N + 1,
            "longest parent chain counts a{N} plus N levels above it"
        );
        assert_eq!(
            height_below(&s, "a0").unwrap(),
            N + 1,
            "the downward mirror folds the same DAG linearly"
        );
    }

    // ---- retype primitive (#yod / #5qp.7): `update --set type=` -------------------------------

    const NOW: &str = "2026-06-27T00:00:00Z";

    /// A real bundled plugin so `resolve_item_type` + `active_matrix` use the shipped vocabulary
    /// (issue-tracker: types epic/bug/feature/chore/decision; only `epic` is a container).
    fn issue_tracker() -> PluginConfig {
        crate::plugin::load("issue-tracker").unwrap()
    }

    #[test]
    fn resolve_priority_accepts_label_and_canonical_ordinal_key() {
        let cfg = issue_tracker(); // labels P0..P4 ⇒ ordinals 0..4
                                   // The named label → its ordinal (the primary vocabulary; unchanged behaviour).
        assert_eq!(resolve_priority(&cfg, "P0").unwrap(), "0");
        assert_eq!(resolve_priority(&cfg, "P1").unwrap(), "1");
        assert_eq!(resolve_priority(&cfg, "P4").unwrap(), "4");
        // ee2h: the canonical ordinal KEY — the exact form `show/list/next --json` emit for
        // `priority` — now round-trips back as input, so machine output is valid machine input.
        assert_eq!(resolve_priority(&cfg, "0").unwrap(), "0");
        assert_eq!(resolve_priority(&cfg, "1").unwrap(), "1");
        assert_eq!(resolve_priority(&cfg, "4").unwrap(), "4");
        // Neither form silently accepts an out-of-range ordinal or an unknown label.
        assert!(
            resolve_priority(&cfg, "5").is_err(),
            "ordinal past the set is rejected"
        );
        assert!(
            resolve_priority(&cfg, "P9").is_err(),
            "unknown label is rejected"
        );
        assert!(
            resolve_priority(&cfg, "high").is_err(),
            "gibberish is rejected"
        );
    }

    #[test]
    fn resolve_priority_prefers_a_numeric_label_over_the_ordinal_key() {
        // The label branch takes precedence over the ordinal-key branch, so a plugin whose priority
        // variants are THEMSELVES integers keeps label semantics (ee2h). Reorder the labels so a
        // label's value and its index diverge: with `["5","10","1"]`, label "1" sits at ordinal 2.
        let mut cfg = issue_tracker();
        cfg.priority.labels = vec!["5".into(), "10".into(), "1".into()];
        // "1" matches the LABEL at index 2 → "2". Had the ordinal-key branch won, it would be "1".
        assert_eq!(
            resolve_priority(&cfg, "1").unwrap(),
            "2",
            "a numeric label resolves by its index, not its face value"
        );
        // "5" matches the label at index 0 → "0" (as a bare ordinal key it would be out of range).
        assert_eq!(resolve_priority(&cfg, "5").unwrap(), "0");
        // "0" is not a label here; the ordinal-key branch accepts it (0 is in range) → "0".
        assert_eq!(resolve_priority(&cfg, "0").unwrap(), "0");
    }

    #[test]
    fn update_retypes_a_legacy_typed_item_to_a_declared_type() {
        // The #yod case: a pre-plugin item stored with the old core-enum type `project` (which the
        // active plugin does not declare) is lifted onto a declared type via the retype primitive —
        // an op-log append (LWW), not an in-place edit.
        let cfg = issue_tracker();
        let mut s = Store::open_in_memory(1);
        s.create_item("0ppa.1", "project", "Old top-level", "t"); // legacy, undeclared type
        let row = update(
            &mut s,
            &cfg,
            NOW,
            "alice",
            "0ppa.1",
            &["type=epic".to_string()],
        )
        .unwrap();
        assert_eq!(
            row.item_type.as_deref(),
            Some("epic"),
            "the item now carries a declared type"
        );
    }

    #[test]
    fn update_rejects_a_retype_to_an_undeclared_type() {
        // Retyping can only ever land on a type the active plugin declares — so the primitive can
        // repair legacy data without ever introducing a NEW undeclared value.
        let cfg = issue_tracker();
        let mut s = Store::open_in_memory(1);
        s.create_item("0ppa.1", "project", "x", "t");
        let err = update(
            &mut s,
            &cfg,
            NOW,
            "alice",
            "0ppa.1",
            &["type=widget".to_string()],
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation, "unknown type is rejected");
    }

    #[test]
    fn update_retype_revalidates_the_parent_edge_under_the_new_type() {
        // F (feature) is parented under E (epic). Retyping F to `epic` would make it a child epic —
        // but `epic` is top-level (no allowed parents), so the existing parent edge is now invalid:
        // the retype is rejected rather than silently leaving a broken hierarchy.
        let cfg = issue_tracker();
        let mut s = Store::open_in_memory(1);
        s.create_item("e", "epic", "E", "t");
        s.create_item("f", "feature", "F", "t");
        s.set_parent("f", "e", "t").unwrap();
        let err = update(&mut s, &cfg, NOW, "alice", "f", &["type=epic".to_string()]).unwrap_err();
        assert_eq!(
            err.kind,
            ErrorKind::Validation,
            "a retype that orphans an edge is rejected"
        );
        assert!(
            err.msg.contains("top-level type"),
            "the rejection names the edge violation (epic can't be parented), not just a kind: {}",
            err.msg
        );
    }

    #[test]
    fn update_retype_revalidates_child_edges_under_the_new_type() {
        // E (epic) contains F (feature). Retyping E to `feature` (a leaf, not a container) would
        // leave F parented under a non-container — rejected.
        let cfg = issue_tracker();
        let mut s = Store::open_in_memory(1);
        s.create_item("e", "epic", "E", "t");
        s.create_item("f", "feature", "F", "t");
        s.set_parent("f", "e", "t").unwrap();
        let err = update(
            &mut s,
            &cfg,
            NOW,
            "alice",
            "e",
            &["type=feature".to_string()],
        )
        .unwrap_err();
        assert_eq!(
            err.kind,
            ErrorKind::Validation,
            "retyping a container to a leaf is rejected"
        );
        assert!(
            err.msg.contains("may not have"),
            "the rejection names the edge violation (a feature can't parent a feature): {}",
            err.msg
        );
    }

    #[test]
    fn retype_revalidation_surfaces_a_parent_edge_db_failure_as_io_not_a_panic() {
        // 07a.7: `revalidate_type_change` (reached by `update --set type=…` on the long-lived server)
        // reads both edge directions via `parents_of_result`/`children_of_result`. Drop the
        // `present_edges` VIEW alone (leaving `items`/`present_parent`, so `require_item` still
        // resolves the target) so the fault lands INSIDE the revalidation walk — the update must map it
        // to `io`, never `.expect()`-panic. Guards the completed de-panic against a revert to the
        // infallible edge reads.
        let cfg = issue_tracker();
        let mut s = Store::open_in_memory(1);
        s.create_item("e", "epic", "E", "t");
        s.create_item("f", "feature", "F", "t");
        s.set_parent("f", "e", "t").unwrap();
        s.connection()
            .execute_batch("DROP VIEW present_edges;")
            .expect("drop the parent-edge view to fault the revalidation walk");
        let err = update(&mut s, &cfg, NOW, "alice", "f", &["type=chore".to_string()])
            .expect_err("a parent-edge db failure during retype revalidation surfaces");
        assert_eq!(
            err.kind,
            ErrorKind::Io,
            "revalidate_type_change maps a parent-edge read failure to io: {err}"
        );
    }

    #[test]
    fn claim_surfaces_a_parent_walk_db_failure_as_io_not_a_panic() {
        // 07a.7: `claim` propagates in_progress UP the parent chain via `claim_up`, which reads the
        // parent edge with `parents_of_result` on the long-lived server. Same isolation as above (drop
        // only the `present_edges` VIEW so `require_item` still resolves the target) so the fault lands
        // INSIDE the walk — a direct exercise of the seam, not just the proxied cycle-check path.
        let mut s = Store::open_in_memory(1);
        s.create_item("ab12.0001", "task", "parent", "t");
        s.create_item("ab12.0002", "task", "child", "t");
        s.add_parent("ab12.0002", "ab12.0001", "t");
        s.connection()
            .execute_batch("DROP VIEW present_edges;")
            .expect("drop the parent-edge view to fault the claim-up walk");
        let err = claim(&mut s, NOW, "alice", "ab12.0002", None)
            .expect_err("a parent-walk db failure during claim surfaces");
        assert_eq!(
            err.kind,
            ErrorKind::Io,
            "claim_up maps a parent-edge read failure to io: {err}"
        );
    }

    #[test]
    fn retype_is_an_op_log_append_not_an_in_place_edit() {
        // The crux of #yod: the retype must be a real op (replayable + convergent), NOT a mutation
        // of the materialized view — otherwise it would not survive a re-fold or a sync.
        let cfg = issue_tracker();
        let mut a = Store::open_in_memory(1);
        a.create_item("0ppa.1", "project", "Legacy", "t");
        let before = a.op_count();
        update(
            &mut a,
            &cfg,
            NOW,
            "alice",
            "0ppa.1",
            &["type=epic".to_string()],
        )
        .unwrap();
        assert!(a.op_count() > before, "the retype appended an op");

        // Replay A's op-log into a fresh replica: the new type converges — proof it's an op, not a
        // view-only edit.
        let mut b = Store::open_in_memory(2);
        b.apply(&a.export());
        assert_eq!(
            b.get_item("0ppa.1").unwrap().unwrap().item_type.as_deref(),
            Some("epic"),
            "the retype converges across replicas via the op-log"
        );
    }

    #[test]
    fn revalidate_type_change_rejects_a_cardinality_violation() {
        // A `wide` child may have THREE `epic` parents; a `mid` child only TWO. An item already
        // holding three parents cannot be retyped to `mid` — the retype's edge re-validation hits
        // the Cardinality arm (the pair arm is covered by the parent/child tests above). Driven
        // directly with a test matrix, since the bundled plugins are all single-parent (which the
        // matrix models as `Single`/re-point, never a cardinality cap).
        let mut s = seeded(&[
            ("e1", "epic"),
            ("e2", "epic"),
            ("e3", "epic"),
            ("x", "wide"),
        ]);
        s.add_parent("x", "e1", "t");
        s.add_parent("x", "e2", "t");
        s.add_parent("x", "e3", "t");
        let m = matrix(&[("wide", &["epic"], 3), ("mid", &["epic"], 2)], None);
        let err = revalidate_type_change(&m, &s, "x", "mid").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(
            err.msg.contains("maximum"),
            "names the cardinality limit: {}",
            err.msg
        );
    }

    #[test]
    fn retype_migrates_a_fully_legacy_hierarchy_skipping_undeclared_neighbor_edges() {
        // The REAL #yod case (verified on the 0ppa board): the WHOLE hierarchy is the pre-plugin
        // enum {project, task}, neither declared by issue-tracker. Strict both-direction
        // re-validation deadlocks — each end is blocked by the other's still-legacy type. The retype
        // must SKIP an edge whose OTHER end is undeclared (5qp.7 spec), so the board migrates item by
        // item with no chicken-and-egg.
        let cfg = issue_tracker();
        let mut s = Store::open_in_memory(1);
        s.create_item("p", "project", "Legacy parent", "t"); // undeclared
        s.create_item("c", "task", "Legacy child", "t"); // undeclared
        s.set_parent("c", "p", "t").unwrap(); // legacy project→task edge

        // child-first: task→chore succeeds even though the parent is still the undeclared `project`
        // (that edge is skipped — its other end is undeclared).
        update(&mut s, &cfg, NOW, "alice", "c", &["type=chore".to_string()]).unwrap();
        assert_eq!(
            s.get_item("c").unwrap().unwrap().item_type.as_deref(),
            Some("chore")
        );

        // then the parent: project→epic succeeds; the child is now the declared `chore`, which an
        // `epic` may parent — so this edge IS validated (both ends declared) and passes.
        update(&mut s, &cfg, NOW, "alice", "p", &["type=epic".to_string()]).unwrap();
        assert_eq!(
            s.get_item("p").unwrap().unwrap().item_type.as_deref(),
            Some("epic")
        );
    }

    #[test]
    fn retype_migrates_a_legacy_hierarchy_parent_first_too() {
        // The skip makes migration order-independent: parent-first works as well (the parent's
        // still-legacy child edge is skipped; once the child is retyped, both ends are declared).
        let cfg = issue_tracker();
        let mut s = Store::open_in_memory(1);
        s.create_item("p", "project", "Legacy parent", "t");
        s.create_item("c", "task", "Legacy child", "t");
        s.set_parent("c", "p", "t").unwrap();

        update(&mut s, &cfg, NOW, "alice", "p", &["type=epic".to_string()]).unwrap();
        update(
            &mut s,
            &cfg,
            NOW,
            "alice",
            "c",
            &["type=feature".to_string()],
        )
        .unwrap();
        assert_eq!(
            s.get_item("p").unwrap().unwrap().item_type.as_deref(),
            Some("epic")
        );
        assert_eq!(
            s.get_item("c").unwrap().unwrap().item_type.as_deref(),
            Some("feature")
        );
    }

    // ---- #76u.14 / 07a termination + de-panic of the parent walks ----------------------------

    #[test]
    fn the_parent_validation_walk_surfaces_a_db_error_as_io_not_a_panic() {
        // #76u.14: the write-validation parent walk (`reaches_via_parent` → `parents_of_result`)
        // reads the `present_edges` view on the long-lived server. Corrupt its base table and the
        // walk returns `Err` (mapped to `io` at the seam) — the symmetric twin of the dep-cycle
        // `reaches` walk — instead of `.expect()`-unwinding the handler.
        let mut s = Store::open_in_memory(1);
        s.create_item("c", "task", "c", "t");
        s.create_item("p", "task", "p", "t");
        s.connection()
            .execute_batch("DROP TABLE edge_adds;")
            .expect("drop edge_adds to corrupt the present_edges view");
        let err = reaches_via_parent(&s, "p", "c")
            .expect_err("a db error in the parent walk is surfaced, not panicked");
        assert_eq!(
            err.kind,
            ErrorKind::Io,
            "a db read failure maps to io: {err}"
        );
    }

    #[test]
    fn claim_up_flips_a_blocked_but_open_ancestor_to_in_progress() {
        // 07a.1 (intended, owner-acknowledged — review #147 Code #3): claim-up flips EVERY open
        // ancestor regardless of its derived lane. A parent that is `blocked` (stored `open`, held
        // only by an open dep) is still stored `open`, so claiming a child flips it to in_progress —
        // the "claimed-then-blocked" state, reachable via `list`, not `next`/`blocked`. This test
        // pins that surprising-but-deliberate side-effect so a future change can't silently drop it.
        let mut s = Store::open_in_memory(1);
        s.create_item("p", "task", "p", "t");
        s.create_item("x", "task", "x", "t"); // p's open blocker
        s.add_edge("p", "x", EdgeKind::Dep, "t"); // p is blocked (open dep), but stored `open`
        s.create_item("c", "task", "c", "t");
        s.add_parent("c", "p", "t");

        claim(&mut s, "2026-06-17T00:00:00Z", "tester", "c", None).unwrap();
        assert_eq!(
            s.get_item("p").unwrap().unwrap().status.as_deref(),
            Some("in_progress"),
            "claim-up flips the blocked-but-open parent to in_progress (intended)"
        );
    }

    #[test]
    fn claim_up_terminates_on_a_transient_parent_cycle() {
        // 07a.1 / Test #1: `add_parent` has no acyclicity check, so a convergence-delivered transient
        // A↔B parent cycle is constructible. The visited-set guard must make claim-up TERMINATE (the
        // test returning at all is the proof) — and still flip both open nodes to in_progress.
        let mut s = Store::open_in_memory(1);
        s.create_item("a", "task", "a", "t");
        s.create_item("b", "task", "b", "t");
        s.add_parent("a", "b", "t");
        s.add_parent("b", "a", "t"); // A↔B parent cycle (no write-seam check at the store level)

        claim(&mut s, "2026-06-17T00:00:00Z", "tester", "a", None).unwrap();
        assert_eq!(
            s.get_item("a").unwrap().unwrap().status.as_deref(),
            Some("in_progress")
        );
        assert_eq!(
            s.get_item("b").unwrap().unwrap().status.as_deref(),
            Some("in_progress"),
            "claim-up walked the cycle once (visited guard) and flipped the open ancestor"
        );
    }
}
