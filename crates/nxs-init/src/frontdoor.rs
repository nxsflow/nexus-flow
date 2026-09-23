//! The single front door (nexus-flow-5jz.2). Typing `nxf init` / `nxm init` on a terminal must land
//! on the SAME `nxs init` frame as `nxs init` itself, only with the entry tool PRE-SELECTED. The
//! umbrella `nxs` is the compile-time composition root (it alone links every module → the full
//! roster), so the front-door binaries do not re-implement the chooser: they **re-exec** the sibling
//! `nxs` with `init --preselect <tool>`, replacing their own process so the umbrella inherits the
//! TTY and its exit status passes through transparently.
//!
//! Only the INTERACTIVE human path is routed here ([`is_interactive`]); the non-interactive contracts
//! (`--json` / piped / the driven `--quiet` seam the umbrella itself calls) stay native and
//! byte-stable — "Nicht-interaktiv unverändert flag-gesteuert". This also makes the re-exec
//! non-recursive: `nxf init` (TTY) → `nxs init --preselect flow` → drives `nxf init --quiet` (NOT a
//! TTY front door) → flow sets itself up.

use crate::spawn;
use nxs_foundation::error::{NxfError, Result};
use std::path::Path;

/// Build the umbrella `nxs init` argv for a front-door re-exec: always `init --preselect <tool>`,
/// plus `--json` when the caller is in json mode and flow's `--plugin` pass-through when present
/// (memory passes none). Pure, so the contract is unit-tested without launching anything.
pub fn umbrella_init_args(
    preselect: &str,
    json: bool,
    plugin: Option<&str>,
    from_beads: bool,
) -> Vec<String> {
    let mut args = vec![
        "init".to_string(),
        "--preselect".to_string(),
        preselect.to_string(),
    ];
    if json {
        args.push("--json".to_string());
    }
    if let Some(p) = plugin {
        args.push("--plugin".to_string());
        args.push(p.to_string());
    }
    // Carry the beads-migration consent flag through the front door (nexus-flow-6ef.1): a
    // `nxf/nxm init --from-beads` must reach the umbrella, which owns the migration.
    if from_beads {
        args.push("--from-beads".to_string());
    }
    args
}

/// Whether a front-door `<mod> init` should be routed through the umbrella frame: only on a real
/// interactive terminal (the human single-front-door experience). The same check the umbrella's
/// chooser uses, so a re-exec only happens when `nxs init` would actually prompt — never for
/// `--json`/piped/CI, which stay native + byte-stable.
pub fn is_interactive() -> bool {
    nxs_ui::chooser::is_interactive()
}

/// Re-exec the sibling `nxs` umbrella with `init --preselect <tool> …`, REPLACING this process so
/// the umbrella inherits the terminal and its exit status passes through transparently. `nxs` is
/// resolved as a sibling of the running binary (an installed suite / the cargo target dir), else by
/// name on `PATH`. It returns only on failure to launch (e.g. the umbrella is not installed), which
/// is a loud error — the front door never silently falls back to a different experience.
pub fn reexec_umbrella_init(
    preselect: &str,
    json: bool,
    plugin: Option<&str>,
    from_beads: bool,
) -> Result<()> {
    let nxs = spawn::resolve_binary("nxs");
    let args = umbrella_init_args(preselect, json, plugin, from_beads);
    exec_replacing(&nxs, &args)
}

/// The umbrella `nxs setup claude` argv for a front-door re-exec (nexus-flow-5od): always `setup
/// claude`, plus the global `--json` when the caller is in json mode. Pure, so the delegation
/// contract is unit-tested without launching anything.
pub fn umbrella_setup_claude_args(json: bool) -> Vec<String> {
    let mut args = vec!["setup".to_string(), "claude".to_string()];
    if json {
        args.push("--json".to_string());
    }
    args
}

