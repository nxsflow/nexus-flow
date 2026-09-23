//! The worker seam (spec §5). `DryWorker` records the trigger for deterministic tests; `SidecarWorker`
//! spawns the real Node sidecar detached. `send`/`reply` summon a role through `Worker::trigger`.
//!
//! **The worker is a long-lived seam** (nxf 6j6v.a5na). It used to be rebuilt per trigger, because
//! the per-hop env stamps (`NXC_ORIGIN`/`NXC_DB`/`NXC_HOP`) were baked into `SidecarWorker` itself.
//! Those moved onto [`TriggerRequest::env`], so a worker is now constructible once and held for the
//! lifetime of whoever owns it — which is what lets [`crate::engine::Engine`] hold one for the
//! app's lifetime instead of reaching for process env at every call.
use crate::error::{NxfError, Result};
use crate::role::Model;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleSpec {
    pub handle: String,
    pub system_prompt: String,
    pub use_claude_code_preset: bool,
    /// Mirrors `role::RoleDecl.tools` (fix round on 6j6v.zenf's final review): `None` when the
    /// role declared no `tools:` at all — the sidecar spec's `tools` key then serializes to JSON
    /// `null`/absent, so `agent-sidecar/src/main.mjs` leaves `options.tools` unset and the SDK's
    /// default full toolset applies. `Some(vec![])` is an explicit "zero tools" declaration,
    /// which reaches the spec as a real empty array and disables the base toolset entirely. A
    /// plain `Vec<String>` here could never represent the first case (it always serializes, even
    /// empty), which is exactly the bug this type fixes.
    pub tools: Option<Vec<String>>,
    /// **What the ENGINE grants this session because it REQUIRES something of it** (nxf 6j6v.kffm)
    /// — separate from [`tools`](RoleSpec::tools), which is what the role's AUTHOR declared.
    ///
    /// The pairing rule this field exists for: *whoever puts a role under an obligation has to make
    /// sure it has the means to discharge it.* A trigger that registers an expectation tells the
    /// session, in its own system prompt, to end its turn with `nxc reply --thread <id>` — and until
    /// this field existed nothing made sure it could run that command. A role that declares no
    /// `tools:` at all (the default, and what every role written before the role runtime does) got
    /// the SDK's full base toolset and an EMPTY auto-approval list, so its own `nxc reply` came back
    /// "This command requires approval" and the teardown then posted a `sidecar:` failure in its
    /// name. From the outside the round looked answered.
    ///
    /// **Two lists, because the sidecar's one `tools` key was doing two jobs.** In the SDK,
    /// `options.tools` is the base toolset and `allowedTools` is the auto-approval list within it,
    /// and both were derived from `tools` alone — so granting the means by widening `tools` would
    /// have SHRUNK an undeclared role's base toolset from "everything" to "Bash". The grant is
    /// therefore its own field, and the sidecar unions it into the two lists differently: into the
    /// approval list always, and into the base toolset only where the role declared one — so an
    /// undeclared role keeps the full default set it always had.
    ///
    /// **Set at the ONE funnel** ([`crate::orchestration`]'s `trigger_and_bind`), never per call
    /// site. The per-call-site repair is what nxf 6j6v.04es did for the `summarize` synthesizer, and
    /// the defect coming back at the next spawn path is what says that repair sat at the wrong
    /// height.
    ///
    /// Empty for every trigger that imposes nothing — which is what a `RoleSpec` built by a host
    /// gets by default, and it changes nothing for it.
    pub granted_tools: Vec<String>,
    pub permissions: Option<String>,
    /// Which model this session runs on, already resolved from the role/step/call precedence
    /// (`crate::role::Model`). `None` — the role declared no preference — reaches the spec JSON as
    /// an absent key, so `agent-sidecar/src/main.mjs` leaves `options.model` unset and the SDK's
    /// own default model applies. Exactly the `tools` discipline above, for exactly the same
    /// reason: a `""` or a hardcoded fallback here would silently pin every undeclared role to one
    /// model, a decision no role author made.
    pub model: Option<Model>,
    /// **Which version of its declaration this session's prompt was composed from** (nxf
    /// 6j6v.pkw9) — [`crate::declaration_version::role_declaration_hash`] of the [`RoleDecl`] the
    /// funnel resolved, reaching the spec JSON as `declarationHash`.
    ///
    /// [`RoleDecl`]: crate::role::RoleDecl
    ///
    /// **It makes one question answerable that was not.** `.nxs-personas/` is runtime configuration
    /// in the working copy the declared agents themselves edit, so a branch switch can roll the
    /// rules back with nothing reported anywhere. When that happened in the proving ground the only
    /// way to establish that a session had run without a rule was to probe the stored
    /// `systemPrompt` for its text; with this key, "did this session run under the declaration I
    /// wrote?" is one comparison.
    ///
    /// **A record, not a control** — the same standing `coordinator` has. The sidecar does not read
    /// it, nothing branches on it, and an older sidecar ignoring a key it does not know is exactly
    /// as correct as one that stores it.
    ///
    /// `None` for a spawn that HAS no declaration, which is the `tools`/`model` discipline above
    /// applied to the one case that needs it: the `summarize` synthesizer is not a declared role —
    /// it is composed from the channel's own `summary_prompt` and reaches this seam without passing
    /// the funnel at all. A hash there would identify nothing, and an empty string would be a value
    /// that looks like one.
    pub declaration_hash: Option<String>,
}

/// **Which named coordinator admitted this spawn** (nxf 6j6v.ntp9).
///
/// A message never reaches a role directly — it reaches a COORDINATOR, and the coordinator decides
/// what happens with it. That held for the channel path (`supervisor_*`, `orchestration.rs`) and did
/// not hold for the persona path, which went from `send --to <persona>` straight into
/// [`crate::orchestration::trigger_role`] with nobody in between. This field is what turns "every
/// role is started by a coordinator" from a sentence in a doc comment into something a test can read
/// off the one seam every spawn passes through.
///
/// It is a REQUIRED field of [`TriggerRequest`], and that is the structural half: a NINTH spawn path
/// added later cannot compile until its author names the place that admitted it. The evidential half
/// is `crates/chat/tests/no_role_starts_without_a_coordinator.rs`, which drives every entrance a
/// role can be started from and reads this value at the worker.
///
/// `#[non_exhaustive]` for the reason [`TriggerError`] is: a fifth coordinator later must be an
/// additive minor for an embedding host that matches on this, never a break.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Coordinator {
    /// [`crate::orchestration::coordinator_commission`] — a task addressed to ONE declared persona,
    /// which is what `nxc send --to <persona>` and [`crate::engine::Engine::send_to`] run.
    Persona,
    /// The channel supervisor (`supervisor_*`): a step of a declared flow, a member handed its next
    /// turn, and the `summarize` consolidation's ephemeral synthesizer — which is not a declared
    /// role and so is the one spawn that reaches the worker without passing
    /// [`crate::orchestration::trigger_role`] at all. It is still the channel's coordinator that
    /// starts it, which is exactly what this value says.
    Channel,
    /// An answer travelling back UP a chain that some coordinator already admitted
    /// (`ChainMove::Unwind`): a completed quorum handed to the board's opener, a reply resuming its
    /// return address. Nothing new is commissioned here — the coordinator that commissioned this
    /// work is one level up, and this is its result arriving.
    Return,
    /// A commission that WAS admitted and then parked behind the working copy, released now
    /// (`fire_queued_trigger`). The admission happened at the coordinator that queued it; the
    /// release path is the named place that lets it through, and re-derives the declaration and the
    /// obligation rather than replaying a frozen decision.
    Released,
}

impl Coordinator {
    /// The stable lower-case token this value renders as — in [`DryWorker`]'s record, and identical
    /// to the JSON [`serde::Serialize`] writes into the sidecar spec, so a black-box test and a
    /// library test read the same word for the same coordinator.
    pub fn as_str(self) -> &'static str {
        match self {
            Coordinator::Persona => "persona",
            Coordinator::Channel => "channel",
            Coordinator::Return => "return",
            Coordinator::Released => "released",
        }
    }
}

/// **The terms of the turn, stated by the coordinator that commissions it** (nxf 6j6v.ntp9,
/// answering nxf 6j6v.553s question (b)).
///
/// A session can fail to answer its thread in two ways that look alike from the outside and are not
/// alike at all, and only the RUNTIME can tell them apart:
///
/// ```text
/// the agent ended its turn without answering  -> remind it, and give the turn back
/// the runtime never ran the agent at all      -> retry, with backoff; nothing was omitted
/// ```
///
/// A `529` from the model runtime is the second. Reminding there asserts an omission that did not
/// happen and spends the one attempt nxf 6j6v.gh7f has, on a session that never got to think. So the
/// worker decides WHICH case it is — it is the only party that can — and the coordinator decides
/// what happens in each, up front, when it hands the task over. **Up front is the load-bearing
/// half**: the numbers live in the engine, travel with the trigger, and are therefore INHERITED by
/// any worker, including a host's own [`WorkerConfig::Custom`] (nxf 41j0.vhsk). A constant in
/// `agent-sidecar/src/main.mjs` would be a rule that holds on the bundled path and nowhere else.
///
/// **Both bounds exist because neither loop terminates on its own**: a reminded session can end
/// without answering again, and a runtime that is down stays down. What happens AFTER either bound
/// is the same in both cases and is the decision that matters more than the numbers — the round is
/// escalated to a human, through the `reply --thread <id> --if-unanswered --escalate` the sidecar
/// already posts, with the text naming which bound was reached.
///
/// `#[non_exhaustive]` for the reason [`Coordinator`], [`TriggerError`] and [`WorkerConfig`] all
/// carry it (independent review of PR #378, Code Quality #5): a FOURTH bound later must be an
/// additive minor for an embedding host, not a source break. Nothing outside this crate needs to
/// construct one (a host installs a [`Worker`], which READS these off the request), but a literal
/// was constructible until this marker, and `crates/chat/tests/worker.rs` proved it by doing so.
/// [`TurnTerms::new`] is what an in-crate caller that genuinely wants other terms uses instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TurnTerms {
    /// How many times a session that ENDED ITS TURN still owing its thread an answer may be resumed
    /// and reminded of the answering rule before the runtime speaks in its place (nxf 6j6v.gh7f).
    ///
    /// One. The ceiling is the point rather than the number — the ticket's own words were "ein
    /// Versuch, hoechstens zwei" — and the measured failure is not a session that was never told but
    /// one that was told and missed it, so a second telling buys little for a second paid turn.
    pub reply_reminders: u32,
    /// How many times a run whose RUNTIME never produced a single message may be started again.
    ///
    /// **Only that shape**, and the narrowness is the whole safety argument: a stream that tore
    /// MID-turn is a turn that already did work, and an agent turn is not idempotent — the files,
    /// the messages, the pull requests all happen a second time. "The agent never got to think" is
    /// the one state in which starting over costs nothing, and it is exactly the `529`-at-the-door
    /// case this exists for. A mid-turn tear is escalated instead, unchanged.
    pub runtime_retries: u32,
    /// The delay before the FIRST retry, doubled for each one after it — so two retries wait 5s and
    /// 10s, and the whole bounded attempt adds at most 15s to a run that would otherwise have died
    /// at the first `529`. Bounded on purpose: the retry holds the working copy while it waits, and
    /// a long outage must reach the human rather than be sat out.
    ///
    /// **The bundled runtime clamps both this and [`runtime_retries`](TurnTerms::runtime_retries)
    /// at its own ceiling** (`agent-sidecar/src/spec-helpers.mjs`: `MAX_RETRY_DELAY_MS`,
    /// `MAX_RUNTIME_RETRIES`), and that is not a disagreement about who decides: it is the
    /// runtime
    /// refusing to hold a checkout indefinitely on any number it is handed. Node collapses a
    /// `setTimeout` above `2**31 - 1` to 1 ms, so an unclamped doubling backoff stops backing off
    /// altogether. Every value shipped from here is far inside those ceilings.
    pub retry_backoff_ms: u64,
}

impl TurnTerms {
    /// Terms other than [the coordinator's](TurnTerms::default) — the constructor
    /// `#[non_exhaustive]` leaves in place of a struct literal. Adding a fourth bound gives it a
    /// default here rather than a new parameter, which is the whole point of the marker.
    pub fn new(reply_reminders: u32, runtime_retries: u32, retry_backoff_ms: u64) -> Self {
        TurnTerms {
            reply_reminders,
            runtime_retries,
            retry_backoff_ms,
        }
    }
}

impl Default for TurnTerms {
    /// **The one definition of both bounds.** The coordinator states these on every commission; no
    /// other place decides them, and no declaration overrides them yet — making them declarable is a
    /// surface question (which file, which key, what a channel says versus a persona) and belongs
    /// with the declaration catalogue, not here.
    fn default() -> Self {
        TurnTerms {
            reply_reminders: 1,
            runtime_retries: 2,
            retry_backoff_ms: 5_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerRequest {
    pub internal_session: String,
    pub resume_real: Option<String>,
    pub message: String,
    pub role: RoleSpec,
    /// **The named place that admitted this spawn** — see [`Coordinator`] for why this is a required
    /// field and what a test at this seam does with it.
    pub coordinator: Coordinator,
    /// **What happens if this turn does not answer its thread** — decided by the coordinator, at the
    /// moment it commissions the turn, so every worker inherits the same answer. See [`TurnTerms`].
    pub terms: TurnTerms,
    /// The env stamps this particular trigger contributes to the spawned session — the ambient
    /// `NXC_ORIGIN`/`NXC_DB` plus `NXC_HOP` incremented for the next hop (see `cli.rs`'s
    /// `trigger_env`). Per-TRIGGER, not per-worker: the hop counter advances with every hop of a
    /// spawn chain, so baking it into the worker (as this used to) is what forced a fresh worker
    /// per call and kept the seam un-holdable by a long-lived handle.
    pub env: Vec<(String, String)>,
    /// The thread THIS trigger's caller declared, on its own thread, that it expects a reply from
    /// the spawned session on — carried straight through from
    /// [`crate::orchestration::RoleSpawn::reply_thread`] by [`crate::orchestration::trigger_role`],
    /// the one funnel every declared-role trigger passes through. Reaches the sidecar's spec JSON
    /// as `replyThread` (nxf 6j6v.7e9d, ticket 8): `Some` is what lets the sidecar's teardown settle
    /// the debt with `nxc reply --thread <id> --if-unanswered <text>` if the session ends without
    /// ever answering; `None` — every caller except the one branch of `coordinator_commission` that
    /// actually wrote `expects_reply_from` — makes that teardown step a no-op, because there is
    /// nothing owed to settle.
    pub reply_thread: Option<String>,
}

/// What a [`Worker`] reports back about the session a trigger was supposed to start (nxf
/// 6j6v.5x9j).
///
/// The trait used to return `Result<()>` — "it worked" or "it didn't" — which quietly assumed that
/// whoever implements it KNOWS, by the time `trigger` returns, whether a session exists. That holds
/// for a local process spawn and for nothing else. This type is what replaces the assumption with a
/// statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerOutcome {
    /// The worker took the trigger; the runtime's own session id is **not known yet**, and the
    /// binding will be completed out of band.
    ///
    /// This is not a weaker `Started` — it is the normal answer for both bundled workers.
    /// [`SidecarWorker`] spawns `node` detached and returns immediately; the SDK session binds
    /// itself from inside, later, through `nxc session bind`. A REMOTE runtime gives the same
    /// answer for the same reason: its provisioning call is a network round-trip that has not
    /// resolved yet, and it completes the binding from its own executor via
    /// [`crate::engine::Engine::bind_runtime_session`] when it does.
    Accepted,
    /// The runtime session already exists and this is its **opaque** id.
    ///
    /// Requirement 3 of the AgentCore probe, in the only place it can be expressed: there is no
    /// process, no pid and no termination signal to hand back — just an identifier whose format
    /// belongs entirely to the runtime that minted it. [`crate::orchestration::trigger_role`] binds
    /// it to the internal session and never parses it.
    Started {
        /// The runtime's own session identifier, verbatim.
        runtime_session: String,
    },
}

/// Why a trigger did not start a session (nxf 6j6v.5x9j).
///
/// **[`SessionGone`](TriggerError::SessionGone) is the point of this type.** A cloud agent runtime
/// reaps a session SILENTLY — AgentCore at 15 minutes idle or 8 hours total — so "resume the
/// session you were told about" fails routinely, in a way that is neither the caller's mistake nor
/// a broken worker, and that calls for a different response than any other failure: start fresh,
/// or escalate, rather than retry. A single opaque error type cannot say that without the caller
/// string-matching a message, which is exactly what the probe's finding rules out.
///
/// `#[non_exhaustive]` for the same reason [`crate::role::Model`] and
/// [`crate::orchestration::WakeSkipReason`] are: a third case later must be an additive minor, not a
/// break for every downstream `match`.
#[derive(Debug)]
#[non_exhaustive]
pub enum TriggerError {
    /// The runtime session named by [`TriggerRequest::resume_real`] no longer exists — reaped for
    /// idleness, past its maximum lifetime, or otherwise gone without anyone being told.
    SessionGone {
        /// The runtime session id that was asked to resume, verbatim as the worker was given it.
        session: String,
        /// What the runtime said, for a human reading a breadcrumb or an app rendering a detail.
        detail: String,
    },
    /// **A process for this internal session is ALREADY ALIVE** (nxf 6j6v.7qtf), so nothing was
    /// started — the refusal IS the fix, not a failure to carry it out.
    ///
    /// One internal session means one process. A wake that arrives while the session it names is
    /// still working used to be answered with a SECOND spawn: two `claude` processes under one
    /// session id, in one working directory, overwriting each other's files — and then a third and
    /// a fourth, because every new process produces events that wake again. Measured live on nxs
    /// 0.62.0: three sidecars on one spec, four twenty seconds later, ~20 `claude` processes on the
    /// machine, and a coder whose own edits vanished under a sibling it could not see.
    ///
    /// **The working-tree lease cannot catch this and never could**: it separates CHAINS, and two
    /// processes of one session are the same chain — they inherit the same claim and both run.
    ///
    /// Its own arm rather than a [`Failed`](TriggerError::Failed), because the two call for
    /// opposite responses, exactly as [`SessionGone`](TriggerError::SessionGone) does: a failed
    /// spawn is worth retrying, and this one must never be retried while it holds — the session is
    /// working, and the message the caller just posted is durably in its thread either way.
    AlreadyRunning {
        /// The internal session that already has a process.
        session: String,
        /// The live pid holding it, so a human can go and look at it (`ps`, `kill`) and an app can
        /// name it.
        pid: u32,
    },
    /// Everything else: the spawn/provisioning call itself failed, the configuration refuses to
    /// spawn at all, the request was rejected. Carries the error unchanged — this arm reclassifies
    /// nothing.
    Failed(NxfError),
}

impl TriggerError {
    /// Whether this is the vanished-session case, without matching on a `#[non_exhaustive]` enum
    /// from outside the crate.
    pub fn is_session_gone(&self) -> bool {
        matches!(self, TriggerError::SessionGone { .. })
    }

    /// Whether the session already had a live process, so nothing was started and nothing should be
    /// retried — [`is_session_gone`](TriggerError::is_session_gone)'s twin, for the same reason.
    pub fn is_already_running(&self) -> bool {
        matches!(self, TriggerError::AlreadyRunning { .. })
    }
}

impl std::fmt::Display for TriggerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TriggerError::SessionGone { session, detail } => {
                write!(f, "runtime session {session} is gone: {detail}")
            }
            TriggerError::AlreadyRunning { session, pid } => write!(
                f,
                "session {session} is already running as pid {pid}: not started a second time \
                 (one internal session, one process)"
            ),
            TriggerError::Failed(inner) => write!(f, "{inner}"),
        }
    }
}

impl std::error::Error for TriggerError {}

/// Into the shared envelope, so a `trigger` failure still travels through any
/// `Result<_, NxfError>` with a plain `?` at the many call sites that have nothing to decide.
/// `SessionGone` becomes `not_found` — the named session does not exist, which is what that kind
/// means everywhere else in this codebase — and keeps both of its fields in the message, since the
/// envelope has no room for structure. Callers that DO have something to decide match on
/// [`TriggerError`] before converting.
impl From<TriggerError> for NxfError {
    fn from(e: TriggerError) -> NxfError {
        match e {
            TriggerError::SessionGone { .. } => NxfError::not_found(e.to_string()),
            // `validation`: the call itself is the thing that is wrong — a second start for a
            // session that is already working — not the environment failing to carry it out.
            TriggerError::AlreadyRunning { .. } => NxfError::validation(e.to_string()),
            TriggerError::Failed(inner) => inner,
        }
    }
}

/// So a worker implementation can keep using `?` on the ordinary fallible calls in its body (file
/// writes, spawns) and only reach for [`TriggerError::SessionGone`] deliberately.
impl From<NxfError> for TriggerError {
    fn from(e: NxfError) -> TriggerError {
        TriggerError::Failed(e)
    }
}

/// What [`Worker::trigger`] returns. Its own `Result` alias rather than the crate-wide one, because
/// its error side is [`TriggerError`], not [`NxfError`].
pub type TriggerResult = std::result::Result<TriggerOutcome, TriggerError>;

/// **What a [`Worker::stop_session`] request turned out to be** (nxf 6j6v.b9nf, fix round 3 of that
/// item's review, Code Quality #6).
///
/// It exists because "there is nothing left to stop" is the GOAL state and was being reported as a
/// failure. A round about to end on its own while somebody withdraws it is the ordinary race on this
/// path — the caller reads liveness, decides, and signals, and the process may leave in between —
/// and an `Err` there made `nxc withdraw` print "the process is still there; stop it by hand" about
/// a process that had gone, and exit 1 on the outcome it was asked for.
///
/// So the seam has two successes and one failure. `Err` is now only what it says: the session may
/// still be running and this call did not end it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionStop {
    /// **The request was delivered** — and only that. The process runs its own teardown in its own
    /// time, so a caller that has to know it is gone watches
    /// [`Worker::session_is_running`](Worker::session_is_running), exactly as before.
    Requested,
    /// **There was nothing to signal, and that is what was wanted.** The session had already ended
    /// — its pid is gone, or the pid the claim names now belongs to an unrelated process and the
    /// session behind it is therefore over ([`SidecarWorker::stop_session`] refuses to touch a
    /// stranger). The string is a sentence for a person saying which of those it found; a caller
    /// puts it on its receipt rather than in a warning.
    NothingToStop(String),
}

