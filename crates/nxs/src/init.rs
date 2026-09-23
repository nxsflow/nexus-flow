//! `nxs init` (spec §6.3, nexus-flow-aye.29) — the primary human first-encounter with the suite.
//! It picks which tools to use in this workspace and fans out to each module's own `init`, then
//! frames the whole result.
//!
//! The roster is no longer hardcoded: every module SELF-REGISTERS a [`registry::ModuleInit`] at
//! compile time (5jz.1), and `nxs init` reads its keys, blurbs, recommended default, accepted args,
//! and `init_fn` exclusively from that roster. Adding a product needs no change here.
//!
//! **The UX contract (5jz.3):** `nxs` frames the result, but each chosen module is driven IN-PROCESS
//! via its self-registered [`registry::InitFn`] — not a captured `--quiet` subprocess. So a module's
//! OWN interactive sub-config (flow's plugin chooser) runs VISIBLY inside the umbrella frame, while
//! the module prints no summary of its own (the umbrella owns the aggregate output). The old
//! "`nxs` owns the ENTIRE visible output via `--quiet`" contract is gone — this is what lets flow's
//! deliberate plugin choice appear via the umbrella path, the original #5jz problem.
//!
//! Presentation is the shared [`nxs_ui`] layer: the manufakt-Forge welcome box + the arrow-key
//! multi-select, TTY-gated (splendor only on a real terminal that is not `--json`; agents/pipes/CI
//! get byte-stable plain ASCII). The init PATH composes the modules as a library (TB-5 reversed for
//! init); `prime`/`agent-manifest` stay shell-out.

use crate::background_service;
use crate::error::{NxfError, Result};
use crate::registry::{self, ModuleInit};
use nxs_foundation::workspace;
use nxs_ui::{chooser, width, Choice, ChooseError, Theme};
use std::path::Path;

