//! Build script for `nexus-chat`: compile the `nxc` agent sidecar INTO the library (nxf 6j6v.smsz).
//!
//! **Why the bundle is embedded rather than shipped beside the binary.** It used to be a second
//! tarball member that `install.sh` and `nxs self-update` placed next to `nxs` (6j6v.81v5). That
//! delivery route has a bootstrap hole it can never climb out of: the code that places the file
//! lives in the version being installed, so the update that INTRODUCES it is performed by the
//! previous version, which knows nothing about it. Every machine coming from <=0.51.0 therefore
//! landed without a sidecar — the normal case, not the exception. Embedding removes the second file
//! entirely, and with it the whole class: an update is one file replaced, and whatever `nxs` you
//! are running carries its own sidecar.
//!
//! **The bundle is an input this script does not produce.** It comes from
//! `agent-sidecar/dist/nxc-agent-sidecar.mjs`, written by `npm run build` (esbuild) in
//! `agent-sidecar/`. Building it here would put `npm ci` — network, ~286 MB of `node_modules` — in
//! the middle of `cargo build`, which no contributor asked for. So:
//!
//! - **present** → its bytes are embedded, and `worker.rs` unpacks them into the user's cache on
//!   first use.
//! - **absent** → nothing is embedded, and `worker.rs` falls back to the source-tree
//!   `agent-sidecar/src/main.mjs` exactly as it did before. That is the plain `cargo build` a
//!   contributor without Node gets, and it still runs agents from a checkout.
//!
//! **Second job: the guide directory** (nexus-flow-e1qn, 6j6v.9e3r). `src/guide.rs` embeds
//! `docs/guide/en` via `include_dir!`, which expands to `include_bytes!` — rustc records those in
//! dep-info, so EDITING a `.md` rebuilds, but ADDING or REMOVING one in the directory does not
//! (directory-level tracking is `include_dir`'s `nightly` feature and we build on stable). The
//! `rerun-if-changed` line below makes the whole embedded directory a first-class build input.
//! That matters most while the directory is EMPTY: the first topic `6j6v.t6vd` writes is an ADD,
//! precisely the case dep-info cannot see. Only `en` is embedded; the `de` tree is website-only
//! and compiles into nothing, so watching it would force rebuilds that change no byte.
//!
//! **`NXF_EMBED_SIDECAR=require` closes the hole that tolerance would otherwise open.** An
//! artifact that SHIPS must never be built from an absent bundle — that would produce an `nxs`
//! which installs cleanly and cannot start an agent, which is the bug this ticket exists for, one
//! layer down. The release workflow (and CI's gate job) set the variable, and this script then
//! fails the build instead of quietly emitting `None`.

use std::path::{Path, PathBuf};

/// Where `npm run build` in `agent-sidecar` writes the bundle, relative to the repo root. Spelled
/// the same in five other places — see `every_place_that_ships_the_sidecar_spells_it_the_same` in
/// `crates/chat/tests/worker.rs`, which counts them.
const BUNDLE: &str = "agent-sidecar/dist/nxc-agent-sidecar.mjs";

fn main() {
    let bundle = repo_root().join(BUNDLE);
    // Tracked even while it does not exist: cargo re-runs this script when the path APPEARS, so the
    // first `npm run build` in a checkout is picked up by the next `cargo build` with no clean.
    println!("cargo:rerun-if-changed={}", bundle.display());
    println!("cargo:rerun-if-env-changed=NXF_EMBED_SIDECAR");
    // The embedded guide tree (see the second job in this script's own docs).
    println!("cargo:rerun-if-changed=docs/guide/en");

    let generated = match (bundle.is_file(), embedding_is_required()) {
        (true, _) => format!(
            "pub(crate) const EMBEDDED_SIDECAR: Option<&[u8]> = Some(include_bytes!({bundle:?}));\n"
        ),
        (false, true) => panic!(
            "NXF_EMBED_SIDECAR requires the agent sidecar to be compiled in, but {} does not \
             exist. Build it first:\n    (cd agent-sidecar && npm ci && npm run build)",
            bundle.display()
        ),
        (false, false) => "pub(crate) const EMBEDDED_SIDECAR: Option<&[u8]> = None;\n".to_string(),
    };

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"))
        .join("embedded_sidecar.rs");
    std::fs::write(&out, generated).unwrap_or_else(|e| panic!("writing {}: {e}", out.display()));
}

/// Is an embedded bundle MANDATORY for this build? `NXF_EMBED_SIDECAR=require` (or the truthy
/// short forms) says so. Anything else — unset, empty, `0` — leaves the tolerant default, because
/// a contributor's `cargo build` must not depend on Node.
fn embedding_is_required() -> bool {
    matches!(
        std::env::var("NXF_EMBED_SIDECAR").as_deref(),
        Ok("require" | "1" | "true")
    )
}

/// The workspace root: `crates/chat` → up two.
fn repo_root() -> PathBuf {
    Path::new(&std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/chat has a repo root two levels up")
        .to_path_buf()
}
