//! Command implementations. Each resolves the workspace, talks to `nexus-flow-core`,
//! and renders through the active plugin config (human) or the canonical record (json).
//! No command hardcodes vocabulary, priority labels, or presentation — it all flows
//! from the `PluginConfig`.

mod guide;

pub use guide::guide;

use crate::error::{NxfError, Result};
use crate::onboarding;
use crate::output::{self, raw_field};
use crate::plugin::{self, PluginConfig};
use crate::validate;
use crate::workspace::{self, Workspace, WorkspaceExt};
use nexus_flow_core::model::{ItemRow, LinkRelation, LinkWeight, ThreadLink};
use nexus_flow_core::store::Store;
use nexus_flow_facade::{read, write};
use nxs_foundation::text_input::{self, StdinReader};
use std::collections::BTreeMap;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// The `create` payload — owned by the shared write layer (E5 #9t7.5) and re-exported so the
/// `nxf create` dispatch keeps building `commands::NewItem`.
pub use nexus_flow_facade::write::NewItem;

/// The actor recorded on each op. Identity is local in E2; sync (E4) refines it. A set-but-empty
/// `NXF_ACTOR`/`USER` counts as unset, not as an empty identity — the rule and its rationale live
/// in [`nxs_foundation::model::resolve_author`] (invariant 1, 6j6v.xsf3).
fn actor() -> String {
    nxs_foundation::model::resolve_author(
        std::env::var("NXF_ACTOR").ok(),
        std::env::var("USER").ok(),
        "nxf",
    )
}

fn cwd() -> Result<std::path::PathBuf> {
    std::env::current_dir().map_err(|e| NxfError::io(format!("reading current dir: {e}")))
}

/// Resolve the workspace and load its selected plugin config.
fn open(db: Option<&str>) -> Result<(Workspace, PluginConfig)> {
    let ws = Workspace::resolve(db, &cwd()?)?;
    let cfg = plugin::load(&workspace::flow_plugin(&ws.config))?;
    Ok((ws, cfg))
}

/// Fetch a live (non-tombstoned) item or fail with `not_found`.
fn require_item(store: &Store, id: &str) -> Result<ItemRow> {
    store
        .get_item(id)?
        .filter(|i| i.deleted.as_deref() != Some("1"))
        .ok_or_else(|| NxfError::not_found(format!("no item '{id}'")))
}

/// Emit a mutated item: the canonical record (full id) under `--json`, else `"<verb> <id>"` with
/// the id in display form — bare for a local item, full for a foreign one (nexus-flow-ykv).
fn emit_item(json: bool, prefix: &str, item: &ItemRow, verb: &str) {
    if json {
        println!("{}", output::item_value(item));
    } else {
        println!("{verb} {}", workspace::display_id(prefix, &item.id));
    }
}

/// Emit a two-id edge receipt (`dep`/`mention`/`contributes`): under `--json` the `msg` carries the
/// FULL ids (the machine contract is untouched), while the human line renders each id in display
/// form — bare for a local item, full for a foreign one (nexus-flow-ykv). `tmpl` builds the
/// sentence from the two already-formatted ids.
fn emit_edge(json: bool, prefix: &str, a: &str, b: &str, tmpl: impl Fn(&str, &str) -> String) {
    if json {
        println!("{}", serde_json::json!({ "ok": true, "msg": tmpl(a, b) }));
    } else {
        println!(
            "{}",
            tmpl(
                workspace::display_id(prefix, a),
                workspace::display_id(prefix, b)
            )
        );
    }
}

/// Emit a label receipt (`label add`/`remove`): under `--json` the `msg` carries the FULL id (the
/// machine contract), while the human line renders the id in display form. The label text is opaque
/// user vocabulary — NOT an id — so it is passed through verbatim (never through `display_id`).
fn emit_label(
    json: bool,
    prefix: &str,
    id: &str,
    label: &str,
    tmpl: impl Fn(&str, &str) -> String,
) {
    if json {
        println!(
            "{}",
            serde_json::json!({ "ok": true, "msg": tmpl(id, label) })
        );
    } else {
        println!("{}", tmpl(workspace::display_id(prefix, id), label));
    }
}

/// Human list view: one row per item, the plugin's `presentation.list` columns with
/// vocabulary applied. The `type` column is wrapped in `[...]` so the kind of each row is
/// unmistakable at a glance (nexus-flow-9dy); the label itself is the plugin's vocabulary.
fn render_list(cfg: &PluginConfig, prefix: &str, items: &[ItemRow]) -> String {
    let mut out = String::new();
    for item in items {
        out.push_str(&list_columns(cfg, prefix, item, &nxs_ui::Theme::plain()).join("  "));
        out.push('\n');
    }
    out
}

/// The plugin's `presentation.list` columns for one item, vocabulary applied, `type` bracketed —
/// the shared column logic behind [`render_list`] and [`render_next`] (factored, not duplicated).
/// The `id` column is shown in display form against `prefix` (bare local / full foreign, ykv).
///
/// `theme` gives the row its visual hierarchy (6j6v.8bax): the title bold — it is the one field a
/// human scans for — the id muted, and the priority on the shared Ember → Flame ramp. Everything
/// else keeps the plain foreground; a row where every field shouts is the row nobody reads. The
/// columns are plugin-declared, so the treatment is keyed on the column NAME and a plugin that
/// lists other columns simply gets them plain.
///
/// [`render_list`] passes [`nxs_ui::Theme::plain`] — `nxf list` is unchanged, and so is every caller that
/// is not a real terminal, down to the byte (see the [`nxs_ui::style`] module guardrail).
fn list_columns(
    cfg: &PluginConfig,
    prefix: &str,
    item: &ItemRow,
    theme: &nxs_ui::Theme,
) -> Vec<String> {
    cfg.presentation
        .list
        .columns
        .iter()
        .map(|c| {
            raw_field(item, c)
                .map(|raw| match c.as_str() {
                    "id" => theme.muted(workspace::display_id(prefix, &raw)),
                    "type" => format!("[{}]", label(cfg, c, &raw)),
                    "title" => theme.emphasis(&label(cfg, c, &raw)),
                    // The ramp reads the CANONICAL ORDINAL, never the label — priority names are
                    // plugin vocabulary (P0..P4 here, now/soon/later there) and matching the string
                    // would be wrong under any other plugin. A value that is not an ordinal is a
                    // defensive fallback (`label` treats it the same way) and stays unpainted.
                    "priority" => match raw.parse::<usize>() {
                        Ok(ordinal) => theme.priority(ordinal, &label(cfg, c, &raw)),
                        Err(_) => label(cfg, c, &raw),
                    },
                    _ => label(cfg, c, &raw),
                })
                .unwrap_or_else(|| "-".to_string())
        })
        .collect()
}

/// Human `next` view (#916.7): the same columns as [`render_list`], plus — for a child item — an
/// indented `↳ <parent-id> · <parent-title>` follow-up line so a child never reads as top-level.
/// Top-level items get no follow-up; `nxf list` is unaffected (it keeps [`render_list`]).
///
/// Takes the whole [`read::NextPage`], not just its rows, because the truncation disclosure is
/// part of the view and not an optional garnish a caller could forget (6j6v.8pf2).
fn render_next(
    cfg: &PluginConfig,
    ws: &Workspace,
    store: &Store,
    page: &read::NextPage,
    theme: &nxs_ui::Theme,
) -> Result<String> {
    let prefix = &ws.replica.prefix;
    let items = &page.items;
    let mut out = String::new();
    if let Some(notice) = next_truncation_notice(page, theme) {
        out.push_str(&notice);
    }
    // nxf 6j6v.8dbe: one bulk read for the whole lane, mirroring the `--json` path's own — and
    // BEARING links only, so a wandering conversation that merely grazed an item cannot make it
    // advertise "there are conversations here" in the work list.
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    let conversations = read::bearing_thread_counts_bulk(store, &ids)?;
    // 6j6v.srpg: the retrieval rule's HINT cell — one bulk read for the whole lane, same shape and
    // same reason as the conversation counts above.
    let memories = crate::memories::about(ws, &ids)?;
    for item in items {
        out.push_str(&list_columns(cfg, prefix, item, theme).join("  "));
        // A count, not the thread ids: `next` is a work list, and the answer it owes the reader is
        // "there is a conversation about this, go look" — `nxf show <id>` is where the ids live.
        // Spelled out rather than pictographic (PR #273 review, Code Quality #3): every other
        // decoration this CLI prints is plain, an emoji renders at an unpredictable width, and a
        // word needs no legend — for the agents this output is measured against, most of all.
        if let Some(n) = conversations.get(&item.id) {
            let plural = if *n == 1 { "" } else { "s" };
            out.push_str(&format!("  ({n} conversation{plural})"));
        }
        // 6j6v.srpg: a HINT, never the text. `next` is a work list, and its answer here is "there
        // is durable knowledge about this item — `nxf show <id>`". Printing the bodies would put a
        // whole paragraph per row into the one output that has to stay scannable at forty items,
        // which is exactly the failure the rule's three columns exist to prevent.
        if let Some(n) = memories.get(&item.id).map(Vec::len) {
            let plural = if n == 1 { "y" } else { "ies" };
            out.push_str(&format!("  ({n} memor{plural})"));
        }
        out.push('\n');
        if let Some(p) = read::parent_of(store, item)? {
            out.push_str(&next_parent_line(prefix, &p, theme));
        }
    }
    Ok(out)
}

/// The `↳ <parent-id> · <parent-title>` follow-up line under a child row — context, not content,
/// so the whole line is muted (6j6v.8bax). The indent stays OUTSIDE the escape: the column a
/// reader's eye follows down the list is plain spaces, not a styled run.
fn next_parent_line(prefix: &str, parent: &read::ParentRef, theme: &nxs_ui::Theme) -> String {
    format!(
        "    {}\n",
        theme.muted(&format!(
            "↳ {} · {}",
            workspace::display_id(prefix, &parent.id),
            parent.title.as_deref().unwrap_or("")
        ))
    )
}

/// The truncation disclosure above a cut `next` list (6j6v.8pf2) — `None` when the limit did not
/// bite, which is what keeps an uncut list byte-identical to the view without `--limit`.
///
/// A limit that cuts silently is worse than no limit at all: a list showing 15 of 180 that looks
/// like 15 of 15 tells the reader they are through. Same disclosure, same wording as `prime`'s
/// "showing 15 of 180" heading. Muted, because it is a note about the list and not part of it.
fn next_truncation_notice(page: &read::NextPage, theme: &nxs_ui::Theme) -> Option<String> {
    page.truncated().then(|| {
        format!(
            "{}\n",
            theme.muted(&format!("showing {} of {}", page.items.len(), page.total))
        )
    })
}

/// `prime`'s `next` rows as Markdown bullets (nexus-flow-7s6): the same plugin-driven columns
/// as [`render_list`], but the `id` column is a code span, the `type` keeps its `[..]` bracket
/// (nexus-flow-9dy), and the columns are dot-separated so each row is one clean bullet.
///
/// Takes NO theme, and must never take one (6j6v.8bax): this Markdown is injected into an agent's
/// context, where an escape sequence is not styling but garbage in a prompt. The visual hierarchy
/// [`render_next`] gained is for the human view only — Markdown already has its own.
fn render_next_md(cfg: &PluginConfig, prefix: &str, rows: &[read::NextRow]) -> String {
    let mut out = String::new();
    for row in rows {
        let cols: Vec<String> = cfg
            .presentation
            .list
            .columns
            .iter()
            .map(|c| {
                raw_field(&row.item, c)
                    .map(|raw| {
                        let v = label(cfg, c, &raw);
                        match c.as_str() {
                            "id" => format!("`{}`", workspace::display_id(prefix, &raw)),
                            "type" => format!("[{v}]"),
                            // Free-text columns (e.g. `title`) are escaped so a backtick,
                            // `<tag>`, or `[x](url)` in user/agent text renders verbatim in the
                            // host-injected Markdown instead of breaking the line. The `id`/`type`
                            // columns above carry their own intentional formatting.
                            _ => md_escape_inline(&v),
                        }
                    })
                    .unwrap_or_else(|| "-".to_string())
            })
            .collect();
        out.push_str("- ");
        out.push_str(&cols.join(" · "));
        out.push('\n');
        // #916.7: an indented parent sub-line for a child item (markdown-escaped like the rest of
        // prime); the id is a code span, matching the `next` column's `id` rendering above.
        if let Some(p) = &row.parent {
            out.push_str(&format!(
                "  ↳ `{}` · {}\n",
                workspace::display_id(prefix, &p.id),
                md_escape_inline(p.title.as_deref().unwrap_or(""))
            ));
        }
    }
    out
}

/// Backslash-escape the inline Markdown metacharacters in free text so it renders verbatim in
/// `prime`'s host-injected output: a `title` carrying a backtick, `<tag>`, or `[x](url)` would
/// otherwise break the surrounding code span, vanish as raw HTML, or turn into a link. Used for
/// the plain-text columns of [`render_next_md`]; the canonical `--json` `title` stays raw.
fn md_escape_inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// The reference time threaded into derivation (reads) and stamped on emitted ops (writes):
/// an explicit `--now` wins, else a pinned `NXF_NOW` (a determinism knob, symmetric with
/// `NXF_ACTOR` — used by the docs harness and the App↔CLI parity differential, #9t7.6), else the
/// system clock. Any explicit value is validated, so a malformed pin fails loudly rather than
/// silently stamping garbage.
fn resolve_now(over: Option<&str>) -> Result<String> {
    let pinned = over
        .map(str::to_string)
        .or_else(|| std::env::var("NXF_NOW").ok().filter(|s| !s.is_empty()));
    match pinned {
        Some(s) => {
            validate::iso_date(&s)?;
            Ok(s)
        }
        None => OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|e| NxfError::io(format!("formatting current time: {e}"))),
    }
}

// ---- init ------------------------------------------------------------------

/// The onboarding first-move label — the human nudge shared by flow's own `nxf init` next-step and
/// the umbrella's post-init banner (5jz.6).
const FIRST_MOVE_LABEL: &str = "create your first item";

/// The umbrella-banner first-move command TEMPLATE with a `{type}` placeholder (nexus-flow-92zt).
/// The umbrella fills `{type}` with the active flow plugin's example type at render time — the same
/// copy as flow's own next-step, just deferred because a compile-time `inventory` descriptor cannot
/// know the runtime plugin choice. Kept in ONE place so the banner and `nxf init` never diverge.
const FIRST_MOVE_TEMPLATE: &str = "nxf create --type {type} --title \"...\"";

/// The plugin-aware first-move command (nexus-flow-92zt): `create --type <the active plugin's
/// example/root type>` so the copy-pasted onboarding command is VALID on a stock init. The old
/// hardcoded `--type task` is in NEITHER bundled plugin (issue-tracker/personal-todo), so the very
/// first suggested command errored.
fn first_move_command(cfg: &PluginConfig) -> String {
    // `example_type` is `None` only for an empty type list, which no loadable plugin has (a plugin
    // with zero types fails validation and never reaches here) — so the `"item"` fallback is
    // UNREACHABLE. It is deliberately a non-type placeholder rather than a real type: were it ever
    // rendered it would fail `create` loudly (the exact 92zt bug class), never masquerade as valid
    // and silently mislead — but it cannot be reached (PR-review Code Quality #4).
    FIRST_MOVE_TEMPLATE.replace("{type}", cfg.types.example_type().unwrap_or("item"))
}

/// The next steps printed (and emitted under `--json` as `next_steps`) after a successful init, so
/// an agent or operator is never left guessing what to do with a fresh workspace. `(label, command)`
/// pairs; the first-move `create` is specialized to the active plugin's vocabulary (nexus-flow-92zt).
fn init_next_steps(cfg: &PluginConfig) -> Vec<(&'static str, String)> {
    vec![
        (FIRST_MOVE_LABEL, first_move_command(cfg)),
        ("read the guide", "nxf guide getting-started".to_string()),
    ]
}

