//! nexus-chat's long-lived embedding handle (be9y) — the mirror of memory's `engine::Engine` /
//! flow's `nexus_flow_facade::engine::Engine`, specialized to [`ChatStore`].
//!
//! Where the `nxc` CLI opens the workspace fresh per invocation, an embedding app (a Tauri backend,
//! via app-foundations) holds ONE [`Engine`] for its whole lifetime and reads/writes through it many
//! times. The handle owns the resolved workspace + chat's store behind a `Mutex` (the store wraps a
//! non-`Sync` `rusqlite::Connection`, so the mutex is what makes the handle `Sync` and safe in async
//! runtime state). It is `Clone` (a cheap `Arc` bump) so it can be shared across tasks.
//!
//! The lifecycle (lock + repoint-on-swap + `PRAGMA data_version` change watch) is the
//! product-agnostic foundation [`Engine`](nxs_foundation::engine::Engine); this layer supplies
//! chat's store factory and exposes the [`crate::facade`] verbs over it. There is NO new semantics:
//! the same `message` reducer, the same views/derivations, the same rejection behavior as the `nxc`
//! CLI (the seam invariant, proven by the `tests/parity.rs` differential).
//!
//! # What the handle offers, since nxf 6j6v.yr59
//!
//! **SEVEN read verbs and THREE writing ones**, and each count is a decision rather than an
//! outcome. Sixteen reads stood here; six of the seven that stayed answer a question none of the others
//! answers — [`status`](Engine::status) (where the operation stands), [`directory`](Engine::directory)
//! (whom and WHAT may be addressed, and what for), [`prime_as`](Engine::prime_as) (session start),
//! [`thread`](Engine::thread) (what was said), [`transcript_page`](Engine::transcript_page) (what a
//! session did) and [`subscribe`](Engine::subscribe) (so a view stays alive).
//!
//! **The seventh is [`search`](Engine::search), and it is here by an OWNER CORRECTION of 2026-08-21
//! rather than by that argument.** yr59's text covers `search` nowhere — not the keep table, not the
//! "what goes and why" list, not the five verbs its DoD sends to be decided one by one — so this
//! build decided it in the item's own terms, removed it as a named loss, and reported the gap. The
//! owner overruled the removal on the fact that outweighs the shape: app-foundations' `ChatClient`
//! (`packages/engine-client/src/chat-client.ts`) carries `search` TODAY — measured on 2026-08-21, it
//! is one of the 14 methods that still exist and are consumed, not one of the 14 that no longer do.
//! Removing a live consumed method inside the very item whose purpose is to cost that consumer ONE
//! migration instead of two is the break the item exists to prevent.
//!
//! The ten that went are recorded at the BOTTOM of this file, one by one, with what each collapsed
//! into or what it cost. `tests/read_surface.rs` is the gate that keeps the number seven.
//!
//! Beside them sit the writes. TWO of them are the ways an app may SPEAK, and both start or wake
//! the session on the other side: [`send_to`](Engine::send_to) and
//! [`reply_thread`](Engine::reply_thread) (nxf 6j6v.ckeq, which cut a surface of four down to
//! exactly these). The THIRD only takes something back: [`withdraw`](Engine::withdraw) (nxf
//! 6j6v.0djn) retracts a commission still parked behind the working-copy lease — it opens no
//! conversation, addresses no target and starts nothing, which is why it does not reopen ckeq's
//! cut. It exists because [`SendToReceipt::queue_position`](crate::surface::SendToReceipt::queue_position)
//! reports that parked state to an app, and a reported state with no call to answer it is the
//! class this house closes.
//!
//! Beyond the verbs, what the handle answers about ITSELF: its [`workspace`](Engine::workspace),
//! its [`origin`](Engine::origin), its [`definitions`](Engine::definitions), the runtime session
//! binding, and the transcript's own two levers — [`transcript_append`](Engine::transcript_append) (nxf 6j6v.c6e8) and retention. None of
//! those is a way to SPEAK; `tests/read_surface.rs` is the gate that keeps that line drawn.
//!
//! Since nxf 6j6v.d49b the handle also carries the **role runtime** (spec
//! `docs/specs/E5c-chat-orchestration-api.md` §4): the orchestration verbs sit FLAT on [`Engine`]
//! beside the reads, so an app reaches the whole engine through one handle. They are
//! the very same [`crate::orchestration`] bodies `nxc` runs — this layer is only the second
//! ADAPTER, filling an [`orchestration::Ctx`] where `cli.rs` fills one from `NXC_*` and the
//! filesystem. What it fills it FROM is the caller's [`Caller`] — who is calling — plus what the
//! handle can answer for itself: its workspace's declarations, its db path, its `origin`, the
//! persona behind the caller's session, and the clock (nxf 6j6v.07me).

use crate::channel::ChannelPolicy;
use crate::definitions::Definitions;
use crate::error::Result;
use crate::facade::{
    self, MessageHitView, PrimeReport, StatusReport, StatusScope, ThreadView, TranscriptView,
};
use crate::naming::{Namer, NamerConfig};
use crate::orchestration::{self, Ambient, Caller, Ctx, ReplyReceipt};
use crate::store::ChatStore;
use crate::surface::{self, ReplyThreadRequest, SendToReceipt, SendToRequest};
use crate::timer::{Timer, TimerConfig};
use crate::transcript::{TranscriptEntry, TranscriptPruneReport};
use crate::worker::{TriggerRequest, TriggerResult, Worker, WorkerConfig};
use crate::workspace::{ChatWorkspaceExt, Workspace};
use nxs_foundation::engine::{Engine as Handle, OpenStoreFn};
use nxs_foundation::watch::Change;
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Duration;

/// What an [`Engine`] is opened WITH (spec §3.2.2) — what turns the messaging handle into the full
/// role runtime. [`Default`] is exactly what [`Engine::open`] uses, so an existing embedder is
/// untouched and orchestration is a deliberate opt-in through [`Engine::open_with`].
///
/// **Declarations are NOT on this config, and that is the point** (nxf 6j6v.dvyq step 6). They come
/// from the workspace's own `.nxs-personas/` folder, always, and are re-read on every verb that
/// needs them — exactly as a fresh `nxc` process re-reads the folder per invocation. There is no
/// injection path any more: `DefinitionSource::Supplied` and `Engine::set_definitions` are gone,
/// because personas and channels live in the folder even for an app, and a later in-app role editor
/// writes into that same folder rather than into an app-owned database (owner decision, 2026-08-14,
/// obtained specifically for this removal; app-foundations named its call sites and released it).
///
/// The property the injection path was built for survives intact and now costs nothing to state:
/// an edit is visible to the NEXT verb on this handle and on every existing clone of it, with no
/// reopen, no torn-down `subscribe` receiver and no lost UI state — because nothing is cached.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Which worker spawns role sessions. [`WorkerConfig::Disabled`] (the default) means reads and
    /// messaging writes behave identically and any verb that WOULD spawn reports the refusal loudly
    /// — as a named `validation` error, or (since nxf 6j6v.hpv8, for
    /// `coordinator_commission`/`send_to`'s
    /// persona branch) as `spawned: false` on the receipt it still returns — see [`DisabledWorker`].
    pub worker: WorkerConfig,
    /// How one-shot re-checks are scheduled — a declared channel's `timeout` tick (PR #269
    /// review, Code Quality #3).
    ///
    /// [`TimerConfig::Disabled`] (the default) schedules nothing, and unlike a disabled worker that
    /// is not a refusal: the verb a job would have run (`nxc tick`) is idempotent and callable by
    /// hand, so declining the automation breaks nothing. It is the right
    /// default for an app, which has its own scheduler and never asked this library to shell out to
    /// `at` — in the host's own process, from inside this handle's locked section, for up to eight
    /// seconds — which is what an unset `NXC_TIMER` used to get it.
    pub timer: TimerConfig,
    /// **Whether a thread this handle opens is given a name** (nxf 6j6v.e76c).
    ///
    /// [`NamerConfig::Disabled`] (the default) names nothing, for [`timer`](Self::timer)'s reason
    /// and one step stronger: the shipped backend starts a detached process that asks a MODEL, and
    /// an app that opened a workspace to read and post must not start paying for model calls
    /// because it sent a message. A thread with no name is the state every thread was in before the
    /// field existed, and every surface renders it.
    ///
    /// An app that wants names either asks for [`NamerConfig::Model`] or names threads itself
    /// through [`Engine::name_thread`] — which is the better answer for a host that already has a
    /// model of its own, because it keeps the model call inside the app's own budget and telemetry.
    pub namer: NamerConfig,
    /// How often the change watcher polls for another writer's commit — what
    /// [`Engine::subscribe`]'s receiver hangs off.
    ///
    /// **A TEST SEAM, and it is a field rather than a third constructor** (nxf 6j6v.yr59).
    /// `Engine::open_with_poll_interval` stood beside [`open`](Engine::open)/
    /// [`open_with`](Engine::open_with) for this ONE value, which made three ways to open a handle
    /// where the config already existed to carry exactly this kind of choice — and a caller who
    /// wanted a worker AND a fast cadence could not have both. Production uses the default; a test
    /// that must observe a `Change` inside its own runtime shortens it.
    pub poll_interval: Duration,
    /// **Where the board's and the memory's session-start blocks come from** when this handle
    /// summons a persona (nxf 6j6v.k8zq) — see [`crate::facade::ModulePrimes`].
    ///
    /// `None` (the default) composes chat's own half alone: the persona learns who it is, whom it
    /// may address and how it answers, and is handed neither the board nor the project's memories.
    /// That is the right default for a handle opened only to read or to post, and it is the honest
    /// one for an app that has not decided yet — the alternative would be this library shelling out
    /// to `nxf` and `nxm` from inside the host's process, which is precisely what the layering
    /// argument on that trait refuses.
    ///
    /// app-foundations starts role sessions of its own (`crates/agent-runtime`), and this is the
    /// field that lets those sessions be given the SAME context an `nxc` spawn gives — the item's
    /// own requirement, because two contracts for one prompt is the split the verb-seam gate
    /// (6j6v.vtvs) exists against.
    pub module_primes: Option<Arc<dyn crate::facade::ModulePrimes>>,
    /// **Which machine this handle's host is, and who else is online** (nxf 6j6v.1c6k) — see
    /// [`crate::machine::Machines`].
    ///
    /// `None` (the default) names no machine: no chat this handle starts is designated, and a
    /// persona starts wherever this handle's worker runs, exactly as before the executing machine
    /// existed. A host that executes passes its machine and its claims; a host that only shows and
    /// writes (a web board) passes presence and no machine, and is then asked to name one.
    pub machines: Option<Arc<dyn crate::machine::Machines>>,
}

/// **Hand-written since nxf 6j6v.k8zq**, because [`EngineConfig::module_primes`] is a trait object
/// and two host implementations are not comparable by value. Two configs are equal when they
/// configure the same behaviour: the three settled fields match, and either both name no prime
/// source or both name the SAME one (`Arc::ptr_eq` — identity, which is the only honest answer for
/// somebody else's code).
impl PartialEq for EngineConfig {
    fn eq(&self, other: &EngineConfig) -> bool {
        self.worker == other.worker
            && self.timer == other.timer
            && self.namer == other.namer
            && self.poll_interval == other.poll_interval
            && match (&self.module_primes, &other.module_primes) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
            && match (&self.machines, &other.machines) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            worker: WorkerConfig::Disabled,
            timer: TimerConfig::Disabled,
            namer: NamerConfig::Disabled,
            poll_interval: nxs_foundation::watch::POLL_INTERVAL,
            module_primes: None,
            machines: None,
        }
    }
}

