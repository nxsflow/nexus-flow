//! `nxs prime` fan-out (spec §6.3, nexus-flow-aye.28) — the target of the ONE SessionStart hook.
//! It fans out to the `prime` verb of each **active** module, in the roster's own order (6j6v.xbnh:
//! the order is a safety property — see [`healed_active_modules`]), shelling out to
//! `<binary> prime` (TB-5) with ONE consistent `now` injected into every module (so a
//! deferred/overdue cutoff is identical across them). Only the active modules' results appear — a
//! flow-only workspace yields exactly flow's prime; memory shows up only after `nxm init`. `--json`
//! + text are both deterministic, in the stable registry order, so the hook output is byte-stable.

use crate::error::{NxfError, Result};
use crate::registry::{self, ModuleInit};
use crate::selfheal;
use crate::spawn;
use nexus_chat::facade::{ModulePrime as ChatModulePrime, ModulePrimes};
use nexus_chat::role::PrimeServices;
use nexus_chat::workspace::ChatWorkspaceExt;
use nxs_foundation::workspace::Workspace;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// One module's prime output (raw stdout — markdown in text mode, a JSON line under `--json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModulePrime {
    pub module: String,
    pub raw: String,
}

/// The fan-out result: the shared `now` every module saw, and each active module's prime output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimeFanOut {
    pub now: String,
    pub modules: Vec<ModulePrime>,
}