/// The active flow plugin's example `--type` at `root`, for the umbrella's post-init banner
/// (nexus-flow-92zt): discover the workspace, load its flow plugin, and return its example/root type
/// — the token the umbrella substitutes into flow's `{type}` first-move template. `None` when `root`
/// is not a flow workspace or the plugin cannot be loaded (the umbrella then leaves a safe default).
pub fn flow_example_type(root: &std::path::Path) -> Option<String> {
    let ws = workspace::discover(root).ok()?;
    let cfg = plugin::load(&workspace::flow_plugin(&ws.config)).ok()?;
    cfg.types.example_type().map(str::to_string)
}

/// Initialize a workspace. A plugin must be chosen *deliberately* (4oa.4 — no silent
/// default): on a TTY we prompt with a described chooser; otherwise `--plugin` is required.
/// Either way the choice is validated (via `plugin::load`) before any workspace is created.
pub fn init(json: bool, quiet: bool, plugin: Option<&str>, from_beads: bool) -> Result<()> {
    // Single front door (5jz.2): a HUMAN running `nxf init` on a terminal is routed through the
    // shared `nxs init` frame with flow PRE-SELECTED, so every entry point lands on the same suite
    // chooser (flow ticked, the user can add memory). The driven `--quiet` seam the umbrella itself
    // calls, and the non-interactive contracts (`--json`/piped), stay flow-native and byte-stable —
    // "Nicht-interaktiv unverändert flag-gesteuert". This also keeps the re-exec non-recursive:
    // `nxf init` (TTY) → `nxs init --preselect flow` → drives `nxf init --quiet` (not a TTY).
    //
    // `--from-beads` (the beads → nxs migration consent, nexus-flow-6ef.1) ALSO routes through the
    // umbrella even non-interactively — the migration is an umbrella operation (it needs the full
    // module roster + owns the import/rollback), so the flag is passed through the re-exec.
    if !quiet && (from_beads || (!json && nxs_init::frontdoor::is_interactive())) {
        return nxs_init::frontdoor::reexec_umbrella_init("flow", json, plugin, from_beads);
    }
    warn_if_deterministic_switch_live();
    // Set flow up via the shared seam: the plugin choice (the TTY chooser for a human-facing init,
    // else the `--plugin` pass-through / flow's default for `--quiet`), the workspace, the store,
    // and the shared agent file. `require_choice = !quiet`: the direct human/json `nxf init`
    // demands a deliberate choice (4oa.4), while the umbrella's Stage-A `--quiet` driver seats
    // flow's default. The same seam serves the in-process umbrella entry ([`flow_init_entry`], 5jz.3).
    // `setup_flow` already loaded + validated the plugin before creating the workspace; reuse that
    // config for the onboarding CTA (nexus-flow-92zt) instead of a second load AFTER the workspace
    // exists — a redundant fallible op that would fail `init` on an already-created workspace
    // (PR-review Integrity & Robustness #2).
    let (ws, cfg, report) = setup_flow(&cwd()?, json, !quiet, plugin)?;
    // **The workspace comes to the background service at its birth on THIS path too** (nxf
    // 6j6v.y12q). `nxs init` has done it since 6j6v.npf9, and this is the path that never reaches
    // it: under `--json` or `--quiet` the re-exec above does not happen, which is exactly how an
    // agent enters. Since 6j6v.8see the service's list is where a workspace's deadlines come from,
    // so without this a workspace an agent set up had no clock and nothing said so.
    //
    // Registration only — INSTALLING the service stays the umbrella's question, and npf9 settled
    // that it is never asked and never done under `--json` or in a pipe.
    let service = nxs_init::service::register_workspace(&ws.dir);
    // **A note is not summary output, so `--quiet` does not silence it** (review of PR #421, Code
    // Quality #1). The first version of this suppressed notes under `--quiet` on the grounds that
    // the umbrella prints them a moment later — true only when the umbrella is the one DRIVING
    // this init. A bare `nxf init --quiet`/`nxm init --quiet` reaches here too, and there is no
    // second chance there: the failure to register, or the overlap this run just created, would be
    // swallowed. That is the class of silence this ticket exists to end, so the worst case is now
    // the harmless one — the umbrella-driven path may say the same thing twice.
    for note in &service.notes {
        eprintln!("note: {note}");
    }
    // Cross-sell the not-yet-active suite modules (aye.31): `Some` here (flow-only init) since
    // memory is addable + inactive. Sourced from `nxs-init` (it owns the suite cross-sell copy).
    let ad = nxs_init::advertise::advertisement(&ws.config.active_modules);

    if json {
        let mut out = serde_json::json!({
            "ok": true,
            "workspace": ws.dir.display().to_string(),
            "site_id": ws.replica.site_id,
            "prefix": ws.replica.prefix,
            "plugin": workspace::flow_plugin(&ws.config),
            "onboarding": {
                "agents": report.agents.as_str(),
                "claude": report.claude.as_str(),
            },
            // Parity with `nxm init --json`: report whether the SessionStart wiring changed, so
            // an agent sees it without parsing the human view.
            //
            // **Superseded 2026-08-28 (nxf n2m6 + a2a1):** `command` was ONE string while the
            // assembler wired one `nxs prime` hook; it is now one entry per active module, so the
            // field is a LIST under the name that says so.
            "hook": {
                "hook_added": report.hook_added,
                "commands": report.hook_commands,
            },
            "next_steps": init_next_steps(&cfg)
                .iter()
                .map(|(label, command)| serde_json::json!({ "label": label, "command": command }))
                .collect::<Vec<_>>(),
            // nxf 6j6v.y12q. The same key `nxs init --json` carries, meaning the same thing:
            // whether this workspace is on the list the background service attends. It has no
            // `answer` beside it, and that is the honest difference — this verb never asks the
            // install question, so it has nothing to report about one.
            "service": { "registered": service.registered },
        });
        // Additive `advertisement` field (aye.31): present only when an addable module is inactive.
        // Agent-addressed copy — make the user aware, set nothing up without their permission.
        if let Some(ad) = &ad {
            out["advertisement"] = serde_json::Value::String(ad.agent_copy.clone());
        }
        println!("{out}");
    } else if !quiet {
        println!("initialized workspace at {}", ws.dir.display());
        println!("  prefix:  {}", ws.replica.prefix);
        println!("  plugin:  {}", workspace::flow_plugin(&ws.config));
        println!("  agent files:  {}", describe_onboarding(&report));
        // **Superseded 2026-08-28 (nxf n2m6 + a2a1): a local `describe_hook` used to render this.**
        // It interpolated the single `HOOK_COMMAND`, "not spelled out … [because] a hand-written
        // copy here would be one more place to go stale the next time it moves." It moved to the
        // assembler for exactly that reason: there is now a LIST to render — one entry per active
        // module — rather than a constant to paste, and this is one of four init paths that would
        // each have had to render it identically.
        println!(
            "  hook:    {}",
            nxs_init::assembler::describe_hooks(report.hook_added, &report.hook_commands)
        );
        println!("  service: {}", service.summary_line());
        println!("\nnext steps:");
        for (label, command) in init_next_steps(&cfg) {
            println!("  {label}:  {command}");
        }
        // The manufakt-Forge upsell CTA (TTY-gated → ember on a terminal, plain text otherwise).
        if let Some(ad) = &ad {
            let theme = nxs_ui::Theme::detect(false);
            println!("\n{}", theme.accent(&ad.human_cta));
        }
    }
    // `--quiet` without `--json`: silent success — the work is done, the umbrella renders.
    Ok(())
}

/// flow's self-registered in-process init entry (5jz.3). The umbrella calls this DIRECTLY (no longer
/// a `<bin> init --quiet` subprocess), so flow sets itself up inside the one umbrella process and
/// frame: when the umbrella init is interactive, flow's OWN plugin chooser runs VISIBLY in the frame
/// (the original #5jz problem — the chooser used to be swallowed by `--quiet`); non-interactively it
/// takes the `--plugin` pass-through or flow's default. flow prints NO summary of its own here — the
/// umbrella frames the aggregate result, so nothing bleeds through.
pub fn flow_init_entry(req: &nxs_init::InitRequest) -> Result<()> {
    warn_if_deterministic_switch_live();
    // Show flow's plugin chooser only when the umbrella init is genuinely interactive (a TTY, not
    // `--json`); otherwise seat the pass-through/default without a prompt. This mirrors how the
    // umbrella itself decided whether to prompt for module SELECTION, so the two stay in lockstep.
    let require_choice = !req.json && nxs_init::frontdoor::is_interactive();
    setup_flow(req.root, req.json, require_choice, req.plugin)?;
    Ok(())
}

/// Set flow up and return the workspace + assembler report, printing NOTHING — the shared seam
/// behind both the direct CLI `init` (which then prints flow's summary) and the in-process umbrella
/// entry (which frames the aggregate, 5jz.3). When `require_choice` the plugin is chosen via flow's
/// own chooser (the deliberate-choice contract, 4oa.4 — visible in whatever frame is active); else
/// it is the `--plugin` pass-through, falling back to flow's default. The choice is validated before
/// any disk write, then the workspace is created (refusing to nest), the store opened, and the
/// shared agent file + single `nxs prime` hook assembled (the assembler shells out to each active
/// module's `agent-manifest`, TB-5).
fn setup_flow(
    root: &std::path::Path,
    json: bool,
    require_choice: bool,
    plugin: Option<&str>,
) -> Result<(Workspace, PluginConfig, nxs_init::assembler::AssembleReport)> {
    let chosen = if require_choice {
        resolve_plugin_choice(json, plugin)?
    } else {
        plugin
            .map(str::to_string)
            .unwrap_or_else(|| workspace::flow_plugin(&workspace::WorkspaceConfig::default()))
    };
    // Load + validate the chosen plugin BEFORE creating the workspace (4oa.4), and KEEP the config so
    // the caller reuses it (e.g. the onboarding CTA) instead of re-loading it after the workspace
    // exists (PR-review Integrity & Robustness #2).
    let cfg = plugin::load(&chosen)?;
    // aye.2.3: a crash mid-init can leave a partial legacy `.nexusflow/` (db.sqlite but no
    // replica.toml) that migration skips. The foundation is a pure library (no stderr), so warn
    // here — before creating a fresh `.nxs/` one dir over — so the stale db is not silently
    // stranded. We only warn; the orphan is left for the user to inspect or remove.
    if let Some(orphan) = workspace::orphaned_legacy_dir(root) {
        eprintln!(
            "warning: found an orphaned legacy workspace at {} (db.sqlite but no replica.toml — \
             likely a crashed init). It will NOT be migrated; a fresh .nxs/ is being created. \
             Inspect or remove the old directory manually.",
            orphan.display()
        );
    }
    // flow seats its chosen plugin under the `[flow]` config section (aye.2.2) and creates the
    // workspace via the foundation (which refuses a nested one + materializes the substrate db).
    let ws = workspace::init_flow(root, &chosen)?;
    // Open flow's store to add the flow views (the foundation created only the substrate db) and
    // verify the pragmas apply, so a fresh workspace is ready to use immediately.
    ws.open_store()?;
    // Delegate the shared agent file + the SINGLE `nxs prime` SessionStart hook to the ONE assembler
    // in `nxs` — flow never self-assembles. The assembler reaches each active module by shelling out
    // to its `agent-manifest` (TB-5).
    let report = assemble_agent_files(root, &ws)?;
    Ok((ws, cfg, report))
}

// Self-registration (5jz.1): flow declares everything the umbrella needs to know about it — its
// roster identity (key/binary/now_env), its chooser copy + recommended-default flag, that it
// accepts `--plugin`, and its in-process init entry. The umbrella collects this via `inventory`.
inventory::submit! {
    nxs_init::ModuleInit {
        key: "flow",
        binary: "nxf",
        now_env: "NXF_NOW",
        blurb: "issue tracker — projects, tasks, and the dependencies between them",
        recommended: true,
        order: 10,
        accepts_plugin: true,
        init_fn: flow_init_entry,
        // 5jz.6: flow's own welcome pitch (shown in the nxs frame when flow is preselected) + its
        // post-init success moment. The first command reuses INIT_NEXT_STEPS[0], so the umbrella's
        // success line and flow's native `nxf init` next-step stay the ONE copy.
        welcome: Some(
            "flow keeps your projects and tasks — and what's ready, blocked, or next — in one \
             place the agent reads every session.",
        ),
        // huy: flow's "learn more" decision-help, shown in the picker's details panel.
        details: Some(
            "Epics, tasks, and the dependencies between them, with priorities. flow derives what is \
             ready, blocked, or next instead of you tracking it. Offline-first and agent-native: \
             the agent reads and updates it directly. A plugin picks the vocabulary and ranking.",
        ),
        first_command: Some(nxs_init::FirstCommand {
            label: FIRST_MOVE_LABEL,
            command: FIRST_MOVE_TEMPLATE,
        }),
    }
}

/// Assemble the shared agent file + one SessionStart hook per active module from the workspace's
/// active modules' manifests (P3-S5; a single `nxs prime` hook until nxf n2m6 + a2a1). nxf delegates to the ONE assembler in `nxs` rather than writing the
/// files itself; the assembler shells out to each active module's `agent-manifest` (TB-5).
fn assemble_agent_files(
    root: &std::path::Path,
    ws: &Workspace,
) -> Result<nxs_init::assembler::AssembleReport> {
    // A lone module assembles only the agent-file blocks it OWNS; an active sibling it does not link
    // (the umbrella does) has already assembled its own block via its own init — so resolve the
    // modules this binary knows and skip the rest, rather than erroring on an unlinked sibling.
    let roster = nxs_init::roster();
    let modules = nxs_init::resolve_known(&roster, &ws.config);
    let manifests = nxs_init::assembler::collect_manifests(&modules)?;
    nxs_init::assembler::assemble(root, &manifests)
}

/// One-line, human summary of what the assembler wrote to AGENTS.md and, since nxf 6j6v.q6e3,
/// what it cleaned back out of CLAUDE.md.
fn describe_onboarding(report: &nxs_init::assembler::AssembleReport) -> String {
    use nxs_init::assembler::{AgentsAction, ClaudeAction};
    let agents = match report.agents {
        AgentsAction::Created => "created AGENTS.md",
        AgentsAction::Augmented => "updated AGENTS.md",
    };
    let claude = match report.claude {
        ClaudeAction::Absent | ClaudeAction::Untouched => "",
        ClaudeAction::BlockRemoved => "; removed the retired block from CLAUDE.md",
        ClaudeAction::FileRemoved => "; removed CLAUDE.md, which held nothing else",
    };
    format!("{agents}{claude}")
}

/// Loudly flag the test-only `NXF_DETERMINISTIC_IDS` switch if it is somehow active in a
/// SHIPPED (release) binary at an interactive terminal. The switch forces a FIXED replica
/// identity (prefix `ab12`, site 1); two `init`s under it adopt the SAME identity, i.e. the
/// cross-replica prefix collision E4 sync exists to detect-and-remap — never what a real user
/// wants. The guards keep this from ever touching the harness: gated on `!debug_assertions`
/// (debug/test builds, where the switch is legitimate, stay quiet) and on stderr being a TTY
/// (the golden-docs harness and any pipe/CI run the switch ON non-interactively, so they never
/// see this — which also keeps trycmd's stderr surface clean). A real human who exported the
/// var by accident gets one loud line.
fn warn_if_deterministic_switch_live() {
    use std::io::IsTerminal;
    if !cfg!(debug_assertions)
        && workspace::deterministic_ids_enabled()
        && std::io::stderr().is_terminal()
    {
        eprintln!(
            "warning: NXF_DETERMINISTIC_IDS is set — forcing a fixed, non-unique replica \
             identity (test/docs only). Unset it for real workspaces to avoid id collisions."
        );
    }
}

