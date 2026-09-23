//! Build script for `nexus-memory`.
//!
//! One job: track `docs/guide/en` — the guide docs `src/guide.rs` embeds via `include_dir!`
//! (nexus-flow-e1qn, 6j6v.9e3r). Editing an existing `.md` is *already* tracked: the macro expands
//! to `include_bytes!`, which rustc records in dep-info, so a content edit rebuilds. What is NOT
//! tracked is ADDING or REMOVING a file in the directory — that needs directory-level tracking,
//! which `include_dir` only does behind its `nightly` feature (we build on stable). This line makes
//! the whole embedded directory a first-class build input, so `cargo build` picks up an added or
//! removed topic too. That mattered most while the directory was EMPTY: the five topics
//! `6j6v.h4k0` wrote were ADDs, precisely the case dep-info cannot see, and the next added or
//! retired topic is the same case.
//!
//! Only `en` is embedded; the `de` tree is website-only and compiles into nothing, so it is
//! deliberately NOT tracked here — watching it would force rebuilds that change no byte of the
//! binary.
fn main() {
    println!("cargo:rerun-if-changed=docs/guide/en");
}