/// **The host's cut-off for a SessionStart hook's output, in bytes** (nxf 6j6v.xbnh).
///
/// It is a CLIFF, not a slope, and that is the whole reason this constant exists.
///
/// **Superseded 2026-08-28 (nxf hrz7, task 1).** The first measurement, taken 2026-08-27, was a
/// single probe against the host this suite ships for — two data points, one workspace, one run:
///
/// ```text
/// 25.893 B   passes through into the session's context unchanged   (single probe, superseded)
/// 32.000 B   is filed away as a file — the session gets a 2 KB preview and a path (superseded)
/// ```
///
/// It was right for what it was, and the 25 KiB it implied was wrong by a factor of 2.5: for weeks
/// every workspace this size or larger was silently filed away rather than delivered, because the
/// gate this number fed compared against a line that sat 2.5× too high to ever catch it.
///
/// **The reasoning behind 25 KiB is superseded along with the number, not dropped with it:** "25
/// KiB, under the largest measured pass-through with a little room, because the true edge lies
/// somewhere in the 6 KB between the two measurements and a ceiling that sits ON an unknown edge is
/// not a ceiling." That held against a two-point probe with a genuinely wide, unknown gap — backing
/// off from the observed watermark was the only honest response to not knowing where the edge sat.
///
/// The replacement is not a second probe but real telemetry — **1.114 hook events across 947
/// transcripts**, measured 2026-08-28:
///
/// ```text
/// 10.164 B   passes through into the session's context unchanged
/// 10.203 B   is filed away as a file — the session gets a 2 KB preview and a path
/// ```
///
/// A 39-byte band, from a sample two orders of magnitude larger than the one that produced the old
/// 6 KB of uncertainty — tight enough to say with confidence that the host's own constant is the
/// clean KiB boundary sitting just past it: 10 × 1024 = 10.240 B (10 KiB), which is what this
/// constant is now set to. The old paragraph's OWN logic — don't sit ON an unknown edge, back off
/// from it — is what argues against carrying its number forward unchanged: a 39-byte band from over
/// a thousand real events is not a genuinely unknown edge the way 6 KB was, so the constant is set
/// at the boundary the data points to rather than padded further below it. Padding here would not
/// buy safety against a real uncertainty; it would only spend more of a budget that is already this
/// tight.
///
/// Above the line the yield of `prime` is therefore not smaller but NIL, and worse than nil: the
/// session then reads the files by hand and burns more context than `prime` would ever have cost.
/// Below it, every byte arrives. So a budget calibrated against token COST — which is what
/// nxf 6j6v.waq9's 48 KiB `rules` budget was — can be met in full while the delivered result is
/// nothing at all. That happened: four workspaces sat above the cliff while every declared budget
/// was satisfied.
///
/// **It is a property of the HOST, not of nxs** — a different host has a different one — and it is
/// deliberately NOT configurable today: a knob is a setting nobody sets until a second host is
/// actually in play. `6j6v.hqq6` carries that question. The measurement that used to make this
/// number worth watching — this project's own workspace projected at 27.933 bytes against a 25 KiB
/// line — is itself superseded now that the line has moved: at real scale (55 memories, a live
/// board, a chat history — see `crates/nxs/tests/the_session_start_ceiling.rs`) the composed
/// fan-out measured roughly 24.469 B **as of task 1, unchanged by that task** (task 1 corrected
/// only this constant, not what `prime` renders), which was already well past the corrected
/// 10.240 B line — and even a freshly initialized workspace with nothing recorded measured
/// 10.747 B, past the line before a single board item, memory or message existed. **Superseded
/// 2026-08-28 (nxf hrz7, task 2):** task 2 reshaped `nxf prime`'s human view (its OWN doc comment,
/// `crates/cli/src/commands/mod.rs::prime`'s `else` branch, carries the full accounting), cutting
/// its fixed prose from 5.570 B to 1.635 B. The composed real-scale fan-out dropped to **20.534 B**
/// and the freshly initialized workspace to **6.310 B** (both re-measured directly, in the
/// composed size band `crates/nxs/tests/the_session_start_ceiling.rs` carried at the time and in
/// its `quiet` fixture) — the real-scale figure is STILL well past this ceiling (task 6, not
/// task 2, is what closes that gap), but the freshly initialized one is now UNDER it. Shrinking
/// what `prime` renders further is still not this constant's own job — see nxf hrz7's later tasks.
///
/// **Superseded 2026-08-28 (nxf n2m6 + a2a1 / qhgw): the paragraph below described what enforced
/// this constant, and BOTH halves of its reasoning have moved.** It read:
///
/// > Two things enforce it. `ceiling_warning` weighs the composed output at run time and says so
/// > before it crosses; `crates/nxs/tests/the_session_start_ceiling.rs` holds the code against one
/// > real-scale workspace at build time. Both measure the COMPOSED fan-out — the thing the host
/// > actually truncates — rather than any single module's half, which is the mistake nxf
/// > 6j6v.waq9's per-module budget made.
///
/// **That was correct, and its warning about 6j6v.waq9 must not be read as retracted.** What made
/// the old per-module budget wrong was measuring ONE module against a limit that applied to all
/// three together; a part weighed against a whole's budget is green exactly when it should be red.
///
/// What changed is the limit, not the logic. The 2026-08-28 telemetry established that the host
/// truncates **per hook output**, and nxf n2m6 + a2a1 then wired one SessionStart hook per active
/// module — so each module's block is now, by itself, one host output with one budget of its own.
/// Measuring a module alone is therefore no longer measuring a part against a whole; it is
/// measuring a whole against its own whole budget. Same arithmetic, opposite meaning, because the
/// thing being truncated moved.
///
/// So, today: the GATE measures each module's hook output separately
/// (`each_modules_hook_output_stays_under_the_hosts_cut_off`), which is what the host truncates.
/// [`ceiling_warning`] still weighs the COMPOSED output and is unchanged — after the split that is
/// the size of the verb a person runs by hand, not of any host output; its own doc comment carries
/// that supersession and names the follow-up gap (no module's `prime` weighs itself).
pub const SESSION_START_CEILING_BYTES: usize = 10 * 1024;

/// **The fraction of [`SESSION_START_CEILING_BYTES`] at which the composed block warns about its
/// own size** (nxf 6j6v.xbnh, review of PR #381, Integrity #2; re-examined nxf hrz7 task 1 when
/// the ceiling itself was corrected, and again task 2 when the floor it argued from moved).
///
/// 90 %, and the number is chosen against the one thing that matters: **the warning has to arrive.**
/// Above the cliff the host files the whole block away and delivers a preview instead, so a warning
/// emitted there is a warning nobody reads. Below it, every byte arrives — which is exactly the
/// window this threshold sits in. It fires while there is still room to act and stops firing once
/// acting is no longer possible, which is the opposite of most size alarms and is the point. This
/// paragraph is the one that keeps standing below — it never depended on any one fixture's byte
/// count, only on where the window is relative to the ceiling.
///
/// **Superseded 2026-08-28 (nxf hrz7, task 2).** The two paragraphs this replaces argued that,
/// right after task 1 corrected the ceiling, the fraction had stopped mattering: `nxf prime`'s
/// fixed prose alone put even a freshly initialized workspace at 10.747 B, already past the
/// corrected 10.240 B ceiling OUTRIGHT — so "no fraction this side of infinity keeps the warning
/// quiet on a bare workspace," and 90 % was kept only because it was not itself the broken part.
/// That argument's premise is gone: task 2 shrank `nxf prime`'s fixed prose from 5.570 B to
/// 1.635 B, and the same freshly initialized fixture now measures **6.310 B** — genuinely UNDER
/// 90 % of the ceiling (9.216 B), re-measured directly (see
/// `crates/nxs/tests/the_session_start_ceiling.rs`'s `quiet` case). The fraction is therefore
/// doing real, fixture-sensitive work again for the first time since the ceiling was corrected: a
/// bare workspace now stays quiet (as it should — nothing to act on yet) while a workspace at real
/// scale still warns (20.534 B, still far past the ceiling; closing THAT gap is task 6's job, not
/// this constant's). Kept at 90 % — the value the opening paragraph above justifies on its own
/// terms, independent of either fixture's size — because nothing in task 2's brief asked it to
/// change and the premise for reconsidering it was never sound to begin with.
const CEILING_WARNING_FRACTION: f64 = 0.9;