/// Resolve which plugin to use, enforcing a conscious choice (4oa.4):
/// - an explicit `--plugin` is always honoured (scriptable, skips any prompt);
/// - otherwise, on a real terminal we run the shared `nxs-ui::chooser` widget (0lj.1);
/// - otherwise (non-TTY / `--json` / piped) we FAIL with the valid list + descriptions,
///   because silently defaulting the vocabulary is exactly what this ticket removes.
fn resolve_plugin_choice(json: bool, plugin: Option<&str>) -> Result<String> {
    if let Some(name) = plugin {
        return Ok(name.to_string());
    }
    // The SAME TTY gate the suite tool-picker uses (stdin AND stderr are terminals, via
    // `nxs-ui::chooser`): a human at a real terminal gets the shared arrow-key chooser, while
    // `--json`/pipes/CI fall through to the `--plugin`-required error below. `--json` callers are by
    // definition non-interactive even on a TTY, so the explicit `!json` keeps machine output from
    // ever being gated on a prompt. (nexus-flow-0lj.1)
    if !json && nxs_ui::chooser::is_interactive() {
        return prompt_for_plugin(plugin::available()?);
    }
    Err(NxfError::validation(format!(
        "--plugin is required (non-interactive); choose one of:\n{}",
        render_plugin_choices(&plugin::available()?)
    )))
}

/// Render the plugin options as a numbered, described list — the byte-stable fallback shown in the
/// non-TTY `--plugin`-required error (the interactive path now renders through `nxs-ui::chooser`,
/// nexus-flow-0lj.1). Pure (takes the configs, returns a string) so it is unit-testable without a
/// terminal. Each line: `  N) name — <EN description>`.
fn render_plugin_choices(plugins: &[PluginConfig]) -> String {
    plugins
        .iter()
        .enumerate()
        .map(|(i, p)| format!("  {}) {} — {}", i + 1, p.name, p.description.en))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Interactive plugin chooser via the shared `nxs-ui::chooser` widget (nexus-flow-0lj.1) — the SAME
/// arrow-key, ember-accented, manufakt-Forge widget the suite tool-picker uses, so the plugin step
/// is branded and operated identically instead of via the old bespoke numbered stdin reader
/// (4oa.4's "no TUI dependency" justification is obsolete: the shared widget is already in the tree
/// for the suite picker). Single-select with the cursor starting on the first plugin and NO
/// committed default — the choice stays a deliberate Enter, never silently defaulted (4oa.4). The
/// widget's own TTY gate means this is only reached on a real terminal; off one it returns the
/// shared `NotInteractive` refusal rather than reading a pipe (the non-TTY contract is the
/// `--plugin`-required error in [`resolve_plugin_choice`]).
fn prompt_for_plugin(plugins: Vec<PluginConfig>) -> Result<String> {
    let choices: Vec<nxs_ui::Choice> = plugins
        .iter()
        .map(|p| nxs_ui::Choice::new(p.name.clone(), p.description.en.clone()))
        .collect();
    let idx = nxs_ui::chooser::choose(
        "Choose a plugin (vocabulary + ranking for this workspace):",
        &choices,
        0,
    )
    .map_err(|e| match e {
        nxs_ui::ChooseError::Cancelled => {
            NxfError::validation("plugin choice cancelled — nothing was set up")
        }
        other => NxfError::io(other.to_string()),
    })?;
    plugins
        .get(idx)
        .map(|p| p.name.clone())
        .ok_or_else(|| NxfError::io(format!("chooser returned an out-of-range index {idx}")))
}

#[cfg(test)]
mod init_tests {
    use super::*;

    #[test]
    fn render_plugin_choices_is_numbered_and_described() {
        let rendered = render_plugin_choices(&plugin::available().unwrap());
        // Every shipped plugin appears, 1-based, with its EN description after an em-dash.
        for (i, p) in plugin::available().unwrap().iter().enumerate() {
            let expected = format!("  {}) {} — {}", i + 1, p.name, p.description.en);
            assert!(
                rendered.contains(&expected),
                "missing line for {}:\n{rendered}",
                p.name
            );
        }
        // First option is option 1 (no preselection / zero-indexing leaking out).
        assert!(rendered.starts_with("  1) "));
    }

    #[test]
    fn prompt_for_plugin_refuses_off_a_tty_via_the_shared_chooser_gate() {
        // nexus-flow-0lj.1: the plugin step now runs through the shared `nxs-ui::chooser` widget —
        // same arrow-key, ember-accented Forge UI AND the same TTY gate as the suite tool-picker.
        // Off a terminal (under `cargo test`) it must refuse with the shared chooser's
        // "no interactive terminal" message instead of reading a pipe. The user-facing non-TTY
        // contract (`--plugin` required + the numbered list) lives in `resolve_plugin_choice` and is
        // covered by the `init.rs` integration tests; this asserts the migration's gate.
        let err = prompt_for_plugin(plugin::available().unwrap()).unwrap_err();
        assert!(
            err.msg.contains("interactive terminal"),
            "plugin chooser uses the shared nxs-ui TTY gate, not a raw stdin read: {}",
            err.msg
        );
    }
}

#[cfg(test)]
mod markdown_helper_tests {
    use super::*;

    #[test]
    fn md_escape_inline_escapes_markdown_metacharacters_only() {
        // Backtick, angle brackets, and link brackets are escaped so free-text titles render
        // verbatim instead of breaking the surrounding Markdown.
        assert_eq!(
            md_escape_inline("Fix <Foo> parsing"),
            "Fix \\<Foo\\> parsing"
        );
        assert_eq!(md_escape_inline("use `nxf next`"), "use \\`nxf next\\`");
        assert_eq!(md_escape_inline("see [x](url)"), "see \\[x\\](url)");
        // Plain text (the common case, e.g. the golden titles) is untouched.
        assert_eq!(md_escape_inline("Ship v1"), "Ship v1");
        assert_eq!(md_escape_inline("in progress"), "in progress");
    }
}

// ---- create ----------------------------------------------------------------

/// Raw `create` inputs as the flags supplied them, before source resolution. Each long-text
/// field carries both its inline value and a `--<field>-file` path; the shared [`text_input`]
/// resolver picks the single source per field (95d.2). The non-long-text fields are taken
/// verbatim. Kept separate from the facade's [`NewItem`] (which holds already-resolved values).
pub struct CreateArgs<'a> {
    pub description: Option<&'a str>,
    pub description_file: Option<&'a str>,
    pub priority: Option<&'a str>,
    pub design: Option<&'a str>,
    pub design_file: Option<&'a str>,
    pub dod: Option<&'a str>,
    pub dod_file: Option<&'a str>,
    pub due: Option<&'a str>,
    pub defer: Option<&'a str>,
    pub parent: Option<&'a str>,
    pub depends_on: &'a [String],
    /// Plugin custom-field assignments (plugin-custom-fields §5), mirroring `update --set`: each a
    /// `name=value` (`--set`) or `name=path` (`--set-file`). Resolved through the SAME source seam
    /// as the long-text fields (inline | `-` STDIN | file), then handed to `write::create` which
    /// validates them against the resolved type.
    pub set: &'a [String],
    pub set_file: &'a [String],
}

impl CreateArgs<'_> {
    /// Any field-content flag set? Used to reject mixing the field flags with `--json -`, where
    /// the whole payload comes from STDIN (95d.3).
    fn any_field_set(&self) -> bool {
        self.description.is_some()
            || self.description_file.is_some()
            || self.priority.is_some()
            || self.design.is_some()
            || self.design_file.is_some()
            || self.dod.is_some()
            || self.dod_file.is_some()
            || self.due.is_some()
            || self.defer.is_some()
            || self.parent.is_some()
            || !self.depends_on.is_empty()
            || !self.set.is_empty()
            || !self.set_file.is_empty()
    }
}

/// The strict JSON shape `nxf create --json -` accepts (95d.3): the create field model keyed by
/// the same names as the flags. `deny_unknown_fields` makes an unexpected key a hard error, and
/// the four mandatory fields have no default so a missing one is rejected at parse time — both as
/// `validation`. Semantic validation (priority/type/dates/parent) stays in `write::create`.
///
/// The keys follow the CREATE FLAGS, which diverge from the read/`schema`/`update --set` spelling
/// for three fields (`dod`/`defer`/`parent` vs `completion_criterion`/`defer_until`/`belongs_to`),
/// and `priority` is a named label (`P0`…`P4`), not the read-form ordinal (3bw6). `nxf schema`
/// reports the exact payload key + priority value form per field, and a payload keyed with the
/// canonical name gets a targeted pointer instead of the bare `deny_unknown_fields` error.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateJson {
    #[serde(rename = "type")]
    ty: String,
    title: String,
    description: String,
    priority: String,
    #[serde(default)]
    design: Option<String>,
    #[serde(default)]
    dod: Option<String>,
    #[serde(default)]
    due: Option<String>,
    #[serde(default)]
    defer: Option<String>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    /// Plugin custom-field assignments as `name=value` strings (plugin-custom-fields §5), mirroring
    /// the `--set` flag. The JSON value is inline, so there is no `--set-file` analogue here. Empty
    /// unless the payload carries them; `write::create` validates each against the resolved type.
    #[serde(default)]
    set: Vec<String>,
}

/// `nxf create`: resolve the workspace, then apply the shared [`write::create`] op sequence
/// (E5 #9t7.5) — the same validation, minting, and ops the MCP server and an embedding app use.
/// `now`/`actor` are resolved here (the CLI's process clock + identity) and passed in explicitly.
///
/// The long-text fields (description/design/dod) may come from an inline value, the `-` STDIN
/// sentinel (95d.1), or a `--<field>-file` path (95d.2), so each is run through the shared
/// [`text_input`] resolver here — one [`StdinReader`] across all three enforces the
/// one-STDIN-reader rule, and a field given two sources is rejected — before the resolved values
/// reach the unchanged `write::create`.
#[allow(clippy::too_many_arguments)]
pub fn create(
    json: bool,
    db: Option<&str>,
    ty: Option<&str>,
    title: Option<&str>,
    raw: CreateArgs,
    json_stdin: Option<&str>,
    id_only: bool,
) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let now = resolve_now(None)?;
    let prefix = &ws.replica.prefix;

    let item = if text_input::json_stdin_requested(json_stdin)? {
        create_from_json(&mut store, &cfg, prefix, &now, ty, title, &raw)?
    } else {
        // Flag mode: resolve each long-text field's single source (inline | `-` STDIN | file),
        // one shared reader enforcing the one-STDIN rule. The mandatory non-long-text fields are
        // guaranteed present by clap's `required_unless_present`; the `ok_or_else` guards are a
        // defensive backstop should that ever be relaxed.
        let mut reader = StdinReader::new();
        let description = text_input::resolve(
            &mut reader,
            "description",
            raw.description,
            raw.description_file,
        )?
        .ok_or_else(|| NxfError::validation("description is required"))?;
        let design = text_input::resolve(&mut reader, "design", raw.design, raw.design_file)?;
        let dod = text_input::resolve(&mut reader, "dod", raw.dod, raw.dod_file)?;
        // Custom `--set`/`--set-file` pairs share the ONE reader (the long-text fields + these all
        // obey the single-STDIN-reader rule), resolved to `name=value` strings for `write::create`.
        let custom = resolve_sets(&mut reader, raw.set, raw.set_file)?;
        let ty = ty.ok_or_else(|| NxfError::validation("--type is required"))?;
        let title = title.ok_or_else(|| NxfError::validation("--title is required"))?;
        let priority = raw
            .priority
            .ok_or_else(|| NxfError::validation("--priority is required"))?;
        // A bare parent/dependency id resolves against the local prefix (ykv), so `--parent 0001`
        // works just like a positional id; a qualified (local or foreign) id passes through.
        let parent = raw
            .parent
            .map(|p| workspace::resolve_id(prefix, p).into_owned());
        let depends_on: Vec<String> = raw
            .depends_on
            .iter()
            .map(|d| workspace::resolve_id(prefix, d).into_owned())
            .collect();
        write::create(
            &mut store,
            &cfg,
            prefix,
            &now,
            &actor(),
            ty,
            title,
            NewItem {
                description: &description,
                priority,
                design: design.as_deref(),
                dod: dod.as_deref(),
                due: raw.due,
                defer: raw.defer,
                parent: parent.as_deref(),
                depends_on: &depends_on,
                custom: &custom,
            },
        )?
    };
    // `-q`/`--id-only` (nexus-flow-82h): print ONLY the new id (display form — bare for the local
    // item it always is), one line, for direct capture. It takes precedence over `--json`, which is
    // ignored rather than erroring — a documented precedence so a `-q --json` slip still captures.
    if id_only {
        println!("{}", ws.replica.display_id(&item.id));
    } else {
        emit_item(json, &ws.replica.prefix, &item, "created");
    }
    Ok(())
}

/// `nxf create --json -`: the whole item as one JSON object on STDIN (95d.3). Strict — unknown
/// keys and missing mandatory fields are rejected by [`CreateJson`]'s deserialization; the
/// resolved fields then go through the SAME `write::create` validation as the flag path, so there
/// is no second validation logic. The field flags are mutually exclusive with this mode.
fn create_from_json(
    store: &mut Store,
    cfg: &PluginConfig,
    prefix: &str,
    now: &str,
    ty: Option<&str>,
    title: Option<&str>,
    raw: &CreateArgs,
) -> Result<ItemRow> {
    if ty.is_some() || title.is_some() || raw.any_field_set() {
        return Err(NxfError::validation(
            "`-` reads the whole item from STDIN as a JSON object; the field flags \
             (--type/--title/--description/… ) are not allowed with it",
        ));
    }
    let payload = StdinReader::new().take()?;
    let value: serde_json::Value = serde_json::from_str(&payload)
        .map_err(|e| NxfError::validation(format!("invalid JSON create payload: {e}")))?;
    // 3bw6: a payload built from `show`/`schema` output can carry a canonical field name whose
    // create-payload key diverges. `deny_unknown_fields` would reject it with a bare "unknown field"
    // that names every accepted key but not the RIGHT one — so point at it explicitly first. The
    // divergent (canonical → payload) keys come from the facade schema report (y5j8), the single
    // source of truth, so this pointer and `nxf schema` can never drift.
    if let Some(obj) = value.as_object() {
        for f in &read::schema(cfg).fields {
            let Some(payload_key) = f.create_json_key.as_deref() else {
                continue;
            };
            let canonical = f.field.as_str();
            if payload_key != canonical && obj.contains_key(canonical) {
                return Err(NxfError::validation(format!(
                    "invalid JSON create payload: use '{payload_key}' (the create-payload key), not \
                     the canonical field name '{canonical}' — see `nxf schema`"
                )));
            }
        }
    }
    let p: CreateJson = serde_json::from_value(value)
        .map_err(|e| NxfError::validation(format!("invalid JSON create payload: {e}")))?;
    // Bare parent/dependency ids resolve against the local prefix (ykv), symmetric with the flag path.
    let parent = p
        .parent
        .map(|x| workspace::resolve_id(prefix, &x).into_owned());
    let depends_on: Vec<String> = p
        .depends_on
        .iter()
        .map(|d| workspace::resolve_id(prefix, d).into_owned())
        .collect();
    write::create(
        store,
        cfg,
        prefix,
        now,
        &actor(),
        &p.ty,
        &p.title,
        NewItem {
            description: &p.description,
            priority: &p.priority,
            design: p.design.as_deref(),
            dod: p.dod.as_deref(),
            due: p.due.as_deref(),
            defer: p.defer.as_deref(),
            parent: parent.as_deref(),
            depends_on: &depends_on,
            // Custom `name=value` sets ride the `set` payload key, mirroring the `--set` flag (ky26
            // review CQ2) — so a `required` custom field is creatable via `create --json -` too. The
            // shared `write::create` validates them against the resolved type.
            custom: &p.set,
        },
    )
}