/// A long-lived in-process handle owning workspace + chat store for the app's lifetime.
/// `Clone` + `Send` + `Sync`: clone freely and share across async tasks. Reads are short and
/// lock-internal; nothing is held across `.await`.
#[derive(Clone)]
pub struct Engine {
    handle: Handle<ChatStore>,
    /// Built ONCE at open and held for the handle's lifetime — the long-lived worker seam nxf
    /// 6j6v.a5na made possible by moving the per-hop env stamps onto [`TriggerRequest::env`].
    worker: Arc<dyn Worker>,
    /// Likewise for the scheduling seam, and for the same reason it is a value at all: a library
    /// call must not reach for `NXC_TIMER` (PR #269 review, Code Quality #3).
    timer: Arc<dyn Timer>,
    /// And likewise for the naming seam (nxf 6j6v.e76c) — a value the app configures once, never
    /// something a verb resolves from the ambient process.
    namer: Arc<dyn Namer>,
    /// The host's sibling-prime source, held for the handle's lifetime exactly as the worker and
    /// the timer are, and for the same reason: it is a seam the app configures once
    /// ([`EngineConfig::module_primes`]), not something a verb resolves from the ambient process.
    module_primes: Option<Arc<dyn crate::facade::ModulePrimes>>,
    /// ([`EngineConfig::machines`]) — held for the handle's lifetime, like the prime source.
    machines: Option<Arc<dyn crate::machine::Machines>>,
}

/// chat's store factory for the foundation handle: open the chat store (message reducer + the six
/// views) over a resolved workspace, in WAL mode.
fn chat_store_factory() -> OpenStoreFn<ChatStore> {
    Arc::new(|ws: &Workspace| ws.open_chat_store())
}

/// The worker a [`WorkerConfig::Disabled`] engine gets: one that refuses every trigger with
/// [`crate::worker::disabled_error`]'s named `validation` error.
///
/// [`WorkerConfig::build`] cannot produce a worker for `Disabled`, and the two eager alternatives
/// are both wrong. Failing `open_with` would make a handle that only ever reads and posts messages
/// — the whole existing `Engine::open` audience — unopenable. Failing at `Ctx` construction would
/// make [`reply_thread`](Engine::reply_thread), the channel-completion `tick` and every other path
/// that needs no worker at all fail for a reason
/// that has nothing to do with them. So the refusal is deferred to the exact moment a spawn is
/// actually attempted, which is the mirror image of what `cli.rs::LazyWorker` does with the
/// opposite problem (it resolves the real worker on first trigger so a non-triggering CLI verb
/// never needs `NXC_SIDECAR`).
///
/// Consequences are deliberate and tested, and split into two shapes since nxf 6j6v.hpv8.
/// [`channel_open`](orchestration::channel_open) still returns the named error outright, because
/// nothing about it has changed — this refusal is exactly the "silently doing nothing behind a
/// successful receipt" it still refuses loudly.
/// [`orchestration::coordinator_commission`]/[`crate::surface::send_to`]'s persona branch instead
/// persist
/// their message and mint their session BEFORE any worker is asked to do anything (the same
/// ordering that made this refusal reach that far in the first place), so the SAME receipt a
/// working worker would return comes back, with `spawned: false` and this refusal named in
/// `warnings` — see [`orchestration::TriggerReceipt::warnings`]'s doc for why an unconfigured
/// worker is treated as no different a cause than a real spawn failure. And a reply still
/// succeeds — whether it completes a board or merely carries a return address to resume: every
/// wake in it is "skip, don't fail" by contract ([`orchestration::reply`]'s own doc), and because
/// the refusal surfaces from INSIDE `Worker::trigger` it lands in exactly the place that contract
/// is implemented — the message is written, the board is still routed, and the receipt reports the
/// skip honestly: `woke: None` plus a `wake_skipped` naming this refusal, on the direct 1:1 resume
/// (nxf 6j6v.0akf) as on both completion paths.
struct DisabledWorker;

impl Worker for DisabledWorker {
    fn trigger(&self, _req: TriggerRequest) -> TriggerResult {
        // `Failed`, deliberately not a new variant of its own: nothing vanished and nothing broke —
        // the caller simply never asked for orchestration, which is what the named `validation`
        // error already says. Keeping it in the general arm is what lets
        // [`crate::worker::TriggerError::SessionGone`] stay the one case a caller branches on.
        Err(crate::worker::disabled_error().into())
    }
}

/// The `visibility` a thread's channel DECLARES, for the one message reader (nxf 6j6v.yr59).
///
/// This resolution used to sit inside `Engine::thread_board` and moved here with the reader that
/// survived. It is the reason the read has to touch the DECLARATIONS at all: `facade::thread` takes
/// the policy as an explicit capability and cannot look one up, deliberately (the compute layer
/// loads no catalogue), so the adapter above it must — exactly as `cli.rs` does for `nxc threads
/// show`. Since nxf 6j6v.v39s the capability carries the declared `members:` beside the visibility,
/// because the READ GATE has to read the file too — see [`ChannelPolicy`].
///
/// [`ChannelPolicy::undeclared`] is the honest fallback for a thread whose channel is not a declared
/// one at all: there is no policy to look up, and the behaviour for such a thread is unchanged. It
/// resolves the channel the SAME two ways `facade::thread` does (the `threads` view, else a stamped
/// message), so a thread that reads at all has its policy resolved from the channel it really sits
/// in.
fn declared_policy_of(
    store: &ChatStore,
    decl_dir: &Path,
    thread_id: &str,
) -> Result<ChannelPolicy> {
    let channel_id = store
        .thread_channel(thread_id)
        .or_else(|| store.thread_channel_via_message(thread_id));
    let channels = crate::channel::load_all_channels(decl_dir)?;
    Ok(crate::channel::declared_policy(
        &channels,
        channel_id.as_deref(),
    ))
}

/// Everything an [`orchestration::Ctx`] needs that the handle resolves for itself, owned for the
/// duration of one verb (the `Ctx` borrows, so these have to outlive it).
struct CtxInputs {
    defs: Definitions,
    /// The minting workspace identity, resolved from the handle's own workspace rather than taken
    /// per call (nxf 6j6v.07me) — see [`Engine::origin`].
    origin: String,
    db_path: String,
    project_claude_md: Option<String>,
}

impl Engine {
    /// Open the workspace `--db`/discovery resolves from `start` and open chat's store over it,
    /// holding both for the handle's lifetime. Fails (`no_workspace`/`io`) if there is no workspace
    /// or the store cannot be opened.
    ///
    /// **Unchanged by the orchestration surface**: declarations come from the workspace's
    /// `.nxs-personas/` folder and there is no worker ([`EngineConfig::default`]), so an embedder
    /// that only reads and posts messages is untouched. Orchestration is an opt-in via
    /// [`Engine::open_with`].
    pub fn open(db: Option<&str>, start: &Path) -> Result<Engine> {
        Self::open_with(db, start, EngineConfig::default())
    }

    /// Open with an explicit [`EngineConfig`] — caller-supplied declarations and/or a real worker.
    /// This is the entry point an embedding app uses to reach the role runtime (spec §3.2.2).
    pub fn open_with(db: Option<&str>, start: &Path, cfg: EngineConfig) -> Result<Engine> {
        Self::open_configured(db, start, cfg)
    }

    /// The one constructor the two public `open*` entry points funnel through.
    ///
    /// There were THREE until nxf 6j6v.yr59: `open_with_poll_interval` existed for a single value,
    /// and that value is [`EngineConfig::poll_interval`] now.
    fn open_configured(db: Option<&str>, start: &Path, cfg: EngineConfig) -> Result<Engine> {
        let handle =
            Handle::open_with_poll_interval(db, start, cfg.poll_interval, chat_store_factory())?;
        let worker: Arc<dyn Worker> = match &cfg.worker {
            WorkerConfig::Disabled => Arc::new(DisabledWorker),
            configured => configured.build()?,
        };
        // `build_in`, not `build` (nxf 6j6v.8see): an app's working directory is not its
        // workspace, and the service timer writes the deadline into the WORKSPACE's own book. The
        // handle already knows which workspace that is; the process does not.
        let timer = cfg.timer.build_in(&handle.workspace());
        // `build_in` for the timer's reason one line up, and one the naming run makes sharper: the
        // detached run it starts asks a MODEL, which reads the project context it is standing in.
        let namer = cfg.namer.build_in(&handle.workspace());
        Ok(Engine {
            handle,
            worker,
            timer,
            namer,
            module_primes: cfg.module_primes,
            machines: cfg.machines,
        })
    }

    /// **So a view stays alive** — external-write notifications (be9y, mirror of #9t7.3/#3rx.2).
    /// Returns a receiver that yields a [`Change`] whenever ANOTHER writer commits to this
    /// workspace: the app's cue to re-read its [`status`](Engine::status) tree or the
    /// [`thread`](Engine::thread) it is showing. Coalesced (do an initial read on subscribe);
    /// domain-blind (whole-db tick) — which thread moved is the app layer's re-read filter, and
    /// naming that residual is what `tests/seam_disposition.rs` does where the run-event stream
    /// with its per-delta payload left.
    ///
    /// The cadence is [`EngineConfig::poll_interval`] since nxf 6j6v.yr59 (it was a constructor of
    /// its own before that).
    pub fn subscribe(&self) -> Result<Receiver<Change>> {
        self.handle.subscribe()
    }

    // ---- reads (SEVEN verbs, nxf 6j6v.yr59 + the owner correction of 2026-08-21) ---------------
    //
    // Sixteen stood here. Seven carry the work — the six below plus `subscribe`, declared just
    // above this header — and the record of what became of the other ten is the comment block at
    // the bottom of this file plus the rows in `tests/seam_disposition.rs`; the SIZE of this
    // section is held by `tests/read_surface.rs`, which is the only gate that can see an eighth
    // growing back.
    //
    // Six of the seven answer a question none of the others answers: where the operation stands
    // (`status`), whom and what may be addressed (`directory`), how a session starts (`prime_as`),
    // what was said (`thread`), what a session did (`transcript_page`), and how a view stays alive
    // (`subscribe`). `search` answers NONE of them and is on the seam anyway, by the owner's
    // correction of 2026-08-21 — app-foundations consumes it today, and this item exists to spare
    // that consumer a second migration; its own doc carries the whole argument. `now` still travels
    // explicitly wherever a read depends on the clock — the library reads no ambient one.

    /// **What was said** — a thread assembled from the messages sharing its id (M1),
    /// membership-gated (`not_found`/`forbidden`) unless its channel is `public` (nxf 6j6v.bd6g).
    ///
    /// **The ONE CONVERSATION reader on the seam** (nxf 6j6v.yr59). `messages` stood beside it,
    /// keyed by CHANNEL, and lost: a thread id is the value [`send_to`](Engine::send_to) hands back
    /// and the only value [`reply_thread`](Engine::reply_thread) takes, and
    /// [`status`](Engine::status) answers in thread ids too — so the reader has to be keyed the way
    /// the rest of the surface is, or every render begins with a lookup. A channel read was also
    /// the wrong GRAIN after nxf 6j6v.pf6j: a two-level channel holds the requester's board and
    /// every member's own thread side by side, so "the messages in this channel" is several
    /// conversations interleaved.
    ///
    /// [`search`](Engine::search) hands back message bodies too and is NOT a second spelling of
    /// this one: it is keyed by TEXT, it answers with hits scattered across channels rather than
    /// with a conversation, and — see its own doc — it carries no `visibility` filter. "Exactly one
    /// message reader" was the choice between this and `messages`, and it still is.
    ///
    /// **It applies the channel's declared `visibility`**, resolved here from the thread's REAL
    /// channel policy — the resolution `thread_board` used to do, moved with the survivor. Without
    /// it this cut would have WIDENED what a non-requester may read, since the reader that carried
    /// the filter is the one that went. `AllMembers` remains the honest fallback for a thread whose
    /// channel no declaration names.
    pub fn thread(&self, thread_id: &str, as_handle: &str) -> Result<ThreadView> {
        let decl_dir = self.workspace().declaration_dir()?;
        self.handle.try_with_state(|s| {
            let policy = declared_policy_of(&s.store, &decl_dir, thread_id)?;
            facade::thread(&s.store, thread_id, as_handle, policy)
        })
    }

