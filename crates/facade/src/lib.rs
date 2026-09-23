//! nexus-flow-facade — the shared engine seam between the meaning-free core and its
//! consumers (the `nxf` CLI, the MCP server, and embedding apps like the Tauri desktop).
//!
//! The facade is a strict **compute→render split** (E5 #9t7.1 / E9 #49g): the read paths
//! (`prime`/`blocked`/`next`/`show`/`list`) compute the *canonical record as a value*
//! here — no `println!`, no clap, no presentation. Each consumer renders over it: the CLI
//! serializes the record under `--json` (byte-identical to before) or draws the plugin's human
//! view; an app reads the typed records directly.
//!
//! It is deliberately dependency-light — `core` + serde + toml + ulid, nothing more. No clap,
//! no tokio, no rmcp — so an embedding app links the engine without dragging in the CLI.
//!
//! - [`error`]: the structured error envelope shared by every consumer.
//! - [`manifest`]: the foundation agent-file Manifest contract (spec §6.2), re-exported so the
//!   CLI emits its declared contribution without a separate foundation dependency edge.
//! - [`plugin`]: the declarative plugin config (vocabulary, ranking, presentation).
//! - [`workspace`]: on-disk `.nexusflow/` resolution + store opening.
//! - [`record`]: the canonical, presentation-independent JSON record.
//! - [`read`]: the read compute layer — typed records + their canonical `to_value()`.
//! - [`write`]: the write compute layer — validate→mint→record, the shared mutation seam.
//! - [`validate`]: shared input validation (dates) for the write paths.
//! - [`engine`]: a long-lived, `Send + Sync` in-process handle owning workspace + store.
//! - [`watch`]: file-based change notification (`PRAGMA data_version` poll, no daemon).
//! - [`ThreadLink`] / [`LinkRelation`] / [`LinkWeight`]: the thread↔item link types, owned by the
//!   core and re-exported so an embedding app can NAME what [`engine`] hands it.

/// The thread↔item link types (nxf 6j6v.8dbe), owned by the core and re-exported here.
///
/// They ride [`engine::Engine`]'s thread-link surface (6j6v.72zn), and an embedding consumer links
/// `nexus-flow-facade` alone — never `nexus-flow-core`. Without this they would appear in the
/// public signature without being writable in the caller's own, which is the same as not being
/// there: a bridge cannot declare `fn links(…) -> Vec<ThreadLink>` for a type it cannot name.
pub use nexus_flow_core::model::{LinkRelation, LinkWeight, ThreadLink};
pub mod engine;
pub mod error;
/// The agent-file Manifest contract (spec §6.2), owned by the foundation and re-exported here.
pub use nxs_foundation::manifest;
pub mod matrix;
pub mod plugin;
pub mod read;
pub mod record;
pub mod validate;
pub mod watch;
pub mod workspace;
pub mod write;
