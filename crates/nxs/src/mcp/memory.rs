//! The memory read+write compute the MCP memory tools wrap (#76u.2 / #76u.3): open the shared
//! `.nxs/` memory store per call (CLI behaviour), compute the canonical record via the memory
//! Record-Facade, and serialize it to the `serde_json::Value` a tool returns as `structuredContent`
//! — byte-identical to `nxm <cmd> --json` for the same state.
//!
//! The mirror of [`super::flow`] for the memory line: plain functions (no rmcp types) over the SAME
//! `nexus_memory::facade` compute the `nxm` CLI serializes under `--json`, so the cross-seam parity
//! tests (#76u.4) compare like for like, and the rmcp wiring in the parent module stays a thin
//! adapter over them. `now`/`actor` are EXPLICIT (epic invariant): writes stamp the caller's `now`
//! and record the resolved hybrid `actor`; reads take neither — the `memories` view is independent
//! of the clock. There is no `memory_prime` tool — prime is delivered via initialize.instructions
//! (#76u.5), the memory module contributing its share through the fan-out.
//!
//! **Classification (6j6v.e0z6 / 6j6v.9a1r).** The reads here carry a memory's category, reach,
//! references and reading position — they are fields of the canonical record, so
//! `memory_list`/`memory_search`/`memory_show` serve them without any work on this side. The WRITES
//! set them too since 6j6v.9a1r: `memory_add`/`memory_update` take the optional classification trio
//! and `memory_classify`/`memory_reorder` are the tool halves of `nxm classify`/`nxm reorder`.
//!
//! That gap was worth closing on its own: MCP is how our own apps write, and a memory written
//! without a reach is one the retrieval rule (6j6v.srpg) cannot show ANYWHERE — the reach is what
//! decides whether it reads at session start or on a board item. Sending none of the three still
//! produces exactly the bytes this tool wrote before they existed (the defaults describe the status
//! quo), so the parity below holds for old and new callers alike.
//!
//! **Parity scope (actor).** The byte-parity with `nxm <cmd> --json` is defined with `now` AND
//! `actor` pinned — exactly as the cross-seam parity tests drive it. Absent an explicit `actor`, a
//! write resolves the author through the UMBRELLA server's ONE actor hybrid (`--actor`/`NXS_ACTOR` →
//! `NXF_ACTOR`/`USER` → `"nxs"`, #zxj — the shared `pick_actor` hybrid), NOT the `nxm` CLI's
//! `NXM_ACTOR`/`"nxm"` default. That divergence in the unpinned path is deliberate: one stdio server
//! writes under the single nxs identity for flow and memory alike, not the per-module persona. The
//! `memory_*_actor`/`nxs`-identity tests in `tests/mcp.rs` lock this in.

use super::params::{
    MemoryAddParams, MemoryClassifyParams, MemoryCloseParams, MemoryListParams,
    MemoryReorderParams, MemorySearchParams, MemoryShowParams, MemoryUpdateParams,
};
use nexus_memory::facade::{self, Classification, MemoryRecord};
use nexus_memory::model::Scope;
use nexus_memory::store::{MemoryQuery, MemoryStore};
use nexus_memory::workspace::MemoryWorkspaceExt;
use nxs_foundation::error::{NxfError, Result};
use nxs_foundation::workspace::Workspace;
use serde_json::{json, Value};
use std::path::Path;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Open the launch workspace's memory store — the per-call open that mirrors the `nxm` CLI's
/// (`Workspace::resolve` + `open_memory_store`), so the same shared store backs every read/write.
fn open(db: Option<&str>, start: &Path) -> Result<MemoryStore> {
    let ws = Workspace::resolve(db, start)?;
    ws.open_memory_store()
}

/// Resolve the write `now` exactly as `nxm` does: an explicit param wins, else a pinned `NXM_NOW`
/// (the determinism knob, symmetric with the CLI), else the system clock as RFC3339. Like the CLI's
/// `resolve_now`, the value is NOT format-validated — `nxm remember`/`forget` stamp it verbatim, so
/// validating here would diverge from `nxm <cmd> --json` (the parity invariant).
fn resolve_now(over: Option<&str>) -> Result<String> {
    let pinned = over
        .map(str::to_string)
        .or_else(|| std::env::var("NXM_NOW").ok().filter(|s| !s.is_empty()));
    match pinned {
        Some(s) => Ok(s),
        None => OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|e| NxfError::io(format!("formatting current time: {e}"))),
    }
}

/// Serialize a memory record to the JSON value a tool returns — byte-identical to the CLI's
/// `print_record` (`serde_json::to_value` over the same [`MemoryRecord`], declared field order).
fn record_value(rec: &MemoryRecord) -> Value {
    serde_json::to_value(rec).expect("memory record serializes")
}

/// Build the facade [`Classification`] from a tool's optional trio (6j6v.9a1r) — the exact mapping
/// the `nxm` CLI's `ClassArgs` performs, so a tool call and the matching `nxm` invocation resolve
/// the same registers. `None` leaves a register untouched; `Some(vec![])` for `refs` is the wire
/// form of `--refs ""` (an explicit "no references").
///
/// Only the REACH is parsed here, because it is the one closed vocabulary of the three: an unknown
/// word is rejected with the accepted set named, rather than stored as a reach nothing can read.
/// The category slug and the reference shapes are validated by the facade itself (one guard for
/// every seam), so this layer adds no second opinion.
///
/// An EMPTY string is a value, not an omission (PR #282 review, Code Quality #3): `""` is rejected
/// as an unknown reach, exactly as `nxm --scope ""` is rejected by clap. The empty-means-unset
/// convention belongs to the `workspace`/`actor` FALLBACK knobs next door and must not leak onto a
/// register a caller is setting, or `scope: ""` would quietly mean "leave it as it was" here and
/// "that is not a reach" on the CLI — a parity divergence in the one field this ticket exists for.
fn classification(
    category: &Option<String>,
    scope: &Option<String>,
    refs: &Option<Vec<String>>,
    introduction: &Option<String>,
) -> Result<Classification> {
    let scope = match scope.as_deref() {
        None => None,
        Some(raw) => Some(Scope::parse(raw).ok_or_else(|| {
            NxfError::validation(format!(
                "invalid scope '{raw}': use one of {}",
                Scope::ALL
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ")
            ))
        })?),
    };
    Ok(Classification {
        category: category.clone(),
        scope,
        refs: refs.clone(),
        introduction: introduction.clone(),
    })
}