// ---- update ----------------------------------------------------------------

pub fn update(
    json: bool,
    db: Option<&str>,
    id: &str,
    sets: &[String],
    set_files: &[String],
    json_stdin: Option<&str>,
) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let now = resolve_now(None)?;
    let id = ws.replica.resolve_id(id);
    // Resolve the source of every assignment (inline | `-` STDIN | `--set-file` path, or a whole
    // JSON object via `--json -`) into the canonical `field=value` strings the facade consumes.
    // One shared reader enforces the one-STDIN rule; a field set twice is a "two sources" error
    // here, never a partial write.
    let resolved = if text_input::json_stdin_requested(json_stdin)? {
        update_sets_from_json(sets, set_files)?
    } else {
        resolve_sets(&mut StdinReader::new(), sets, set_files)?
    };
    let item = write::update(&mut store, &cfg, &now, &actor(), &id, &resolved)?;
    emit_item(json, &ws.replica.prefix, &item, "updated");
    Ok(())
}

/// `nxf update <id> --json -`: read the fields to set as one JSON object on STDIN (95d.3) and map
/// each `"field": "value"` to the canonical `field=value` string [`write::update`] validates — the
/// SAME field whitelist, aliases, and per-field checks as the flag path, no second logic. Values
/// must be JSON strings (every updatable field is textual). The `--set`/`--set-file` flags are
/// mutually exclusive with this mode.
fn update_sets_from_json(sets: &[String], set_files: &[String]) -> Result<Vec<String>> {
    if !sets.is_empty() || !set_files.is_empty() {
        return Err(NxfError::validation(
            "`-` reads the fields to set from STDIN as a JSON object; --set/--set-file are not \
             allowed with it",
        ));
    }
    let payload = StdinReader::new().take()?;
    let obj: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&payload).map_err(|e| {
            NxfError::validation(format!(
                "invalid JSON update payload (expected an object): {e}"
            ))
        })?;
    let mut out = Vec::with_capacity(obj.len());
    for (field, value) in &obj {
        let s = value.as_str().ok_or_else(|| {
            NxfError::validation(format!("field '{field}' must be a JSON string"))
        })?;
        out.push(format!("{field}={s}"));
    }
    Ok(out)
}

/// Resolve `--set field=value` and `--set-file field=path` into the canonical `field=value`
/// strings [`write::update`] consumes. A right-hand side / path of `-` reads STDIN (95d.1); a
/// `--set-file` reads the file (95d.2). The `field=` prefix is preserved so the facade still owns
/// the field whitelist + per-field validation — there is no second validation path. Each field
/// may be assigned by exactly one source: a field appearing twice (in either flag) is rejected.
/// A `--set` entry with no `=` is passed through untouched so the facade emits the canonical
/// `expected field=value` error rather than this layer second-guessing it.
fn resolve_sets(
    reader: &mut StdinReader,
    sets: &[String],
    set_files: &[String],
) -> Result<Vec<String>> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(sets.len() + set_files.len());

    let mut claim = |field: &str| -> Result<()> {
        if seen.iter().any(|f| f == field) {
            return Err(NxfError::validation(format!(
                "field '{field}' is set more than once; assign each field exactly once \
                 (via --set or --set-file)"
            )));
        }
        seen.push(field.to_string());
        Ok(())
    };

    for raw in sets {
        match raw.split_once('=') {
            Some((field, value)) => {
                claim(field)?;
                let resolved = text_input::resolve(reader, field, Some(value), None)?
                    .expect("an inline source always resolves to Some");
                out.push(format!("{field}={resolved}"));
            }
            // No `=`: defer to the facade's canonical `expected field=value` error.
            None => out.push(raw.clone()),
        }
    }
    for raw in set_files {
        let (field, path) = raw.split_once('=').ok_or_else(|| {
            NxfError::validation(format!("expected field=path for --set-file, got '{raw}'"))
        })?;
        claim(field)?;
        let resolved = text_input::resolve(reader, field, None, Some(path))?
            .expect("a file source always resolves to Some");
        out.push(format!("{field}={resolved}"));
    }
    Ok(out)
}

// ---- claim / close ---------------------------------------------------------

/// Sugar: mark an item in progress (and optionally assign it).
pub fn claim(json: bool, db: Option<&str>, id: &str, assignee: Option<&str>) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let now = resolve_now(None)?;
    let id = ws.replica.resolve_id(id);
    let item = write::claim(&mut store, &now, &actor(), &id, assignee)?;
    emit_item(json, &ws.replica.prefix, &item, "claimed");
    Ok(())
}

/// Sugar: close an item with a mandatory closing comment. Closing is the only way an item leaves
/// the board (no hard delete), so a reason is always required (nexus-flow-ohh) — the missing-reason
/// check is in the shared write layer, before any write, so a rejected close changes nothing.
pub fn close(
    json: bool,
    db: Option<&str>,
    id: &str,
    reason: Option<&str>,
    reason_file: Option<&str>,
) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let now = resolve_now(None)?;
    let id = ws.replica.resolve_id(id);
    // `--reason -` reads the closing comment from STDIN (95d.1); `--reason-file` reads it from a
    // file (95d.2). A missing reason stays `None` so the facade still raises the mandatory-reason
    // error before any write; giving both `--reason` and `--reason-file` is a "two sources" error.
    let mut reader = StdinReader::new();
    let reason = text_input::resolve(&mut reader, "reason", reason, reason_file)?;
    let item = write::close(&mut store, &now, &actor(), &id, reason.as_deref())?;
    // 07a.5: closing a parent can push still-open children into the effective-closed lane (no open
    // parent left) without writing to them. Make that derived sweep loud — a sparse `swept_children`
    // join under `--json`, a warning line for humans — so the closer can re-open / follow up.
    if json {
        println!("{}", read::close_receipt_value(&store, &item)?);
    } else {
        println!(
            "closed {}",
            workspace::display_id(&ws.replica.prefix, &item.id)
        );
        let swept = read::swept_children(&store, &item.id)?;
        if !swept.is_empty() {
            println!(
                "{}",
                render_swept_warning(&ws.replica.prefix, &item.id, &swept)
            );
        }
    }
    Ok(())
}

/// The human close-warning (07a.5): closing a parent pushed these still-open children into the
/// effective-closed lane (no open parent left). Each is listed with its OTHER parents + status, so
/// the closer sees whether a swept child hangs anywhere still open and can re-open / follow up.
fn render_swept_warning(prefix: &str, parent_id: &str, swept: &[read::SweptChild]) -> String {
    let children: Vec<String> = swept
        .iter()
        .map(|c| {
            let title = c.title.as_deref().unwrap_or("");
            let others = if c.other_parents.is_empty() {
                String::new()
            } else {
                let list = c
                    .other_parents
                    .iter()
                    .map(|(pid, st)| {
                        format!(
                            "{} {}",
                            workspace::display_id(prefix, pid),
                            st.as_deref().unwrap_or("?")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(" (other parents: {list})")
            };
            format!("{} — {title}{others}", workspace::display_id(prefix, &c.id))
        })
        .collect();
    format!(
        "warning: you closed not only {} but also its children, now with no open parent: {}",
        workspace::display_id(prefix, parent_id),
        children.join("; ")
    )
}

// ---- archive / unarchive (C5 #916.5) ---------------------------------------

/// `nxf archive <ids…>`: archive each root + its closed subtree (cascade down). Partial batch —
/// always exits 0; the per-root outcomes and the full affected list are the contract.
pub fn archive_items(json: bool, db: Option<&str>, ids: &[String]) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let ids = resolve_ids(&ws, ids);
    let res = write::archive(&mut store, &resolve_now(None)?, &actor(), &ids)?;
    emit_batch(json, &ws.replica.prefix, &cfg, &res, "archived");
    Ok(())
}

/// `nxf unarchive <ids…>`: surface each item + its ancestor chain (cascade up only).
pub fn unarchive_items(json: bool, db: Option<&str>, ids: &[String]) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let ids = resolve_ids(&ws, ids);
    let res = write::unarchive(&mut store, &resolve_now(None)?, &actor(), &ids)?;
    emit_batch(json, &ws.replica.prefix, &cfg, &res, "unarchived");
    Ok(())
}

/// Resolve every id in a batch against the local prefix (ykv): a bare id gets the local prefix,
/// a qualified (local/foreign) id passes through. Owned, so it outlives the borrowed input slice.
fn resolve_ids(ws: &Workspace, ids: &[String]) -> Vec<String> {
    ids.iter()
        .map(|id| ws.replica.resolve_id(id).into_owned())
        .collect()
}

/// Render an archive/unarchive [`write::BatchResult`]. `--json` is the deterministic contract:
/// `{ "<verb>": [affected ids], "results": [{id, status, reason}] }` where `status` is the verb
/// success word (`archived`/`unarchived`) or `failed` with a `reason` code. The human view lists
/// the affected ids and each failure inline. `verb` is the success word + the affected-list key.
fn emit_batch(json: bool, prefix: &str, _cfg: &PluginConfig, res: &write::BatchResult, verb: &str) {
    if json {
        let results: Vec<serde_json::Value> = res
            .outcomes
            .iter()
            .map(|o| {
                serde_json::json!({
                    "id": o.id,
                    "status": if o.ok { verb } else { "failed" },
                    "reason": o.reason.map(|r| r.code()),
                })
            })
            .collect();
        let payload = serde_json::json!({
            verb: res.affected,
            "results": results,
        });
        println!("{payload}");
    } else {
        if res.affected.is_empty() {
            println!("nothing {verb}");
        } else {
            let affected: Vec<&str> = res
                .affected
                .iter()
                .map(|id| workspace::display_id(prefix, id))
                .collect();
            println!("{verb} {} ({} items)", affected.join(", "), affected.len());
        }
        for o in res.outcomes.iter().filter(|o| !o.ok) {
            let reason = o.reason.map(|r| r.code()).unwrap_or("failed");
            println!("failed {}: {reason}", workspace::display_id(prefix, &o.id));
        }
    }
}

// ---- prime -----------------------------------------------------------------

/// Attach bbq6's sparse declared `custom` join to `prime --json`'s `next` rows, in place. The rows
/// in `value["next"]` are `report.next` projected in order (`read::prime_next_value`), so they align
/// with the ids by index. A plugin with no declared `[fields]` (or a row with no set custom value)
/// leaves the row byte-identical — the same sparse contract `list`/`next --json` follow.
fn attach_prime_custom(
    cfg: &PluginConfig,
    store: &Store,
    report: &read::PrimeReport,
    value: &mut serde_json::Value,
) -> Result<()> {
    if cfg.fields.is_empty() {
        return Ok(());
    }
    let ids: Vec<&str> = report.next.iter().map(|r| r.item.id.as_str()).collect();
    let custom_by_id = read::declared_custom_by_id(cfg, store, &ids)?;
    if let Some(rows) = value
        .get_mut("next")
        .and_then(serde_json::Value::as_array_mut)
    {
        // Both the `next` json rows and `ids` project `report.next` in order, so `zip` pairs each
        // row with its own id. Assert it in debug builds — a future divergence would otherwise
        // silently cross-wire custom fields rather than fail.
        debug_assert_eq!(
            rows.len(),
            ids.len(),
            "attach_prime_custom: prime next rows/ids length mismatch ({} vs {})",
            rows.len(),
            ids.len(),
        );
        for (row, id) in rows.iter_mut().zip(ids.iter()) {
            if let Some(custom) = custom_by_id.get(*id) {
                row["custom"] = custom.clone();
            }
        }
    }
    Ok(())
}

/// `nxf prime`: a plugin-determined purpose, workflow rules, the real `next` recommendation, a
/// leverage-aware blocked snapshot, a create example, and a command reference. The record is
/// computed by [`read::prime`] (shared with the MCP server and embedding apps); `--json` renders
/// it in full, the canonical record other consumers read. The human session-start layout is a
/// CURATED SUBSET of the same record (nxf xe2z, task-2 brief): the SessionStart hook this command
/// backs has a hard host-side output ceiling, so the human view keeps only what no `--help`/`nxf
/// guide` page can teach and drops the rest — see the long comment at the top of the `else`
/// branch below for the full accounting. The truth lives in the facade; the SessionStart hook
/// only wraps this command.
pub fn prime(json: bool, db: Option<&str>) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let store = ws.open_store()?;
    let now = resolve_now(None)?;
    // `bound` (does `.nxs/sync.toml` exist?) gates the sync section + the session-close
    // sync step. The facade can't see the workspace filesystem, so the CLI passes it
    // (nexus-flow-vux); the same record then drives both `--json` and the human view.
    let bound = crate::sync::is_bound(&ws);
    let report = read::prime(&cfg, &store, &now, bound)?;

    if json {
        let mut value = report.to_value();
        // bbq6: attach the same sparse declared `custom` join `list`/`next --json` carry to prime's
        // `next` rows. Done CLI-side (like the ee2h labels) so prime's public `PrimeReport` seam stays
        // untouched; sparse + declared-only, so a plugin without `[fields]` stays byte-identical.
        attach_prime_custom(&cfg, &store, &report, &mut value)?;
        println!("{}", value);
    } else {
        // **Superseded 2026-08-28 (nxf xe2z, task 2).** This branch used to open with: "The human
        // view is valid, structured Markdown (nexus-flow-7s6/0b4): hosts render the SessionStart-
        // hook output as Markdown. It renders the SAME record as `--json` 1:1 — the sync hint,
        // session-close steps, and long-text pointer all come from `report`, so the two views
        // never drift (nexus-flow-vux)." That was true when written. It is no longer true: the
        // human view is now a CURATED SUBSET of `report`, not a 1:1 mirror of `--json`. Why: every
        // hook output above 10.240 B is silently dropped by the host rather than delivered smaller
        // (task-2 brief), and this human view IS that hook's payload; `nxf prime`'s fixed prose
        // alone used to be 5.570 B of the SessionStart hook's ~9.719 B total output (NOT "composed"
        // — that word is reserved elsewhere in this codebase for the `nxs` fan-out of all three
        // modules together; this is `nxf`'s own contribution to it). xe2z's brief: keep what no
        // `--help`/`nxf guide` page can teach (the plugin predicate, the workspace prefix — without
        // it the ids in `## Next`/`## Blocked` below are not resolvable — a six-line command
        // cheatsheet, and three rules), drop what an agent can already pull on demand. The dropped
        // content (the Context Recovery line, the full purpose+derivation sentence, all nine Core
        // Rules, Essential Commands, Common Workflows, Create's long-text hint + recommendation,
        // the IDs section's `-q`/`--jq` aside, Sync, Session close) is **superseded, not deleted**:
        // `report` still carries every field verbatim and the `if json` branch above still renders
        // all of it — `prime --json` is the stable canonical record the MCP server and embedding
        // apps read (see `PrimeReport`'s doc comment, itself updated to match), so only this human
        // branch shrank. `nxf mention add` drops from the human view without replacement — a
        // deliberate loss the project owner signed off on (task-2 brief), not an oversight.
        //
        // Global constraint (q3fh session-start-budget plan): this block must read on its own —
        // nothing here names `nxs`/`nxc`/`nxm`'s own blocks, because the order hook outputs reach a
        // session is not guaranteed (a foreign hook has been observed landing between two of ours).
        println!("# nexus-flow — the board\n");
        println!(
            "`nxf` is {}. You record items, the dependencies between them, due/defer dates and \
             priority; `next` and `blocked` are derived from that graph rather than stored, so \
             the work list is always consistent. It is the single source of truth for this \
             project's work — open an item rather than keeping a side TODO list or scratch \
             notes. `nxf --help` lists the commands, `nxf guide` the topics behind them.\n",
            read::prime_tagline(&cfg)
        );
        println!(
            "Item ids are `<prefix>.<suffix>` and this workspace's prefix is `{}`. Name a local \
             item by its bare suffix, a foreign one by its full id.\n",
            ws.replica.prefix
        );

        // #yod: a non-silent, prominent hint at session start when the workspace holds item types
        // the active plugin doesn't declare (a pre-plugin workspace's legacy core-enum values). The
        // supported repair is the retype primitive. Human view only — the `--json` prime contract is
        // untouched (the record stays plugin-/workspace-static).
        let undeclared = read::undeclared_types(&cfg, &store)?;
        if !undeclared.is_empty() {
            println!(
                "> ⚠ **Legacy item types** — the active plugin does not declare: {}. These items \
                 show their raw type and reject parent-edge writes. Repair each with \
                 `nxf update <id> --set type=<declared>` (suggested: project→epic, task→feature).\n",
                undeclared.join(", ")
            );
        }

        // `## How work moves` (nxf xe2z): a copy-pasteable six-line cheatsheet that replaces the
        // old separate `## Create` section AND the Essential Commands/Common Workflows reference.
        // `create_example` is the one plugin-aware line (the active config's type + priority
        // vocabulary, same string `--json`'s `create.example` carries); the rest is fixed,
        // vocabulary-generic prose an agent can run as shown. Rendered as 4-space-indented lines,
        // NOT a fenced ` ```bash ` block — carried over from the old Common Workflows renderer this
        // replaces (`read::Workflow`'s doc comment): an embedded ``` fence here would collide with
        // the golden test harness's own ` ```console ` fence around the whole example
        // (`crates/cli/tests/golden/issue-tracker/prime.trycmd`), corrupting the fixture. Keep this
        // block indented, not fenced, if you ever reach for tidying it.
        println!("## How work moves\n");
        println!(
            "    nxf show <id>                        # one item, with its dependencies and notes"
        );
        println!("    nxf claim <id>                       # take it before you start");
        println!("    nxf note add <id> \"<what you learned>\"");
        println!("    nxf close <id> --reason \"<why it is done>\"\n");
        println!("    {}", report.create_example);
        println!("    nxf dep add <child> <prereq>         # prereq must close first\n");

        // The three Core Rules no `--help`/`nxf guide` page can teach on its own (nxf xe2z): the
        // gating `parent` vs. non-gating `contributes_to` edge (07a.4), the defer-is-a-real-date
        // vs. WAIT-chore convention (oxmu), and the fields-settle-then-append-only-notes discipline
        // (8qv.6/8qv.9) — each condensed here to its essential shape with a `nxf guide` pointer for
        // the worked example. The other five entries of `PrimeReport::rules` (find-next, claim,
        // close-with-reason, `--json` everywhere, and mention-on-citation, which drops without
        // replacement per the task-2 brief) are things `--help` already shows or, for mention, a
        // deliberate loss — see this branch's opening comment. `report.rules` itself, and `--json`'s
        // `rules` array, are UNCHANGED: all nine entries still ride the canonical record.
        println!("**Three rules `--help` will not teach you:**\n");
        println!(
            "- `parent` gates, `contributes_to` does not — a child rests when its parent rests, \
             so a deferred or closed parent hides its children. (`nxf guide core-concepts`)"
        );
        println!(
            "- `--defer` takes a real calendar date, never a placeholder for \"someday, once X \
             ships\". To wait on a delivery, open a `WAIT: <what ships>` chore and have the \
             dependents `dep add` onto it. (`nxf guide deferring-and-waiting`)"
        );
        println!(
            "- Fields settle shortly after creation; what you learn later goes into append-only \
             notes, so title-versus-close-reason stays an honest intent-vs-outcome pair."
        );

        // `next`: the real recommendation, already truncated; when the candidate set exceeds the
        // shown rows the heading names the true total (else just the count).
        if report.next_total > report.next.len() {
            println!(
                "\n## Next (showing {} of {})\n",
                report.next.len(),
                report.next_total
            );
        } else {
            println!("\n## Next ({})\n", report.next_total);
        }
        if report.next.is_empty() {
            println!("_Nothing ready — create work, or unblock something below._");
        } else {
            print!("{}", render_next_md(&cfg, &ws.replica.prefix, &report.next));
        }

        // `blocked`: each blocked item with its open blockers and their leverage.
        println!("\n## Blocked ({})", report.blocked.len());
        if report.blocked.is_empty() {
            println!("\n_Nothing blocked._");
        } else {
            println!();
            for entry in &report.blocked {
                let blockers: Vec<String> = entry
                    .blockers
                    .iter()
                    .map(|b| {
                        format!(
                            "`{}` ({}, blocks {})",
                            workspace::display_id(&ws.replica.prefix, &b.id),
                            label(&cfg, "status", &b.status),
                            b.blocks_count
                        )
                    })
                    .collect();
                let id = workspace::display_id(&ws.replica.prefix, &entry.id);
                if blockers.is_empty() {
                    println!("- `{id}`");
                } else {
                    println!("- `{id}` — blocked by {}", blockers.join(", "));
                }
            }
        }

        // `recently_closed` (r4kb): the recency recall glance — what got finished lately, one
        // compact line each (`id · title · closing note`, the note collapsed + capped). Renders
        // straight after Blocked. A 1:1 mirror of `recently_closed` in `--json` — both read the same
        // capped record, so the two views never drift.
        println!("\n## Recently Closed ({})", report.recently_closed.len());
        if report.recently_closed.is_empty() {
            println!("\n_Nothing closed recently._");
        } else {
            println!();
            for entry in &report.recently_closed {
                let id = workspace::display_id(&ws.replica.prefix, &entry.id);
                println!(
                    "- `{id}` · {} · {}",
                    md_escape_inline(entry.title.as_deref().unwrap_or("")),
                    md_escape_inline(&entry.closing_notes)
                );
            }
        }

        // The human view ENDS here (nxf xe2z): `## Recently Closed` above is the last section.
        // The old `## Create` (beyond the example already folded into `## How work moves`),
        // `## IDs` (beyond the prefix paragraph already folded above), `## Essential Commands`,
        // `## Common Workflows`, `## Sync`, and `## Session close` sections are gone from THIS
        // view — `report.create_recommendation`, `report.create_long_text_hint`,
        // `report.commands`, `report.workflows`, `report.sync`, and `report.session_close` are
        // still computed above and still rendered in the `if json` branch; they are simply not
        // read again down here. See this branch's opening comment for the full accounting.
    }
    Ok(())
}