/// Re-exec the sibling `nxs` umbrella with `setup claude [--json]`, REPLACING this process
/// (nexus-flow-5od). Host integration is an umbrella responsibility (the umbrella owns the whole
/// SessionStart set — one hook per active module since nxf n2m6 + a2a1, so no single module can
/// wire it alone),
/// so `nxf setup claude` — and prospectively `nxm setup claude` — carry NO own wiring path: they
/// delegate to the ONE canonical `nxs setup claude`, mirroring the `init` front door. Returns only
/// on a launch failure (the umbrella is not installed), which is a loud error.
pub fn reexec_umbrella_setup_claude(json: bool) -> Result<()> {
    let nxs = spawn::resolve_binary("nxs");
    let args = umbrella_setup_claude_args(json);
    exec_replacing(&nxs, &args)
}

/// Replace the current process with `<bin> <args…>` (unix `exec`): on success the image is replaced
/// and this never returns; on failure it returns a loud error naming the umbrella.
#[cfg(unix)]
fn exec_replacing(bin: &Path, args: &[String]) -> Result<()> {
    use std::os::unix::process::CommandExt;
    // `exec` only RETURNS when it fails to launch — a successful exec replaces this process image.
    let err = std::process::Command::new(bin).args(args).exec();
    let argv = args.join(" ");
    Err(NxfError::io(format!(
        "could not launch the nxs umbrella (`{} {argv}`): {err}. The single front door for this \
         verb needs the `nxs` binary alongside this one — install the suite, or run `nxs {argv}` \
         directly.",
        bin.display(),
    )))
}

/// Non-unix fallback: spawn the umbrella, wait, and exit with its status code (no in-place `exec`).
#[cfg(not(unix))]
fn exec_replacing(bin: &Path, args: &[String]) -> Result<()> {
    let status = std::process::Command::new(bin)
        .args(args)
        .status()
        .map_err(|e| {
            let argv = args.join(" ");
            NxfError::io(format!(
                "could not launch the nxs umbrella (`{} {argv}`): {e}. The single front door for \
                 this verb needs the `nxs` binary alongside this one — install the suite, or run \
                 `nxs {argv}` directly.",
                bin.display(),
            ))
        })?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_front_door_carries_the_preselected_tool() {
        assert_eq!(
            umbrella_init_args("flow", false, None, false),
            vec!["init", "--preselect", "flow"]
        );
    }

    #[test]
    fn json_and_plugin_pass_through_in_order() {
        // `nxf init --json --plugin issue-tracker` → the umbrella gets both, after the preselect.
        assert_eq!(
            umbrella_init_args("flow", true, Some("issue-tracker"), false),
            vec![
                "init",
                "--preselect",
                "flow",
                "--json",
                "--plugin",
                "issue-tracker"
            ]
        );
    }

    #[test]
    fn from_beads_passes_through_after_the_other_flags() {
        // `nxf init --from-beads` (the beads-migration consent) must reach the umbrella.
        assert_eq!(
            umbrella_init_args("flow", true, None, true),
            vec!["init", "--preselect", "flow", "--json", "--from-beads"]
        );
        assert_eq!(
            umbrella_init_args("memory", false, None, true),
            vec!["init", "--preselect", "memory", "--from-beads"]
        );
    }

    #[test]
    fn memory_front_door_carries_memory_and_no_plugin() {
        // memory takes no plugin; `nxm init` re-execs with just the preselect (+ --json if set).
        assert_eq!(
            umbrella_init_args("memory", false, None, false),
            vec!["init", "--preselect", "memory"]
        );
        assert_eq!(
            umbrella_init_args("memory", true, None, false),
            vec!["init", "--preselect", "memory", "--json"]
        );
    }

    #[test]
    fn setup_claude_front_door_re_execs_nxs_setup_claude() {
        // nexus-flow-5od: `nxf setup claude` delegates to the umbrella `nxs setup claude`; `--json`
        // (the nxs global flag) rides along only when the caller is in json mode.
        assert_eq!(
            umbrella_setup_claude_args(false),
            vec!["setup", "claude"],
            "human mode: no --json"
        );
        assert_eq!(
            umbrella_setup_claude_args(true),
            vec!["setup", "claude", "--json"],
            "json mode carries the global --json to the umbrella"
        );
    }
}
