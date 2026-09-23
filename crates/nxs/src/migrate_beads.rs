//! beads → nxs migration (nexus-flow-6ef), orchestrated from `nxs init`. On a project that still
//! uses beads, `nxs init` detects it, OFFERS the migration (consent-gated — never silent), and on
//! consent: backs up `bd export --all`, sets up flow (issue-tracker) + memory, imports the tickets
//! and memories, then rolls back beads' managed config and applies the nxs config.
//!
//! This module owns the orchestration + the consent gate + the report; the source shapes + the
//! detection/rebuild predicates live in [`nxs_init::beads`], the flow ticket import in
//! `nexus_flow_cli`, the memory import in `nexus_memory`. Three leitplanken thread through it:
//! consent is mandatory, only beads' MANAGED artifacts are rolled back (never hand-written prose),
//! and nothing is destroyed (the beads store is backed up, not deleted).

use crate::error::{NxfError, Result};
use nxs_init::beads::BeadsSignals;
use nxs_init::{InitRequest, ModuleInit};
use nxs_ui::Theme;
use std::path::{Path, PathBuf};

/// How consent gates the offer (6ef.1). The agent/`--json` path NEVER prompts and NEVER auto-
/// migrates: it requires the explicit `--from-beads` flag; an interactive terminal is asked; the
/// flag short-circuits the prompt (explicit consent already given).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Consent already given (the `--from-beads` flag) — run the migration without prompting.
    Proceed,
    /// Interactive terminal, no flag — ask the human to confirm.
    Prompt,
    /// Non-interactive, no flag — do nothing but tell the user how to opt in (`--from-beads`).
    RequireFlag,
}

/// Decide how to gate the migration offer from the consent inputs. Pure, so the
/// never-prompt-never-auto-migrate contract is unit-tested without a TTY.
pub fn gate(interactive: bool, from_beads: bool) -> Gate {
    if from_beads {
        Gate::Proceed
    } else if interactive {
        Gate::Prompt
    } else {
        Gate::RequireFlag
    }
}

/// The migration plan / report (6ef.1) — the dry-run preview of what WILL happen and, after a run,
/// what DID. Rendered in both human and `--json` form. The execution-result fields stay zero on a
/// dry run; later children fill them in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MigrationReport {
    /// True for the pre-flight preview (offer), false once the migration actually ran.
    pub dry_run: bool,
    /// Tickets found in the source artifact.
    pub issues: usize,
    /// Memories found in the source artifact.
    pub memories: usize,
    /// Whether the source carries memories at all (false on the tickets-only `issues.jsonl`
    /// fallback, where memory import is skipped with a note).
    pub memories_supported: bool,
    /// Where the timestamped backup is / will be written.
    pub backup_path: String,
    /// The agent files (`AGENTS.md`/`CLAUDE.md`) whose beads managed block will be / was removed.
    pub block_files: Vec<String>,
    /// Where the migrated workspace stands with the background service (nxf 6j6v.npf9).
    ///
    /// `None` on the dry-run preview, and on any run that did not reach the end: a migration is a
    /// workspace being BORN like any other, so it registers and offers exactly as `nxs init` does
    /// — but only once there is a workspace to register.
    pub service: Option<crate::background_service::Report>,
    /// Whether a `bd prime` hook is / was present to remove.
    pub remove_hook: bool,

    // ---- execution results (zero/empty on a dry run) ----
    /// Tickets actually imported into flow.
    pub imported_issues: usize,
    /// Of the imported tickets, how many were carried over closed.
    pub imported_closed: usize,
    /// Of the closed tickets, how many got the default closing reason (none recorded in beads).
    pub default_close_reasons: usize,
    pub parent_edges: usize,
    pub dep_edges: usize,
    pub mention_edges: usize,
    /// User labels carried into flow's label OR-set (h89s.4).
    pub labels: usize,
    /// Edges that could not be carried (a cycle / an endpoint not migrated) — surfaced, not fatal.
    pub skipped_edges: Vec<String>,
    /// Non-edge, non-fatal notes: a label that failed validation, or a malformed `defer_until`/`due`
    /// that was skipped rather than written (a bad date would misclassify the deferred/ready lane).
    pub warnings: Vec<String>,
    /// Memories actually imported into nxm.
    pub imported_memories: usize,
    /// Agent files whose beads block was removed by the rollback.
    pub blocks_removed: Vec<String>,
    /// Whether a `bd prime` hook was removed by the rollback.
    pub hook_removed: bool,
}