// ---- agent-manifest --------------------------------------------------------

/// `nxf agent-manifest`: emit nxf's declared contribution to the shared agent file (spec §6.2 /
/// nexus-flow-aye.10) — the prime fan-out command and the SessionStart hook — as data. Since
/// nexus-flow-0lj.2 nxf declares no per-module Markdown block: the umbrella assembler writes a single
/// nxs-owned AGENTS.md discovery pointer instead. The contribution is static (no workspace, no plugin
/// dependency), so this runs anywhere. `--json` is the contract the `nxs` umbrella (P3) consumes; the
/// human form is a readable summary.
pub fn agent_manifest(json: bool) -> Result<()> {
    let manifest = onboarding::manifest();
    if json {
        // Serialize the typed struct directly (NOT via `to_value`, whose `Map` sorts keys): the
        // struct's declaration order IS the contract, guarded by the foundation field-order test.
        let value = serde_json::to_string(&manifest)
            .map_err(|e| NxfError::io(format!("serializing manifest: {e}")))?;
        println!("{value}");
    } else {
        println!("nexus-flow agent manifest");
        println!("  prime command:  {}", manifest.prime_command);
        println!(
            "  hook:           {} → {}",
            manifest.hook.event, manifest.hook.command
        );
        println!("\nRun with --json for the machine contract the `nxs` umbrella assembles from.");
    }
    Ok(())
}

// ---- setup -----------------------------------------------------------------

/// `nxf setup claude`: wire Claude Code host integration for this workspace. Since nexus-flow-5od the
/// verb's ONE implementation lives on the `nxs` umbrella — host setup is an umbrella responsibility
/// (the umbrella owns the whole SessionStart set — one hook per active module since nxf n2m6 +
/// a2a1) — so this persona keeps a
/// working verb but carries NO own wiring path: it re-execs the sibling `nxs setup claude`, exactly
/// as `nxf init` delegates to `nxs init` (`nxs_init::frontdoor`). That keeps the two from ever
/// drifting and means the umbrella owns workspace resolution, the agent-file (re)assembly, and the
/// hook + allowlist wiring — including regenerating a deleted AGENTS.md (nexus-flow-6c4).
pub fn setup_claude(json: bool) -> Result<()> {
    nxs_init::frontdoor::reexec_umbrella_setup_claude(json)
}

// ---- blocked ---------------------------------------------------------------

pub fn blocked(json: bool, db: Option<&str>, sort: Option<&str>) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let store = ws.open_store()?;
    let sort = sort.map(read::parse_sort).transpose()?;
    let rows = read::blocked(&cfg, &store, sort)?;
    if json {
        // Each blocked item is its canonical record plus a structured `blockers` list (id +
        // core status), so an agent sees *which* open blocker holds it without a follow-up
        // call (nexus-flow-97b). The top-level `id` is preserved for existing consumers.
        // bbq6: the sparse declared `custom` join rides each record (seam-shared with flow_blocked).
        // yfwt: the additive priority_label/type_label decorate each record (CLI-only, like list/next).
        println!(
            "{}",
            with_presentation_labels(
                &cfg,
                read::blocked_to_value_with_custom(&cfg, &store, &rows)?
            )
        );
    } else {
        print!("{}", render_blocked(&cfg, &ws.replica.prefix, &rows));
    }
    Ok(())
}

/// Human `blocked` view: the plugin's list row for each blocked item, with its open blockers
/// (id + status in the plugin's vocabulary) named inline so the "why" needs no follow-up call.
fn render_blocked(cfg: &PluginConfig, prefix: &str, rows: &[read::BlockedItem]) -> String {
    let mut out = String::new();
    for row in rows {
        out.push_str(
            render_list(cfg, prefix, std::slice::from_ref(&row.item)).trim_end_matches('\n'),
        );
        if !row.blockers.is_empty() {
            let rendered: Vec<String> = row
                .blockers
                .iter()
                .map(|(b, st)| {
                    format!(
                        "{} ({})",
                        workspace::display_id(prefix, b),
                        label(cfg, "status", st)
                    )
                })
                .collect();
            out.push_str(&format!("\n    ↳ blocked by: {}", rendered.join(", ")));
        }
        out.push('\n');
    }
    out
}

// ---- next ------------------------------------------------------------------

pub fn next(
    json: bool,
    db: Option<&str>,
    now: Option<&str>,
    label: Option<&str>,
    sort: Option<&str>,
    limit: Option<usize>,
) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let store = ws.open_store()?;
    let now = resolve_now(now)?;
    let sort = sort.map(read::parse_sort).transpose()?;
    let mut items = read::next(&cfg, &store, &now, sort)?;
    if let Some(label) = label {
        items = read::with_label(&store, items, label)?;
    }
    // 6j6v.8pf2: the cut is LAST — after the ranking and after `--label` — and it goes through the
    // shared mechanism `prime` uses, so the disclosure of the untruncated total cannot drift apart
    // between the two callers.
    let page = read::truncate_next(items, limit);

    // Emit in the chosen order (default rank) — do NOT re-sort by id. Each record/row carries its
    // resolved parent join (#916.7).
    if json {
        // ee2h: additive priority_label/type_label alongside the canonical record (CLI-only).
        let mut value = with_presentation_labels(
            &cfg,
            read::next_to_value_with_custom(&cfg, &store, &page.items)?,
        );
        // 6j6v.srpg: the retrieval rule's hint cell, machine form — the SAME join the MCP
        // `flow_next` tool attaches, so the two seams cannot drift.
        let ids: Vec<&str> = page.items.iter().map(|i| i.id.as_str()).collect();
        crate::memories::attach_counts_to_next(&ws, &ids, &mut value)?;
        // Without `--limit` the payload is the bare array agents read today — untouched. `--limit`
        // is the opt-in to the envelope: the flag that can cut is the flag that owes the reader a
        // total, and a consumer must never have to infer truncation from the array's length. The
        // shape follows the FLAG, not the data, so it does not change under a consumer's feet when
        // the board shrinks past the limit.
        if limit.is_some() {
            println!(
                "{}",
                serde_json::json!({ "items": value, "total": page.total })
            );
        } else {
            println!("{value}");
        }
    } else {
        // The one place the human `next` view learns whether it may paint at all: a real terminal
        // and not `--json`. Everything downstream just carries the answer (6j6v.8bax).
        let theme = nxs_ui::Theme::detect(json);
        print!("{}", render_next(&cfg, &ws, &store, &page, &theme)?);
    }
    Ok(())
}

// ---- list ------------------------------------------------------------------

pub fn list(
    json: bool,
    db: Option<&str>,
    status: Option<&str>,
    item_type: Option<&str>,
    label: Option<&str>,
    sort: Option<&str>,
) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let store = ws.open_store()?;
    let sort = sort.map(read::parse_sort).transpose()?;
    let mut items = read::list(&cfg, &store, status, item_type, sort)?;
    if let Some(label) = label {
        items = read::with_label(&store, items, label)?;
    }

    // Emit in the chosen order (default rank) — preserve it into JSON, do NOT re-sort by id. Each
    // record carries a sparse `parent_closed_reason` join for a closed-masked open child (07a.3 §5).
    if json {
        // ee2h: additive priority_label/type_label alongside the canonical record (CLI-only).
        println!(
            "{}",
            with_presentation_labels(&cfg, read::list_to_value_with_custom(&cfg, &store, &items)?)
        );
    } else {
        print!("{}", render_list(&cfg, &ws.replica.prefix, &items));
    }
    Ok(())
}

// ---- lane verbs (C3 #916.4) ------------------------------------------------

/// `nxf deferred`: the deferred lane — open, unblocked work with a future defer date, ordered by
/// defer-date ascending. `--now` sets the defer boundary; `--sort` overrides the order.
pub fn deferred(json: bool, db: Option<&str>, now: Option<&str>, sort: Option<&str>) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let store = ws.open_store()?;
    let now = resolve_now(now)?;
    let sort = sort.map(read::parse_sort).transpose()?;
    let items = read::deferred(&cfg, &store, &now, sort)?;
    // Emit in the chosen order — preserve it into JSON, do NOT re-sort by id.
    if json {
        // bbq6: the sparse declared `custom` join rides each record, identical to list/next --json.
        println!(
            "{}",
            read::items_in_order_value_with_custom(&cfg, &store, &items)?
        );
    } else {
        print!("{}", render_list(&cfg, &ws.replica.prefix, &items));
    }
    Ok(())
}

/// `nxf closed`: the closed lane (archived excluded), ordered by close-date descending (recency).
pub fn closed(json: bool, db: Option<&str>, sort: Option<&str>) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let store = ws.open_store()?;
    let sort = sort.map(read::parse_sort).transpose()?;
    let items = read::closed(&cfg, &store, sort)?;
    if json {
        // bbq6: the sparse declared `custom` join rides each record, identical to list/next --json.
        println!(
            "{}",
            read::items_in_order_value_with_custom(&cfg, &store, &items)?
        );
    } else {
        print!("{}", render_list(&cfg, &ws.replica.prefix, &items));
    }
    Ok(())
}

/// `nxf archived`: the archived lane (any status), ordered by archive-date descending (recency).
pub fn archived(json: bool, db: Option<&str>, sort: Option<&str>) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let store = ws.open_store()?;
    let sort = sort.map(read::parse_sort).transpose()?;
    let items = read::archived(&cfg, &store, sort)?;
    if json {
        // bbq6: the sparse declared `custom` join rides each record, identical to list/next --json.
        println!(
            "{}",
            read::items_in_order_value_with_custom(&cfg, &store, &items)?
        );
    } else {
        print!("{}", render_list(&cfg, &ws.replica.prefix, &items));
    }
    Ok(())
}

/// The default `nxf recap` window when `--limit` is omitted.
const RECAP_DEFAULT_LIMIT: usize = 10;

