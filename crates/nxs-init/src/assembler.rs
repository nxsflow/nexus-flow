//! The multi-module agent-file assembler (spec §6.2, nexus-flow-aye.27) — the core of P3. Each
//! module declares its contribution as data (`<cli> agent-manifest --json`); this assembles the
//! shared `AGENTS.md`/`CLAUDE.md` and wires ONE SessionStart hook PER ACTIVE MODULE. It is a
//! shared library UNIT: `nxs init` (S4) and the per-module `nxf`/`nxm init` (S5) all call it — same
//! logic, same hooks, wherever triggered.
//!
//! **Superseded 2026-08-28 (nxf n2m6 + a2a1).** This paragraph used to promise **exactly one**
//! SessionStart hook → `nxs prime`, and that was the right shape while the host's limit was
//! believed to be per SESSION: one output, one budget, so one hook was strictly simpler than
//! three. The measurement that replaces it (2026-08-28, 1.114 SessionStart events across 947
//! transcripts) is that the host truncates **per hook output**, at 10.240 B — and that three
//! SessionStart hooks of ~8 KB each were observed arriving WHOLE, 24.127 B between them. So the
//! budget is per hook, and splitting the fan-out into one hook per module multiplies the room a
//! session actually gets by the number of active modules instead of sharing one 10 KiB slot
//! between them. `SESSION_START_CEILING_BYTES` in `crates/nxs/src/prime.rs` carries the CEILING's
//! telemetry (this crate cannot link it — `nxs` depends on `nxs-init`, not the other way round);
//! the three-hooks-arrived-whole observation quoted just above lives with the gate in
//! `crates/nxs/tests/the_session_start_ceiling.rs` and in the spec, not in `prime.rs`. That gate is
//! what now measures per hook output.
//!
//! `nxs prime` itself is UNCHANGED and still fans out to every active module in one composed
//! output — it stays the verb a person runs by hand, and the only thing that moved is what the
//! HOST is wired to run.
//!
//! Since nexus-flow-0lj.2 the shared files carry a SINGLE, `nxs`-owned discovery pointer (one
//! `<!-- BEGIN NEXUS -->` block naming the active tools + pointing at `nxs prime`) — NOT a per-module
//! block each module authored. A module's operating instructions + the compaction-recovery hint now
//! live in its `prime` output (fanned out from `nxs prime`), so AGENTS.md stays a thin,
//! vocabulary-free pointer. Re-assembly refreshes the one block in place (byte-idempotent) and
//! migrates any legacy `NEXUS-FLOW`/`NEXUS-MEMORY` blocks away with no orphans.
//!
//! **"The shared files" is now ONE file** (nxf 6j6v.q6e3, owner's decision 2026-08-30). The block
//! goes into `AGENTS.md` and nowhere else. `CLAUDE.md` belongs to the host that RUNS the
//! SessionStart hook this same function wires, so everything the block carries has already arrived
//! by the time that file is read: the header and tool list open each of the three prime blocks, and
//! "run `nxs prime` for project context" names a command the host has just run.
//!
//! One of the three lines was worse than redundant. `@NEXUS_MEMORY.md` is a Claude Code IMPORT
//! directive, and the file it names is the projection of the memory store's whole BODY text — so a
//! session was handed the budgeted index (one line per memory, capped, with "read one in full with
//! `nxm recall`" written under it) and, beside it, every body underneath. Measured 2026-08-30:
//! 95.359 B in this repo, 110.508 B in app-foundations, 119.625 B in manufakt-io, against a hook
//! block of ~6 KB. The defect was not there when the line was written (6j6v.8q88): `nxm prime` was
//! unbounded then and the file was young. It BECAME one when the budget arrived (6j6v.5jm3) and
//! nobody looked at the second delivery route — "a change is right, its neighbourhood is not".
//!
//! So this function only ever CLEANS `CLAUDE.md`: it removes a block an earlier version wrote (and
//! any legacy per-module block), and writes nothing. It never adds an `@AGENTS.md` import either —
//! that would pull the same block in by another door, its memory binding included. What a hookless
//! reader needs stays exactly where it was always meant to be, in `AGENTS.md`.

use crate::module::ModuleInit;
use crate::spawn;
use nxs_foundation::error::{NxfError, Result};
use nxs_foundation::manifest::AgentManifest;
use serde_json::{json, Map, Value};
use std::path::Path;

/// The verb each module's SessionStart hook runs, appended to the module's own binary
/// (`nxf` → `nxf prime`). One hook per active module since nxf n2m6 + a2a1 — see the module doc
/// above for the per-hook measurement that replaced the single-hook shape.
const PRIME_VERB: &str = "prime";

/// The `|| cat NEXUS_MEMORY.md 2>/dev/null` tail, appended to EXACTLY ONE wired hook entry.
///
/// **Which one: memory's** ([`HOOK_FALLBACK_BINARY`], nxf 6j6v.1k6y). It hangs on one entry and not
/// on all of them because three entries each carrying it would deliver `NEXUS_MEMORY.md` three
/// times into the same session.
///
/// **Superseded 2026-08-29: it used to be "the first the assembler wires"** (Ruling R1, nxf a2a1),
/// argued as "deliberately not the flow entry — nothing guarantees flow is active, while the first
/// is well-defined for every module combination and deterministic". That reasoning is right about
/// what makes a rule well-defined, and it was applied to the wrong kind of thing. A POSITION rule
/// fits a tail that belongs to the suite. This tail belongs to a MODULE: `NEXUS_MEMORY.md` is the
/// projection of exactly the memories `nxm prime` replays, so it is the one command it can stand in
/// for. On every other entry it answers a failure with something unrelated to what failed.
///
/// Measured before the change (2026-08-29, `nxs init` in fresh workspaces): in a flow-only
/// workspace the tail sat on `nxf prime` where the file does not exist and cannot — `nxm` writes
/// it — so it was dead weight that could not produce a byte; and adding memory later did not move
/// it, so the arrangement went from useless to wrong at exactly the moment the document began to
/// exist. Binding it to the module removes both cases: no memory, no tail.
///
/// **Nothing catches the other modules' failure, and that is deliberate** (owner, 2026-08-29). The
/// obvious companion change — `|| echo "<the suite is not installed>"` on the first entry — was
/// considered and declined: `echo` succeeds, so the hook would exit 0 and a host that surfaces
/// failing hooks would stop saying anything. A contributor without `nxs` gets the shell's own
/// `command not found` and a failing hook, which is the honest report that nothing was delivered.
///
/// The fallback itself (6j6v.8q88) is what carries the project's context to a contributor who does
/// NOT have nexus-flow installed: `.claude/settings.json` is committed, so they get the hooks either
/// way, and on their machine the module binaries are simply not commands. `NEXUS_MEMORY.md` is a
/// projection of the same memories `prime` replays — a true SUBSET of it — so the fallback can
/// only ever deliver less than the main path, never more or something different.
///
/// `2>/dev/null` on the `cat` and nothing else: a workspace that has remembered nothing has no
/// file, which is a legitimate state, and the shell's "No such file or directory" would otherwise
/// be injected into the agent's context at every session start — noise that reads as a fault. The
/// exit status is deliberately left alone, so a host that surfaces failing hooks still says
/// something when BOTH paths are unavailable.
pub const HOOK_FALLBACK: &str = " || cat NEXUS_MEMORY.md 2>/dev/null";

/// The binary whose hook entry carries [`HOOK_FALLBACK`] — memory's (nxf 6j6v.1k6y).
///
/// Naming a module here is not new coupling: [`HOOK_FALLBACK`] already spells `NEXUS_MEMORY.md`,
/// which is memory's file and nobody else's. This makes the pair explicit rather than leaving the
/// document named in one place and its owner implied by a position in another.
const HOOK_FALLBACK_BINARY: &str = "nxm";

/// The SessionStart hook commands for `binaries`, in the given order: `<binary> prime` each, with
/// [`HOOK_FALLBACK`] on memory's entry only (nxf 6j6v.1k6y — see that constant, which also records
/// the position rule this replaces and why the other modules' failure is left loud).
///
/// **Superseded 2026-08-28 (nxf n2m6 + a2a1): this replaces a single `HOOK_COMMAND` constant**
/// that read `nxs prime || cat NEXUS_MEMORY.md 2>/dev/null` and was wired verbatim for every module
/// combination. That constant's own promise — "stable across every module combination … modules
/// grow, the hook does not" — was true and was the point, while one hook meant one host output
/// against one budget. It stopped being true when the budget turned out to be per hook output (see
/// the module doc above), so the hook list now grows with the modules on purpose.
///
/// Derived from the binary rather than read from each module's declared `manifest.hook.command`
/// so that `nxs migrate`, which has a roster but no manifests (collecting them shells out to every
/// module binary), builds the identical list. The two cannot drift: every module declares
/// `<binary> prime` and `the_declared_hook_matches_what_the_assembler_wires` pins that.
pub fn hook_commands(binaries: &[&str]) -> Vec<String> {
    binaries
        .iter()
        .map(|binary| {
            let fallback = if *binary == HOOK_FALLBACK_BINARY {
                HOOK_FALLBACK
            } else {
                ""
            };
            format!("{binary} {PRIME_VERB}{fallback}")
        })
        .collect()
}

/// The one-line human summary of the wired SessionStart hooks, shared by every init path.
///
/// **Superseded 2026-08-28 (nxf n2m6 + a2a1): there is no single hook COMMAND to interpolate any
/// more, so the rendering itself moved here.** Each module CLI used to build this line locally and
/// interpolate `HOOK_COMMAND` into it, "not spelled out … [because] three hand-written copies of it
/// across the module CLIs would be three places to go stale the next time it moves." That reasoning
/// is unchanged and is exactly why the line is now assembled in ONE place: with one entry per
/// module there is a LIST to render, not a constant to paste, and four callers formatting a list
/// four ways is the same defect one size larger.
///
/// `commands` is [`AssembleReport::hook_commands`] — what this run actually wired, never a
/// re-derivation.
pub fn describe_hooks(hook_added: bool, commands: &[String]) -> String {
    let rendered: Vec<String> = commands.iter().map(|c| format!("`{c}`")).collect();
    let list = rendered.join(", ");
    // A workspace with no active module wires no hook. Saying "SessionStart → " with nothing after
    // it would read as a truncated line rather than as the honest "there is nothing to run".
    if list.is_empty() {
        return "no active module, so no SessionStart hook".to_string();
    }
    if hook_added {
        format!("wired SessionStart → {list}")
    } else {
        format!("SessionStart → {list} already wired")
    }
}

/// Every SessionStart command this assembler could have written, for `binaries` — both the bare
/// `<binary> prime` and the [`HOOK_FALLBACK`] form of each, plus the retired
/// [`SUPERSEDED_HOOK_COMMANDS`].
///
/// This is the removal set, and it is why **a module that falls away takes its hook entry with
/// it**: re-wiring drops every command in here and then re-adds only the ones the current module
/// set wants, so a workspace that drops `chat` loses `nxc prime` in the same pass that keeps the
/// other two. Both forms of each binary are listed because the fallback moves: dropping the module
/// that happened to be first hands the fallback to the next one, and the old first entry has to go
/// rather than linger with a `cat` no other entry expects to own.
///
/// Unrelated SessionStart hooks — a user's own, or another tool's `bd prime` — are NOT in here and
/// are never touched. That is the reason this is an exact-command set built from the roster rather
/// than a pattern like "anything ending in ` prime`", which would silently eat a neighbour's hook.
pub fn managed_hook_commands(binaries: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = SUPERSEDED_HOOK_COMMANDS
        .iter()
        .map(|c| (*c).to_string())
        .collect();
    for binary in binaries {
        out.push(format!("{binary} {PRIME_VERB}"));
        out.push(format!("{binary} {PRIME_VERB}{HOOK_FALLBACK}"));
    }
    out
}