    /// **Body-substring search over the caller's channels** — every message whose body contains
    /// `query`, in the channels `handle` is a member of, as [`MessageHitView`]s. The `nxc search`
    /// derivation verbatim, which is what the `tests/parity.rs` differential compares the two seams
    /// on.
    ///
    /// **What "is a member of" means is the DECLARATION's answer since nxf 6j6v.cs03**, decided by
    /// the same rule [`thread`](Engine::thread) gates with rather than by the substrate's
    /// materialised member set: an edit to a declared channel's `members:` reaches this read
    /// directly, in both directions, and a member arrives here under the qualified `origin/handle`
    /// identity the surface gives it without having to have SENT anything first. Before that it
    /// answered "no matches" to any declared member that had only ever replied — including for that
    /// member's own words. [`facade::search`](crate::facade::search) carries the whole finding and
    /// the two doors it deliberately leaves shut.
    ///
    /// **THIS VERB IS ON THE SEAM BY AN OWNER CORRECTION OF 2026-08-21, not by nxf 6j6v.yr59's
    /// argument**, and that is recorded here rather than smoothed over. The item covers `search`
    /// NOWHERE — not the keep table, not the "what goes and why" list, not the five verbs its DoD
    /// sends to be decided one by one — and it was on the surface, so this build decided it in the
    /// item's own terms and removed it as a NAMED LOSS: it answers none of the six questions the
    /// keep table is built out of (not where we stand, not who may be addressed, not what was said
    /// in THIS conversation, not one session's history, not session start, not liveness), and its
    /// scoping is the channel-shaped one `channels` and `inbox` are leaving for. The gap was
    /// reported rather than glossed.
    ///
    /// **The owner overruled the removal on the fact that outweighs the shape.**
    /// app-foundations' `ChatClient` (`packages/engine-client/src/chat-client.ts`) carries `search`
    /// TODAY — measured on 2026-08-21, it is one of the 14 methods that still exist and are
    /// consumed, not one of the 14 that no longer do. Deleting a LIVE, consumed method inside the
    /// item whose stated purpose is to cost app-foundations ONE migration instead of two is a
    /// break, not a cleanup. So the read seam is SEVEN verbs; the argument for the other six is
    /// untouched, and the count in the item's own title is the thing that went stale.
    ///
    /// **It applies the channel's declared `visibility` since nxf 6j6v.px98**, resolved from the
    /// workspace's declarations exactly as [`thread`](Engine::thread) resolves the one board's
    /// policy beside it — so the two readers this seam has cannot answer "may I see this?"
    /// differently. Until then they did: `search` joined live memberships and nothing else, so on a
    /// `requester_only` board it handed a member a body `thread` withheld from that same caller.
    /// The paragraph that stood here said so plainly and called it unchanged rather than new, which
    /// it was — what it was NOT is harmless, once the owner ruled (2026-08-23) that `visibility` is
    /// an access rule and not a display rule. [`facade::search`](crate::facade::search) carries the
    /// rule and what it costs; `AllMembers` remains the honest fallback for a channel no
    /// declaration names, so an undeclared workspace reads exactly as it always did.
    pub fn search(&self, handle: &str, query: &str) -> Result<Vec<MessageHitView>> {
        let channels = crate::channel::load_all_channels(&self.workspace().declaration_dir()?)?;
        self.handle
            .try_with_state(|s| facade::search(&s.store, handle, query, &channels))
    }

    /// **Where does this operation stand** (nxf 6j6v.a71h) — the thread TREE an operation is, across
    /// channel borders, with every thread's state derived from the log.
    ///
    /// The same read `nxc status` runs, in all of its forms ([`StatusScope`]), and it is on
    /// the seam precisely because striking a CLI verb is not the same as striking a seam read (§5):
    /// an app builds its operation view out of this, and there is no second way to get it. What it
    /// replaces — `workflow status` / `workflow list` — reads a stored run record; this reads none,
    /// because there is none.
    ///
    /// **Three more reads folded in here in nxf 6j6v.yr59**, on that item's own condition that
    /// `status` first carry enough per thread — so [`StatusThread`](crate::facade::StatusThread)
    /// gained `deadline` and the working-tree fields, and [`StatusScope::Threads`] takes the
    /// EXPLICIT SET that used to be `thread_quorums`. `threads` (the caller's boards) and
    /// `thread_board` (one board with its replies) are gone from the handle; the messages half of
    /// the latter is [`thread`](Engine::thread). What does NOT come along is `threads`' membership
    /// scoping, deliberately: an operation crosses channels its reader is not in.
    ///
    /// `now` drives only the clock-dependent `stale` flag, like every other read here; the app passes its
    /// own wall clock rather than an ambient one.
    pub fn status(&self, now: &str, scope: StatusScope<'_>) -> Result<StatusReport> {
        self.handle
            .try_with_state(|s| facade::status(&s.store, &*self.worker, now, scope))
    }

    /// The `prime` session-bootstrap record for a caller that may be a declared PERSONA (nxf
    /// 6j6v.r5a2 / 6j6v.p6m1) — the coordination rule, the recovery hint, the command reference, the
    /// requester wake, the declared-team roster and, with a persona named, its own identity block
    /// and address book: all as data.
    ///
    /// **THE session-start read** (nxf 6j6v.yr59), and the reason three neighbours could leave.
    /// `prime` stood beside it and WAS `prime_as(consumer, None, now)` — a pure overload. `inbox`
    /// was the catch-up this record carried, split by disposition, until nxf 6j6v.4d2z removed the
    /// unread set from both. `opener_wake` is by its own doc the same derivation this CARRIES as
    /// `wake`. A persona gets what it needs at session start and on resume, which is what makes
    /// this one call rather than four.
    ///
    /// **That sentence used to end "…the same derivation this RENDERS as 'Threads you opened'", and
    /// nxf 6j6v.1gm9 read it as the confession it was.** If the wake is the same derivation a
    /// session is pushed anyway, rendering it at every start is the duplicate — 46.487 bytes of it
    /// in the workspace where this was measured. The record still CARRIES it for an app; the
    /// Markdown block does not draw it (see [`PrimeReport::render_markdown`]).
    ///
    /// **The host says who this is**, rather than the handle deriving it, because an app starts its
    /// own sessions and therefore knows — the same reason `--persona` exists on the CLI (see
    /// [`crate::persona`] for the identity rules and what they deliberately do NOT authorize).
    /// `consumer` stays separate from `persona`: the record is read for a qualified chat handle,
    /// the brief is composed for a bare declared one, and an app addressing one persona from two
    /// origins needs both. `None` is the human at the keyboard.
    ///
    /// Both render methods ([`render_markdown`](PrimeReport::render_markdown) for the SessionStart
    /// block, [`to_value`](PrimeReport::to_value) for its JSON form) take `declaration_errors`:
    /// whether to surface the roster's referential errors is the CALLER's rendering decision. `nxc`
    /// shows them only to a human at the keyboard; an app with no terminal notion of "interactive"
    /// decides on its own terms. The record carries them either way — see [`PrimeReport`].
    ///
    /// `now` drives only the wake's clock-dependent `stale` flag; the workspace's declaration
    /// directory is resolved from the handle's own workspace.
    pub fn prime_as(
        &self,
        consumer: &str,
        persona: Option<&str>,
        now: &str,
    ) -> Result<PrimeReport> {
        let source =
            // `resolve`, not `locate`: the report carries the declaration COUNT, and only loading
            // the catalogue can answer that. `locate` leaves it at 0, which reads as "nothing is
            // declared" on a workspace that declares plenty — a lie the parity differential caught
            // against `nxc prime`, which had always taken the loading path.
            crate::definitions::DeclarationSource::resolve(self.workspace().workspace_root()?)?;
        self.handle
            .try_with_state(|s| facade::prime_for(&s.store, consumer, persona, now, &source))
    }

    /// **What a spawned persona is handed at its session start** (nxf 6j6v.k8zq) — the composed
    /// `nxs prime --persona <handle>` text: the sibling modules' blocks this persona's declaration
    /// admits ([`EngineConfig::module_primes`]), then chat's own persona block.
    ///
    /// **THE EIGHTH READ, and it exists for a measured consumer rather than for a shape.**
    /// app-foundations starts role sessions of its own (`crates/agent-runtime`) and composes their
    /// system prompts itself. Without this call it would have to reassemble the block from
    /// [`prime_as`](Engine::prime_as) plus its own idea of the order, the filter and the two
    /// sibling modules — and the first time either side changed, an app's persona and an `nxc`
    /// persona would be reading different instructions. That is the two-contracts split the verb
    /// seam gate (6j6v.vtvs) exists against, one layer up. `crates/chat/tests/read_surface.rs`
    /// carries the same argument as the row that widened the count.
    ///
    /// It is the SAME function [`crate::orchestration`] composes a trigger's prompt from, so what
    /// this returns is what a spawn would use, byte for byte — that is the point of it being a
    /// read and not a second assembly.
    ///
    /// `handle` is a declared persona's bare handle. An undeclared one is a `not_found`
    /// ([`Definitions::role`](crate::definitions::Definitions::role)), unlike
    /// [`prime_as`](Engine::prime_as)'s soft degrade: there a session whose role file vanished
    /// mid-flight still deserves its session start, while here the caller is ASKING about a
    /// declaration and an empty answer would read as "this persona is told nothing", which is a
    /// different fact.
    pub fn persona_prime(&self, handle: &str, now: &str) -> Result<String> {
        let defs = self.definitions()?;
        let decl = defs.role(handle)?.clone();
        if !decl.prime.is_primed() {
            return Ok(String::new());
        }
        let services = decl.prime.services();
        let db_path = self.workspace().db_path_str()?;
        let siblings = match &self.module_primes {
            Some(source) => source.module_primes(services, &db_path)?,
            None => Vec::new(),
        };
        let consumer = format!("{}/{}", self.origin(), handle);
        self.handle.try_with_state(|s| {
            facade::compose_persona_prime(
                &s.store,
                &consumer,
                handle,
                services,
                &siblings,
                now,
                defs.source(),
            )
        })
    }

    /// **Whom and WHAT may be addressed, and what for** (`nxc list`, nxf 6j6v.p6m1) — the full
    /// declared catalogue with no `persona`, that persona's own possibilities with one.
    ///
    /// Resolved through [`Engine::definitions`], off the workspace's own declaration folder — the
    /// same catalogue `nxc list` prints, so the two surfaces cannot disagree, which is the reason
    /// this read exists on the handle instead of being re-derived app-side.
    ///
    /// **It names the CHANNELS and what they are for, and since nxf 6j6v.yr59 the FRONT DOORS too**
    /// — owner, 2026-08-21: *"`directory` zeigt uebrigens auch die Kanaele an und wofuer sie da
    /// sind."* `Engine::public_channels` was the workspace's discovery read, and it was the one
    /// thing the declarations genuinely could not answer: a public channel reaching this workspace
    /// by SYNC carries no declaration here. So it rides on this record now
    /// ([`Directory::public_channels`](crate::persona::Directory::public_channels)), and learning
    /// everything addressable is ONE call. `Engine::channels` — the membership-scoped lane read,
    /// which then still carried per-channel unread counts — went in the same cut and did NOT come
    /// along; see [`FrontDoor`](crate::persona::FrontDoor), whose argument for carrying no `unread`
    /// nxf 6j6v.4d2z then settled everywhere by removing the counts outright.
    /// **`None` means "for the human", not "for nobody"** (nxf 6j6v.st83, named here after the
    /// review of PR #472 found this doc silent about it). Since a persona may declare exactly WHO
    /// reaches it, [`PersonaEntry::direct`](crate::persona::PersonaEntry::direct) is no longer a
    /// caller-free fact: it answers for the audience the projection was made for — the person at
    /// the terminal here, the named persona in the other branch. An app rendering this for its
    /// logged-in user therefore gets the right answer; one rendering it as a neutral catalogue must
    /// read [`PersonaEntry::addressable`](crate::persona::PersonaBrief::addressable)'s sibling on
    /// the declaration instead of reading `direct` as "is a direct message possible at all".
    pub fn directory(&self, persona: Option<&str>) -> Result<crate::persona::Directory> {
        let defs = self.definitions()?;
        let directory = match persona {
            Some(handle) => {
                crate::persona::Directory::for_persona(defs.roles(), defs.channels(), handle)
            }
            None => crate::persona::Directory::full(defs.roles(), defs.channels()),
        };
        let doors = self
            .handle
            .try_with_state(|s| facade::front_doors(&s.store))?;
        // The same attachment `nxc list` makes, from the same catalogue and the same store read —
        // an app renders its EMPTY directory with the explanation and the path beside it, and the
        // parity differential can compare the two surfaces byte for byte because there is one
        // record, not two.
        Ok(directory
            .with_declarations(defs.source())
            .with_front_doors(doors))
    }

