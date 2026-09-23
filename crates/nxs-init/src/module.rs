//! The compile-time module self-registration registry (nexus-flow-5jz.1). Each suite module
//! (`flow`/`memory`/…) `inventory::submit!`s ONE [`ModuleInit`] descriptor declaring everything the
//! umbrella needs to know about it — for BOTH `init` (its blurb, recommended-default flag, accepted
//! args, and the in-process [`InitFn`] that owns its setup) AND `prime`/`agent-manifest` (its CLI
//! `binary` + pin-clock `now_env`, still reached by shell-out). The umbrella reads this roster
//! exclusively: it hardcodes no per-module knowledge, and a new product appears in the chooser +
//! the prime fan-out purely by registering — no `nxs` source change.
//!
//! Link order across `inventory` submissions is unspecified, so [`roster`] sorts deterministically
//! (declared `order`, then `key`) to keep golden/`--json` output byte-stable.

use nxs_foundation::error::{NxfError, Result};
use nxs_foundation::workspace::WorkspaceConfig;
use nxs_ui::Theme;
use std::path::Path;

/// What a module's [`InitFn`] is handed to set itself up IN-PROCESS (5jz.3): the workspace root, an
/// optional product-specific arg pass-through (flow's `--plugin`), the `--json` switch, and the
/// shared [`Theme`] so its own interactive sub-config renders in the one umbrella look.
pub struct InitRequest<'a> {
    /// The workspace root the module sets itself up in.
    pub root: &'a Path,
    /// An optional product-specific pass-through (flow honors `--plugin`; memory takes none — gated
    /// by [`ModuleInit::accepts_plugin`]).
    pub plugin: Option<&'a str>,
    /// `--json`/non-interactive: the module must stay prompt-free + byte-stable when set.
    pub json: bool,
    /// The shared presentation theme for the module's own sub-config UI.
    pub theme: &'a Theme,
}

/// A module's own init entry, called IN-PROCESS by the umbrella driver (5jz.3) instead of shelling
/// out to `<binary> init --quiet`. The module owns every follow-up step (workspace join, its
/// interactive sub-config); it returns once its setup is complete.
pub type InitFn = fn(&InitRequest) -> Result<()>;

/// A module's post-init success moment (5jz.6): a prominent, copy-paste-able first command the
/// umbrella renders at the end of its frame when this module ends up active. `label` is the human
/// nudge; `command` is the load-bearing, copy-pasteable part. Self-registered, so the umbrella shows
/// a real first step without hardcoding any product's own vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirstCommand {
    /// The short human nudge (e.g. "create your first item").
    pub label: &'static str,
    /// The copy-pasteable command. MAY contain a `{type}` placeholder that the umbrella fills with
    /// the active plugin's example type at render time (nexus-flow-92zt), e.g.
    /// `nxf create --type {type} --title "..."` → `nxf create --type epic --title "..."`.
    pub command: &'static str,
}