/// `nxf recap`: the recency recall view (r4kb) — the most-recently CLOSED items, archived INCLUDED,
/// newest first, capped to `--limit` (default 10). Distinct from `nxf closed` (the lane, archived
/// excluded): its own code path over the shared [`read::recap`] compute — `nxf closed` is untouched.
/// `--json` emits the full, uncapped records in recency order (the consumer truncates); the text view
/// is the compact recall line (`id · title · note≤280…`) sharing its shape + [`read::recap_note`]
/// helper with the prime "Recently Closed" section, so the two never drift.
pub fn recap(
    json: bool,
    db: Option<&str>,
    limit: Option<usize>,
    since: Option<&str>,
) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let store = ws.open_store()?;
    // `--since` is validated inside `read::recap` (the shared facade gate), so a malformed date is a
    // loud error for every caller — the CLI and any embedder alike — not a silently-empty result.
    let items = read::recap(
        &cfg,
        &store,
        Some(limit.unwrap_or(RECAP_DEFAULT_LIMIT)),
        since,
    )?;
    if json {
        // Full, uncapped records in recency order (like `nxf closed --json`) — the consumer caps.
        println!(
            "{}",
            read::items_in_order_value_with_custom(&cfg, &store, &items)?
        );
    } else {
        print!("{}", render_recap(&ws.replica.prefix, &items));
    }
    Ok(())
}

/// The `nxf recap` text view: one compact recall line per item — `- id · title · note` — the title
/// escaped and the note collapsed + capped via the shared [`read::recap_note`], identical in shape to
/// the prime "Recently Closed" section. Empty ⇒ a single friendly line.
fn render_recap(prefix: &str, items: &[ItemRow]) -> String {
    if items.is_empty() {
        return "_Nothing closed recently._\n".to_string();
    }
    let mut out = String::new();
    for item in items {
        let id = workspace::display_id(prefix, &item.id);
        let (notes, _) = read::recap_note(item.closing_comment.as_deref(), read::RECAP_NOTES_MAX);
        out.push_str(&format!(
            "- `{id}` · {} · {}\n",
            md_escape_inline(item.title.as_deref().unwrap_or("")),
            md_escape_inline(&notes)
        ));
    }
    out
}

// ---- search ----------------------------------------------------------------

/// The `nxf search` flags beyond the query (C6 #916.6).
pub struct SearchArgs<'a> {
    pub status: Option<&'a str>,
    pub item_type: Option<&'a str>,
    pub now: Option<&'a str>,
    pub include_archived: bool,
    pub archived_only: bool,
    pub sort: Option<&'a str>,
}

/// `nxf search`: lane-ranked substring search (C6 #916.6) — computed by the shared facade
/// [`read::search`], so the CLI and any in-process consumer return the identical grouping. The
/// result is already in lane-grouped (or `--sort`-flattened) order; the `--json` and human views
/// emit it in that order, NOT re-sorted by id.
pub fn search(json: bool, db: Option<&str>, query: &str, args: SearchArgs) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let store = ws.open_store()?;
    let now = resolve_now(args.now)?;
    let sort = args.sort.map(read::parse_sort).transpose()?;
    let items = read::search(
        &cfg,
        &store,
        &now,
        query,
        read::SearchArgs {
            status: args.status,
            item_type: args.item_type,
            include_archived: args.include_archived,
            archived_only: args.archived_only,
            sort,
        },
    )?;

    if json {
        // yfwt: the additive priority_label/type_label decorate each record (CLI-only, like list/next).
        // Custom fields are deliberately NOT added here (bbq6 scopes custom to the lane lists + prime).
        println!(
            "{}",
            with_presentation_labels(&cfg, read::items_in_order_value(&items))
        );
    } else {
        print!("{}", render_list(&cfg, &ws.replica.prefix, &items));
    }
    Ok(())
}

// ---- dep -------------------------------------------------------------------

/// Add a `from -> to` dependency (from depends on to). Cycle rejection + existence checks live in
/// the shared write layer ([`write::dep_add`]); a cycle is a loud error and writes nothing.
pub fn dep_add(json: bool, db: Option<&str>, from: &str, to: &str) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let from = ws.replica.resolve_id(from);
    let to = ws.replica.resolve_id(to);
    write::dep_add(&mut store, &resolve_now(None)?, &actor(), &from, &to)?;
    emit_edge(json, &ws.replica.prefix, &from, &to, |a, b| {
        format!("{a} -> {b}")
    });
    Ok(())
}

pub fn dep_remove(json: bool, db: Option<&str>, from: &str, to: &str) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let from = ws.replica.resolve_id(from);
    let to = ws.replica.resolve_id(to);
    write::dep_remove(&mut store, &resolve_now(None)?, &actor(), &from, &to)?;
    emit_edge(json, &ws.replica.prefix, &from, &to, |a, b| {
        format!("removed {a} -> {b}")
    });
    Ok(())
}

// ---- mention (reference edge, bm0/§8) --------------------------------------

/// Record a `from -> to` reference edge: `from`'s free text cites the short-id `to`. Pure
/// reference, no blocking semantics (it is not a `dep`); a self-reference is permitted. Both
/// endpoints must exist (checked in [`write::mention_add`]).
pub fn mention_add(json: bool, db: Option<&str>, from: &str, to: &str) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let from = ws.replica.resolve_id(from);
    let to = ws.replica.resolve_id(to);
    write::mention_add(&mut store, &resolve_now(None)?, &actor(), &from, &to)?;
    emit_edge(json, &ws.replica.prefix, &from, &to, |a, b| {
        format!("{a} mentions {b}")
    });
    Ok(())
}

/// Remove a `from -> to` reference edge (observed-remove).
pub fn mention_remove(json: bool, db: Option<&str>, from: &str, to: &str) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let from = ws.replica.resolve_id(from);
    let to = ws.replica.resolve_id(to);
    write::mention_remove(&mut store, &resolve_now(None)?, &actor(), &from, &to)?;
    emit_edge(json, &ws.replica.prefix, &from, &to, |a, b| {
        format!("removed mention {a} -> {b}")
    });
    Ok(())
}

/// List the short-ids `id` mentions (sorted, deterministic).
pub fn mention_list(json: bool, db: Option<&str>, id: &str) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let store = ws.open_store()?;
    let id = ws.replica.resolve_id(id);
    let mentions = read::mentions(&store, &id)?;
    if json {
        println!("{}", read::mentions_to_value(&mentions));
    } else {
        for m in &mentions {
            println!("{}", workspace::display_id(&ws.replica.prefix, m));
        }
    }
    Ok(())
}

// ---- thread links (nxf 6j6v.8dbe) ------------------------------------------

/// Parse a `--relation` value, or a `validation` error listing the accepted tokens — the same loud
/// treatment every other closed vocabulary in this CLI gets, rather than silently defaulting.
fn parse_relation(value: &str) -> Result<LinkRelation> {
    LinkRelation::parse(value).ok_or_else(|| {
        NxfError::validation(format!(
            "unknown relation '{value}'; expected 'worked_on' or 'cited'"
        ))
    })
}

/// Parse a `--weight` value. Mirrors [`parse_relation`].
fn parse_weight(value: &str) -> Result<LinkWeight> {
    LinkWeight::parse(value).ok_or_else(|| {
        NxfError::validation(format!(
            "unknown weight '{value}'; expected 'bearing' or 'passing'"
        ))
    })
}

/// Link a chat thread to a board item with a relation and a weight.
pub fn thread_link(
    json: bool,
    db: Option<&str>,
    thread: &str,
    item: &str,
    relation: &str,
    weight: &str,
) -> Result<()> {
    let relation = parse_relation(relation)?;
    let weight = parse_weight(weight)?;
    // Trimmed ONCE, up front, and used on every output path (PR #273 review, Code Quality #2). The
    // facade normalizes the id it stores; printing the raw argument would let `--json` and the human
    // line disagree about what was just linked, which the deterministic-output contract forbids.
    let thread = thread.trim();
    let (ws, _cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let item = ws.replica.resolve_id(item);
    write::thread_link_add(
        &mut store,
        &resolve_now(None)?,
        &actor(),
        thread,
        &item,
        relation,
        weight,
    )?;
    // `--json` carries the FULL id (the machine contract), the human line the display form — the
    // same split `emit_edge`/`emit_label` make, so every receipt in this CLI answers "which id do I
    // get back?" the same way.
    if json {
        println!(
            "{}",
            serde_json::json!({
                "thread_id": thread,
                "item_id": item,
                "relation": relation.as_str(),
                "weight": weight.as_str(),
            })
        );
    } else {
        println!(
            "linked {thread} -> {} ({}, {})",
            workspace::display_id(&ws.replica.prefix, &item),
            relation.as_str(),
            weight.as_str()
        );
    }
    Ok(())
}

/// Unlink a thread from an item (observed-remove), whatever attributes the link carries.
pub fn thread_unlink(json: bool, db: Option<&str>, thread: &str, item: &str) -> Result<()> {
    let thread = thread.trim(); // see `thread_link` — one trimmed value on every output path
    let (ws, _cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let item = ws.replica.resolve_id(item);
    write::thread_link_remove(&mut store, &resolve_now(None)?, &actor(), thread, &item)?;
    if json {
        println!(
            "{}",
            serde_json::json!({ "thread_id": thread, "item_id": item, "unlinked": true })
        );
    } else {
        println!(
            "unlinked {thread} -> {}",
            workspace::display_id(&ws.replica.prefix, &item)
        );
    }
    Ok(())
}

/// List the board items a thread is linked to (sorted by item id, deterministic).
pub fn thread_list(json: bool, db: Option<&str>, thread: &str) -> Result<()> {
    let thread = thread.trim(); // see `thread_link`
    let (ws, _cfg) = open(db)?;
    let store = ws.open_store()?;
    let links = read::thread_items(&store, thread)?;
    if json {
        // Full ids, exactly as `mention list --json` emits them — see `thread_link` for the split.
        println!("{}", read::thread_links_to_value(&links));
    } else {
        for l in &links {
            println!(
                "{} ({}, {})",
                workspace::display_id(&ws.replica.prefix, &l.item_id),
                l.relation.as_str(),
                l.weight.as_str()
            );
        }
    }
    Ok(())
}

// ---- contributes-to (structural n:m relation, vf4) -------------------------

/// Record a `from -> to` contributes-to edge: `from` contributes to `to` (n:m, nexus-flow-vf4).
/// Like a `mention` it never blocks (forms no cycle, never appears in `blocked`); both endpoints
/// must exist (checked in [`write::contributes_add`]), and a self-edge is permitted.
pub fn contributes_add(json: bool, db: Option<&str>, from: &str, to: &str) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let from = ws.replica.resolve_id(from);
    let to = ws.replica.resolve_id(to);
    write::contributes_add(&mut store, &resolve_now(None)?, &actor(), &from, &to)?;
    emit_edge(json, &ws.replica.prefix, &from, &to, |a, b| {
        format!("{a} contributes to {b}")
    });
    Ok(())
}

/// Remove a `from -> to` contributes-to edge (observed-remove).
pub fn contributes_remove(json: bool, db: Option<&str>, from: &str, to: &str) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let from = ws.replica.resolve_id(from);
    let to = ws.replica.resolve_id(to);
    write::contributes_remove(&mut store, &resolve_now(None)?, &actor(), &from, &to)?;
    emit_edge(json, &ws.replica.prefix, &from, &to, |a, b| {
        format!("removed contributes {a} -> {b}")
    });
    Ok(())
}

/// List the items `id` contributes to (sorted, deterministic).
pub fn contributes_list(json: bool, db: Option<&str>, id: &str) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let store = ws.open_store()?;
    let id = ws.replica.resolve_id(id);
    require_item(&store, &id)?;
    let contributes = store.contributes_to_of(&id)?;
    if json {
        println!("{}", serde_json::json!(contributes));
    } else {
        for c in &contributes {
            println!("{}", workspace::display_id(&ws.replica.prefix, c));
        }
    }
    Ok(())
}

// ---- labels (user vocabulary OR-set, h89s.2) -------------------------------

/// Attach a user label to an item. Existence + validation (trim/non-empty/separator-free) live in
/// the shared write layer ([`write::label_add`]).
pub fn label_add(json: bool, db: Option<&str>, id: &str, label: &str) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let id = ws.replica.resolve_id(id);
    write::label_add(&mut store, &resolve_now(None)?, &actor(), &id, label)?;
    emit_label(json, &ws.replica.prefix, &id, label, |i, l| {
        format!("{i} labelled {l}")
    });
    Ok(())
}

/// Detach a label from an item (observed-remove).
pub fn label_remove(json: bool, db: Option<&str>, id: &str, label: &str) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let id = ws.replica.resolve_id(id);
    write::label_remove(&mut store, &resolve_now(None)?, &actor(), &id, label)?;
    emit_label(json, &ws.replica.prefix, &id, label, |i, l| {
        format!("{i} unlabelled {l}")
    });
    Ok(())
}

/// List an item's labels (sorted, deterministic).
pub fn label_list(json: bool, db: Option<&str>, id: &str) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let store = ws.open_store()?;
    let id = ws.replica.resolve_id(id);
    let labels = read::labels(&store, &id)?;
    if json {
        println!("{}", read::labels_to_value(&labels));
    } else {
        for l in &labels {
            println!("{l}");
        }
    }
    Ok(())
}

// ---- note ------------------------------------------------------------------

pub fn note_add(json: bool, db: Option<&str>, id: &str, text: &str) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let mut store = ws.open_store()?;
    let id = ws.replica.resolve_id(id);
    // `note add <id> -` reads the note body from STDIN (95d.1) — same escaping-free path as the
    // long-text fields, so a multi-line worklog entry needs no shell-quoting.
    let mut reader = StdinReader::new();
    let text = text_input::resolve(&mut reader, "text", Some(text), None)?
        .expect("an inline value always resolves to Some");
    let note_id = write::note_add(&mut store, &resolve_now(None)?, &actor(), &id, &text)?;
    if json {
        println!("{}", serde_json::json!({ "id": note_id, "body": text }));
    } else {
        println!("noted {note_id}");
    }
    Ok(())
}

pub fn note_list(json: bool, db: Option<&str>, id: &str) -> Result<()> {
    let (ws, _cfg) = open(db)?;
    let store = ws.open_store()?;
    let id = ws.replica.resolve_id(id);
    // 2kjy: notes carry their op-log created_at — `--json` as a sparse `created_at` key (the same
    // note shape `show --json` emits), the human view as a leading date.
    let notes = read::notes_with_created(&store, &id)?;
    if json {
        println!("{}", read::notes_with_created_to_value(&notes));
    } else {
        for (_, body, created) in &notes {
            println!("- {}{body}", note_date_prefix(created.as_deref()));
        }
    }
    Ok(())
}

/// The leading `YYYY-MM-DD ` a human note line carries when its op stamped a `wall_clock` (2kjy) —
/// the date portion of the RFC3339/date value, empty when un-stamped so a legacy note stays bare.
fn note_date_prefix(created: Option<&str>) -> String {
    match created {
        Some(c) => format!("{} ", c.split('T').next().unwrap_or(c)),
        None => String::new(),
    }
}

// ---- sync ------------------------------------------------------------------
//
// The sync VERB (bind/run) moved to the umbrella as `nxs sync` (aye.2.1): with one shared op-log,
// sync is a platform operation, not a flow-module one. flow keeps only `crate::sync::is_bound`
// (read by `prime` to gate its sync hints).

// ---- show ------------------------------------------------------------------