    /// **What one session DID**, over a WINDOW: the Claude Agent SDK transcript entries with `seq`
    /// strictly greater than `after_seq` (`-1` for the start), at most `limit` of them (nxf
    /// 6j6v.t7pa) — assistant text, extended thinking, tool calls and results, and every
    /// Task-spawned subagent's own sub-timeline nested under the `tool_use` that spawned it (nxf
    /// epic 6wt2, ticket 5bym). This is the seam an embedding app renders a session-detail timeline
    /// from: the messages a role posts are its distilled work product, this is what it actually did
    /// to produce them. Keyed by the INTERNAL session id (the one `refs.session_id` carries, and
    /// the one [`StatusThread::session`](crate::facade::StatusThread::session) hands back); an
    /// unknown one reads as an empty transcript, never an error.
    ///
    /// **The unwindowed `transcript` went with nxf 6j6v.yr59** and is `transcript_page(s, -1,
    /// None)` — which is what its own doc already said it delegated to. Pass those arguments for
    /// the whole session, and note what the whole session means: it materialises unbounded
    /// `tool_use` inputs, which is why the bounded read is the one that stayed.
    ///
    /// The cursor for the next window is the largest `seq` in this one; a window shorter than
    /// `limit` is the end. See [`facade::transcript_page`] for what a window boundary does to the
    /// subagent nesting — nothing is dropped, but a cut-off child surfaces at top level and the view
    /// carries no parent id to re-attach it with, so an app that needs the exact tree reads the
    /// session unwindowed.
    ///
    /// **This read takes no acting handle and enforces no membership**, unlike
    /// [`thread`](Engine::thread) beside it — a transcript has no channel to gate on, so the
    /// sibling gate is undefined rather than merely skipped (the full argument is on
    /// [`facade::transcript`]). It returns raw tool inputs and results, so **an app serving more
    /// than one user must authorize the caller itself before calling this.**
    pub fn transcript_page(
        &self,
        internal_session: &str,
        after_seq: i64,
        limit: Option<i64>,
    ) -> Result<TranscriptView> {
        self.handle.try_with_state(|s| {
            facade::transcript_page(&s.store, internal_session, after_seq, limit)
        })
    }

    /// **Record what one session DID**: append a batch of normalized transcript entries and report
    /// how many landed (nxf 6j6v.c6e8) — the write half of [`transcript_page`](Engine::transcript_page).
    ///
    /// This is the call for a host that drives the Claude Agent SDK itself rather than letting this
    /// binary's sidecar do it. That host is the same one [`bind_runtime_session`](Engine::bind_runtime_session)
    /// exists for, and for the same reason: a local session can shell out to `nxc transcript
    /// append`, a remote runtime cannot — so without this, binding a session and then having
    /// nothing to show for it was the whole of what a host could achieve.
    ///
    /// `seq` is the STORE's to assign (it is not on the wire), so batches from any number of
    /// callers continue ONE history for the session; pass the entries in the order they happened.
    /// A batch is one transaction — a single invalid entry rejects the whole flush, named by its
    /// index — and the first flush of a new session also runs retention. See
    /// [`facade::transcript_append`] for the full argument on each of those.
    ///
    /// **Takes no acting handle and enforces no membership**, symmetrically with the read beside
    /// it: a transcript has no channel to gate on. An app serving more than one user authorizes the
    /// caller itself.
    ///
    /// **And no size bound.** Unlike `nxc transcript append`, which caps the stdin it reads, this
    /// call takes the batch it is given: one `BEGIN IMMEDIATE` holding the workspace's single write
    /// lock for as long as the batch takes. A host ingesting from an untrusted or unbounded byte
    /// stream bounds it BEFORE calling — [`facade::transcript_append`] carries the full argument for
    /// why the bound is the caller's and not this call's (PR #407 review, Integrity #1).
    pub fn transcript_append(
        &self,
        internal_session: &str,
        entries: &[TranscriptEntry],
    ) -> Result<u64> {
        self.handle
            .with_state_mut(|s| facade::transcript_append(&mut s.store, internal_session, entries))
    }

    /// Retire the transcripts of sessions whose last recorded activity is older than `keep_days`
    /// before `now` (nxf 6j6v.t7pa), reporting what went — the operator's lever over
    /// `agent_transcript`, which is otherwise the highest-volume thing chat records.
    ///
    /// **An app does not have to call this to stay bounded**: retention also rides the first flush of
    /// every new session (see [`ChatStore::append_transcript`](crate::store::ChatStore::append_transcript)).
    /// This is for clearing out a workspace that has gone quiet — where no new session is coming to
    /// trigger the automatic pass — or for applying a tighter window than the configured one.
    /// `dry_run` reports what a real run would remove and writes nothing. Note that it bounds growth
    /// rather than shrinking the file on disk; `ChatStore::prune_transcripts` says why no `VACUUM`.
    pub fn prune_transcripts(
        &self,
        now: &str,
        keep_days: i64,
        dry_run: bool,
    ) -> Result<TranscriptPruneReport> {
        self.handle
            .with_state_mut(|s| facade::prune_transcripts(&mut s.store, now, keep_days, dry_run))
    }

    // ---- writes (now + acting-handle explicit) -----------------------------

    // `mark_read(now, consumer, channel, through)` stood here — the ack that advanced the synced
    // read cursor. REMOVED with the whole unread apparatus (nxf 6j6v.4d2z), and it was the LAST
    // write on this handle whose fact nothing on the handle could read back.
    //
    // What it was for was checked rather than assumed, twice. The 2026-09-05 gate found it moved
    // two things: the unread catch-up `prime_as` carried, and the session-start notice
    // (`threads_you_opened`), which dropped a completed board once this cursor passed its
    // completing reply. The second one is why the removal was held for three days — the
    // measurement it rested on named only the first. nxf 6j6v.2hx9 then derived the notice from
    // the OPERATION instead, which retired that consumer by REPLACING it, and the catch-up went
    // here with the cursor it was derived from.

    /// The resolved workspace (db path, replica identity).
    pub fn workspace(&self) -> Workspace {
        self.handle.workspace()
    }

    /// Bind a runtime's OWN opaque session id to the internal session a trigger minted — the
    /// out-of-band half of the worker seam (nxf 6j6v.5x9j), and the library twin of `nxc session
    /// bind`, which until now was the only way to do this at all.
    ///
    /// **This is what makes a host worker able to be asynchronous without the engine being.** A
    /// [`crate::worker::Worker`] whose provisioning is a network call answers
    /// [`crate::worker::TriggerOutcome::Accepted`], hands the call to its own executor and returns
    /// immediately; when the call resolves it calls this with the id the runtime handed back. The
    /// local sidecar does the same thing by a different route — from INSIDE the spawned session,
    /// through the CLI — which is why this existed nowhere on the handle before: a local session can
    /// reach a CLI, a remote one cannot reach anything but its host.
    ///
    /// A worker that already has the id when `trigger` returns should answer
    /// [`Started`](crate::worker::TriggerOutcome::Started) instead and let the engine bind it; this
    /// is for the case where it genuinely does not yet.
    ///
    /// An `internal_session` that was never minted is `not_found` — a binding must not silently
    /// succeed against nothing, exactly as `nxc session bind` has always held.
    pub fn bind_runtime_session(
        &self,
        internal_session: &str,
        runtime_session: &str,
    ) -> Result<()> {
        self.handle
            .with_state_mut(|s| s.store.bind_session(internal_session, runtime_session))
    }

    // ---- declarations ------------------------------------------------------

    /// The declarations this handle resolves through, read fresh from the workspace's
    /// `.nxs-personas/` folder (see [`crate::definitions::DeclarationSource`] for the resolution
    /// and for the legacy `roles/` compatibility). Carries its own source resolution, so an app can
    /// render "nothing is declared, and here is where it goes" without re-deriving the path.
    ///
    /// **A READ, and it does not fail because there is nothing to read.** A workspace that declares
    /// nothing resolves to an EMPTY catalogue and succeeds — pinned on the app side by
    /// `engine-bridge/src/chat.rs` and `engine-server/tests/chat_wire.rs`, and here by
    /// `tests/declaration_source.rs`. It fails only for what genuinely is a failure: a malformed or
    /// unreadable declaration, with the loaders' own `io`/`validation` error.
    ///
    /// **This is the READ, and it stays** while the WRITE (`set_definitions`) went with
    /// `DefinitionSource::Supplied` in nxf 6j6v.dvyq step 6. The app still reads the catalogue to
    /// render roles and channels; it merely no longer supplies it. That the two must not be grabbed
    /// together during a cleanup is not left to care: `tests/seam_disposition.rs` names this method
    /// with its reason, and fails if it disappears.
    pub fn definitions(&self) -> Result<Definitions> {
        self.resolve_definitions(&self.workspace())
    }

    /// Resolve the workspace's declaration folder into a catalogue. Per call, never cached: an edit
    /// on disk is therefore visible to the NEXT verb on this handle and on every existing clone of
    /// it, with no reopen and no torn-down `subscribe` receiver — the realistic desktop case is a
    /// user editing an agent in the app, and the next trigger has to see it. That property used to
    /// need `set_definitions` and a lock; with the injection path gone it is simply what re-reading
    /// means.
    ///
    /// **"The next verb" is not "the next STEP of a chain already running"** (nxf 6j6v.n92p). What
    /// this resolves is the folder as it is now, which is what every READ on this handle wants and
    /// what a new operation opens under. A verb that continues an operation swaps this for the
    /// version that operation was bound to ([`crate::declaration_freeze`]), so an edit reaches the
    /// next operation rather than the middle of one.
    ///
    /// The read happens BEFORE the store lock is taken (see [`Engine::with_orchestration`]), so a
    /// verb never holds the store while touching the filesystem.
    fn resolve_definitions(&self, ws: &Workspace) -> Result<Definitions> {
        Definitions::resolve(ws.workspace_root()?)
    }

    // ---- the orchestration verbs (spec §4) ---------------------------------
    //
    // Flat on the handle, beside the messaging half, because app-foundations reaches the engine
    // through ONE bridge. Each verb takes two things: a [`Caller`] — WHO IS CALLING, as NAMED
    // fields rather than a fixed-order prefix that a transposition could silently reorder — and its
    // `orchestration::*Request`. Nothing here is a parallel implementation: every body is the SAME
    // `orchestration` function `nxc` calls, and the seam adds no semantics.
    //
    // `Caller` carried five values until nxf 6j6v.07me (`now`, `origin`, `actor`, `session`, `hop`)
    // and carries two and a half now. What the host still owns is `session` — the SENDER's, the
    // return address a reply wakes — plus an `actor` for the case there is no session and an
    // optional `now`. The rest the handle answers for itself, right here in this block:
    // [`Engine::origin`] from its own workspace, `actor` from the session map
    // ([`Caller::resolve_actor`]), `now` from the clock when nothing is pinned, and `hop` from
    // nowhere at all (see [`Caller::into_ctx`]). The CLI is unchanged and still resolves its own
    // from `NXC_*`, which is what keeps "the library has no ambient environment" true: what moved
    // is one clock read into the ADAPTER, not an environment read into the verbs.