/// One module's self-registered descriptor. Replaces the umbrella's old hardcoded `KNOWN_MODULES`
/// roster + `blurb()` + the `--plugin` special-case: every field a module needs the umbrella to
/// know now travels WITH the module, submitted via [`inventory`].
#[derive(Clone, Copy)]
pub struct ModuleInit {
    /// The module key as stored in `config.active_modules` (e.g. `flow`).
    pub key: &'static str,
    /// The standalone CLI binary the umbrella shells out to for `prime`/`agent-manifest` (e.g. `nxf`).
    pub binary: &'static str,
    /// The env var the binary reads to pin its reference time (e.g. `NXF_NOW`); `nxs prime` sets it
    /// to the shared `now` so every module sees the same instant.
    pub now_env: &'static str,
    /// The one-line description shown in the chooser + the summary (was `init.rs::blurb`).
    pub blurb: &'static str,
    /// Whether this module is the ember-highlighted recommended default in the chooser (flow today).
    pub recommended: bool,
    /// The canonical roster ordinal — the sort key for deterministic, byte-stable roster order
    /// (declared per module so ordering is intentional, never link-order or alphabetical accident).
    ///
    /// **It drives two unrelated things, and changing it moves both** (review of PR #381, Code
    /// Quality #1). One is a SAFETY property: `nxs prime` fans out in this order, and a host that
    /// truncates the session start takes the last block — so the last block must be the one a
    /// session can fetch back (memory, via `nxm recall`) rather than the board or the command
    /// vocabulary, which are written down nowhere else. The other is PRESENTATION: `nxs guide`
    /// lists topics in this order, its `--json` records follow it, and its
    /// "exists in N building blocks — ask one of them" error names the binaries in it; `nxs init`,
    /// `nxs migrate` and `nxs setup` walk the roster in it too. nxf 6j6v.xbnh swapped chat (30→20)
    /// and memory (20→30) for the first reason and moved all of the second with it.
    ///
    /// **A third order exists and is NOT this one:** `nexus_chat::facade::PRIME_MODULE_ORDER`,
    /// which sequences a persona's composed prompt. chat cannot read this registry (it does not
    /// depend on `nxs-init`), so the two are kept in step by hand and by
    /// `the_two_module_order_tables_agree` in `crates/nxs/tests/the_session_start_ceiling.rs` —
    /// the one place that links both and can compare them.
    pub order: u16,
    /// Whether the module honors an `InitRequest::plugin` pass-through (flow yes, memory no) — the
    /// data that replaces the umbrella's hardcoded `--plugin` branch in `drive_module`.
    pub accepts_plugin: bool,
    /// The module's own in-process init entry (5jz.3 calls it; 5jz.1 only reads it from the roster).
    pub init_fn: InitFn,
    /// The module's richer product-intro line for the welcome box (5jz.6), rendered when the module
    /// is preselected/recommended in the interactive frame. `None` → only the generic suite copy.
    pub welcome: Option<&'static str>,
    /// The module's "learn more" decision-help copy (nexus-flow-huy) — a couple of sentences shown
    /// in the interactive picker's details panel, so the user chooses from more than the one-line
    /// [`blurb`](Self::blurb). Self-registered, so the umbrella hardcodes no product vocabulary;
    /// `None` → the tool contributes no details line.
    pub details: Option<&'static str>,
    /// The module's post-init first command (5jz.6) — the umbrella's success moment when this module
    /// ends up active. `None` → no success line for this module.
    pub first_command: Option<FirstCommand>,
}

impl std::fmt::Debug for ModuleInit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The `init_fn` pointer is noise in assertions; identify a descriptor by its key + binary.
        f.debug_struct("ModuleInit")
            .field("key", &self.key)
            .field("binary", &self.binary)
            .field("order", &self.order)
            .finish_non_exhaustive()
    }
}

inventory::collect!(ModuleInit);

/// The full suite roster, collected from every linked module's self-registration and sorted into
/// the canonical deterministic order. Only populated where the module crates are LINKED (the `nxs`
/// binary + the e2e harness); unit tests inject their own roster via the slice-taking helpers below.
pub fn roster() -> Vec<&'static ModuleInit> {
    sort_roster(inventory::iter::<ModuleInit>.into_iter().collect())
}

/// Sort a roster into canonical order: by declared `order`, then `key` as a stable tiebreak. Pure,
/// so the determinism guarantee is unit-testable without `inventory`.
pub fn sort_roster(mut modules: Vec<&ModuleInit>) -> Vec<&ModuleInit> {
    modules.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.key.cmp(b.key)));
    modules
}

/// The roster entry for `key`, or `None` if the roster does not know it.
pub fn find<'a>(roster: &[&'a ModuleInit], key: &str) -> Option<&'a ModuleInit> {
    roster.iter().copied().find(|m| m.key == key)
}

/// The binary the umbrella shells out to for `key` (`prime`/`agent-manifest`), or `None`.
pub fn binary_for(roster: &[&ModuleInit], key: &str) -> Option<&'static str> {
    find(roster, key).map(|m| m.binary)
}

/// Resolve a workspace's active modules to their roster entries, **in `active_modules` order** (the
/// stable fan-out/assembly order the byte-stability contracts require). An active module the roster
/// does not know is a loud `validation` error naming it — never silently dropped, so a typo or a
/// not-yet-linked product surfaces instead of vanishing from prime.
pub fn resolve_active<'a>(
    roster: &[&'a ModuleInit],
    config: &WorkspaceConfig,
) -> Result<Vec<&'a ModuleInit>> {
    config
        .active_modules
        .iter()
        .map(|key| {
            find(roster, key).ok_or_else(|| {
                NxfError::validation(format!(
                    "workspace lists active module '{key}', which is not in the nxs roster \
                     (known: {}). Upgrade nxs, or fix .nxs/config.toml.",
                    roster_keys(roster)
                ))
            })
        })
        .collect()
}

/// The roster's keys, comma-joined — for the unknown-module error messages.
pub fn roster_keys(roster: &[&ModuleInit]) -> String {
    roster.iter().map(|m| m.key).collect::<Vec<_>>().join(", ")
}