/// How a session is started. **This is the seam a host owns entirely** — the engine decides WHAT a
/// role is and WHEN it runs; who executes it, and where, is the host's (spec §3.3, the IP boundary).
///
/// `Send + Sync` because whoever owns a worker may hold it across threads —
/// [`crate::engine::Engine`] keeps one for the app's lifetime and must itself stay `Send + Sync` to
/// live in a Tauri backend's managed state.
///
/// # Why this stays a `fn`, not an `async fn` (nxf 6j6v.5x9j)
///
/// The AgentCore probe's first requirement reads "async: provisioning and teardown become a NETWORK
/// call, not a local process start — a synchronous signature does not carry". What actually did not
/// carry was the `Result<()>` return: it forced the answer to be known before `trigger` returned,
/// so a remote host had only bad options — block a network round-trip on whatever thread the engine
/// happens to be on (inside an async runtime, `block_on` does not merely stall, it panics), or lie
/// about an outcome it does not have yet.
///
/// **Submission is synchronous; completion is reported out of band.** That is what
/// [`TriggerOutcome::Accepted`] means, and it is not a workaround invented here — it is what
/// [`SidecarWorker`] has always done (spawn detached, return, let the session bind itself later).
/// A remote host implements `trigger` as "hand the provisioning call to my own executor and
/// return", then calls [`crate::engine::Engine::bind_runtime_session`] with the opaque id when the
/// call resolves. Nothing blocks, and no `.await` appears anywhere on the path.
///
/// The alternative — an `async fn trigger` — would make every orchestration verb async, and through
/// them every `Engine` verb, which is the whole embedding surface app-foundations links against.
/// That is a break across three projects for a seam that is fire-and-forget by contract anyway
/// (spec §5.1). It is not this ticket's to take unilaterally; see §3.3 of the spec.
///
/// **Teardown exists now, and it is synchronous for `trigger`'s own reason.** The probe names
/// provisioning *and* teardown as network calls, and until nxf 6j6v.b9nf there was no `reap`/`stop`
/// here because the engine had no call site that would use one: nothing in the runtime ended a
/// session, sessions ended themselves — a trait method with no caller would have been a guess at a
/// shape. `withdraw` on a RUNNING round is that call site now, [`stop_session`](Worker::stop_session)
/// is the write it needed, and it was built to exactly the shape argued above: SUBMISSION is
/// synchronous and completion is reported out of band — through
/// [`session_is_running`](Worker::session_is_running) turning `false`, not through a return value a
/// caller would have to block on. [`stop_session`](Worker::stop_session)'s own doc says why waiting
/// for the exit here would deadlock the very call that is asking.
///
/// # Two rules a host implementation MUST follow (PR #269 review, Integrity #1)
///
/// Both exist because of where this method is CALLED FROM: [`crate::engine::Engine`] holds its
/// single store mutex for the whole orchestration verb, and `trigger` runs inside it. The engine
/// bounds a breach rather than trusting the contract ([`CUSTOM_WORKER_BOUND`]), but a breach still
/// costs the whole handle real time, so:
///
/// 1. **Return promptly. Do not block on I/O you do not control.** Hand a network call to your own
///    executor and answer [`TriggerOutcome::Accepted`]; report the id later through
///    [`crate::engine::Engine::bind_runtime_session`]. A `trigger` that waits out a provisioning
///    round-trip stalls every verb on every clone of the handle for as long as it waits.
/// 2. **Do not call back into the `Engine` from inside `trigger`.** The store mutex is held and is
///    not reentrant, so `bind_runtime_session` from in here blocks until this call gives up. If the
///    id is already in hand, **return [`TriggerOutcome::Started`]** and let the engine bind it —
///    that is what the variant is for, and it is why no host ever needs the callback synchronously.
pub trait Worker: Send + Sync {
    /// Start (or resume) the session `req` describes. Fire-and-forget by contract: this returns as
    /// soon as the trigger has been HANDED OVER, never once the session has finished anything.
    fn trigger(&self, req: TriggerRequest) -> TriggerResult;

    /// **Is the process behind `internal_session` still running?** (nxf 6j6v.10yb) — a READ, the
    /// first of the two questions the engine asks a worker about a session it already started; the
    /// second is [`stop_session`](Worker::stop_session)'s WRITE, added by nxf 6j6v.b9nf so
    /// `withdraw` can ask "and now stop it".
    ///
    /// It does not itself end a session or ask a worker to — that is
    /// [`stop_session`](Worker::stop_session)'s job, and this stays the plain read beside it rather
    /// than growing a side effect of its own. The engine needs it because a sequential channel used
    /// to advance on a MESSAGE — "the member answered" — while the process that answered kept
    /// writing into the same working copy for another eighteen minutes. Whether that process exists
    /// is not a fact any store holds; it belongs to whoever executes sessions, which is exactly this
    /// trait.
    ///
    /// **The default is `false`, and the direction is deliberate.** A worker that cannot tell
    /// answers "not running", so the engine behaves exactly as it did before this method existed: a
    /// flow advances on the answer. Answering `true` by default would let one unknowable session
    /// stall a channel with nothing able to release it, which is the strictly worse failure. A host
    /// worker that CAN tell should say so — it is what stops two of its own sessions writing into
    /// one tree — and should say so TWICE, because since nxf 6j6v.t41e there is a second question
    /// that carries the half this answer cannot:
    /// [`answers_liveness`](Worker::answers_liveness), which separates the `false` that means
    /// "nothing is running" from the `false` that means "I never looked".
    ///
    /// It is the BACKSTOP rather than the primary signal: a session announces its own end through
    /// `nxc session ended` (`session_map.ended`), which is what covers the moment this method cannot
    /// — the announcement is made by the ending session, whose process is necessarily still alive
    /// while it speaks. This answers for the session that was killed hard and never announced
    /// anything.
    ///
    /// **It is asked when something looks, and something now looks ON A CLOCK** (nxf 6j6v.858n,
    /// which closed what the review of PR #361 opened). A member that has replied is `complete`, so
    /// the member-window re-arm does not count it among the deadlines it schedules a job for — on
    /// one member per step there is no candidate at all, and until 858n that meant nothing ever
    /// asked again. A decline whose only remaining blocker is a live session now schedules its own
    /// re-check
    /// ([`SESSION_LIVENESS_RECHECK_SECS`](crate::orchestration::SESSION_LIVENESS_RECHECK_SECS)
    /// later), and keeps scheduling one for as long as this method keeps answering `true`.
    ///
    /// So a worker that answers `true` forever still holds the flow forever — the clock cannot fix
    /// a wrong answer, only a missing look. Answer promptly and truthfully; a `true` you cannot
    /// retract is a stall nothing else will clear.
    ///
    /// Must return PROMPTLY and must not call back into the engine — [`Worker`]'s two rules, for
    /// their reason: this is called from inside the engine's held store mutex, like `trigger`.
    fn session_is_running(&self, internal_session: &str) -> bool {
        let _ = internal_session;
        false
    }

    /// **Does this worker answer [`session_is_running`](Worker::session_is_running) at all?** (nxf
    /// 6j6v.t41e) — the question that read cannot carry, and the reason it needs one: its default is
    /// `false`, so at every call site a `false` meaning *nothing is running* is the same word as a
    /// `false` meaning *I never looked*.
    ///
    /// Reported from app-foundations (41j0.4yjx), measured on nxs 0.67.0 rather than supposed. What
    /// a host was left with was a CLASSIFICATION of its own worker configuration, kept in another
    /// repository against a `#[non_exhaustive]` enum — and silent for the one case that needs it
    /// most, a host that brings its own worker.
    ///
    /// **It takes no session, and that is what makes it the right question.** The warning it exists
    /// for fires at STARTUP, before anything has been triggered: *"this server's chat worker cannot
    /// tell a live role session from a dead one, so a chain whose holder died hard will not be
    /// noticed until its lease's own bound runs out, and `withdraw` cannot trust a running session
    /// there to have actually stopped"*. A per-session answer cannot be asked before there is a
    /// session to name.
    ///
    /// **Additive, with the default that is true of a worker that has not thought about it.** An
    /// existing implementation keeps exactly the behaviour it has; one that overrides
    /// `session_is_running` overrides this too, and the pair is what a caller reads together.
    ///
    /// **Answer `true` only if you LOOK.** Not "no session of mine can be running" — the workspace
    /// is shared, and a session another process started is a live process this worker would have to
    /// see to answer for. [`DryWorker`] and the engine's `DisabledWorker` start nothing and look at
    /// nothing, so they rest on this default rather than claiming a truthful-looking `false`;
    /// [`SidecarWorker`] reads a pid file and says `true`.
    ///
    /// **It changes no default's direction, and is not meant to.** The fail-OPEN direction of
    /// `session_is_running` is reasoned and stays: answering `true` by default would let one
    /// unknowable session stall a channel with nothing able to release it. What this adds is that
    /// every park and reclaim this crate makes — the acquire-path gate, the stranded-escalation and
    /// availability-boundary occasions, the tick's sweep past the bound or over a holder proven
    /// dead inside it, a withdrawn holder's own hand-off — plus every host outside this crate can
    /// now SEE which `false` they were given, and decide for themselves.
    ///
    /// Must return promptly, from local state, and must not call back into the engine — the two
    /// rules the read above it carries, for the same reason.
    fn answers_liveness(&self) -> bool {
        false
    }

    /// **Run one declared precondition's command and say what it did** (nxf 6j6v.n92p) — the second
    /// READ on this trait, and it is here for [`session_is_running`](Worker::session_is_running)'s
    /// reason: the answer is a fact about the machine the sessions run on, and whoever executes
    /// sessions is the one who owns that machine's working directory.
    ///
    /// `command` is a channel's declared
    /// [`Precondition::run`](crate::precondition::Precondition::run) verbatim. It is executed by the
    /// platform shell **in the working directory this worker starts its sessions in** — which is
    /// what "in the channel's working directory" means in practice, and why the engine cannot run it
    /// itself: `Ctx` has no cwd, `SidecarWorker` does.
    ///
    /// **The default REFUSES, and that is the opposite direction from the read above it.** That is
    /// not an inconsistency, it is decision 1: a liveness read that cannot be answered must say
    /// "not running", because answering `true` by default would let one unknowable session stall a
    /// channel forever; a hurdle that cannot be checked must say "not cleared", because letting it
    /// through is exactly the fail-open the hurdle exists to prevent. Each default is the safe
    /// direction of ITS OWN question, and the two questions point opposite ways.
    ///
    /// **A host that declines this loses nothing it had.** Only a channel that DECLARES
    /// `preconditions:` ever reaches here, so the default costs a host precisely the feature it did
    /// not implement — and it costs it loudly, as a
    /// [`RefusalKind::Unrunnable`](crate::precondition::RefusalKind::Unrunnable) on the requester's
    /// own receipt, rather than by quietly running the step anyway.
    ///
    /// Must return PROMPTLY — same rule, same reason: the engine holds its store mutex across this
    /// call. An implementation that runs a real process must bound it (see
    /// [`PRECONDITION_BOUND`]); a bound that runs out is
    /// [`PreconditionOutcome::Unavailable`](crate::precondition::PreconditionOutcome::Unavailable),
    /// never a pass.
    fn run_precondition(&self, command: &str) -> crate::precondition::PreconditionOutcome {
        let _ = command;
        crate::precondition::PreconditionOutcome::Unavailable(
            "this worker does not run commands, so the hurdle could not be checked — a \
             precondition that cannot be asked does not pass"
                .to_string(),
        )
    }

    /// **Which directory this worker starts its sessions in** (nxf 6j6v.de9s) — the THIRD read on
    /// this trait, and it is here for the reason the other two are: the answer is a fact about the
    /// machine sessions run on, and whoever executes sessions owns it.
    ///
    /// It is the same directory [`run_precondition`](Worker::run_precondition) runs a hurdle in and
    /// the same one a triggered session is `chdir`'d into — for [`SidecarWorker`] literally the same
    /// field, so a hurdle asking about the working copy, a session writing into it and a park saving
    /// it cannot be looking at three different trees.
    ///
    /// **What asks.** `orchestration::park_the_stranded_holder` — when an unanswered escalation has
    /// held the working copy past [`crate::park::PARK_AFTER`] with another operation waiting, the
    /// engine commits that operation's work onto a branch and hands the copy on. It needs a
    /// directory to do that in, and this is the only thing that knows one.
    ///
    /// **The default is `None`, and that is a decline rather than a guess.** A host whose sessions
    /// do not run in a directory this process can see — a remote runtime, an in-process stub — must
    /// not have a park attempted in whatever directory `nxc` happened to be started from: that would
    /// commit an unrelated tree under an operation's name. `None` means no parking, said out loud
    /// on the receipt (`ParkRefusal::NoWorkingCopy`), and nothing else about the host changes.
    ///
    /// **Answer only with a directory you actually start sessions in.** Not "where my config lives",
    /// not the process cwd — the tree a session writes to, because what this permits is a commit in
    /// it.
    ///
    /// Must return promptly, from local state, and must not call back into the engine — [`Worker`]'s
    /// two rules, for their reason.
    fn working_copy(&self) -> Option<PathBuf> {
        None
    }

    /// **Can this runtime CONTINUE a conversation it began?** (nxf 6j6v.npy3)
    ///
    /// Not "can it start a session" — every worker can do that — but: given a
    /// [`resume_real`](TriggerRequest::resume_real) it once handed back, does a trigger carrying it
    /// continue that conversation, with its own history in front of the model, rather than open a
    /// fresh one under the same name?
    ///
    /// **Why it is a PROVIDER capability and not a property of this crate** (owner, 2026-09-12,
    /// question 3). Continuation is a thing a runtime either does or does not do, and it is done
    /// differently by each one that does: Claude Code keeps its transcript on disk at the project
    /// path and `--resume` finds it days later, a cloud runtime reaps a session silently
    /// ([`TriggerError::SessionGone`] is that fact), and a third may carry no conversation across
    /// processes at all. The engine's own record — `nxc transcript`, which stores the normalized
    /// stream with every tool call AND its result — is provider-neutral and outlives all of them;
    /// replaying it into another vendor's conversation format is a separate piece of work and is
    /// deliberately not day one. What is day one is that the SEAM already asks the question here,
    /// so the day that work happens it has a place to land, instead of a resume that was screwed
    /// onto Claude Code and has to be prised off.
    ///
    /// **The default is `false`, and it is a decline rather than a guess** — exactly
    /// [`working_copy`](Worker::working_copy)'s direction, for a sharper reason. A host that does
    /// not in fact resume would, on a `true`, silently start a session from NOTHING under a name
    /// that has a transcript: the continued session would not see the `nxf create` it already ran,
    /// and would run it again. The transcript is the whole protection against doing the work twice,
    /// and a wrong `true` here is precisely the state in which that protection is absent while
    /// everything claims it is present. A `false` costs a named refusal
    /// ([`crate::orchestration::ResumeOutcome::ProviderCannotResume`]) and nothing else.
    ///
    /// Must return promptly, from local state, and must not call back into the engine — [`Worker`]'s
    /// two rules, for their reason.
    fn resumes_sessions(&self) -> bool {
        false
    }

    /// **Can this worker STOP a session it started?** (nxf 6j6v.b9nf) — the question that precedes
    /// the first WRITE on this trait, asked separately for
    /// [`answers_liveness`](Worker::answers_liveness)'s reason: a capability a host does not have
    /// must be refusable BY NAME, before anything is attempted, rather than discovered as a stop
    /// that never took.
    ///
    /// **What asks.** `withdraw` on a RUNNING round. Until this existed the engine could take back a
    /// queued commission and nothing else: a round whose session was still writing into the checkout
    /// could be parked, but the session went on writing, and the park was stale before its receipt
    /// printed. Stopping the session is what makes a withdrawal of running work mean anything.
    ///
    /// **The default is `false`, and it is a decline rather than a guess** — exactly
    /// [`resumes_sessions`](Worker::resumes_sessions)'s direction. A host that does not in fact
    /// stop anything would, on a `true`, have a withdrawal believe its stop was delivered, wait for a
    /// process that is not going anywhere, and then hand the working copy to the next operation with
    /// the old session still in it — the two-agents-one-checkout failure this whole seam exists to
    /// prevent. A `false` costs a named refusal and nothing else.
    ///
    /// Must return promptly, from local state, and must not call back into the engine — [`Worker`]'s
    /// two rules, for their reason.
    fn stops_sessions(&self) -> bool {
        false
    }

    /// **Ask the session to stop** (nxf 6j6v.b9nf) — the one WRITE on this trait, and the shape of
    /// it is the whole contract: it returns once the request is DELIVERED, never once the process
    /// has exited.
    ///
    /// **Why delivered and not gone.** This is called with the engine's store mutex held, like every
    /// other method here, and a session's teardown is real work — it binds its runtime session,
    /// flushes its transcript and announces its own end through `nxc session ended`, which is a call
    /// back INTO the store this caller is holding. A `stop_session` that waited for the exit would
    /// deadlock on the announcement it is waiting for. So the caller delivers, releases, and watches
    /// [`session_is_running`](Worker::session_is_running) turn false before it touches the tree.
    ///
    /// **A request, not a kill.** [`SidecarWorker`] sends SIGTERM to the one pid its own lock
    /// recorded — not SIGKILL, not the process group — because the sidecar has that teardown to run
    /// and every step of it is lost to a hard kill. A host worker in front of another runtime does
    /// whatever that runtime's graceful stop is.
    ///
    /// **Two successes and one failure** (nxf 6j6v.b9nf, fix round 3 of that item's review).
    /// [`SessionStop::Requested`] is the signal delivered; [`SessionStop::NothingToStop`] is the
    /// session already over, which is the state the caller wanted and must not be reported as a
    /// failure — a round that finishes on its own between the liveness read and the stop is the
    /// ordinary race here, not a fault. `Err` means the session may still be running and this call
    /// did not end it: a missing pid file, a claim whose process cannot be identified, a signal the
    /// operating system refused.
    ///
    /// **The default REFUSES, by name.** A host that has not implemented this cannot stop a session,
    /// and the honest answer is the one [`stops_sessions`](Worker::stops_sessions) already gave —
    /// the refusal here is what a caller that skipped the question gets instead of a silent `Ok`
    /// that reads as delivered.
    ///
    /// Must return promptly and must not call back into the engine — the two rules, for the reason
    /// above.
    fn stop_session(&self, internal_session: &str) -> std::result::Result<SessionStop, String> {
        let _ = internal_session;
        Err("this worker cannot stop a running session".to_string())
    }
}

/// How long one declared precondition's command gets before [`SidecarWorker`] kills it and reports
/// [`PreconditionOutcome::Unavailable`](crate::precondition::PreconditionOutcome::Unavailable).
///
/// **A breach detector, not a budget** — [`CUSTOM_WORKER_BOUND`]'s argument at the third external
/// seam, and the same one `timer.rs`'s `AT_SUBPROCESS_BOUND` makes at the second. A hurdle is a
/// deterministic read (`git status --porcelain`, `git rev-list --count`), and those answer in
/// milliseconds on any tree small enough to work in. Thirty seconds is far past anything a
/// well-formed hurdle needs and far short of "forever", which is what no bound at all amounts to —
/// and forever here is worse than at the other two seams, because this call is made with the store
/// mutex held and with a requester waiting on the receipt.
///
/// The bound is enforced by KILLING the child, not by walking away from it (see the implementation).
/// A hurdle that hangs is usually a hurdle that took a lock in the very working copy the next step
/// wants, so leaving it running would be the one failure this whole item exists to prevent.
pub const PRECONDITION_BOUND: Duration = Duration::from_secs(30);

/// How long the collection of a killed hurdle's output may take before the engine stops waiting for
/// it — the backstop behind [`kill_the_hurdle`].
///
/// **Not a second bound on the command; a bound on the PIPE.** A pipe closes when its last WRITER
/// lets go, which is not the same event as the child exiting: anything the shell left behind
/// inherits the write end and holds it open. Reading until end-of-file therefore has no bound of its
/// own, and this call is made with the engine's store mutex held. Killing the process group is what
/// normally makes that moot; this is what makes it moot even when it does not (a process the group
/// kill cannot reach — one that changed its own group, or a platform without groups at all).
///
/// A quarter of a second because it is only ever reached in that pathological case, and there is
/// nothing left worth waiting for: whatever a killed hurdle had written is already in the pipe.
pub const OUTPUT_GRACE: Duration = Duration::from_millis(250);

