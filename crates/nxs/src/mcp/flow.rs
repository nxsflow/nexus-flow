//! The flow read compute+render the MCP tools wrap (#4ti): open the shared `.nxs/` store per call
//! (CLI behaviour), compute the canonical record via the flow Record-Facade, and serialize it to the
//! `serde_json::Value` a tool returns as `structuredContent` — byte-identical to `nxf <cmd> --json`.
//!
//! These are plain functions (no rmcp types) that funnel through the SAME facade compute + render
//! helpers the `nxf` CLI uses (`read::list`/`list_to_value`, `read::next`/`next_to_value`,
//! `read::show`/`to_value`), so the cross-seam parity tests (#42x) compare like for like, and the
//! rmcp wiring in the parent module stays a thin adapter over them. prime is NOT among them: it is
//! delivered as static `initialize.instructions` (#76u.5/#76u.6), not a tool — see the note below.

use super::params::{
    FlowArchiveParams, FlowBlockedParams, FlowClaimParams, FlowCloseParams, FlowCreateParams,
    FlowEdgeParams, FlowItemRefParams, FlowLabelParams, FlowListParams, FlowNextParams,
    FlowNoteAddParams, FlowSchemaParams, FlowSearchParams, FlowShowParams, FlowUpdateParams,
};
use nexus_flow_facade::plugin::{self, PluginConfig};
use nexus_flow_facade::workspace::{self as fws, WorkspaceExt};
use nexus_flow_facade::{read, record, validate, write};
use nxs_foundation::error::{NxfError, Result};
use nxs_foundation::workspace::Workspace;
use serde_json::{json, Value};
use std::path::Path;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Open the launch workspace and its active flow plugin — the per-call open that mirrors the CLI's
/// (`Workspace::resolve` + `plugin::load`), so the same store + same vocabulary back every read.
fn open(db: Option<&str>, start: &Path) -> Result<(Workspace, PluginConfig)> {
    let ws = Workspace::resolve(db, start)?;
    let cfg = plugin::load(&fws::flow_plugin(&ws.config))?;
    Ok((ws, cfg))
}

/// Resolve the reference time exactly as `nxf` does: an explicit param wins, else a pinned `NXF_NOW`
/// (the determinism knob, symmetric with the CLI), else the system clock as RFC3339. A malformed
/// explicit value is `validation`-rejected here, before any derivation.
fn resolve_now(over: Option<&str>) -> Result<String> {
    let pinned = over
        .map(str::to_string)
        .or_else(|| std::env::var("NXF_NOW").ok().filter(|s| !s.is_empty()));
    match pinned {
        Some(s) => {
            validate::iso_date(&s)?;
            Ok(s)
        }
        None => OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|e| NxfError::io(format!("formatting current time: {e}"))),
    }
}

/// `flow_list`: items filtered by status/type, in the chosen order (default id). Canonical record:
/// an item array, each with a sparse `parent_closed_reason` join for a closed-masked open child
/// (07a.3 §5). Mirrors `nxf list --json`.
pub(super) fn list(db: Option<&str>, start: &Path, p: &FlowListParams) -> Result<Value> {
    let (ws, cfg) = open(db, start)?;
    let store = ws.open_store()?;
    let sort = p.sort.as_deref().map(read::parse_sort).transpose()?;
    let items = read::list(
        &cfg,
        &store,
        p.status.as_deref(),
        p.item_type.as_deref(),
        sort,
    )?;
    read::list_to_value_with_custom(&cfg, &store, &items)
}