/// `nxs init`: set up the chosen modules in the current directory and frame the result.
///
/// `modules` are explicit `--module` flags (the non-interactive agent entry); when empty we either
/// prompt with the shared multi-select (a real terminal, not `--json`) or default to the
/// recommended module (any non-interactive sink). `preselect` is the entry-point pre-selection
/// (5jz.2): the front-door binaries re-exec `nxs init --preselect <tool>`, so `nxf init` lands on
/// this frame with flow pre-checked, `nxm init` with memory — the multi-select starts with those
/// ticked (the user can add/remove others), and any non-interactive sink takes them as THE
/// selection. `plugin` is an optional pass-through to a plugin-accepting module's init (flow).
///
/// `service` is the explicit background-service answer (`--service` / `--no-service`, nxf
/// 6j6v.npf9); `None` means neither flag was given, and only a real terminal is then asked. See
/// [`crate::background_service`] for why the offer belongs here at all.
pub fn run(
    json: bool,
    modules: &[String],
    preselect: &[String],
    plugin: Option<&str>,
    from_beads: bool,
    service: Option<bool>,
) -> Result<()> {
    let roster = registry::roster();
    let root =
        std::env::current_dir().map_err(|e| NxfError::io(format!("reading current dir: {e}")))?;
    let theme = Theme::detect(json);

    // beads → nxs migration (nexus-flow-6ef): if this project still uses beads, the migration owns
    // `init` — it offers (consent-gated) to import the beads tickets + memories and roll back beads'
    // managed config, then sets nxs up. Returning here keeps a beads project from ending up half
    // migrated / half freshly-initialized. A project with no beads signal falls through to the
    // normal module-selection flow below unchanged.
    //
    // Gated on there being NO nxs workspace yet (idempotence): a successful migration leaves the
    // `.beads/` data in place as the backup (and a `.beads/issues.jsonl` export can linger), so
    // without this guard a re-run would keep re-detecting beads and re-offering — worse, a re-run
    // with `--from-beads` would re-import as duplicates (fresh ids). Once `.nxs/` exists the
    // migration is done; fall through to the normal, idempotent re-run instead.
    let beads = nxs_init::beads::detect(&root);
    if beads.present() && workspace::discover(&root).is_err() {
        return crate::migrate_beads::run(&roster, &root, json, &theme, from_beads, service, beads);
    }

    // What's ALREADY active in this workspace (5jz.4), captured BEFORE driving so the chooser can
    // pre-check + mark it, the summary can say what was already there vs newly set up, and `--json`
    // can report it as a well-defined additive field — making a re-run informative, not confusing.
    let already_active = workspace::discover(&root)
        .map(|ws| ws.config.active_modules)
        .unwrap_or_default();

    let selected = resolve_selection(&roster, &theme, json, modules, preselect, &already_active)?;

    // Drive each selected module that is not already active, in roster order — flow first, since
    // `nxf init` creates the workspace (it refuses to nest) while `nxm init` joins an existing one.
    for m in &roster {
        if selected.iter().any(|s| s.key == m.key) && !already_active.iter().any(|a| a == m.key) {
            drive_module(m, &root, json, &theme, plugin)?;
        }
    }

    // Resolve the final state for the report (the driven inits registered + assembled).
    let ws = workspace::discover(&root)?;

    // **The workspace comes to the background service at its birth** (nxf 6j6v.npf9), rather than
    // only if somebody happens to bind it to a sync stream later. Registration is unconditional;
    // installing the service itself is offered on a terminal and never done under `--json`/a pipe
    // without an explicit flag. Both are follow-up to a workspace that is already set up, so
    // neither can fail this `init` — see `background_service::apply`.
    let service = background_service::apply(
        &background_service::Real,
        &ws,
        service,
        !json && chooser::is_interactive(),
    );

    // Assemble the shared agent file from the FINAL active modules, on EVERY run (nexus-flow-6c4).
    // The per-module drives above already assemble as a side effect, but a re-run where every
    // selected module was ALREADY active drives nothing — so without this an AGENTS.md deleted
    // since the first init would never come back (the "install nothing, files stay gone"
    // bug). `assemble()` is idempotent (byte-stable re-run) and also wires the single `nxs prime`
    // hook + allowlist, so running it unconditionally here is safe and is what regenerates the
    // files. The umbrella links every module, so the roster resolves ALL active tools into one
    // pointer (a lone-module self-assembly would name only what that binary links).
    let assembled = umbrella_assemble(&root, &ws.config)?;

    let active = ws.config.active_modules.clone();
    // Read back from disk rather than trusting the report: this line tells the user what the HOST
    // will actually run, and `assemble` reporting a successful write is not the same claim.
    let hook_wired = session_hook_present(&root, &assembled.hook_commands);
    // Resolve the active flow plugin's example type so the banner's first-move command is valid for
    // THIS workspace's plugin (nexus-flow-92zt). Resolved via flow's own crate (it owns the plugin
    // vocabulary); `None` when flow is not active or the plugin cannot load, and the banner then
    // renders a safe placeholder rather than a stale hardcoded type.
    let example_type = nexus_flow_cli::flow_example_type(&root);

    if json {
        let out = serde_json::json!({
            "ok": true,
            "workspace": ws.dir.display().to_string(),
            "modules": active,
            // 5jz.4: the modules that were already active BEFORE this init ran (empty on a fresh
            // workspace). Additive + deterministic, so an agent re-running `nxs init --json` can tell
            // what it set up from what was already there.
            "already_active": already_active,
            // **Superseded 2026-08-28 (nxf n2m6 + a2a1):** `command` was ONE string while the
            // assembler wired a single `nxs prime` hook. It is now one entry per active module, so
            // the field is a LIST under the name that says so — a consumer reading the old key
            // gets nothing rather than one of three silently mistaken for the whole wiring.
            "hook": { "commands": assembled.hook_commands, "wired": hook_wired },
            // nxf 6j6v.npf9. `registered` is whether this workspace is on the list the background
            // service attends; `answer` is what this workspace has recorded about installing the
            // service itself (`null` when the question has not been settled here), which is what
            // keeps a re-run from asking again.
            //
            // **Whether a service is INSTALLED on this machine is deliberately not here.** It is
            // the one fact in this frame that differs by platform — there is no installer off
            // macOS — and a `--json` record whose shape depends on the host is a record two
            // machines cannot compare. It is also not this verb's question: `nxs sync daemon
            // status` answers what the MACHINE is, and this frame reports what this workspace
            // settled. `answer` reads `installed` only where an `init` in this workspace actually
            // installed one.
            "service": {
                "registered": service.registered,
                "answer": service.answer.as_ref().map(background_service::Answer::as_str),
            },
        });
        println!("{out}");
    } else {
        print_summary(
            &theme,
            &ws.dir,
            &active,
            &StatusLines {
                hook: hook_summary(hook_wired, &assembled.hook_commands),
                service: background_service::summary_line(&service),
            },
            &roster,
            &already_active,
            example_type.as_deref(),
        );
    }
    Ok(())
}

/// Resolve which modules to set up: explicit `--module` flags win outright (the agent entry); else
/// the *default selection* — the entry-point `--preselect` set if given (the front-door re-exec
/// passes it), else the recommended module — drives an interactive multi-select on a real terminal
/// (pre-checked, user-adjustable) or is taken verbatim by any non-interactive sink.
fn resolve_selection<'a>(
    roster: &[&'a ModuleInit],
    theme: &Theme,
    json: bool,
    modules: &[String],
    preselect: &[String],
    already_active: &[String],
) -> Result<Vec<&'a ModuleInit>> {
    if !modules.is_empty() {
        return resolve_flagged(roster, modules);
    }
    let defaults = default_selection(roster, preselect)?;
    if json || !chooser::is_interactive() {
        // Non-interactive with no `--module`: take the default selection without a prompt.
        return Ok(defaults);
    }
    prompt_for_modules(roster, theme, &defaults, already_active)
}