impl MigrationReport {
    /// The deterministic `--json` record. The plan fields are always present; an `executed` section
    /// is added once the migration actually ran (so the dry-run record stays a pure preview).
    pub fn to_json(&self) -> serde_json::Value {
        let mut out = serde_json::json!({
            "dry_run": self.dry_run,
            "issues": self.issues,
            "memories": self.memories,
            "memories_supported": self.memories_supported,
            "backup_path": self.backup_path,
            "config_rollback": {
                "block_files": self.block_files,
                "remove_bd_hook": self.remove_hook,
            },
        });
        if !self.dry_run {
            out["executed"] = serde_json::json!({
                "imported_issues": self.imported_issues,
                "imported_closed": self.imported_closed,
                "default_close_reasons": self.default_close_reasons,
                "parent_edges": self.parent_edges,
                "dep_edges": self.dep_edges,
                "mention_edges": self.mention_edges,
                "labels": self.labels,
                "skipped_edges": self.skipped_edges,
                "warnings": self.warnings,
                "imported_memories": self.imported_memories,
                "blocks_removed": self.blocks_removed,
                "hook_removed": self.hook_removed,
            });
            // nxf 6j6v.npf9 — the same two facts, under the same names, as `nxs init --json`'s
            // own `service` block. A migrated workspace must not be a workspace an agent has to
            // ask about differently.
            if let Some(service) = &self.service {
                out["service"] = serde_json::json!({
                    "registered": service.registered,
                    "answer": service.answer.as_ref().map(crate::background_service::Answer::as_str),
                });
            }
        }
        out
    }
}

/// Which agent files actually carry a beads managed block (so the report names exactly what the
/// rollback will touch — never claims to edit a file that has no block). Reads `AGENTS.md` then
/// `CLAUDE.md` under `root`; a missing/blockless file simply contributes nothing.
pub fn block_files(root: &std::path::Path) -> Vec<String> {
    ["AGENTS.md", "CLAUDE.md"]
        .into_iter()
        .filter(|name| {
            std::fs::read_to_string(root.join(name))
                .map(|c| nxs_init::beads::content_has_block(&c))
                .unwrap_or(false)
        })
        .map(String::from)
        .collect()
}

// ── extract + backup (6ef.2) ─────────────────────────────────────────────────

/// The directory (under `.beads/`, which the migration never deletes) where the timestamped
/// backup/import artifact lands. Keeping it inside `.beads/` groups it with the data it backs up;
/// it is NOT `issues.jsonl`, so it never re-triggers detection on a later `nxs init`.
pub const BACKUP_DIR: &str = ".beads/migration-backups";

/// The migration's source = backup: one JSONL artifact and whether it carries memories. `bd export
/// --all` carries tickets AND memories; the `.beads/issues.jsonl` fallback (used only when `bd` is
/// not installed) carries tickets ONLY, so memory import is skipped with a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The raw JSONL artifact bytes — written to the backup VERBATIM (byte-faithful: "source =
    /// backup, never lossy"); parsing lossy-converts a copy via [`nxs_init::beads::parse_export`].
    pub bytes: Vec<u8>,
    /// True for `bd export` (tickets + memories); false for the tickets-only fallback.
    pub memories_supported: bool,
    /// True when this came from the `.beads/issues.jsonl` fallback (bd not installed).
    pub via_fallback: bool,
}

impl Source {
    /// The artifact as text for PARSING (lossy — JSON is UTF-8, so this only differs on a malformed
    /// non-UTF-8 export; the backup keeps the original bytes regardless).
    pub fn text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.bytes)
    }
}