/// The line the composed fan-out appends when it is close to the host's cut-off — or nothing at all
/// when it is not (nxf 6j6v.xbnh).
///
/// **This is the live half of a ceiling that was otherwise only a CI gate.** nxf 6j6v.waq9's
/// per-write budget was removed with the full-text channel it governed, and what replaced it —
/// `crates/nxs/tests/the_session_start_ceiling.rs` — measures one synthetic workspace at build
/// time. A real workspace grows after that, so the failure this whole item exists to end could
/// quietly return with nothing live to notice. The review of PR #381 named that gap; this closes it.
///
/// **Superseded 2026-08-28 (nxf hrz7, task 1):** this used to say the project's own workspace sat at
/// roughly 6 % headroom in the gate's worst case — true against the 25 KiB ceiling in place when it
/// was written. With that ceiling corrected to its true 10 KiB, the same worst-case workspace is no
/// longer near the line, it is roughly 2.4× past it (~24.5 KB composed against a 10.240 B ceiling).
/// This function still fires correctly on that workspace — see this file's `CEILING_WARNING_FRACTION`
/// doc comment, a few lines above, for why the floor alone is now most of the budget — but
/// "headroom" is no longer the right word for what a real workspace has left; closing the gap
/// between what `prime` renders and what the corrected ceiling allows is nxf hrz7's later tasks,
/// not this one.
///
/// It is a WARNING and not a refusal on purpose. There is no single write to refuse — the size is a
/// property of three modules' output together, and the only honest actor is the person reading it.
/// Naming the measurement, the ceiling and the one verb that shrinks the block is what a reader can
/// act on; failing the session start would take the bootstrap away from exactly the workspace that
/// still needs it.
///
/// The line costs ~200 bytes, and only in the window where it fires — which is the trade it is
/// making: a little of the remaining margin, spent on being told the margin is nearly gone.
///
/// **Superseded 2026-08-28 (nxf n2m6 + a2a1 / qhgw): what this measures is no longer a host
/// output.** Everything above was written while the SessionStart hook was `nxs prime`, so the
/// composed size this weighs and the size the host truncates were the same number. Since the hook
/// became one entry per active module, the host receives three separate outputs and truncates each
/// at [`SESSION_START_CEILING_BYTES`] on its own; the composed size is what a person gets from
/// running `nxs prime` BY HAND. The warning is still worth printing for exactly that reader — a
/// composed block past 10.240 B is a workspace whose modules are collectively large, and the verbs
/// it names still take bytes back — but it is no longer the session-start tripwire it was, because
/// a session start is now three smaller things.
///
/// **The gap this leaves, named rather than quietly carried:** no module's own `prime` weighs
/// itself, so nothing live warns the workspace whose ONE module's block crosses the line. Giving
/// each module's `prime` its own tripwire (or moving this one behind the fan-out so it weighs each
/// block) is the honest follow-up; it belongs to no task on this branch, and the gate test's own
/// doc comment records the same gap from the other side.
///
/// **Narrowed 2026-08-29 (nxf 6j6v.5jm3), not closed.** The workspace this paragraph was written
/// about was a concrete one: `nxm prime`, whose index grew a line per memory with nothing bounding
/// it, and which the gate measured at 13.528 B for a 55-memory fixture. Memory now bounds its own
/// block (`nexus_memory::facade::PRIME_BLOCK_BUDGET_BYTES`) — a self-imposed budget rather than a
/// live warning, which is the stronger answer for that module and needs no tripwire at all. The GAP
/// above survives it: flow and chat still render whatever their workspace holds, and nothing weighs
/// them at run time.
pub fn ceiling_warning(composed_bytes: usize) -> Option<String> {
    let threshold = (SESSION_START_CEILING_BYTES as f64 * CEILING_WARNING_FRACTION) as usize;
    (composed_bytes > threshold).then(|| {
        format!(
            "> **This session start is {composed_bytes} bytes and the host stops delivering one \
             at about {SESSION_START_CEILING_BYTES}.** Past that it hands the session a short \
             preview and a file path instead of the board, these rules and the memory index — so \
             shrink it before it crosses: `nxm memories` lists what is replayed, and \
             `nxm forget <key>` or a shorter `nxm classify <key> --introduction \"…\"` is what \
             takes bytes back."
        )
    })
}

