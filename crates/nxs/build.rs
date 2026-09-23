//! Multicall persona symlinks (nexus-flow-5jz.7). After the collapse, `nxf`/`nxm` are no longer
//! their own bin targets — they are SYMLINKS to the single `nxs` binary, which routes by `argv[0]`.
//!
//! Cargo only builds `nxs`, so this build script creates `nxf`/`nxm` symlinks next to it in the
//! profile's artifact dir. That makes every entry point work in-tree, and — crucially — lets the
//! test harness keep resolving `nxf`/`nxm` by name: both `assert_cmd` and `trycmd`/`snapbox` fall
//! back to `target/<profile>/<name>` when `CARGO_BIN_EXE_<name>` is unset, so a symlink there is
//! found and executed with `argv[0]` basename `nxf`/`nxm` (preserving the multicall persona).
//!
//! The symlink is RELATIVE (`nxf -> nxs`), so it stays valid even though `nxs` is linked AFTER this
//! script runs, and survives the artifact dir being moved. On a non-unix host it is best-effort.
//!
//! `nexus-flow` (nxf 6j6v.8see) is the fifth name and the odd one out: what SHIPS is not a link in
//! the install directory but one the service install writes into `~/.nexusflow/bin`, because its
//! whole job is to give the macOS background item a NAME (`ServiceHome::program` — macOS reads a
//! process's displayed name off the path it `exec`s). It is linked here for the same reason as the
//! other four: so the persona is reachable in-tree and the routing test drives the real `argv[0]`
//! path rather than a synthesized one.

use std::path::{Path, PathBuf};

fn main() {
    // The umbrella's own embedded guide tree (6j6v.0fvt). `include_dir!` expands to
    // `include_bytes!`, which rustc records — but an ADDED or REMOVED topic is invisible to
    // dep-info, so the directory is declared here exactly as chat's build.rs declares its own.
    println!("cargo:rerun-if-changed=docs/guide/en");
    // The develop tree is a SECOND embedded tree in this same crate (6j6v.jepw): architecture and
    // the material for people building on nexus-flow rather than using it. Same dep-info blind
    // spot, so it needs its own declaration — one per `include_dir!`, not one per crate.
    println!("cargo:rerun-if-changed=docs/develop/en");
    let Some(artifact_dir) = artifact_dir() else {
        return;
    };
    let nxs = format!("nxs{}", std::env::consts::EXE_SUFFIX);
    for persona in ["nxf", "nxm", "nxc", "nexus-flow"] {
        let link = artifact_dir.join(format!("{persona}{}", std::env::consts::EXE_SUFFIX));
        let _ = link_persona(&nxs, &link);
    }
}

/// `target/<profile>` (where the binaries land), derived from `OUT_DIR`
/// (`target/<profile>/build/<pkg>-<hash>/out` → up three: `out` → `<pkg>-<hash>` → `build`).
fn artifact_dir() -> Option<PathBuf> {
    let out = std::env::var_os("OUT_DIR")?;
    Path::new(&out).ancestors().nth(3).map(Path::to_path_buf)
}

/// Point `link` at `target` as a relative symlink, replacing whatever is there first — including a
/// stale REAL `nxf`/`nxm` binary left over from before the collapse (cargo does not clean up a
/// removed bin target's artifact).
#[cfg(unix)]
fn link_persona(target: &str, link: &Path) -> std::io::Result<()> {
    if link.exists() || link.is_symlink() {
        std::fs::remove_file(link)?;
    }
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn link_persona(target: &str, link: &Path) -> std::io::Result<()> {
    if link.exists() || link.is_symlink() {
        std::fs::remove_file(link)?;
    }
    std::os::windows::fs::symlink_file(target, link)
}
