//! Build script for `nxf`.
//!
//! Two rebuild triggers, both for inputs cargo would otherwise miss:
//!
//! 1. `NXF_MINISIGN_PUBKEY` (nexus-flow-85y.13): the release pipeline bakes the minisign public key
//!    into the binary via this env var; `src/signature.rs` reads it with `option_env!`. Cargo does
//!    not re-run a build on an env-var-only change, so without this the embedded key could go stale
//!    across rebuilds. The key is injected by CI, never generated here.
//!
//! 2. `docs/guide/en` — the guide docs `src/commands/guide.rs` embeds via `include_dir!`
//!    (nexus-flow-e1qn). Editing an existing `.md` is *already* tracked: the macro expands to
//!    `include_bytes!`, which rustc records in dep-info, so a content edit rebuilds. What is NOT
//!    tracked is ADDING or REMOVING a file in the directory — that needs directory-level tracking,
//!    which `include_dir` only does behind its `nightly` feature (we build on stable). This line
//!    makes the whole embedded directory a first-class build input, so `cargo build` picks up an
//!    added or removed topic too. Only `en` is embedded; the `de` tree is website-only and compiled
//!    into nothing, so it is deliberately NOT tracked here — watching it would force rebuilds that
//!    change no byte of the binary.
fn main() {
    println!("cargo:rerun-if-env-changed=NXF_MINISIGN_PUBKEY");
    println!("cargo:rerun-if-changed=docs/guide/en");
}
