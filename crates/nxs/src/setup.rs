//! `nxs setup claude` — the canonical host-integration verb (nexus-flow-5od).
//!
//! Host setup is an UMBRELLA responsibility: the wired SessionStart set is one hook per ACTIVE
//! module (nxf n2m6 + a2a1; it was a single `nxs prime` hook before), which no single module can
//! know on its own, so `setup claude` belongs on the `nxs` persona, not the flow one — analogous
//! to `nxf init` re-exec'ing `nxs init`. The flow persona keeps a working `nxf setup claude`, but it
//! now DELEGATES here (via `nxs_init::frontdoor::reexec_umbrella_setup_claude`), so there is exactly
//! ONE implementation of the verb.
//!
//! What it does: resolve the workspace, then (re)assemble the shared agent file + wire one
//! SessionStart hook per active module and the allowlist, via the ONE shared seam
//! [`nxs_init::assembler::assemble`]. Because `assemble()` regenerates AGENTS.md as well as wiring
//! the hook, this is also the repair path that brings a DELETED agent file back (nexus-flow-6c4) —
//! not just the hook. It also carries a workspace across nxf 6j6v.q6e3, by taking the retired
//! managed block back OUT of `CLAUDE.md`; that file is only ever cleaned here, never written.
//! Everything is idempotent + merge-only + atomic.

use crate::assembler::{self, AgentsAction, ClaudeAction};
use crate::error::{NxfError, Result};
use crate::registry;
use nxs_foundation::workspace;

/// `nxs setup claude`: (re)wire Claude Code integration for the current workspace — the shared agent
/// file AND one SessionStart hook per active module + allowlist — idempotently and without
/// clobbering existing settings.
///
/// The settings + agent files land at the PROJECT ROOT — the directory holding `.nxs/`, found by
/// walking up from the cwd exactly as every other command resolves its workspace. So running it from
/// a subdirectory still writes at the project root, and outside a workspace it fails loudly
/// (`no_workspace`) rather than scattering a `.claude/` in the wrong place.
pub fn claude(json: bool) -> Result<()> {
    let cwd =
        std::env::current_dir().map_err(|e| NxfError::io(format!("reading current dir: {e}")))?;
    let ws = workspace::discover(&cwd)?;
    // `ws.dir` is the `.nxs` directory; its parent is the project root the agent's `.claude/` lives in.
    let root = ws.dir.parent().unwrap_or(ws.dir.as_path());

    // The umbrella links every module, so resolve ALL active tools (a loud error on an unknown one,
    // exactly as the `prime` fan-out does) and assemble: agent files + one SessionStart hook per
    // active module +
    // the `nxs`/per-module allowlist. `assemble()` is idempotent, so a re-run that needs no change
    // leaves everything byte-stable; a run after a deleted AGENTS.md regenerates it (nexus-flow-6c4).
    let roster = registry::roster();
    let modules = registry::resolve_active(&roster, &ws.config)?;
    let manifests = assembler::collect_manifests(&modules)?;
    let report = assembler::assemble(root, &manifests)?;

    let settings_path = root.join(".claude").join("settings.json");
    let permission_added = !report.permissions_added.is_empty();
    // `assemble()` reports the agent file as `Created` only when it did not exist — i.e. it was
    // missing and this run brought it back. Together with a newly-wired hook, a newly-added
    // permission, and a CLAUDE.md that actually changed on disk (`claude_changed` — since nxf
    // 6j6v.q6e3 that means a retired managed block was taken back OUT of it, or the file went with
    // it), these are the ways this run made a change; none ⇒ a clean, idempotent no-op.
    let agents_created = report.agents == AgentsAction::Created;
    let changed = report.hook_added || permission_added || agents_created || report.claude_changed;

    if json {
        let out = serde_json::json!({
            "ok": true,
            "settings": settings_path.display().to_string(),
            "hook_added": report.hook_added,
            // Additive (nxf n2m6 + a2a1): WHAT was wired, now that it is one entry per active
            // module rather than a constant. Unlike the four module init paths this verb never
            // carried a singular `command` key, so there is nothing to rename here — adding the
            // list additively keeps every existing `hook_added` consumer working.
            "hook_commands": report.hook_commands,
            "permission_added": permission_added,
            // Additive, deterministic detail on what the assemble did to the agent files.
            "agents": report.agents.as_str(),
            "claude": report.claude.as_str(),
            "claude_changed": report.claude_changed,
            "modules": report.modules,
        });
        println!("{out}");
    } else if changed {
        println!(
            "wired Claude Code integration into {}",
            settings_path.display()
        );
        if agents_created {
            println!("  + regenerated AGENTS.md");
        }
        // Which of the two removals happened, not merely THAT one did (review of PR #413,
        // Integrity #1). `claude_changed` is true for both, and calling a deleted file "updated"
        // told a user their CLAUDE.md was still there — the false map this verb exists to end, in
        // the one output a person actually reads. `describe_onboarding` in the module init paths
        // already distinguishes them; this is the same distinction, in the umbrella's own words.
        match report.claude {
            ClaudeAction::BlockRemoved => println!("  + removed the retired block from CLAUDE.md"),
            ClaudeAction::FileRemoved => println!("  + removed CLAUDE.md, which held nothing else"),
            ClaudeAction::Absent | ClaudeAction::Untouched => {}
        }
        if report.hook_added {
            // Rendered by the assembler, not spelled out here: the wired set is one entry per
            // active module since nxf n2m6 + a2a1, so there is a LIST to render rather than a
            // constant to paste — and one renderer beats five call sites drifting apart.
            println!(
                "  + {}",
                assembler::describe_hooks(report.hook_added, &report.hook_commands)
            );
        }
        if permission_added {
            println!("  + permission allowlist: nxs + module commands");
        }
    } else {
        println!(
            "Claude Code integration already configured in {} (no changes)",
            settings_path.display()
        );
    }
    Ok(())
}