/// Hook commands earlier versions of the assembler wired, which [`hook_commands`] supersedes.
/// Removed before the current ones are ensured, so re-running any init path UPGRADES an existing
/// workspace instead of leaving a second hook behind that would print the session block twice.
///
/// **Extended 2026-08-28 (nxf n2m6 + a2a1)** with the umbrella hook's own fallback form. Both
/// `nxs prime` shapes are here because BOTH shipped: the bare one before 6j6v.8q88 added the
/// fallback, the `|| cat` one after it and up to this change. A workspace on either is rewired to
/// the per-module set, not topped up — leaving one behind would fan the composed block out a
/// second time alongside the three per-module ones.
///
/// (`nxf prime`, the pre-umbrella hook, is ALSO handled one layer up by `nxs migrate`, which
/// rewrites the config around it. It is not listed here as a static entry because it is no longer
/// retired vocabulary: since this change `nxf prime` is a hook command the assembler itself writes
/// when flow is active. [`managed_hook_commands`] covers the legacy bare form through the roster
/// instead, which is what upgrades a v0.5.x workspace on the next init.)
const SUPERSEDED_HOOK_COMMANDS: &[&str] =
    &["nxs prime", "nxs prime || cat NEXUS_MEMORY.md 2>/dev/null"];

/// One active module's collected manifest plus the binary `nxs` invokes for it (for the permission
/// allowlist). Since nexus-flow-0lj.2 the manifest carries only the module's `prime` fan-out target +
/// the hook; the assembler builds the single nxs-owned AGENTS.md pointer from the module `key`s
/// itself, so the discovery line names the active tools without any per-module block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleManifest {
    /// The module key (`config.active_modules`, e.g. `flow`) — the name shown in the discovery line.
    pub module: String,
    /// The module's CLI binary (e.g. `nxf`) — wired as a `Bash(<binary>:*)` allowlist entry.
    pub binary: String,
    /// The module's declared contribution (prime fan-out target + the hook to wire).
    pub manifest: AgentManifest,
}

/// What the assembler did to `AGENTS.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentsAction {
    /// `AGENTS.md` did not exist; it was created with the managed block(s).
    Created,
    /// `AGENTS.md` existed; the managed block(s) were added/refreshed alongside its content.
    Augmented,
}

impl AgentsAction {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentsAction::Created => "created",
            AgentsAction::Augmented => "augmented",
        }
    }
}

/// What the assembler did to `CLAUDE.md` — which since nxf 6j6v.q6e3 is only ever to CLEAN it.
///
/// The assembler writes NOTHING there: no managed block, no `@AGENTS.md` import. `CLAUDE.md`
/// belongs to the host that RUNS the SessionStart hook, so anything put in it is a second delivery
/// of what the hook already delivered — and, through the block's `@NEXUS_MEMORY.md` line, a second
/// delivery in the one shape the budget exists to prevent: the memory's whole body text instead of
/// its budgeted index (measured 2026-08-30: 95–120 KB per session in the three product repos, ten
/// to twenty times what the hook lets through). What it still does is REMOVE a block an earlier
/// version wrote, so an existing workspace is carried along by the next ordinary run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeAction {
    /// No `CLAUDE.md` present — left untouched (never created).
    Absent,
    /// `CLAUDE.md` exists and carries nothing this assembler owns; returned byte-for-byte.
    Untouched,
    /// A managed block written by an earlier version was removed; the rest of the file is
    /// untouched.
    BlockRemoved,
    /// The file consisted of NOTHING BUT that block, so it was deleted rather than left empty.
    ///
    /// Owner's call, 2026-08-30, from the hand-run that preceded this change: in two of the nine
    /// workspaces cleaned by hand the whole `CLAUDE.md` was the managed block, and "an empty file
    /// is worse than no file, because a reader opens it and learns nothing". The file was this
    /// tool's own leftover; leaving a hollow one behind would be a false map rather than restraint.
    FileRemoved,
}

impl ClaudeAction {
    pub fn as_str(self) -> &'static str {
        match self {
            ClaudeAction::Absent => "absent",
            ClaudeAction::Untouched => "untouched",
            ClaudeAction::BlockRemoved => "block_removed",
            ClaudeAction::FileRemoved => "file_removed",
        }
    }
}

/// What the assembler did, for an init path to report (human + `--json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssembleReport {
    pub agents: AgentsAction,
    pub claude: ClaudeAction,
    /// Whether the SessionStart wiring changed this run — see [`HookWireReport::hook_added`],
    /// which this mirrors, for what "changed" covers now that there is one entry per module.
    pub hook_added: bool,
    /// The SessionStart commands wired, in order — one per active module, the first carrying
    /// [`HOOK_FALLBACK`]. Every init path reports this instead of a constant, so the human line and
    /// the `--json` record name what was really wired for THIS module combination.
    pub hook_commands: Vec<String>,
    /// The permission allowlist entries newly added this run (`Bash(nxs:*)` + per-module binaries).
    pub permissions_added: Vec<String>,
    /// `CLAUDE.md` was written with different bytes this run (false ⇒ absent, or byte-identical).
    ///
    /// Since the assembler only ever REMOVES from `CLAUDE.md` (nxf 6j6v.q6e3) this is now true
    /// exactly for the two removing outcomes, [`ClaudeAction::BlockRemoved`] and
    /// [`ClaudeAction::FileRemoved`] — kept as its own field because it is what every init path's
    /// "what changed" line reads, and because the two answer different questions: one names what
    /// became of the file, the other names whether this run touched it at all.
    pub claude_changed: bool,
    /// The modules assembled, in order.
    pub modules: Vec<String>,
}

/// Collect the active modules' manifests by shelling out to `<binary> agent-manifest --json`
/// (TB-5). Order preserved (the stable assembly/fan-out order).
pub fn collect_manifests(modules: &[&ModuleInit]) -> Result<Vec<ModuleManifest>> {
    collect_with(modules, |binary| {
        spawn::run_capture(binary, &["agent-manifest", "--json"])
    })
}

/// [`collect_manifests`] with the per-binary runner injected — the testable core (a stub runner
/// returns canned JSON, so the assembler is exercised without real binaries).
pub fn collect_with<F>(modules: &[&ModuleInit], run: F) -> Result<Vec<ModuleManifest>>
where
    F: Fn(&str) -> Result<String>,
{
    modules
        .iter()
        .map(|m| {
            let raw = run(m.binary)?;
            let manifest: AgentManifest = serde_json::from_str(raw.trim()).map_err(|e| {
                NxfError::io(format!(
                    "parsing `{} agent-manifest --json` output: {e}",
                    m.binary
                ))
            })?;
            Ok(ModuleManifest {
                module: m.key.to_string(),
                binary: m.binary.to_string(),
                manifest,
            })
        })
        .collect()
}

/// Assemble the shared agent file for the project rooted at `root` from the active modules'
/// manifests, and wire ONE SessionStart hook per active module. Idempotent + re-assemblable:
/// running it again (or after a second module joins) refreshes each block in place, never
/// duplicates a hook, and drops the entry of a module that is no longer active. Modules' section
/// order — and the hook order, and therefore which entry carries [`HOOK_FALLBACK`] — is the
/// `modules` slice order (byte-stable).
///
/// **Superseded 2026-08-28 (nxf n2m6 + a2a1):** this used to wire "the single SessionStart hook →
/// `nxs prime`". See the module doc for the per-hook-output measurement that replaced it.
pub fn assemble(root: &Path, modules: &[ModuleManifest]) -> Result<AssembleReport> {
    let roster: Vec<&str> = crate::module::roster().iter().map(|m| m.binary).collect();
    assemble_with(root, modules, &roster)
}

/// [`assemble`] with the suite roster's binaries injected — the testable core, mirroring
/// [`crate::module::roster`]'s own note and `nxs migrate`'s `migrate_with`.
///
/// `roster_binaries` is every module binary the BUILD knows, not just the active ones, and it is
/// what makes a departed module's hook entry removable: `modules` alone cannot name a module that
/// is no longer there. `inventory` only populates the roster where the module crates are LINKED
/// (the `nxs` binary), so this crate's own unit tests pass it explicitly.
pub fn assemble_with(
    root: &Path,
    modules: &[ModuleManifest],
    roster_binaries: &[&str],
) -> Result<AssembleReport> {
    // **Sorted into ROSTER order here, and not at the call sites** (nxf 6j6v.shwz). The order a
    // workspace lists its modules in is an accident of which `init` ran first; the order this
    // writes is the one `nxs prime` fans out in, where memory going last carries a reason (it is
    // the one block a session can fetch back afterwards).
    //
    // In `assemble_with` because FOUR paths reach it — `nxs init`, `nxs setup claude`, `nxm init`
    // and `nxc init`, each resolving its own module list — and shwz exists precisely because two
    // paths disagreed about an order. A sort at each call site is a rule that can be forgotten by
    // the fifth. `roster_binaries` already carries the canonical sequence, so no new input is
    // needed; a binary the roster does not know keeps its relative place at the end rather than
    // being dropped or sorted arbitrarily.
    let ordered: Vec<ModuleManifest> = {
        let rank = |m: &ModuleManifest| {
            roster_binaries
                .iter()
                .position(|b| *b == m.binary)
                .unwrap_or(usize::MAX)
        };
        let mut v = modules.to_vec();
        v.sort_by_key(rank);
        v
    };
    let modules: &[ModuleManifest] = &ordered;

    let agents_path = root.join("AGENTS.md");
    let claude_path = root.join("CLAUDE.md");
    let agents_existed = agents_path.exists();
    let claude_existed = claude_path.exists();

    // The single `nxs`-owned discovery pointer (nexus-flow-0lj.2): ONE managed `NEXUS` block naming
    // the active tools + pointing at `nxs prime`. A module no longer contributes its own block —
    // operating instructions + the compaction-recovery hint live in `nxs prime` (re-run by the
    // SessionStart hook), so AGENTS.md carries only this thin, vocabulary-free discovery line.
    let block = nexus_discovery_block(modules);

    // 1. AGENTS.md — migrate away any OLD per-module blocks (`NEXUS-FLOW`/`NEXUS-MEMORY`/…), then
    //    upsert the ONE `NEXUS` pointer. Idempotent: a re-run replaces it in place, byte-for-byte.
    let migrated = strip_legacy_blocks(&read_or_empty(&agents_path)?)?;
    let agents = upsert_section(&migrated, &block)?;
    write_file(&agents_path, &agents)?;
    let agents_action = if agents_existed {
        AgentsAction::Augmented
    } else {
        AgentsAction::Created
    };

    // 2. CLAUDE.md — CLEANED, never written to (nxf 6j6v.q6e3). It is the file of the host that
    //    RUNS the hook wired in step 3, so a block here is a second delivery of what the hook has
    //    already delivered — and the block's `@NEXUS_MEMORY.md` line makes it the wrong delivery
    //    too: Claude Code expands the directive into an import of the memory's whole BODY text,
    //    beside the budgeted index the hook just handed the session. So: migrate away any legacy
    //    per-module blocks, remove the `NEXUS` block an earlier version wrote, and touch nothing
    //    else. Never created, and the file is written only when those removals changed something,
    //    so an ordinary re-run is a true no-op on a `CLAUDE.md` this assembler never owned.
    let (claude_action, claude_changed) = if !claude_existed {
        (ClaudeAction::Absent, false)
    } else {
        let original = read_or_empty(&claude_path)?;
        // Legacy blocks FIRST: their marker (`<!-- BEGIN NEXUS-<NAME>`) shares its prefix with the
        // current one, and stripping them here is what leaves `remove_section` a single shape to
        // match rather than a family.
        let target = remove_section(&strip_legacy_blocks(&original)?, &block)?;
        let changed = target != original;
        if !changed {
            (ClaudeAction::Untouched, false)
        } else if target.trim().is_empty() {
            // Nothing of the user's was ever in here — see [`ClaudeAction::FileRemoved`].
            std::fs::remove_file(&claude_path)
                .map_err(|e| NxfError::io(format!("removing {}: {e}", claude_path.display())))?;
            (ClaudeAction::FileRemoved, true)
        } else {
            write_file(&claude_path, &target)?;
            (ClaudeAction::BlockRemoved, true)
        }
    };

    // 3. ONE SessionStart hook per active module, plus a `Bash(<binary>:*)` allowlist entry per
    //    active module and `Bash(nxs:*)` for the umbrella, via the shared settings-merge seam.
    //
    //    The removal set spans the whole ROSTER, not just the active modules: that is what lets a
    //    module that has fallen out of `active_modules` take its hook entry with it — see
    //    `assemble_with`'s `roster_binaries`.
    let binaries: Vec<&str> = modules.iter().map(|m| m.binary.as_str()).collect();
    // The roster UNION the active modules. The roster is what catches a DEPARTED module; the active
    // binaries are folded in as well so that a still-active module whose entry carried the fallback
    // and no longer leads is recognized and dropped rather than left behind as a second `cat`.
    let mut managed_binaries: Vec<&str> = roster_binaries.to_vec();
    for binary in &binaries {
        if !managed_binaries.contains(binary) {
            managed_binaries.push(binary);
        }
    }
    let commands = hook_commands(&binaries);
    let managed = managed_hook_commands(&managed_binaries);
    let perms = assembler_permissions(modules);
    let perm_refs: Vec<&str> = perms.iter().map(String::as_str).collect();
    let wired = wire_session_hook(root, &commands, &managed, &perm_refs)?;

    Ok(AssembleReport {
        agents: agents_action,
        claude: claude_action,
        hook_added: wired.hook_added,
        hook_commands: wired.commands,
        permissions_added: wired.permissions_added,
        claude_changed,
        modules: modules.iter().map(|m| m.module.clone()).collect(),
    })
}