    /// Everything the `Ctx` borrows that the handle resolves for itself, in the CLI's own
    /// resolution (`cli.rs::CliCtx::resolve`) but off the handle's OWN resolved workspace rather
    /// than the process working directory:
    ///
    /// - `db_path` — the workspace's db, stamped into a spawned role's `NXC_DB` so its in-session
    ///   `nxc` calls resolve the same workspace the app is driving. There is no other sensible
    ///   answer: the handle owns exactly one workspace.
    /// - `project_claude_md` — `<workspace-root>/CLAUDE.md`, read PER CALL, not cached at open. A
    ///   long-lived handle that snapshotted it would serve a file the user edited hours ago, which
    ///   is the same staleness [`resolve_definitions`](Engine::resolve_definitions) avoids by
    ///   re-reading the declaration folder per call; this read costs one `read_to_string` on the
    ///   verbs that compose a prompt and makes the handle behave exactly like a fresh `nxc`
    ///   process. Absent file → `None`, as in the CLI: a workspace without one still composes
    ///   cleanly.
    ///
    /// - `origin` — the minting workspace identity, since nxf 6j6v.07me resolved HERE instead of
    ///   arriving on every call. See [`Engine::origin`] for the value and the argument.
    fn ctx_inputs(&self) -> Result<CtxInputs> {
        let ws = self.workspace();
        let root = ws.workspace_root()?.to_path_buf();
        Ok(CtxInputs {
            defs: self.resolve_definitions(&ws)?,
            origin: self.origin(),
            db_path: ws.db_path_str()?,
            project_claude_md: std::fs::read_to_string(root.join("CLAUDE.md")).ok(),
        })
    }

    /// The MINTING WORKSPACE IDENTITY (spec §2.2) every write through this handle is stamped with,
    /// and the `<origin>` half of the qualified handles it mints — `<origin>/<actor>` as a message
    /// `sender`, and the `consumer` an app has to pass back into [`thread`](Engine::thread) /
    /// [`prime_as`](Engine::prime_as) to read what it just wrote. (It used to name `messages` and
    /// `channels` here; both went with nxf 6j6v.yr59, and the OBLIGATION did not — every read that
    /// takes an acting handle still takes `<origin>/<actor>`.)
    ///
    /// **It used to be a [`Caller`] field and is now the handle's answer** (nxf 6j6v.07me). One
    /// handle is one workspace, so a per-call value could only ever agree with this or be wrong —
    /// and this is precisely the value access rights will hang off (nxf 6j6v.6aza), which makes
    /// "wrong" expensive rather than cosmetic. It is also why this READ exists rather than the value
    /// simply disappearing: a caller that no longer supplies the origin still has to be able to name
    /// what it wrote.
    ///
    /// The value is [`crate::workspace::origin_of`] over this handle's OWN workspace — the
    /// workspace's replica prefix, and the same answer `cli.rs` falls back to when `NXC_ORIGIN` is
    /// unset, so an app and a terminal in one workspace address one another's personas rather than
    /// two disjoint sets of handles. That function's own doc carries the rest: why the owner chose
    /// the prefix over a workspace-independent constant on 2026-08-21, where the federated identity
    /// lands when 6j6v.kz8p arrives, and the accepted `adopt_prefix` hazard (nxf 6j6v.dnzt).
    ///
    /// It reads [`workspace`](Engine::workspace), which is the identity resolved AT OPEN and held in
    /// handle state — not a fresh `replica.toml` read per call, unlike the declarations and the
    /// project `CLAUDE.md` next door. So a long-lived handle keeps minting under the prefix it
    /// opened with even if `adopt_prefix` rewrites the file underneath it. That is not a second
    /// hazard: it is the same accepted one (nxf 6j6v.dnzt), and re-reading per call would make the
    /// handle mint under two identities within one session instead of one stale one.
    pub fn origin(&self) -> String {
        crate::workspace::origin_of(&self.workspace()).to_string()
    }

    /// Build the `Ctx` and run one verb against the locked store: the caller's own half comes in as
    /// a [`Caller`], the adapter's half from [`ctx_inputs`](Engine::ctx_inputs), and
    /// [`Caller::into_ctx`] joins them — the mapping is written once, beside `Ctx`, not restated at
    /// every verb. The definitions snapshot and the workspace-derived inputs are resolved BEFORE
    /// the store lock is taken (see the [`Engine::defs`] field doc).
    ///
    /// **The two values nxf 6j6v.07me took off the caller are resolved here**, and where each is
    /// resolved is deliberate:
    ///
    /// - `now` — BEFORE the lock. Every verb inside one call must agree on one instant, and reading
    ///   the clock per use inside a locked section would make the same call's writes disagree with
    ///   each other by however long the section took. `Caller::now` overrides it; the CLI always
    ///   passes one, which is what keeps its goldens byte-stable.
    /// - `actor` — INSIDE the lock, because it is a store read ([`Caller::resolve_actor`] asks the
    ///   session map which persona the caller's session belongs to). Under the same lock the verb
    ///   runs in, so the identity a write is attributed to is read from the same snapshot the write
    ///   lands in.
    fn with_orchestration<T>(
        &self,
        caller: Caller<'_>,
        f: impl FnOnce(&Ctx, &mut ChatStore) -> Result<T>,
    ) -> Result<T> {
        let inputs = self.ctx_inputs()?;
        let now = match caller.now {
            Some(pinned) => pinned.to_string(),
            None => orchestration::system_now()?,
        };
        self.handle.with_state_mut(|s| {
            let actor = caller.resolve_actor(&s.store)?;
            let ctx = caller.into_ctx(
                Ambient {
                    now: &now,
                    origin: &inputs.origin,
                    actor: &actor,
                },
                orchestration::Adapter {
                    defs: &inputs.defs,
                    worker: &*self.worker,
                    timer: &*self.timer,
                    namer: &*self.namer,
                    db_path: &inputs.db_path,
                    project_claude_md: inputs.project_claude_md.as_deref(),
                    module_primes: self
                        .module_primes
                        .as_deref()
                        .map(|p| p as &dyn crate::facade::ModulePrimes),
                    machines: self
                        .machines
                        .as_deref()
                        .map(|m| m as &dyn crate::machine::Machines),
                },
            );
            f(&ctx, &mut s.store)
        })
    }

    /// Open a conversation with a target and get its thread back (`nxc send --to`, nxf 6j6v.p6m1).
    ///
    /// **The one call an app needs to start anything.** What follows — materialise the direct
    /// conversation and start a persona, fan a declared channel out to its members, or post into a
    /// plain channel and wake nobody — is decided by the TARGET's declaration rather than by
    /// picking one of several verbs, which is the whole point of this surface (see
    /// [`crate::surface`] for the failure that motivated it). The verbs it sits over are untouched
    /// and still callable.
    ///
    /// The receipt's `thread_id` is workspace-unique: it is the only value the caller has to keep,
    /// and [`reply_thread`](Engine::reply_thread) takes it with no channel context.
    ///
    /// **A summon that fails on the persona path still returns `Ok`** (nxf 6j6v.hpv8): the thread is
    /// already open and the message already posted by the time the role is triggered, so a spawn
    /// failure reports through [`SendToReceipt::warnings`] rather than discarding the receipt behind
    /// an `Err`. Check that field — it is never omitted, even empty — the way
    /// [`orchestration::coordinator_commission`]'s own
    /// doc explains for the sibling verb.
    pub fn send_to(&self, caller: Caller<'_>, req: SendToRequest) -> Result<SendToReceipt> {
        self.with_orchestration(caller, |ctx, store| surface::send_to(ctx, store, req))
    }

    /// **Where a chat runs, before anything is sent** (`nxc machine`, nxf 6j6v.1c6k) — the machine
    /// a new chat with a persona would run on, or the one an existing chat runs on; where that came
    /// from (this call's choice, the chat, the persona's `machine:`, or the machine asking); whether
    /// it is online; whether sending now would ASK instead of sending; and the online machines to
    /// offer. Writes nothing. [`send_to`](Engine::send_to) and
    /// [`reply_thread`](Engine::reply_thread) make exactly this resolution before they write, and
    /// refuse with the same question when [`must_ask`](crate::machine::MachineAnswer::must_ask) —
    /// so an app renders the choice from here and passes the answer as `machine`.
    pub fn machine(
        &self,
        caller: Caller<'_>,
        query: surface::MachineQuery,
    ) -> Result<crate::machine::MachineAnswer> {
        // A READ, and it takes the read lock: it writes nothing, so it must not spell itself the
        // way a write does (`read_surface.rs` holds every method on this handle to that).
        let inputs = self.ctx_inputs()?;
        let now = match caller.now {
            Some(pinned) => pinned.to_string(),
            None => orchestration::system_now()?,
        };
        self.handle.try_with_state(|s| {
            let actor = caller.resolve_actor(&s.store)?;
            let ctx = caller.into_ctx(
                Ambient {
                    now: &now,
                    origin: &inputs.origin,
                    actor: &actor,
                },
                orchestration::Adapter {
                    defs: &inputs.defs,
                    worker: &*self.worker,
                    timer: &*self.timer,
                    namer: &*self.namer,
                    db_path: &inputs.db_path,
                    project_claude_md: inputs.project_claude_md.as_deref(),
                    module_primes: None,
                    machines: self
                        .machines
                        .as_deref()
                        .map(|m| m as &dyn crate::machine::Machines),
                },
            );
            surface::machine(&ctx, &s.store, query)
        })
    }

    /// **Pick up every chat this machine runs that is owed a turn** (`nxc pick-up`, nxf
    /// 6j6v.1c6k) — see [`orchestration::pick_up`]. For a host that executes: the `nxs` service
    /// runs it after every pass that synced, and an embedding host that runs persona sessions
    /// itself calls it when its own sync pulled something. Idempotent, and a handle whose config
    /// names no machine ([`EngineConfig::machines`]) picks up nothing.
    pub fn pick_up(&self, caller: Caller<'_>) -> Result<orchestration::PickupReport> {
        self.with_orchestration(caller, orchestration::pick_up)
    }

    /// Answer in a thread (`nxc reply --thread`, nxf 6j6v.p6m1).
    ///
    /// [`orchestration::reply`]'s whole routing runs here — it IS that verb, called with the
    /// thread as its target — plus the thread's own return address, so an app replying into a
    /// conversation hands the turn back to whoever is on the other side of it instead of posting
    /// into silence. The receipt reports what happened, including a wake that was attempted and
    /// missed.
    ///
    /// **Three statements, and the third is one bit** (nxf 6j6v.ckeq): `thread`, `body`, and
    /// [`escalate`](ReplyThreadRequest::escalate) — "I need help, or a decision", the one
    /// remaining door to [`MessageKind::Escalation`](crate::model::MessageKind::Escalation). What
    /// went is argued on [`ReplyThreadRequest`] itself. `if_unanswered` went too and is NOT a
    /// missing app capability: it is the sidecar teardown's idempotency switch and lives at
    /// [`surface::settle_if_unanswered`], which the CLI reaches and an app never needs.
    pub fn reply_thread(
        &self,
        caller: Caller<'_>,
        req: ReplyThreadRequest,
    ) -> Result<ReplyReceipt> {
        self.with_orchestration(caller, |ctx, store| {
            surface::reply_in_thread(ctx, store, req)
        })
    }