/// Choose the migration source from the captured `bd export` (preferred) and the `issues.jsonl`
/// fallback. `bd export` wins (tickets + memories); else the fallback (tickets only); else a loud
/// error — there is nothing to migrate from. Pure, so the precedence is unit-tested without `bd`.
pub fn select_source(bd_export: Option<Vec<u8>>, fallback: Option<Vec<u8>>) -> Result<Source> {
    match (bd_export, fallback) {
        (Some(bytes), _) => Ok(Source {
            bytes,
            memories_supported: true,
            via_fallback: false,
        }),
        (None, Some(bytes)) => Ok(Source {
            bytes,
            memories_supported: false,
            via_fallback: true,
        }),
        (None, None) => Err(NxfError::io(
            "no beads source to migrate from: `bd` is not installed and there is no \
             .beads/issues.jsonl export to fall back to",
        )),
    }
}

/// The timestamped backup file name for a migration run at `now` (an RFC3339-ish instant). Colons
/// are not filesystem-safe, so they become `-`; the artifact is always a `.jsonl`. Pure.
pub fn backup_filename(now: &str) -> String {
    format!("beads-export-{}.jsonl", now.replace(':', "-"))
}

/// Read the `.beads/issues.jsonl` fallback export if present (tickets only), as raw bytes. `None`
/// when absent.
pub fn read_fallback(root: &Path) -> Option<Vec<u8>> {
    std::fs::read(root.join(".beads/issues.jsonl")).ok()
}

/// Write the backup/import artifact under [`BACKUP_DIR`] (raw bytes, byte-faithful) and return its
/// path. Created before any mutation (so the beads data is captured first); the directory is made on
/// demand. Atomic write.
pub fn write_backup(root: &Path, bytes: &[u8], filename: &str) -> Result<PathBuf> {
    let dir = root.join(BACKUP_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| NxfError::io(format!("creating {}: {e}", dir.display())))?;
    let path = dir.join(filename);
    let tmp = dir.join(format!(".{filename}.nxs.tmp.{}", std::process::id()));
    std::fs::write(&tmp, bytes)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            NxfError::io(format!("writing {}: {e}", path.display()))
        })?;
    Ok(path)
}

/// Resource ceilings for the `bd export` capture (Integrity hardening): generous — a large board can
/// be many MB and a Dolt export takes a few seconds — but BOUNDED, so a hung or runaway `bd` can
/// never wedge `nxs init` nor OOM it by buffering unbounded stdout. 120s wall-clock, 256 MiB stdout.
fn export_limits() -> nxs_init::spawn::Limits {
    nxs_init::spawn::Limits {
        wall_clock: std::time::Duration::from_secs(120),
        max_stdout: 256 * 1024 * 1024,
    }
}

/// Run `bd export --all`, returning its raw JSONL stdout bytes. `Ok(None)` ONLY when `bd` is not
/// installed (the caller then falls back to `.beads/issues.jsonl`); a `bd` that runs but FAILS is a
/// loud error — never a silent fallback to a possibly-stale export. Goes through the hardened
/// shell-out seam (timeout + stdout cap, child killed on breach) so it cannot wedge/OOM init. `bd`
/// auto-discovers `.beads` from the process cwd, which is the project root under `nxs init`. Not
/// unit-tested (it shells out to the real `bd`); exercised by an e2e with a stub `bd` + real-terminal.
pub fn capture_bd_export() -> Result<Option<Vec<u8>>> {
    nxs_init::spawn::run_capture_optional_bytes("bd", &["export", "--all"], export_limits())
}

// ── config rollback (6ef.5) ──────────────────────────────────────────────────

/// What the beads-config rollback removed (for the report).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rollback {
    /// Agent files whose beads managed block was removed.
    pub blocks_removed: Vec<String>,
    /// Whether a `bd prime` hook was removed from `.claude/settings.json`.
    pub hook_removed: bool,
}