// ── AGENTS.md / CLAUDE.md block upsert (module-agnostic, marker-derived) ─────

/// The managed markers for a section: `(begin_prefix, end_marker)` derived from its own first
/// line `<!-- BEGIN <NAME> … -->`. So the assembler keys on each module's own markers without
/// hardcoding any module name. A section without a parseable BEGIN marker is a loud contract
/// violation (every module's `agent-manifest` section is a marker-delimited managed block).
fn markers_of(section: &str) -> Result<(String, String)> {
    let first = section.lines().next().unwrap_or("");
    let rest = first.strip_prefix("<!-- BEGIN ").ok_or_else(|| {
        NxfError::io(format!(
            "module manifest section does not start with a `<!-- BEGIN <NAME>` marker: {first:?}"
        ))
    })?;
    let name = rest.split_whitespace().next().unwrap_or("");
    if name.is_empty() {
        return Err(NxfError::io(
            "module manifest section has an empty managed-block name",
        ));
    }
    Ok((format!("<!-- BEGIN {name}"), format!("<!-- END {name} -->")))
}

/// Byte offset of the first occurrence of `needle` that BEGINS A LINE (column 0). The managed
/// markers are always written at column 0, so an INDENTED marker — the shape a marker takes when
/// it is quoted inside prose or an indented code block — is never matched.
///
/// **The column-0 case is not excluded, and since [`remove_section`] deletes, that is worth saying
/// out loud** (review of PR #413, Test Quality #3 / Integrity #2): a `BEGIN`/`END` pair written
/// flush left inside a fenced code block — someone documenting the managed block in their own
/// `CLAUDE.md` — is indistinguishable from the real thing to this function, and is treated as the
/// real thing. `a_marker_written_flush_left_inside_a_fence_is_still_a_marker` pins that behaviour
/// so the hazard is recorded rather than discovered. Indent such an example by one space and it is
/// prose again.
fn line_start_find(content: &str, needle: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = content[from..].find(needle) {
        let at = from + rel;
        if at == 0 || content.as_bytes()[at - 1] == b'\n' {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// Upsert one module's `section` into `content` by its own derived markers: rewrite the marked
/// region in place if present, else append after the existing text. Idempotent.
fn upsert_section(content: &str, section: &str) -> Result<String> {
    let (begin_prefix, end_marker) = markers_of(section)?;
    let region = line_start_find(content, &begin_prefix).and_then(|b| {
        line_start_find(&content[b..], &end_marker).map(|e| (b, b + e + end_marker.len()))
    });
    Ok(match region {
        Some((begin, end)) => format!("{}{section}{}", &content[..begin], &content[end..]),
        None if content.trim().is_empty() => format!("{section}\n"),
        None => format!("{}\n\n{section}\n", content.trim_end()),
    })
}

/// The Markdown header the managed `NEXUS` block opens with (nexus-flow-s1w). The block covers the
/// WHOLE suite (flow + memory + whatever else is active), so the wording is deliberately not
/// memory-only. The legacy per-module blocks each carried a `## nexus-<name>` header that was lost
/// in the nexus-flow-0lj.2 collapse to one pointer; this restores a header for the unified block so
/// the pointer reads as its own section in the agent files rather than a bare stray sentence.
const NEXUS_BLOCK_HEADER: &str = "## nexus-flow tools for agents";

/// The single `nxs`-owned discovery pointer (nexus-flow-0lj.2) written into the shared agent file:
/// one managed `NEXUS` block, opening with a Markdown header (nexus-flow-s1w), naming the active
/// tools (by key, for discovery) and pointing at `nxs prime`. Vocabulary-free and free of
/// operating/recovery prose — that lives in `nxs prime`, which the SessionStart hook runs (and
/// re-runs after a compaction). The marker is `NEXUS` (a space, not a hyphen, after it), distinct
/// from the legacy per-module `NEXUS-<NAME>` blocks.
fn nexus_discovery_block(modules: &[ModuleManifest]) -> String {
    let tools = modules
        .iter()
        .map(|m| m.module.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let tools_clause = if tools.is_empty() {
        String::new()
    } else {
        format!(" ({tools})")
    };
    format!(
        "<!-- BEGIN NEXUS v:1 — managed, do not edit (regenerated by `nxs init`) -->\n\
         {NEXUS_BLOCK_HEADER}\n\n\
         This project uses **nexus-flow**{tools_clause} — run `nxs prime` for project context and \
         how to work with it.\n\
         {}\
         <!-- END NEXUS -->",
        memory_doc_binding(modules)
    )
}

/// The `NEXUS_MEMORY.md` binding inside the discovery block (6j6v.8q88), or nothing when the memory
/// module is not active (there would be no file to bind, and the block must stay true of the
/// workspace it describes).
///
/// The hook already delivers this content to a host that runs SessionStart hooks. This line is for
/// every agent that does NOT — it reads `AGENTS.md` and nothing else. Claude Code expands the `@`
/// directive into an import; another reader simply sees the path of the file to open, which is why
/// the sentence around it names the file rather than relying on the directive alone.
///
/// **That reasoning is why the block belongs in `AGENTS.md` — and it is the same reasoning that
/// took it out of `CLAUDE.md`** (nxf 6j6v.q6e3). This comment named the distinction correctly from
/// the start; what was missing was the consequence, because the assembler wrote the block to both
/// files. In the file of a host that DOES run the hook, this line is an import of every memory's
/// body beside the index that host was just handed — see the module doc for the measurement.
///
/// The check that had to be answered before the change, and its answer: **is every `CLAUDE.md`
/// reader a host that runs the hook?** No — and this repository builds the counter-example itself.
/// A `nxc` persona session composes the project's `CLAUDE.md` into its system prompt
/// (`crate::role::compose_system_prompt`) with the SDK's `settingSources: []`, so no
/// `.claude/settings.json` loads and the SessionStart hook never fires. But that reader loses
/// nothing here: it is handed the composed `nxs prime --persona` text FIRST, ahead of `CLAUDE.md`
/// (6j6v.k8zq) — the same budgeted index the hook delivers — and it reads `CLAUDE.md` as plain
/// text, so it never expanded this directive in the first place. It was getting a line it could
/// not follow. Every other agent that reads a shared file without a hook reads `AGENTS.md`, which
/// is where the binding stays.
///
/// **That ordering is held by a test, not by this paragraph**:
/// `crates/chat/tests/orchestration_trigger.rs`'s
/// `trigger_composes_the_prime_block_then_claude_md_then_the_roles_own_prompt` asserts the memory
/// block lands BEFORE the `CLAUDE.md` body in the composed prompt. Named here because a reviewer
/// went looking for exactly that enforcement and concluded it was missing (review of PR #413, Code
/// Quality #3) — an argument that rests on a guarantee should say where the guarantee lives.
fn memory_doc_binding(modules: &[ModuleManifest]) -> String {
    if !modules.iter().any(|m| m.module == "memory") {
        return String::new();
    }
    "\nThe project's durable memory is projected into `NEXUS_MEMORY.md`, which is generated — \
     change it with `nxm remember`, never by hand:\n\n\
     @NEXUS_MEMORY.md\n"
        .to_string()
}

/// Remove any LEGACY per-module managed block (`<!-- BEGIN NEXUS-<NAME> … <!-- END NEXUS-<NAME> -->`
/// — e.g. flow's `NEXUS-FLOW`, memory's `NEXUS-MEMORY`) for the nexus-flow-0lj.2 migration to the
/// single `nxs`-owned `NEXUS` pointer. The new `<!-- BEGIN NEXUS -->` block has a SPACE (not a
/// hyphen) after `NEXUS`, so it is never matched — re-assembly stays idempotent.
///
/// Only when a block was actually removed are blank-line runs collapsed (so the gap a removed block
/// leaves does not widen); a file with no legacy block is returned **byte-for-byte**, never touching
/// whitespace the user wrote outside the managed region. An unterminated legacy block (a column-0
/// `<!-- BEGIN NEXUS-… -->` with no matching `<!-- END NEXUS-… -->`) is a corrupted managed region:
/// it is a loud `validation` error (matching the fail-loud, never-clobber style of the
/// settings-merge seam), not a silent orphan left behind.
fn strip_legacy_blocks(content: &str) -> Result<String> {
    let mut out = content.to_string();
    let mut stripped = false;
    while let Some(begin) = line_start_find(&out, "<!-- BEGIN NEXUS-") {
        let first_line = out[begin..].lines().next().unwrap_or("").to_string();
        let (_, end_marker) = markers_of(&first_line)?;
        let end_rel = line_start_find(&out[begin..], &end_marker).ok_or_else(|| {
            NxfError::validation(format!(
                "managed agent file has an unterminated legacy block: `{first_line}` has no \
                 matching `{end_marker}`. Repair or remove it, then re-run init."
            ))
        })?;
        let end = begin + end_rel + end_marker.len();
        out.replace_range(begin..end, "");
        stripped = true;
    }
    Ok(if stripped {
        collapse_blank_runs(&out)
    } else {
        out
    })
}

/// Collapse runs of 3+ consecutive newlines down to a single blank line, so removing a managed block
/// leaves no widening gap. Single/double newlines (paragraph breaks) are preserved.
fn collapse_blank_runs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newlines = 0usize;
    for ch in s.chars() {
        if ch == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push(ch);
            }
        } else {
            newlines = 0;
            out.push(ch);
        }
    }
    out
}

/// [`upsert_section`]'s other direction: remove `section`'s marked region from `content` if it is
/// there, and return `content` BYTE-FOR-BYTE when it is not.
///
/// This is what carries an existing workspace across nxf 6j6v.q6e3. The block is marked "managed,
/// regenerated by `nxs init`", so an owner who deletes it by hand gets it back on the next init of
/// any module — which is exactly what happened to the nine workspaces cleaned by hand on
/// 2026-08-30. Only the tool can retire what the tool writes.
///
/// It matches by the same derived markers [`upsert_section`] writes with, so a block from any
/// earlier version is removed whatever its version tag says, and the blank-line collapse is
/// [`strip_legacy_blocks`]'s: applied ONLY when something was removed, so a file this assembler has
/// no business in is returned untouched down to its whitespace.
fn remove_section(content: &str, section: &str) -> Result<String> {
    let (begin_prefix, end_marker) = markers_of(section)?;
    let Some((begin, end)) = line_start_find(content, &begin_prefix).and_then(|b| {
        line_start_find(&content[b..], &end_marker).map(|e| (b, b + e + end_marker.len()))
    }) else {
        return Ok(content.to_string());
    };
    let mut out = content.to_string();
    out.replace_range(begin..end, "");
    Ok(collapse_blank_runs(&out))
}

// ── .claude/settings.json: one SessionStart hook + permission allowlist ──────

/// What [`wire_session_hook`] changed in `.claude/settings.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookWireReport {
    /// Whether the SessionStart wiring CHANGED this run — an entry added, a stale/superseded one
    /// removed, or the entries reordered (false ⇒ the document this run would write is the one
    /// already on disk).
    ///
    /// **Measured, not inferred, since nxf 6j6v.shwz:** the wiring is rewritten from scratch every
    /// run, so counting insertions and removals would report a change every time. The settings are
    /// compared before and after instead.
    ///
    /// **Superseded 2026-08-28 (nxf n2m6 + a2a1).** This used to mean "the single SessionStart hook
    /// → `nxs prime` was newly added", which was a complete account while there was exactly one
    /// entry to add and nothing was ever removed on a steady-state run. With one entry per active
    /// module, "added" alone would report `false` for the run that REWIRES a workspace from the old
    /// single `nxs prime` hook — a run that changed the file — so it now covers removals too.
    pub hook_added: bool,
    /// The SessionStart commands wired for this workspace, in order — one per active module, the
    /// first carrying [`HOOK_FALLBACK`]. Reported so a caller renders what was actually wired
    /// instead of re-deriving it (`nxs`/`nxf`/`nxm`/`nxc init` all print or serialize this).
    pub commands: Vec<String>,
    /// The permission allowlist entries newly added this run, in the order given.
    pub permissions_added: Vec<String>,
}