/// The default selection when no explicit `--module` is given: the entry-point `--preselect` set
/// (validated against the roster, roster-ordered) if any was passed, else the single recommended
/// module (flow today). An unknown `--preselect` key is a loud error — never silently dropped.
fn default_selection<'a>(
    roster: &[&'a ModuleInit],
    preselect: &[String],
) -> Result<Vec<&'a ModuleInit>> {
    if preselect.is_empty() {
        Ok(vec![default_module(roster)?])
    } else {
        resolve_flagged(roster, preselect)
    }
}

/// The recommended default module (the first `recommended` in roster order), or the first rostered
/// module if none is flagged recommended. An empty roster is a loud error (nothing to set up).
fn default_module<'a>(roster: &[&'a ModuleInit]) -> Result<&'a ModuleInit> {
    roster
        .iter()
        .find(|m| m.recommended)
        .or_else(|| roster.first())
        .copied()
        .ok_or_else(|| NxfError::io("no suite modules are registered — this nxs build links none"))
}

/// Validate explicit `--module` flags against the roster, preserving roster order. An unknown
/// module is a loud error naming it (never silently dropped).
fn resolve_flagged<'a>(
    roster: &[&'a ModuleInit],
    modules: &[String],
) -> Result<Vec<&'a ModuleInit>> {
    for key in modules {
        if registry::binary_for(roster, key).is_none() {
            return Err(NxfError::validation(format!(
                "unknown module '{key}'; known modules: {}",
                registry::roster_keys(roster)
            )));
        }
    }
    Ok(roster
        .iter()
        .copied()
        .filter(|m| modules.iter().any(|k| k == m.key))
        .collect())
}

/// Interactive: print the suite welcome box, then the arrow-key multi-select with the *default
/// selection* pre-checked. The defaults are the entry-point pre-selection (5jz.2) — flow for
/// `nxf init`, memory for `nxm init`, the recommended module for a bare `nxs init` — plus whatever
/// is ALREADY active in this workspace (5jz.4): a re-run shows the already-set-up tools ticked and
/// marked, so the chooser reads informatively instead of silently re-driving them. The user can
/// tick more or untick them before confirming.
fn prompt_for_modules<'a>(
    roster: &[&'a ModuleInit],
    theme: &Theme,
    defaults: &[&ModuleInit],
    already_active: &[String],
) -> Result<Vec<&'a ModuleInit>> {
    print_welcome(theme, defaults);
    // Calm, deliberate rhythm (nexus-flow-92k): TWO blank lines below the welcome box (print_welcome
    // leaves one, this adds the second), then — when a tool registered "learn more" copy — the
    // details panel, followed by TWO more blank lines before the question. With no details panel the
    // two blank lines below the box lead straight into the question.
    println!();
    let details = tool_details_panel(theme, roster);
    if !details.is_empty() {
        println!("{details}");
        println!();
        println!();
    }
    let choices: Vec<Choice> = roster
        .iter()
        .map(|m| {
            Choice::new(
                m.key,
                format!("{}{}", m.blurb, active_suffix(m.key, already_active)),
            )
        })
        .collect();
    let default_idx = chooser_default_indices(roster, defaults, already_active);
    // The trailing newline renders as ONE blank line between the question and the options: inquire's
    // frame renderer treats an embedded `\n` as a finished line and counts it in the frame height,
    // so the cursor restore stays correct (no redraw artifacts when navigating).
    let picked = chooser::choose_many("Which tools do you want to use?\n", &choices, &default_idx)
        .map_err(|e| match e {
            ChooseError::Cancelled => NxfError::validation("init cancelled — nothing was set up"),
            other => NxfError::io(other.to_string()),
        })?;
    // A blank line after the selection so the per-module setup / summary that follows does not butt
    // up against the chooser (nexus-flow-92k).
    println!();
    modules_from_picks(roster, &picked)
}

/// The chooser/summary marker for a module that is ALREADY active in this workspace (5jz.4), or an
/// empty string for one that is not. Pure, so the re-run annotation is unit-testable without a TTY;
/// off a first run (`already_active` empty) it marks nothing, keeping the first-run view pristine.
fn active_suffix(key: &str, already_active: &[String]) -> &'static str {
    if already_active.iter().any(|a| a == key) {
        " (already set up)"
    } else {
        ""
    }
}

/// The chooser's pre-checked indices: the entry-point default selection UNION whatever is already
/// active in the workspace (5jz.4), in roster order. Already-active modules are ticked + marked so a
/// re-run reflects the ist-state; re-confirming one is a no-op (the driver skips active modules).
fn chooser_default_indices(
    roster: &[&ModuleInit],
    defaults: &[&ModuleInit],
    already_active: &[String],
) -> Vec<usize> {
    roster
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            defaults.iter().any(|d| d.key == m.key) || already_active.iter().any(|a| a == m.key)
        })
        .map(|(i, _)| i)
        .collect()
}