/// Roll back beads' MANAGED config under `root`: strip the managed block from `AGENTS.md`/`CLAUDE.md`
/// and the `bd prime` hook from `.claude/settings.json`. Only marked/known artifacts are touched —
/// hand-written prose and unrelated hooks are left exactly as they are (the migration's leitplanke).
/// `.beads/` data is never touched here (the backup is the safety net). Idempotent: a re-run with
/// nothing left to remove writes nothing and reports an empty rollback.
pub fn rollback_beads_config(root: &Path) -> Result<Rollback> {
    let mut r = Rollback::default();
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let path = root.join(name);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (out, removed) = nxs_init::beads::strip_block(&content)?;
        if removed {
            write_atomic(&path, &out)?;
            r.blocks_removed.push(name.to_string());
        }
    }
    let settings_path = root.join(".claude").join("settings.json");
    if let Ok(raw) = std::fs::read_to_string(&settings_path) {
        if !raw.trim().is_empty() {
            let value: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
                NxfError::validation(format!("parsing {}: {e}", settings_path.display()))
            })?;
            let (out, removed) = nxs_init::beads::strip_hooks(value);
            if removed {
                let body = format!(
                    "{}\n",
                    serde_json::to_string_pretty(&out)
                        .map_err(|e| NxfError::io(format!("serializing settings.json: {e}")))?
                );
                write_atomic(&settings_path, &body)?;
                r.hook_removed = true;
            }
        }
    }
    Ok(r)
}

// ── orchestration: detect → offer/consent → execute (6ef.1 + 6ef.5) ──────────

/// The actor recorded on the imported ops: `NXS_ACTOR`, else `$USER`, else `nxs` — with a
/// set-but-empty variable counting as unset (invariant 1, 6j6v.xsf3; see
/// `nxs_foundation::model::resolve_author`). A migration authors ops in BULK, so a blank identity
/// here would unattribute an entire imported history at once.
fn actor() -> String {
    nxs_foundation::model::resolve_author(
        std::env::var("NXS_ACTOR").ok(),
        std::env::var("USER").ok(),
        "nxs",
    )
}

/// The migration clock (RFC3339): a pinned `NXS_NOW` for deterministic runs, else the system clock.
/// Used to backdate-fallback timestamp-less records and to stamp the backup filename.
fn resolve_now() -> Result<String> {
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;
    match std::env::var("NXS_NOW").ok().filter(|s| !s.is_empty()) {
        Some(s) => Ok(s),
        None => OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|e| NxfError::io(format!("formatting current time: {e}"))),
    }
}

/// The top of the beads→nxs migration, called from `nxs init` when beads is detected. Captures the
/// source (for the dry-run counts), gates on consent, and on consent runs [`execute`] and reports.
/// Without consent it is a pure no-op (no writes) plus a hint — the leitplanke.
pub fn run(
    roster: &[&ModuleInit],
    root: &Path,
    json: bool,
    theme: &Theme,
    from_beads: bool,
    service: Option<bool>,
    signals: BeadsSignals,
) -> Result<()> {
    let source = select_source(capture_bd_export()?, read_fallback(root))?;
    let export = nxs_init::beads::parse_export(&source.text())?;
    let now = resolve_now()?;
    let backup_name = backup_filename(&now);
    let mut report = MigrationReport {
        dry_run: true,
        issues: export.issues.len(),
        memories: if source.memories_supported {
            export.memories.len()
        } else {
            0
        },
        memories_supported: source.memories_supported,
        backup_path: format!("{BACKUP_DIR}/{backup_name}"),
        block_files: block_files(root),
        remove_hook: signals.session_hook,
        ..Default::default()
    };

    let interactive = !json && nxs_init::frontdoor::is_interactive();
    match gate(interactive, from_beads) {
        Gate::RequireFlag => {
            // Non-interactive without the flag: never migrate, never prompt — preview + how to opt in.
            if json {
                let mut v = report.to_json();
                v["migrated"] = serde_json::Value::Bool(false);
                v["hint"] = serde_json::Value::String(
                    "re-run with --from-beads to perform the migration".to_string(),
                );
                println!("{v}");
            } else {
                print!("{}", render_offer(theme, &report));
                println!(
                    "\n{}",
                    theme.muted(
                        "Not migrating: re-run `nxs init --from-beads` to confirm \
                         (non-interactive mode never migrates without it)."
                    )
                );
            }
            return Ok(());
        }
        Gate::Prompt => {
            print!("{}", render_offer(theme, &report));
            if !confirm()? {
                println!(
                    "\n{}",
                    theme.muted("Migration skipped — nothing was changed.")
                );
                return Ok(());
            }
        }
        Gate::Proceed => {}
    }

    execute(
        roster,
        root,
        theme,
        &source,
        &export,
        &now,
        &backup_name,
        &mut report,
    )?;

    // **A migration is a workspace being born** (nxf 6j6v.npf9), and this path returns from `init`
    // before its frame is ever reached — so without this the one route that sets up a workspace
    // FROM an existing project would be the one route that never told the background service about
    // it. Same call, same seam, same non-fatal contract as the normal frame's.
    let ws = nxs_foundation::workspace::discover(root)?;
    report.service = Some(crate::background_service::apply(
        &crate::background_service::Real,
        &ws,
        service,
        interactive,
    ));

    if json {
        println!("{}", report.to_json());
    } else {
        print!("{}", render_done(theme, &report));
    }
    Ok(())
}