/// Stop a hurdle that ran past its bound — **the whole process group, not just the shell**.
///
/// `Child::kill` signals the one pid it started. For a simple command that is the command itself
/// (a shell `exec`s it) and nothing more is needed. For a pipeline, a `&`, or any compound line the
/// shell stays and its children are separate processes: killing the shell leaves them running, in
/// the very working copy the next step is about to be started in — which is the collision this whole
/// epic exists to prevent — and holding the output pipe open, which used to hang the engine.
///
/// The child is put in its own group at spawn time, so `kill(-pid)` reaches everything the hurdle
/// started and nothing else. `wait` afterwards reaps the shell; a grandchild is reparented and
/// already dead.
///
/// Best-effort throughout: a signal to a group that has just exited is an errno, not a problem, and
/// the caller's answer ([`crate::precondition::PreconditionOutcome::Unavailable`]) is the same
/// either way. On a platform without process groups this degrades to exactly what it did before —
/// the shell dies, [`OUTPUT_GRACE`] covers the rest.
fn kill_the_hurdle(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // SAFETY: `kill` with a negative pid signals the process group of that pid, and this child
        // was spawned into a group of its own (`process_group(0)`), so the group contains the
        // hurdle and nothing else. It delivers a signal and touches no memory of this process; a
        // group that is already gone is an `ESRCH`, not undefined behaviour.
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// How long a host-supplied worker gets to RETURN from [`Worker::trigger`] before the engine stops
/// waiting on it (PR #269 review, Code Quality #1 / Integrity #1).
///
/// **A breach detector, not a budget.** A worker that honours [`Worker`]'s first rule returns in
/// microseconds — it hands its network call to its own executor and answers
/// [`TriggerOutcome::Accepted`]. Thirty seconds is far beyond anything a conforming implementation
/// needs and far short of "forever", which is what the wait used to be.
///
/// This is `timer.rs`'s [`AT_SUBPROCESS_BOUND`](crate::timer) argument at the other external seam,
/// and it is here for the same reason that one exists: an unbounded wait on something outside this
/// process was not a hypothetical there either — it wedged a live session for eight minutes.
pub const CUSTOM_WORKER_BOUND: Duration = Duration::from_secs(30);

/// Runs a host-supplied worker on its own thread so a breach of [`Worker`]'s two rules is BOUNDED
/// rather than unbounded (PR #269 review, Code Quality #1 / Integrity #1 / Test Quality #2).
///
/// [`crate::engine::Engine`] holds one mutex over its store for the duration of an orchestration
/// verb, and the trigger happens inside it. That is fine for the workers this crate ships — a
/// detached `Command::spawn`, a log line, an immediate refusal — and it was fine while those were
/// the only ones there could be. [`WorkerConfig::Custom`] makes the code inside that lock a third
/// party's, and a doc comment is not a mechanism: an implementation that blocks on its provisioning
/// call would freeze every verb on every clone of the handle, forever, with nothing to observe it.
///
/// So the engine stops waiting after [`CUSTOM_WORKER_BOUND`] and reports the breach. Three
/// properties follow, and all three are the point:
///
/// - **The mutex is released.** The verb returns, the handle keeps working; the cost of a
///   misbehaving worker is bounded per trigger instead of terminal for the process.
/// - **Rule 2's deadlock becomes a delay.** A worker that calls back into the `Engine` from inside
///   `trigger` now blocks on a DIFFERENT thread than the one holding the mutex, so when this call
///   gives up and the verb unwinds, that callback proceeds instead of hanging forever. Still a
///   contract breach, still reported — but no longer unrecoverable.
/// - **A panic in host code stays in host code.** It disconnects the channel instead of unwinding
///   through the engine's locked section and poisoning the mutex on its way out.
///
/// **What a timeout cannot know is whether the session started.** The thread is still running when
/// this gives up, so the answer is genuinely indeterminate and the error says so rather than
/// implying "nothing happened". A caller treats it as a failed trigger, which is the fail-VISIBLE
/// direction: a receipt that reports a skip is recoverable, a receipt that silently claims success
/// is not.
struct BoundedWorker {
    inner: Arc<dyn Worker>,
}

impl Worker for BoundedWorker {
    /// **Forwarded, not bounded** (nxf 6j6v.10yb). The bound above exists because `trigger` is where
    /// a host does provisioning work it might block on; this is a read that a conforming
    /// implementation answers from local state, and wrapping every one of them in a thread and a
    /// channel would cost more than the call. A host that blocks here breaks the same rule it would
    /// break by blocking in `trigger`, and the honest consequence — a stalled verb — is the one its
    /// own doc names.
    fn session_is_running(&self, internal_session: &str) -> bool {
        self.inner.session_is_running(internal_session)
    }

    /// **Forwarded for the same reason, and it is the reason the wrapper exists at all**: a host
    /// worker that CAN answer the liveness question would be reported as unable to if this
    /// delegator answered for it (nxf 6j6v.t41e). It is a plain field read — nothing to bound.
    fn answers_liveness(&self) -> bool {
        self.inner.answers_liveness()
    }

    /// **Forwarded, not bounded** — [`session_is_running`](Worker::session_is_running)'s argument
    /// above, unchanged. A host that runs a precondition owns the bound on it the way it owns the
    /// bound on any other work it does in a read; wrapping it here would bound the wrong thing and
    /// would still not be able to stop the host's process.
    fn run_precondition(&self, command: &str) -> crate::precondition::PreconditionOutcome {
        self.inner.run_precondition(command)
    }

    /// **Forwarded, for the reason the wrapper exists** (nxf 6j6v.de9s) — the same argument
    /// [`answers_liveness`](Worker::answers_liveness) makes one method up. A host worker that DOES
    /// start its sessions in a directory would be reported as having none if this delegator answered
    /// the default for it, and the visible consequence would be an operation that is never parked
    /// with nothing saying why.
    fn working_copy(&self) -> Option<PathBuf> {
        self.inner.working_copy()
    }

    /// Forwarded like its three neighbours — a selection failure answers `false`, which is
    /// [`Worker::resumes_sessions`]'s own reasoned default and the safe direction here: a refusal
    /// to continue costs a named outcome, and a wrong claim to continue costs the work being done
    /// twice.
    fn resumes_sessions(&self) -> bool {
        self.inner.resumes_sessions()
    }

    /// Forwarded, for the reason the wrapper exists (nxf 6j6v.b9nf): a host worker that CAN stop
    /// its sessions would be reported as unable to, and a withdrawal of a running round would then
    /// refuse by name for a runtime that can. A plain field read on a conforming host.
    fn stops_sessions(&self) -> bool {
        self.inner.stops_sessions()
    }

    /// **Forwarded, not bounded** — the same argument
    /// [`session_is_running`](Worker::session_is_running) makes at the top of this block, and it
    /// holds for the one WRITE too: a conforming `stop_session` DELIVERS a request and returns,
    /// which is local work. A host that blocks here until its process is gone breaks the rule its
    /// own doc names, and wrapping it in a thread would bound the wait without being able to stop
    /// the host's process either.
    fn stop_session(&self, internal_session: &str) -> std::result::Result<SessionStop, String> {
        self.inner.stop_session(internal_session)
    }

    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        let (tx, rx) = std::sync::mpsc::channel();
        let inner = self.inner.clone();
        let session = req.internal_session.clone();
        // Detached on purpose: a thread blocked in host code cannot be cancelled from the outside in
        // Rust, so the only honest options are "wait forever" (what this replaces) and "stop waiting
        // and let it finish on its own". `send` on a receiver that is already gone is an error this
        // deliberately ignores — by then nobody is listening, which is exactly the case being
        // handled.
        std::thread::spawn(move || {
            let _ = tx.send(inner.trigger(req));
        });
        match rx.recv_timeout(CUSTOM_WORKER_BOUND) {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(TriggerError::Failed(
                NxfError::io(format!(
                    "the host-supplied worker did not return within {CUSTOM_WORKER_BOUND:?} while \
                     triggering session {session}; it may or may not have started — a `Worker` must \
                     hand long work to its own executor and return `Accepted`, and must not call \
                     back into the Engine from inside `trigger`"
                )),
            )),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(TriggerError::Failed(
                NxfError::io(format!(
                    "the host-supplied worker panicked while triggering session {session}"
                )),
            )),
        }
    }
}

/// The recording worker: writes one line per trigger and spawns nothing.
///
/// **Where it writes is a FIELD, not process env** (nxf 6j6v.570x). `trigger` used to read
/// `NXC_DRY_LOG` from `std::env::var` on every call, which made the destination process-global in a
/// test binary whose tests run in parallel threads. Two integration tests set the variable to their
/// own `TempDir`; a third one — which wanted no dry log at all and therefore took no lock — reached
/// the same worker through the persona coordinator and inherited whichever path happened to be
/// set, long
/// after that `TempDir` and its directory were gone. `create(true)` does not create a missing
/// *directory*, so the append failed with `ENOENT` and reddened a run that had nothing to do with
/// the change under test.
///
/// Making it a field ends that class rather than the one case: a `DryWorker` writes where its owner
/// said and nowhere else, so no test can inherit a neighbour's path and none of them need a lock.
/// The environment variable keeps its meaning at the one seam that reads environment on purpose —
/// [`WorkerConfig::from_ambient_or_installed`], where the CLI resolves it once per process.
pub struct DryWorker {
    /// Where to append the trigger record. `None` records nothing — the same "silently a no-op"
    /// behaviour an unset `NXC_DRY_LOG` used to produce, now stated by whoever built the worker.
    pub log: Option<PathBuf>,
}

impl Worker for DryWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        if let Some(path) = &self.log {
            use std::io::Write;
            // `coordinator=` sits AFTER `resume=` and before `msg=` deliberately (nxf 6j6v.ntp9):
            // `msg` embeds real newlines, so nothing may follow it, and two suites match on the
            // literal `role=<h> session=` adjacency. Appending a field is what keeps every existing
            // reader — all of which look a key up by name — byte-compatible, which is why
            // `declaration=` (nxf 6j6v.pkw9) joins it at the same end rather than anywhere earlier.
            //
            // SHORT here and full in the spec file, deliberately: this line is read by a human and
            // by tests grepping it, and a 64-digit hash in the middle of it buys neither of them
            // anything the first twelve digits do not. `-` for a spawn with no declaration, the
            // same placeholder `resume=` already uses — this is a whitespace-separated key=value
            // line, so an empty value would make the NEXT key read as this one's.
            let line = format!(
                "trigger role={} session={} resume={} coordinator={} declaration={} msg={}\n",
                req.role.handle,
                req.internal_session,
                req.resume_real.as_deref().unwrap_or("-"),
                req.coordinator.as_str(),
                req.role
                    .declaration_hash
                    .as_deref()
                    .map(crate::declaration_version::short)
                    .unwrap_or("-"),
                req.message
            );
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| NxfError::io(format!("dry log: {e}")))?;
            f.write_all(line.as_bytes())
                .map_err(|e| NxfError::io(format!("dry log: {e}")))?;
        }
        Ok(TriggerOutcome::Accepted)
    }
}

pub struct SidecarWorker {
    pub sidecar: PathBuf,
    pub cwd: PathBuf,
}

impl SidecarWorker {
    /// Is this the SHIPPED bundle rather than a source-tree `main.mjs`? The two differ in exactly
    /// one way that matters here: the bundle has no `node_modules` beside it, so the SDK cannot
    /// resolve the native `claude` on its own and the host's resolution is not an optimisation but
    /// a precondition. Keyed on the file name, which is the name `install.sh`/`self-update` write
    /// and [`sidecar_beside`] looks for — an `NXC_SIDECAR` pointed at a copy of the bundle is
    /// correctly treated as the bundle.
    fn is_bundled_sidecar(&self) -> bool {
        self.sidecar.file_name().is_some_and(|n| n == SIDECAR_FILE)
    }

    /// [`Worker::stop_session`]'s body, with the SIGNAL as a parameter — every decision here, and
    /// no way for a test of those decisions to send one (fix round 3 of nxf 6j6v.b9nf's review,
    /// Test Quality #4).
    ///
    /// The parameter is a test seam, and a measured one. The two cases that pin the group-signal
    /// guard hand this function a claim holding `0` and one holding `u32::MAX`; if the guard ever
    /// regressed, calling the real `stop_session` for them would run `kill(0, SIGTERM)` — the test
    /// runner's own process group — or `kill(-1, SIGTERM)`. That is not hypothetical: a pid file
    /// holding `0` took down the test run that first exercised this stop (see [`a_signalable_pid`]).
    /// With the signaller injected, a regression is a recorded call the test FAILS on instead of a
    /// dead run, and [`Worker::stop_session`] keeps exactly one line of its own.
    fn stop_session_by(
        &self,
        internal_session: &str,
        send: impl Fn(u32) -> std::result::Result<SessionStop, String>,
    ) -> std::result::Result<SessionStop, String> {
        let path = agent_logs_dir(&self.cwd).join(format!("{internal_session}.pid"));
        let content = std::fs::read_to_string(&path).map_err(|e| {
            format!(
                "session {internal_session} has no pid file at {} — this workspace never started \
                 it: {e}",
                path.display()
            )
        })?;
        // **ONE process, or nothing is sent** (review of this item's first round, Important #1) —
        // [`a_session_claim`], which is where that rule is argued and which every other reader of
        // these files now shares (fix round 2). It runs here at the parse step, before the platform
        // and identity reads, so the refusal is the same on every platform.
        let claim = a_session_claim(&content).ok_or_else(|| {
            format!(
                "the pid file for session {internal_session} at {} does not hold a pid — a \
                 single process is a positive number no larger than i32::MAX; 0 or anything that \
                 wraps negative would signal a whole process group",
                path.display()
            )
        })?;
        if !SESSIONS_ARE_STOPPABLE {
            return Err(format!(
                "this build cannot signal a process, so session {internal_session} cannot be \
                 stopped from here"
            ));
        }
        let pid = claim.pid;
        match who_holds(&claim) {
            ClaimHolder::TheSession => send(pid),
            ClaimHolder::Gone => Ok(SessionStop::NothingToStop(format!(
                "session {internal_session} has no live process — pid {pid} from {} is gone, so \
                 there was nothing to stop",
                path.display()
            ))),
            // **Gone, not running** — the session this claim names ended, and the pid it left
            // behind is now an unrelated process of this user. Reported as the goal state (there
            // is nothing of this session to stop) and NEVER signalled.
            ClaimHolder::SomebodyElse => Ok(SessionStop::NothingToStop(format!(
                "session {internal_session} has no live process — pid {pid} from {} is a process \
                 that started at a different time from the one this claim recorded, so the session \
                 ended and its pid was handed on. Nothing was signalled",
                path.display()
            ))),
            // Refused BY NAME, and a failure: unlike the two above, nothing here says the session
            // ended — only that this file cannot prove which process is behind it.
            ClaimHolder::Unverifiable => Err(format!(
                "session {internal_session} was not signalled: this session's claim predates \
                 identity recording — it cannot be told apart from a recycled pid. {} names pid \
                 {pid}, which is alive, and nothing in the file says that process is this \
                 session's. Check it (`ps -p {pid}`) and stop it by hand if it is the session",
                path.display()
            )),
        }
    }

    /// [`Worker::run_precondition`]'s body, with the bound as a PARAMETER.
    ///
    /// The bound is a parameter for one reason, and it is a test one: the fail-closed timeout path
    /// is the arm that decides whether a hung hurdle stops a step or lets it through, and a test
    /// that had to wait out [`PRECONDITION_BOUND`] to reach it would take half a minute and be
    /// skipped. `tests/worker.rs` drives it with a bound of milliseconds. Same shape as
    /// [`CUSTOM_WORKER_BOUND`]'s own test seam, and for the same reason.
    ///
    /// `pub(crate)` deliberately: the SEAM a host implements is `run_precondition`, which fixes the
    /// bound. Nothing outside this crate gets to choose it.
    pub(crate) fn run_precondition_within(
        &self,
        command: &str,
        bound: Duration,
    ) -> crate::precondition::PreconditionOutcome {
        use crate::precondition::PreconditionOutcome;
        use std::io::Read;

        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(command)
            .current_dir(&self.cwd)
            .env_clear();
        for (k, v) in hurdle_env(|key| std::env::var(key).ok()) {
            cmd.env(k, v);
        }
        // **Its OWN process group, so the bound can reach everything the hurdle started.** A shell
        // `exec`s a simple command, so for `git status --porcelain` the child IS git and killing the
        // child is enough. It does not exec a pipeline, a `&`, or anything compound — there the
        // shell stays and its children are separate processes that a kill aimed at the shell does
        // not touch. Left in this process's group they would also outlive the whole `nxc`
        // invocation. One group per hurdle makes "stop this hurdle" mean what it says.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = match cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                return PreconditionOutcome::Unavailable(format!(
                    "could not run `{command}` in {}: {e}",
                    self.cwd.display()
                ))
            }
        };
        // Both pipes drained off-thread, BEFORE the wait loop below: a command whose output fills
        // the pipe buffer blocks in `write` until somebody reads, and a wait loop that reads only
        // after exit would then be waiting for an exit that cannot happen until it reads.
        //
        // **Over a CHANNEL and not a `JoinHandle`, and that difference is a defect this had.** A
        // pipe closes when its last writer lets go — which is not the same as "the child exited": a
        // grandchild the shell left behind inherits the write end and holds it open. `join()` on
        // such a reader NEVER RETURNS, and this runs inside the engine's held store mutex, so a
        // single hurdle that backgrounds anything wedged the whole handle permanently — past the
        // bound, past the kill, forever. Measured against `sh -c 'sleep 424242 & wait'`.
        //
        // So the readers hand their result over rather than being waited on, and the collection
        // below is bounded. A reader still parked on a fd nobody needs costs one thread and one
        // descriptor and ends on its own when the last writer finally goes; the engine does not wait
        // for it.
        let drain = |handle: Option<Box<dyn Read + Send>>| {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut buf = String::new();
                if let Some(mut h) = handle {
                    let _ = h.read_to_string(&mut buf);
                }
                let _ = tx.send(buf);
            });
            rx
        };
        let out = drain(
            child
                .stdout
                .take()
                .map(|h| Box::new(h) as Box<dyn Read + Send>),
        );
        let err = drain(
            child
                .stderr
                .take()
                .map(|h| Box::new(h) as Box<dyn Read + Send>),
        );

        let deadline = std::time::Instant::now() + bound;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Err(e) => {
                    return PreconditionOutcome::Unavailable(format!(
                        "waiting for `{command}` failed: {e}"
                    ))
                }
                Ok(None) => {}
            }
            if std::time::Instant::now() >= deadline {
                kill_the_hurdle(&mut child);
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // The child is reaped by here, so a healthy hurdle's pipes are already closed and both of
        // these return at once. The grace is only for the pathological case above — long enough
        // that a grandchild racing to exit still gets its output collected, short enough that
        // nothing can hold the mutex on it.
        let stdout = out.recv_timeout(OUTPUT_GRACE).unwrap_or_default();
        let stderr = err.recv_timeout(OUTPUT_GRACE).unwrap_or_default();
        match status {
            None => PreconditionOutcome::Unavailable(format!(
                "`{command}` did not finish within {bound:?} and was killed — a hurdle is a read \
                 and must answer promptly; it left {}",
                match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
                    (true, true) => "nothing behind".to_string(),
                    _ => format!("behind: {}{}", stdout.trim(), stderr.trim()),
                }
            )),
            Some(status) => PreconditionOutcome::Ran {
                status: status.code(),
                stdout,
                stderr,
            },
        }
    }
}
/// **One internal session, one process** (nxf 6j6v.7qtf) — the guard [`SidecarWorker`] takes before
/// it writes a spec or spawns anything.
///
/// The defect it closes: a wake that arrived while the session it names was still working was
/// answered with a second `node` process on the SAME spec file, in the SAME working directory, under
/// the SAME internal session id. The two overwrote each other's edits with no error, no warning and
/// no `warnings` entry, and it did not stop at two — every new process produced events that woke
/// again, so the count grew while it was being watched.
///
/// **A file with a pid in it, taken with `create_new`.** The claim is the atomic creation of
/// `<internal_session>.pid` in the agent-log directory: POSIX `O_EXCL` decides the race between two
/// `nxc` processes reaching this at once, so exactly one of them proceeds. The file's content is the
/// pid that holds the session **and the instant that process started** ([`session_claim_for`]) —
/// first this process (which is alive for the whole window in which the child does not exist yet),
/// then the child's, swapped in by [`SessionLock::hand_to`] through a temp-file `rename` so no
/// reader can ever see a torn claim.
///
/// **Nothing releases it, and that is deliberate.** A sidecar that finished, crashed, or was killed
/// leaves its pid file behind; the next trigger finds the pid DEAD and takes the claim over. There
/// is no teardown hook to miss and no way for a crash to wedge a session forever — the liveness of a
/// real process is the whole of the answer, which is the one fact that cannot go stale.
///
/// **The residuals, stated rather than hidden.** A lock file whose content does not parse is treated
/// as stale, because the alternative — assuming a live holder — is a session no trigger can ever
/// start again. A pid REUSED by an unrelated process used to refuse one legitimate wake until that
/// process exited; since nxf 6j6v.b9nf's fix round 3 the recorded start instant disproves that
/// claim and the trigger proceeds ([`who_holds`]). What is left of the residual is the claim that
/// cannot be judged at all — an old file written before the instant was recorded, or a platform
/// with no start-time API — which this gate still believes, on purpose: being wrong here costs a
/// delayed wake, and being wrong the other way costs two processes on one spec.
#[derive(Debug)]
struct SessionLock {
    path: std::path::PathBuf,
}

impl SessionLock {
    /// Claim `session` for this process, or say who already holds it.
    ///
    /// `Err(TriggerError::AlreadyRunning)` is the refusal the item asks for and is NOT a failure of
    /// this function — see that variant's own doc.
    fn acquire(logs: &Path, session: &str) -> std::result::Result<SessionLock, TriggerError> {
        let path = logs.join(format!("{session}.pid"));
        // At most two rounds: claim, and — if the holder turned out to be dead — clear the stale
        // claim and try once more. A second failure means another process won the same race in
        // between, and the honest answer is that somebody is running.
        for _ in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    use std::io::Write;
                    // **This process, and WHEN this process started** ([`session_claim_for`], fix
                    // round 3 of nxf 6j6v.b9nf's review). The claim written here names `nxc`
                    // itself for the window in which the child does not exist yet, and a reader
                    // that may SIGTERM what it finds has to be able to tell that process from one
                    // the system handed this pid to afterwards. One `write_all` of the whole
                    // claim, so a concurrent reader sees the file complete or not at all.
                    f.write_all(session_claim_for(std::process::id()).as_bytes())
                        .map_err(|e| NxfError::io(format!("session lock: {e}")))?;
                    return Ok(SessionLock { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // The same [`a_session_claim`] every other reader of these files uses (fix
                    // rounds 2 and 3 of nxf 6j6v.b9nf's review): a claim holding `0` would
                    // otherwise be probed with `kill(0, 0)` — a question about THIS process's own
                    // group, true for as long as `nxc` is alive — and every trigger for the session
                    // would be refused as "already running" against a claim nobody holds. It falls
                    // to the stale-claim branch below instead, which is what the comment there
                    // already says about content that does not parse.
                    //
                    // **And the holder has to be the SESSION's process, not merely a live one.**
                    // A claim whose pid the system handed on after the sidecar died names a
                    // stranger, and refusing every trigger for the session behind it would wedge
                    // that session for as long as the stranger runs. [`ClaimHolder::SomebodyElse`]
                    // falls to the same stale branch. An UNVERIFIABLE claim (an old bare-pid file,
                    // or a platform that cannot say) keeps this gate's old answer — believed —
                    // because the cost of being wrong here is a delayed wake and the cost of the
                    // other direction is two processes on one spec.
                    if let Some(claim) = claim_in(&path) {
                        if matches!(
                            who_holds(&claim),
                            ClaimHolder::TheSession | ClaimHolder::Unverifiable
                        ) {
                            return Err(TriggerError::AlreadyRunning {
                                session: session.to_string(),
                                pid: claim.pid,
                            });
                        }
                    }
                    // Dead holder, a pid that is now somebody else's, or content that does not
                    // parse — a claim nobody is honouring.
                    std::fs::remove_file(&path)
                        .map_err(|e| NxfError::io(format!("clearing a stale session lock: {e}")))?;
                }
                Err(e) => return Err(NxfError::io(format!("session lock: {e}")).into()),
            }
        }
        Err(NxfError::io(format!(
            "session lock for {session} was taken by another process while this one was clearing it"
        ))
        .into())
    }

    /// Hand the claim to the spawned child, atomically: a temp file plus a `rename`, so a concurrent
    /// reader sees either the old pid or the new one and never half of either.
    ///
    /// A failure here is NOT fatal to the trigger: the session is running, and the claim still names
    /// THIS process, which is alive until this `nxc` invocation ends. What it costs is that the
    /// window in which a second wake would be refused ends early rather than lasting the child's
    /// life — the old behaviour, for this one call, rather than a spawn that is reported as failed
    /// after the process is already up.
    fn hand_to(&self, pid: u32) {
        let tmp = self.path.with_extension("pid.tmp");
        // **The child's pid AND the instant the child started** ([`session_claim_for`], fix round 3
        // of nxf 6j6v.b9nf's review, Integrity #1). This is the moment the claim moves to the
        // process a withdrawal may one day SIGTERM, so it is the moment its identity has to be
        // recorded: the child is alive right here, which is what makes the operating system's
        // answer about it available at all. A claim written without it can never be told apart from
        // a recycled pid afterwards, and is refused by the stop rather than signalled.
        // Breadcrumbed rather than swallowed (independent review of this branch, Low): every other
        // best-effort failure in this change says so on stderr, and a claim that quietly stopped
        // moving to the child is a guard that quietly narrowed to the length of one `nxc`
        // invocation. Still not fatal — see this function's own doc for what it costs.
        let handed = std::fs::write(&tmp, session_claim_for(pid))
            .and_then(|()| std::fs::rename(&tmp, &self.path));
        if let Err(e) = handed {
            eprintln!(
                "warning: could not hand the session claim {} to pid {pid}: {e} — this session is \
                 running, but a wake arriving after this process exits will not be refused",
                self.path.display()
            );
        }
    }
}