/// Resolve the active modules THIS roster knows, in `active_modules` order, SKIPPING any it does not
/// find. Used by a single module's OWN init when it re-assembles the shared agent file: a sibling
/// it does not link (e.g. flow, when `nxm init` joins a flow workspace) has already assembled its
/// own block via its own init, so skipping it here is correct — not an error.
///
/// The skip is by key match alone, so it does NOT distinguish a benign unlinked sibling from a
/// genuinely-unknown key (a typo, or a not-yet-installed product) — BOTH are silently dropped here.
/// That is deliberate: assembly is not the place the roster contract is enforced. The loud-error
/// guarantee lives in the strict [`resolve_active`], which `prime` (the umbrella, which links every
/// module) uses for its fan-out — there an unknown active module MUST surface loudly because it
/// cannot be fanned out to. Anything wrong with `config.active_modules` therefore surfaces at
/// `prime`/`migrate`, never silently from this assembly path.
pub fn resolve_known<'a>(
    roster: &[&'a ModuleInit],
    config: &WorkspaceConfig,
) -> Vec<&'a ModuleInit> {
    config
        .active_modules
        .iter()
        .filter_map(|key| find(roster, key))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop(_: &InitRequest) -> Result<()> {
        Ok(())
    }

    /// A test descriptor — the real modules self-register via `inventory`, but the roster LOGIC is
    /// exercised here with hand-built entries (no linked modules, no TTY).
    fn module(key: &'static str, binary: &'static str, order: u16) -> ModuleInit {
        ModuleInit {
            key,
            binary,
            now_env: "X_NOW",
            blurb: "",
            recommended: false,
            order,
            accepts_plugin: false,
            init_fn: noop,
            welcome: None,
            details: None,
            first_command: None,
        }
    }

    fn cfg(active: &[&str]) -> WorkspaceConfig {
        WorkspaceConfig {
            active_modules: active.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn sort_roster_orders_by_declared_ordinal_not_link_or_input_order() {
        // Submitted/collected in reverse — the canonical roster is still flow(10) then memory(20).
        let flow = module("flow", "nxf", 10);
        let memory = module("memory", "nxm", 20);
        let sorted = sort_roster(vec![&memory, &flow]);
        assert_eq!(
            sorted.iter().map(|m| m.key).collect::<Vec<_>>(),
            vec!["flow", "memory"]
        );
    }

    #[test]
    fn sort_roster_breaks_an_ordinal_tie_by_key_for_determinism() {
        let b = module("bbb", "b", 5);
        let a = module("aaa", "a", 5);
        let sorted = sort_roster(vec![&b, &a]);
        assert_eq!(
            sorted.iter().map(|m| m.key).collect::<Vec<_>>(),
            vec!["aaa", "bbb"]
        );
    }

    #[test]
    fn binary_for_maps_a_known_key_and_misses_an_unknown_one() {
        let flow = module("flow", "nxf", 10);
        let memory = module("memory", "nxm", 20);
        let roster = vec![&flow, &memory];
        assert_eq!(binary_for(&roster, "flow"), Some("nxf"));
        assert_eq!(binary_for(&roster, "memory"), Some("nxm"));
        assert_eq!(binary_for(&roster, "chat"), None);
    }

    #[test]
    fn resolve_active_preserves_active_order_not_roster_order() {
        // The fan-out/assembly order is the workspace append order, NOT the roster ordinal — a
        // memory-first workspace resolves memory before flow.
        let flow = module("flow", "nxf", 10);
        let memory = module("memory", "nxm", 20);
        let roster = vec![&flow, &memory];
        let resolved = resolve_active(&roster, &cfg(&["memory", "flow"])).unwrap();
        assert_eq!(
            resolved.iter().map(|m| m.binary).collect::<Vec<_>>(),
            vec!["nxm", "nxf"]
        );
    }

    #[test]
    fn resolve_active_is_empty_for_a_sectionless_config() {
        let flow = module("flow", "nxf", 10);
        let roster = vec![&flow];
        assert!(resolve_active(&roster, &WorkspaceConfig::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn an_unknown_active_module_is_a_loud_error_naming_it() {
        let flow = module("flow", "nxf", 10);
        let roster = vec![&flow];
        let err = resolve_active(&roster, &cfg(&["flow", "ghost"])).unwrap_err();
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Validation);
        assert!(err.msg.contains("ghost"), "names the offender: {}", err.msg);
    }
}