/// The migration proper, once consent is given: back up → set up flow (issue-tracker) + memory →
/// import tickets + memories → roll back beads' managed config. Takes an already-captured
/// source/export, so the whole chain is exercisable from an e2e test without `bd`. Fills the
/// execution results into `report`.
#[allow(clippy::too_many_arguments)]
pub fn execute(
    roster: &[&ModuleInit],
    root: &Path,
    theme: &Theme,
    source: &Source,
    export: &nxs_init::beads::BeadsExport,
    now: &str,
    backup_name: &str,
    report: &mut MigrationReport,
) -> Result<()> {
    // 1. Capture the backup BEFORE any mutation (the beads data is preserved, never deleted).
    write_backup(root, &source.bytes, backup_name)?;

    // 2. Set up flow (issue-tracker pinned — the only plugin whose types fit beads) + memory, driven
    //    quietly in-process; the assembler writes the NEXUS pointer + one hook per active module.
    for m in roster {
        if m.key == "flow" || m.key == "memory" {
            drive(m, root, theme, Some("issue-tracker"))?;
        }
    }

    // 3. Import tickets → flow, then memories → nxm (memories only when the source carried them).
    let issues = nexus_flow_cli::beads_import::import_at(root, &actor(), now, &export.issues)?;
    let imported_memories = if source.memories_supported {
        nexus_memory::cli::import_beads_memories_at(root, now, &export.memories)?.len()
    } else {
        0
    };

    // 4. Roll back beads' MANAGED config (block + hook); `.beads/` data stays as the backup.
    let rollback = rollback_beads_config(root)?;

    report.dry_run = false;
    report.imported_issues = issues.created;
    report.imported_closed = issues.closed;
    report.default_close_reasons = issues.default_close_reasons;
    report.parent_edges = issues.parent_edges;
    report.dep_edges = issues.dep_edges;
    report.mention_edges = issues.mention_edges;
    report.labels = issues.labels;
    report.skipped_edges = issues.skipped_edges;
    report.warnings = issues.warnings;
    report.imported_memories = imported_memories;
    report.blocks_removed = rollback.blocks_removed;
    report.hook_removed = rollback.hook_removed;
    Ok(())
}

/// Drive one module's in-process init QUIETLY (`json: true` keeps it from prompting/printing — the
/// umbrella owns the migration's output), pinning `plugin` for a plugin-accepting module.
fn drive(m: &ModuleInit, root: &Path, theme: &Theme, plugin: Option<&str>) -> Result<()> {
    let req = InitRequest {
        root,
        plugin: if m.accepts_plugin { plugin } else { None },
        json: true,
        theme,
    };
    (m.init_fn)(&req)
}