    /// **Say what a conversation is called** (`nxc threads name`, nxf 6j6v.e76c).
    ///
    /// The write behind the register [`crate::store::ThreadQuorum::name`] reports, under the rule
    /// that makes the name safe to render: **once**. A thread that already carries a name keeps it
    /// and the receipt says `named: false`; whatever is passed is cut to a display name first, so a
    /// host cannot put a paragraph where a conversation list expects a label.
    ///
    /// **Why an app wants this even though `send` names threads by itself.** It only does so where
    /// the handle asked for it — [`EngineConfig::namer`] defaults to naming nothing, because the
    /// shipped derivation starts a detached process that runs a MODEL, and an app that opened a
    /// workspace to read and post must not start paying for that. A host with a model of its own
    /// derives the name inside its own budget and writes it here; a host with none can let the user
    /// type it. Both reach the same register the engine's own naming run writes.
    ///
    /// A thin forwarding, in [`send_to`](Engine::send_to)'s shape and for its reason: the verb body
    /// is [`crate::facade::name_thread`] on the compute layer, and a second implementation here
    /// would be a second contract nobody reviews.
    pub fn name_thread(
        &self,
        caller: Caller<'_>,
        thread: &str,
        name: &str,
    ) -> Result<crate::facade::NameReceipt> {
        self.with_orchestration(caller, |ctx, store| {
            crate::facade::name_thread(store, ctx.now, &ctx.caller_handle(), thread, name)
        })
    }

    /// Take back a commission — one still parked behind the working copy, or a round that is
    /// running (`nxc withdraw`, nxf
    /// 6j6v.0h3p).
    ///
    /// **Who may call it** (nxf 6j6v.ezbr — the owner's decision of 2026-09-20). A person, or an
    /// assistant a person started — and only the caller who ran the operation's first
    /// [`send_to`](Engine::send_to), naming the thread that call handed back. Refused before
    /// anything changes, each by name:
    ///
    /// - a [`Caller::session`] this workspace minted (`forbidden`), whatever it names — the
    ///   refusal names the session, its role, and escalation on its own thread as the way to ask.
    ///   A host acting for its logged-in user passes no session, and is not touched by this;
    /// - a caller who did not open the operation (`forbidden`), naming who did;
    /// - a thread below the operation's first one (`validation`), naming that first thread.
    ///
    /// **This narrows the seam** — until this item any caller who could READ the thread could
    /// withdraw it. A host that let an agent session call this, or that withdrew a member thread
    /// it was handed by a status read, now gets the refusal instead.
    ///
    /// **A withdrawal interrupts the chain below the thread it names** (nxf 6j6v.s2cj): every
    /// thread between what was taken back and that thread that is still waiting is discharged
    /// too, so no next step is commissioned — on either shape of ordered channel — and nothing is
    /// consolidated. See [`orchestration::withdraw`].
    ///
    /// **The answer to a receipt this seam already hands out.** [`SendToReceipt::queue_position`]
    /// reports "the persona is NOT working on this yet — it is Nth in the working-tree queue", and
    /// until nxf 6j6v.0djn there was no call on this handle to act on that report: a state was
    /// announced and was not actionable. The measurement that settled it is
    /// `embed_surface.rs::engine_send_to_a_busy_exclusive_persona_reports_the_queue_instead_of_a_started_session`
    /// — two `exclusive` personas, two [`send_to`](Engine::send_to) calls through this handle and
    /// nothing else, and the second receipt comes back `queue_position: Some(1)`. The lease is not
    /// a property of the command line: `working_tree: exclusive` is a DECLARATION, the gate that
    /// reads it sits in the one trigger funnel under [`send_to`](Engine::send_to), and a host that
    /// opens such a workspace contends for the lease exactly as the CLI does.
    ///
    /// A thin forwarding in the shape of [`send_to`](Engine::send_to) and
    /// [`reply_thread`](Engine::reply_thread), for the same reason those are thin: the verb body is
    /// [`orchestration::withdraw`] on the compute layer, and a second implementation here would be
    /// a second contract nobody reviews.
    ///
    /// **Both cases** (the running one since nxf 6j6v.b9nf): a commission with no session, no
    /// transcript and no model call behind it is simply taken out of the queue; a round whose
    /// session is RUNNING is discharged and its session asked to stop through
    /// [`Worker::stop_session`](crate::worker::Worker::stop_session) — and when that round holds
    /// this workspace's working copy AND a park of it would produce a branch, the receipt says
    /// `will_park` and the background tick commits whatever it left uncommitted onto a branch once
    /// nothing in that claim area has a live process any more — the stopped session and anything
    /// sharing its claim — then hands the copy on. Where a park could never succeed — this host's
    /// worker names no working copy, the directory is no git repository, the operation recorded no
    /// base — the receipt says so on
    /// [`cannot_park`](orchestration::WithdrawReceipt::cannot_park) instead, and the same tick
    /// hands the copy on UNPARKED with the named finding: the work stays in the tree (fix round 2
    /// of this item's review, Code Quality #1).
    /// Nothing is rolled back. The way back is the one every park has: the next commission into the
    /// SAME thread — [`reply_thread`](Engine::reply_thread) into it on a direct persona thread, a
    /// follow-up into the channel thread for a round — restores the branch and tells the session
    /// where its work is; [`status`](Engine::status) lists the branch under the operation until
    /// then. A fresh [`send_to`](Engine::send_to) opens a new claim and does not find it.
    ///
    /// **A host whose worker cannot stop a session is refused WHOLE**, by name and before anything
    /// changes ([`Worker::stops_sessions`](crate::worker::Worker::stops_sessions) is `false`): a
    /// [`WorkerConfig::Custom`](crate::worker::WorkerConfig::Custom) worker that has not
    /// implemented the stop gets a `validation` error for any area with a running session, and the
    /// queued case behaves exactly as before. A stop the worker supports and then cannot deliver is
    /// a [`ConsequenceClass::SessionNotStopped`](orchestration::ConsequenceClass::SessionNotStopped)
    /// finding on the receipt's `warnings`, not a failure of the call — the thread's debt was
    /// discharged before the attempt. A session that had ALREADY ended when the stop was sent is
    /// not that case at all: it is the state the call was asked for, carried as
    /// [`StoppedSession::already_gone`](orchestration::StoppedSession::already_gone) with no
    /// warning. [`orchestration::withdraw`] carries the whole argument, including why the queue
    /// removal is itself the precondition check for the queued case.
    ///
    /// **A host whose worker cannot ANSWER the liveness question is refused by name too** (nxf
    /// 6j6v.b9nf, fix round 3 of that item's review): where
    /// [`Worker::answers_liveness`](crate::worker::Worker::answers_liveness) is `false` and the
    /// area holds sessions that never reported an end, this returns a `validation` error naming
    /// them rather than the `not_found` that would have said no session there has a live process —
    /// a fact such a host never established.
    ///
    /// `thread` is the thread the caller was handed — [`SendToReceipt::thread_id`] — and the AREA
    /// below it is what is searched, so a `send_to` into a declared channel takes back the round it
    /// commissioned rather than only the one thread it names.
    pub fn withdraw(
        &self,
        caller: Caller<'_>,
        thread: &str,
    ) -> Result<orchestration::WithdrawReceipt> {
        self.with_orchestration(caller, |ctx, store| {
            orchestration::withdraw(ctx, store, thread)
        })
    }

    // `release_working_tree` was here — "give this workspace's working copy back when nothing is
    // going to give it back on its own" (`nxc release`, nxf 6j6v.fabb). REMOVED by nxf 6j6v.b9nf
    // (owner decision, 2026-09-17): offered on this handle, it could take the working copy away
    // from a running coding operation, and a `WorkerConfig::Custom` worker that never implemented
    // `Worker::session_is_running` made it an unconditional release with no guard at all. Every
    // hand-off now parks first and this handle's two ways out are [`withdraw`](Engine::withdraw)
    // for a chain a caller wants back, running or queued, and the background service's own sweep
    // for a chain that is dead — see [`orchestration::sweep_expired_working_tree`] and
    // `tests/seam_disposition.rs`'s row for the decision and removal dates.

    /// **A session reports that its own process is over** (`nxc session ended`, nxf 6j6v.10yb) —
    /// `bind_runtime_session`'s sibling, and on the handle for its reason: a local sidecar can shell
    /// out to `nxc`, a runtime that owns its sessions elsewhere cannot reach anything but its host.
    ///
    /// **What a host gains by calling it.** A channel that declares `working_tree: exclusive` no
    /// longer opens its next step on a member's REPLY alone — it also asks whether that member's
    /// session is still running, because a reply is a claim on a message board and the process
    /// behind it can keep writing into the same checkout (measured: eighteen minutes, three
    /// overlapping sessions, one working copy). A host that ends a session and says so releases the
    /// step here and now; one that says nothing falls back to
    /// [`Worker::session_is_running`](crate::worker::Worker::session_is_running), asked again on a
    /// clock of the gate's own (nxf 6j6v.858n): the channel's declared `timeout:` cannot end this
    /// wait — that is a deadline for an ANSWER, and the member has answered — so a decline blocked
    /// only by a live session schedules its own re-check
    /// [`SESSION_LIVENESS_RECHECK_SECS`](orchestration::SESSION_LIVENESS_RECHECK_SECS) later.
    ///
    /// **A host that supplies no timer gets no re-check.** [`TimerConfig::Disabled`](crate::timer::
    /// TimerConfig::Disabled) is the default for an embedder and schedules nothing, deliberately —
    /// so for such a host a lost end announcement still leaves the round standing until something
    /// looks. Calling this is what keeps you out of that; supplying a timer is what makes the
    /// automatic look happen.
    ///
    /// Idempotent, and safe to call for a session that never stood in a channel at all — see
    /// [`orchestration::session_ended`] for both. An `internal_session` this workspace never minted
    /// is `not_found`, exactly as [`bind_runtime_session`](Engine::bind_runtime_session) holds.
    pub fn session_ended(
        &self,
        caller: Caller<'_>,
        internal_session: &str,
    ) -> Result<orchestration::SessionEndedReceipt> {
        self.with_orchestration(caller, |ctx, store| {
            orchestration::session_ended(ctx, store, internal_session)
        })
    }

    /// **Record that a session stopped because the model went away** (`nxc session interrupted`,
    /// nxf 6j6v.npy3) — the runtime's third callback, beside
    /// [`bind_runtime_session`](Engine::bind_runtime_session) and
    /// [`session_ended`](Engine::session_ended).
    ///
    /// An exhausted quota, an unreachable provider, a broken network: one state, and nobody did
    /// anything wrong. It says something the engine cannot work out for itself — only the runtime
    /// knows that a provider REFUSED this session as opposed to crashing under it, and only the
    /// runtime is told when the window lifts — which is why this is a callback and not a
    /// derivation.
    ///
    /// `limit` is the runtime's own name for the window (`seven_day`, `five_hour`, or the error
    /// class where it named none) and `until` is when it falls. **`None` for `until` is a real
    /// answer and must not be filled in with a guess**: what it costs is the automatic way back —
    /// there is no moment to arm one for — and what is left is
    /// [`resume_interrupted`](Engine::resume_interrupted), run by somebody who knows what the record
    /// does not.
    ///
    /// Idempotent while the hold stands: a retried teardown records one hold, never two.
    ///
    /// **What a host gets for calling it**: the session stops looking like a clean end, so
    /// [`status`](Engine::status) and [`session_state`](Engine::session_state) can both say why
    /// nobody is working in that operation — and the way back is armed, if this handle has a timer
    /// that anything is watching.
    pub fn session_interrupted(
        &self,
        caller: Caller<'_>,
        internal_session: &str,
        limit: &str,
        until: Option<&str>,
        detail: &str,
    ) -> Result<orchestration::InterruptionReceipt> {
        self.with_orchestration(caller, |ctx, store| {
            orchestration::session_interrupted(ctx, store, internal_session, limit, until, detail)
        })
    }