/// Map chooser indices back to roster modules. An empty selection (the user toggled everything off)
/// is a loud error — `nxs init` must set up at least one tool. Pure, so the empty-selection mapping
/// is unit-tested without a terminal (the interactive widget's TTY-refusal is tested in nxs-ui).
fn modules_from_picks<'a>(
    roster: &[&'a ModuleInit],
    picked: &[usize],
) -> Result<Vec<&'a ModuleInit>> {
    if picked.is_empty() {
        return Err(NxfError::validation(
            "select at least one tool to set up (space to toggle, enter to confirm)",
        ));
    }
    Ok(picked.iter().map(|&i| roster[i]).collect())
}

/// Drive one module's init IN-PROCESS (5jz.3) by calling its self-registered [`registry::InitFn`],
/// instead of shelling out to `<binary> init --quiet`. The module sets itself up inside this one
/// umbrella process and frame: when interactive, its OWN sub-config (flow's plugin chooser) runs
/// VISIBLY here (the original #5jz problem — it used to be swallowed by the captured `--quiet`
/// subprocess); the module prints no summary of its own, so the umbrella still frames everything. A
/// plugin-accepting module (flow) receives the `--plugin` pass-through; others get none. A module
/// failure (e.g. flow refusing to nest) propagates DIRECTLY — surfaced loudly, never swallowed.
fn drive_module(
    m: &ModuleInit,
    root: &Path,
    json: bool,
    theme: &Theme,
    plugin: Option<&str>,
) -> Result<()> {
    let req = registry::InitRequest {
        root,
        plugin: if m.accepts_plugin { plugin } else { None },
        json,
        theme,
    };
    (m.init_fn)(&req)
}

/// Assemble the shared agent file + wire one SessionStart hook per active module (a single
/// `nxs prime` hook until nxf n2m6 + a2a1) from the workspace's FINAL active modules — the umbrella's OWN assembly, run on every `nxs init` regardless of whether a
/// module was driven (nexus-flow-6c4). This is what regenerates a deleted AGENTS.md on a
/// re-run where every selected module was already active (so nothing was driven). Idempotent:
/// `assemble()` re-runs byte-stably and never double-wires the hook. The umbrella links every
/// module, so [`registry::resolve_active`] resolves them all — an active module the roster does not
/// know is a loud error here (as it is for the `prime` fan-out), never a silently half-assembled file.
fn umbrella_assemble(
    root: &Path,
    config: &workspace::WorkspaceConfig,
) -> Result<crate::assembler::AssembleReport> {
    let roster = registry::roster();
    let modules = registry::resolve_active(&roster, config)?;
    let manifests = crate::assembler::collect_manifests(&modules)?;
    crate::assembler::assemble(root, &manifests)
}

/// The summary line naming what the host will run at session start — one entry per active module
/// since nxf n2m6 + a2a1, so it is a list rather than the single `nxs prime` this used to name.
///
/// Not [`crate::assembler::describe_hooks`]: that one answers "what did this run CHANGE" for the
/// module init paths (`wired …` / `… already wired`), while this answers "what IS wired", read back
/// from disk, and has a "not wired" case that has no equivalent there.
fn hook_summary(wired: bool, commands: &[String]) -> String {
    if !wired {
        return "not wired".to_string();
    }
    let list: Vec<String> = commands.iter().map(|c| format!("`{c}`")).collect();
    format!("SessionStart → {}", list.join(", "))
}

/// Whether EVERY expected SessionStart hook is present in `<root>/.claude/settings.json`.
///
/// **Superseded 2026-08-28 (nxf n2m6 + a2a1):** this used to ask whether "the single `nxs prime`
/// SessionStart hook" was there. With one entry per active module a partial wiring is possible —
/// two of three present — and reporting that as wired would be the worst of the three answers, so
/// it is all-or-nothing. An empty `expected` (no active module) is vacuously true and the caller
/// renders "no active module" rather than a hook line.
fn session_hook_present(root: &Path, expected: &[String]) -> bool {
    let path = root.join(".claude").join("settings.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    let wired: Vec<&str> = v["hooks"]["SessionStart"]
        .as_array()
        .map(|groups| {
            groups
                .iter()
                .filter_map(|g| g["hooks"].as_array())
                .flatten()
                .filter_map(|h| h["command"].as_str())
                .collect()
        })
        .unwrap_or_default();
    expected.iter().all(|e| wired.contains(&e.as_str()))
}

/// The richer welcome copy of each preselected module (5jz.6), in the given order, skipping modules
/// that registered none. Pure, so the welcome composition is unit-testable without a TTY; the
/// umbrella owns NO product copy — each line travels with its module's self-registration.
fn welcome_blurbs(defaults: &[&ModuleInit]) -> Vec<&'static str> {
    defaults.iter().filter_map(|m| m.welcome).collect()
}