/// Read a `y/N` confirmation from stdin (only ever called on a real terminal).
fn confirm() -> Result<bool> {
    use std::io::Write;
    print!("\nMigrate beads → nexus-flow now? [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| NxfError::io(format!("reading confirmation: {e}")))?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}

/// The human dry-run offer: what was found and what the migration will do.
fn render_offer(theme: &Theme, r: &MigrationReport) -> String {
    let memories = if r.memories_supported {
        r.memories.to_string()
    } else {
        "n/a (no `bd` on PATH — tickets-only fallback)".to_string()
    };
    let mut out = String::new();
    out.push_str(&format!(
        "\n{}\n",
        theme.heading("beads detected in this project")
    ));
    out.push_str("  nexus-flow can migrate your beads data:\n");
    out.push_str(&format!(
        "    tickets:   {}\n",
        theme.accent(&r.issues.to_string())
    ));
    out.push_str(&format!("    memories:  {}\n", theme.accent(&memories)));
    out.push_str("  What will happen:\n");
    out.push_str(&format!(
        "    • back up all beads data to {} (never deleted)\n",
        r.backup_path
    ));
    out.push_str("    • set up flow (issue-tracker) + memory and import the above\n");
    let artifacts = rollback_summary(r);
    out.push_str(&format!(
        "    • roll back beads' managed config: {artifacts}\n"
    ));
    out
}

/// The human completion summary.
fn render_done(theme: &Theme, r: &MigrationReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "\n{}\n",
        theme.heading("migrated beads → nexus-flow")
    ));
    out.push_str(&format!(
        "  imported:  {} tickets ({} closed), {} memories\n",
        r.imported_issues, r.imported_closed, r.imported_memories
    ));
    out.push_str(&format!(
        "  edges:     {} parent, {} dependency, {} reference\n",
        r.parent_edges, r.dep_edges, r.mention_edges
    ));
    if r.labels > 0 {
        out.push_str(&format!("  labels:    {} carried\n", r.labels));
    }
    if !r.skipped_edges.is_empty() {
        out.push_str(&format!(
            "  skipped:   {} edge(s) could not be carried (run with --json for details)\n",
            r.skipped_edges.len()
        ));
    }
    if !r.warnings.is_empty() {
        out.push_str(&format!(
            "  warnings:  {} field/label(s) skipped (run with --json for details)\n",
            r.warnings.len()
        ));
    }
    out.push_str(&format!("  backup:    {}\n", r.backup_path));
    let removed_from = if r.blocks_removed.is_empty() {
        "(no managed block found)".to_string()
    } else {
        r.blocks_removed.join(", ")
    };
    out.push_str(&format!(
        "  rollback:  removed beads block from {}; bd-prime hook {}\n",
        removed_from,
        if r.hook_removed {
            "removed"
        } else {
            "not present"
        }
    ));
    // nxf 6j6v.npf9: the same line the normal `init` frame carries, so a migrated workspace's
    // standing with the background service is not the one thing this path leaves unsaid.
    if let Some(service) = &r.service {
        out.push_str(&format!(
            "  service:   {}\n",
            crate::background_service::summary_line(service)
        ));
    }
    out.push_str(&format!(
        "\n{}\n",
        theme.muted(
            "Note: any bd references you hand-wrote OUTSIDE the managed block are left as-is — \
             review AGENTS.md/CLAUDE.md if needed. Your `.beads/` data is untouched (the backup)."
        )
    ));
    out
}

/// One-line summary of the config artifacts the rollback will touch (for the offer).
fn rollback_summary(r: &MigrationReport) -> String {
    let block = if r.block_files.is_empty() {
        "no managed block".to_string()
    } else {
        format!("remove the block from {}", r.block_files.join(", "))
    };
    let hook = if r.remove_hook {
        ", remove the `bd prime` hook"
    } else {
        ""
    };
    format!("{block}{hook}")
}