/// `flow_next`: actionable work (ready + already-claimed) ranked by the plugin's `next` policy and
/// tiered to finish before starting. Each record carries its resolved parent join, so a client can
/// group a started epic's cluster by the header row or by each child's `parent`. Mirrors
/// `nxf next --json`.
pub(super) fn next(db: Option<&str>, start: &Path, p: &FlowNextParams) -> Result<Value> {
    let (ws, cfg) = open(db, start)?;
    let store = ws.open_store()?;
    let now = resolve_now(p.now.as_deref())?;
    let sort = p.sort.as_deref().map(read::parse_sort).transpose()?;
    let items = read::next(&cfg, &store, &now, sort)?;
    let mut value = read::next_to_value_with_custom(&cfg, &store, &items)?;
    // 6j6v.srpg: the retrieval rule's hint cell — a sparse `memories` count per row, attached
    // through the SAME shared join `nxf next --json` uses, so this tool stays byte-identical to it
    // rather than growing a second reading of the rule.
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    nexus_flow_cli::memories::attach_counts_to_next(&ws, &ids, &mut value)?;
    Ok(value)
}

/// `flow_show`: one item with its deps, contributes-to edges, and notes. `not_found` if the id does
/// not resolve to a live item. Mirrors `nxf show <id> --json`.
pub(super) fn show(db: Option<&str>, start: &Path, p: &FlowShowParams) -> Result<Value> {
    let (ws, cfg) = open(db, start)?;
    let store = ws.open_store()?;
    // Resolve a bare local suffix to the full `<prefix>.<suffix>` id, exactly as `nxf show` does,
    // so a host can pass a bare id (and the result matches the CLI seam).
    let id = ws.replica.resolve_id(&p.id);
    // h89s.3 + 6j6v.ekf5: the store-aware value carries the item's labels AND the sparse declared
    // `custom` map (both sparse), byte-identical to `nxf show --json`.
    let mut value = read::show_value_with_custom(&cfg, &store, &id)?;
    // 6j6v.srpg: the retrieval rule's full-text cell — the item-scoped memories filed against this
    // item, through the same shared join the CLI uses. An agent reading an item over MCP is exactly
    // the reader the distillate was written for.
    nexus_flow_cli::memories::attach_to_show(
        &nexus_flow_cli::memories::of_item(&ws, &id)?,
        &mut value,
    );
    Ok(value)
}

/// `flow_blocked`: open items held up by an open blocker (or in a cycle), each with its open
/// blockers (id + core status) so a host sees *which* blocker without a follow-up call. Ranked by
/// default. Mirrors `nxf blocked --json`.
pub(super) fn blocked(db: Option<&str>, start: &Path, p: &FlowBlockedParams) -> Result<Value> {
    let (ws, cfg) = open(db, start)?;
    let store = ws.open_store()?;
    let sort = p.sort.as_deref().map(read::parse_sort).transpose()?;
    let rows = read::blocked(&cfg, &store, sort)?;
    // bbq6: the sparse declared `custom` join rides each record, byte-identical to `nxf blocked
    // --json` (the CLI-only priority_label/type_label are stripped in the conformance comparison).
    read::blocked_to_value_with_custom(&cfg, &store, &rows)
}

/// `flow_search`: a lane-ranked substring search over title/description/design/DoD/notes, refined by
/// the optional status/type/archive filters, in the effective-lane order (a `sort` override
/// flattens it). `now` is the defer boundary, resolved like the other reads. Mirrors
/// `nxf search <query> --json`.
pub(super) fn search(db: Option<&str>, start: &Path, p: &FlowSearchParams) -> Result<Value> {
    let (ws, cfg) = open(db, start)?;
    let store = ws.open_store()?;
    let now = resolve_now(p.now.as_deref())?;
    let sort = p.sort.as_deref().map(read::parse_sort).transpose()?;
    let items = read::search(
        &cfg,
        &store,
        &now,
        &p.query,
        read::SearchArgs {
            status: p.status.as_deref(),
            item_type: p.item_type.as_deref(),
            include_archived: p.include_archived,
            archived_only: p.archived_only,
            sort,
        },
    )?;
    Ok(read::items_in_order_value(&items))
}

