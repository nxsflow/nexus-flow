//! A minimal but real Tauri v2 host that embeds [`nexus_flow_facade::engine::Engine`] directly —
//! no `nxf` subprocess. It holds ONE long-lived handle in managed state, renders `next`/`blocked`
//! in-process, and re-renders **live** whenever an external writer (`nxf`, MCP, E4 sync) commits:
//! the watcher's [`Change`](nexus_flow_facade::watch::Change) is forwarded to the webview as a
//! `nxf-change` event, on which the frontend re-queries. Run it inside a `.nxs` workspace
//! and drive `nxf create/dep/close` in another terminal to watch the lists move by themselves.
//!
//! This is the GUI proof; its deterministic, headless twin is `crates/cli/tests/embedding.rs`.
//! Neither adds semantics — the derivation shown is exactly the CLI's, read through the facade.

use nexus_flow_core::model::ItemRow;
use nexus_flow_facade::engine::Engine;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{Emitter, Manager};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// A render projection of an item for the demo lists — the canonical record carries more; ready /
/// blocked rows only need these fields. A projection, not new semantics.
#[derive(Serialize)]
struct ItemView {
    id: String,
    title: String,
    item_type: String,
    status: String,
    priority: String,
}

impl From<&ItemRow> for ItemView {
    fn from(r: &ItemRow) -> Self {
        ItemView {
            id: r.id.clone(),
            title: r.title.clone().unwrap_or_default(),
            item_type: r.item_type.clone().unwrap_or_default(),
            status: r.status.clone().unwrap_or_default(),
            priority: r.priority.clone().unwrap_or_default(),
        }
    }
}

#[derive(Serialize)]
struct BlockedView {
    id: String,
    title: String,
    /// Each open blocker as `"<id> (<status>)"`, mirroring the facade's `(blocker_id, status)`.
    blockers: Vec<String>,
}

/// The host's current instant as RFC3339 — the reference time the engine derives ready / defer
/// splits from. A real host passes its system clock; so does the demo.
fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// The `next` set — the actionable candidates, read in-process through the long-lived handle.
///
/// **It is `ready ∪ in_progress`, and the name here says so.** This command used to be called
/// `ready` and pass a second argument that kept claimed work out; the seam dropped that argument
/// and made inclusion unconditional. Fixing only the CALL would have left a window headed
/// *Ready* listing work somebody had already claimed — a label the engine stopped backing.
#[tauri::command]
fn next(engine: tauri::State<'_, Engine>) -> Result<Vec<ItemView>, String> {
    let rows = engine.next(&now_rfc3339()).map_err(|e| e.to_string())?;
    Ok(rows.iter().map(ItemView::from).collect())
}

/// The `blocked` set with each item's open blockers, read in-process through the handle.
#[tauri::command]
fn blocked(engine: tauri::State<'_, Engine>) -> Result<Vec<BlockedView>, String> {
    let rows = engine.blocked().map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|b| BlockedView {
            id: b.item.id.clone(),
            title: b.item.title.clone().unwrap_or_default(),
            blockers: b
                .blockers
                .iter()
                .map(|(id, status)| format!("{id} ({status})"))
                .collect(),
        })
        .collect())
}

/// Open the workspace the way the CLI does: an explicit `NXF_DB` override wins, otherwise discover
/// upward from the current directory. Run the app from inside a `.nxs` workspace.
fn open_engine() -> Engine {
    let db = std::env::var("NXF_DB").ok();
    let start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    Engine::open(db.as_deref(), &start)
        .expect("open a nexus-flow workspace (run inside one, or set NXF_DB to its db.sqlite)")
}

pub fn run() {
    tauri::Builder::default()
        .manage(open_engine())
        .setup(|app| {
            // Forward every external-write Change to the webview as `nxf-change`. The handle lives
            // in managed state for the app's lifetime, so the watcher (and this receiver) stay
            // alive; the loop ends only when the channel disconnects at process shutdown.
            let engine = app.state::<Engine>().inner().clone();
            let handle = app.handle().clone();
            let rx = engine
                .subscribe()
                .expect("subscribe to the change watcher");
            std::thread::spawn(move || {
                for change in rx {
                    let _ = handle.emit("nxf-change", change.data_version);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![next, blocked])
        .run(tauri::generate_context!())
        .expect("error while running the tauri-board example");
}