    /// **Take up an operation that stopped at an availability boundary** (`nxc resume`, nxf
    /// 6j6v.npy3) — the way back, for a human who is not waiting for the window and for a scheduler
    /// that runs when it falls.
    ///
    /// **It checks the world before it starts anything**, and most of [`ResumeOutcome`](
    /// orchestration::ResumeOutcome) is the list of what it found instead: the round was already
    /// answered, a process is already alive, the window has not lifted, the working copy moved, this
    /// runtime cannot continue a conversation it began. An interrupted turn may already have written
    /// files and posted messages — in the case this was cut from, its `nxc reply` landed 0.6 seconds
    /// before the wall — so a resume that trusted the last intention would redo whatever had
    /// already landed.
    ///
    /// The session is continued WITH its runtime conversation, never restarted: the transcript
    /// carries every tool call and what it returned, which is what stops the work happening twice.
    /// A worker that cannot do that says so ([`Worker::resumes_sessions`](crate::worker::Worker::
    /// resumes_sessions)) and is refused by name rather than quietly given a fresh session.
    ///
    /// Idempotent and safe to call speculatively: nothing on hold is a clean
    /// [`NothingOnHold`](orchestration::ResumeOutcome::NothingOnHold).
    pub fn resume_interrupted(
        &self,
        caller: Caller<'_>,
        req: orchestration::ResumeRequest<'_>,
    ) -> Result<orchestration::ResumeReceipt> {
        self.with_orchestration(caller, |ctx, store| {
            orchestration::resume_interrupted(ctx, store, req.clone())
        })
    }

    /// **Hand a settled caller everything that arrived while it was working** (`nxc session
    /// deliver`, nxf 6j6v.gn8b) — ONE resume with every answer held for `internal_session`, in the
    /// same delivery shape a channel round uses, naming what is still outstanding.
    ///
    /// **The seam half of a queue an app can otherwise only fill.** An embedder that replies
    /// through [`reply_thread`](Engine::reply_thread) into a caller that is mid-turn gets
    /// [`ReplyReceipt::held`](orchestration::ReplyReceipt::held) back — the answer is durably
    /// queued — and without this call there would be no way to hand it over from the same seam. The
    /// CLI has the background service arming it; a host that runs its own sessions knows when one
    /// settles and can say so directly.
    ///
    /// Idempotent and safe to call speculatively: nothing held is a clean `delivered: 0`, and the
    /// rows are released only once the resume has been handed over. A caller that is running again
    /// is refused by the single-process guard and its answers stay queued.
    pub fn deliver_held(
        &self,
        caller: Caller<'_>,
        internal_session: &str,
    ) -> Result<orchestration::DeliveryReceipt> {
        self.with_orchestration(caller, |ctx, store| {
            orchestration::deliver_held(ctx, store, internal_session)
        })
    }

    /// **What is still waiting for a caller** (nxf 6j6v.gn8b) — the answers held for
    /// `internal_session` because it was mid-turn when they arrived, in arrival order.
    ///
    /// THE READER for the queue the two calls around it move.
    /// [`reply_thread`](Engine::reply_thread) fills it — its receipt's
    /// [`held`](orchestration::ReplyReceipt::held) says when — and
    /// [`deliver_held`](Engine::deliver_held) empties it. Without this an app could do both and
    /// never ask what was in it, which is a fact recorded on this seam and unreadable through it;
    /// `crates/chat/tests/read_surface.rs` is the gate that refuses exactly that, and it refused
    /// this one before the read existed.
    ///
    /// **A plain read: no [`Caller`], nothing written, no clock** — and on the handle rather than
    /// among the read verbs for [`session_state`](Engine::session_state)'s reason, which is the
    /// same reason: it asks about this handle's own delivery machinery, not about what was said in
    /// the workspace.
    pub fn held_deliveries(
        &self,
        internal_session: &str,
    ) -> Result<Vec<crate::collecting::HeldRow>> {
        self.handle
            .try_with_state(|s| facade::held_deliveries(&s.store, internal_session))
    }

    /// **What became of a session** (`nxc session state`, nxf 6j6v.h383) — the READ opposite of the
    /// two writes above, on the same seam and about the same rows.
    ///
    /// [`bind_runtime_session`](Engine::bind_runtime_session) records which runtime session a
    /// trigger became; [`session_ended`](Engine::session_ended) records that its process is over.
    /// The second carries the fact a `working_tree: exclusive` channel's advance rests on — *a step
    /// ends when its session ends* — and until this there was no way to ask for either of them
    /// back. What a host was left with was measured rather than imagined: over six hours of
    /// continuous operation in a foreign project, the coordinator of that run resorted to
    /// `ps -p $(cat .nxs/agent-logs/<id>.pid)` — reaching past the seam into `nxc`'s own directory,
    /// for a file whose format is promised nowhere — because it was the only way to tell a thread
    /// that was thinking from a thread that had died. Its own words: *"the pid check saved me from
    /// reading the interim state as a failure yet again"*.
    ///
    /// **A plain read: no [`Caller`], nothing written, no clock.** It needs the store and this
    /// handle's own [`Worker`](crate::worker::Worker), and nothing else — see
    /// [`orchestration::session_state`] for the two sources and the order they are asked in.
    ///
    /// **What a host gets out of it depends on what its worker can answer.**
    /// [`SessionState::Ended`](orchestration::SessionState) comes from the session's own
    /// announcement and is available to everyone.
    /// [`Running`](orchestration::SessionState::Running) comes from
    /// [`Worker::session_is_running`](crate::worker::Worker::session_is_running), whose default is
    /// `false` — so a host whose worker does not implement it reads
    /// [`Unknown`](orchestration::SessionState::Unknown) for every unended session, which is the
    /// honest answer for a runtime nobody can ask, not a wrong one. Which of those two an `Unknown`
    /// is — a session killed hard, or a runtime nobody can ask — is on the report itself since nxf
    /// 6j6v.t41e:
    /// [`SessionStateReport::worker_answers_liveness`](orchestration::SessionStateReport::worker_answers_liveness).
    ///
    /// It is on the handle rather than among the seven read verbs for its two siblings' reason: it
    /// answers about this handle's own session machinery, not about what was said in the workspace.
    pub fn session_state(
        &self,
        scope: orchestration::SessionScope<'_>,
    ) -> Result<orchestration::SessionStateReport> {
        self.handle
            .try_with_state(|s| orchestration::session_state(&s.store, &*self.worker, scope))
    }

    /// **Can this handle's worker answer the liveness question at all?** (nxf 6j6v.t41e) — the
    /// question [`session_state`](Engine::session_state) above cannot be asked before there is a
    /// session, and the one a host has to answer before it acts on a `false`.
    ///
    /// [`Worker::session_is_running`](crate::worker::Worker::session_is_running) has a default of
    /// `false`, so its answer is two answers wearing one word: *nothing is running* and *I never
    /// looked*. Every consequence in this crate that acts on it — the guard every park and reclaim
    /// shares, `orchestration::sessions_still_running`, above all — treats the second as the first,
    /// deliberately and in the fail-open direction, which for THAT guard is the destructive one: a
    /// worker that never implements the method reads every session as gone, so nothing before a
    /// hand-off can ever look alive and the checkout moves to a rival while the holder may still
    /// have hands on the git index.
    ///
    /// **Measured, not supposed.** app-foundations (41j0.4yjx) shipped a stand-in for this from
    /// v0.67.0: it classified the resolved `WorkerConfig` in its own repository, against an enum it
    /// does not own, and stayed silent for `Custom` — the host that brings its own worker and needs
    /// the answer most. This is the worker's own answer instead of a guess about its configuration.
    ///
    /// **What to do with a `false` is the caller's, and that is the whole point of asking.** Warn at
    /// startup, as that server does; refuse a withdrawal by name, the way this crate's own
    /// `withdraw` does since nxf 6j6v.b9nf; or proceed, the way this crate's own gates do. What none
    /// of them could do before was tell which `false` they had been given. A plain read on the
    /// handle's own worker: no [`Caller`](orchestration::Caller), no store, no clock.
    pub fn worker_answers_liveness(&self) -> bool {
        self.worker.answers_liveness()
    }

    // `send` and `reply` were here — the LAST TWO of the four writing verbs. REMOVED by nxf
    // 6j6v.ckeq (owner, 2026-08-21, verb by verb at the source): "`send_to` und `reply_thread`
    // sollen der einzige Weg der Kommunikation sein. Es startet immer auch die jeweilige Sitzung
    // bzw. weckt sie wieder auf." Both had already been off the command line since 6j6v.dvyq §3 and
    // survived only here, which is the shape this cut exists to end: an entrance an app can reach
    // and an agent cannot is a second contract nobody reviews.
    //
    // `send` LEAVES WITHOUT A SUCCESSOR AND IS A NAMED LOSS, and the loss is the point rather than a
    // regret. It was the RAW post: a substrate channel id in, a message written, and nobody woken.
    // That is precisely the act the owner's sentence abolishes — something laid down that nothing
    // picks up. [`send_to`](Engine::send_to) is not a renaming of it: it takes a DECLARED target,
    // and every one of its two branches starts or wakes a session. A caller holding a bare channel
    // id has no path left, deliberately; since 6j6v.dvyq §3 such a channel is not addressable at all
    // (`send_to` refuses one by name), so the entrance was already writing into a namespace the rest
    // of the surface had stopped recognising.
    //
    // `reply` COLLAPSES into [`reply_thread`](Engine::reply_thread), and the collapse is total on
    // the thread half: `reply_thread` calls `orchestration::reply` with the thread as its target, so
    // the quorum-completion routing, the `on_complete` policy, the depth guard and the return-address
    // resume are the same code on the same path. What is LOST is the MESSAGE half — `ReplyRequest`'s
    // `target` also took a message id and resolved it to that message's thread. An app holding a
    // message id must now read its thread (`Engine::thread`/`status` both carry it; the two reads
    // this line named until nxf 6j6v.yr59, `messages` and `thread_board`, went in that cut) and
    // reply into that. Named rather than papered over: it is one read, and the alternative was
    // keeping a second address for one conversation, which is the shape 6j6v.dvyq §3 spent a whole
    // block removing from the CLI.
    //
    // **The BODIES stay**, as for the persona coordinator/`channel_open`/`ask` below:
    // `facade::send` is what
    // every post in this crate is built out of (`orchestration` calls it three times), and
    // `orchestration::reply`/`facade::reply` are what `reply_thread` runs. What left the seam is the
    // flat handle method, not the mechanism — which is also why `tests/seam_disposition.rs` records
    // these two rows as `Stays`: that gate compares NAMES across `engine.rs` + `facade.rs`, and
    // `facade::send`/`facade::reply` are still on it. The rows say so in full.

    // `role_trigger` and `role_resume` were here. REMOVED by nxf 6j6v.dvyq §3 (owner, 2026-08-19),
    // which names both on §5's closed list of what LEAVES `Engine`, together with their CLI half
    // `send --role`/`send --session`.
    //
    // `role_trigger` COLLAPSES into [`send_to`](Engine::send_to): its persona branch runs the very
    // same body, `orchestration::coordinator_commission` (named `role_trigger_with` until nxf
    // 6j6v.ntp9), and adds the two things this method could not
    // give an answer anywhere to go — it stamps a THREAD and tells the summoned persona which one
    // to answer into. Without that, a reply had no address: `reply --thread` is the one reply form
    // §3 leaves standing, and a threadless trigger produces no thread to name.
    //
    // `role_resume` LEAVES WITHOUT A SUCCESSOR, and the record says so rather than pointing at a
    // method that answers a different question. §3's line reads "send --session -> reply --thread",
    // but the two are not the same act: a resume COMMISSIONS an existing session (`ChainMove::
    // Deeper`, registering no obligation), while `reply` ANSWERS one (`ChainMove::Unwind`, through
    // the thread's return address). A caller holding a session id and no thread has no path left.
    // Decided as a named LOSS rather than a collapse (owner, 2026-08-19); the residual case —
    // delivering into a session that is mid-turn — is nxf 6j6v.xr3z's `reply --thread <id> --force`,
    // which waits on a runtime signal that does not exist yet.
    //
    // **The bodies stay**, as they did for `channel_open`/`ask` below:
    // `orchestration::coordinator_commission` and `orchestration::role_resume` are the crate's own
    // verbs, and the first of those beneath
    // the first is what `send_to`'s persona branch runs on every call. What left the seam is the
    // flat handle method an app reached for, not the mechanism.