// ---- reads --------------------------------------------------------------------------------------

/// `memory_list`: all active memories, key-sorted. Canonical record: an array of memory records.
/// Mirrors `nxm memories --json`.
pub(super) fn list(db: Option<&str>, start: &Path, _p: &MemoryListParams) -> Result<Value> {
    let store = open(db, start)?;
    let records = facade::memories(&store, &MemoryQuery::default())?;
    Ok(serde_json::to_value(&records).expect("records serialize"))
}

/// `memory_search`: active memories matching a case-insensitive substring over key + body. Canonical
/// record: an array of memory records. Mirrors `nxm memories <query> --json`.
pub(super) fn search(db: Option<&str>, start: &Path, p: &MemorySearchParams) -> Result<Value> {
    let store = open(db, start)?;
    let records = facade::memories(&store, &MemoryQuery::search(&p.query))?;
    Ok(serde_json::to_value(&records).expect("records serialize"))
}

/// `memory_show`: one memory's full record by key, or `not_found` (forgotten / never-remembered).
/// Mirrors `nxm recall <key> --json`.
pub(super) fn show(db: Option<&str>, start: &Path, p: &MemoryShowParams) -> Result<Value> {
    let store = open(db, start)?;
    let rec = facade::recall(&store, &p.key)?;
    Ok(record_value(&rec))
}

// ---- writes (now + actor explicit) --------------------------------------------------------------

/// `memory_add`: capture a new fact under the content-hash auto-key, optionally filing it in the
/// same write (6j6v.9a1r), returning its record. Mirrors
/// `nxm remember <text> [--category …] [--scope …] [--refs …] --json` (no `--key`).
pub(super) fn add(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &MemoryAddParams,
) -> Result<Value> {
    // Validate the reach BEFORE opening the store: a rejected call must not have touched anything.
    let class = classification(
        &p.category,
        &p.scope,
        &p.refs,
        &Some(p.introduction.clone()),
    )?;
    let mut store = open(db, start)?;
    let now = resolve_now(p.now.as_deref())?;
    let rec = facade::remember(&mut store, &now, actor, None, &p.text, &class)?;
    Ok(record_value(&rec))
}

/// `memory_update`: upsert a fact in place under an explicit key, optionally re-filing it in the
/// same write (6j6v.9a1r), returning its record. Mirrors
/// `nxm remember <text> --key <key> [--category …] [--scope …] [--refs …] --json`.
pub(super) fn update(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &MemoryUpdateParams,
) -> Result<Value> {
    let class = classification(
        &p.category,
        &p.scope,
        &p.refs,
        &Some(p.introduction.clone()),
    )?;
    let mut store = open(db, start)?;
    let now = resolve_now(p.now.as_deref())?;
    let rec = facade::remember(&mut store, &now, actor, Some(&p.key), &p.text, &class)?;
    Ok(record_value(&rec))
}

/// `memory_classify`: file an EXISTING memory — category, reach, references and/or introduction —
/// without touching its text (6j6v.9a1r, 6j6v.xbnh). Mirrors `nxm classify <key> --json`. An empty
/// classification is a `validation` error and an unknown key a `not_found`, both from the shared
/// facade guard.
pub(super) fn classify(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &MemoryClassifyParams,
) -> Result<Value> {
    let class = classification(&p.category, &p.scope, &p.refs, &p.introduction)?;
    let mut store = open(db, start)?;
    let now = resolve_now(p.now.as_deref())?;
    let rec = facade::classify(&mut store, &now, actor, &p.key, &class)?;
    Ok(record_value(&rec))
}

/// `memory_reorder`: store `keys` as the reading order — position 1, 2, … (6j6v.9a1r). Mirrors
/// `nxm reorder <key>… --json`, whose receipt is the ARRAY of records in the order just written.
/// The facade checks every key before writing any position, so a rejected sequence leaves the
/// stored order untouched.
pub(super) fn reorder(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &MemoryReorderParams,
) -> Result<Value> {
    let mut store = open(db, start)?;
    let now = resolve_now(p.now.as_deref())?;
    let records = facade::reorder(&mut store, &now, actor, &p.keys)?;
    Ok(serde_json::to_value(&records).expect("records serialize"))
}

/// `memory_close`: forget a memory by key (a reversible tombstone). The receipt is the CLI's
/// `{ ok, key }` shape — NOT the facade tombstone record — so it is byte-identical to
/// `nxm forget <key> --json`. [`facade::forget`] performs the existence check (→ `not_found`) and
/// the tombstone write; the receipt then echoes the requested key, exactly like the CLI.
pub(super) fn close(
    db: Option<&str>,
    start: &Path,
    actor: &str,
    p: &MemoryCloseParams,
) -> Result<Value> {
    let mut store = open(db, start)?;
    let now = resolve_now(p.now.as_deref())?;
    facade::forget(&mut store, &now, actor, &p.key)?;
    Ok(json!({ "ok": true, "key": p.key }))
}