/// **The directory a workspace's session claims live in** — `<root>/.nxs/agent-logs`.
///
/// One spelling, because there are now five readers of it: the trigger that creates a claim,
/// [`Worker::session_is_running`] that reads one back, [`live_sessions_in`] that reads all of them,
/// [`SidecarWorker::stop_session`] that signals the process one names, and every message that tells
/// a person where to look.
pub fn agent_logs_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".nxs").join("agent-logs")
}

/// **Every session in `workspace_root` whose process is still alive** (nxf 6j6v.7q3r), by internal
/// session id, sorted.
///
/// The same fact [`Worker::session_is_running`] answers for ONE session, asked of a whole workspace
/// by something that does not know the ids — the background service, which has to decide whether to
/// hold the machine awake and only knows which workspaces it attends. It reads exactly the same pid
/// files, through the same [`a_session_claim`] parse and the same [`who_holds`], so what a claim
/// file may hold and what it proves are decided in ONE place and this is one of its readers.
///
/// **A cheap, allocation-light read on a hot path**: the service asks this of every attended
/// workspace on every tick. A missing or unreadable directory is an empty answer, never an error —
/// a workspace that has never started a session simply has none.
///
/// It is here rather than in `nxs-service` because the pid file is THIS crate's: the service must
/// not learn the layout of a directory somebody else writes, which is the very reaching-past-the-seam
/// that 6j6v.h383 was cut to end.
pub fn live_sessions_in(workspace_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(agent_logs_dir(workspace_root)) else {
        return Vec::new();
    };
    let mut live: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            // `.pid` and nothing else: the same directory holds `<session>.log` and
            // `<session>.spec.json`, and a `.spec.json` outlives the process that read it.
            if path.extension().and_then(|x| x.to_str()) != Some("pid") {
                return None;
            }
            let session = path.file_stem()?.to_str()?.to_string();
            a_claim_a_session_still_holds(&path).then_some(session)
        })
        .collect();
    live.sort();
    live
}

/// **What a `.pid` claim has to say before anything asks the operating system about it** (nxf
/// 6j6v.b9nf, fix round 2 of that item's review) — the pid half of the ONE parse every reader of
/// these files shares ([`a_session_claim`]), so that what `stop_session` refuses and what the
/// liveness reads believe cannot drift apart.
///
/// Two shapes of a parsed `u32` do not name a process to `kill`, whose first argument is a `pid_t`:
/// `0` is the CALLER's own process group, and a value past `i32::MAX` wraps negative, which is a
/// group too (`-1` is every process this user may signal). Both are `None` here — "this file does
/// not hold a pid" — which is the same answer the residual [`SessionLock`] already names for
/// unparseable content, and it is the safe one in both directions: a read that answered `true` for
/// `0` would report *some* session as running for any live process at all, and a signal sent on it
/// would take down the caller's whole group.
///
/// That second half was measured before it was argued: a pid file holding `0` took down the test run
/// that first exercised the stop. The first half is what fix round 2 was about — until it, the two
/// liveness readers parsed a bare `u32`, and `process_is_alive(0)` is `kill(0, 0)`, a question about
/// the caller's group that answers `true` whenever this process is alive. In a withdrawn holder's
/// claim area (nxf 6j6v.b9nf) that answer is not a stalled gate but a wedged working copy: the
/// tick's park waits for nothing to be running, the marker never clears, and every release path
/// declines until somebody deletes the file by hand.
///
/// Pure, and pinned as such (`worker::tests::a_claim_names_one_process_or_nothing`): it is the one
/// decision a signal rests on, and a test that had to send one to exercise it is a test that can
/// take down the run it is part of.
fn a_signalable_pid(contents: &str) -> Option<u32> {
    contents
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|&pid| pid != 0 && i32::try_from(pid).is_ok())
}

/// **What a session claim names: one process, and WHICH process it is** (nxf 6j6v.b9nf, fix round 3
/// of that item's review, Integrity #1).
///
/// A pid on its own is not an identity. Claims are deliberately never removed ([`SessionLock`]), so
/// a sidecar that crashed or a machine that rebooted leaves a file naming a number the operating
/// system is free to hand to something else — an editor, a dev server, another `nxc`. `kill(pid, 0)`
/// answers `true` for that stranger just as it did for the session, which was tolerable while the
/// only consequence was a delayed wake and is not tolerable now that a withdrawal SIGTERMs what this
/// file names. So the claim records the instant its process started as well, and the reader compares
/// it against what the operating system says about the pid it finds today — the identity check
/// `nxs-service` already argues for the service heartbeat (`heartbeat::born_together`), shared
/// rather than copied.
///
/// [`started_at`](SessionClaim::started_at) is `None` for an OLD claim — a bare pid, written before
/// this existed. Such a file cannot be told apart from a recycled pid by any means, so it is
/// unverifiable rather than either proven or disproven; [`who_holds`] says what each reader does
/// with that, and the two directions are not the same.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionClaim {
    /// The process that holds the session, through [`a_signalable_pid`].
    pid: u32,
    /// When that process started, as [`nxs_service::heartbeat::rfc3339`] renders an instant.
    /// `None` when the file predates identity recording.
    started_at: Option<String>,
}

/// **The one spelling of a claim file's contents** (nxf 6j6v.b9nf): the pid on the first line, the
/// instant that process started on the second, and a trailing newline.
///
/// Public because a claim is written in exactly two places in this crate ([`SessionLock::acquire`]
/// and [`SessionLock::hand_to`]) and read back in four, and anything outside that models a running
/// session — every test in this workspace that stands a real child in for a sidecar — has to produce
/// the SAME shape or it is modelling a file no `nxc` would ever write.
///
/// **Line-oriented on purpose, in both directions.** The pid stays on the first line, so the read
/// this house's own history records (`ps -p $(cat .nxs/agent-logs/<id>.pid)`, reaching past the
/// seam) degrades into a visible error rather than a wrong pid; and a later field is a later line,
/// which today's readers ignore. A file with only the first line is the OLD format — see
/// [`SessionClaim::started_at`].
///
/// The instant comes from [`nxs_service::heartbeat::recorded_start`]: the operating system's own
/// answer for `pid` where it has one, and this process's "now" where it has none. A fallback stamp
/// is LATER than the process's real start, which is the direction `born_together` allows.
pub fn session_claim_for(pid: u32) -> String {
    let started = nxs_service::heartbeat::recorded_start(pid, std::time::SystemTime::now());
    format!("{pid}\n{}\n", nxs_service::heartbeat::rfc3339(started))
}

/// [`session_claim_for`]'s inverse — the ONE parse every reader of these files goes through.
///
/// `None` is "this file does not name a process": empty, unparseable, or a number that would signal
/// a GROUP ([`a_signalable_pid`]). A second line that is present but unusable is carried as-is and
/// disproved by [`who_holds`], which is where a recorded instant is judged; a MISSING second line is
/// the old format and says so.
fn a_session_claim(contents: &str) -> Option<SessionClaim> {
    let mut lines = contents.lines();
    let pid = a_signalable_pid(lines.next()?)?;
    let started_at = lines
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(SessionClaim { pid, started_at })
}

/// [`a_session_claim`] of what `path` holds — `None` for a file that is not there, cannot be read,
/// or does not name one process. The shape the three liveness readers want, which have no reason to
/// tell those cases apart; [`SidecarWorker::stop_session`] reads the file itself, because it refuses
/// each of them BY NAME.
fn claim_in(path: &Path) -> Option<SessionClaim> {
    a_session_claim(&std::fs::read_to_string(path).ok()?)
}

/// **Who the process behind a claim turns out to be** (nxf 6j6v.b9nf, fix round 3) — the four
/// answers every reader of a claim file branches on, so that no reader has to assemble them itself.
///
/// The two middle answers are the reason this is an enum and not a bool: "I proved it is somebody
/// else" and "I could not prove anything" are different facts, and the readers point in DIFFERENT
/// directions on the second one. A gate that only delays (a second trigger for a session, the park
/// waiting for the area to fall quiet) is safest believing an unverifiable claim is alive — the
/// alternative is two `claude` processes on one spec, which is the defect nxf 6j6v.7qtf closed. A
/// SIGNAL is safest refusing it, because the alternative is a stranger's process killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaimHolder {
    /// Nothing is there: `kill(pid, 0)` says `ESRCH`. The session ended, however it ended.
    Gone,
    /// A live process, and the start instant it records is the one the operating system reports for
    /// that pid — this is the session's own process.
    TheSession,
    /// A live process whose start instant is NOT the recorded one: the pid was handed on after the
    /// session died. The SESSION is gone; the process belongs to somebody else and is not this
    /// workspace's to touch.
    SomebodyElse,
    /// A live process and no way to tell: the claim records no instant (it predates this), or this
    /// platform cannot say when a process began
    /// ([`nxs_service::heartbeat::process_started_at`] answers `None`), or the pid belongs to
    /// another user and cannot be inspected.
    Unverifiable,
}

/// [`ClaimHolder`] for one claim: the liveness question first, then the identity question about
/// whatever answered it.
///
/// **Compared with slack, never for equality** (project memory
/// `linux-process-start-is-reconstructed`). On Linux a start instant is reconstructed from the boot
/// time plus a tick count, so two readings of one process can differ by about a second; on macOS the
/// kernel reports `execve` directly. `born_together` is the shared comparison that already carries
/// that argument, its two bounds and the clock-step residue they leave — see its doc in
/// `nxs-service`, which is why this crate does not have a second copy of it.
fn who_holds(claim: &SessionClaim) -> ClaimHolder {
    if !process_is_alive(claim.pid) {
        return ClaimHolder::Gone;
    }
    let recorded = match claim.started_at.as_deref() {
        // The old format: a bare pid, and nothing that could disagree with the operating system.
        None => return ClaimHolder::Unverifiable,
        Some(recorded) => recorded,
    };
    match nxs_service::heartbeat::born_together(
        recorded,
        nxs_service::heartbeat::process_started_at(claim.pid),
    ) {
        Some(true) => ClaimHolder::TheSession,
        Some(false) => ClaimHolder::SomebodyElse,
        // No answer to compare against: this platform has no start-time API, the pid belongs to
        // another user, or the recorded instant is not one. Nothing is proved either way.
        None => ClaimHolder::Unverifiable,
    }
}

/// **Is the session that took this claim still running?** — the whole of what the three liveness
/// readers ask ([`Worker::session_is_running`], [`live_sessions_in`], [`SessionLock::acquire`]).
///
/// `true` for a claim whose process is provably the session's, AND for one that cannot be verified:
/// see [`ClaimHolder`] for why a delaying gate points that way. `false` for a pid that is gone and
/// for one the identity check disproves — a claim on a recycled pid names a session that ENDED, and
/// reporting it as running is what would wedge a withdrawn holder's claim area for good.
fn a_claim_a_session_still_holds(path: &Path) -> bool {
    claim_in(path).is_some_and(|claim| {
        matches!(
            who_holds(&claim),
            ClaimHolder::TheSession | ClaimHolder::Unverifiable
        )
    })
}

/// Whether `pid` names a process that is alive — `kill(pid, 0)`, the POSIX question with no side
/// effect, which succeeds for a live process and fails with `ESRCH` for one that is gone.
///
/// **Precondition: `pid` came through [`a_signalable_pid`]** — `kill`'s first argument is read as a
/// `pid_t`, where `0` and a wrapped-negative value are process GROUPS, and a group's liveness is not
/// a session's.
///
/// `EPERM` (a live process this user may not signal) counts as ALIVE, which is what the errno
/// means: the process exists.
///
/// **It answers about a NUMBER, not about a session, and no caller may forget that** (nxf
/// 6j6v.b9nf, fix round 3 of that item's review, Integrity #1). This used to add that a reused pid
/// reading as alive was "the fail-safe direction for the one caller" — true while the only caller
/// was a gate that costs a delayed turn when it is wrong, and false the moment a caller SIGTERMs
/// what it read: there the same wrong `true` costs a stranger's process. There is no one safe
/// direction for both, which is why the identity question is asked separately ([`who_holds`]) and
/// every reader goes through it rather than through this.
#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    // SAFETY: `kill` with signal 0 performs only the existence/permission check and delivers
    // nothing. It cannot affect this process's memory, and a bad pid is an errno, not UB.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// No process-liveness question can be asked here, so every claim reads as stale and this platform
/// keeps the behaviour it had before nxf 6j6v.7qtf. Windows has no supported sidecar today (nxf
/// 6j6v.mwn4); wedging a session that no trigger could ever start again would be the worse trade.
#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> bool {
    false
}

/// **Whether THIS BUILD can ask about a process at all** — the condition
/// [`process_is_alive`] above is split on, named once so that the ANSWER and the CLAIM about the
/// answer ([`Worker::answers_liveness`]) cannot drift apart (review of PR #409, Integrity #1).
///
/// On unix the read asks the operating system. Everywhere else it is the stub above, which returns
/// `false` without asking anything — so a worker built for such a platform must say it does NOT
/// answer the liveness question, or a host acting on that `false` would be MORE confident in it
/// than before the question existed. That is the inversion nxf 6j6v.t41e was cut to prevent, and
/// duplicating the `cfg` at the claim is how it would come back: two spellings of one platform
/// condition, one of which someone edits.
#[cfg(unix)]
const LIVENESS_IS_ASKABLE: bool = true;
/// The blind half — see the unix arm above for the whole argument.
#[cfg(not(unix))]
const LIVENESS_IS_ASKABLE: bool = false;

/// **Whether THIS BUILD can send a session its stop** (nxf 6j6v.b9nf) — spelled as an ALIAS of
/// [`LIVENESS_IS_ASKABLE`] rather than as a second `#[cfg]`, because it is the same platform
/// condition: `kill(pid, 0)` and `kill(pid, SIGTERM)` are one system call with two arguments, and
/// a build that can ask the first can send the second. One spelling, so the claim
/// ([`Worker::stops_sessions`]) and the act ([`signal_to_stop`]) cannot drift apart the way the
/// review of PR #409 found the liveness pair could.
const SESSIONS_ARE_STOPPABLE: bool = LIVENESS_IS_ASKABLE;

/// Ask `pid` to stop: `SIGTERM`, to that one process — not `SIGKILL`, and not the group.
///
/// The sidecar CATCHES this signal (`agent-sidecar/src/main.mjs`): it aborts the running turn
/// through the SDK's own abort door, binds the runtime session, flushes the transcript and announces
/// its end through `nxc session ended`, and every one of those is lost to a hard kill. The group is
/// left alone for the opposite reason from [`kill_the_hurdle`]'s: a hurdle is a shell line whose
/// children must die with it, while the sidecar's child is the SDK's own `claude` process, which the
/// abort tears down in order.
///
/// NOT best-effort, unlike [`kill_the_hurdle`]: an `errno` is reported, never swallowed, because
/// the caller is about to wait on the liveness read for a process this may not have reached.
///
/// **Precondition: `pid` names one process** — non-zero and within `pid_t`'s range — enforced by
/// [`a_signalable_pid`], the one parse every reader of these files goes through and where the rule
/// is argued. `0` and a wrapped-negative value are both GROUP signals to `kill`, and this function
/// must never be handed either.
#[cfg(unix)]
fn signal_to_stop(pid: u32) -> std::result::Result<SessionStop, String> {
    // SAFETY: `kill` delivers a signal to a pid and touches no memory of this process; a pid that
    // is gone is `ESRCH`, one this user may not signal is `EPERM`, and both are errnos, not UB.
    let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if rc == 0 {
        return Ok(SessionStop::Requested);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        // **The race this whole path lives with, and it lands on the GOAL state** (fix round 3 of
        // nxf 6j6v.b9nf's review, Code Quality #6). The liveness read that chose this pid and this
        // signal are two system calls, and a session that finished in between is exactly what the
        // withdrawal was asking for. It used to be an `Err`, which made `nxc withdraw` say "the
        // process is still there; stop it by hand" about a process that had left, and exit 1.
        Some(libc::ESRCH) => Ok(SessionStop::NothingToStop(format!(
            "pid {pid} was gone by the time the signal was sent — the session ended on its own \
             between the liveness read and the stop, which is the state the stop was for"
        ))),
        // **Not "refused": not signalable BY THIS USER.** `EPERM` is a live process this process
        // may not signal, which on a session claim means the pid belongs to another user — so it
        // is not this workspace's sidecar at all. A real failure, because nothing was delivered
        // and whatever is running is still running.
        Some(libc::EPERM) => Err(format!(
            "pid {pid} is not signalable by this user — it is a live process belonging to somebody \
             else, so nothing was sent"
        )),
        _ => Err(format!("SIGTERM to pid {pid} was refused: {err}")),
    }
}

/// The blind half of [`signal_to_stop`]: nothing on this platform can be signalled, and
/// [`SESSIONS_ARE_STOPPABLE`] already says so to anyone who asks first.
#[cfg(not(unix))]
fn signal_to_stop(_pid: u32) -> std::result::Result<SessionStop, String> {
    Err("this build cannot signal a process".to_string())
}

impl Worker for SidecarWorker {
    /// **The pid file [`SessionLock`] already writes, read back** (nxf 6j6v.10yb). The claim taken
    /// with `create_new` at every trigger holds the pid of the detached `node` that IS the session
    /// and the instant that process started, so "is this session running" needs no new bookkeeping
    /// — it is the same file, the same [`a_session_claim`] parse that
    /// [`stop_session`](Worker::stop_session) refuses on, and the same [`who_holds`] identity check.
    ///
    /// **A recycled pid reads as NOT running** (nxf 6j6v.b9nf, fix round 3). It used to read as
    /// running, which was the stated residual of asking `kill(pid, 0)` about a number; a claim whose
    /// recorded start instant disagrees with the live process now names a session that ended, and
    /// saying otherwise is what would leave a withdrawn holder's claim area waiting for a process
    /// that is nobody's. What still reads as running is a claim that cannot be judged at all — see
    /// [`ClaimHolder::Unverifiable`], where the direction is argued.
    ///
    /// `false` for a session this workspace never started, which is what a missing file means.
    fn session_is_running(&self, internal_session: &str) -> bool {
        a_claim_a_session_still_holds(
            &agent_logs_dir(&self.cwd).join(format!("{internal_session}.pid")),
        )
    }

    /// **It looks — where it can — so it says so, and only there** (nxf 6j6v.t41e; the platform
    /// half from the review of PR #409). On unix the read above asks the operating system about a
    /// real pid out of a real file, which is what earns the claim: its `false` is *this session has
    /// no live process*, not *nobody asked*. The two residuals named above are residuals of the
    /// ANSWER, not of the ability to answer, so they do not qualify this.
    ///
    /// On a platform where [`process_is_alive`] is the stub that answers without asking, none of
    /// that is true and this says `false` — see [`LIVENESS_IS_ASKABLE`], which is the ONE place
    /// that condition is spelled, so the read and the claim about the read cannot drift.
    fn answers_liveness(&self) -> bool {
        LIVENESS_IS_ASKABLE
    }

    /// **The same one field the other three uses read** (nxf 6j6v.de9s): [`self.cwd`](SidecarWorker::
    /// cwd), the WORKSPACE ROOT that nxf 6j6v.npn0 made the worker stand at. That is what makes the
    /// park's commit and a session's writes provably the same tree rather than two directories that
    /// usually agree.
    fn working_copy(&self) -> Option<PathBuf> {
        Some(self.cwd.clone())
    }

    /// **Yes, and the mechanism is already here** (nxf 6j6v.npy3): a
    /// [`resume_real`](TriggerRequest::resume_real) travels into the spec as `resume`, the sidecar
    /// puts it on the SDK's `options.resume`, and Claude Code continues that conversation from the
    /// transcript it keeps at the project path. That transcript is on disk rather than in a process,
    /// which is why `--resume` still finds it days later — the case this item is about, where a
    /// weekly window puts the way back up to a week out.
    ///
    /// The cloud runtime this trait also serves is the counter-example that makes the question worth
    /// asking at all: it reaps a session silently (see [`TriggerError::SessionGone`]), so a worker
    /// in front of one answers `false` and gets a named refusal instead of a fresh conversation
    /// wearing a continued session's name.
    fn resumes_sessions(&self) -> bool {
        true
    }

    /// **Where the liveness read can be asked, the stop can be sent** (nxf 6j6v.b9nf) — see
    /// [`SESSIONS_ARE_STOPPABLE`], which is [`LIVENESS_IS_ASKABLE`] under the name of the thing it
    /// now also decides.
    fn stops_sessions(&self) -> bool {
        SESSIONS_ARE_STOPPABLE
    }

    /// **The pid file [`SessionLock`] wrote, read once more — and this time written TO** (nxf
    /// 6j6v.b9nf). The same path, the same [`a_session_claim`] parse and the same [`who_holds`]
    /// identity check that [`session_is_running`](Worker::session_is_running) uses, so the process
    /// this signals is by construction the one that read reports as running.
    ///
    /// **Nothing is signalled that has not proved it is this session** (fix round 3 of this item's
    /// review, Integrity #1). Liveness alone cannot: claims are deliberately never removed, so a
    /// crashed session leaves a file naming a number the system may since have handed to an editor,
    /// a dev server or another `nxc` of the same user — and `kill(pid, 0)` says `true` about that
    /// stranger. The whole point of a withdrawal is that somebody is clearing up a stuck operation,
    /// which is exactly when the claim is most likely to be stale. So the claim's recorded start
    /// instant is compared against what the operating system reports for the pid it finds
    /// ([`who_holds`]), and the answer decides all four cases below.
    ///
    /// The refusals and the two successes, each naming what it found, because the caller is about to
    /// wait on the liveness read and has to know whether anything was sent:
    ///
    /// * no pid file — this workspace never started the session (a claim is replaced by a later
    ///   trigger, never removed);
    /// * a file that does not hold a pid — the residual [`SessionLock`] names, and, sharper, a
    ///   number that `kill` would read as a GROUP (see [`a_signalable_pid`]);
    /// * a claim with no recorded identity — an OLD file, written before the instant was recorded.
    ///   It cannot be told apart from a recycled pid by any means, so it is refused by name and NOT
    ///   signalled. This is the one place the unverifiable claim goes the other way from the
    ///   liveness reads, and [`ClaimHolder`] carries the argument: a gate that is wrong costs a
    ///   delayed turn, a signal that is wrong costs a stranger's process;
    /// * a pid that is gone, or one whose identity disproves the claim — nothing to stop, which is
    ///   the state the caller wanted: [`SessionStop::NothingToStop`], not a failure (fix round 3,
    ///   Code Quality #6);
    /// * otherwise the signal, whose own outcome is [`signal_to_stop`]'s — including the `ESRCH`
    ///   that lands between this function's liveness read and the `kill`, which is the same goal
    ///   state one call later.
    ///
    /// The platform check comes AFTER the file is read and BEFORE the identity read: a missing file
    /// is a fact on every platform, while [`process_is_alive`]'s `false` on a platform that cannot
    /// ask would otherwise be reported here as "the process is gone" about a process nobody looked
    /// at.
    fn stop_session(&self, internal_session: &str) -> std::result::Result<SessionStop, String> {
        self.stop_session_by(internal_session, signal_to_stop)
    }