/// The interactive picker's "learn more" details panel (nexus-flow-huy): each rostered tool's OWN
/// self-registered `details` copy, wrapped under its name, so the user chooses from real decision-
/// help instead of a one-line blurb. A tool that registered no `details` contributes nothing; if
/// none do, the panel is empty and the caller prints it (a no-op). Pure, so the composition is
/// unit-testable without a TTY — the umbrella hardcodes NO product copy, every line travels with its
/// module. Long copy is wrapped to the content width with a hanging indent so the panel stays calm.
fn tool_details_panel(theme: &Theme, roster: &[&ModuleInit]) -> String {
    const INDENT: &str = "    ";
    let wrap_width = width::content_width().saturating_sub(INDENT.len()).max(20);
    let mut out = String::new();
    for m in roster {
        let Some(details) = m.details else { continue };
        if out.is_empty() {
            out.push_str(&theme.heading("What each tool does"));
        }
        out.push('\n');
        out.push_str(&format!("  {}", theme.emphasis(m.key)));
        for row in width::wrap(details, wrap_width) {
            out.push('\n');
            out.push_str(INDENT);
            out.push_str(&row);
        }
    }
    out
}

/// The post-init first commands (5jz.6) of the modules that ended up active, in `active` order,
/// skipping modules that registered none — each as a `(label, command)` pair. Pure, so the
/// success-moment composition is unit-testable; the command TEXT is the active module's OWN
/// (self-registered), never hardcoded in the umbrella. The umbrella only fills the `{type}` slot a
/// plugin-bearing module's template leaves (nexus-flow-92zt) with `example_type` (the active flow
/// plugin's example type), so the copy-pasted first move is valid for the chosen plugin; a command
/// without the slot is unchanged. `example_type` is `None` only when flow is not active — in which
/// case flow's `{type}` command is not in the roster to substitute — so the `"item"` fallback is
/// effectively unreachable; it is a non-type sentinel that would fail `create` loudly rather than
/// masquerade as valid, never silently mislead (PR-review Code Quality #4).
fn first_commands(
    roster: &[&ModuleInit],
    active: &[String],
    example_type: Option<&str>,
) -> Vec<(&'static str, String)> {
    active
        .iter()
        .filter_map(|key| registry::find(roster, key).and_then(|m| m.first_command))
        .map(|fc| {
            (
                fc.label,
                fc.command.replace("{type}", example_type.unwrap_or("item")),
            )
        })
        .collect()
}

/// The suite welcome box (manufakt-Forge on a TTY, plain otherwise). When a module is preselected
/// (5jz.6 — flow for `nxf init`/a bare `nxs init`), its richer product-intro copy joins the generic
/// suite line, so the human sees what the chosen tool is for right in the frame.
fn print_welcome(theme: &Theme, defaults: &[&ModuleInit]) {
    let mut lines =
        vec!["The board, the memory and the channel your agents work from.".to_string()];
    for blurb in welcome_blurbs(defaults) {
        lines.push(blurb.to_string());
    }
    lines.push(String::new());
    lines.push("Pick the tools to set up in this workspace — you can add more later.".to_string());
    // Give the first impression room to breathe (nexus-flow-92k): a blank line ABOVE the box (so it
    // is not wedged directly under the shell prompt) and one below it.
    println!();
    println!("{}", width::boxed(theme, "Welcome to nexus-flow", &lines));
    println!();
}

/// The frame's two READ-BACK status lines: what the host will actually run at session start, and
/// what this workspace's standing with the background service is.
///
/// Bundled rather than passed as two more parameters, for the reason `hook_line` was pre-rendered
/// by the caller in the first place: [`print_summary`] is already at clippy's argument limit, and
/// both lines are composed where the facts they report already live.
struct StatusLines {
    hook: String,
    service: String,
}

