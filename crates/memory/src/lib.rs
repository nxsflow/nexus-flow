//! nexus-memory — the `fact` product over the shared nxs-foundation substrate (spec
//! `docs/specs/nxs-memory-datamodel.md`).
//!
//! memory is the same substrate as flow in a different vocabulary: append-only op-log →
//! materialized views → keep-if-beats LWW + tombstones over SQLite. flow folds the `task` domain;
//! memory folds the **`fact`** domain into one `memories` view of keyed text facts. The
//! [`FactReducer`](fact_reducer::FactReducer) is the clean data boundary (spec §4.2/§4.4): it knows
//! no flow vocabulary, and the foundation knows no memory vocabulary.
//!
//! - [`model`]: the `fact` op vocabulary (domain/kind/field constants) + the `MemoryRow` view shape.
//! - [`key`]: deterministic content-hash auto-keys (`f-`+sha256), the bd-parity dedup lever.
//! - [`schema`]: the `memories` materialized view DDL.
//! - [`fact_reducer`]: folds `fact` ops into `memories` (one keep-if-beats LWW register per key).
//! - [`store`]: memory's store over the substrate — remember/recall/memories/forget + revive.
//! - [`facade`]: the compute→render seam (E5m/#3rx.1) — each verb returns the canonical
//!   [`MemoryRecord`](facade::MemoryRecord) as a value (`now`/`actor` explicit); the CLI, the
//!   in-process engine, and the MCP server all render over it. No new semantics over the store.
//! - [`engine`]: the long-lived in-process handle (E5m/#3rx.2) an embedding app holds for its
//!   lifetime — workspace + store behind the foundation handle, reads/writes over the facade.
//! - [`watch`]: the consumer-facing change-notification re-export ([`Change`](watch::Change)).
//! - [`cli`]: the `nxm` command tree + verb bodies (memory verbs + the init/agent-manifest/prime
//!   contract, parity with nxf — spec §5).
//! - [`project_doc`]: `NEXUS_MEMORY.md` as a BUILD PRODUCT (6j6v.8q88) — the project-memory
//!   document projected out of the store on every write, with the drift check behind `nxm doc`.
//! - [`migration`]: the JUDGING migration (6j6v.9yaj) — the explicitly invoked command that files
//!   the existing memories, moves the hand-written context document into them section by section,
//!   and thereby completes the handover to the projection. The one place a model may be asked.
//! - [`onboarding`]: nxm's DECLARED agent-file contribution (the managed AGENTS.md block
//!   with the "store knowledge only via `nxm remember`" directive). `nxm init` no longer writes the
//!   shared files or the hook itself — since P3-S5 it delegates to the ONE assembler in `nxs`.

pub mod cli;
pub mod engine;
pub mod error;
pub mod facade;
pub mod fact_reducer;
/// memory's half of the ONE embedded-guide mechanism (6j6v.9e3r): the `include_dir!` embed + the
/// ordered topic list `nxm guide` serves and `nxs guide` fans out over.
pub mod guide;
pub mod import;
pub mod key;
pub mod migration;
pub mod model;
pub mod onboarding;
pub mod project_doc;
pub mod schema;
pub mod store;
pub mod watch;
pub mod workspace;

pub use cli::{run, run_from};