    /// **The declared hurdle, run where the sessions run** (nxf 6j6v.n92p): `sh -c <command>` in
    /// [`self.cwd`](SidecarWorker::cwd) — the WORKSPACE ROOT (nxf 6j6v.npn0), which is the checkout
    /// a triggered session is `chdir`'d into. A hurdle asking about the working copy and a session
    /// writing into it therefore see the same directory, by construction rather than by agreement.
    ///
    /// `stdin` is `/dev/null`: a hurdle is a read, and a command that decides to ask a question
    /// would otherwise block until the bound with nobody able to answer it.
    ///
    /// **Bounded by killing, and the shape of the wait is what makes that safe.** Each pipe is
    /// drained by its own thread, so a command that prints more than a pipe buffer holds cannot
    /// deadlock the wait; the child itself stays owned HERE, so the bound is enforced with
    /// [`std::process::Child::kill`] on a handle rather than with a signal to a raw pid, and a pid
    /// this process has already reaped can never be signalled by mistake.
    ///
    /// **The environment is CLEARED and rebuilt from [`hurdle_env`]** — see that function for why
    /// this seam needs it even harder than [`trigger`](Worker::trigger) does.
    fn run_precondition(&self, command: &str) -> crate::precondition::PreconditionOutcome {
        self.run_precondition_within(command, PRECONDITION_BOUND)
    }

    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        // Which `claude` the SDK will drive (nxf 6j6v.81v5), resolved BEFORE anything is written:
        // for the shipped bundle an unresolvable one is a certain failure, and a failure that can
        // only be discovered inside a detached child is no better than the bug this ticket exists
        // to fix — `trigger` returns `Accepted` unconditionally, so `nxc send --to` would print a
        // receipt, exit 0, and leave the SDK's own "Native CLI binary not found. Reinstall
        // @anthropic-ai/claude-agent-sdk without --omit=optional" in a log file nothing points at.
        let claude = claude_on_path(std::env::var("PATH").ok());
        if claude.is_none() && self.is_bundled_sidecar() {
            return Err(TriggerError::Failed(NxfError::validation(
                "Claude Code is not installed, or not on this shell's PATH: the shipped agent \
                 sidecar drives the `claude` executable and cannot find one. Install Claude Code \
                 (https://claude.com/claude-code) and make sure `claude` runs in this shell.",
            )));
        }
        let logs = agent_logs_dir(&self.cwd);
        std::fs::create_dir_all(&logs).map_err(|e| NxfError::io(format!("agent-logs: {e}")))?;
        // env for the session: the ambient stamp so in-session `nxc` resolves this role + session.
        let mut env: serde_json::Map<String, serde_json::Value> = req
            .env
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        env.insert("NXC_SESSION".into(), req.internal_session.clone().into());
        env.insert("NXC_ACTOR".into(), req.role.handle.clone().into());
        let spec = serde_json::json!({
            "session": req.internal_session, "resume": req.resume_real, "role": req.role.handle,
            "message": req.message, "systemPrompt": req.role.system_prompt,
            "useClaudeCodePreset": req.role.use_claude_code_preset, "tools": req.role.tools,
            // **What the engine GRANTED, kept apart from what the role DECLARED** (nxf 6j6v.kffm).
            // `tools` above is the author's; this is what the coordinator added because it demands
            // something of this session. The sidecar unions the two — see
            // `RoleSpec::granted_tools` for which of the SDK's two lists each one reaches, and why
            // widening `tools` itself would have shrunk an undeclared role's base toolset. An
            // older sidecar ignores a key it does not know, which leaves it exactly as broken as it
            // was and no worse.
            "grantedTools": req.role.granted_tools,
            "permissions": req.role.permissions,
            "model": req.role.model.map(|m| m.sdk_id()),
            // `null` on every trigger that carries no obligation to answer anyone — see
            // `TriggerRequest::reply_thread`'s own doc for which those are. `agent-sidecar/src/
            // main.mjs`'s teardown treats `null` as "nothing to settle" and skips its new step
            // entirely (nxf 6j6v.7e9d, ticket 8).
            "replyThread": req.reply_thread,
            // **The coordinator's terms, travelling with the task** (nxf 6j6v.ntp9). The sidecar
            // used to hold `MAX_REPLY_REMINDERS` as a constant of its own, which made the bound a
            // property of the bundled runtime rather than of the engine — so a host installing its
            // own `WorkerConfig::Custom` inherited nothing and had to re-decide it. These travel
            // instead. An older sidecar ignores keys it does not know; a newer one reads them with
            // its own fallbacks, so neither half has to move first.
            "replyReminders": req.terms.reply_reminders,
            "runtimeRetries": req.terms.runtime_retries,
            "retryBackoffMs": req.terms.retry_backoff_ms,
            // Which named place admitted this spawn (nxf 6j6v.ntp9). Not read by the sidecar — it
            // is here so a run's own spec file says who commissioned it, which is the first thing
            // anyone reading `.nxs/agent-logs/<session>.spec.json` after a bad round wants to know.
            "coordinator": req.coordinator,
            // Which VERSION of the declaration this prompt was built from (nxf 6j6v.pkw9). Not read
            // by the sidecar either, and here for the same reason as `coordinator` one line up: the
            // second thing anyone reading a spec file after a bad round wants to know is whether the
            // session ran under the rules they wrote. See `RoleSpec::declaration_hash`.
            "declarationHash": req.role.declaration_hash,
            "cwd": self.cwd, "env": env,
            // The host states which `claude` it resolved, because the SHIPPED sidecar — bundled
            // into one file, no `node_modules` — cannot resolve the SDK's native binary itself.
            // `null` only reaches here for a SOURCE-TREE sidecar (the guard above refuses the
            // bundled one outright), where the SDK's own resolution works and must stay in charge.
            "claudePath": claude,
        });
        // BEFORE the spec is written, which is the second half of nxf 6j6v.7qtf: there is exactly
        // ONE spec file per session and every trigger rewrites it, so a second spawn did not merely
        // add a process — it changed the truth the FIRST process was still reading. The coder in the
        // proving ground read `"resume": null`, and minutes later the same file said
        // `"resume": "ea9ab5e9-…"`. A refused trigger never reaches this write.
        let lock = SessionLock::acquire(&logs, &req.internal_session)?;
        let spec_path = logs.join(format!("{}.spec.json", req.internal_session));
        std::fs::write(&spec_path, spec.to_string())
            .map_err(|e| NxfError::io(format!("spec: {e}")))?;
        let log = std::fs::File::create(logs.join(format!("{}.log", req.internal_session)))
            .map_err(|e| NxfError::io(format!("log: {e}")))?;
        let err = log
            .try_clone()
            .map_err(|e| NxfError::io(format!("log: {e}")))?;
        // Detached: spawn and DO NOT wait — the worker outlives this `send` (spec §5.1).
        //
        // Integrity finding #4 (2026-07-19 review): without `.env_clear()`, the spawned `node`
        // process inherits the OPERATOR's entire shell environment — `main.mjs` then merges
        // `process.env` into the live SDK session's OWN env (`options.env`, what the Bash tool
        // sees), so any Bash-enabled role could read whatever secrets happen to be set in the
        // caller's shell, not just the documented NXC_* stamps. Hardened to an explicit
        // allowlist: `PATH` (node/nxc resolution — the whole role-runtime spawn chain depends on
        // it, proved live across Tasks 1/2/8's smokes) and `HOME` + `USER` (the Claude Agent SDK's
        // ambient auth — the smokes' `apiKeySource:"none"` behavior depends on it resolving).
        // WHERE that credential lives is PLATFORM-DEPENDENT, and this comment used to say only
        // `~/.claude/`: on macOS it is in the login keyring (`security find-generic-password -s
        // "Claude Code-credentials"`), there is no `~/.claude/.credentials.json` at all, and
        // reaching the keyring resolves through `USER`. Without it EVERY spawned persona session
        // on macOS died at authentication — and said so as `Failed to authenticate: OAuth session
        // expired and could not be refreshed`, which sends a reader to `claude login` rather than
        // to the environment (nxf 6j6v.zq0t; bisected `env -i` with one variable at a time, in
        // both directions: `PATH HOME` fails, `PATH HOME LOGNAME` fails, `PATH HOME USER`
        // succeeds). `USER` discloses nothing `HOME` does not already carry. Added on Task 4
        // (2026-07-20): `NXC_SIDECAR`
        // (the path to `agent-sidecar/src/main.mjs`) and `NXC_WORKER` (worker selection, e.g.
        // `sidecar`/`dry`), so that a role's own nested `nxc send --to <sub>` calls can
        // resolve the Worker for the next hop without an error. Also forwarded explicitly via
        // `req.env`: `NXC_ORIGIN`/`NXC_DB`/`NXC_HOP` (from `trigger_env`) and the two inserted
        // above (`NXC_SESSION`/`NXC_ACTOR`). Nothing else from the operator's shell reaches the
        // child. Do not widen this allowlist without a concrete, stated reason.
        let mut cmd = Command::new("node");
        cmd.arg(&self.sidecar)
            .arg("--spec")
            .arg(&spec_path)
            .current_dir(&self.cwd)
            .env_clear();
        for (k, v) in forwarded_real_env(|key| std::env::var(key).ok()) {
            cmd.env(k, v);
        }
        for (k, v) in &req.env {
            cmd.env(k, v);
        }
        cmd.env("NXC_SESSION", &req.internal_session)
            .env("NXC_ACTOR", &req.role.handle)
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(err);
        let child = cmd
            .spawn()
            .map_err(|e| NxfError::io(format!("spawning sidecar: {e}")))?;
        // The claim moves from this short-lived `nxc` invocation to the detached child that will
        // outlive it — the process whose liveness is what "this session is running" actually means.
        lock.hand_to(child.id());
        // `Accepted`, never `Started`: the detached `node` has not reached the SDK yet, so the real
        // session id does not exist to hand back. It arrives later, from inside the session, via
        // `nxc session bind` — the local instance of the out-of-band completion every remote
        // runtime needs too (see [`Worker`]'s own doc).
        Ok(TriggerOutcome::Accepted)
    }
}

/// Determines which real ambient environment variables to forward to spawned role sessions.
///
/// Takes an explicit ambient-lookup closure (dependency injection) so the decision is pure and
/// independently unit-testable without mutating process-global env. The closure receives an env
/// var key and returns `Some(value)` if present, `None` if unset.
fn forwarded_real_env(ambient: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    ["PATH", "HOME", "USER", "NXC_SIDECAR", "NXC_WORKER"]
        .iter()
        .filter_map(|k| ambient(k).map(|v| (k.to_string(), v)))
        .collect()
}

/// **What a declared HURDLE's shell may see** (nxf 6j6v.n92p; PR #391 review, Integrity #1) — the
/// same `.env_clear()` + allowlist discipline [`forwarded_real_env`] states one seam over, and a
/// STRICTLY NARROWER list.
///
/// **Why it needs it even harder than a spawned session does.** The 2026-07-19 finding that hardened
/// [`Worker::trigger`] was: without clearing, a Bash-enabled role could read whatever secrets
/// happened to be in the operator's shell. A hurdle is worse in one specific way — `preconditions:`
/// lives in `.nxs-personas/`, in the working copy the gated personas are told to work in, so a
/// persona can WRITE the command the COORDINATOR then executes. Inheriting the engine's environment
/// there would hand a persona a shell outside its own sandbox, which is an escalation past the very
/// boundary this item exists to draw. A hurdle that this list starves is a hurdle that refuses —
/// fail-closed, visibly — and that is the right direction for a rule nobody can audit.
///
/// **Two variables, and each earns its place:**
///
/// * `PATH` — without it `/bin/sh` finds no `git`, and every hurdle anybody has written is a git
///   plumbing call. This is the one that makes the feature exist at all.
/// * `HOME` — `git` reads `~/.gitconfig` for `safe.directory` and for `core.*`; without it a
///   perfectly clean checkout can answer `detected dubious ownership` on some hosts, which is a
///   refusal for a reason that has nothing to do with the project's rule.
///
/// **Deliberately NOT forwarded, and each omission is a decision:** `USER` (the sibling list carries
/// it for the Claude SDK's keyring auth; a hurdle authenticates to nothing), `NXC_SIDECAR` /
/// `NXC_WORKER` (they exist so a spawned session can start the NEXT hop — a hurdle starts nothing),
/// and `NXC_DB` / `NXC_ORIGIN` / `NXC_SESSION` / `NXC_ACTOR` (a hurdle is a question about the
/// WORKING COPY, and handing it the workspace's own coordinates would let a declared command reach
/// back into the board that is asking it). Do not widen this list without a concrete, stated reason.
///
/// Takes the ambient lookup as a closure for [`forwarded_real_env`]'s reason: the decision stays
/// pure and testable without mutating process-global env.
fn hurdle_env(ambient: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    ["PATH", "HOME"]
        .iter()
        .filter_map(|k| ambient(k).map(|v| (k.to_string(), v)))
        .collect()
}

/// Which worker to build, as a value rather than a read of process env (nxf 6j6v.a5na). This is
/// what lets a library caller SAY which worker it wants — the CLI keeps deriving it from
/// `NXC_WORKER`/`NXC_SIDECAR` via [`WorkerConfig::from_ambient`], and an embedding app states it
/// outright.
/// `#[non_exhaustive]` from now on (nxf 6j6v.5x9j): adding [`Custom`](WorkerConfig::Custom) already
/// costs downstream exhaustive `match`es this once, so the same cut buys every later variant for
/// free — the reasoning [`crate::role::Model`] and [`crate::orchestration::WakeSkipReason`] already
/// carry. Construction is unaffected; only an exhaustive `match` outside this crate must gain a
/// wildcard arm.
#[derive(Clone)]
#[non_exhaustive]
pub enum WorkerConfig {
    /// No worker at all: reads and messaging writes work, and any verb that would spawn a role
    /// session fails loudly (see [`WorkerConfig::build`]). This is what [`crate::engine::Engine`]'s
    /// plain `open` gives an embedder that never opted into orchestration — a spawn attempt has to
    /// say so out loud rather than silently doing nothing behind a successful receipt.
    ///
    /// **"Fails loudly" splits into two shapes since nxf 6j6v.hpv8** — see `engine.rs`'s
    /// `DisabledWorker` for exactly which verbs still return the named error outright and which
    /// ones report the SAME refusal through their receipt's `spawned: false` +
    /// `crate::orchestration::TriggerReceipt::warnings` instead, because by the time they reach a
    /// worker at all their message is already durably persisted.
    Disabled,
    /// The recording worker, and WHERE it records (nxf 6j6v.570x). `log: None` records nothing.
    ///
    /// The path is carried here rather than read from `NXC_DRY_LOG` inside
    /// [`DryWorker::trigger`] — see that type's own doc for the flake that forced it. The CLI still
    /// resolves the variable, once, in [`from_ambient_or_installed`](Self::from_ambient_or_installed);
    /// an in-process caller says the path outright and is then immune to what any other thread has
    /// put in the environment.
    Dry {
        log: Option<PathBuf>,
    },
    Sidecar {
        sidecar: PathBuf,
        cwd: PathBuf,
    },
    /// **The escape hatch spec §3.3 always promised** (nxf 6j6v.5x9j): the host brings its own
    /// [`Worker`] and the engine drives it exactly as it drives the bundled two.
    ///
    /// This is how a SECOND agent runtime docks onto the seam — the concrete case it was built
    /// against is a remote one (Bedrock AgentCore, probed live in app-foundations 41j0.9t68), which
    /// is why the trait around it deals in an opaque runtime session id and an outcome that may
    /// resolve later rather than in a `std::process::Command`. The engine keeps shipping a default
    /// arm so it is usable on its own; the apps bring their own through here.
    ///
    /// It stays inside the IP boundary the platform draws: the engine defines **what** a role is
    /// and when it runs, never **who** executes it or where.
    Custom(Arc<dyn Worker>),
}

/// Hand-written because [`WorkerConfig::Custom`] holds a `dyn Worker`, which has no `Debug`. The
/// other three render exactly as the derive did — an `EngineConfig` ends up in app logs and those
/// lines are not this ticket's to change; `Custom` says only that a host worker is installed,
/// because there is nothing else it can truthfully say.
impl std::fmt::Debug for WorkerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerConfig::Disabled => f.write_str("Disabled"),
            WorkerConfig::Dry { log } => f.debug_struct("Dry").field("log", log).finish(),
            WorkerConfig::Sidecar { sidecar, cwd } => f
                .debug_struct("Sidecar")
                .field("sidecar", sidecar)
                .field("cwd", cwd)
                .finish(),
            WorkerConfig::Custom(_) => f.write_str("Custom(..)"),
        }
    }
}