/// The shared reference time for the fan-out: a pinned `NXS_NOW` (determinism knob, symmetric with
/// `NXF_NOW`/`NXM_NOW`), else the system clock as RFC3339. The ONE `now` injected into every module
/// **that `nxs prime` runs**.
///
/// **Superseded 2026-08-28 (nxf qvw3): the promise is WITHDRAWN for the SessionStart hooks, and
/// kept — unchanged and still tested — for `nxs prime`.** This used to read "The ONE `now` injected
/// into every module" without qualification, and while the host ran a single hook → `nxs prime`,
/// those were the same statement: every block a session was handed came from this one process, so
/// every block carried this one instant. Since nxf n2m6 + a2a1 the host runs one hook per active
/// module (`nxf prime`, `nxm prime`, `nxc prime` — see
/// `crates/nxs-init/src/assembler.rs`'s module doc for the per-hook-output measurement behind
/// that), and three hooks are three PROCESSES. Nothing in this function runs in two of them.
///
/// **Why it is not reinstated.** Sharing one instant across three independent host-launched
/// processes needs somewhere to put it that all three read, and the only candidates are the
/// workspace (a file — state written on every session start, for a value nothing reads twice) or
/// the environment (which the host owns; a hook cannot export into its siblings). The task that
/// raised this (nxf qvw3) allowed either outcome and explicitly ruled out writing workspace state
/// to buy it.
///
/// **What the divergence costs, stated rather than waved away.** The three hooks run within one
/// session start, so their instants differ by however long the host takes between them. Exactly ONE
/// rendered block consumes `now` at all, and not the way this comment used to claim: `nxf prime`'s,
/// where it gates deferred CANDIDACY — the `i.defer_until > ?1` bound in `derive::next_candidates`,
/// an explicit user-set deadline deciding which items are eligible at all. The ranking step
/// (`order_next_tiered`) takes no `now` and orders by tier + id, so nothing here is age-derived.
/// `nxc`'s block does not read it either: `PrimeReport::render_markdown` stopped reading `wake`
/// (nxf 6j6v.1gm9), the only field `now` feeds. Nothing renders a sub-second field, so a skew of
/// that size changes no byte unless two hooks fall on opposite sides of a `defer_until` deadline.
/// That is a real if rare case — a session started across midnight UTC could admit an item to one
/// block's candidate set and not to another's — and it is accepted rather than hidden: the blocks
/// are independent by design (no block may refer to another, since hook output order is not
/// guaranteed), so no reader is comparing a timestamp in one against a timestamp in another.
///
/// **Pinning still works, per module.** A test or a golden that needs all three blocks on one
/// instant sets `NXF_NOW`/`NXM_NOW`/`NXC_NOW` — exactly what this function's value is injected AS
/// when `nxs prime` fans out. `crates/nxs/tests/the_session_start_ceiling.rs` pins `NXF_NOW` and
/// `NXM_NOW` for every fixture it builds and leaves `NXC_NOW` unset, because per the paragraph
/// above `nxc`'s block does not read `now` — which is why that gate is byte-reproducible anyway.
pub fn resolve_now() -> Result<String> {
    match std::env::var("NXS_NOW").ok().filter(|s| !s.is_empty()) {
        Some(s) => Ok(s),
        None => OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|e| NxfError::io(format!("formatting current time: {e}"))),
    }
}