/// `flow_note_list`: an item's worklog notes as `{ id, body, created_at? }`, oldest-first.
/// `not_found` if the id does not resolve to a live item. Mirrors `nxf note list <id> --json` —
/// including the sparse op-log `created_at` per note (2kjy), so the MCP and CLI seams stay identical.
pub(super) fn note_list(db: Option<&str>, start: &Path, p: &FlowItemRefParams) -> Result<Value> {
    let (ws, _cfg) = open(db, start)?;
    let store = ws.open_store()?;
    // Resolve a bare local suffix to the full id, exactly as `nxf note list` does.
    let id = ws.replica.resolve_id(&p.id);
    let notes = read::notes_with_created(&store, &id)?;
    Ok(read::notes_with_created_to_value(&notes))
}

/// `flow_mention_list`: the short-ids the item cites in its free text, sorted. `not_found` if the id
/// does not resolve to a live item. Mirrors `nxf mention list <id> --json`.
pub(super) fn mention_list(db: Option<&str>, start: &Path, p: &FlowItemRefParams) -> Result<Value> {
    let (ws, _cfg) = open(db, start)?;
    let store = ws.open_store()?;
    let id = ws.replica.resolve_id(&p.id);
    let mentions = read::mentions(&store, &id)?;
    Ok(read::mentions_to_value(&mentions))
}

/// `flow_schema`: the active plugin's valid item types, their containment roles (which types are
/// containers, allowed parents, cardinality, depth limit), and the create/update field model. Pure
/// over the plugin config — no store open. Mirrors `nxf schema --json`.
pub(super) fn schema(db: Option<&str>, start: &Path, _p: &FlowSchemaParams) -> Result<Value> {
    let (_ws, cfg) = open(db, start)?;
    Ok(read::schema(&cfg).to_value())
}

// Note (#76u.6): there is no `flow_prime` tool. prime is delivered as static
// `initialize.instructions` (#76u.5), sourced from `crate::prime::fan_out`; the live ready set comes
// from `flow_next`/`flow_list`. So no prime read-helper lives here.

// ---- write tools (#wpw / #yie) ------------------------------------------------------------------
//
// The mutation half of the seam: open the shared store per call (CLI behaviour), apply the op
// through the SAME `nexus_flow_facade::write` layer the `nxf` CLI uses, and return the canonical
// receipt — byte-identical to `nxf <cmd> --json` (proven by the cross-seam parity tests #ei9).
// `now` and `actor` are EXPLICIT parameters (epic invariant): `now` resolves like the CLI
// (`resolve_now`); `actor` is the already-resolved hybrid identity (#zxj), threaded in by the
// handler. Each receipt mirror is documented against the `nxf` command it must equal.

/// Open the launch workspace + active plugin for a write, and resolve the op-stamp `now` — the
/// shared prelude every write tool runs before applying its facade op. Returns the workspace (for
/// id resolution + the prefix), the plugin config, the open store, and the resolved `now`.
fn open_for_write(
    db: Option<&str>,
    start: &Path,
    now: Option<&str>,
) -> Result<(
    Workspace,
    PluginConfig,
    nexus_flow_core::store::Store,
    String,
)> {
    let (ws, cfg) = open(db, start)?;
    let store = ws.open_store()?;
    let now = resolve_now(now)?;
    Ok((ws, cfg, store, now))
}