/// Hand-written for the same reason, and `Eq` stays honest because [`Arc::ptr_eq`] is reflexive:
/// "the same worker" is the only comparison a trait object admits, and it is the one an embedder
/// asking `is this handle still configured the way I configured it?` actually means.
impl PartialEq for WorkerConfig {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (WorkerConfig::Disabled, WorkerConfig::Disabled) => true,
            (WorkerConfig::Dry { log: a }, WorkerConfig::Dry { log: b }) => a == b,
            (
                WorkerConfig::Sidecar { sidecar: a, cwd: b },
                WorkerConfig::Sidecar { sidecar: c, cwd: d },
            ) => a == c && b == d,
            (WorkerConfig::Custom(a), WorkerConfig::Custom(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for WorkerConfig {}

impl WorkerConfig {
    /// The CLI's selection rules, unchanged, over an INJECTED ambient lookup rather than
    /// `std::env::var` directly — the same dependency-injection shape [`forwarded_real_env`] uses,
    /// so the decision is pure and unit-testable without mutating process-global env (which
    /// parallel tests in one binary would otherwise race on).
    ///
    /// Never yields [`WorkerConfig::Disabled`]: "no worker" is a deliberate library-side choice,
    /// not something an unset environment variable should silently produce.
    /// Looks at the environment and NOTHING else — in particular it does not go looking for a
    /// shipped sidecar, and its refusal says so, naming only the variable it actually consulted.
    /// A caller that wants the installed one asks for it: [`from_ambient_or_installed`].
    ///
    /// [`from_ambient_or_installed`]: WorkerConfig::from_ambient_or_installed
    pub fn from_ambient(
        cwd: PathBuf,
        ambient: impl Fn(&str) -> Option<String>,
    ) -> Result<WorkerConfig> {
        WorkerConfig::from_ambient_or_installed(cwd, ambient, || Ok(None)).map_err(|e| {
            // The generic refusal describes a lookup this entry point did not perform ("expected
            // `nxc-agent-sidecar.mjs` beside the running nxs binary") — true for the CLI, a lie to
            // a library embedder that only ever handed us an env lookup. Same failure, its own
            // words.
            if e.msg.contains(SIDECAR_FILE) {
                NxfError::io(
                    "NXC_SIDECAR (path to agent-sidecar/src/main.mjs) is not set, and this \
                     selection consults nothing else",
                )
            } else {
                e
            }
        })
    }

    /// [`from_ambient`](WorkerConfig::from_ambient) plus the program's OWN sidecar as the fallback
    /// when `NXC_SIDECAR` says nothing (nxf 6j6v.81v5, 6j6v.smsz).
    ///
    /// The environment variable is a DEVELOPER OVERRIDE, not a delivery route: an installed
    /// nexus-flow carries its own sidecar and must find it without anyone exporting anything. Which
    /// is precisely the host's half of the division — *resolving a name to an installed thing is
    /// the host's job* — so the lookup is INJECTED here rather than performed: this function stays
    /// as pure and as testable as `from_ambient` is, and [`select_worker`] (the CLI adapter, i.e.
    /// the host) is the one place that reaches for the running program's own delivery.
    ///
    /// `installed` is only consulted when `NXC_SIDECAR` is unset, and only on the sidecar path — a
    /// `dry` worker never looks for a file, and an override never gets second-guessed. It answers
    /// with a `Result` because the lookup itself can fail now that the sidecar is compiled into the
    /// binary and unpacked on first use (6j6v.smsz): "I have one and cannot make it usable" is not
    /// "I have none", and a failure there is PROPAGATED rather than folded into the generic refusal
    /// below — which would name a file nobody is missing.
    pub fn from_ambient_or_installed(
        cwd: PathBuf,
        ambient: impl Fn(&str) -> Option<String>,
        installed: impl FnOnce() -> Result<Option<PathBuf>>,
    ) -> Result<WorkerConfig> {
        match ambient("NXC_WORKER").as_deref() {
            // The ONE place `NXC_DRY_LOG` is read, and it is read through the SAME injected lookup
            // as every other variable here — so the decision stays pure, and a caller that never
            // hands us an environment never gets one (nxf 6j6v.570x).
            Some("dry") => Ok(WorkerConfig::Dry {
                log: ambient("NXC_DRY_LOG").map(PathBuf::from),
            }),
            Some("sidecar") | None => {
                let sidecar = match ambient("NXC_SIDECAR").map(PathBuf::from) {
                    Some(overridden) => overridden,
                    None => installed()?.ok_or_else(|| {
                        NxfError::io(format!(
                            "no agent sidecar found: this nxs was built without the bundled \
                             `{SIDECAR_FILE}` compiled in (build it with `npm ci && npm run build` \
                             in agent-sidecar), there is none beside the running binary, and \
                             NXC_SIDECAR — the developer override, a path to \
                             agent-sidecar/src/main.mjs — is not set"
                        ))
                    })?,
                };
                Ok(WorkerConfig::Sidecar { sidecar, cwd })
            }
            Some(other) => Err(NxfError::io(format!("unknown NXC_WORKER '{other}'"))),
        }
    }

    /// Build the worker. `Arc` (not `Box`) because a long-lived owner shares it across clones of
    /// itself — `Engine` is `Clone` and every clone drives the same worker.
    ///
    /// [`WorkerConfig::Disabled`] has no worker to build and says so — see [`disabled_error`] for
    /// the one place that refusal is worded, and `engine.rs`'s `DisabledWorker` for why the handle
    /// defers it to the moment a spawn is actually attempted rather than to open time.
    pub fn build(&self) -> Result<Arc<dyn Worker>> {
        match self {
            WorkerConfig::Disabled => Err(disabled_error()),
            WorkerConfig::Dry { log } => Ok(Arc::new(DryWorker { log: log.clone() })),
            WorkerConfig::Sidecar { sidecar, cwd } => Ok(Arc::new(SidecarWorker {
                sidecar: sidecar.clone(),
                cwd: cwd.clone(),
            })),
            // The host's OWN worker, behind the bound that makes third-party code inside the
            // engine's locked section survivable (see [`BoundedWorker`]). It DELEGATES rather than
            // replacing: a host that installs a worker and then observes it from the outside (its
            // own logs, its own kill switch, its own counters) is still watching the object the
            // engine calls.
            WorkerConfig::Custom(worker) => Ok(Arc::new(BoundedWorker {
                inner: worker.clone(),
            })),
        }
    }
}

/// The ONE wording of "this configuration has no worker", so the refusal a caller sees is the same
/// string whether it came from [`WorkerConfig::build`] or from the [`crate::engine::Engine`]'s own
/// refusing worker (which produces it at trigger time instead of at open time — see that type's own
/// doc). A `validation` kind, not `io`: nothing failed, the caller simply did not ask for
/// orchestration.
pub(crate) fn disabled_error() -> NxfError {
    NxfError::validation(
        "no worker is configured, so no role session can be started; \
         open the engine with an explicit WorkerConfig to use the orchestration verbs",
    )
}

/// The CLI's worker: [`WorkerConfig::from_ambient_or_installed`] over the real process env and the
/// running program's own location, then built.
pub fn select_worker(cwd: PathBuf) -> Result<Arc<dyn Worker>> {
    WorkerConfig::from_ambient_or_installed(cwd, |key| std::env::var(key).ok(), installed_sidecar)?
        .build()
}

/// The file name of the bundled sidecar: what `agent-sidecar`'s `npm run build` produces, what
/// [`unpack_sidecar`] writes into the cache, and what `install.sh`/`nxs self-update` still
/// reconcile beside the `nxs` binary for installs from <=0.53.0. A single self-contained ESM bundle
/// (the SDK is bundled in), so there is one file and no `node_modules` to carry.
pub const SIDECAR_FILE: &str = "nxc-agent-sidecar.mjs";

// `EMBEDDED_SIDECAR: Option<&[u8]>` — the sidecar bundle compiled INTO this program (nxf
// 6j6v.smsz), or `None` for a build that had none to compile in. Generated by `build.rs`, which
// embeds `agent-sidecar/dist/nxc-agent-sidecar.mjs` when it exists — every release, and any
// checkout that has run `npm run build` — and emits `None` otherwise, so a contributor without Node
// still builds. (A `///` doc comment cannot describe a macro invocation, hence `//`.)
include!(concat!(env!("OUT_DIR"), "/embedded_sidecar.rs"));

/// The sidecar this program will actually run, or `None` if it has none at all.
///
/// The host's half of the division, performed: the one place that reads `current_exe()`.
///
/// **The embedded bundle comes first, and that ordering is the point of nxf 6j6v.smsz.** It is the
/// sidecar this exact binary was built with, so binary and sidecar are one pair by construction —
/// there is no second file for an installer to forget, no older updater that had to know about it,
/// and nothing left to drift (which is what makes nxf 6j6v.5vct moot rather than merely unlikely).
/// A file BESIDE the binary is still consulted after it, for two cases that both deserve to keep
/// working: an install from <=0.53.0 that still has one lying there, and a `cargo build` in a
/// checkout, where [`sidecar_beside`] reaches for the source tree's `agent-sidecar/src/main.mjs`.
///
/// `Err` is reserved for "this program HAS a sidecar and could not make it usable" — an unwritable
/// cache, nowhere to write at all. That is a different fact from `Ok(None)` ("this program has
/// none"), and collapsing the two would send a reader looking for a file that is not missing.
pub(crate) fn installed_sidecar() -> Result<Option<PathBuf>> {
    installed_sidecar_from(
        EMBEDDED_SIDECAR,
        |key| std::env::var_os(key),
        || std::env::current_exe().ok(),
    )
}

/// [`installed_sidecar`] over an injected bundle, environment and running-executable path — the
/// same dependency-injection shape the rest of this module uses.
///
/// It exists so the ORDER itself is testable (PR #302 review, Test Quality #1). The precedence is
/// the whole of what nxf 6j6v.smsz buys, and it used to be asserted only in the prose above: every
/// test injected a lookup or called the unpack helpers directly, so a regression that swapped the
/// two arms would have passed the entire suite.
fn installed_sidecar_from(
    bundle: Option<&[u8]>,
    ambient: impl Fn(&str) -> Option<std::ffi::OsString>,
    running_exe: impl FnOnce() -> Option<PathBuf>,
) -> Result<Option<PathBuf>> {
    if let Some(embedded) = embedded_sidecar_from(bundle, ambient)? {
        return Ok(Some(embedded));
    }
    Ok(running_exe().and_then(|exe| sidecar_beside(&exe)))
}

/// The embedded bundle, unpacked into the user's cache and ready for `node` — over an injected
/// bundle and an injected environment, the same dependency-injection shape the rest of this module
/// uses, so the decision is testable without a particular build and without mutating process-global
/// env.
fn embedded_sidecar_from(
    bundle: Option<&[u8]>,
    ambient: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<Option<PathBuf>> {
    let Some(bytes) = bundle else {
        return Ok(None);
    };
    let root = cache_root(ambient).ok_or_else(|| {
        NxfError::io(
            "the nxc agent sidecar is built into this nxs, but there is nowhere to unpack it to: \
             none of NXF_CACHE_DIR, XDG_CACHE_HOME or HOME names a directory. Set NXF_CACHE_DIR to \
             a writable location.",
        )
    })?;
    unpack_sidecar(bytes, &root).map(Some)
}

/// The nexus-flow cache directory: `NXF_CACHE_DIR` (override), else `XDG_CACHE_HOME/nxf`, else
/// `~/.cache/nxf`.
///
/// Deliberately the SAME ladder `nxs self-update`'s update-hint cache walks
/// (`crates/cli/src/selfupdate.rs::cache_dir`), spelled again here because chat cannot depend on
/// the CLI: one machine gets one nexus-flow cache, and an operator who redirects it redirects all
/// of it. An empty value is not a location.
fn cache_root(ambient: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    let named = |key: &str| ambient(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    named("NXF_CACHE_DIR")
        .or_else(|| named("XDG_CACHE_HOME").map(|x| x.join("nxf")))
        .or_else(|| named("HOME").map(|h| h.join(".cache").join("nxf")))
}

/// Write `bytes` into `<cache_root>/sidecar/<content-id>/nxc-agent-sidecar.mjs` and answer with
/// that path, reusing what is already there.
///
/// **Content-addressed, not version-addressed.** The directory is named after the bundle's own
/// hash, so a rebuilt bundle at an unchanged version cannot land on the entry the previous one
/// wrote, and two binaries carrying the same bundle share one file. That is also the whole of what
/// nxf 6j6v.5vct asked for: skew between binary and sidecar is not defended against here, it is
/// unconstructible.
///
/// **The file keeps the SHIPPED name** inside that directory, because
/// [`SidecarWorker::is_bundled_sidecar`] keys on it — an unpacked bundle has no `node_modules`
/// beside it either, so it needs the same "the host resolves `claude` for you" precheck a bundle
/// placed by an installer needed.
fn unpack_sidecar(bytes: &[u8], cache_root: &std::path::Path) -> Result<PathBuf> {
    let dir = cache_root.join("sidecar").join(content_id(bytes));
    let file = dir.join(SIDECAR_FILE);
    // Already unpacked by an earlier run (or by another process a moment ago) — reused only if the
    // BYTES match, not merely the length (PR #302 review, Integrity & Robustness #1).
    //
    // The path is public knowledge: the bundle ships inside a signed binary anyone can download, so
    // anyone can compute the hash that names this directory. A length check would let whoever can
    // write into the cache first — a shared `NXF_CACHE_DIR`, a synced `$HOME/.cache`, a
    // multi-tenant box — pre-plant a same-length file at a path we would then hand straight to
    // `node`. Comparing the content costs one read of 1.4 MB on a path that only runs when an agent
    // is being started, which is the cheaper side of that trade by a wide margin.
    if std::fs::read(&file).is_ok_and(|found| found == bytes) {
        return Ok(file);
    }
    std::fs::create_dir_all(&dir).map_err(|e| cannot_unpack(&file, e))?;
    // Owner-only, so the pre-planting above needs the user's own account rather than any account on
    // the machine. Best-effort and deliberately not fatal: a cache root on a filesystem with no
    // unix modes (a mounted share) is not a reason to refuse to start an agent, and the content
    // comparison above is what actually decides whether the file is used.
    restrict_to_owner(&dir);
    // Write-then-rename, with the temporary in the SAME directory so the rename is atomic: two
    // `nxc send`s racing must never hand `node` a half-written file. The name is unique per process
    // AND per call, so two threads of one process do not collide either.
    static UNPACK_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = UNPACK_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{SIDECAR_FILE}.{}.{seq}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| cannot_unpack(&file, e))?;
    std::fs::rename(&tmp, &file).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        cannot_unpack(&file, e)
    })?;
    Ok(file)
}

/// Narrow `dir` to owner-only (`0700`) where the platform has such a thing. Best-effort by design —
/// see the call site for why a failure here is not a reason to refuse.
#[cfg(unix)]
fn restrict_to_owner(dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict_to_owner(_dir: &std::path::Path) {}

/// The refusal when the sidecar is here and cannot be made usable — acceptance 4 of nxf 6j6v.smsz.
/// A silent failure at this exact spot is the class of bug the whole ticket exists to remove, so it
/// names the path it tried, why a file is needed at all, and both ways out.
fn cannot_unpack(file: &std::path::Path, e: std::io::Error) -> NxfError {
    NxfError::io(format!(
        "the nxc agent sidecar is built into this nxs, but it could not be unpacked to {}: {e}\n\
         `node` runs it from there, so it needs a writable cache directory — point NXF_CACHE_DIR at \
         one, or set NXC_SIDECAR to a checked-out agent-sidecar/src/main.mjs.",
        file.display()
    ))
}

/// The first 16 hex chars of `sha256(bundle)` — enough to name one build's bundle apart from every
/// other, and short enough that the cache path stays readable in an error message.
fn content_id(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hex: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    hex[..16].to_string()
}

/// The sidecar that belongs to the program at `exe`, searched in a fixed order.
///
/// Pure in its decision and honest about the filesystem: every candidate is returned only if it
/// EXISTS, so the answer is "the sidecar this program ships", never a hopeful path. The answer is
/// canonicalized, so what reaches `node` names one file by one route.
///
/// **Symlinks are the trap this is written around.** `nxc` is a symlink to `nxs`
/// (`~/.local/bin/nxc -> nxs`), and `current_exe()` resolves symlinks on some platforms and not on
/// others — on macOS it hands back the path as invoked. A resolution that assumed either behaviour
/// would work for `nxs` and fail for `nxc`, which is the command the whole surface is named after;
/// that is exactly how the same resolution broke in manufakt.io on 2026-08-05. So both the literal
/// directory and the canonicalized one are searched.
///
/// **Every SHIPPED candidate is tried before any SOURCE-TREE one**, across both directories rather
/// than within each — otherwise an install whose bundle is missing (a pre-81v5 release, or a
/// tolerated skip) would reach for `<dir>/../../agent-sidecar/src/main.mjs`, which from
/// `~/.local/bin` is `$HOME/agent-sidecar/src/main.mjs`: a predictable path in the user's own home
/// with nothing marking it as belonging to this program. The source-tree candidate is additionally
/// only offered from a directory that actually looks like a Cargo output dir
/// (`…/target/{debug,release}`), which is the only place it was ever meant for.
fn sidecar_beside(exe: &std::path::Path) -> Option<PathBuf> {
    let literal = exe
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from);
    let canonical = std::fs::canonicalize(exe)
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .filter(|p| Some(p) != literal.as_ref());
    let dirs: Vec<PathBuf> = [literal, canonical].into_iter().flatten().collect();
    let shipped = dirs.iter().map(|dir| dir.join(SIDECAR_FILE));
    let source_tree = dirs
        .iter()
        .filter(|dir| is_cargo_out_dir(dir))
        .filter_map(|dir| dir.parent().and_then(|p| p.parent()))
        .map(|root| root.join("agent-sidecar/src/main.mjs"));
    shipped
        .chain(source_tree)
        .find(|candidate| candidate.is_file())
        .map(|found| std::fs::canonicalize(&found).unwrap_or(found))
}

/// Does `dir` look like `…/target/{debug,release}` — the only shape the source-tree fallback was
/// ever meant for? Profile names beyond the two built-ins are deliberately not matched: a custom
/// profile is a developer who can set `NXC_SIDECAR`, and every extra name widens the one candidate
/// that points outside the program's own install.
fn is_cargo_out_dir(dir: &std::path::Path) -> bool {
    let profile_is_known = dir
        .file_name()
        .is_some_and(|n| n == "debug" || n == "release");
    let parent_is_target = dir
        .parent()
        .and_then(|p| p.file_name())
        .is_some_and(|n| n == "target");
    profile_is_known && parent_is_target
}

/// The installed Claude Code executable, looked up on `path_var` **the way a shell would**: the
/// first entry holding a file named `claude` that is actually executable wins, and a non-executable
/// one is skipped rather than selected — otherwise a plain data file called `claude`, sitting in the
/// workspace being operated on (`PATH=/usr/bin:` yields an empty component, i.e. the CWD), would
/// shadow the real binary and the run would die on the SDK's own opaque spawn failure.
///
/// The host's half again, and the reason the shipped sidecar can start a session at all: bundled
/// into one file it has no `node_modules`, so the SDK's own `require.resolve` of the native binary
/// finds nothing. `None` — nothing usable named `claude` on the PATH — is refused outright for the
/// bundled sidecar ([`SidecarWorker::trigger`]) and leaves the SDK's own resolution in charge for a
/// source-tree one, which is what a developer run wants.
///
/// Takes the raw PATH value rather than reading the environment, so the decision is unit-testable.
pub(crate) fn claude_on_path(path_var: Option<String>) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    std::env::split_paths(&path_var?)
        .map(|dir| dir.join(exe))
        .find(|candidate| is_executable_file(candidate))
}

/// A file that could actually be run. On unix that means any execute bit; elsewhere the bit does
/// not exist and a file is a file.
fn is_executable_file(path: &std::path::Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    // ---- the single-process guard (nxf 6j6v.7qtf) ---------------------------------------------
    //
    // Driven against `SessionLock` directly rather than through `SidecarWorker::trigger`, because
    // what has to be proved is the decision — "is somebody already running this session?" — and
    // reaching it through the worker would mean spawning a real `node` and hoping it stays alive
    // long enough to be observed. A real long-lived child (`sleep`) stands in for the sidecar, so
    // `process_is_alive` is exercised against a genuine pid in both of its answers.

    /// A real, live process to hold a claim with, plus the guarantee it is reaped.
    struct LiveProcess(std::process::Child);
    impl LiveProcess {
        fn spawn() -> LiveProcess {
            LiveProcess(
                Command::new("sleep")
                    .arg("30")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("spawning `sleep` (assumed present, as the timer tests already do)"),
            )
        }
        fn pid(&self) -> u32 {
            self.0.id()
        }
        fn kill_and_reap(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    impl Drop for LiveProcess {
        fn drop(&mut self) {
            self.kill_and_reap();
        }
    }

    #[test]
    fn a_session_with_no_claim_at_all_is_acquired() {
        let tmp = tempfile::tempdir().unwrap();
        SessionLock::acquire(tmp.path(), "m-sess").expect("nobody holds it");
        assert!(
            tmp.path().join("m-sess.pid").exists(),
            "the claim is a file, so a SECOND process can see it"
        );
    }

    #[test]
    fn a_second_trigger_is_refused_while_the_first_process_is_alive() {
        // The defect itself, in one assertion: this is the moment two `claude` processes used to
        // start on one spec, in one working directory, and overwrite each other's edits.
        let tmp = tempfile::tempdir().unwrap();
        let held = LiveProcess::spawn();
        let lock = SessionLock::acquire(tmp.path(), "m-sess").expect("the first one gets it");
        lock.hand_to(held.pid());

        let err = SessionLock::acquire(tmp.path(), "m-sess")
            .expect_err("one internal session, one process");
        match err {
            TriggerError::AlreadyRunning { session, pid } => {
                assert_eq!(session, "m-sess");
                assert_eq!(
                    pid,
                    held.pid(),
                    "the refusal names the process a human can go and look at"
                );
            }
            other => panic!("expected AlreadyRunning, got {other:?}"),
        }
    }

    #[test]
    fn a_claim_whose_process_has_exited_is_taken_over() {
        // Nothing releases a claim — a sidecar that finished, crashed or was killed leaves its pid
        // file behind. The liveness of a real process is the whole release mechanism, which is what
        // makes a crash unable to wedge a session forever.
        let tmp = tempfile::tempdir().unwrap();
        let mut held = LiveProcess::spawn();
        let lock = SessionLock::acquire(tmp.path(), "m-sess").unwrap();
        lock.hand_to(held.pid());
        held.kill_and_reap();

        SessionLock::acquire(tmp.path(), "m-sess")
            .expect("a dead holder is a stale claim, not a locked session");
    }

    #[test]
    fn a_claim_that_does_not_parse_is_treated_as_stale() {
        // The stated residual: assuming a LIVE holder for content that cannot be read would leave a
        // session no trigger could ever start again, which is worse than the case it would guard.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("m-sess.pid"), "not a pid").unwrap();
        SessionLock::acquire(tmp.path(), "m-sess").expect("unreadable is stale");
    }

    #[test]
    fn the_refusal_converts_to_a_validation_error_not_an_io_one() {
        // What a caller that has nothing to decide sees through the `?` conversion: the CALL is
        // wrong (a second start for a session that is working), not the environment.
        let e: NxfError = TriggerError::AlreadyRunning {
            session: "m-sess".into(),
            pid: 4242,
        }
        .into();
        assert_eq!(e.kind, ErrorKind::Validation);
        assert!(e.msg.contains("m-sess"), "{}", e.msg);
        assert!(e.msg.contains("4242"), "{}", e.msg);
    }

    #[test]
    fn process_is_alive_answers_a_live_pid_and_a_dead_one() {
        let mut p = LiveProcess::spawn();
        assert!(process_is_alive(p.pid()), "a running child is alive");
        let pid = p.pid();
        p.kill_and_reap();
        assert!(
            !process_is_alive(pid),
            "a reaped child is gone — if this ever flakes, the claim would never be released"
        );
    }

    // ---- the liveness READ (nxf 6j6v.10yb) ----------------------------------------------------

    #[test]
    fn the_sidecar_worker_reads_a_live_session_off_the_pid_file_its_own_lock_wrote() {
        // No new bookkeeping: the claim `SessionLock` takes at every trigger IS the record of which
        // process is this session, so the liveness answer is that same file read back. Driven
        // against a real live child so `process_is_alive` is exercised for real, in both answers.
        let tmp = tempfile::tempdir().unwrap();
        let logs = tmp.path().join(".nxs/agent-logs");
        std::fs::create_dir_all(&logs).unwrap();
        let worker = SidecarWorker {
            sidecar: tmp.path().join("stub.mjs"),
            cwd: tmp.path().to_path_buf(),
        };

        assert!(
            !worker.session_is_running("m-never-started"),
            "a session this workspace never started has no claim, and no claim is `not running`"
        );

        let mut held = LiveProcess::spawn();
        let lock = SessionLock::acquire(&logs, "m-sess").unwrap();
        lock.hand_to(held.pid());
        assert!(
            worker.session_is_running("m-sess"),
            "while the process behind the claim lives, so does the session"
        );

        held.kill_and_reap();
        assert!(
            !worker.session_is_running("m-sess"),
            "and a claim left behind by a dead process is not a live session — otherwise a hard \
             kill would pin a channel's step open forever"
        );
    }

    #[test]
    fn a_pid_file_that_does_not_parse_reads_as_not_running() {
        // `SessionLock::acquire`'s own residual, answered the same way and for the same reason: the
        // alternative — assuming a live holder — would wedge a flow that nothing could release.
        let tmp = tempfile::tempdir().unwrap();
        let logs = tmp.path().join(".nxs/agent-logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("m-sess.pid"), "not a pid").unwrap();
        let worker = SidecarWorker {
            sidecar: tmp.path().join("stub.mjs"),
            cwd: tmp.path().to_path_buf(),
        };
        assert!(!worker.session_is_running("m-sess"));
    }

    #[test]
    fn a_worker_that_says_nothing_about_liveness_answers_not_running() {
        // The trait default, asserted where it is DECLARED rather than only where it is relied on:
        // a host that does not override it must leave every channel behaving as it did before this
        // method existed. `DryWorker` overrides nothing, so this runs the default body.
        let dry = DryWorker { log: None };
        assert!(!dry.session_is_running("m-anything"));
    }

    #[test]
    fn the_worker_says_it_answers_the_liveness_question_only_where_it_can_actually_look() {
        // The half `session_is_running` cannot say (nxf 6j6v.t41e). Its `false` for `m-never-
        // started` above and its `false` for a session another process holds are the same word, and
        // only the worker knows which one it just said. This is the worker saying it.
        //
        // **And it says it per PLATFORM** (review of PR #409, Integrity #1). `process_is_alive` is
        // two functions: on unix it asks the operating system, and on every other platform it is a
        // stub that answers `false` without asking anything. A worker whose read is that stub must
        // not claim to look — it would leave a host MORE confident in a `false` than before this
        // method existed, which is the exact inversion the method was added to prevent. So the
        // claim is asserted against the CONDITION, not against a literal: the other half of the
        // pair is compile-checked only, since CI's Windows job runs clippy and no tests.
        let tmp = tempfile::tempdir().unwrap();
        let worker = SidecarWorker {
            sidecar: tmp.path().join("stub.mjs"),
            cwd: tmp.path().to_path_buf(),
        };
        assert_eq!(
            worker.answers_liveness(),
            cfg!(unix),
            "where it reads a pid file and asks the OS it may say so; where `process_is_alive` is \
             the blind stub it may not — one condition, spelled once in `LIVENESS_IS_ASKABLE`"
        );
    }

    #[test]
    fn a_worker_that_says_nothing_about_liveness_admits_it_rather_than_claiming_the_answer() {
        // The default, in the same place its sibling default is asserted, and pointing the same
        // way: a worker that cannot look at a process must not have its `false` read as a fact.
        let dry = DryWorker { log: None };
        assert!(
            !dry.answers_liveness(),
            "it starts no process and looks at none, so it cannot tell a live session from a dead \
             one — including one another worker in this workspace started"
        );
    }

    // ---- the one claim parse, and the identity behind it (nxf 6j6v.b9nf) ----------------------

    /// **The decision a signal rests on, pinned without sending one** (fix round 3 of this item's
    /// review, Test Quality #4).
    ///
    /// Until this, the parse was covered only through `SidecarWorker::stop_session` from an
    /// integration test — which meant the pins for `0` and for a value past `i32::MAX` called the
    /// real thing, and a regressed guard would have run `kill(0, SIGTERM)` against the test runner
    /// (see [`a_signalable_pid`]'s own doc: that is a measured event, not a worry). It is a pure
    /// function; this is what pinning it costs.
    #[test]
    fn a_claim_names_one_process_or_nothing() {
        assert_eq!(a_signalable_pid("4242"), Some(4242), "the ordinary case");
        assert_eq!(
            a_signalable_pid("  4242\n"),
            Some(4242),
            "trailing whitespace and the newline every writer leaves"
        );
        assert_eq!(
            a_signalable_pid(&i32::MAX.to_string()),
            Some(i32::MAX as u32),
            "the largest value `pid_t` can hold is a pid, not a group"
        );
        for not_a_pid in [
            "0",                                // the CALLER's own process group
            &(i32::MAX as u32 + 1).to_string(), // wraps negative in `pid_t`
            &u32::MAX.to_string(),              // wraps to -1: every process this user may signal
            "-1",                               // the same thing, written out
            "",                                 // a claim written by a process that died mid-write
            "   ",
            "not a pid",
            "4242 4243",
        ] {
            assert_eq!(
                a_signalable_pid(not_a_pid),
                None,
                "{not_a_pid:?} does not name one process, and nothing may be signalled on it"
            );
        }
    }

    /// The claim file's two halves, written and read back through the one spelling of each — and
    /// what an OLD file (a bare pid, no instant) parses as, since that is the shape every claim had
    /// before identity was recorded and the shape the stop refuses by name.
    #[test]
    fn a_claim_round_trips_and_an_old_one_says_it_records_no_identity() {
        let claim = a_session_claim(&session_claim_for(std::process::id()))
            .expect("what `session_claim_for` writes is what `a_session_claim` reads");
        assert_eq!(claim.pid, std::process::id());
        assert!(
            claim.started_at.is_some(),
            "the instant is the whole point of the second line"
        );
        assert_eq!(
            who_holds(&claim),
            ClaimHolder::TheSession,
            "this process is alive and started when it says it did"
        );

        let old = a_session_claim("4242").expect("a bare pid is still a claim");
        assert_eq!(old.pid, 4242);
        assert_eq!(
            old.started_at, None,
            "an old file records no identity, and that is a fact about the FILE rather than a \
             failure to parse it"
        );
    }

    /// **A claim on a pid the system handed on names a session that ENDED** (fix round 3, Integrity
    /// #1) — the answer the whole identity check exists to produce, driven against a real live
    /// process whose recorded start instant is one it cannot have had.
    #[test]
    fn a_claim_whose_recorded_start_disagrees_with_the_process_is_somebody_else() {
        let p = LiveProcess::spawn();
        let recycled = SessionClaim {
            pid: p.pid(),
            started_at: Some("2020-01-01T00:00:00Z".to_string()),
        };
        assert_eq!(
            who_holds(&recycled),
            ClaimHolder::SomebodyElse,
            "a live process that started years after the claim did is not the claim's session"
        );
        let unverifiable = SessionClaim {
            pid: p.pid(),
            started_at: None,
        };
        assert_eq!(
            who_holds(&unverifiable),
            ClaimHolder::Unverifiable,
            "and one that records nothing is neither proved nor disproved"
        );
    }

    /// **The `0` claim at the READER `SessionLock::acquire` is, which no test reached** (fix round
    /// 3, Test Quality #4). Only "not a pid" was covered here, so reverting this one reader to a
    /// bare `parse::<u32>` stayed green — and a claim holding `0` would then be probed with
    /// `kill(0, 0)`, a question about THIS process's own group that is true for as long as `nxc`
    /// lives, and every trigger for the session would be refused as "already running" against a
    /// claim nobody holds.
    #[test]
    fn a_claim_holding_zero_is_nobody_and_is_taken_over() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("m-sess.pid"), "0").unwrap();
        SessionLock::acquire(tmp.path(), "m-sess")
            .expect("`0` names no holder, so the claim is stale and this trigger takes it");
    }

    /// Every case [`SidecarWorker::stop_session_by`] must refuse, driven with a RECORDING signaller
    /// so the test can assert the part that matters most: that nothing was sent.
    ///
    /// The first two are the group-signal guard, which used to be pinned from an integration test
    /// through the real `kill` — see [`SidecarWorker::stop_session_by`] for why that could take the
    /// run down with it. The third is the old-format claim, which is refused because it cannot be
    /// told apart from a recycled pid.
    #[test]
    fn nothing_is_signalled_for_a_claim_that_does_not_earn_the_signal() {
        let tmp = tempfile::tempdir().unwrap();
        let logs = tmp.path().join(".nxs/agent-logs");
        std::fs::create_dir_all(&logs).unwrap();
        let worker = SidecarWorker {
            sidecar: tmp.path().join("stub.mjs"),
            cwd: tmp.path().to_path_buf(),
        };
        let sent: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::default();
        let record = |pid: u32| {
            sent.lock().unwrap().push(pid);
            Ok(SessionStop::Requested)
        };

        // A live process for the last case: the point there is not that the pid is dead but that
        // the claim cannot prove the live process behind it is this session's.
        let held = LiveProcess::spawn();
        for (session, contents, expected) in [
            ("s-zero", "0".to_string(), "does not hold a pid"),
            ("s-max", u32::MAX.to_string(), "does not hold a pid"),
            ("s-junk", "not a pid".to_string(), "does not hold a pid"),
            ("s-empty", String::new(), "does not hold a pid"),
            ("s-old", held.pid().to_string(), "recycled pid"),
        ] {
            std::fs::write(logs.join(format!("{session}.pid")), &contents).unwrap();
            let refused = worker
                .stop_session_by(session, record)
                .expect_err("a claim that does not earn the signal is refused, never sent");
            assert!(
                refused.contains(session) && refused.contains(expected),
                "{session}: refused by name, saying what it found: {refused}"
            );
        }
        // A session this workspace never started: no file at all, and still nothing sent.
        let missing = worker
            .stop_session_by("s-never", record)
            .expect_err("no claim is a refusal — there is nothing to signal");
        assert!(missing.contains("no pid file"), "{missing}");
        assert!(
            sent.lock().unwrap().is_empty(),
            "a signal was sent for a claim that does not earn one: {:?}",
            sent.lock().unwrap()
        );
    }

    /// The other side of the same seam: a claim that DOES prove its process is signalled, exactly
    /// once, with the pid the file names.
    #[test]
    fn the_signal_goes_to_the_one_pid_a_provable_claim_names() {
        let tmp = tempfile::tempdir().unwrap();
        let logs = tmp.path().join(".nxs/agent-logs");
        std::fs::create_dir_all(&logs).unwrap();
        let worker = SidecarWorker {
            sidecar: tmp.path().join("stub.mjs"),
            cwd: tmp.path().to_path_buf(),
        };
        let held = LiveProcess::spawn();
        std::fs::write(logs.join("m-sess.pid"), session_claim_for(held.pid())).unwrap();

        let sent: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::default();
        let outcome = worker
            .stop_session_by("m-sess", |pid| {
                sent.lock().unwrap().push(pid);
                Ok(SessionStop::Requested)
            })
            .expect("a claim whose identity holds is signalled");
        assert_eq!(outcome, SessionStop::Requested);
        assert_eq!(
            *sent.lock().unwrap(),
            vec![held.pid()],
            "one signal, to the pid the claim names and to nothing else"
        );
    }

    // ---- the declared-hurdle seam (nxf 6j6v.n92p) ---------------------------------------------

    #[test]
    fn a_worker_that_runs_no_commands_refuses_the_hurdle_rather_than_passing_it() {
        // Decision 1 at the DEFAULT, which is where it decides most: the safe direction of this
        // question is the opposite of the liveness read's above, and both are asserted on the one
        // worker that overrides neither.
        let dry = DryWorker { log: None };
        match dry.run_precondition("true") {
            crate::precondition::PreconditionOutcome::Unavailable(why) => {
                assert!(why.contains("does not run commands"), "{why}")
            }
            other => panic!("a hurdle nobody can check must not pass: {other:?}"),
        }
    }

    #[test]
    fn the_sidecar_worker_runs_a_hurdle_in_the_working_copy_and_reports_what_it_did() {
        // Driven against real processes, because every claim here is about one: the command runs in
        // the worker's OWN cwd (not this test process's), a zero exit and its output come back
        // verbatim, and a non-zero exit comes back as itself rather than as an error.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("marker.txt"), "here").unwrap();
        let worker = SidecarWorker {
            sidecar: tmp.path().join("stub.mjs"),
            cwd: tmp.path().to_path_buf(),
        };

        match worker.run_precondition("cat marker.txt") {
            crate::precondition::PreconditionOutcome::Ran { status, stdout, .. } => {
                assert_eq!(status, Some(0));
                assert_eq!(
                    stdout.trim(),
                    "here",
                    "the hurdle ran in the working copy the sessions run in, not in this process's \
                     cwd — that is the whole reason this seam is on the worker"
                );
            }
            other => panic!("{other:?}"),
        }

        match worker.run_precondition("echo to-stderr >&2; exit 3") {
            crate::precondition::PreconditionOutcome::Ran {
                status,
                stdout,
                stderr,
            } => {
                assert_eq!(status, Some(3));
                assert!(stdout.trim().is_empty());
                assert_eq!(stderr.trim(), "to-stderr");
            }
            other => panic!("a command that says no is not an error, it is a verdict: {other:?}"),
        }
    }

    #[test]
    fn a_hurdle_that_writes_more_than_a_pipe_buffer_still_finishes() {
        // The wait's own premise. A command whose output exceeds the pipe buffer blocks in `write`
        // until somebody drains it, so a wait loop that read only after exit would wait for an exit
        // that could never come. 512 KiB is several times any pipe buffer this runs on.
        let tmp = tempfile::tempdir().unwrap();
        let worker = SidecarWorker {
            sidecar: tmp.path().join("stub.mjs"),
            cwd: tmp.path().to_path_buf(),
        };
        match worker
            .run_precondition("yes 0123456789012345678901234567890123456789 | head -c 524288")
        {
            crate::precondition::PreconditionOutcome::Ran { status, stdout, .. } => {
                assert_eq!(status, Some(0));
                assert_eq!(stdout.len(), 524_288);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_hurdle_sees_the_allowlist_and_nothing_else_of_the_engines_environment() {
        // PR #391 review, Integrity #1. `preconditions:` lives in the working copy the gated
        // personas are told to work in, so a persona can WRITE the command the COORDINATOR runs.
        // Inheriting the engine's environment there would hand it a shell outside its own sandbox.
        // Driven against a REAL child, because `.env_clear()` is a property of the spawn and not of
        // the list — `hurdle_env`'s own unit test below cannot see a missing `.env_clear()`.
        let tmp = tempfile::tempdir().unwrap();
        let worker = SidecarWorker {
            sidecar: tmp.path().join("stub.mjs"),
            cwd: tmp.path().to_path_buf(),
        };
        // A secret in THIS process's environment, of exactly the shape the 2026-07-19 finding was
        // about. `set_var` is process-global; nothing else in this binary reads this key.
        std::env::set_var("NXC_TEST_OPERATOR_SECRET", "hunter2");
        let seen = match worker.run_precondition("echo \"[${NXC_TEST_OPERATOR_SECRET:-}]\"") {
            crate::precondition::PreconditionOutcome::Ran { stdout, .. } => stdout,
            other => panic!("{other:?}"),
        };
        std::env::remove_var("NXC_TEST_OPERATOR_SECRET");
        assert_eq!(
            seen.trim(),
            "[]",
            "the hurdle must not see the engine's own environment"
        );

        match worker.run_precondition("test -n \"$PATH\" && echo have-path") {
            crate::precondition::PreconditionOutcome::Ran { stdout, .. } => assert_eq!(
                stdout.trim(),
                "have-path",
                "…and it must still see the two it is given, or no hurdle could find `git`"
            ),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_hurdle_allowlist_is_strictly_narrower_than_a_spawned_sessions() {
        // The list itself, as a pure decision. A key that appears here and not in the sibling list
        // would be a widening nobody argued for, so the relationship is asserted rather than left to
        // two lists that happen to agree today.
        let ambient = |k: &str| Some(format!("value-of-{k}"));
        let hurdle: Vec<String> = hurdle_env(ambient).into_iter().map(|(k, _)| k).collect();
        let session: Vec<String> = forwarded_real_env(ambient)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(hurdle, vec!["PATH".to_string(), "HOME".to_string()]);
        assert!(
            hurdle.iter().all(|k| session.contains(k)) && hurdle.len() < session.len(),
            "a hurdle gets a strict subset of what a session gets: {hurdle:?} vs {session:?}"
        );
    }

    #[test]
    fn hurdle_env_omits_a_key_that_is_not_set() {
        assert_eq!(
            hurdle_env(|k| (k == "PATH").then(|| "/bin".to_string())).len(),
            1
        );
    }

    #[test]
    fn a_hurdle_that_never_finishes_is_killed_and_refuses() {
        // PR #391 review, Test Quality #2: the fail-closed timeout arm — the one that decides
        // whether a hung hurdle stops a step or lets it through — had no test at all. Driven
        // through `run_precondition_within` with a bound of milliseconds rather than by waiting out
        // the real `PRECONDITION_BOUND`, which is why that parameter exists.
        let tmp = tempfile::tempdir().unwrap();
        let worker = SidecarWorker {
            sidecar: tmp.path().join("stub.mjs"),
            cwd: tmp.path().to_path_buf(),
        };
        // **The duration is a UNIQUE MARKER, and both halves of that are load-bearing.** The
        // `pgrep` below is a question about THIS MACHINE, so it sees every process on it: `30`
        // collided with the `sleep 30` that `LiveProcess::spawn` starts in this same binary (green
        // in debug, red under `--release` where they run in parallel), and a fixed odd number then
        // collided with an ORPHAN left by an earlier aborted run of this very test. Tagging it with
        // this process's id makes the pattern unique to this run, so neither can happen again.
        let marker = format!("424242.{}", std::process::id());
        let started = std::time::Instant::now();
        let outcome =
            worker.run_precondition_within(&format!("sleep {marker}"), Duration::from_millis(150));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "it gave up on its own bound rather than on the command's"
        );
        match outcome {
            crate::precondition::PreconditionOutcome::Unavailable(why) => {
                assert!(why.contains("did not finish"), "{why}");
                assert!(
                    why.contains("was killed"),
                    "and it says the child was killed rather than left running in the working \
                     copy the next step wants: {why}"
                );
            }
            other => panic!("a hurdle that never answers must never pass: {other:?}"),
        }
        assert!(
            std::process::Command::new("pgrep")
                .args(["-f", &format!("^sleep {marker}$")])
                .output()
                .map(|o| !o.status.success())
                .unwrap_or(true),
            "and the killing is real: no `sleep {marker}` is left behind"
        );
    }

    #[test]
    fn a_hurdle_that_leaves_a_child_behind_still_returns_and_takes_it_with_it() {
        // **The defect this test exists for, found by CI hanging for an hour and a half.** A shell
        // `exec`s a SIMPLE command, so killing the child is the command — which is every hurdle
        // anybody writes, and which is why the first version of this looked correct. It does not
        // exec a `&`, a pipeline, or anything compound: there the shell stays, its children are
        // separate processes, and killing the shell left them running AND holding the output pipe.
        // Reading that pipe to end-of-file then never returned — inside the engine's held store
        // mutex, so one such hurdle wedged the whole handle permanently, past the bound and past
        // the kill.
        //
        // Two things fix it and this asserts both: the hurdle runs in its OWN PROCESS GROUP so the
        // bound reaches everything it started, and the output collection is bounded so even a
        // process the group kill cannot reach cannot hold the engine.
        let tmp = tempfile::tempdir().unwrap();
        let worker = SidecarWorker {
            sidecar: tmp.path().join("stub.mjs"),
            cwd: tmp.path().to_path_buf(),
        };
        // Tagged with this process's id for the reason the test above states in full: the `pgrep`
        // below sees the whole machine, so the marker has to be unique to this run.
        let marker = format!("424243.{}", std::process::id());
        let started = std::time::Instant::now();
        let outcome = worker.run_precondition_within(
            &format!("sleep {marker} & wait"),
            Duration::from_millis(150),
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "IT RETURNED. Before the fix this call never came back at all — that is the whole \
             assertion, and the elapsed bound is how a hang shows up as a failure rather than as a \
             suite that runs forever"
        );
        assert!(
            matches!(
                outcome,
                crate::precondition::PreconditionOutcome::Unavailable(_)
            ),
            "and it refuses, fail-closed: {outcome:?}"
        );
        // Give the group kill a moment to be reaped, then insist the grandchild is gone: a hurdle
        // left running in the working copy is the collision this epic exists to prevent.
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            std::process::Command::new("pgrep")
                .args(["-f", &format!("^sleep {marker}$")])
                .output()
                .map(|o| !o.status.success())
                .unwrap_or(true),
            "the process group went with it — killing only the shell would leave this behind"
        );
    }

    #[test]
    fn a_hurdle_whose_child_escapes_the_group_kill_still_does_not_hold_the_engine() {
        // The SECOND layer, and the only path on which it is observable. The group kill above
        // normally makes the output collection moot — the grandchild dies, the pipe closes, the read
        // ends. This is what happens when it cannot: a process that puts itself in a group of its
        // own is out of the kill's reach, keeps the write end of the pipe, and reading to
        // end-of-file would never return. `OUTPUT_GRACE` is what stops that being the engine's
        // problem, and `run_precondition_within` returning at all is the assertion.
        //
        // `perl -e setpgrp` rather than `setsid`, which macOS does not ship.
        let tmp = tempfile::tempdir().unwrap();
        let worker = SidecarWorker {
            sidecar: tmp.path().join("stub.mjs"),
            cwd: tmp.path().to_path_buf(),
        };
        let marker = format!("424244.{}", std::process::id());
        let escapee = format!("perl -e 'setpgrp(0,0); sleep {marker}' & wait");
        let started = std::time::Instant::now();
        let outcome = worker.run_precondition_within(&escapee, Duration::from_millis(150));
        let elapsed = started.elapsed();

        // Clean up after ourselves: this one is out of the engine's reach BY CONSTRUCTION, so the
        // test has to reap it or leave a process behind for the length of the sleep.
        let _ = std::process::Command::new("pkill")
            .args(["-f", &format!("sleep {marker}")])
            .output();

        assert!(
            elapsed < Duration::from_secs(5),
            "it returned rather than reading a pipe nobody will ever close: {elapsed:?}"
        );
        assert!(
            elapsed >= Duration::from_millis(150),
            "…and it did wait for its own bound first, so this is the grace and not an early exit"
        );
        assert!(
            matches!(
                outcome,
                crate::precondition::PreconditionOutcome::Unavailable(_)
            ),
            "fail-closed either way: {outcome:?}"
        );
    }

    #[test]
    fn a_hurdle_that_asks_a_question_gets_no_answer_instead_of_blocking() {
        // `stdin` is `/dev/null`, so a command that reads gets EOF at once. Without it a hurdle
        // that prompts would hold the store mutex for the whole bound with nobody able to answer.
        let tmp = tempfile::tempdir().unwrap();
        let worker = SidecarWorker {
            sidecar: tmp.path().join("stub.mjs"),
            cwd: tmp.path().to_path_buf(),
        };
        match worker.run_precondition("read line; echo \"got:[$line]\"") {
            crate::precondition::PreconditionOutcome::Ran { stdout, .. } => {
                assert_eq!(stdout.trim(), "got:[]")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn forwarded_real_env_includes_every_allowlisted_key_when_present() {
        let ambient = |k: &str| {
            Some(match k {
                "PATH" => "/usr/bin:/bin".to_string(),
                "HOME" => "/home/user".to_string(),
                "USER" => "ckoch".to_string(),
                "NXC_SIDECAR" => "/path/to/main.mjs".to_string(),
                "NXC_WORKER" => "sidecar".to_string(),
                _ => return None,
            })
        };

        let result = forwarded_real_env(ambient);

        assert_eq!(result.len(), 5);
        assert!(result.iter().any(|(k, _)| k == "PATH"));
        assert!(result.iter().any(|(k, _)| k == "HOME"));
        assert!(result.iter().any(|(k, _)| k == "NXC_SIDECAR"));
        assert!(result.iter().any(|(k, _)| k == "NXC_WORKER"));
        // `USER` is what reaches the SDK's ambient credential on macOS, where it lives in the
        // login keyring rather than under `HOME` (nxf 6j6v.zq0t). `HOME` alone authenticated
        // nothing there, and the failure surfaced as an expired OAuth session — so this key is
        // asserted by name, with the reason, rather than left to the count above.
        assert_eq!(
            result
                .iter()
                .find(|(k, _)| k == "USER")
                .map(|(_, v)| v.as_str()),
            Some("ckoch"),
            "USER must reach the sidecar: without it the SDK cannot read the macOS keyring"
        );
    }

    #[test]
    fn forwarded_real_env_omits_missing_keys() {
        let ambient = |k: &str| {
            Some(match k {
                "PATH" => "/usr/bin".to_string(),
                _ => return None,
            })
        };

        let result = forwarded_real_env(ambient);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "PATH");
        assert!(!result.iter().any(|(k, _)| k == "NXC_SIDECAR"));
        assert!(!result.iter().any(|(k, _)| k == "NXC_WORKER"));
    }

    // ---- the shipped sidecar (nxf 6j6v.81v5) -----------------------------------------------

    /// `sidecar_beside` canonicalizes its answer, so an expectation built from a literal temp path
    /// has to be canonicalized too: on macOS `$TMPDIR` itself lives under `/var -> /private/var`,
    /// and the same file then has two names.
    #[cfg(unix)]
    fn same_file(found: Option<PathBuf>, expected: PathBuf) {
        assert_eq!(
            found,
            Some(std::fs::canonicalize(&expected).unwrap_or(expected))
        );
    }

    /// A fake install: `<dir>/nxs` with the bundled sidecar beside it, plus `<dir>/nxc` as the
    /// symlink a real install creates. Returns the temp dir (kept alive by the caller).
    #[cfg(unix)]
    fn fake_install(sidecar: bool) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("nxs"), "binary").expect("nxs");
        if sidecar {
            std::fs::write(dir.path().join(SIDECAR_FILE), "// bundle").expect("sidecar");
        }
        std::os::unix::fs::symlink("nxs", dir.path().join("nxc")).expect("persona link");
        dir
    }

    #[test]
    #[cfg(unix)]
    fn an_install_finds_the_sidecar_shipped_beside_the_binary() {
        let install = fake_install(true);
        same_file(
            sidecar_beside(&install.path().join("nxs")),
            install.path().join(SIDECAR_FILE),
        );
    }

    #[test]
    #[cfg(unix)]
    fn the_resolution_carries_across_the_nxc_symlink() {
        // THE trap this is written around: `nxc` is a symlink to `nxs`, and `nxc send --to` is the
        // command the whole surface is named after. Both the path as invoked and the resolved one
        // must land on the same sidecar — the manufakt.io failure of 2026-08-05 was exactly this.
        let install = fake_install(true);
        same_file(
            sidecar_beside(&install.path().join("nxc")),
            install.path().join(SIDECAR_FILE),
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_from_another_directory_resolves_to_the_real_install() {
        // The harder half of the same trap: a link that does NOT sit in the install dir. The
        // literal parent has no sidecar; only canonicalizing the exe finds one.
        let install = fake_install(true);
        let elsewhere = tempfile::tempdir().expect("tempdir");
        let link = elsewhere.path().join("nxc");
        std::os::unix::fs::symlink(install.path().join("nxs"), &link).expect("cross-dir link");
        same_file(sidecar_beside(&link), install.path().join(SIDECAR_FILE));
    }

    #[test]
    #[cfg(unix)]
    fn no_sidecar_anywhere_is_none_rather_than_a_hopeful_path() {
        let install = fake_install(false);
        assert_eq!(sidecar_beside(&install.path().join("nxs")), None);
        assert_eq!(sidecar_beside(&install.path().join("nxc")), None);
    }

    /// A fake source tree: `<root>/target/<profile>/nxs` with `<root>/agent-sidecar/src/main.mjs`.
    fn fake_source_tree(profile: &str) -> (tempfile::TempDir, PathBuf) {
        let repo = tempfile::tempdir().expect("tempdir");
        let bindir = repo.path().join("target").join(profile);
        std::fs::create_dir_all(&bindir).expect("bindir");
        std::fs::create_dir_all(repo.path().join("agent-sidecar/src")).expect("src");
        std::fs::write(repo.path().join("agent-sidecar/src/main.mjs"), "// dev").expect("main.mjs");
        std::fs::write(bindir.join("nxs"), "binary").expect("nxs");
        (repo, bindir)
    }

    #[test]
    fn a_source_tree_binary_finds_the_unbundled_sidecar_two_levels_up() {
        // `target/<profile>/nxs` → `<repo>/agent-sidecar/src/main.mjs`, so a `cargo build` binary
        // needs no NXC_SIDECAR either. Both built-in profiles, since the gate below is by name.
        for profile in ["debug", "release"] {
            let (repo, bindir) = fake_source_tree(profile);
            let expected = repo.path().join("agent-sidecar/src/main.mjs");
            assert_eq!(
                sidecar_beside(&bindir.join("nxs")),
                Some(std::fs::canonicalize(&expected).unwrap_or(expected)),
                "profile {profile}"
            );
        }
    }

    #[test]
    fn the_source_tree_candidate_is_only_offered_from_a_cargo_output_directory() {
        // The candidate two levels up is `$HOME/agent-sidecar/src/main.mjs` for an install in
        // `~/.local/bin` — a predictable path in the user's own home with nothing marking it as
        // this program's. It is offered ONLY from a directory that really looks like
        // `target/{debug,release}`, so a bundle-less install reaches for nothing.
        let home = tempfile::tempdir().expect("tempdir");
        let bindir = home.path().join(".local/bin");
        std::fs::create_dir_all(&bindir).expect("bindir");
        std::fs::create_dir_all(home.path().join("agent-sidecar/src")).expect("src");
        std::fs::write(home.path().join("agent-sidecar/src/main.mjs"), "// stray")
            .expect("main.mjs");
        std::fs::write(bindir.join("nxs"), "binary").expect("nxs");
        assert_eq!(
            sidecar_beside(&bindir.join("nxs")),
            None,
            "an install must never run a stray main.mjs that merely sits two levels up"
        );
        assert!(is_cargo_out_dir(std::path::Path::new("/w/target/debug")));
        assert!(is_cargo_out_dir(std::path::Path::new("/w/target/release")));
        assert!(!is_cargo_out_dir(std::path::Path::new("/w/target/custom")));
        assert!(!is_cargo_out_dir(std::path::Path::new("/w/build/debug")));
    }

    #[test]
    fn the_shipped_sidecar_wins_over_a_source_tree_beneath_it() {
        // Order matters: an install that ALSO happens to sit two levels above an `agent-sidecar`
        // tree must still run its own shipped bundle.
        let (_repo, bindir) = fake_source_tree("debug");
        std::fs::write(bindir.join(SIDECAR_FILE), "// bundle").expect("bundle");
        let expected = bindir.join(SIDECAR_FILE);
        assert_eq!(
            sidecar_beside(&bindir.join("nxs")),
            Some(std::fs::canonicalize(&expected).unwrap_or(expected))
        );
    }

    #[test]
    #[cfg(unix)]
    fn every_shipped_candidate_is_tried_before_any_source_tree_one() {
        // Across the literal/canonical PAIR, not just within one directory: a link from a source
        // tree pointing at an install must still run the install's bundle, never the tree's
        // `main.mjs` that happens to sit two levels above the link.
        let install = fake_install(true);
        let (_repo, bindir) = fake_source_tree("debug");
        let link = bindir.join("nxs-link");
        std::os::unix::fs::symlink(install.path().join("nxs"), &link).expect("link");
        same_file(sidecar_beside(&link), install.path().join(SIDECAR_FILE));
    }

    /// Write `name` into `dir` and make it executable — what a PATH entry has to look like for a
    /// shell to pick it.
    #[cfg(unix)]
    fn executable(dir: &std::path::Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    #[test]
    #[cfg(unix)]
    fn claude_is_looked_up_on_the_path_the_way_a_shell_would() {
        let dir = tempfile::tempdir().expect("tempdir");
        let empty = tempfile::tempdir().expect("tempdir");
        let claude = executable(dir.path(), "claude");
        // Earlier entries win, and an entry that has no `claude` is simply skipped.
        let path = std::env::join_paths([empty.path(), dir.path()]).expect("join_paths");
        assert_eq!(
            claude_on_path(Some(path.to_string_lossy().into_owned())),
            Some(claude)
        );
        assert_eq!(
            claude_on_path(Some(empty.path().to_string_lossy().into_owned())),
            None,
            "nothing named claude on the PATH is no claude at all"
        );
        assert_eq!(claude_on_path(None), None, "no PATH at all is not a panic");
    }

    #[test]
    #[cfg(unix)]
    fn a_non_executable_claude_is_skipped_in_favour_of_a_real_one_further_along() {
        // A shell walks past a file it cannot run. Selecting it instead would let a plain data file
        // named `claude` — committed to the workspace being operated on, which an empty PATH
        // component makes reachable — shadow the real binary, and the run would then die on the
        // SDK's own opaque spawn failure rather than on anything a reader could act on.
        let decoy = tempfile::tempdir().expect("tempdir");
        let real = tempfile::tempdir().expect("tempdir");
        std::fs::write(decoy.path().join("claude"), "not a program").expect("decoy");
        let claude = executable(real.path(), "claude");
        let path = std::env::join_paths([decoy.path(), real.path()]).expect("join_paths");
        assert_eq!(
            claude_on_path(Some(path.to_string_lossy().into_owned())),
            Some(claude)
        );
        assert_eq!(
            claude_on_path(Some(decoy.path().to_string_lossy().into_owned())),
            None,
            "a file that cannot be run is not a claude"
        );
    }

    #[test]
    fn only_the_shipped_bundle_counts_as_bundled() {
        // What the refusal in `trigger` keys on: the bundle cannot resolve the native `claude`
        // itself, a source-tree sidecar can. An `NXC_SIDECAR` pointed at a copy of the bundle is
        // correctly the bundle.
        let bundled = |p: &str| {
            SidecarWorker {
                sidecar: PathBuf::from(p),
                cwd: PathBuf::from("/w"),
            }
            .is_bundled_sidecar()
        };
        assert!(bundled("/usr/local/bin/nxc-agent-sidecar.mjs"));
        assert!(bundled("/anywhere/else/nxc-agent-sidecar.mjs"));
        assert!(!bundled("/repo/agent-sidecar/src/main.mjs"));
        assert!(!bundled(""));
    }

    // ---- the EMBEDDED sidecar (nxf 6j6v.smsz) ----------------------------------------------

    /// An ambient lookup over a fixed list, in the `var_os` shape [`cache_root`] takes.
    fn env_of<'a>(
        vals: &'a [(&'a str, &'a str)],
    ) -> impl Fn(&str) -> Option<std::ffi::OsString> + 'a {
        move |k: &str| {
            vals.iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| std::ffi::OsString::from(*v))
        }
    }

    #[test]
    fn the_cache_root_walks_the_same_ladder_the_updater_does() {
        // One machine, one nexus-flow cache: this ladder is `selfupdate.rs::cache_dir`'s, so the
        // unpacked sidecar lands beside the update-hint cache instead of inventing a second home.
        assert_eq!(
            cache_root(env_of(&[
                ("NXF_CACHE_DIR", "/c/override"),
                ("XDG_CACHE_HOME", "/c/xdg"),
                ("HOME", "/home/u"),
            ])),
            Some(PathBuf::from("/c/override"))
        );
        assert_eq!(
            cache_root(env_of(&[("XDG_CACHE_HOME", "/c/xdg"), ("HOME", "/home/u")])),
            Some(PathBuf::from("/c/xdg/nxf"))
        );
        assert_eq!(
            cache_root(env_of(&[("HOME", "/home/u")])),
            Some(PathBuf::from("/home/u/.cache/nxf"))
        );
        // An EMPTY value is not a location — the same filter the updater applies, and the reason
        // `HOME=` in a scrubbed environment does not resolve to `/.cache/nxf`.
        assert_eq!(
            cache_root(env_of(&[("NXF_CACHE_DIR", ""), ("HOME", "")])),
            None
        );
        assert_eq!(cache_root(env_of(&[])), None);
    }

    #[test]
    fn unpacking_writes_the_bundle_under_a_content_addressed_directory() {
        let cache = tempfile::tempdir().expect("tempdir");
        let bundle = b"// the bundle" as &[u8];
        let path = unpack_sidecar(bundle, cache.path()).expect("unpacked");

        assert_eq!(std::fs::read(&path).expect("readable"), bundle);
        // The file keeps the SHIPPED name, because `SidecarWorker::is_bundled_sidecar` keys on it —
        // an unpacked bundle has no `node_modules` beside it either, so it needs the same
        // "resolve `claude` for me" precheck. The content hash is the DIRECTORY.
        assert_eq!(path.file_name().expect("named"), SIDECAR_FILE);
        assert!(
            path.starts_with(cache.path().join("sidecar")),
            "{}",
            path.display()
        );
        assert!(
            !path.exists() || std::fs::metadata(&path).expect("meta").is_file(),
            "the answer is a file, never a directory"
        );
        // Nothing temporary is left lying around next to it.
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().expect("parent"))
            .expect("dir")
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .filter(|n| n != SIDECAR_FILE)
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn unpacking_is_idempotent_and_two_bundles_never_share_an_entry() {
        let cache = tempfile::tempdir().expect("tempdir");
        let first = unpack_sidecar(b"// bundle A", cache.path()).expect("A");
        let again = unpack_sidecar(b"// bundle A", cache.path()).expect("A again");
        assert_eq!(first, again, "the same bytes resolve to the same file");

        // Content-addressed, not version-addressed: this is what makes the binary and its sidecar
        // one pair by construction (nxf 6j6v.5vct) — a changed bundle CANNOT land on the entry the
        // old one wrote, so there is no stale-file case left to defend against.
        let other = unpack_sidecar(b"// bundle B", cache.path()).expect("B");
        assert_ne!(first, other);
        assert_eq!(std::fs::read(&first).expect("A"), b"// bundle A");
        assert_eq!(std::fs::read(&other).expect("B"), b"// bundle B");
    }

    #[test]
    fn a_planted_file_of_the_right_length_is_overwritten_rather_than_run() {
        // PR #302 review, Integrity & Robustness #1. The cache path is PUBLIC knowledge — the
        // bundle ships inside a signed binary anyone can download, so anyone can compute the hash
        // that names its directory. Whoever can write there first (a shared NXF_CACHE_DIR, a synced
        // $HOME/.cache, a multi-tenant box) could otherwise pre-plant a same-length file and have
        // `node` run it. Reuse is decided on the BYTES, so a planted file is replaced, not obeyed.
        let cache = tempfile::tempdir().expect("tempdir");
        let bundle = b"// the real bundle" as &[u8];
        let planted = b"// EVIL, same len!" as &[u8];
        assert_eq!(
            bundle.len(),
            planted.len(),
            "the whole point is a length an attacker can trivially match"
        );

        // Plant it exactly where the real bundle would land.
        let dir = cache.path().join("sidecar").join(content_id(bundle));
        std::fs::create_dir_all(&dir).expect("plant dir");
        std::fs::write(dir.join(SIDECAR_FILE), planted).expect("plant");

        let path = unpack_sidecar(bundle, cache.path()).expect("unpacked");
        assert_eq!(
            std::fs::read(&path).expect("readable"),
            bundle,
            "the bytes `node` gets are the ones compiled into this binary"
        );
    }

    #[test]
    #[cfg(unix)]
    fn the_cache_entry_is_owner_only() {
        // The other half of the same finding: narrowing the directory means the planting above
        // needs the user's OWN account rather than any account on the machine.
        use std::os::unix::fs::PermissionsExt;
        let cache = tempfile::tempdir().expect("tempdir");
        let path = unpack_sidecar(b"// bundle", cache.path()).expect("unpacked");
        let dir = path.parent().expect("parent");
        assert_eq!(
            std::fs::metadata(dir).expect("meta").permissions().mode() & 0o777,
            0o700,
            "{}",
            dir.display()
        );
    }

    #[test]
    fn a_cache_that_cannot_be_written_says_so_instead_of_failing_silently() {
        // Acceptance 4. A silent failure here would be exactly the class of bug this whole ticket
        // exists to remove — so the refusal names the path it tried, says why the file is needed,
        // and names both ways out.
        //
        // The obstacle is a FILE where the cache root should be (ENOTDIR), not a read-only
        // directory: a test that assumes it cannot write somewhere quietly stops testing anything
        // when the suite runs as root, which it does in some containers.
        let tmp = tempfile::tempdir().expect("tempdir");
        let not_a_dir = tmp.path().join("cache");
        std::fs::write(&not_a_dir, "I am a file").expect("occupied");

        let err = unpack_sidecar(b"// bundle", &not_a_dir).expect_err("cannot unpack into a file");
        assert_eq!(err.kind, ErrorKind::Io);
        assert!(
            err.msg.contains(&not_a_dir.display().to_string()),
            "names where it tried: {}",
            err.msg
        );
        assert!(err.msg.contains("NXF_CACHE_DIR"), "{}", err.msg);
        assert!(err.msg.contains("NXC_SIDECAR"), "{}", err.msg);
    }

    #[test]
    fn nowhere_to_unpack_to_is_its_own_refusal_not_a_missing_sidecar() {
        // The other half of acceptance 4: an environment with no HOME, no XDG_CACHE_HOME and no
        // override. "I have the sidecar and nowhere to put it" is a different problem from "I have
        // no sidecar", and saying the latter would send a reader looking for a file that is not
        // missing.
        let err = embedded_sidecar_from(Some(b"// bundle"), env_of(&[]))
            .expect_err("no cache location at all");
        assert_eq!(err.kind, ErrorKind::Io);
        assert!(err.msg.contains("NXF_CACHE_DIR"), "{}", err.msg);
        assert!(err.msg.contains("HOME"), "{}", err.msg);
    }

    #[test]
    fn a_build_without_a_bundle_carries_none_and_says_nothing_about_a_cache() {
        // A plain `cargo build` in a checkout that never ran `npm run build`: no embedded bundle,
        // no cache lookup, no error — the source-tree fallback takes over one level up.
        assert_eq!(
            embedded_sidecar_from(None, |_| panic!("a build with no bundle asks for no cache"))
                .expect("no bundle is not a failure"),
            None
        );
    }

    #[test]
    #[cfg(unix)]
    fn the_embedded_bundle_outranks_a_file_beside_the_binary() {
        // PR #302 review, Test Quality #1. THE property of nxf 6j6v.smsz, exercised through the
        // real resolution instead of asserted in prose: an install from <=0.53.0 still has a bundle
        // lying next to `nxs`, and the one this binary was BUILT with must win — that is what makes
        // binary and sidecar one pair, and what makes 6j6v.5vct unconstructible rather than
        // unlikely. Every other test here injects the lookup, so a swapped order would pass them
        // all.
        let install = fake_install(true); // `<dir>/nxs` with a stale bundle beside it
        let cache = tempfile::tempdir().expect("tempdir");
        let env = [("NXF_CACHE_DIR", cache.path().to_str().expect("utf8"))];
        let exe = install.path().join("nxs");

        let resolved =
            installed_sidecar_from(Some(b"// the embedded bundle"), env_of(&env), || {
                Some(exe.clone())
            })
            .expect("resolves")
            .expect("something was found");
        assert!(
            resolved.starts_with(cache.path()),
            "the embedded bundle, unpacked into the cache — not {}",
            resolved.display()
        );
        assert_eq!(
            std::fs::read(&resolved).expect("readable"),
            b"// the embedded bundle"
        );

        // And the beside-file is still the answer for a build that carries nothing — the fallback
        // an install from <=0.53.0 and a source-tree `cargo build` both rely on.
        let fallback = installed_sidecar_from(None, env_of(&env), || Some(exe.clone()))
            .expect("resolves")
            .expect("the file beside the binary");
        same_file(Some(fallback), install.path().join(SIDECAR_FILE));
    }

    #[test]
    fn the_bundle_is_embedded_exactly_when_the_build_had_one() {
        // The gate that cannot silently skip. `build.rs` embeds `agent-sidecar/dist/…` when it is
        // there and `None` when it is not, so an absent bundle must be explained by an absent
        // INPUT — never by a build script that stopped working. Release builds set
        // `NXF_EMBED_SIDECAR=require`, which turns the absent case into a failed build outright.
        let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("crates/chat has a repo root two levels up")
            .join("agent-sidecar/dist")
            .join(SIDECAR_FILE);
        assert_eq!(
            EMBEDDED_SIDECAR.is_some(),
            dist.is_file(),
            "the embedded bundle and {} must appear and disappear together",
            dist.display()
        );
        if let Some(bytes) = EMBEDDED_SIDECAR {
            assert_eq!(
                bytes.len() as u64,
                std::fs::metadata(&dist).expect("dist").len(),
                "the embedded bytes ARE the bundle on disk, not a truncated copy"
            );
        }
    }
}

/// **What the background service reads to decide whether to hold the machine awake** (nxf
/// 6j6v.7q3r).
///
/// The direction that matters is the SECOND one: a session that has ended must stop counting, or
/// the machine never sleeps again — a fault a person meets days later as a battery problem and
/// never attributes to us.
#[cfg(test)]
mod live_sessions_tests {
    use super::*;
    use tempfile::TempDir;

    /// A workspace whose agent-log directory holds `<session>.pid` files with the given contents.
    fn workspace_with(pids: &[(&str, String)]) -> TempDir {
        let tmp = TempDir::new().unwrap();
        let logs = agent_logs_dir(tmp.path());
        std::fs::create_dir_all(&logs).unwrap();
        for (session, content) in pids {
            std::fs::write(logs.join(format!("{session}.pid")), content).unwrap();
        }
        tmp
    }

    /// A pid that is certainly not running: the kernel refuses to allocate `pid_max`-adjacent ids
    /// to a live process here, and the claim is checked rather than assumed by the first assert.
    fn dead_pid() -> String {
        let mut candidate = 4_194_303u32;
        while process_is_alive(candidate) {
            candidate -= 1;
        }
        candidate.to_string()
    }

    #[test]
    fn a_session_whose_process_has_gone_stops_counting_as_live() {
        let live = std::process::id().to_string();
        let tmp = workspace_with(&[("m-dead", dead_pid()), ("m-live", live)]);
        assert_eq!(
            live_sessions_in(tmp.path()),
            vec!["m-live".to_string()],
            "a pid file left behind by a finished session must not keep a machine awake"
        );
    }

    #[test]
    fn a_workspace_that_has_never_started_a_session_has_none_and_is_not_an_error() {
        let tmp = TempDir::new().unwrap();
        assert!(live_sessions_in(tmp.path()).is_empty());
        // …and neither is one with the directory but nothing in it.
        std::fs::create_dir_all(agent_logs_dir(tmp.path())).unwrap();
        assert!(live_sessions_in(tmp.path()).is_empty());
    }

    #[test]
    fn only_pid_files_are_read_because_the_directory_holds_three_kinds() {
        let tmp = workspace_with(&[("m-live", std::process::id().to_string())]);
        let logs = agent_logs_dir(tmp.path());
        // The two siblings a session leaves beside its claim, both of which OUTLIVE the process.
        std::fs::write(logs.join("m-ghost.log"), "output").unwrap();
        std::fs::write(logs.join("m-ghost.spec.json"), "{}").unwrap();
        assert_eq!(live_sessions_in(tmp.path()), vec!["m-live".to_string()]);
    }

    #[test]
    fn a_claim_nobody_can_read_reads_as_gone_rather_than_as_a_reason_to_stay_awake() {
        // The same residual `SessionLock` already states, pointed at this reader: guessing "alive"
        // for an unparseable claim would be a machine that never sleeps because of a corrupt file.
        let tmp = workspace_with(&[("m-torn", "not-a-pid".to_string())]);
        assert!(live_sessions_in(tmp.path()).is_empty());
    }

    #[test]
    fn the_answer_is_the_same_one_session_is_running_gives_for_the_same_session() {
        // Two readers of one fact must not disagree — the whole reason this shares
        // `process_is_alive` and the path helper rather than spelling either again.
        let tmp = workspace_with(&[
            ("m-live", std::process::id().to_string()),
            ("m-dead", dead_pid()),
        ]);
        let worker = SidecarWorker {
            sidecar: PathBuf::from("/nonexistent/sidecar.mjs"),
            cwd: tmp.path().to_path_buf(),
        };
        for session in ["m-live", "m-dead"] {
            assert_eq!(
                worker.session_is_running(session),
                live_sessions_in(tmp.path()).iter().any(|s| s == session),
                "the two readings of {session} disagree"
            );
        }
    }
}