/// Wire one SessionStart hook per entry of `commands` (build it with [`hook_commands`]) plus the
/// given `permissions` allowlist entries into `<root>/.claude/settings.json`. Idempotent and
/// merge-only: unrelated hooks/permissions are preserved, and a re-run that needs no change leaves
/// the file untouched.
///
/// `managed` is every command this assembler may own — [`managed_hook_commands`] over the full
/// roster, not just the active modules — and is what makes the wiring a REWRITE rather than an
/// append: everything in `managed` is dropped first, then `commands` is written back IN ORDER.
/// Running twice therefore leaves the same document, a workspace on the old single `nxs prime` hook
/// is upgraded in place instead of gaining a fourth entry, and the order of the entries is the
/// caller's (nxf 6j6v.shwz) rather than a residue of which module was set up first.
///
/// This is the ONE settings-merge seam (spec §6.2): both the assembler ([`assemble`]) and
/// `nxs setup claude`/`nxs migrate` reach it, so the hook wiring can never drift between the entry
/// points.
pub fn wire_session_hook(
    root: &Path,
    commands: &[String],
    managed: &[String],
    permissions: &[&str],
) -> Result<HookWireReport> {
    let dir = root.join(".claude");
    let path = dir.join("settings.json");

    let mut settings = read_settings(&path)?;
    // Drop EVERYTHING this assembler owns — superseded umbrella hooks, the entries of modules that
    // are no longer active, and the wanted ones too — then write the wanted set back in order.
    //
    // **Superseded 2026-08-29 (nxf 6j6v.shwz): the wanted commands used to be EXCLUDED from the
    // removal set.** The reason was sound and is worth keeping: "remove-then-add would report a
    // change every single time and `nxs setup claude` would never be able to say 'already
    // configured'". But excluding them means a surviving entry keeps its position while a rewritten
    // one is appended, so the ORDER in the file is a residue of the workspace's history rather than
    // something anybody decided — which is exactly the defect shwz names. The hooks are written in
    // the order `nxs prime` fans out, and an order that only holds for workspaces with the right
    // history does not hold.
    //
    // The no-op property is kept, by measuring it instead of inferring it: the settings are
    // compared before and after. A steady-state re-run removes and re-adds the same three entries
    // and produces the identical document, so nothing is written and nothing is reported. What
    // changes once, for a workspace whose entries sat in another order, is the order — and then
    // never again.
    let before = settings.clone();
    remove_managed_hooks(&mut settings, managed);
    for command in commands {
        ensure_session_start_hook(&mut settings, command);
    }
    let changed = settings != before;
    let mut permissions_added = Vec::new();
    for entry in permissions {
        if ensure_permission(&mut settings, entry) {
            permissions_added.push((*entry).to_string());
        }
    }

    if changed || !permissions_added.is_empty() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| NxfError::io(format!("creating {}: {e}", dir.display())))?;
        write_settings(&path, &settings)?;
    }
    Ok(HookWireReport {
        hook_added: changed,
        commands: commands.to_vec(),
        permissions_added,
    })
}

/// Drop every SessionStart hook running one of `managed`, and any hook group left empty by the
/// removal. Returns whether anything was actually removed.
///
/// Unrelated SessionStart hooks (a user's own, another tool's `bd prime`) are untouched — this
/// matches on the exact commands this assembler itself writes or has written, nothing else. See
/// [`managed_hook_commands`] for how that set is built and why it is not a pattern match.
fn remove_managed_hooks(settings: &mut Value, managed: &[String]) -> bool {
    let Some(groups) = settings
        .get_mut("hooks")
        .and_then(|h| h.get_mut("SessionStart"))
        .and_then(Value::as_array_mut)
    else {
        return false;
    };
    let before = groups.len();
    let mut removed = false;
    for group in groups.iter_mut() {
        if let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            let n = hooks.len();
            hooks.retain(|h| {
                !h.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|c| managed.iter().any(|m| m == c))
            });
            removed |= hooks.len() != n;
        }
    }
    groups.retain(|g| {
        g.get("hooks")
            .and_then(Value::as_array)
            .map(|a| !a.is_empty())
            .unwrap_or(true)
    });
    removed || groups.len() != before
}

/// The allowlist entries the assembler wires: the umbrella binary plus each active module's
/// binary. Deterministic: `nxs` first, then the modules in assembly order.
///
/// **Superseded 2026-08-28 (nxf n2m6 + a2a1):** `Bash(nxs:*)` used to be justified here as "the
/// hook runs `nxs prime`". It no longer does — the hooks run each module's own `prime`, which is
/// what the per-module entries cover. `nxs` stays on the list for what an agent types by hand:
/// `nxs prime` after a compaction, plus `nxs doctor`/`nxs sync`.
fn assembler_permissions(modules: &[ModuleManifest]) -> Vec<String> {
    let mut perms = vec!["Bash(nxs:*)".to_string()];
    perms.extend(modules.iter().map(|m| format!("Bash({}:*)", m.binary)));
    perms
}

fn read_settings(path: &Path) -> Result<Value> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({})),
        Err(e) => return Err(NxfError::io(format!("reading {}: {e}", path.display()))),
    };
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_str(&raw)
        .map_err(|e| NxfError::validation(format!("parsing {}: {e}", path.display())))?;
    if !value.is_object() {
        return Err(NxfError::validation(format!(
            "{} is not a JSON object",
            path.display()
        )));
    }
    Ok(value)
}

/// Ensure a SessionStart hook running `command` exists. Detects an existing one by the command
/// string (beads' marker-free convention), so re-runs never duplicate it; unrelated SessionStart
/// entries are preserved. Returns whether it was newly added.
fn ensure_session_start_hook(settings: &mut Value, command: &str) -> bool {
    // **Superseded 2026-08-28 (nxf n2m6 + a2a1): the upgrade step moved OUT of this function.** It
    // used to call `remove_superseded_hooks` here — "upgrade before adding (6j6v.8q88)", because
    // the detection below keys on the exact command string and a workspace wired by an older
    // assembler would otherwise end up with BOTH hooks and print its session block twice. That
    // reasoning is unchanged and still load-bearing; only its PLACE moved, to
    // `wire_session_hook`, which now removes the whole managed set ONCE and then ensures several
    // commands. Removing per ensured command would have re-run the sweep three times and, worse,
    // let the second call delete the entry the first had just written.
    let arr = obj_entry_array(
        obj_entry_object(object_mut(settings), "hooks"),
        "SessionStart",
    );
    let already = arr.iter().any(|group| {
        group
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|hs| {
                hs.iter()
                    .any(|h| h.get("command").and_then(Value::as_str) == Some(command))
            })
    });
    if already {
        return false;
    }
    arr.push(json!({
        "matcher": "",
        "hooks": [ { "type": "command", "command": command } ],
    }));
    true
}

/// Ensure `permissions.allow` contains `entry`. Idempotent by set membership. Returns whether added.
fn ensure_permission(settings: &mut Value, entry: &str) -> bool {
    let allow = obj_entry_array(
        obj_entry_object(object_mut(settings), "permissions"),
        "allow",
    );
    if allow.iter().any(|v| v.as_str() == Some(entry)) {
        return false;
    }
    allow.push(json!(entry));
    true
}

fn object_mut(settings: &mut Value) -> &mut Map<String, Value> {
    settings
        .as_object_mut()
        .expect("settings is a JSON object (read_settings guarantees it)")
}

fn obj_entry_object<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Value {
    let slot = map.entry(key).or_insert_with(|| json!({}));
    if !slot.is_object() {
        *slot = json!({});
    }
    slot
}

fn obj_entry_array<'a>(parent: &'a mut Value, key: &str) -> &'a mut Vec<Value> {
    let map = parent
        .as_object_mut()
        .expect("parent is an object (obj_entry_object guarantees it)");
    let slot = map.entry(key).or_insert_with(|| json!([]));
    if !slot.is_array() {
        *slot = json!([]);
    }
    slot.as_array_mut().expect("slot was just set to an array")
}

// ── atomic file writers (mirror the foundation / onboarding writers) ─────────

fn read_or_empty(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(NxfError::io(format!("reading {}: {e}", path.display()))),
    }
}

fn write_file(path: &Path, content: &str) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("nxs-agent");
    let tmp = dir.join(format!(".{name}.nxs.tmp.{}", std::process::id()));
    std::fs::write(&tmp, content)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            NxfError::io(format!("writing {}: {e}", path.display()))
        })
}