/// Fan `prime` out over the workspace's active modules with one shared `now`, shelling out to each
/// module's binary against the SAME db (so cwd never matters). Production path — see [`fan_out_with`]
/// for the testable core.
pub fn fan_out(ws: &Workspace, json: bool) -> Result<PrimeFanOut> {
    let now = resolve_now()?;
    let roster = registry::roster();
    let modules = healed_active_modules(&roster, ws)?;
    let db_path = ws.db_path_str()?;
    let modules = fan_out_with(&modules, &now, json, |m, now, json| {
        let mut args: Vec<String> = vec!["prime".to_string()];
        if json {
            args.push("--json".to_string());
        }
        // Pin the child to the shared db so the fan-out is independent of cwd.
        args.push("--db".to_string());
        args.push(db_path.clone());
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        spawn::run_capture_env(m.binary, &argv, &[(m.now_env, now)])
    })?;
    Ok(PrimeFanOut { now, modules })
}

/// The workspace's active modules to fan out over, after self-healing the active-module registry
/// (nexus-flow-cur): an initialized-but-unregistered module (a pre-umbrella flow workspace whose
/// `[flow]` section predates `active_modules`) is back-filled here so prime never silently drops
/// it — no explicit `nxs migrate` required. Idempotent (a healthy workspace heals to itself, no
/// write); the back-fill only APPENDS, so an already-active sibling (memory after `nxm init`) is
/// preserved. Extracted from [`fan_out`] so the heal + resolution is unit-testable without
/// shelling out to the module binaries.
///
/// **The result is sorted into ROSTER order, not `active_modules` order** (6j6v.xbnh). The order a
/// workspace happens to list its modules in is an accident of which `init` ran first; the fan-out
/// order is a safety property — the host truncates a large SessionStart output, so whatever comes
/// last is what a truncation takes. That must be the same everywhere, and it must be the block a
/// session can fetch afterwards (memory, via `nxm recall`) rather than the board or the command
/// vocabulary, which are written down nowhere else. [`registry::resolve_active`] keeps answering in
/// `active_modules` order — its loud unknown-module error is what it is for — and the sort is
/// applied here, at the one place the order carries that meaning.
fn healed_active_modules<'a>(
    roster: &[&'a ModuleInit],
    ws: &Workspace,
) -> Result<Vec<&'a ModuleInit>> {
    let registered = selfheal::backfill_active_modules(roster, ws)?;
    // The back-fill persisted the new keys to config.toml; mirror them onto the in-memory config
    // (appended in roster order, matching the on-disk write) so the fan-out sees the healed set
    // without a re-resolve (robust for the `--db` path, which never re-discovers).
    let mut config = ws.config.clone();
    config.active_modules.extend(registered);
    Ok(registry::sort_roster(registry::resolve_active(
        roster, &config,
    )?))
}

/// [`fan_out`] with the per-module runner injected — the testable core. `run(module, now, json)`
/// returns that module's prime stdout; results are collected in the given order.
pub fn fan_out_with<F>(
    modules: &[&ModuleInit],
    now: &str,
    json: bool,
    run: F,
) -> Result<Vec<ModulePrime>>
where
    F: Fn(&ModuleInit, &str, bool) -> Result<String>,
{
    modules
        .iter()
        .copied()
        .map(|m| {
            Ok(ModulePrime {
                module: m.key.to_string(),
                raw: run(m, now, json)?,
            })
        })
        .collect()
}

// ---- the persona's composed block (nxf 6j6v.k8zq) --------------------------

/// **What a declared persona is handed at its session start** — the composed
/// `nxs prime --persona <handle>` text.
///
/// The composition itself lives in `crates/chat`
/// ([`nexus_chat::facade::compose_persona_prime`]), and that is the layering decision this item
/// made deliberately: the crate that owns the persona owns what a persona is told, while THIS crate
/// — the one that knows the module registry — answers only "what does `nxf prime` say, what does
/// `nxm prime` say". Neither knows the other's stores, and neither reassembles the other's text.
///
/// The persona's own `prime:` filter is read from its declaration and decides which siblings are
/// asked at all: a module a persona excludes is never even run, because a filter is a statement
/// about what reaches the session and paying for a block that is about to be discarded is the kind
/// of waste the two neighbouring items (nxf 6j6v.4mmk, nxf 6j6v.waq9) exist to end.
///
/// A `prime: false` persona composes to the empty string — it declared that its own `system_prompt`
/// covers this ground. The forced ending is composed separately and unconditionally
/// ([`nexus_chat::role::compose_system_prompt`]), so it survives that regardless.
pub fn persona_block(ws: &Workspace, handle: &str) -> Result<String> {
    let root = ws.workspace_root()?.to_path_buf();
    let source = nexus_chat::definitions::DeclarationSource::resolve(&root)?;
    let defs = nexus_chat::definitions::Definitions::resolve(&root)?;
    let decl = defs.role(handle)?;
    if !decl.prime.is_primed() {
        return Ok(String::new());
    }
    let services = decl.prime.services();
    let db_path = ws.db_path_str()?;
    let siblings = SiblingPrimes { ws }.module_primes(services, &db_path)?;
    let consumer = format!("{}/{handle}", nexus_chat::workspace::origin_of(ws));
    let store = ws.open_chat_store()?;
    nexus_chat::facade::compose_persona_prime(
        &store,
        &consumer,
        handle,
        services,
        &siblings,
        &resolve_now()?,
        Some(&source),
    )
}