/// The framed completion summary (after driving). Themed; plain + byte-stable off a TTY / `--json`.
fn print_summary(
    theme: &Theme,
    workspace: &Path,
    modules: &[String],
    status: &StatusLines,
    roster: &[&ModuleInit],
    already_active: &[String],
    example_type: Option<&str>,
) {
    let module_list = if modules.is_empty() {
        "(none)".to_string()
    } else {
        modules.join(", ")
    };
    // Two blank lines separate the last interactive step (e.g. flow's plugin chooser) from the
    // summary, so the success moment lands with room to breathe (init whitespace polish).
    println!();
    println!();
    println!("{}", theme.heading("nexus-flow is set up"));
    println!("  workspace:    {}", workspace.display());
    println!("  modules:      {}", theme.accent(&module_list));
    println!("  agent files:  assembled (AGENTS.md)");
    // **Superseded 2026-08-28 (nxf n2m6 + a2a1):** this line read "SessionStart → nxs prime" and
    // was built here from a `hook_wired: bool`. It is one entry per active module now, so there is
    // a LIST to render; the rendering moved to the caller (`hook_summary`), which is where both the
    // wired-ness check and the command list already live — and keeps this function under clippy's
    // argument limit rather than growing an eighth parameter to carry the same one line.
    println!("  hook:         {}", status.hook);
    // nxf 6j6v.npf9: whether a background service exists on this machine, and whether it knows
    // about THIS workspace. Neither is visible from anywhere else in the frame, and after 6j6v.8see
    // the answer decides whether a declared window in this workspace ever fires.
    println!("  service:      {}", status.service);
    // Per-module detail; a module already active before this run is marked (5jz.4), so re-running
    // `nxs init` in an existing workspace reads as "this was already here", not a silent repeat.
    for m in modules {
        if let Some(entry) = registry::find(roster, m) {
            if !entry.blurb.is_empty() {
                println!(
                    "    {} — {}{}",
                    theme.emphasis(m),
                    entry.blurb,
                    active_suffix(m, already_active)
                );
            }
        }
    }
    // The post-init success moment (5jz.6): each active module's own copy-paste first command, in
    // manufakt-Forge ember — the prominent "now do this" at the end of the frame. Self-registered, so
    // the umbrella never hardcodes a product's command. Empty (e.g. memory-only) → no block.
    let firsts = first_commands(roster, modules, example_type);
    if !firsts.is_empty() {
        // Two blank lines before the "your first move" call-to-action.
        println!();
        println!();
        println!("{}", theme.heading("your first move"));
        for (label, command) in &firsts {
            println!("  {label}");
            println!("  {}", theme.accent(command));
        }
    }
    println!();
    println!(
        "{}",
        theme.muted("Open a new session and `nxs prime` loads your project context.")
    );
    // A trailing blank line so the shell prompt is not glued to the last line.
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::FirstCommand;
    use nxs_foundation::error::ErrorKind;

    fn noop(_: &registry::InitRequest) -> Result<()> {
        Ok(())
    }

    /// A test roster descriptor — the real modules self-register via `inventory`, but the umbrella's
    /// selection LOGIC is exercised here with hand-built entries (no linked modules, no TTY).
    fn module(
        key: &'static str,
        binary: &'static str,
        order: u16,
        recommended: bool,
    ) -> ModuleInit {
        ModuleInit {
            key,
            binary,
            now_env: "X_NOW",
            blurb: "",
            recommended,
            order,
            accepts_plugin: key == "flow",
            init_fn: noop,
            welcome: None,
            details: None,
            first_command: None,
        }
    }

    fn keys(mods: &[&ModuleInit]) -> Vec<&'static str> {
        mods.iter().map(|m| m.key).collect()
    }

    #[test]
    fn modules_from_picks_maps_indices_in_roster_order() {
        let (flow, memory) = (
            module("flow", "nxf", 10, true),
            module("memory", "nxm", 20, false),
        );
        let roster = vec![&flow, &memory];
        assert_eq!(
            keys(&modules_from_picks(&roster, &[0]).unwrap()),
            vec!["flow"]
        );
        assert_eq!(
            keys(&modules_from_picks(&roster, &[0, 1]).unwrap()),
            vec!["flow", "memory"]
        );
    }

    #[test]
    fn modules_from_picks_rejects_an_empty_selection() {
        let flow = module("flow", "nxf", 10, true);
        let roster = vec![&flow];
        let err = modules_from_picks(&roster, &[]).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(
            err.msg.contains("at least one"),
            "explains the user must pick a tool: {}",
            err.msg
        );
    }

    #[test]
    fn resolve_flagged_preserves_roster_order_not_input_order() {
        // memory-then-flow on the flags still resolves in roster order (flow first) — the order
        // the driven fan-out depends on (flow creates the workspace before memory joins).
        let (flow, memory) = (
            module("flow", "nxf", 10, true),
            module("memory", "nxm", 20, false),
        );
        let roster = vec![&flow, &memory];
        assert_eq!(
            keys(&resolve_flagged(&roster, &["memory".into(), "flow".into()]).unwrap()),
            vec!["flow", "memory"]
        );
    }

    #[test]
    fn resolve_flagged_rejects_an_unknown_module_naming_it() {
        let flow = module("flow", "nxf", 10, true);
        let roster = vec![&flow];
        let err = resolve_flagged(&roster, &["ghost".into()]).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.msg.contains("ghost"), "names the offender: {}", err.msg);
    }

    #[test]
    fn resolve_selection_explicit_flags_win() {
        // Flags are honored regardless of json/TTY — the agent entry point.
        let (flow, memory) = (
            module("flow", "nxf", 10, true),
            module("memory", "nxm", 20, false),
        );
        let roster = vec![&flow, &memory];
        let t = Theme::plain();
        assert_eq!(
            keys(&resolve_selection(&roster, &t, false, &["memory".into()], &[], &[]).unwrap()),
            vec!["memory"]
        );
    }

    #[test]
    fn resolve_selection_non_interactive_with_no_flags_defaults_to_recommended() {
        // `--json` (or any non-terminal sink) with no `--module` and no `--preselect` → the
        // recommended module (flow), no prompt. Under `cargo test` stdin/stderr are not terminals,
        // so the no-flags path defaults too.
        let (flow, memory) = (
            module("flow", "nxf", 10, true),
            module("memory", "nxm", 20, false),
        );
        let roster = vec![&flow, &memory];
        let t = Theme::plain();
        assert_eq!(
            keys(&resolve_selection(&roster, &t, true, &[], &[], &[]).unwrap()),
            vec!["flow"]
        );
        assert_eq!(
            keys(&resolve_selection(&roster, &t, false, &[], &[], &[]).unwrap()),
            vec!["flow"]
        );
    }

    #[test]
    fn resolve_selection_preselect_drives_the_non_interactive_default() {
        // The front-door re-exec (`nxm init` → `nxs init --preselect memory`): with no `--module`,
        // the preselected module — NOT the recommended default — is what a non-interactive sink
        // sets up. This is what makes `nxm init --json` land on memory while `nxf init --json`
        // lands on flow, both through the one umbrella frame.
        let (flow, memory) = (
            module("flow", "nxf", 10, true),
            module("memory", "nxm", 20, false),
        );
        let roster = vec![&flow, &memory];
        let t = Theme::plain();
        assert_eq!(
            keys(&resolve_selection(&roster, &t, true, &[], &["memory".into()], &[]).unwrap()),
            vec!["memory"],
            "preselect overrides the recommended default for a non-interactive sink"
        );
    }

    #[test]
    fn resolve_selection_explicit_module_overrides_preselect() {
        // `--module` is the explicit agent entry and wins over the entry-point `--preselect` hint.
        let (flow, memory) = (
            module("flow", "nxf", 10, true),
            module("memory", "nxm", 20, false),
        );
        let roster = vec![&flow, &memory];
        let t = Theme::plain();
        assert_eq!(
            keys(
                &resolve_selection(&roster, &t, true, &["flow".into()], &["memory".into()], &[])
                    .unwrap()
            ),
            vec!["flow"]
        );
    }

    #[test]
    fn resolve_selection_preselect_in_roster_order_for_multiple() {
        // A preselect set resolves in roster order (flow before memory), like `--module`, so the
        // driven fan-out order is stable regardless of the order the flags arrived in.
        let (flow, memory) = (
            module("flow", "nxf", 10, true),
            module("memory", "nxm", 20, false),
        );
        let roster = vec![&flow, &memory];
        let t = Theme::plain();
        assert_eq!(
            keys(
                &resolve_selection(
                    &roster,
                    &t,
                    true,
                    &[],
                    &["memory".into(), "flow".into()],
                    &[]
                )
                .unwrap()
            ),
            vec!["flow", "memory"]
        );
    }

    #[test]
    fn resolve_selection_unknown_preselect_is_a_loud_error_naming_it() {
        let flow = module("flow", "nxf", 10, true);
        let roster = vec![&flow];
        let t = Theme::plain();
        let err = resolve_selection(&roster, &t, true, &[], &["ghost".into()], &[]).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.msg.contains("ghost"), "names the offender: {}", err.msg);
    }

    // ---- Installed-Detection (5jz.4): show what's already active in the workspace ----

    #[test]
    fn active_suffix_marks_only_an_already_active_module() {
        // The chooser + summary append this marker to a module already in `config.active_modules`,
        // so a re-run reads informatively ("flow already set up") instead of silently skipping it.
        let already = vec!["flow".to_string()];
        assert_eq!(active_suffix("flow", &already), " (already set up)");
        assert_eq!(active_suffix("memory", &already), "");
        // No workspace yet (first run) → nothing is marked, so the first-run chooser stays pristine.
        assert_eq!(active_suffix("flow", &[]), "");
    }

    #[test]
    fn chooser_default_indices_union_preselect_with_already_active_in_roster_order() {
        let (flow, memory) = (
            module("flow", "nxf", 10, true),
            module("memory", "nxm", 20, false),
        );
        let roster = vec![&flow, &memory];
        // Re-run of `nxm init` in a flow workspace: memory preselected by the entry point, flow
        // already active → BOTH pre-checked (flow as done, memory as the new pick), roster order.
        assert_eq!(
            chooser_default_indices(&roster, &[&memory], &["flow".to_string()]),
            vec![0, 1]
        );
        // First run, flow preselected, nothing active yet → just flow ticked.
        assert_eq!(chooser_default_indices(&roster, &[&flow], &[]), vec![0]);
    }

    // ---- #hai-Rest (5jz.6): flow welcome copy + post-init success moment, self-registered ----

    // ---- huy: the picker's self-registered "learn more" details panel ----

    #[test]
    fn tool_details_panel_lists_each_tools_self_registered_details_and_skips_those_without() {
        // The "learn more" panel is composed from each rostered tool's OWN `details` copy (the
        // umbrella hardcodes none), and a tool that registered no details contributes nothing — so
        // the user gets real decision-help per tool instead of a one-line blurb.
        let flow = ModuleInit {
            details: Some("flow tracks epics, tasks, and their dependencies."),
            ..module("flow", "nxf", 10, true)
        };
        let memory = ModuleInit {
            details: Some("memory recalls durable facts across sessions."),
            ..module("memory", "nxm", 20, false)
        };
        let chat = module("chat", "nxc", 30, false); // details: None
        let panel = tool_details_panel(&Theme::plain(), &[&flow, &memory, &chat]);
        assert!(
            panel.contains("flow")
                && panel.contains("flow tracks epics, tasks, and their dependencies."),
            "flow's details are shown: {panel}"
        );
        assert!(
            panel.contains("memory")
                && panel.contains("memory recalls durable facts across sessions."),
            "memory's details are shown: {panel}"
        );
        assert!(
            !panel.contains("chat"),
            "a tool that registered no details contributes nothing: {panel}"
        );
    }

    #[test]
    fn tool_details_panel_is_empty_when_no_tool_registers_details() {
        // No details anywhere → no panel at all, so the caller prints nothing (the picker just shows
        // the one-line blurbs). Keeps the umbrella free of any product copy of its own.
        let a = module("a", "na", 10, false); // details: None
        assert!(
            tool_details_panel(&Theme::plain(), &[&a]).is_empty(),
            "an all-None roster yields no panel"
        );
    }

    #[test]
    fn tool_details_panel_wraps_long_copy_with_a_four_space_hanging_indent() {
        // Test Quality #5: long details copy wraps to MULTIPLE rows, each carrying the 4-space
        // hanging indent under the tool name — so the panel stays a tidy block, not a ragged
        // overflow. The short-copy cases above never exercise the wrap; this pins the indent
        // composition itself (the underlying `wrap` primitive is tested in nxs-ui).
        let long = "Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi \
                    omicron pi rho sigma tau upsilon phi chi psi omega and then some more words.";
        assert!(
            long.len() > width::content_width(),
            "fixture must exceed the wrap width to force more than one row"
        );
        let flow = ModuleInit {
            details: Some(long),
            ..module("flow", "nxf", 10, true)
        };
        let panel = tool_details_panel(&Theme::plain(), &[&flow]);
        let detail_rows: Vec<&str> = panel.lines().filter(|l| l.starts_with("    ")).collect();
        assert!(
            detail_rows.len() >= 2,
            "long copy wrapped onto multiple indented rows: {panel}"
        );
        for row in &detail_rows {
            assert!(
                row.starts_with("    ") && !row[4..].starts_with(' '),
                "each continuation row carries exactly the 4-space hanging indent: {row:?}"
            );
        }
    }

    #[test]
    fn welcome_blurbs_renders_a_preselected_modules_own_copy_only() {
        // The umbrella stays vocabulary-free: a module's richer welcome copy travels WITH the module
        // (self-registered), and the welcome box shows it only when that module is preselected.
        let flow = ModuleInit {
            welcome: Some("Track projects, tasks, and the dependencies between them."),
            ..module("flow", "nxf", 10, true)
        };
        let memory = module("memory", "nxm", 20, false); // welcome: None
                                                         // flow preselected → its copy; memory (no copy) → nothing extra.
        assert_eq!(
            welcome_blurbs(&[&flow]),
            vec!["Track projects, tasks, and the dependencies between them."]
        );
        assert!(welcome_blurbs(&[&memory]).is_empty());
    }

    #[test]
    fn first_commands_lists_active_modules_that_registered_one_in_active_order() {
        // The post-init success moment is the active module's OWN first command (self-registered), so
        // the umbrella renders a copy-paste first step without hardcoding any product's vocabulary.
        let flow = ModuleInit {
            first_command: Some(FirstCommand {
                label: "create your first item",
                command: "nxf create --type {type} --title \"...\"",
            }),
            ..module("flow", "nxf", 10, true)
        };
        let memory = module("memory", "nxm", 20, false); // first_command: None
        let roster = vec![&flow, &memory];
        let cmds = first_commands(
            &roster,
            &["flow".to_string(), "memory".to_string()],
            Some("epic"),
        );
        assert_eq!(cmds.len(), 1, "only flow registered a first command");
        assert_eq!(cmds[0].0, "create your first item");
        assert_eq!(cmds[0].1, "nxf create --type epic --title \"...\"");
    }

    #[test]
    fn first_commands_fills_the_type_placeholder_with_the_active_plugin() {
        // nexus-flow-92zt: flow's self-registered first command carries a `{type}` slot; the umbrella
        // fills it with the active plugin's example type so the copy-pasted first move is VALID (the
        // old hardcoded `--type task` is in no bundled plugin). The command TEXT stays the module's
        // own — the umbrella only fills the plugin slot.
        let flow = ModuleInit {
            first_command: Some(FirstCommand {
                label: "create your first item",
                command: "nxf create --type {type} --title \"...\"",
            }),
            ..module("flow", "nxf", 10, true)
        };
        let roster = vec![&flow];
        let active = ["flow".to_string()];

        assert_eq!(
            first_commands(&roster, &active, Some("project"))[0].1,
            "nxf create --type project --title \"...\"",
        );
        // No resolved type (memory-only run, or a non-flow workspace) → a safe placeholder, never a
        // dangling `{type}` shown to the user.
        assert_eq!(
            first_commands(&roster, &active, None)[0].1,
            "nxf create --type item --title \"...\"",
        );
    }
}
