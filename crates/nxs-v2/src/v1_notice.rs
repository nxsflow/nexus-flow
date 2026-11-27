//! The notice for an unmoved v1 workspace (RFC 0008, "The installed base"). Upgrading the binary
//! moves no workspace (NEX-22): in a repository whose `.nxs/` has no marker, `nxs` works on the
//! repository space as RFC 0001 describes and says on standard error, never in its output, that
//! the v1 workspace has not moved and that `nxs migrate --check` shows what the move would do.
//!
//! The repository is found as RFC 0001 finds it: the nearest folder, the working one or one of
//! its ancestors, that holds `nexus-flow.toml`, and the working folder where none does, which is
//! the folder `nxs init` makes the root.

use std::path::Path;

/// v1's workspace folder at the repository's root.
const V1_WORKSPACE: &str = ".nxs";
/// The marker `nxs migrate` writes into a moved workspace (RFC 0008, "The marker").
const MARKER: &str = "migrated.toml";
/// The file whose folder is the repository's root (RFC 0001).
const CONFIG: &str = "nexus-flow.toml";

/// The line `nxs` adds to standard error when it runs in `dir`, or `None` when the repository
/// holds no v1 workspace or a moved one.
pub fn check(dir: &Path) -> Option<String> {
    let root = dir
        .ancestors()
        .find(|folder| folder.join(CONFIG).is_file())
        .unwrap_or(dir);
    let workspace = root.join(V1_WORKSPACE);
    if !workspace.is_dir() || workspace.join(MARKER).symlink_metadata().is_ok() {
        return None;
    }
    Some(format!(
        "note: the v1 workspace in {V1_WORKSPACE}/ has not moved into the repository space; nxs migrate --check shows what the move would do\n"
    ))
}