/// **The provider the `nxc` binary hands `crates/chat` at its entry** (nxf 6j6v.k8zq) — a
/// workspace-less [`ModulePrimes`] that resolves the workspace per call, because the CLI resolves
/// its own workspace per verb and a provider built at `main` would pin the wrong one for a
/// `--db`/discovery that happens later.
///
/// This is the composition root's whole contribution: `crates/chat` owns what a persona is told and
/// cannot know the module registry, so the umbrella — which is the binary either way — supplies the
/// siblings' text and nothing else. See [`nexus_chat::facade::ModulePrimes`] for the argument.
#[derive(Debug)]
pub struct RegistrySiblingPrimes;

/// The one instance, `'static` because [`nexus_chat::run_from_with`] takes a reference that outlives
/// the process's CLI run — which it does: there is nothing in it.
pub static SIBLING_PRIMES: RegistrySiblingPrimes = RegistrySiblingPrimes;

impl ModulePrimes for RegistrySiblingPrimes {
    /// **`db_path` decides the workspace, not the process working directory** (review of PR #379,
    /// Code Quality #1).
    ///
    /// This resolved `Workspace::resolve(None, cwd)` in its first cut, which made the persona spawn
    /// the one verb in this CLI that ignored `--db`/`NXC_DB`: a `nxc send --db <workspace-A> --to
    /// <persona>` issued from a cwd inside workspace B put B's board and B's memories into a
    /// persona that then ran against A. The caller has the resolved answer already and hands it
    /// over; the cwd is still what `resolve` walks up from, which is what makes a `--db` naming a
    /// db outside any workspace fail the way it does everywhere else rather than silently here.
    fn module_primes(
        &self,
        services: PrimeServices,
        db_path: &str,
    ) -> Result<Vec<ChatModulePrime>> {
        let ws = Workspace::resolve(
            Some(db_path),
            &std::env::current_dir()
                .map_err(|e| NxfError::io(format!("resolving the current directory: {e}")))?,
        )?;
        SiblingPrimes { ws: &ws }.module_primes(services, db_path)
    }
}

/// The suite's sibling `prime` blocks for one workspace, obtained the way everything else in this
/// module obtains them: by running `<binary> prime` over the SAME db, with the shared `now`
/// injected (nxf 6j6v.k8zq).
///
/// Deliberately not the whole [`fan_out`]: chat's own block is composed in-process from the
/// persona's declaration, so shelling out to `nxc prime --persona` as well would render it twice
/// and pay for it twice.
struct SiblingPrimes<'a> {
    ws: &'a Workspace,
}

impl<'a> SiblingPrimes<'a> {
    /// Whether this module contributes at all, per the persona's declared filter. An unknown module
    /// — one a newer peer registered — contributes: a filter that silently dropped what it does not
    /// recognise would take a block away on the strength of not knowing about it.
    fn admits(services: PrimeServices, module: &str) -> bool {
        match module {
            "flow" => services.flow,
            "memory" => services.memory,
            // chat's block is composed in-process, never fanned out to here.
            "chat" => false,
            _ => true,
        }
    }
}

impl<'a> ModulePrimes for SiblingPrimes<'a> {
    /// `db_path` is ignored here, and only here: this provider was handed the RESOLVED workspace by
    /// its constructor, which is the same answer the parameter carries. Re-resolving from it would
    /// be a second path to one value — the shape the trait's own doc argues against.
    fn module_primes(
        &self,
        services: PrimeServices,
        _db_path: &str,
    ) -> Result<Vec<ChatModulePrime>> {
        let now = resolve_now()?;
        let roster = registry::roster();
        let modules = healed_active_modules(&roster, self.ws)?;
        let db_path = self.ws.db_path_str()?;
        let mut out = Vec::new();
        for m in modules {
            if !Self::admits(services, m.key) {
                continue;
            }
            let argv = ["prime", "--db", db_path.as_str()];
            out.push(ChatModulePrime {
                module: m.key.to_string(),
                text: spawn::run_capture_env(m.binary, &argv[..], &[(m.now_env, &now)])?,
            });
        }
        Ok(out)
    }
}