fn write_settings(path: &Path, settings: &Value) -> Result<()> {
    let body = format!("{}\n", serde_json::to_string_pretty(settings).unwrap());
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(".settings.json.nxs.tmp.{}", std::process::id()));
    std::fs::write(&tmp, &body)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            NxfError::io(format!("writing {}: {e}", path.display()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nxs_foundation::manifest::{HookSpec, SESSION_START};
    use tempfile::TempDir;

    fn module(name: &str, binary: &str) -> ModuleManifest {
        // Since nexus-flow-0lj.2 a module declares no agent-file block — only its prime fan-out
        // target + the hook (the assembler composes the single nxs-owned discovery pointer itself).
        ModuleManifest {
            module: name.to_string(),
            binary: binary.to_string(),
            manifest: AgentManifest {
                prime_command: format!("{binary} prime"),
                hook: HookSpec {
                    event: SESSION_START.to_string(),
                    command: format!("{binary} prime"),
                },
            },
        }
    }

    /// Build a LEGACY per-module block as the OLD assembler wrote it (for migration tests).
    fn legacy_block(name: &str, binary: &str) -> String {
        let upper = name.to_uppercase();
        format!(
            "<!-- BEGIN NEXUS-{upper} v:1 — managed, do not edit -->\n## nexus-{name}\n\nuse `{binary} prime`\n<!-- END NEXUS-{upper} -->"
        )
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }
    fn count(h: &str, n: &str) -> usize {
        h.matches(n).count()
    }
    fn wired_commands(settings: &Value) -> Vec<String> {
        settings["hooks"]["SessionStart"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["hooks"].as_array().unwrap().clone())
            .filter_map(|h| h["command"].as_str().map(String::from))
            .collect()
    }
    fn settings(root: &Path) -> Value {
        serde_json::from_str(&read(&root.join(".claude/settings.json"))).unwrap()
    }

    // ---- the fallback hook + the NEXUS_MEMORY.md binding (6j6v.8q88) -------

    #[test]
    fn the_hook_falls_back_to_the_projected_document() {
        // `.claude/settings.json` is committed, so a contributor WITHOUT nexus-flow gets this hook
        // too — and on their machine the first half is not a command at all. The second half is
        // what still hands them the project's memory.
        let tmp = TempDir::new().unwrap();
        assemble(tmp.path(), &[module("memory", "nxm")]).unwrap();
        assert_eq!(
            wired_commands(&settings(tmp.path())),
            vec!["nxm prime || cat NEXUS_MEMORY.md 2>/dev/null"]
        );
    }

    #[test]
    fn the_fallback_hangs_on_the_memory_entry_and_on_no_other() {
        // Ruling R1 (nxf a2a1) put the `|| cat` on the FIRST wired entry; nxf 6j6v.1k6y moved it to
        // memory's. R1's reasoning was sound for a SUITE-level tail and this is not one:
        // `NEXUS_MEMORY.md` is memory's own projection, so it stands in for `nxm prime` and for
        // nothing else. Still exactly one entry, for R1's other reason — three entries carrying it
        // would hand the same file to one session three times.
        let tmp = TempDir::new().unwrap();
        assemble(
            tmp.path(),
            &[
                module("flow", "nxf"),
                module("chat", "nxc"),
                module("memory", "nxm"),
            ],
        )
        .unwrap();
        let commands = wired_commands(&settings(tmp.path()));
        assert_eq!(
            commands,
            vec![
                "nxf prime",
                "nxc prime",
                "nxm prime || cat NEXUS_MEMORY.md 2>/dev/null",
            ],
            "one entry per module, and the fallback on memory's"
        );
        assert_eq!(
            commands
                .iter()
                .filter(|c| c.contains("cat NEXUS_MEMORY.md"))
                .count(),
            1,
            "the file is delivered once, not once per module: {commands:?}"
        );

        // …and it does not follow the lead: with flow absent, memory still carries it from second
        // place, which is the whole difference from the rule this replaces.
        let other = TempDir::new().unwrap();
        assemble(
            other.path(),
            &[module("chat", "nxc"), module("memory", "nxm")],
        )
        .unwrap();
        assert_eq!(
            wired_commands(&settings(other.path())),
            vec!["nxc prime", "nxm prime || cat NEXUS_MEMORY.md 2>/dev/null"]
        );

        // …and a workspace WITHOUT memory carries it nowhere: the file is written by `nxm`, so
        // there is nothing there to fall back to. It used to ride `nxf prime` and could not once
        // produce a byte.
        let no_memory = TempDir::new().unwrap();
        assemble(
            no_memory.path(),
            &[module("flow", "nxf"), module("chat", "nxc")],
        )
        .unwrap();
        let commands = wired_commands(&settings(no_memory.path()));
        assert_eq!(commands, vec!["nxf prime", "nxc prime"]);
        assert!(
            !commands.iter().any(|c| c.contains("NEXUS_MEMORY.md")),
            "no document, no fallback: {commands:?}"
        );
    }

    #[test]
    fn a_workspace_on_the_old_fallback_placement_has_it_moved_not_duplicated() {
        // The acceptance criterion of nxf 6j6v.1k6y: every workspace out there was wired by the
        // position rule, so the fix is only real if an existing one converges. Nothing new is
        // needed for it — `managed_hook_commands` already carries BOTH forms of every binary, so
        // the old `nxf prime || cat …` is stale the moment it is not what this build wants.
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(
            tmp.path().join(".claude/settings.json"),
            format!(
                r#"{{ "hooks": {{ "SessionStart": [
                    {{ "hooks": [ {{ "type": "command", "command": "nxf prime{HOOK_FALLBACK}" }} ] }},
                    {{ "hooks": [ {{ "type": "command", "command": "nxm prime" }} ] }},
                    {{ "hooks": [ {{ "type": "command", "command": "bd prime" }} ] }}
                ] }} }}"#
            ),
        )
        .unwrap();

        let report = assemble(
            tmp.path(),
            &[module("flow", "nxf"), module("memory", "nxm")],
        )
        .unwrap();

        // The neighbour's hook keeps its place and ours are re-appended after it: BOTH of the old
        // entries are stale here — `nxf prime || cat …` because the tail left it, and the bare
        // `nxm prime` because the tail joined it — so the rewrite removes the pair and writes the
        // wanted set fresh. Our own two keep their relative order.
        let commands = wired_commands(&settings(tmp.path()));
        assert_eq!(
            commands,
            vec![
                "bd prime",
                "nxf prime",
                "nxm prime || cat NEXUS_MEMORY.md 2>/dev/null"
            ],
            "the tail moved to memory, flow lost it, and the neighbour is untouched"
        );
        assert_eq!(
            commands
                .iter()
                .filter(|c| c.contains("NEXUS_MEMORY.md"))
                .count(),
            1,
            "moved, not duplicated: {commands:?}"
        );
        assert!(report.hook_added, "and the move is reported as a change");
    }

    #[test]
    fn a_workspace_wired_by_an_older_assembler_is_upgraded_not_doubled() {
        // The hook is detected by its exact command string, so without the upgrade an existing
        // workspace would end up running BOTH and printing its session block twice.
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(
            tmp.path().join(".claude/settings.json"),
            r#"{"hooks":{"SessionStart":[{"matcher":"","hooks":[{"type":"command","command":"nxs prime"}]}]}}"#,
        )
        .unwrap();

        assemble(tmp.path(), &[module("memory", "nxm")]).unwrap();
        assert_eq!(
            wired_commands(&settings(tmp.path())),
            vec!["nxm prime || cat NEXUS_MEMORY.md 2>/dev/null"],
            "the superseded command is replaced, not joined"
        );
    }

    #[test]
    fn a_workspace_on_the_single_umbrella_hook_is_rewired_to_one_entry_per_module() {
        // The migration this change owes an existing workspace (nxf n2m6): the composed
        // `nxs prime` hook is REPLACED by the per-module set, not topped up. Left in place it
        // would deliver the whole fan-out a fourth time alongside the three module blocks.
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(
            tmp.path().join(".claude/settings.json"),
            r#"{"hooks":{"SessionStart":[{"matcher":"","hooks":[{"type":"command","command":"nxs prime || cat NEXUS_MEMORY.md 2>/dev/null"}]}]}}"#,
        )
        .unwrap();

        let report = assemble(
            tmp.path(),
            &[module("flow", "nxf"), module("memory", "nxm")],
        )
        .unwrap();
        assert_eq!(
            wired_commands(&settings(tmp.path())),
            vec!["nxf prime", "nxm prime || cat NEXUS_MEMORY.md 2>/dev/null"],
            "the umbrella entry is gone, one entry per module took its place"
        );
        assert!(report.hook_added, "the rewiring is reported as a change");
    }

    #[test]
    fn running_twice_leaves_exactly_one_entry_per_module() {
        // Idempotent + merge-only (nxf n2m6 DoD): a second `nxs setup` leaves no fourth entry, and
        // reports no change at all.
        let tmp = TempDir::new().unwrap();
        let modules = [
            module("flow", "nxf"),
            module("chat", "nxc"),
            module("memory", "nxm"),
        ];
        assemble(tmp.path(), &modules).unwrap();
        let second = assemble(tmp.path(), &modules).unwrap();
        assert_eq!(
            wired_commands(&settings(tmp.path())),
            vec![
                "nxf prime",
                "nxc prime",
                "nxm prime || cat NEXUS_MEMORY.md 2>/dev/null",
            ]
        );
        assert!(
            !second.hook_added,
            "a steady-state re-run changes nothing — remove-then-re-add would report a change here"
        );
    }

    #[test]
    fn a_module_that_falls_away_takes_its_hook_entry_with_it() {
        // The other half of "a module that joins brings its entry": re-assembling WITHOUT chat
        // drops `nxc prime`, rather than leaving a hook that runs a module this workspace no
        // longer has. Covered here through the active binaries, which `assemble` folds into the
        // managed set precisely so this does not depend on `inventory` being populated.
        let tmp = TempDir::new().unwrap();
        assemble(
            tmp.path(),
            &[
                module("flow", "nxf"),
                module("chat", "nxc"),
                module("memory", "nxm"),
            ],
        )
        .unwrap();
        let report = assemble_with(
            tmp.path(),
            &[module("flow", "nxf"), module("memory", "nxm")],
            &["nxf", "nxc", "nxm"],
        )
        .unwrap();
        assert_eq!(
            wired_commands(&settings(tmp.path())),
            vec!["nxf prime", "nxm prime || cat NEXUS_MEMORY.md 2>/dev/null"],
            "chat's entry left with chat"
        );
        assert!(report.hook_added, "the removal is reported as a change");
    }

    #[test]
    fn an_unrelated_session_start_hook_survives_the_upgrade() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(
            tmp.path().join(".claude/settings.json"),
            r#"{"hooks":{"SessionStart":[{"matcher":"","hooks":[
                {"type":"command","command":"nxs prime"},
                {"type":"command","command":"./scripts/my-own-hook.sh"}]}]}}"#,
        )
        .unwrap();

        assemble(tmp.path(), &[module("memory", "nxm")]).unwrap();
        let commands = wired_commands(&settings(tmp.path()));
        assert!(commands.contains(&"./scripts/my-own-hook.sh".to_string()));
        assert!(commands.contains(&format!("nxm prime{}", HOOK_FALLBACK)));
        assert_eq!(commands.len(), 2, "{commands:?}");
    }

    #[test]
    fn agents_md_binds_the_projected_document_when_memory_is_active() {
        let tmp = TempDir::new().unwrap();
        assemble(
            tmp.path(),
            &[module("flow", "nxf"), module("memory", "nxm")],
        )
        .unwrap();
        let agents = read(&tmp.path().join("AGENTS.md"));
        assert!(agents.contains("@NEXUS_MEMORY.md"), "{agents}");
        assert!(
            agents.contains("never by hand"),
            "an agent that cannot expand the import still learns not to edit it: {agents}"
        );
    }

    #[test]
    fn agents_md_binds_nothing_when_memory_is_not_active() {
        // The block must stay true of the workspace it describes — a flow-only checkout has no
        // such file, and pointing at one would be an instruction to open something absent.
        let tmp = TempDir::new().unwrap();
        assemble(tmp.path(), &[module("flow", "nxf")]).unwrap();
        assert!(!read(&tmp.path().join("AGENTS.md")).contains("NEXUS_MEMORY.md"));
    }

    #[test]
    fn re_assembling_with_the_binding_stays_byte_idempotent() {
        let tmp = TempDir::new().unwrap();
        let modules = [module("flow", "nxf"), module("memory", "nxm")];
        assemble(tmp.path(), &modules).unwrap();
        let once = read(&tmp.path().join("AGENTS.md"));
        assemble(tmp.path(), &modules).unwrap();
        assert_eq!(once, read(&tmp.path().join("AGENTS.md")));
    }

    // ---- collect_with (shell-out core, injected runner) --------------------

    /// A roster descriptor for the collect tests (the assembler only reads `.binary`/`.key`).
    fn init_module(key: &'static str, binary: &'static str, order: u16) -> ModuleInit {
        fn noop(_: &crate::module::InitRequest) -> Result<()> {
            Ok(())
        }
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

    #[test]
    fn collect_with_parses_each_modules_manifest_in_order() {
        let (flow, memory) = (
            init_module("flow", "nxf", 10),
            init_module("memory", "nxm", 20),
        );
        let roster = vec![&flow, &memory];
        let modules = crate::module::resolve_active(&roster, &cfg(&["flow", "memory"])).unwrap();
        let out = collect_with(&modules, |binary| {
            let m = module(if binary == "nxf" { "flow" } else { "memory" }, binary);
            Ok(serde_json::to_string(&m.manifest).unwrap())
        })
        .unwrap();
        assert_eq!(
            out.iter().map(|m| m.binary.as_str()).collect::<Vec<_>>(),
            vec!["nxf", "nxm"],
            "collected in the order given — `collect_with` preserves its input, and since \
             nxf 6j6v.shwz the sort into roster order happens one layer up, in `assemble_with`"
        );
    }

    #[test]
    fn collect_with_surfaces_a_module_spawn_failure() {
        let flow = init_module("flow", "nxf", 10);
        let roster = vec![&flow];
        let modules = crate::module::resolve_active(&roster, &cfg(&["flow"])).unwrap();
        let err = collect_with(&modules, |_| Err(NxfError::io("boom"))).unwrap_err();
        assert!(err.msg.contains("boom"));
    }

    fn cfg(modules: &[&str]) -> nxs_foundation::workspace::WorkspaceConfig {
        nxs_foundation::workspace::WorkspaceConfig {
            active_modules: modules.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    // ---- assemble: agent files ---------------------------------------------

    #[test]
    fn flow_only_assembles_the_single_pointer_and_one_hook() {
        let tmp = TempDir::new().unwrap();
        let report = assemble(tmp.path(), &[module("flow", "nxf")]).unwrap();
        let agents = read(&tmp.path().join("AGENTS.md"));
        // ONE nxs-owned NEXUS pointer — never a per-module NEXUS-FLOW block (0lj.2).
        assert_eq!(
            count(&agents, "<!-- BEGIN NEXUS "),
            1,
            "one pointer: {agents}"
        );
        assert_eq!(
            count(&agents, "<!-- BEGIN NEXUS-"),
            0,
            "no per-module block: {agents}"
        );
        assert!(agents.contains("(flow)"), "names the active tool: {agents}");
        assert!(
            agents.contains("nxs prime"),
            "points at nxs prime: {agents}"
        );
        assert_eq!(report.agents, AgentsAction::Created);
        // One hook, for the one active module — and NO fallback on it (nxf 6j6v.1k6y): the
        // `NEXUS_MEMORY.md` it would `cat` is written by `nxm`, which is not active here, so the
        // tail could never have produced a byte. **Superseded 2026-08-28 (nxf n2m6 + a2a1):** this
        // used to assert the single
        // umbrella command "and it is the umbrella's — NOT `nxf prime`", which was the whole point
        // while one hook carried the composed fan-out. `nxf prime` is now exactly what a flow-only
        // workspace wires; see the module doc for the measurement behind the change.
        assert_eq!(
            wired_commands(&settings(tmp.path())),
            vec!["nxf prime".to_string()]
        );
        assert!(report.hook_added);
    }

    #[test]
    fn flow_plus_memory_assembles_one_pointer_naming_both_with_one_hook() {
        let tmp = TempDir::new().unwrap();
        let report = assemble(
            tmp.path(),
            &[module("flow", "nxf"), module("memory", "nxm")],
        )
        .unwrap();
        let agents = read(&tmp.path().join("AGENTS.md"));
        // STILL one pointer (not two blocks); it names both tools in assembly order.
        assert_eq!(
            count(&agents, "<!-- BEGIN NEXUS "),
            1,
            "one pointer: {agents}"
        );
        assert_eq!(
            count(&agents, "<!-- BEGIN NEXUS-"),
            0,
            "no per-module blocks: {agents}"
        );
        assert!(
            agents.contains("(flow, memory)"),
            "names both tools, flow before memory (assembly order): {agents}"
        );
        // No per-module operating/recovery prose leaks into AGENTS.md (it lives in `nxs prime`).
        // `NEXUS_MEMORY.md` is not prose but a BINDING (6j6v.8q88) — the file an agent that reads
        // no hooks must still open — so the guard names what it actually forbids: a module's own
        // rules, and the `MEMORY.md` a memory rule used to argue against.
        let lower = agents.to_lowercase();
        assert!(
            !lower.contains("compaction") && !lower.contains(" memory.md"),
            "no operating/recovery prose in AGENTS.md: {agents}"
        );
        // ONE HOOK PER MODULE now (nxf n2m6 + a2a1) — this used to assert that flow-only and
        // flow+memory wire the same single `nxs prime`, which stopped being true when the budget
        // turned out to be per hook output. Memory is active, so memory carries the fallback —
        // wherever it sits (nxf 6j6v.1k6y).
        assert_eq!(
            wired_commands(&settings(tmp.path())),
            vec![
                "nxf prime".to_string(),
                format!("nxm prime{}", HOOK_FALLBACK)
            ]
        );
        assert_eq!(report.modules, vec!["flow", "memory"]);
        // Permission allowlist: nxs (the hook) + each module binary.
        let allow = settings(tmp.path())["permissions"]["allow"].clone();
        for entry in ["Bash(nxs:*)", "Bash(nxf:*)", "Bash(nxm:*)"] {
            assert!(
                allow.as_array().unwrap().iter().any(|v| v == entry),
                "missing {entry}"
            );
        }
    }

    #[test]
    fn re_assembly_when_a_second_module_joins_refreshes_the_one_pointer_no_double_hook() {
        // flow-only first, then flow+memory (the nxm-after-nxf re-assembly) — still exactly one
        // pointer (now naming both) and one hook, no duplication.
        let tmp = TempDir::new().unwrap();
        assemble(tmp.path(), &[module("flow", "nxf")]).unwrap();
        let report = assemble(
            tmp.path(),
            &[module("flow", "nxf"), module("memory", "nxm")],
        )
        .unwrap();
        let agents = read(&tmp.path().join("AGENTS.md"));
        assert_eq!(
            count(&agents, "<!-- BEGIN NEXUS "),
            1,
            "pointer not duplicated: {agents}"
        );
        assert!(
            agents.contains("(flow, memory)"),
            "refreshed to name both: {agents}"
        );
        assert_eq!(
            wired_commands(&settings(tmp.path())),
            vec![
                "nxf prime".to_string(),
                format!("nxm prime{}", HOOK_FALLBACK)
            ],
            "re-assembly never duplicates a hook — and the joining module brings its own \
             (with the fallback, which it owns and which flow therefore gives up)"
        );
        assert!(
            report.hook_added,
            "the second pass ADDS memory's entry (nxf n2m6): before one hook per module this \
             asserted the opposite, because there was nothing left to add once `nxs prime` was in"
        );
    }

    #[test]
    fn migrates_legacy_per_module_blocks_to_the_single_pointer() {
        // nexus-flow-0lj.2 migration: an existing AGENTS.md with the OLD two per-module blocks is
        // rebuilt to the single nxs-owned pointer — old markers gone, no orphans, user text kept.
        let tmp = TempDir::new().unwrap();
        let legacy = format!(
            "# House rules\n\nbe kind\n\n{}\n\n{}\n",
            legacy_block("flow", "nxf"),
            legacy_block("memory", "nxm"),
        );
        std::fs::write(tmp.path().join("AGENTS.md"), &legacy).unwrap();
        assemble(
            tmp.path(),
            &[module("flow", "nxf"), module("memory", "nxm")],
        )
        .unwrap();
        let agents = read(&tmp.path().join("AGENTS.md"));
        assert_eq!(
            count(&agents, "<!-- BEGIN NEXUS-FLOW"),
            0,
            "old flow marker gone: {agents}"
        );
        assert_eq!(
            count(&agents, "<!-- BEGIN NEXUS-MEMORY"),
            0,
            "old memory marker gone: {agents}"
        );
        assert_eq!(
            count(&agents, "<!-- END NEXUS-"),
            0,
            "no orphan end markers: {agents}"
        );
        assert_eq!(
            count(&agents, "<!-- BEGIN NEXUS "),
            1,
            "one new pointer: {agents}"
        );
        assert!(
            agents.contains("# House rules"),
            "user text preserved: {agents}"
        );
        // No widening blank-line gap left where the blocks were.
        assert!(
            !agents.contains("\n\n\n"),
            "no orphan blank-line run: {agents:?}"
        );
    }

    #[test]
    fn re_running_the_same_assembly_is_byte_idempotent() {
        let tmp = TempDir::new().unwrap();
        let mods = [module("flow", "nxf"), module("memory", "nxm")];
        assemble(tmp.path(), &mods).unwrap();
        let agents_once = read(&tmp.path().join("AGENTS.md"));
        let settings_once = read(&tmp.path().join(".claude/settings.json"));
        assemble(tmp.path(), &mods).unwrap();
        assert_eq!(
            read(&tmp.path().join("AGENTS.md")),
            agents_once,
            "AGENTS.md byte-stable"
        );
        assert_eq!(
            read(&tmp.path().join(".claude/settings.json")),
            settings_once,
            "settings.json byte-stable"
        );
    }

    #[test]
    fn no_claude_md_is_left_absent() {
        let tmp = TempDir::new().unwrap();
        let report = assemble(tmp.path(), &[module("flow", "nxf")]).unwrap();
        assert_eq!(report.claude, ClaudeAction::Absent);
        assert!(!report.claude_changed, "absent ⇒ nothing changed");
        assert!(!tmp.path().join("CLAUDE.md").exists(), "never conjured");
    }

    #[test]
    fn a_preexisting_claude_md_is_returned_byte_for_byte() {
        // The whole of the new contract in one case: `CLAUDE.md` belongs to the host that RUNS the
        // hook, so the assembler puts nothing in it — not the block, and not an `@AGENTS.md`
        // import, which would pull the same block in by another door AND, through its
        // `@NEXUS_MEMORY.md` line, the memory's whole body text beside the budgeted index the hook
        // just delivered (nxf 6j6v.q6e3).
        let tmp = TempDir::new().unwrap();
        let mine = "# my notes\n\nwhatever I keep here.\n";
        std::fs::write(tmp.path().join("CLAUDE.md"), mine).unwrap();
        let report = assemble(tmp.path(), &[module("flow", "nxf")]).unwrap();
        assert_eq!(report.claude, ClaudeAction::Untouched);
        assert!(!report.claude_changed, "nothing was written: {report:?}");
        assert_eq!(
            read(&tmp.path().join("CLAUDE.md")),
            mine,
            "byte-for-byte, down to the whitespace"
        );
    }

    #[test]
    fn the_memory_binding_goes_to_agents_md_and_never_to_claude_md() {
        // The measured defect (nxf 6j6v.q6e3): `@NEXUS_MEMORY.md` in `CLAUDE.md` is expanded by
        // Claude Code into an import of the memory's whole body — 95-120 KB per session in the
        // product repos, ten to twenty times the budgeted index the SessionStart hook delivers.
        // This test is the one that goes red if somebody writes the binding to both files again.
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "# shared\n").unwrap();
        std::fs::write(tmp.path().join("CLAUDE.md"), "# mine\n").unwrap();
        assemble(tmp.path(), &[module("memory", "nxm")]).unwrap();

        let agents = read(&tmp.path().join("AGENTS.md"));
        assert!(
            agents.contains("@NEXUS_MEMORY.md"),
            "the hookless reader's binding stays exactly where it was meant: {agents}"
        );
        let claude = read(&tmp.path().join("CLAUDE.md"));
        assert!(
            !claude.contains("@NEXUS_MEMORY.md") && !claude.contains("<!-- BEGIN NEXUS"),
            "and nothing of it reaches the file of the host that runs the hook: {claude}"
        );
    }

    #[test]
    fn a_block_an_earlier_version_wrote_is_removed_and_the_rest_of_the_file_kept() {
        // Existing workspaces are carried along by an ordinary run — the block is marked "managed,
        // regenerated by `nxs init`", so removing it by hand does not hold (nine workspaces were
        // cleaned by hand on 2026-08-30 and had it back by the next init).
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "# shared\n").unwrap();
        let mods = [module("flow", "nxf"), module("memory", "nxm")];
        // Plant exactly what an older assembler left behind.
        let planted = format!(
            "# my notes\n\nkeep me.\n\n{}\n",
            nexus_discovery_block(&mods)
        );
        std::fs::write(tmp.path().join("CLAUDE.md"), &planted).unwrap();

        let report = assemble(tmp.path(), &mods).unwrap();
        assert_eq!(report.claude, ClaudeAction::BlockRemoved);
        assert!(report.claude_changed, "the removal is a write: {report:?}");
        let out = read(&tmp.path().join("CLAUDE.md"));
        assert_eq!(
            count(&out, "<!-- BEGIN NEXUS"),
            0,
            "the block is gone, marker and all: {out}"
        );
        assert!(
            out.contains("# my notes") && out.contains("keep me."),
            "and the user's own text is untouched: {out}"
        );

        // …and the SECOND run is a true no-op, so this is a migration and not a churn.
        let again = assemble(tmp.path(), &mods).unwrap();
        assert_eq!(again.claude, ClaudeAction::Untouched);
        assert!(!again.claude_changed, "nothing left to remove: {again:?}");
    }

    #[test]
    fn a_claude_md_that_was_nothing_but_the_block_is_removed_rather_than_left_empty() {
        // Owner's call from the 2026-08-30 hand-run: in two of nine workspaces the whole file was
        // this block, and an empty file is worse than no file — a reader opens it and learns
        // nothing.
        let tmp = TempDir::new().unwrap();
        let claude = tmp.path().join("CLAUDE.md");
        std::fs::write(
            &claude,
            format!("{}\n", nexus_discovery_block(&[module("flow", "nxf")])),
        )
        .unwrap();
        let report = assemble(tmp.path(), &[module("flow", "nxf")]).unwrap();
        assert_eq!(report.claude, ClaudeAction::FileRemoved);
        assert!(report.claude_changed);
        assert!(!claude.exists(), "no hollow file left behind");
    }

    #[test]
    fn claude_md_with_legacy_inlined_blocks_is_migrated_to_nothing_at_all() {
        // The 0lj.2 migration and this one meet here: a CLAUDE.md carrying the OLD per-module
        // blocks used to be migrated to the single pointer. It is now migrated to no block at all,
        // and the legacy markers must not survive as orphans on the way out.
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "# shared\n").unwrap();
        let claude = format!("# shared\n\n{}\n", legacy_block("flow", "nxf"));
        std::fs::write(tmp.path().join("CLAUDE.md"), &claude).unwrap();
        let report = assemble(tmp.path(), &[module("flow", "nxf")]).unwrap();
        assert_eq!(report.claude, ClaudeAction::BlockRemoved);
        let out = read(&tmp.path().join("CLAUDE.md"));
        assert_eq!(
            count(&out, "<!-- BEGIN NEXUS"),
            0,
            "neither the legacy block nor a pointer in its place: {out}"
        );
        assert!(out.contains("# shared"), "the user's text stays: {out}");
    }

    #[test]
    fn merges_into_existing_settings_without_clobbering() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            r#"{ "hooks": { "SessionStart": [ { "matcher": "", "hooks": [ { "type": "command", "command": "bd prime" } ] } ] }, "model": "opus" }"#,
        )
        .unwrap();
        let report = assemble(tmp.path(), &[module("flow", "nxf")]).unwrap();
        assert!(report.hook_added);
        let s = settings(tmp.path());
        assert_eq!(s["model"], "opus", "unrelated keys preserved");
        let mut cmds = wired_commands(&s);
        cmds.sort();
        assert_eq!(
            cmds,
            vec!["bd prime", "nxf prime"],
            "a neighbouring tool's own SessionStart hook is never in the managed set"
        );
    }

    // ---- the per-module hook set, pure (nxf n2m6 + a2a1) -------------------

    #[test]
    fn the_fallback_hangs_on_the_module_whose_document_it_is_wherever_that_module_sits() {
        // nxf 6j6v.1k6y. `NEXUS_MEMORY.md` is memory's projection, so the entry it stands in for is
        // memory's — not whichever entry happens to be written first.
        assert_eq!(
            hook_commands(&["nxf", "nxc", "nxm"]),
            vec![
                "nxf prime",
                "nxc prime",
                "nxm prime || cat NEXUS_MEMORY.md 2>/dev/null",
            ]
        );
        // Position is not what decides it, so moving memory moves the tail with it.
        assert_eq!(
            hook_commands(&["nxm", "nxf"]),
            vec!["nxm prime || cat NEXUS_MEMORY.md 2>/dev/null", "nxf prime"]
        );
        // Memory alone still carries it — and now for the right reason.
        assert_eq!(
            hook_commands(&["nxm"]),
            vec!["nxm prime || cat NEXUS_MEMORY.md 2>/dev/null"]
        );
        // **Without memory, NOTHING carries it.** There is no document to fall back to: the file is
        // written by `nxm`, so a flow-only workspace never has one. It used to ride `nxf prime`
        // there and could not once produce a byte.
        assert_eq!(
            hook_commands(&["nxf", "nxc"]),
            vec!["nxf prime", "nxc prime"]
        );
        // No modules, no hooks — and no stray fallback-only entry.
        assert!(hook_commands(&[]).is_empty());
    }

    #[test]
    fn the_managed_set_covers_both_forms_and_the_retired_umbrella_hooks() {
        let managed = managed_hook_commands(&["nxf", "nxm"]);
        for expected in [
            // Retired: both `nxs prime` shapes that shipped (pre- and post-6j6v.8q88).
            "nxs prime",
            "nxs prime || cat NEXUS_MEMORY.md 2>/dev/null",
            // Both forms of each module. Not because the fallback moves between them any more
            // (nxf 6j6v.1k6y bound it to memory), but because a workspace wired by an OLDER
            // assembler carries the other placement — `nxf prime || cat …` — and this set is what
            // takes it away again.
            "nxf prime",
            "nxf prime || cat NEXUS_MEMORY.md 2>/dev/null",
            "nxm prime",
            "nxm prime || cat NEXUS_MEMORY.md 2>/dev/null",
        ] {
            assert!(
                managed.iter().any(|m| m == expected),
                "{expected:?} is one this assembler may own: {managed:?}"
            );
        }
        // And a neighbouring tool's hook is NOT in it — the reason this is an exact set built from
        // the roster and not a "ends with ` prime`" pattern.
        assert!(!managed.iter().any(|m| m == "bd prime"), "{managed:?}");
    }

    #[test]
    fn the_declared_hook_matches_what_the_assembler_wires() {
        // `hook_commands` derives `<binary> prime` rather than reading each module's declared
        // `manifest.hook.command`, so that `nxs migrate` — which has a roster but no manifests —
        // builds the identical list. This is the pin that keeps the two from drifting: what a
        // module DECLARES has to be what the assembler WIRES for it, modulo the fallback tail.
        //
        // **Corrected 2026-08-29 (nxf 6j6v.1k6y): the tail used to be appended to every module
        // here**, because a single-module list always carried it — it was the first. That made the
        // parenthesis "(modulo the fallback tail, which is the assembler's own and belongs to no
        // module)" read true, and it has stopped being: the tail belongs to memory. So the tail is
        // STRIPPED before the comparison rather than assumed, and which module carries it is
        // asserted separately — otherwise stripping would hide exactly the regression this task
        // fixed.
        for (key, binary) in [("flow", "nxf"), ("chat", "nxc"), ("memory", "nxm")] {
            let m = module(key, binary);
            assert_eq!(m.manifest.hook.event, SESSION_START);
            let wired = hook_commands(&[binary]);
            assert_eq!(wired.len(), 1, "one entry per binary: {wired:?}");
            assert_eq!(
                wired[0].strip_suffix(HOOK_FALLBACK).unwrap_or(&wired[0]),
                m.manifest.hook.command,
                "what {key} declares is what the assembler wires for it"
            );
            assert_eq!(
                wired[0].ends_with(HOOK_FALLBACK),
                binary == HOOK_FALLBACK_BINARY,
                "and only {HOOK_FALLBACK_BINARY} carries the tail: {wired:?}"
            );
        }
    }

    // ---- the nxs-owned pointer + the legacy-block migration (pure) ----------

    #[test]
    fn nexus_discovery_block_names_active_tools_and_points_at_prime() {
        let block = nexus_discovery_block(&[module("flow", "nxf"), module("memory", "nxm")]);
        // Well-formed single block, parseable by the same marker logic that upserts it.
        assert!(
            block.starts_with("<!-- BEGIN NEXUS "),
            "begins with the NEXUS marker: {block}"
        );
        assert!(
            block.trim_end().ends_with("<!-- END NEXUS -->"),
            "closes it: {block}"
        );
        assert!(markers_of(&block).is_ok(), "derivable markers: {block}");
        assert!(block.contains("(flow, memory)"), "lists the tools: {block}");
        assert!(
            block.contains("nxs prime"),
            "points at the umbrella: {block}"
        );
        // Vocabulary-free: no per-module operating/recovery prose.
        assert!(
            !block.to_lowercase().contains("compaction"),
            "no recovery prose: {block}"
        );
    }

    #[test]
    fn nexus_discovery_block_opens_with_a_markdown_header_before_the_prose() {
        // nexus-flow-s1w: the block body starts with a Markdown header on its OWN line, INSIDE the
        // managed markers, before the prose — so the pointer reads as its own section in the agent
        // files (restoring the header the 0lj.2 per-module→single-pointer collapse dropped).
        let block = nexus_discovery_block(&[module("flow", "nxf")]);
        let lines: Vec<&str> = block.lines().collect();
        assert!(
            lines[0].starts_with("<!-- BEGIN NEXUS "),
            "line 0 is the BEGIN marker: {block}"
        );
        assert_eq!(
            lines[1], NEXUS_BLOCK_HEADER,
            "line 1 is the Markdown header, on its own line: {block}"
        );
        assert!(
            lines[1].starts_with("## "),
            "the header is a Markdown h2: {block}"
        );
        // The header sits between the markers (not before BEGIN / after END).
        let header_at = block.find(NEXUS_BLOCK_HEADER).unwrap();
        assert!(
            header_at > block.find("<!-- BEGIN NEXUS ").unwrap()
                && header_at < block.find("<!-- END NEXUS -->").unwrap(),
            "header is inside the managed region: {block}"
        );
        // A blank line separates the header from the prose (Markdown convention).
        assert!(
            lines[2].is_empty() && lines[3].contains("This project uses"),
            "blank line then prose after the header: {block}"
        );
    }

    #[test]
    fn nexus_discovery_block_omits_the_parenthetical_when_no_tools() {
        let block = nexus_discovery_block(&[]);
        // No tool-list parenthetical after the product name (the marker's own "(regenerated…)" is
        // unrelated): the content reads "**nexus-flow** — run …", not "**nexus-flow** (…) — run …".
        assert!(
            !block.contains("** ("),
            "no empty tool parenthetical: {block}"
        );
        assert!(
            block.contains("**nexus-flow** —"),
            "names the product then dashes in: {block}"
        );
    }

    #[test]
    fn strip_legacy_blocks_removes_old_blocks_keeps_text_and_the_new_pointer() {
        let kept_pointer = "<!-- BEGIN NEXUS -->\nthe pointer\n<!-- END NEXUS -->";
        let content = format!(
            "# notes\n\n{}\n\n{}\n\n{kept_pointer}\n",
            legacy_block("flow", "nxf"),
            legacy_block("memory", "nxm"),
        );
        let stripped = strip_legacy_blocks(&content).unwrap();
        assert!(
            !stripped.contains("BEGIN NEXUS-FLOW"),
            "flow block removed: {stripped}"
        );
        assert!(
            !stripped.contains("BEGIN NEXUS-MEMORY"),
            "memory block removed: {stripped}"
        );
        assert!(
            !stripped.contains("END NEXUS-"),
            "no orphan end markers: {stripped}"
        );
        assert!(
            stripped.contains("# notes"),
            "user text preserved: {stripped}"
        );
        assert!(
            stripped.contains("<!-- BEGIN NEXUS "),
            "the new single pointer (a space, not a hyphen) is NOT stripped: {stripped}"
        );
        assert!(
            !stripped.contains("\n\n\n"),
            "no widening blank-line gap: {stripped:?}"
        );
    }

    #[test]
    fn strip_legacy_blocks_leaves_a_block_free_file_byte_for_byte() {
        // Code Quality #1: with NO legacy block to remove, the file is returned untouched — the
        // blank-run collapse must NOT normalize whitespace the user wrote outside a managed region.
        let content = "# notes\n\n\n\nintentional triple gap above\n\n<!-- BEGIN NEXUS -->\nthe pointer\n<!-- END NEXUS -->\n";
        let out = strip_legacy_blocks(content).unwrap();
        assert_eq!(out, content, "no legacy block ⇒ byte-for-byte identical");
    }

    #[test]
    fn strip_legacy_blocks_errors_loudly_on_an_unterminated_legacy_marker() {
        // Integrity #1: a column-0 legacy BEGIN with no matching END is a corrupted managed region —
        // a loud validation error (fail-loud, never clobber), not a silently orphaned marker.
        let content =
            "# notes\n\n<!-- BEGIN NEXUS-FLOW v:1 — managed -->\n## nexus-flow\n\norphan, no END\n";
        let err = strip_legacy_blocks(content).unwrap_err();
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Validation);
        assert!(
            err.msg.contains("unterminated") && err.msg.contains("END NEXUS-FLOW"),
            "names the missing closing marker: {}",
            err.msg
        );
    }

    #[test]
    fn assemble_errors_on_an_agents_file_with_an_unterminated_legacy_block() {
        // The fail-loud path reaches the caller: a workspace whose AGENTS.md was hand-corrupted to an
        // unterminated legacy block fails init rather than leaving an orphan or clobbering content.
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("AGENTS.md"),
            "# notes\n\n<!-- BEGIN NEXUS-FLOW v:1 -->\nbody, no end\n",
        )
        .unwrap();
        let err = assemble(tmp.path(), &[module("flow", "nxf")]).unwrap_err();
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Validation);
    }

    #[test]
    fn assemble_errors_on_a_claude_file_with_an_unterminated_legacy_block() {
        // The same fail-loud path on the OTHER file — the one whose branch deletes (review of
        // PR #413, Test Quality #4). A CLAUDE.md corrupted to an unterminated legacy block must
        // stop init rather than have the assembler guess where the region ends.
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "# shared\n").unwrap();
        std::fs::write(
            tmp.path().join("CLAUDE.md"),
            "# mine\n\n<!-- BEGIN NEXUS-FLOW v:1 -->\nbody, no end\n",
        )
        .unwrap();
        let err = assemble(tmp.path(), &[module("flow", "nxf")]).unwrap_err();
        assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Validation);
        assert!(
            read(&tmp.path().join("CLAUDE.md")).contains("# mine"),
            "and the file is left exactly as found — a loud stop, not a partial edit"
        );
    }

    #[test]
    fn a_marker_written_flush_left_inside_a_fence_is_still_a_marker() {
        // Pins the residual `line_start_find`'s doc names, on the branch that DELETES: a managed
        // block quoted flush left inside a fenced example is removed like any other, because
        // nothing in the text distinguishes it (review of PR #413, Test Quality #3).
        //
        // Recorded rather than fixed. The alternative — teaching the matcher about fences — would
        // put a Markdown parser on the path that edits a user's file, to serve a shape that has
        // never occurred, and it would still be wrong for a block quoted inside a fence in order to
        // BE managed. One space of indentation is the whole remedy, and now it is written down.
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "# shared\n").unwrap();
        let quoted = format!(
            "# mine\n\nHere is what the tool writes:\n\n```markdown\n{}\n```\n",
            nexus_discovery_block(&[module("flow", "nxf")])
        );
        std::fs::write(tmp.path().join("CLAUDE.md"), &quoted).unwrap();

        let report = assemble(tmp.path(), &[module("flow", "nxf")]).unwrap();
        assert_eq!(report.claude, ClaudeAction::BlockRemoved);
        let out = read(&tmp.path().join("CLAUDE.md"));
        assert!(
            !out.contains("<!-- BEGIN NEXUS"),
            "the quoted block went with the real ones: {out}"
        );
        assert!(
            out.contains("Here is what the tool writes:") && out.contains("```markdown"),
            "the prose around it stays, so what is left is a fence with a hole in it: {out}"
        );
    }

    #[test]
    fn wire_session_hook_refuses_a_malformed_settings_file() {
        // The shared settings-merge seam fails loud rather than clobbering a malformed
        // settings.json — the failing case lives WITH the merge logic (TQ#4), not only in the
        // caller that delegates to it.
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), "not json {").unwrap();
        let err = wire_session_hook(
            tmp.path(),
            &hook_commands(&["nxf"]),
            &managed_hook_commands(&["nxf"]),
            &["Bash(nxs:*)"],
        )
        .unwrap_err();
        assert_eq!(
            err.kind,
            nxs_foundation::error::ErrorKind::Validation,
            "a malformed settings.json is a validation error, never a clobber"
        );
    }

    #[test]
    fn a_hand_written_group_is_normalized_once_and_then_stable() {
        // A consequence of measuring the document rather than counting insertions (nxf 6j6v.shwz),
        // pinned so it is a decision and not a surprise: a SessionStart group somebody wrote by
        // hand without `"matcher"` carries the right command but not the right shape, so the first
        // run rewrites it and reports a change. The second reports none. Worth having either way —
        // the wiring is ours to keep canonical — but it must converge, not oscillate.
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            r#"{ "hooks": { "SessionStart": [ { "hooks": [ { "type": "command", "command": "nxf prime" } ] } ] } }"#,
        )
        .unwrap();

        let first = wire_session_hook(
            tmp.path(),
            &hook_commands(&["nxf"]),
            &managed_hook_commands(&["nxf"]),
            &[],
        )
        .unwrap();
        assert!(first.hook_added, "the shape is normalized on the first run");
        assert_eq!(wired_commands(&settings(tmp.path())), vec!["nxf prime"]);

        let second = wire_session_hook(
            tmp.path(),
            &hook_commands(&["nxf"]),
            &managed_hook_commands(&["nxf"]),
            &[],
        )
        .unwrap();
        assert!(!second.hook_added, "and never again");
    }

    #[test]
    fn wire_session_hook_adds_missing_perms_even_when_the_hook_is_already_present() {
        // Mixed state (review Test Quality low nit): the hook already exists but a permission is
        // still missing — the hook is NOT re-added (hook_added=false) yet the missing perm IS
        // appended. Pins the branch the deleted cli `setup` test used to cover, now at the shared
        // seam that owns the merge.
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            // Seeded in the form a flow-only workspace carries TODAY — bare, since nxf 6j6v.1k6y
            // moved the fallback onto memory's entry. Seeded with the tail it would be a STALE
            // command this run has to remove, which is a different test (`..._is_upgraded_...`).
            //
            // `"matcher": ""` is spelled out because since nxf 6j6v.shwz the run reports a change
            // whenever the document it would write differs from the one on disk — and a group
            // written by hand without `matcher` differs. That normalization is real and is pinned
            // by `a_hand_written_group_is_normalized_once_and_then_stable`; here it would only
            // obscure the mixed state this case is about.
            r#"{ "hooks": { "SessionStart": [ { "matcher": "", "hooks": [ { "type": "command", "command": "nxf prime" } ] } ] } }"#,
        )
        .unwrap();
        let report = wire_session_hook(
            tmp.path(),
            &hook_commands(&["nxf"]),
            &managed_hook_commands(&["nxf"]),
            &["Bash(nxs:*)"],
        )
        .unwrap();
        assert!(
            !report.hook_added,
            "the existing hook is detected by its command, not re-added"
        );
        assert_eq!(
            report.permissions_added,
            vec!["Bash(nxs:*)".to_string()],
            "the missing permission is still appended"
        );
    }

    #[test]
    fn wire_session_hook_is_idempotent_and_merge_only() {
        // The shared seam directly: first call wires the hook + perms; a second is a no-op.
        let tmp = TempDir::new().unwrap();
        let wire = |root: &Path| {
            wire_session_hook(
                root,
                &hook_commands(&["nxf", "nxm"]),
                &managed_hook_commands(&["nxf", "nxm"]),
                &["Bash(nxs:*)", "Bash(nxf:*)"],
            )
        };
        let first = wire(tmp.path()).unwrap();
        assert!(first.hook_added && first.permissions_added.len() == 2);
        let second = wire(tmp.path()).unwrap();
        assert!(
            !second.hook_added && second.permissions_added.is_empty(),
            "re-run is a no-op"
        );
        assert_eq!(
            wired_commands(&settings(tmp.path())),
            vec![
                "nxf prime".to_string(),
                format!("nxm prime{}", HOOK_FALLBACK)
            ]
        );
        assert_eq!(second.commands, first.commands, "and reports the same set");
    }
}