/// Atomic write (temp + rename), matching the assembler's writers so a rewritten file is consistent.
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("nxs");
    let tmp = dir.join(format!(".{name}.nxs.tmp.{}", std::process::id()));
    std::fs::write(&tmp, content)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            NxfError::io(format!("writing {}: {e}", path.display()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_proceeds_without_prompting_even_on_a_tty() {
        // Explicit consent (`--from-beads`) means run it — never re-prompt.
        assert_eq!(gate(true, true), Gate::Proceed);
        assert_eq!(gate(false, true), Gate::Proceed);
    }

    #[test]
    fn an_interactive_terminal_without_the_flag_is_prompted() {
        assert_eq!(gate(true, false), Gate::Prompt);
    }

    #[test]
    fn a_non_interactive_sink_without_the_flag_requires_the_flag() {
        // The agent/`--json`/piped path NEVER prompts and NEVER auto-migrates.
        assert_eq!(gate(false, false), Gate::RequireFlag);
    }

    fn sample(dry_run: bool) -> MigrationReport {
        MigrationReport {
            dry_run,
            issues: 344,
            memories: 23,
            memories_supported: true,
            backup_path: ".beads/migration-backups/beads-20260626T120000Z.jsonl".to_string(),
            block_files: vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()],
            remove_hook: true,
            ..Default::default()
        }
    }

    #[test]
    fn json_report_carries_counts_backup_and_rollback_artifacts() {
        let v = sample(true).to_json();
        assert_eq!(v["dry_run"], true);
        assert_eq!(v["issues"], 344);
        assert_eq!(v["memories"], 23);
        assert_eq!(v["memories_supported"], true);
        assert!(v["backup_path"]
            .as_str()
            .unwrap()
            .contains("migration-backups"));
        assert_eq!(v["config_rollback"]["remove_bd_hook"], true);
        let files = v["config_rollback"]["block_files"].as_array().unwrap();
        assert_eq!(files.len(), 2, "both agent files listed");
    }

    #[test]
    fn block_files_lists_only_files_that_carry_the_managed_block() {
        let tmp = tempfile::TempDir::new().unwrap();
        // AGENTS.md has the block; CLAUDE.md does not; no other file.
        std::fs::write(
            tmp.path().join("AGENTS.md"),
            "# x\n<!-- BEGIN BEADS INTEGRATION v:1 -->\nb\n<!-- END BEADS INTEGRATION -->\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("CLAUDE.md"),
            "# hand-written, no beads block\n",
        )
        .unwrap();
        assert_eq!(block_files(tmp.path()), vec!["AGENTS.md".to_string()]);
    }

    #[test]
    fn block_files_is_empty_when_no_agent_file_has_a_block() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(block_files(tmp.path()).is_empty());
    }

    // ---- config rollback (6ef.5) --------------------------------------------

    #[test]
    fn rollback_strips_blocks_and_the_hook_but_keeps_prose_and_other_hooks() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("AGENTS.md"),
            "# Agent Instructions\n\nhand-written.\n\n<!-- BEGIN BEADS INTEGRATION v:1 -->\nmanaged\n<!-- END BEADS INTEGRATION -->\n",
        )
        .unwrap();
        std::fs::write(
            root.join("CLAUDE.md"),
            "# project\n\n<!-- BEGIN BEADS INTEGRATION v:1 -->\nmanaged\n<!-- END BEADS INTEGRATION -->\n\nmy rules\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(
            root.join(".claude/settings.json"),
            r#"{ "hooks": { "SessionStart": [ { "hooks": [ { "command": "bd prime" } ] }, { "hooks": [ { "command": "nxs prime" } ] } ], "PreCompact": [ { "hooks": [ { "command": "bd prime" } ] } ] }, "model": "opus" }"#,
        )
        .unwrap();

        let r = rollback_beads_config(root).unwrap();
        assert_eq!(
            r.blocks_removed,
            vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()]
        );
        assert!(r.hook_removed);

        let agents = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert!(!agents.contains("BEGIN BEADS"), "block removed: {agents}");
        assert!(agents.contains("hand-written."), "prose kept: {agents}");
        let claude = std::fs::read_to_string(root.join("CLAUDE.md")).unwrap();
        assert!(!claude.contains("BEGIN BEADS") && claude.contains("my rules"));

        let settings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(".claude/settings.json")).unwrap(),
        )
        .unwrap();
        assert!(
            settings["hooks"].get("PreCompact").is_none(),
            "bd-only event dropped"
        );
        let session: Vec<&str> = settings["hooks"]["SessionStart"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["hooks"].as_array().unwrap())
            .filter_map(|h| h["command"].as_str())
            .collect();
        assert_eq!(session, vec!["nxs prime"], "bd prime gone, nxs prime kept");
        assert_eq!(settings["model"], "opus");

        // Idempotent: a second rollback removes nothing.
        let again = rollback_beads_config(root).unwrap();
        assert!(again.blocks_removed.is_empty() && !again.hook_removed);
    }

    // ---- extract + backup (6ef.2) -------------------------------------------

    #[test]
    fn bd_export_is_the_preferred_source_and_carries_memories() {
        let src = select_source(Some(b"{\"_type\":\"issue\"}".to_vec()), None).unwrap();
        assert!(src.memories_supported, "bd export carries memories");
        assert!(!src.via_fallback);
        assert_eq!(src.text(), "{\"_type\":\"issue\"}");
    }

    #[test]
    fn the_fallback_is_tickets_only_when_bd_is_absent() {
        // No `bd`, but a `.beads/issues.jsonl` is there → tickets-only source, memories skipped.
        let src = select_source(None, Some(b"{\"id\":\"x\"}".to_vec())).unwrap();
        assert!(!src.memories_supported, "fallback has no memories");
        assert!(src.via_fallback);
    }

    #[test]
    fn bd_export_wins_over_a_present_fallback() {
        let src = select_source(Some(b"bd".to_vec()), Some(b"fallback".to_vec())).unwrap();
        assert_eq!(src.text(), "bd");
        assert!(src.memories_supported);
    }

    #[test]
    fn the_backup_keeps_non_utf8_export_bytes_byte_faithful() {
        // Integrity: the backup is written from the raw bytes (source = backup, never lossy), so a
        // non-UTF-8 byte in the export survives in the artifact even though parsing is lossy.
        let raw = vec![b'{', 0xff, b'}'];
        let src = select_source(Some(raw.clone()), None).unwrap();
        assert_eq!(src.bytes, raw, "raw bytes preserved for the backup");
        assert!(
            src.text().contains('\u{fffd}'),
            "parsing view is lossy, the bytes are not"
        );
    }

    #[test]
    fn no_source_at_all_is_a_loud_error() {
        assert!(
            select_source(None, None).is_err(),
            "neither bd nor a fallback export → nothing to migrate from"
        );
    }

    #[test]
    fn backup_filename_is_timestamped_and_filesystem_safe() {
        let name = backup_filename("2026-06-26T12:00:00Z");
        assert!(name.ends_with(".jsonl"), "is a jsonl: {name}");
        assert!(
            !name.contains(':'),
            "colons are not filesystem-safe: {name}"
        );
        assert!(name.contains("2026-06-26"), "carries the date: {name}");
    }

    #[test]
    fn read_fallback_reads_issues_jsonl_when_present() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert_eq!(read_fallback(tmp.path()), None, "absent → None");
        std::fs::create_dir_all(tmp.path().join(".beads")).unwrap();
        std::fs::write(tmp.path().join(".beads/issues.jsonl"), "line\n").unwrap();
        assert_eq!(read_fallback(tmp.path()).as_deref(), Some(&b"line\n"[..]));
    }

    #[test]
    fn write_backup_creates_the_artifact_under_the_backup_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = write_backup(tmp.path(), b"the export\n", "beads-export-x.jsonl").unwrap();
        assert!(path.exists(), "artifact written");
        assert!(
            path.to_string_lossy().contains("migration-backups"),
            "under the backup dir: {}",
            path.display()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "the export\n");
        // The beads data dir is preserved, never deleted.
        assert!(tmp.path().join(".beads").exists());
    }
}