pub fn show(json: bool, db: Option<&str>, id: &str) -> Result<()> {
    let (ws, cfg) = open(db)?;
    let store = ws.open_store()?;
    let id = ws.replica.resolve_id(id);
    // 6j6v.srpg: the retrieval rule's full-text cell — the item-scoped memories filed against THIS
    // item. `show` is where a reader has already narrowed the field to one item, so they get the
    // whole memory rather than a pointer to it. Empty (and unread) without an active memory module.
    let memories = crate::memories::of_item(&ws, &id)?;

    if json {
        // h89s.2 + 6j6v.ekf5: the store-aware value carries the item's labels AND the sparse declared
        // `custom` map alongside the record. ee2h: augment with the additive priority_label/type_label
        // (CLI-only; the canonical keys and the MCP seam are untouched).
        let mut value =
            with_presentation_labels(&cfg, read::show_value_with_custom(&cfg, &store, &id)?);
        // The same sparse join the MCP `flow_show` tool attaches — one definition, two seams.
        crate::memories::attach_to_show(&memories, &mut value);
        println!("{value}");
    } else {
        let rec = read::show(&store, &id)?;
        // The declared, non-empty custom values in field-sorted order (§6): narrow the item's set
        // custom fields to the active plugin's declaration so a foreign field is not rendered.
        // T4-review efficiency (ky26): a plugin with no declared `[fields]` renders no custom section,
        // so skip the `custom_fields_of` read entirely rather than query then filter everything out.
        let custom: BTreeMap<String, String> = if cfg.fields.is_empty() {
            BTreeMap::new()
        } else {
            store
                .custom_fields_of(&id)?
                .into_iter()
                .filter(|(name, _)| cfg.fields.contains_key(name))
                .collect()
        };
        // 2kjy: the NOTES section renders each note's op-log date, so fetch the dated projection.
        let notes = read::notes_with_created(&store, &id)?;
        // nxf 6j6v.8dbe: BOTH weights are shown here, unlike `next`. A reader who asked about THIS
        // item by name has already narrowed the field, so a link that only grazed it is signal, not
        // noise — and being able to see one is what lets a reader judge whether to open the thread.
        let thread_links = read::thread_links(&store, &id)?;
        print!(
            "{}",
            render_show(
                &cfg,
                &ws.replica.prefix,
                &rec.item,
                &rec.deps,
                &rec.contributes_to,
                &notes,
                &custom,
                &thread_links,
                &memories,
                rec.parents_notice.as_deref(),
            )
        );
    }
    Ok(())
}

/// Human view of an item (8qv.8): the FULL field model in the plugin's language. A header
/// (`<id> <priority> <title>` + `=` rule), the inline TYPE/STATUS (and PARENT/BLOCKED BY when
/// they apply), then the DESCRIPTION / DEFINITION OF DONE / DESIGN / NOTES sections whose
/// headings are the plugin's field labels. `--json` carries everything for machines; this is
/// the human surface, so due/defer/assignee live in the record but not in this layout.
/// `parents_notice` (6j6v.zvd0) — the engine-computed parent-pointer notice — is rendered
/// verbatim as the very last thing when the item has parent(s); omitted entirely otherwise.
#[allow(clippy::too_many_arguments)]
fn render_show(
    cfg: &PluginConfig,
    prefix: &str,
    item: &ItemRow,
    deps: &[(String, Option<String>)],
    contributes_to: &[String],
    notes: &[(String, String, Option<String>)],
    custom: &BTreeMap<String, String>,
    thread_links: &[ThreadLink],
    memories: &[nexus_memory::facade::MemoryRecord],
    parents_notice: Option<&str>,
) -> String {
    let mut out = String::new();

    // Header: id (display form — bare local / full foreign, ykv), priority as its named variant
    // (not the ordinal), title — underlined by an `=` rule the width of the line.
    let priority = item
        .priority
        .as_deref()
        .map(|p| label(cfg, "priority", p))
        .unwrap_or_else(|| "-".to_string());
    let title = item.title.as_deref().unwrap_or("");
    let header = format!(
        "{} {} {}",
        workspace::display_id(prefix, &item.id),
        priority,
        title
    );
    out.push_str(&header);
    out.push('\n');
    out.push_str(&"=".repeat(header.chars().count()));
    out.push_str("\n\n");

    // Inline structural fields: fixed labels, plugin-vocabulary values.
    if let Some(t) = &item.item_type {
        out.push_str(&format!("TYPE: {}\n", label(cfg, "type", t)));
    }
    if let Some(s) = &item.status {
        out.push_str(&format!("STATUS: {}\n", label(cfg, "status", s)));
    }
    // ARCHIVED marker (C1 #916.1): shown only for an archived item — `show` resolves it by id even
    // though the enumerating reads hide it, so the marker (carrying the archive instant) tells the
    // reader why it is absent elsewhere and that `unarchive` can restore it.
    if let Some(at) = &item.archived {
        out.push_str(&format!("ARCHIVED: {at}\n"));
    }
    // PARENT only when set; BLOCKED BY only when an open blocker actually holds this item up
    // (a live, non-closed dependency) — id-sorted, since `deps` is.
    if let Some(parent) = &item.belongs_to {
        out.push_str(&format!(
            "PARENT: {}\n",
            workspace::display_id(prefix, parent)
        ));
    }
    let blockers: Vec<&str> = deps
        .iter()
        .filter(|(_, st)| matches!(st.as_deref(), Some(s) if s != "closed"))
        .map(|(d, _)| workspace::display_id(prefix, d))
        .collect();
    if !blockers.is_empty() {
        out.push_str(&format!("BLOCKED BY: {}\n", blockers.join(", ")));
    }
    // CONTRIBUTES TO: the items this one feeds into (vf4) — shown only when present, kept
    // distinct from BLOCKED BY since it carries no blocking semantics. Ids are sorted.
    if !contributes_to.is_empty() {
        let to: Vec<&str> = contributes_to
            .iter()
            .map(|c| workspace::display_id(prefix, c))
            .collect();
        out.push_str(&format!("CONTRIBUTES TO: {}\n", to.join(", ")));
    }
    // CONVERSATIONS (nxf 6j6v.8dbe): the chat threads that worked on or cited this item — the line
    // that ends "the ticket never found out". Each carries its two attributes, because "a thread is
    // WORKING on this" and "a thread mentioned this in passing" are different invitations to open it.
    if !thread_links.is_empty() {
        let rendered: Vec<String> = thread_links
            .iter()
            .map(|l| {
                format!(
                    "{} ({}, {})",
                    l.thread_id,
                    l.relation.as_str(),
                    l.weight.as_str()
                )
            })
            .collect();
        out.push_str(&format!("CONVERSATIONS: {}\n", rendered.join(", ")));
    }

    // The four field sections, always present (the full model, not a subset).
    out.push_str(&field_section(
        cfg,
        "description",
        item.description.as_deref(),
    ));
    out.push_str(&field_section(
        cfg,
        "completion_criterion",
        item.completion_criterion.as_deref(),
    ));
    out.push_str(&field_section(cfg, "design", item.design.as_deref()));

    // Declared custom fields (plugin-custom-fields §6), after the canonical block, each under its
    // label (declared or titleised). Only set (non-empty) declared values reach here, so a plugin
    // with no `[fields]` renders nothing and the bundled-plugin goldens are unchanged. Field-sorted
    // (the `BTreeMap`), matching the sparse `custom` map's key order.
    for (name, value) in custom {
        if let Some(decl) = cfg.fields.get(name) {
            let heading = read::custom_field_label(decl, name).to_uppercase();
            out.push_str(&section_heading_raw(&heading));
            out.push_str(value);
            out.push('\n');
        }
    }

    // NOTES, newest first (`notes` arrives in canonical oldest-first order). Each carries its op-log
    // date as a leading `YYYY-MM-DD` when stamped (2kjy); a legacy un-stamped note stays bare.
    let notes_heading = section_heading(cfg, "notes");
    out.push_str(&notes_heading);
    for (_, body, created) in notes.iter().rev() {
        out.push_str(&format!(
            "- {}{body}\n",
            note_date_prefix(created.as_deref())
        ));
    }

    // MEMORIES (6j6v.srpg): the item-scoped memories filed against this item, in FULL. This is the
    // rule's full-text cell — a reader who asked about THIS item by name is exactly the reader the
    // distillate was written for, and making them run a second command to read it would defeat the
    // point. Only present when the item carries any, so an item without them (and every workspace
    // without memory) renders byte-identically to before. Each body is its own block under its key,
    // because a memory may be a whole paragraph and must not collapse into a bullet.
    if !memories.is_empty() {
        out.push_str(&section_heading_raw("MEMORIES"));
        for (i, m) in memories.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&format!(
                "{}:\n{}\n",
                m.key,
                m.body.as_deref().unwrap_or("")
            ));
        }
    }

    // Parent-pointer notice (6j6v.zvd0): the engine's verbatim, loud reminder to also read the
    // parent(s) — appended last, only when the item actually has parent(s). Deliberately restates
    // the quiet `PARENT:` line above for a single parent (Code Quality #2, PR #255 review): the
    // `PARENT:` field is easy to skim past, and for MULTIPLE parents it shows only the LWW
    // `belongs_to` winner — this notice is the one place that is both unmissable and lists every
    // live parent.
    if let Some(notice) = parents_notice {
        out.push('\n');
        out.push_str(notice);
        out.push('\n');
    }

    out
}

/// A blank line, the uppercased plugin field heading, and an underline its width — the shared
/// section header for `show` (8qv.8). The field-label vocabulary now lives in the facade
/// (`read::field_label`, y5j8) so the CLI holds no second copy.
fn section_heading(cfg: &PluginConfig, key: &str) -> String {
    section_heading_raw(&read::field_label(cfg, key).to_uppercase())
}

/// A blank line, the given (already-uppercased) heading, and an underline its width — the shared
/// section-header body. Split from [`section_heading`] so a custom field (whose label comes from its
/// `[fields]` declaration, not the `[vocabulary.fields]` map) reuses the identical layout.
fn section_heading_raw(heading: &str) -> String {
    let rule = "-".repeat(heading.chars().count());
    format!("\n{heading}\n{rule}\n\n")
}

/// A full field section: the heading followed by the field value (blank when unset).
fn field_section(cfg: &PluginConfig, key: &str, value: Option<&str>) -> String {
    format!("{}{}\n", section_heading(cfg, key), value.unwrap_or(""))
}

// ---- schema (plugin-aware introspection, 8qv.5 / y5j8) ---------------------