/// `flow_create`: mint a new item and return its canonical record receipt. Bare parent/dependency
/// ids resolve against the local prefix, exactly like `nxf create`. Mirrors `nxf create --json`.
pub(super) fn create(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowCreateParams,
) -> Result<Value> {
    let (ws, cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let prefix = &ws.replica.prefix;
    let parent = p
        .parent
        .as_ref()
        .map(|x| ws.replica.resolve_id(x).into_owned());
    let depends_on: Vec<String> = p
        .depends_on
        .iter()
        .map(|d| ws.replica.resolve_id(d).into_owned())
        .collect();
    let item = write::create(
        &mut store,
        &cfg,
        prefix,
        &now,
        actor,
        &p.item_type,
        &p.title,
        write::NewItem {
            description: &p.description,
            priority: &p.priority,
            design: p.design.as_deref(),
            dod: p.dod.as_deref(),
            due: p.due.as_deref(),
            defer: p.defer.as_deref(),
            parent: parent.as_deref(),
            depends_on: &depends_on,
            // Custom `name=value` sets ride straight through to the shared write layer, byte-identical
            // to `nxf create --set` (plugin-custom-fields §5); the facade validates + emits them.
            custom: &p.set,
        },
    )?;
    Ok(record::item_value(&item))
}

/// `flow_update`: set fields on an existing item and return its canonical record receipt. Mirrors
/// `nxf update <id> --set … --json`.
pub(super) fn update(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowUpdateParams,
) -> Result<Value> {
    let (ws, cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let id = ws.replica.resolve_id(&p.id);
    let item = write::update(&mut store, &cfg, &now, actor, &id, &p.set)?;
    Ok(record::item_value(&item))
}

/// `flow_claim`: mark an item in progress (and optionally assign it). Mirrors `nxf claim <id> --json`.
pub(super) fn claim(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowClaimParams,
) -> Result<Value> {
    let (ws, _cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let id = ws.replica.resolve_id(&p.id);
    let item = write::claim(&mut store, &now, actor, &id, p.assignee.as_deref())?;
    Ok(record::item_value(&item))
}

/// `flow_close`: close an item with a mandatory closing comment. Mirrors `nxf close <id> --reason …
/// --json`.
pub(super) fn close(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowCloseParams,
) -> Result<Value> {
    let (ws, _cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let id = ws.replica.resolve_id(&p.id);
    let item = write::close(&mut store, &now, actor, &id, Some(&p.reason))?;
    // 07a.5: the receipt carries a sparse `swept_children` join — children this close pushed into the
    // effective-closed lane (now no open parent left). Byte-identical to `nxf close --json`.
    read::close_receipt_value(&store, &item)
}

/// The two-id edge receipt shape the CLI's `emit_edge` emits under `--json`:
/// `{ "ok": true, "msg": <sentence over the FULL ids> }`. Shared by the dep/mention add+remove
/// tools so each is byte-identical to its `nxf <cmd> --json`.
fn edge_receipt(from: &str, to: &str, sentence: impl Fn(&str, &str) -> String) -> Value {
    json!({ "ok": true, "msg": sentence(from, to) })
}

/// `flow_dep_add`: add a `from -> to` dependency (from depends on to); cycle-rejected at write time.
/// Mirrors `nxf dep add <from> <to> --json`.
pub(super) fn dep_add(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowEdgeParams,
) -> Result<Value> {
    let (ws, _cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let from = ws.replica.resolve_id(&p.from).into_owned();
    let to = ws.replica.resolve_id(&p.to).into_owned();
    write::dep_add(&mut store, &now, actor, &from, &to)?;
    Ok(edge_receipt(&from, &to, |a, b| format!("{a} -> {b}")))
}

/// `flow_dep_remove`: remove the `from -> to` dependency (observed-remove). Mirrors
/// `nxf dep remove <from> <to> --json`.
pub(super) fn dep_remove(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowEdgeParams,
) -> Result<Value> {
    let (ws, _cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let from = ws.replica.resolve_id(&p.from).into_owned();
    let to = ws.replica.resolve_id(&p.to).into_owned();
    write::dep_remove(&mut store, &now, actor, &from, &to)?;
    Ok(edge_receipt(&from, &to, |a, b| {
        format!("removed {a} -> {b}")
    }))
}

/// `flow_mention_add`: record a `from -> to` reference edge (from's text cites to); never blocks.
/// Mirrors `nxf mention add <from> <to> --json`.
pub(super) fn mention_add(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowEdgeParams,
) -> Result<Value> {
    let (ws, _cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let from = ws.replica.resolve_id(&p.from).into_owned();
    let to = ws.replica.resolve_id(&p.to).into_owned();
    write::mention_add(&mut store, &now, actor, &from, &to)?;
    Ok(edge_receipt(&from, &to, |a, b| format!("{a} mentions {b}")))
}

/// `flow_mention_remove`: remove a `from -> to` reference edge (observed-remove). Mirrors
/// `nxf mention remove <from> <to> --json`.
pub(super) fn mention_remove(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowEdgeParams,
) -> Result<Value> {
    let (ws, _cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let from = ws.replica.resolve_id(&p.from).into_owned();
    let to = ws.replica.resolve_id(&p.to).into_owned();
    write::mention_remove(&mut store, &now, actor, &from, &to)?;
    Ok(edge_receipt(&from, &to, |a, b| {
        format!("removed mention {a} -> {b}")
    }))
}

/// `flow_note_add`: append an immutable worklog note; returns `{ id, body }` like the CLI. Mirrors
/// `nxf note add <id> <text> --json`.
pub(super) fn note_add(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowNoteAddParams,
) -> Result<Value> {
    let (ws, _cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let id = ws.replica.resolve_id(&p.id);
    let note_id = write::note_add(&mut store, &now, actor, &id, &p.text)?;
    Ok(json!({ "id": note_id, "body": p.text }))
}

// ---- remaining write tools (#76u.12) ------------------------------------------------------------
//
// The rest of the shared facade write surface that the #yie cut did not expose: the contributes-to
// edge (add/remove) and the archive/unarchive batch ops. Same seam shape as above — open per call,
// apply through `nexus_flow_facade::write`, return the canonical receipt byte-identical to
// `nxf <cmd> --json` (the CLI's `emit_edge` / `emit_batch`).

/// `flow_contributes_add`: record a `from -> to` contributes-to edge (n:m, never blocks). Mirrors
/// `nxf contributes add <from> <to> --json`.
pub(super) fn contributes_add(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowEdgeParams,
) -> Result<Value> {
    let (ws, _cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let from = ws.replica.resolve_id(&p.from).into_owned();
    let to = ws.replica.resolve_id(&p.to).into_owned();
    write::contributes_add(&mut store, &now, actor, &from, &to)?;
    Ok(edge_receipt(&from, &to, |a, b| {
        format!("{a} contributes to {b}")
    }))
}

/// `flow_contributes_remove`: remove the `from -> to` contributes-to edge (observed-remove). Mirrors
/// `nxf contributes remove <from> <to> --json`.
pub(super) fn contributes_remove(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowEdgeParams,
) -> Result<Value> {
    let (ws, _cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let from = ws.replica.resolve_id(&p.from).into_owned();
    let to = ws.replica.resolve_id(&p.to).into_owned();
    write::contributes_remove(&mut store, &now, actor, &from, &to)?;
    Ok(edge_receipt(&from, &to, |a, b| {
        format!("removed contributes {a} -> {b}")
    }))
}

// ---- labels (h89s.3): user-vocabulary OR-set, byte-identical to `nxf label …` --

/// The label receipt shape the CLI's `emit_label` emits under `--json`: `{ "ok": true, "msg":
/// <sentence over the FULL id + the verbatim label> }`. Shared by the label add/remove tools so each
/// is byte-identical to its `nxf label <cmd> --json`.
fn label_receipt(id: &str, label: &str, sentence: impl Fn(&str, &str) -> String) -> Value {
    json!({ "ok": true, "msg": sentence(id, label) })
}

/// `flow_label_add`: attach a user label to an item. Mirrors `nxf label add <id> <label> --json`.
pub(super) fn label_add(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowLabelParams,
) -> Result<Value> {
    let (ws, _cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let id = ws.replica.resolve_id(&p.id).into_owned();
    write::label_add(&mut store, &now, actor, &id, &p.label)?;
    Ok(label_receipt(&id, &p.label, |i, l| {
        format!("{i} labelled {l}")
    }))
}

/// `flow_label_remove`: detach a label from an item (observed-remove). Mirrors
/// `nxf label remove <id> <label> --json`.
pub(super) fn label_remove(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowLabelParams,
) -> Result<Value> {
    let (ws, _cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let id = ws.replica.resolve_id(&p.id).into_owned();
    write::label_remove(&mut store, &now, actor, &id, &p.label)?;
    Ok(label_receipt(&id, &p.label, |i, l| {
        format!("{i} unlabelled {l}")
    }))
}

/// `flow_label_list`: the item's labels (sorted). Mirrors `nxf label list <id> --json`.
pub(super) fn label_list(db: Option<&str>, start: &Path, p: &FlowItemRefParams) -> Result<Value> {
    let (ws, _cfg) = open(db, start)?;
    let store = ws.open_store()?;
    let id = ws.replica.resolve_id(&p.id);
    let labels = read::labels(&store, &id)?;
    Ok(read::labels_to_value(&labels))
}

/// The archive/unarchive batch receipt the CLI's `emit_batch` emits under `--json`:
/// `{ "<verb>": [affected ids], "results": [{ id, status, reason }] }`, where `status` is the verb
/// success word or `"failed"` with a machine-readable `reason` code. `verb` is the success word and
/// the affected-list key (`"archived"` / `"unarchived"`). Shared so both tools are byte-identical to
/// their `nxf <cmd> --json`.
fn batch_receipt(res: &write::BatchResult, verb: &str) -> Value {
    let results: Vec<Value> = res
        .outcomes
        .iter()
        .map(|o| {
            json!({
                "id": o.id,
                "status": if o.ok { verb } else { "failed" },
                "reason": o.reason.map(|r| r.code()),
            })
        })
        .collect();
    json!({ verb: res.affected, "results": results })
}

/// `flow_archive`: archive each root + its closed subtree (cascade DOWN). A partial batch — a root
/// that fails its precondition is reported in `results` (never a tool error), so this returns `Ok`
/// even when some root fails. Mirrors `nxf archive <ids…> --json`.
pub(super) fn archive(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowArchiveParams,
) -> Result<Value> {
    let (ws, _cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let ids: Vec<String> = p
        .ids
        .iter()
        .map(|id| ws.replica.resolve_id(id).into_owned())
        .collect();
    let res = write::archive(&mut store, &now, actor, &ids)?;
    Ok(batch_receipt(&res, "archived"))
}

/// `flow_unarchive`: surface each item + its ancestor chain (cascade UP only). Same partial-batch
/// receipt as [`archive`]. Mirrors `nxf unarchive <ids…> --json`.
pub(super) fn unarchive(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &FlowArchiveParams,
) -> Result<Value> {
    let (ws, _cfg, mut store, now) = open_for_write(db, start, p.now.as_deref())?;
    let ids: Vec<String> = p
        .ids
        .iter()
        .map(|id| ws.replica.resolve_id(id).into_owned())
        .collect();
    let res = write::unarchive(&mut store, &now, actor, &ids)?;
    Ok(batch_receipt(&res, "unarchived"))
}

#[cfg(test)]
mod custom_field_tests {
    //! In-process MCP handler coverage for plugin custom fields (mitigation of the ky26 PR review,
    //! Test Quality #1). The subprocess `nxs mcp serve` harness (`tests/mcp.rs`) cannot load the
    //! test-only `file-fixture` plugin — a consumer plugin never bundled into the shipped `nxs` — so
    //! this drives the flow handlers IN-PROCESS with `file-fixture` linked: it proves the MCP WRITE
    //! path forwards a custom `set` to the shared facade and the READ path surfaces the sparse
    //! `custom` map, closing the "parity is load-bearing" gap for this transport.
    use super::{create, show, update, FlowCreateParams, FlowShowParams, FlowUpdateParams};
    use nxs_foundation::workspace::{self, WorkspaceConfig};
    use tempfile::TempDir;

    // Force `file-fixture` (a `file` type + a required `uri` custom field) into the inventory registry
    // for THIS test binary — it is registered from `plugin-probe`, never linked into shipped `nxs`.
    use plugin_probe as _;

    /// A fresh workspace whose active flow plugin is `file-fixture`.
    fn file_workspace() -> TempDir {
        let mut flow = toml::value::Table::new();
        flow.insert("plugin".into(), toml::Value::String("file-fixture".into()));
        let mut products = toml::value::Table::new();
        products.insert("flow".into(), toml::Value::Table(flow));
        let cfg = WorkspaceConfig {
            active_modules: vec!["flow".into()],
            products,
        };
        let tmp = TempDir::new().unwrap();
        workspace::setup(tmp.path(), &cfg).unwrap();
        tmp
    }

    fn create_params(set: Vec<String>, now: &str) -> FlowCreateParams {
        FlowCreateParams {
            item_type: "file".into(),
            title: "report.md".into(),
            description: "a generated report".into(),
            priority: "P1".into(),
            design: None,
            dod: None,
            due: None,
            defer: None,
            parent: None,
            depends_on: vec![],
            set,
            now: Some(now.into()),
            actor: None,
            workspace: None,
        }
    }

    #[test]
    fn flow_create_and_show_round_trip_a_custom_field_through_the_mcp_handlers() {
        let tmp = file_workspace();
        let start = tmp.path();
        // flow_create with a custom `set` (the file-fixture `uri` field is REQUIRED).
        let receipt = create(
            None,
            start,
            "tester",
            &create_params(
                vec!["uri=.nxs/files/report.md".into()],
                "2026-07-13T00:00:00Z",
            ),
        )
        .expect("flow_create with a valid custom set succeeds");
        let id = receipt["id"]
            .as_str()
            .expect("the receipt carries the new id")
            .to_string();
        // The create receipt is the canonical record (no `custom` map), byte-identical to
        // `nxf create --json` (both render `record::item_value`); the value surfaces on reads.
        assert!(
            receipt.get("custom").is_none(),
            "the create receipt is canonical-only"
        );
        // flow_show surfaces the sparse `custom` map carrying the value the MCP write forwarded —
        // rendered by the SAME `read::show_value_with_custom` the CLI `nxf show --json` uses.
        let shown = show(
            None,
            start,
            &FlowShowParams {
                id: id.clone(),
                workspace: None,
            },
        )
        .expect("flow_show resolves the item");
        assert_eq!(
            shown["custom"]["uri"],
            serde_json::json!(".nxs/files/report.md"),
            "the MCP write path forwarded the custom `set` and the read path surfaces it"
        );
    }

    #[test]
    fn flow_create_rejects_a_missing_required_custom_field() {
        let tmp = file_workspace();
        // No `uri` supplied → the create-time `required` guard rejects, naming the field.
        let err = create(
            None,
            tmp.path(),
            "tester",
            &create_params(vec![], "2026-07-13T00:00:00Z"),
        )
        .expect_err("a file without its required uri is rejected");
        assert!(
            err.to_string().contains("uri"),
            "the error names the missing required field: {err}"
        );
    }

    #[test]
    fn flow_update_sets_then_clears_a_custom_field() {
        let tmp = file_workspace();
        let start = tmp.path();
        let id = create(
            None,
            start,
            "tester",
            &create_params(vec!["uri=.nxs/files/a.md".into()], "2026-07-13T00:00:00Z"),
        )
        .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        // Re-set the value via flow_update.
        update(
            None,
            start,
            "tester",
            &FlowUpdateParams {
                id: id.clone(),
                set: vec!["uri=.nxs/files/b.md".into()],
                now: Some("2026-07-13T00:01:00Z".into()),
                actor: None,
                workspace: None,
            },
        )
        .expect("flow_update re-sets the custom field");
        let shown = show(
            None,
            start,
            &FlowShowParams {
                id: id.clone(),
                workspace: None,
            },
        )
        .unwrap();
        assert_eq!(shown["custom"]["uri"], serde_json::json!(".nxs/files/b.md"));
        // Clear it (empty value, §4.2) → the sparse `custom` key drops entirely.
        update(
            None,
            start,
            "tester",
            &FlowUpdateParams {
                id: id.clone(),
                set: vec!["uri=".into()],
                now: Some("2026-07-13T00:02:00Z".into()),
                actor: None,
                workspace: None,
            },
        )
        .expect("clearing a custom field on update is allowed (create-time-only required)");
        let shown = show(
            None,
            start,
            &FlowShowParams {
                id,
                workspace: None,
            },
        )
        .unwrap();
        assert!(
            shown.get("custom").is_none(),
            "a cleared custom field drops the sparse map"
        );
    }
}
