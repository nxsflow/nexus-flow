//! nxs-foundation — the platform's domain-agnostic substrate + embedding API (spec §4, §5).
//!
//! One append-only op-log → a reducer registry that folds each op into its domain's materialized
//! views → keep-if-beats LWW + observed-remove OR-set + tombstones over SQLite, plus the
//! schema-version compatibility spine. On top of that substrate sits the product-agnostic embedding
//! seam: workspace resolution, the long-lived `Engine` handle (generic over the product's store),
//! the file-based change watch, and the escaping-free long-text input path every product CLI reads
//! a description, a definition of done or a message body through ([`text_input`]). Every nxs
//! product (flow/memory/chat) builds on this; the foundation knows op-log mechanics, the reducer +
//! handle seams and the contracts every product shares, never any product's vocabulary.

pub mod engine;
pub mod error;
pub mod image;
pub mod manifest;
pub mod model;
pub mod reducer;
pub mod schema;
pub mod signing;
pub mod store;
pub mod text_input;
pub mod trust;
pub mod watch;
pub mod workspace;