/// `nxf schema`: the active plugin's full field model. `--json` is the machine contract (the agent
/// reads it to learn what `create`/`update` accept here); the human form is a readable summary.
/// Everything plugin-specific flows from the live `PluginConfig`, so it always reflects the active
/// vocabulary — the gap a static, compile-time clap `--help` cannot fill.
pub fn schema(json: bool, db: Option<&str>) -> Result<()> {
    let (_ws, cfg) = open(db)?;
    let report = read::schema(&cfg);

    if json {
        println!("{}", report.to_value());
    } else {
        println!("schema for plugin: {}", report.plugin);
        println!(
            "  types:       {}",
            report
                .types
                .iter()
                .map(|(_, label)| label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        // y5j8: name the container types + the depth limit.
        let containers: Vec<&str> = report
            .hierarchy
            .types
            .iter()
            .filter(|t| t.container)
            .map(|t| t.name.as_str())
            .collect();
        let depth = report
            .hierarchy
            .max_depth
            .map(|d| d.to_string())
            .unwrap_or_else(|| "unlimited".to_string());
        println!(
            "  containers:  {} (max depth: {depth})",
            if containers.is_empty() {
                "—".to_string()
            } else {
                containers.join(", ")
            }
        );
        println!(
            "  statuses:    {}",
            report
                .statuses
                .values()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!("  priorities:  {}", report.priorities.join(", "));
        println!("\nfields (create / update):");
        for f in &report.fields {
            let mut tags: Vec<String> = Vec::new();
            tags.push(if f.required {
                "required".into()
            } else {
                "optional".into()
            });
            if let Some(flag) = &f.create_flag {
                tags.push(format!("create {flag}"));
            }
            if let Some(k) = &f.create_json_key {
                if let Some(vals) = &f.create_json_values {
                    tags.push(format!("create --json \"{k}\": {}", vals.join("|")));
                } else {
                    tags.push(format!("create --json \"{k}\""));
                }
            }
            if f.settable {
                let how = f.update_alias.as_deref().unwrap_or(f.field.as_str());
                tags.push(format!("update --set {how}=…"));
            }
            println!(
                "  {:<20} {:<20} {:<9} {}",
                f.field,
                f.label,
                f.kind,
                tags.join(", ")
            );
        }
        println!(
            "\ndependencies:  --depends-on <id> at create (repeatable, `create --json` key \
             \"depends_on\"), or `dep add <this> <id>`"
        );
    }
    Ok(())
}

#[cfg(test)]
mod first_move_tests {
    use super::*;

    #[test]
    fn first_move_command_uses_a_valid_type_for_each_bundled_plugin() {
        // nexus-flow-92zt: the onboarding first-move command must name a type the ACTIVE plugin
        // declares. The old hardcoded `--type task` is in NEITHER bundled plugin
        // (issue-tracker: epic/bug/feature/chore/decision; personal-todo: project/todo/termin), so a
        // stock init's copy-pasted first command errored — the worst possible first impression.
        for name in ["issue-tracker", "personal-todo"] {
            let cfg = plugin::load(name).unwrap();
            let cmd = first_move_command(&cfg);
            let ty = cmd
                .split("--type ")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .unwrap_or("");
            assert!(
                cfg.types.is_type(ty),
                "{name}: first-move type '{ty}' must be a declared type; got `{cmd}`"
            );
            assert!(
                !cmd.contains("--type task"),
                "{name}: must not emit the invalid hardcoded `task`; got `{cmd}`"
            );
        }
    }

    #[test]
    fn init_next_steps_first_step_is_the_plugin_aware_create() {
        let cfg = plugin::load("issue-tracker").unwrap();
        let steps = init_next_steps(&cfg);
        assert_eq!(steps[0].0, "create your first item");
        assert_eq!(steps[0].1, "nxf create --type epic --title \"...\"");
        // the guide step stays static + plugin-independent.
        assert_eq!(
            steps[1],
            ("read the guide", "nxf guide getting-started".to_string())
        );
    }
}

#[cfg(test)]
mod schema_tests {
    use super::*;

    #[test]
    fn schema_create_json_keys_match_the_payload() {
        // 3bw6 drift guard: the create-payload key `schema` advertises for each createable field MUST
        // be exactly what `CreateJson` accepts — sourced from the facade schema report (read::schema).
        let cfg = read::schema(&crate::plugin::load("issue-tracker").unwrap());
        let key_of = |field: &str| -> Option<String> {
            cfg.fields
                .iter()
                .find(|f| f.field == field)
                .unwrap_or_else(|| panic!("field '{field}' present in schema"))
                .create_json_key
                .clone()
        };

        // The three fields whose create-payload key DIVERGES from the canonical name (3bw6): pin the
        // exact mapping so a rename on either side turns this red instead of silently diverging.
        assert_eq!(key_of("completion_criterion").as_deref(), Some("dod"));
        assert_eq!(key_of("defer_until").as_deref(), Some("defer"));
        assert_eq!(key_of("belongs_to").as_deref(), Some("parent"));
        // A non-divergent createable field advertises its own name as the key.
        assert_eq!(key_of("title").as_deref(), Some("title"));
        // Createable ⇔ has a create key: a field with no create flag advertises no key, else an agent
        // would build a payload for a field `create` cannot set.
        assert_eq!(
            key_of("status"),
            None,
            "a non-createable field advertises no create key"
        );

        // Round-trip: a payload keyed by EVERY advertised create key must pass the strict
        // `deny_unknown_fields` parse (`type`/`title`/`description`/`priority` are required), so a key
        // the schema advertises but `CreateJson` rejects turns this red.
        let mut obj = serde_json::Map::new();
        for f in &cfg.fields {
            if let Some(k) = &f.create_json_key {
                // A representative value; only the KEY is under test (deny_unknown_fields).
                obj.insert(k.clone(), serde_json::json!("x"));
            }
        }
        let parsed = serde_json::from_value::<CreateJson>(serde_json::Value::Object(obj));
        assert!(
            parsed.is_ok(),
            "advertised create keys must match CreateJson: {parsed:?}"
        );
    }

    #[test]
    fn create_json_accepts_a_set_key_for_custom_fields() {
        // ky26 review CQ2: `create --json -` carries plugin custom fields via a `set` array (mirroring
        // the `--set` flag), so a REQUIRED custom field is creatable via the JSON payload too. Before
        // the fix, `deny_unknown_fields` rejected the key and `custom` was hardcoded empty.
        let p: CreateJson = serde_json::from_value(serde_json::json!({
            "type": "file", "title": "report.md", "description": "why", "priority": "P1",
            "set": ["uri=.nxs/files/report.md"]
        }))
        .expect("a `set` array parses under deny_unknown_fields");
        assert_eq!(p.set, vec!["uri=.nxs/files/report.md".to_string()]);
        // Omitted ⇒ empty (default) — a canonical-only payload is unchanged.
        let p2: CreateJson = serde_json::from_value(serde_json::json!({
            "type": "task", "title": "t", "description": "d", "priority": "P1"
        }))
        .unwrap();
        assert!(p2.set.is_empty(), "no `set` key ⇒ empty custom sets");
    }
}

/// Inject the active plugin's presentation labels for `priority`/`type` into a read verb's
/// `--json` value (ee2h). ADDITIVE and CLI-only: the canonical `priority` (ordinal) and `type`
/// (declared key) fields stay byte-identical — `priority_label`/`type_label` are added alongside,
/// resolved through the plugin exactly as the human views map them. This deliberately does NOT
/// touch the facade record or the MCP `structuredContent` seam (both pinned to the core fields by
/// the parity gate, and both fed by the SAME `read::*` values used here): it augments only the
/// bytes `nxf show/list/next --json` print. A null `priority`/`type` yields a null label. Inserted
/// keys land in lexicographic position (the value is BTreeMap-backed), preserving the sorted-key
/// contract every consumer relies on.
fn with_presentation_labels(cfg: &PluginConfig, mut value: serde_json::Value) -> serde_json::Value {
    match &mut value {
        // `list`/`next --json` are arrays whose elements ARE the record.
        serde_json::Value::Array(items) => {
            for it in items.iter_mut() {
                add_presentation_labels(cfg, it);
            }
        }
        serde_json::Value::Object(map) => {
            if map.contains_key("priority") || map.contains_key("type") {
                // A bare/flat record.
                add_labels_to_map(cfg, map);
            } else if let Some(item) = map.get_mut("item") {
                // `show --json` nests the canonical record under `item` (alongside deps/notes/…);
                // label it there so `item.priority_label` sits with `item.priority`.
                add_presentation_labels(cfg, item);
            }
        }
        _ => {}
    }
    value
}

/// Add `priority_label`/`type_label` to one item value (a no-op for a non-object value).
fn add_presentation_labels(cfg: &PluginConfig, item: &mut serde_json::Value) {
    if let Some(obj) = item.as_object_mut() {
        add_labels_to_map(cfg, obj);
    }
}

/// Insert the two additive label keys into a record map, resolved through the plugin. Absent/null
/// `priority`/`type` yield a null label.
fn add_labels_to_map(cfg: &PluginConfig, obj: &mut serde_json::Map<String, serde_json::Value>) {
    let label_value = |obj: &serde_json::Map<String, serde_json::Value>, field: &str| {
        obj.get(field)
            .and_then(serde_json::Value::as_str)
            .map(|raw| serde_json::Value::String(label(cfg, field, raw)))
            .unwrap_or(serde_json::Value::Null)
    };
    let priority_label = label_value(obj, "priority");
    let type_label = label_value(obj, "type");
    obj.insert("priority_label".to_string(), priority_label);
    obj.insert("type_label".to_string(), type_label);
    // yfwt: decorate the nested `parent` reference join (`next --json`) with its own `type_label`.
    // The parent join is a lightweight `{id,title,type}` (no priority), so it gains ONLY `type_label`
    // — never a null `priority_label`. A null/absent `parent`, or one without a `type`, is left as-is.
    if let Some(parent) = obj
        .get_mut("parent")
        .and_then(serde_json::Value::as_object_mut)
    {
        if let Some(raw) = parent.get("type").and_then(serde_json::Value::as_str) {
            let pt = serde_json::Value::String(label(cfg, "type", raw));
            parent.insert("type_label".to_string(), pt);
        }
    }
}

/// Apply the plugin's vocabulary/labels to a raw field value.
fn label(cfg: &PluginConfig, field: &str, raw: &str) -> String {
    match field {
        "status" => cfg
            .vocabulary
            .status
            .get(raw)
            .cloned()
            .unwrap_or_else(|| raw.to_string()),
        "type" => cfg
            .vocabulary
            .types
            .get(raw)
            .cloned()
            .unwrap_or_else(|| raw.to_string()),
        // Priority is stored as its canonical ordinal (8qv.2); display maps it back to the
        // plugin's named variant (e.g. "1" → "P1"), with the raw value as a defensive fallback.
        "priority" => raw
            .parse::<usize>()
            .ok()
            .and_then(|i| cfg.priority.labels.get(i).cloned())
            .unwrap_or_else(|| raw.to_string()),
        _ => raw.to_string(),
    }
}

#[cfg(test)]
mod prime_custom_tests {
    //! bbq6: `attach_prime_custom` splices the sparse declared `custom` map into `prime --json`'s
    //! `next` rows. The bundled plugins declare no `[fields]`, so the golden prime tests only cover
    //! the no-op path; these unit tests drive the ACTIVE path with a fields-declaring plugin, and
    //! pin the no-op path directly (Test Quality follow-up).
    use super::*;
    use nexus_flow_core::model::MergeStrategy;
    use nexus_flow_core::store::Store;

    const NOW: &str = "2026-06-23T00:00:00Z";

    /// A plugin that DECLARES a custom `uri` text field on a `file` type — the same fixture shape
    /// the facade read tests use.
    fn cfg_with_fields() -> PluginConfig {
        toml::from_str(
            r#"
            name = "fields-fixture"
            [description]
            en = "x"
            de = "y"
            [priority]
            labels = ["P1", "P2", "P3"]
            [types]
            list = ["file"]
            [vocabulary.status]
            open = "open"
            in_progress = "in progress"
            closed = "closed"
            [ranking.next]
            order = [ { field = "id", dir = "asc" } ]
            [presentation.list]
            columns = ["id"]

            [fields.uri]
            type = "text"
            on = ["file"]
            required = true
            label = "File URI"
            "#,
        )
        .expect("cfg_with_fields parses")
    }

    fn set_custom(s: &mut Store, id: &str, field: &str, value: &str) {
        s.set_custom_field_merge(id, field, Some(value.to_string()), "t", MergeStrategy::Lww);
    }

    #[test]
    fn attach_prime_custom_splices_declared_custom_into_next_rows() {
        let cfg = cfg_with_fields();
        let mut s = Store::open_in_memory(1);
        // A ready `file` item (open, no blockers, no defer) so it lands in prime's `next`.
        s.create_item("ab12.0001", "file", "A", "t");
        set_custom(&mut s, "ab12.0001", "uri", ".nxs/files/a.md");

        let report = read::prime(&cfg, &s, NOW, false).unwrap();
        let mut value = report.to_value();
        attach_prime_custom(&cfg, &s, &report, &mut value).unwrap();

        let next = value["next"]
            .as_array()
            .expect("prime json has a next array");
        let row = next
            .iter()
            .find(|r| r["id"] == "ab12.0001")
            .expect("the ready item is in prime's next");
        assert_eq!(
            row["custom"],
            serde_json::json!({ "uri": ".nxs/files/a.md" }),
            "attach_prime_custom splices the declared custom map onto the matching next row"
        );
    }

    #[test]
    fn attach_prime_custom_is_a_no_op_without_declared_fields() {
        // A bundled plugin declares no `[fields]`, so prime's `next` rows stay byte-identical —
        // no `custom` key is added.
        let cfg = plugin::load("issue-tracker").unwrap();
        let mut s = Store::open_in_memory(1);
        s.create_item("ab12.0001", "feature", "A", "t");

        let report = read::prime(&cfg, &s, NOW, false).unwrap();
        let before = report.to_value();
        let mut value = before.clone();
        attach_prime_custom(&cfg, &s, &report, &mut value).unwrap();

        assert_eq!(
            value, before,
            "no `[fields]` ⇒ attach_prime_custom leaves the json unchanged"
        );
        assert!(
            value["next"][0].get("custom").is_none(),
            "a fieldless plugin's prime next rows carry no `custom` key"
        );
    }
}

#[cfg(test)]
mod next_render_tests {
    //! 6j6v.8bax: the `nxf next` row as a HUMAN reads it — a bold title to scan for, a muted id and
    //! epic line that stay out of the way, and a priority on the brand's Ember → Flame ramp.
    //!
    //! These are unit tests because they are the only place the styling can be SEEN: on a real
    //! terminal the theme paints, and the whole test harness (trycmd, `assert_cmd`, CI, pipes) is
    //! by construction not a terminal — which is exactly the byte-stability guardrail
    //! `nxs_ui::style` promises. So the theme is injected here rather than detected.
    use super::*;
    use nxs_ui::{ColorLevel, Theme};

    const MUTED_RGB: &str = "107;107;107";
    const EMBER_RGB: &str = "220;38;38";
    const BOLD: &str = "\u{1b}[1m";

    fn tty() -> Theme {
        Theme::new(true, ColorLevel::TrueColor)
    }

    fn item(id: &str, item_type: &str, priority: &str, title: &str) -> ItemRow {
        ItemRow {
            id: id.into(),
            item_type: Some(item_type.into()),
            title: Some(title.into()),
            completion_criterion: None,
            description: None,
            design: None,
            status: Some("in_progress".into()),
            priority: Some(priority.into()),
            due: None,
            defer_until: None,
            assignee: None,
            belongs_to: None,
            closing_comment: None,
            deleted: None,
            archived: None,
            closed_at: None,
        }
    }

    #[test]
    fn next_row_bolds_the_title_and_mutes_the_id_leaving_the_rest_alone() {
        // The owner's complaint was that six fields arrive at one weight, with the only thing a
        // human scans for — the title — last and no heavier than the id in front of it.
        let cfg = plugin::load("issue-tracker").unwrap();
        let cells = list_columns(
            &cfg,
            "ab12",
            &item("ab12.0001", "bug", "1", "Fix it"),
            &tty(),
        );

        let [id, _priority, status, item_type, title] = &cells[..] else {
            panic!("issue-tracker lists five columns: {cells:?}");
        };
        // `display_id` shows a LOCAL id bare (ykv) — the muting is around whatever it renders.
        assert!(
            id.contains(MUTED_RGB) && id.contains("0001"),
            "id muted: {id:?}"
        );
        assert!(
            title.contains(BOLD) && title.contains("Fix it"),
            "title bold: {title:?}"
        );
        assert_eq!(
            status, "in progress",
            "status carries no styling of its own"
        );
        assert_eq!(item_type, "[bug]", "type carries no styling of its own");
    }

    #[test]
    fn next_row_paints_priority_by_ordinal_under_a_plugin_with_its_own_names() {
        // The trap this item names: priority LABELS are plugin vocabulary. The same stored ordinal
        // has to reach the same rung whether the plugin calls it "P1" or "soon" — a ramp that
        // matched the string would be wrong under `personal-todo` on its first row.
        let issue = list_columns(
            &plugin::load("issue-tracker").unwrap(),
            "ab12",
            &item("ab12.0001", "bug", "1", "t"),
            &tty(),
        );
        let todo = list_columns(
            &plugin::load("personal-todo").unwrap(),
            "ab12",
            &item("ab12.0001", "todo", "1", "t"),
            &tty(),
        );
        assert!(
            issue[1].contains("P1") && issue[1].contains(EMBER_RGB),
            "{:?}",
            issue[1]
        );
        assert!(
            todo[1].contains("soon") && todo[1].contains(EMBER_RGB),
            "{:?}",
            todo[1]
        );
    }

    #[test]
    fn next_row_never_paints_a_priority_it_cannot_rank() {
        // PR #363 review, Test Quality #3. The canonical form is the ordinal; anything else reaches
        // the defensive fallback `label` already has. Painting it would mean GUESSING a rung, and a
        // wrongly-guessed rung is worse than no colour — it would tell the reader an urgency the
        // board never recorded. So: pass the text through, leave it unpainted.
        let cfg = plugin::load("issue-tracker").unwrap();
        let tty = tty();

        // A label where an ordinal belongs (a hand-edited row, an older import).
        let named = list_columns(&cfg, "ab12", &item("ab12.0001", "bug", "P1", "t"), &tty);
        assert_eq!(named[1], "P1", "passed through verbatim: {:?}", named[1]);

        // And no priority at all — the column's own "-" placeholder, equally unranked.
        let mut none = item("ab12.0002", "bug", "1", "t");
        none.priority = None;
        let unset = list_columns(&cfg, "ab12", &none, &tty);
        assert_eq!(unset[1], "-", "the placeholder is not a rung either");
    }

    #[test]
    fn next_row_is_byte_stable_ascii_on_a_plain_theme() {
        // Agents, `--json`, pipes, CI and the trycmd goldens: the row is exactly the bytes it was
        // before this item existed. Not "escapes that happen to be invisible" — none at all.
        let cfg = plugin::load("issue-tracker").unwrap();
        let cells = list_columns(
            &cfg,
            "ab12",
            &item("ab12.0001", "bug", "1", "Fix it"),
            &Theme::plain(),
        );
        assert_eq!(cells, vec!["0001", "P1", "in progress", "[bug]", "Fix it"]);
    }

    #[test]
    fn next_parent_line_mutes_the_text_but_not_its_indent() {
        // The `↳` line is context, not content — muted whole. The indent stays outside the escape
        // so the column a reader's eye follows is plain spaces.
        let parent = read::ParentRef {
            id: "ab12.0002".into(),
            title: Some("Ship v1".into()),
            item_type: Some("epic".into()),
        };
        let styled = next_parent_line("ab12", &parent, &tty());
        assert!(
            styled.starts_with("    \u{1b}"),
            "indent precedes the escape: {styled:?}"
        );
        assert!(
            styled.contains(MUTED_RGB) && styled.contains("Ship v1"),
            "{styled:?}"
        );
        assert_eq!(
            next_parent_line("ab12", &parent, &Theme::plain()),
            "    ↳ 0002 · Ship v1\n",
            "and the plain rendering is unchanged"
        );
    }

    #[test]
    fn truncation_notice_appears_only_when_the_limit_actually_bit() {
        // 6j6v.8pf2's disclosure, seen through the theme: auxiliary text, so muted — and absent
        // when there is nothing to disclose, which is what keeps an uncut list byte-identical.
        // Built through `read::truncate_next` — the type's only constructor, since it is
        // `#[non_exhaustive]` — which means this test drives the real cut instead of a
        // hand-assembled result that could disagree with it.
        let five: Vec<ItemRow> = (0..5)
            .map(|n| item(&format!("ab12.000{n}"), "bug", "1", "t"))
            .collect();
        let cut = read::truncate_next(five, Some(1));
        let notice = next_truncation_notice(&cut, &tty()).expect("a cut list discloses");
        assert!(
            notice.contains("showing 1 of 5") && notice.contains(MUTED_RGB),
            "{notice:?}"
        );
        assert_eq!(
            next_truncation_notice(&cut, &Theme::plain()),
            Some("showing 1 of 5\n".to_string())
        );
        let whole = read::truncate_next(vec![item("ab12.0001", "bug", "1", "t")], None);
        assert_eq!(
            next_truncation_notice(&whole, &tty()),
            None,
            "nothing was cut"
        );
    }
}