impl std::fmt::Debug for SiblingPrimes<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SiblingPrimes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nxs_foundation::workspace::{self, WorkspaceConfig};
    use tempfile::TempDir;

    fn noop(_: &registry::InitRequest) -> Result<()> {
        Ok(())
    }

    /// A test roster descriptor (the fan-out reads `.key`/`.binary`/`.now_env`).
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

    fn cfg(modules: &[&str]) -> WorkspaceConfig {
        WorkspaceConfig {
            active_modules: modules.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    /// A pre-umbrella flow workspace on disk: a `[flow]` product section plus whatever `active`
    /// modules were already registered (empty mimics a v0.5.x `nxf init`; `["memory"]` mimics a
    /// later `nxm init` that left flow unregistered).
    fn flow_workspace(active: &[&str]) -> (TempDir, Workspace) {
        let mut flow = toml::value::Table::new();
        flow.insert("plugin".into(), toml::Value::String("issue-tracker".into()));
        let mut products = toml::value::Table::new();
        products.insert("flow".into(), toml::Value::Table(flow));
        let cfg = WorkspaceConfig {
            active_modules: active.iter().map(|s| s.to_string()).collect(),
            products,
        };
        let tmp = TempDir::new().unwrap();
        let ws = workspace::setup(tmp.path(), &cfg).unwrap();
        (tmp, ws)
    }

    #[test]
    fn the_fan_out_still_injects_one_shared_now_into_every_module() {
        // **The half of the `now` promise that SURVIVED the hook split (nxf qvw3).** `nxs prime` is
        // one process fanning out to every module, so every block it composes carries the same
        // instant — unchanged by nxf n2m6 + a2a1, and pinned here so a later refactor that resolved
        // the clock per module inside the fan-out would redden rather than pass quietly.
        //
        // The OTHER half is withdrawn and deliberately has no test: the three SessionStart hooks
        // are three processes, each resolving its own `now`, and there is no seam in this crate at
        // which their instants could be compared. See `resolve_now`'s doc comment.
        let (flow, memory) = (module("flow", "nxf", 10), module("memory", "nxm", 30));
        let roster = vec![&flow, &memory];
        let seen = std::sync::Mutex::new(Vec::new());
        let primes = fan_out_with(&roster, "2026-08-28T12:00:00Z", false, |m, now, _| {
            seen.lock().unwrap().push((m.key, now.to_string()));
            Ok(String::new())
        })
        .unwrap();
        assert_eq!(primes.len(), 2);
        assert_eq!(
            seen.into_inner().unwrap(),
            vec![
                ("flow", "2026-08-28T12:00:00Z".to_string()),
                ("memory", "2026-08-28T12:00:00Z".to_string()),
            ],
            "every module is handed the SAME now, not one resolved per module"
        );
    }

    #[test]
    fn healed_active_modules_backfills_a_pre_umbrella_flow_workspace() {
        // nexus-flow-cur acceptance: a `[flow]` section with empty active_modules fans out flow
        // after resolving — without the user first running `nxs migrate`.
        let (_t, ws) = flow_workspace(&[]);
        let (flow, memory) = (module("flow", "nxf", 10), module("memory", "nxm", 20));
        let roster = vec![&flow, &memory];
        let keys: Vec<&str> = healed_active_modules(&roster, &ws)
            .unwrap()
            .iter()
            .map(|m| m.key)
            .collect();
        assert_eq!(keys, vec!["flow"]);
    }

    #[test]
    fn healed_active_modules_returns_both_when_memory_active_and_flow_initialized() {
        // nexus-flow-cur acceptance: after `nxm init` over a pre-umbrella flow workspace, prime
        // fans out BOTH modules — neither displaces the other.
        //
        // **In ROSTER order, not in the workspace's own** (6j6v.xbnh): this workspace lists
        // `["memory"]` and heals flow in behind it, and the fan-out still answers flow first. The
        // order is a safety property (what a truncation is allowed to eat), so it cannot depend on
        // which `init` a particular workspace happened to run first.
        let (_t, ws) = flow_workspace(&["memory"]);
        let (flow, memory) = (module("flow", "nxf", 10), module("memory", "nxm", 30));
        let roster = vec![&flow, &memory];
        let keys: Vec<&str> = healed_active_modules(&roster, &ws)
            .unwrap()
            .iter()
            .map(|m| m.key)
            .collect();
        assert_eq!(keys, vec!["flow", "memory"]);
    }

    #[test]
    fn fan_out_threads_one_now_in_active_order_to_each_module() {
        let (flow, memory) = (module("flow", "nxf", 10), module("memory", "nxm", 20));
        let roster = vec![&flow, &memory];
        let modules = registry::resolve_active(&roster, &cfg(&["flow", "memory"])).unwrap();
        // The stub records the now it was handed and echoes a per-module marker.
        let out = fan_out_with(&modules, "2026-06-22T00:00:00Z", false, |m, now, _json| {
            assert_eq!(
                now, "2026-06-22T00:00:00Z",
                "every module sees the SAME now"
            );
            Ok(format!("{} primed @ {now}", m.binary))
        })
        .unwrap();
        assert_eq!(
            out.iter().map(|m| m.module.as_str()).collect::<Vec<_>>(),
            vec!["flow", "memory"],
            "results in active-module (fan-out) order"
        );
        assert_eq!(out[0].raw, "nxf primed @ 2026-06-22T00:00:00Z");
        assert_eq!(out[1].raw, "nxm primed @ 2026-06-22T00:00:00Z");
    }

    #[test]
    fn fan_out_over_a_flow_only_workspace_yields_only_flow() {
        let flow = module("flow", "nxf", 10);
        let roster = vec![&flow];
        let modules = registry::resolve_active(&roster, &cfg(&["flow"])).unwrap();
        let out = fan_out_with(&modules, "t", false, |m, _n, _j| Ok(m.key.to_string())).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].module, "flow",
            "memory absent until nxm init activates it"
        );
    }

    #[test]
    fn fan_out_propagates_json_choice_to_each_module() {
        let flow = module("flow", "nxf", 10);
        let roster = vec![&flow];
        let modules = registry::resolve_active(&roster, &cfg(&["flow"])).unwrap();
        let out = fan_out_with(&modules, "t", true, |_m, _n, json| {
            Ok(if json {
                "{\"ok\":true}".into()
            } else {
                "text".into()
            })
        })
        .unwrap();
        assert_eq!(out[0].raw, "{\"ok\":true}");
    }

    #[test]
    fn fan_out_surfaces_a_failing_module() {
        let (flow, memory) = (module("flow", "nxf", 10), module("memory", "nxm", 20));
        let roster = vec![&flow, &memory];
        let modules = registry::resolve_active(&roster, &cfg(&["flow", "memory"])).unwrap();
        let err = fan_out_with(&modules, "t", false, |m, _n, _j| {
            if m.key == "memory" {
                Err(NxfError::io("nxm prime blew up"))
            } else {
                Ok("ok".into())
            }
        })
        .unwrap_err();
        assert!(err.msg.contains("nxm prime blew up"));
    }

    #[test]
    fn resolve_now_honors_the_pin() {
        // The determinism knob: NXS_NOW pins the shared now. (Env mutation is process-global, so
        // this test owns the var for its duration.)
        std::env::set_var("NXS_NOW", "2026-01-02T03:04:05Z");
        assert_eq!(resolve_now().unwrap(), "2026-01-02T03:04:05Z");
        std::env::remove_var("NXS_NOW");
    }

    /// **The corrected ceiling, pinned directly** (nxf hrz7, task 1). `SESSION_START_CEILING_BYTES`
    /// is 10.240 B, not the 25 KiB it used to be; this fixes it at the unit level, right next to the
    /// function, rather than relying only on the heavier real-workspace suite in
    /// `crates/nxs/tests/the_session_start_ceiling.rs` to notice a regression.
    #[test]
    fn ceiling_warning_fires_past_the_corrected_threshold_and_stays_quiet_below_it() {
        assert_eq!(SESSION_START_CEILING_BYTES, 10 * 1024);
        // 90 % of 10.240 B is 9.216 B: 9.000 sits below it, 10.300 sits past the ceiling itself —
        // both sides of the DoD this task was given.
        assert!(
            ceiling_warning(9_000).is_none(),
            "9.000 bytes is comfortably below the warning threshold"
        );
        let warning = ceiling_warning(10_300).expect("10.300 bytes is past the ceiling and warns");
        assert!(
            warning.contains("10240"),
            "the warning names the corrected ceiling: {warning}"
        );
    }
}
