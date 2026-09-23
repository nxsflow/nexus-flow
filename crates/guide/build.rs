//! Build script for `nxs-guide`.
//!
//! Same reason every product crate that embeds guides needs one (nexus-flow-e1qn, 6j6v.9e3r):
//! `include_dir!` expands to `include_bytes!`, which rustc records in dep-info — so EDITING an
//! embedded file rebuilds, but ADDING or REMOVING one in the directory does not. Directory-level
//! tracking is `include_dir`'s `nightly` feature and we build on stable, so the directories are
//! declared here instead. Here the embedded trees are this crate's own test fixtures: the filled
//! one and the deliberately EMPTY one that pins the zero-topics shape.
fn main() {
    println!("cargo:rerun-if-changed=tests/fixture/en");
    println!("cargo:rerun-if-changed=tests/fixture/empty");
}