    // `channel_open` and `ask` were here. REMOVED by nxf 6j6v.dvyq §3, which names both on §5's
    // closed list of what LEAVES `Engine`.
    //
    // They were two flat handle methods onto one mechanism. `ask` routed between a DECLARED channel
    // (fan out to its members under its own policy) and a RAW one (post a board, trigger nobody,
    // and demand an explicit `--expect` because there was no declared policy to read). The raw
    // channels are gone, so the second half has no target and the first half is exactly what
    // [`send_to`](Engine::send_to) does when its target names a channel — one entrance, resolved
    // against the declarations.
    //
    // **The bodies stay.** `orchestration::open_declared_channel_and_fan_out` is what `send_to`'s
    // channel branch runs, and `facade::ask`/`ask_under` — mint the thread, declare its expects,
    // stamp the deadline, post the request — are what that is built out of, and what the
    // consolidation claim uses. What left the seam is the flat method, not the capability.

    // `workflow_start`, `workflow_step_done`, `workflow_tick`, `workflow_liveness`,
    // `workflow_status`, `workflow_list`, `workflow_events`, `workflow_event_cursor` and
    // `subscribe_workflow` stood here. ALL NINE REMOVED by 6j6v.dvyq §3 with the run record they
    // read and wrote — §5's closed list names the first four, and the last three are its acceptance
    // point 4; the two READS (`workflow_status`/`workflow_list`) are not on that list and fell
    // anyway, by owner decision of 2026-08-20 and for a reason the list itself supplies. §5's rule
    // protects a read an APP RENDERS. With `workflow start` gone from both surfaces nothing can
    // ever write a `workflow_runs` row again, so those two would render a table that can never be
    // filled — the rule does not apply to them, rather than being waived for them. The record with
    // the full argument is `tests/seam_disposition.rs`.
    //
    // What an app watching this workspace uses instead: [`subscribe`](Engine::subscribe) for the
    // "someone wrote, re-read" tick, and [`status`](Engine::status) for where an operation stands.

    // ============================================================================================
    // THE TEN READS THAT WERE HERE — nxf 6j6v.yr59, owner, 2026-08-21.
    // ============================================================================================
    //
    // Sixteen read verbs stood on this handle; seven carry the work — the six the item chose, plus
    // `search`, which this build removed and the owner PUT BACK on 2026-08-21 (the argument is on
    // [`search`](Engine::search), and the closing note of this block says what it means for the
    // record below). Each removal below is a COLLAPSE (the question is answered elsewhere, in full)
    // or a NAMED LOSS (it is not, and saying so is the point). `open_with_poll_interval` went with
    // them and is `EngineConfig::poll_interval`.
    //
    // THE BODIES BELOW `facade::` MOSTLY STAY, and that is not a hedge: `nxc threads list`,
    // `nxc threads show`, `nxc prime` and `nxc transcript show` are all still verbs a human or an
    // agent types, and `verb_seam.rs` requires their compute half to exist. (`nxc inbox` stood in
    // that list until 6j6v.1gm9 removed the verb, and `facade::inbox` outlived it because `prime`
    // read it — until nxf 6j6v.4d2z removed the unread set itself, and with it both halves.) So
    // `tests/seam_disposition.rs` records most of these rows as `Stays` — that gate compares
    // symbol NAMES across `engine.rs` + `facade.rs` — with the reason spelling out that what left
    // is the FLAT HANDLE METHOD an app reached for. `ask`/`send`/`reply` set that precedent; these
    // follow it. The one gate that sees the handle alone is `tests/read_surface.rs`.
    //
    // ---- pure overloads: nothing was decided, because there was nothing to decide -------------
    //
    // `prime` COLLAPSES into [`prime_as`](Engine::prime_as): it was literally
    // `self.prime_as(consumer, None, now)`, one line, and the `None` is the human at the keyboard.
    //
    // `transcript` COLLAPSES into [`transcript_page`](Engine::transcript_page):
    // `transcript_page(s, -1, None)` is the same value, which the removed method's own doc already
    // said ("`transcript` itself now delegates to it"). The unbounded read is still reachable, by
    // asking for it — which is the honest form, since what it materialises is a whole session with
    // its unbounded `tool_use` inputs.
    //
    // ---- decided individually, as the item required -------------------------------------------
    //
    // `public_channels` is ABSORBED by [`directory`](Engine::directory) — owner: "`directory`
    // zeigt uebrigens auch die Kanaele an und wofuer sie da sind." It was the workspace's DISCOVERY
    // read (6j6v.bd6g), the one thing `list` could not answer for itself, because a front door
    // reaching this workspace by SYNC carries no declaration here to read. That was the exact
    // argument on which the owner kept it as a read of its own on 2026-08-19, and yr59 overrules it
    // by making the directory carry the store read rather than by dropping the question. ONE thing
    // did not come along and is named on `persona::FrontDoor`: the per-caller `unread`, which is
    // read state rather than addressability, and which was `0` for the discovering audience anyway.
    // nxf 6j6v.4d2z has since settled that everywhere — there is no unread count on any record.
    //
    // `channels` LEAVES, and it is half a collapse and half a named loss. Half one: "which channels
    // are there, and what for" is [`directory`](Engine::directory), which reads the DECLARATION —
    // the source of truth for membership since 6j6v.dvyq. Half two: the per-channel UNREAD COUNTS
    // have no successor. The item put this exactly there ("bleibt nur, wenn eine App
    // Ungelesen-Zaehler je Kanal zeigen will"), and the answer is no: after 6j6v.dvyq the surface is
    // THREAD-shaped — `send_to` returns a thread, `reply_thread` takes one, `status` is a tree of
    // them — so an unread badge on a channel is a badge on the one thing an app can no longer
    // address. The ACK stayed at the time, reachable through a `MessageView` from
    // [`thread`](Engine::thread), which carries both the `channel_id` and the `message_id` it took;
    // nxf 6j6v.4d2z has since removed it, so the named loss is the whole of it and the counts have
    // left `facade::channels` too.
    //
    // `inbox` LEAVES. Owner, 2026-08-21: "`inbox` gibt es eigentlich nicht [mehr]." A persona gets
    // what it needs at session start and on resume, and that was one call at the time:
    // [`prime_as`](Engine::prime_as) carried the catch-up already SPLIT by disposition, which is
    // the whole of what `--all` distinguished. The verb was kept for a HUMAN — "a product question,
    // not a seam one" — and 6j6v.1gm9 answered the product question: a human reads a CONVERSATION
    // (`nxc threads show`, `nxc status`), not a flat unread list of one, so the verb is `Gone` in
    // the disposition record now too. THE RESIDUAL this named — an app that wants the unread list
    // WITHOUT priming has to prime — has no site left either: nxf 6j6v.4d2z removed the catch-up
    // from `prime_as` and the derivation under it, because all three delivery paths PUSH and a
    // second, pull-shaped copy of what already arrived is what the whole direction was clearing
    // away.
    //
    // `opener_wake` COLLAPSES into [`prime_as`](Engine::prime_as), and by its own doc: it was "the
    // SAME derivation `nxc inbox`/`prime` renders as 'Threads you opened'" — both of those
    // renderings are themselves gone since 6j6v.1gm9 (and `nxc inbox`'s compute half since
    // 6j6v.4d2z), and `PrimeReport` carries the derivation
    // verbatim as `threads_you_opened` for an app. [`status`](Engine::status) answers the same
    // question the other way round, per operation. Two names for one derivation is precisely what
    // this item is about.
    //
    // `threads` COLLAPSES into [`status`](Engine::status), on the item's own condition — "sobald
    // `status` je Faden genug traegt" — which is why `StatusThread` grew `deadline` and the two
    // working-tree fields in the same change rather than later. The evidence the item supplied is
    // that app-foundations' `ChatClient` names it nowhere at all. What is deliberately NOT carried
    // over is its MEMBERSHIP SCOPING: `status` is not membership-scoped and must not be, because an
    // operation crosses channels its reader is not in (6j6v.a71h §3.2), so the two were never the
    // same list — the fold keeps the richer question and drops the narrower one.
    //
    // `thread_board` COLLAPSES into [`status`](Engine::status) + [`thread`](Engine::thread): the
    // quorum half is a `StatusThread`, the message half is the one message reader. TWO calls where
    // there was one, said out loud as the cost it is. What made this safe rather than merely
    // cheaper is that the `visibility` policy moved WITH the messages: `thread` resolves the
    // channel's declared policy and applies it, so a `requester_only` board is not readable in full
    // through the survivor. Had it not, this "consolidation" would have widened a read.
    //
    // `thread_quorums` BECOMES A PARAMETER of its neighbour: [`StatusScope::Threads`]. The item's
    // ruling was "die Anti-N+1-Eigenschaft muss bleiben, der zweite Name nicht", and the property
    // is now measured rather than asserted — `tests/bulk_quorum.rs` counts the SELECT statements a
    // two-thread read and a forty-thread read prepare and fails if the number moves.
    //
    // `messages` LOSES to [`thread`](Engine::thread) — the item's "beide zu behalten ist keine
    // Antwort", decided here. A thread id is the value `send_to` returns, the only value
    // `reply_thread` takes and what `status` speaks in; a channel id is a value no surviving verb
    // hands out. And since 6j6v.pf6j a channel holds the requester's board and every member's own
    // thread side by side, so "the messages in this channel" is several conversations interleaved
    // where "the messages in this thread" is one. WHAT IS LOST: reading a whole channel's flat
    // history in one call — a public front door's, for instance. The route is
    // [`status`](Engine::status) with `StatusScope::Channel` for the thread ids, then this reader
    // per thread; the loss is the SINGLE CALL, not the capability.
    //
    // ---- and the one that did NOT leave, after all --------------------------------------------
    //
    // `search` STOOD IN THIS BLOCK AS AN ELEVENTH REMOVAL AND DOES NOT ANY MORE. The argument for
    // cutting it was the one every entry above is built on and it is not withdrawn: the item covers
    // `search` nowhere, it was on the surface, so this build decided it in the item's own terms —
    // it answers none of the six questions, it is discovery-by-text, and its scoping is the
    // channel-shaped one `channels`/`inbox` are leaving for. The removal was reported as a NAMED
    // LOSS together with the gap in the item, not slipped in.
    //
    // THE OWNER OVERRULED IT ON 2026-08-21, and the deciding fact is not about shape:
    // app-foundations' `ChatClient` consumes `search` today — one of the 14 methods of that
    // interface that still exist on this seam, not one of the 14 that do not. This item's whole
    // purpose is to cost that consumer ONE migration instead of two, so breaking a live method
    // inside it is the failure it exists to prevent. `Engine::search` is restored unchanged, the
    // read seam is SEVEN verbs, and every removal recorded above stands exactly as it did.
    //
    // The lesson worth keeping for the next cut, since it is what the sixteen-to-six frame missed:
    // "answers none of the six questions" is an argument about the SHAPE of a surface, and the
    // consumer's measured usage outranks it. `tests/seam_disposition.rs`'s `search` row carries
    // both halves side by side, and `tests/read_surface.rs` counts seven.
    // ============================================================================================
}
