//! The `nxs` command surface — the platform umbrella CLI (spec §6.3). `--json` everywhere,
//! deterministic output (agent-ergonomics is the measuring stick, identical to `nxf`/`nxm`).
//!
//! Verbs shipped so far: `prime` (the composed fan-out — the SessionStart hooks run each module's
//! own `prime` since nxf n2m6 + a2a1, so this is the by-hand verb, S3) and the
//! foundation-only `migrate`, `doctor`/`status` (S1) over the ONE shared `.nxs/` workspace. `init`
//! (interactive module selection) lands in S4; the assembler the per-module inits delegate to (S5)
//! lives in this crate's library.

use crate::error::{self, NxfError, Result};
use crate::{doctor, guide, init, migrate, prime, setup, sync};
use clap::{Parser, Subcommand};
use nxs_foundation::workspace::Workspace;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// nexus-flow — the board (nxf), the memory (nxm) and the channel (nxc) your agents work from, in
/// one binary.
#[derive(Parser, Debug)]
#[command(
    name = "nxs",
    version,
    about = "nexus-flow — the board (nxf), the memory (nxm) and the channel (nxc) your agents work from, in one binary",
    long_about = None
)]
struct Cli {
    /// Emit machine-readable JSON instead of human output.
    #[arg(long, global = true)]
    json: bool,

    /// Use this sqlite db file directly instead of discovering `.nxs/`.
    #[arg(long, global = true, env = "NXS_DB")]
    db: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Set up the suite in the current directory: pick which tools to use, fan out to each module's
    /// own init (driven silently — no sub-init output bleeds through), assemble the shared agent
    /// files, and wire one SessionStart hook per active module. On a terminal it prompts;
    /// non-interactively use
    /// `--module` (repeatable) or `--json`.
    Init {
        /// Set up this module (repeatable): `flow` | `memory` | `chat`. Omit on a terminal to choose
        /// interactively; non-interactively (`--json`/piped) with none given defaults to flow.
        #[arg(long = "module")]
        module: Vec<String>,
        /// Pre-select this module in the chooser (repeatable): the multi-select starts with it
        /// ticked (the user can add/remove others), and any non-interactive sink takes it as the
        /// selection. The seam the front-door binaries use — `nxf init` re-execs
        /// `nxs init --preselect flow`, `nxm init` `--preselect memory`. Unlike `--module` it is a
        /// starting point, not a forced, exclusive choice.
        #[arg(long = "preselect")]
        preselect: Vec<String>,
        /// flow's plugin, passed through to its init (e.g. `issue-tracker`); omit for flow's default.
        #[arg(long)]
        plugin: Option<String>,
        /// Consent to the beads → nxs migration in a non-interactive run. When this project still
        /// uses beads, `init` offers to import its tickets + memories and roll back
        /// beads' config; on a terminal it asks, but `--json`/piped never prompts and never migrates
        /// without this explicit flag.
        #[arg(long = "from-beads")]
        from_beads: bool,
        /// Set up the nexus-flow background service too, without being asked. One process keeps
        /// the deadlines of every workspace it attends and syncs the ones bound to a stream; on a
        /// terminal `init` offers it, while `--json`/piped runs never install it without this flag.
        #[arg(long = "service", conflicts_with = "no_service")]
        service: bool,
        /// Do NOT set up the background service — and do not ask again in this workspace. The
        /// answer is recorded in `.nxs/config.toml`; `nxs sync daemon install` still sets it up
        /// whenever you change your mind.
        #[arg(long = "no-service")]
        no_service: bool,
    },
    /// Session bootstrap fan-out: run the `prime` of each ACTIVE module (in registry order) with
    /// one shared `now`, and concatenate the results. This is the target of the single SessionStart
    /// hook — a flow-only workspace yields exactly flow's prime; memory appears after `nxm init`.
    Prime {
        /// Compose the block for a DECLARED PERSONA rather than for whoever is at the keyboard
        /// (nxf 6j6v.k8zq): chat's block is that persona's own (identity, address book, answering
        /// rules), and the persona's `prime:` filter decides which modules contribute at all — a
        /// persona that excludes the board gets no `nxf prime` section, and another persona's still
        /// carries one.
        ///
        /// This is the text a spawned session is handed in its system prompt, composed by the same
        /// function the spawn path uses, so `nxs prime --persona <h>` shows exactly what that
        /// persona reads.
        #[arg(long)]
        persona: Option<String>,
    },
    /// Read the suite's guides: with no topic, every ACTIVE module's own topics (flow's, memory's,
    /// chat's) in one listing; with a topic, that guide. Fans out to `<binary> guide` exactly as
    /// `prime` fans out to `<binary> prime`. Needs no workspace — with none, every installed
    /// module's guides are listed — and a topic several blocks share (`getting-started`) is never
    /// picked for you: it names `nxf guide …` / `nxm guide …` / `nxc guide …` instead.
    Guide {
        /// Guide topic; omit to list every block's topics.
        topic: Option<String>,
    },
    /// Raise the shared workspace db to this binary's foundation schema version. Migrating on open
    /// is the default; `migrate` is the explicit CI/repair lever and reports `from → to`.
    /// Foundation-only — works whatever modules are active.
    Migrate,
    /// Cross-module diagnosis of the shared workspace: active modules, schema version + standing,
    /// replica identity, sync state, op count, and db integrity. Foundation-only.
    Doctor,
    /// Alias for `doctor`: the same cross-module diagnosis.
    Status,
    /// Wire host-specific agent integration for this workspace (idempotent, merge-only). Host setup
    /// is an umbrella responsibility — the umbrella owns the whole SessionStart set, one hook per
    /// active module — so this is the canonical home of the verb; `nxf setup claude` delegates here.
    Setup {
        #[command(subcommand)]
        action: SetupAction,
    },
    /// Sync the ONE shared op-log with a remote relay: bind a stream, then push/pull. A suite
    /// operation (one shared log ⇒ sync is no longer per-module), relocated here from `nxf`.
    Sync {
        #[command(subcommand)]
        action: SyncAction,
    },
    /// Update the whole suite in place from the release channel (sha256 + minisign verified, no
    /// downgrade): swaps the single `nxs` binary and relinks the `nxf`/`nxm`/`nxc` personas —
    /// including the binary now running. The canonical, suite-wide self-update; `nxf self-update`
    /// remains a hidden, deprecated alias. `--check` reports without installing.
    SelfUpdate {
        /// Promotion ring to update from; persisted as this install's default.
        #[arg(long, value_parser = ["stable", "beta", "alpha"])]
        channel: Option<String>,
        /// Report whether an update is available without installing it.
        #[arg(long)]
        check: bool,
    },
    /// Serve the suite over the Model Context Protocol for MCP-native hosts (Claude Desktop et
    /// al.): the board and memory as MCP tools — reads AND writes — over the same shared `.nxs/`
    /// store as the CLI and apps. The third seam beside the CLI (agents) and the embed API (apps).
    /// A host that connects can create, update, close and archive items, so give it the same trust
    /// you would give a shell in this workspace.
    #[cfg(feature = "mcp")]
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
}

/// `nxs mcp` actions. `serve` (stdio) carries the read and write tools; `install` registers this
/// binary into installed MCP hosts' config. Remote transports are a later slice (E9 #76u).
#[cfg(feature = "mcp")]
#[derive(Subcommand, Debug)]
enum McpAction {
    /// Run an MCP server over stdio. MCP hosts have no useful cwd, so the launch workspace is pinned
    /// here via `--workspace` (or `NXS_WORKSPACE`); omitted, it auto-initializes and serves a stable
    /// per-user application-data directory — never a cwd walk-up.
    Serve {
        /// Workspace root to serve (the directory holding `.nxs/`). Defaults to `NXS_WORKSPACE`,
        /// then a stable per-user App-Data-Home (auto-initialized on first start).
        #[arg(long, env = "NXS_WORKSPACE")]
        workspace: Option<String>,
        /// Default actor recorded on writes by this server instance. A long-lived, host-started
        /// server would otherwise author every op under one ambient identity; a tool call may
        /// override it per write with its own `actor`. Defaults to `NXS_ACTOR`, then the
        /// `NXF_ACTOR`/`USER` fallback the CLI uses.
        #[arg(long, env = "NXS_ACTOR")]
        actor: Option<String>,
    },
    /// Register this `nxs` in installed MCP hosts' config: write an idempotent `mcpServers.nxs`
    /// entry naming the ABSOLUTE path of this binary + `mcp serve`. With no `--host`, it scans the
    /// known hosts (Claude Desktop, Cursor, Windsurf, Amazon Quick) and patches every one that is
    /// installed; a re-run whose entry already matches changes nothing.
    Install {
        /// Pin this workspace root in the written entry (its `--workspace` arg). Omit to let the
        /// server serve its per-user application-data default (auto-initialized on first start).
        /// With `--setup`, this is ALSO the directory the workspace is materialized in.
        #[arg(long)]
        workspace: Option<String>,
        /// Target a specific host by id (repeatable): `claude-desktop` | `cursor` | `windsurf` |
        /// `amazon-quick`. It is written even if not auto-detected (its config dir is created). Omit
        /// to auto-detect all.
        #[arg(long = "host")]
        host: Vec<String>,
        /// Also set up the workspace the server will open: materialize `.nxs/` (flow active + the
        /// chosen plugin) at the SAME target the entry points at — the pinned `--workspace`, or
        /// the per-user default when none is pinned — so one command registers AND creates the
        /// board. Idempotent; never clobbers an existing board.
        #[arg(long)]
        setup: bool,
        /// The flow plugin to seat when setting up — implies `--setup`. Defaults to `personal-todo`
        /// (the knowledge-worker board); pass `issue-tracker` for the coding case. The server stays
        /// plugin-agnostic and reads this from the workspace's `config.toml`.
        #[arg(long)]
        plugin: Option<String>,
        /// How the written entry launches the server: `native` (default) names the absolute path of
        /// this `nxs` binary; `npx` writes the portable `npx @nexus-flow/mcp` runner shim that
        /// fetches + signature-verifies + execs `nxs` on demand — machine-independent, and the only
        /// entry that works before `nxs` is installed (needs Node + a one-time first-run download).
        #[arg(long, default_value = "native", value_parser = ["native", "npx"])]
        runner: String,
    },
    /// List the workspaces registered with this service instance (`~/.nexusflow/workspaces.toml`,
    /// or `~/.nexusflow-<name>/` for a development BUILD that `NXS_SERVICE_INSTANCE` names) — the
    /// deterministic list
    /// the `list_workspaces` MCP tool returns (`--json` prints the same `{name, path}` records).
    /// A host selects one by name for a tool's per-call `workspace` override without knowing paths;
    /// entries are added by `nxs mcp install --workspace <path>` or by hand-editing the TOML.
    Workspaces,
}

#[derive(Subcommand, Debug)]
enum SetupAction {
    /// Wire Claude Code: (re)assemble the shared agent file (AGENTS.md) and one
    /// SessionStart hook per active module (`nxf prime`, `nxm prime`, `nxc prime`) plus the
    /// `nxs`/module permission allowlist, merged into `.claude/settings.json` (idempotent, never
    /// overwriting). Regenerates a deleted agent file as well as wiring the hooks.
    Claude,
}

/// `nxs sync trust` actions.
#[derive(Subcommand, Debug)]
enum TrustAction {
    /// The keys this workspace trusts (its own first), and the keys that signed ops here without
    /// being trusted — with the authors those ops claim, as a pointer to what to compare.
    List,
    /// Trust a key: the ops it signed and signs, past ones included, may carry agent actions here.
    /// Takes the FULL key id another machine prints with `nxs sync key`; compare it with that
    /// machine's owner over a channel you trust before adding it.
    Add {
        /// The key id, `ed25519:…`.
        key_id: String,
        /// What to call it — usually the other machine's name (one line, up to 64 characters).
        #[arg(long)]
        name: Option<String>,
    },
    /// Stop trusting a key — revocation, effective at once: its ops stay on the board and none of
    /// them carries an agent action from now on. This replica's own key cannot be removed.
    Remove {
        /// The key id, `ed25519:…`.
        key_id: String,
    },
}

#[derive(Subcommand, Debug)]
enum SyncAction {
    /// Bind this workspace to a stream. Without flags: the `stream_id` is derived
    /// deterministically from the git `origin` remote, so every clone lands on the same stream
    /// with no `--join` to copy around; `--create` mints a fresh id instead, `--join` takes a
    /// shared id verbatim. An explicit one-time step (never an implicit side effect of `sync
    /// run`).
    Bind {
        /// Mint a fresh stream id (first replica) instead of deriving one from the git remote.
        #[arg(long)]
        create: bool,
        /// Join an existing, out-of-band-shared stream id (every other device) instead of
        /// deriving one from the git remote.
        #[arg(long)]
        join: Option<String>,
        /// Bind this workspace to a specific relay endpoint (overrides the global default for
        /// this workspace only).
        #[arg(long)]
        endpoint: Option<String>,
        /// Switch an already-bound workspace to a different stream. Re-pushes the local log to
        /// the new stream and resets both watermarks. Also the migration path off an older
        /// randomly-minted stream id.
        #[arg(long)]
        rebind: bool,
        /// Skip the best-effort launchd autostart a successful bind otherwise attempts. Use
        /// this for CI/tests/environments where a background daemon is unwanted, or
        /// where `nxs sync run` will be driven manually instead.
        #[arg(long)]
        no_daemon: bool,
        /// Start this FRESH workspace from a snapshot file (`nxs sync snapshot <file>` on a
        /// machine that syncs the stream): it joins the snapshot's stream, takes its log and folded
        /// board, and the first pass pulls only what came after. The workspace must hold no ops
        /// yet, and a relay must be known (`--endpoint`, or the global default) — the snapshot's
        /// position is confirmed against it first.
        #[arg(long, value_name = "FILE")]
        snapshot: Option<String>,
    },
    /// Write a snapshot of this workspace to FILE: the whole op log, the folded board, and the
    /// relay position they reach. A new machine starts from it with `nxs sync bind --snapshot
    /// FILE` and pulls only what came after, instead of folding the whole history. Refuses while
    /// this replica holds ops it has not pushed — run `nxs sync run` first.
    Snapshot {
        /// Where to write it. Overwritten if it exists.
        file: String,
    },
    /// Take a workspace back off the list this service instance attends
    /// (`~/.nexusflow/workspaces.toml`, or `~/.nexusflow-<name>/` for a development BUILD that
    /// `NXS_SERVICE_INSTANCE` names) — the counterpart to the registration `bind` performs.
    ///
    /// Nothing inside the workspace is touched: its `.nxs/sync.toml` stream binding survives, so a
    /// later `bind` resumes where this left off instead of starting a fresh sync. With no argument
    /// this unregisters the workspace you are standing in; pass a path to prune an entry whose
    /// directory is gone (which is the case that could previously only be fixed by hand-editing
    /// the TOML).
    Unregister {
        /// The workspace root to remove. Omit to use the workspace resolved from the current
        /// directory.
        path: Option<String>,
    },
    /// Run one push/pull anti-entropy pass against the relay.
    Run {
        /// Relay base URL. Optional: without it the endpoint bound to this workspace, or
        /// the global default (`nxs sync endpoint <url>`), is used. Passing it overrides
        /// both for this one run and is NOT persisted.
        #[arg(long)]
        remote: Option<String>,
    },
    /// Show or set the GLOBAL default relay endpoint (`~/.nexusflow/config.toml`, or
    /// `~/.nexusflow-<name>/` for a development BUILD that `NXS_SERVICE_INSTANCE` names), used by
    /// every workspace that has no `--endpoint` of its own.
    Endpoint {
        /// The URL to store. Omit to print the current default.
        url: Option<String>,
    },
    /// Show or rename THIS machine: the name every relay lists it under for the streams its
    /// background service syncs, and the id it is known by. A machine is one service home —
    /// `~/.nexusflow`, or `~/.nexusflow-<name>` for a development BUILD that
    /// `NXS_SERVICE_INSTANCE` names — so a development instance is a machine of its own. With no
    /// argument this prints the machine (minting it on first use); with a name it renames it, the
    /// id stays, and the service announces the new name on its next pass.
    Machine {
        /// The new name (up to 64 characters). Omit to print the current one.
        name: Option<String>,
    },
    /// List the machines that sync this workspace: when the relay last heard from each, and whether
    /// it is online now. A machine appears once its background service has synced the workspace
    /// (a manual `nxs sync run` never lists one); it counts as online while its last announcement
    /// is at most twice its service's cadence plus a minute old — the cadence is the interval, but
    /// never less than the 5-minute retry backoff, so 11 minutes by default. A machine nobody heard
    /// from for 30 days is no longer listed.
    Machines {
        /// Relay base URL for this one read, like `run --remote`. Optional and NOT persisted.
        #[arg(long)]
        remote: Option<String>,
    },
    /// Print this replica's signing key id — the string another machine adds with `nxs sync trust
    /// add` to believe the ops this one signs. Every op this workspace writes is signed with it;
    /// the private half stays in `.nxs/signing.key`, readable by its owner only. Compare the string
    /// with the other machine over a channel you trust, never through the relay.
    Key,
    /// Whose signed ops may carry an agent action in this workspace. Every op is kept and shown
    /// whoever signed it; only a verified signature of a key on this list — or this replica's own —
    /// lets a remote instruction act here. The list is local to this replica, never synced.
    Trust {
        #[command(subcommand)]
        action: TrustAction,
    },
    /// Show where one op came from: who signed it, whether the signature checks out, whether this
    /// workspace trusts that key — and so whether an agent action may follow it.
    Verify {
        /// An op id, or a chat message id (as `nxc` prints it) for the op behind that message.
        op_id: String,
    },
    /// Run the daemon: continuously keep every registered, bound workspace in sync — on a
    /// periodic safety-net interval, on wake, and on local write. Foreground; run it under a
    /// process supervisor (launchd et al.) to keep it alive. Unlike every other
    /// `nxs sync` verb this is machine-wide, not workspace-scoped: it iterates the WHOLE
    /// registry, so it works from any directory, including one with no `.nxs/` anywhere up the
    /// tree. With no subcommand this runs the loop (holding a single-instance lock for its
    /// whole lifetime); `status` instead just reads what the running loop last did.
    Daemon {
        /// Safety-net interval in seconds between passes when nothing else triggered one.
        /// Defaults to 300s (5 min) — wake and write-nudge are the steady-state mechanism; this
        /// only guards against missing both. Ignored when a subcommand (`status`) is given.
        #[arg(long)]
        interval: Option<u64>,
        #[command(subcommand)]
        action: Option<DaemonAction>,
    },
}

/// `nxs sync daemon` actions beyond running the loop itself.
#[derive(Subcommand, Debug)]
enum DaemonAction {
    /// Answers "is it running, and what did it last do" from the heartbeat file the running
    /// daemon rewrites after each pass — no IPC, the same file-based idiom as the write-nudge.
    /// A missing heartbeat reads as `running: false`, a state, not an error.
    Status {
        /// Compare the loaded launchd registration even though this process's `$HOME` is not the
        /// home the login session owns.
        ///
        /// The comparison is a READ and can poison nothing; it is gated all the same, because
        /// `launchctl print gui/<uid>` answers for the REAL login session whatever `$HOME` says —
        /// so from a redirected home it would hold somebody else's registration against this
        /// process's plist path and report a machine-dependent "FOREIGN" on a healthy Mac. The
        /// same flag as `install`'s, asserting the same thing: this home is redirected on purpose.
        #[arg(long = "allow-redirected-home")]
        allow_redirected_home: bool,
    },
    /// Install the daemon as a per-user launchd agent so it keeps syncing with the app closed
    /// and for CLI-/agent-driven use — `nxs sync bind` already does this
    /// automatically on every successful bind (unless `--no-daemon`); this is the explicit,
    /// retry-able form for when that best-effort autostart failed, or to (re-)install after an
    /// upgrade. Idempotent. macOS-only — fails loudly elsewhere, naming the foreground
    /// alternative (`nxs sync daemon`).
    Install {
        /// Install even though this process's `$HOME` is not the home the login session owns.
        ///
        /// Normally that mismatch is refused (nxf 6j6v.kvda): `launchctl bootstrap gui/<uid>`
        /// registers into the REAL login session whatever `$HOME` says, so a redirected home
        /// hands launchd paths the session cannot resolve — which is how a test run once left the
        /// production service unable to load for five days. On a machine whose home is redirected
        /// ON PURPOSE (MDM, a roaming profile, security tooling) both paths are real and
        /// permanent, the refusal is wrong, and this says so. Typed per invocation and never
        /// inherited from an environment — that is the whole reason it is a flag.
        #[arg(long = "allow-redirected-home")]
        allow_redirected_home: bool,
    },
    /// Stop and remove the launchd agent `install` sets up. Not an error if it was never
    /// installed. macOS-only — fails loudly elsewhere.
    Uninstall {
        /// The same escape as `install`'s, and needed for the same machine: without it, an
        /// operator whose home is redirected could install the agent and never remove it.
        #[arg(long = "allow-redirected-home")]
        allow_redirected_home: bool,
    },
}

/// The fully-built clap command tree for `nxs` (parity with `nxf`/`nxm`).
pub fn command() -> clap::Command {
    <Cli as clap::CommandFactory>::command()
}

/// Entry point for the binary — the **multicall** dispatcher (nexus-flow-5jz.7). One compiled binary
/// serves all three CLIs; its persona follows how it was invoked:
///
/// - called via the `nxf` symlink (`argv[0]` basename `nxf`) → the flow surface, byte-identical to
///   the old standalone `nxf`;
/// - via `nxm` → the memory surface;
/// - via `nxc` → the chat surface;
/// - as `nxs` → the umbrella surface (init/prime/migrate/doctor/sync), PLUS the hidden `flow`/`memory`/
///   `chat` routes (`nxs flow …` ≡ `nxf …`, `nxs memory …` ≡ `nxm …`, `nxs chat …` ≡ `nxc …`). These
///   are intercepted here, BEFORE clap parses, so they never appear in `nxs --help`.
///
/// The discriminator is `argv[0]`, NOT `std::env::current_exe()` — the latter resolves the symlink
/// back to `nxs` on Linux, losing the persona. The routed sub-tools each parse via their own
/// `run_from`, which takes the program name from a synthesized `argv[0]` so help/errors read `nxf`/
/// `nxm` exactly as the standalone tools did.
pub fn run() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    say_if_the_service_program_is_gone();
    match program_name(&args).as_deref() {
        Some("nxf") => return route(PERSONA_FLOW, || nexus_flow_cli::run_from(args)),
        Some("nxm") => return route(PERSONA_MEMORY, || nexus_memory::run_from(args)),
        // **The one place a persona's session start is wired** (nxf 6j6v.k8zq): `crates/chat` owns
        // what a persona is told and cannot know the module registry, so the binary — which is this
        // crate either way — hands it the siblings' `prime` text. Without this the spawn path
        // composes chat's half alone, which still answers, but with neither the board nor the
        // project's memories.
        Some("nxc") => {
            return route(PERSONA_CHAT, || {
                nexus_chat::run_from_with(
                    args,
                    Some(&prime::SIBLING_PRIMES),
                    Some(&crate::machines::SERVICE_MACHINES),
                )
            })
        }
        // The BACKGROUND SERVICE (6j6v.8see). macOS takes a process's displayed name from the last
        // component of the path it `exec`s, so the launchd agent runs a link named `nexus-flow` and
        // the user's background items read that instead of `nxs` — or, before this, `sh`. The
        // persona is therefore not decoration: it is the whole reason the link exists, and this arm
        // is what makes the link mean something when it is run.
        //
        // It is an ALIAS, not a fifth surface: everything after the program name is passed to
        // `nxs sync daemon`, so `nexus-flow` runs the loop, `nexus-flow status` reads the
        // heartbeat, and `--help` describes the verb it forwards to.
        //
        // **A GUARD, not a literal, since 6j6v.gd9p**: the alias's name is the INSTANCE's, so this
        // arm answers to `nexus-flow` and to `nexus-flow-dev` alike. It has to be a guard rather
        // than a list because the set is open — each repo picks its own name — and it is the ONLY
        // channel a launchd-started service has for learning which instance it is, since
        // `render_plist` writes `ProgramArguments` with exactly one element. `nxs_service::Instance`
        // owns the parse, so what routes here and what resolves a home cannot drift apart.
        Some(name) if nxs_service::Instance::named(name).is_ok() => {
            return route(PERSONA_UMBRELLA, || {
                umbrella_run(synth_service_argv(&args[1..]))
            })
        }
        _ => {}
    }
    // Invoked as `nxs`: a leading `flow`/`memory`/`chat` token routes into that tool (program name
    // synthesized), everything else is the umbrella.
    match args.get(1).and_then(|a| a.to_str()) {
        Some("flow") => route(PERSONA_FLOW, || {
            nexus_flow_cli::run_from(synth_argv(PERSONA_FLOW, &args[2..]))
        }),
        Some("memory") => route(PERSONA_MEMORY, || {
            nexus_memory::run_from(synth_argv(PERSONA_MEMORY, &args[2..]))
        }),
        Some("chat") => route(PERSONA_CHAT, || {
            // Same wiring as the `nxc` symlink above — `nxs chat …` IS `nxc …`, so a persona
            // summoned through it must be told the same things (nxf 6j6v.k8zq).
            nexus_chat::run_from_with(
                synth_argv(PERSONA_CHAT, &args[2..]),
                Some(&prime::SIBLING_PRIMES),
                Some(&crate::machines::SERVICE_MACHINES),
            )
        }),
        _ => route(PERSONA_UMBRELLA, || umbrella_run(args)),
    }
}

/// **A dead service program link is loud on EVERY invocation** (nxf 6j6v.dcpk (c)).
///
/// The background agent does not name a binary, it names an alias — and when that alias points at
/// a file that is gone, launchd's ONLY report is a service that never starts. That is the whole
/// hazard: no error, no log line, no exit code; the machine simply stops keeping time and stops
/// syncing, and stays that way. It very nearly happened twice in one week here (the disk filled and
/// build artefacts were swept), and it did happen in a sibling repo.
///
/// So the report cannot wait for the one caller who happens to need a deadline. It runs at the
/// multicall entrance, which means every `nxs`, `nxf`, `nxm`, `nxc` and `nexus-flow` invocation
/// carries it — three `stat`s and nothing else on the overwhelmingly common path where the link is
/// healthy or was never installed.
///
/// **Before the verb, not after**, including before the two verbs that would repair it. A line that
/// says "this was broken when you started" beside a successful `nxs sync daemon install` is a small
/// oddity; a line that never appears because the repair itself failed silently is the defect this
/// exists to end.
///
/// stderr, never stdout: every one of those five surfaces has a `--json` contract on stdout that
/// this must not touch. That is not the reporting class going to the wrong place — the same fact
/// reaches an app and an agent as a `service_not_running` entry in chat's `warnings` array (nxf
/// 6j6v.0j12); this is the channel for the four surfaces that have no receipt at all.
fn say_if_the_service_program_is_gone() {
    if let Some(line) = dead_service_program_line() {
        eprintln!("{line}");
    }
}

/// The macOS reading: is a launchd job installed, and what does its program link resolve to?
#[cfg(target_os = "macos")]
fn dead_service_program_line() -> Option<String> {
    let installed = nxs_service::launchd::plist_path()
        .map(|p| p.exists())
        .unwrap_or(false);
    let home = nxs_service::ServiceHome::resolve().ok()?;
    dead_program_warning(installed, &home.program_state(), &home.program())
}

/// Every other platform has no launchd job to be installed, so there is no "installed and dead"
/// state to report. `nxs sync daemon` runs in the foreground under whatever supervisor the operator
/// chose, and a supervisor that cannot find its binary reports that itself.
#[cfg(not(target_os = "macos"))]
fn dead_service_program_line() -> Option<String> {
    None
}

/// The decision, pure over its three inputs so both of its answers are provable on any platform.
///
/// It takes BOTH halves because either alone is wrong: a dangling link with no job installed is
/// leftover litter from an uninstall and worth nobody's attention, and an installed job says
/// nothing about whether its program is there.
///
/// **The sentence is NOT written here** (review of PR #393, Code Quality #2). It is
/// [`nxs_service::ServiceFault::ProgramMissing`]'s own, because this is the same fault an app reads
/// off a `service_not_running` warning and the two must not be able to describe the same machine
/// differently. Writing it out a second time is precisely what this branch's own
/// `crates/chat/src/timer/service.rs` change refactored away from, and the review caught this copy
/// surviving it — the words agreed today, and nothing was holding them together.
///
/// **Not `cfg`-gated to macOS, so its tests compile everywhere** — the same shape (and the same
/// narrow `cfg_attr`) `nxs_service::heartbeat::starttime_ticks` carries for Linux. Gating the pure
/// decision to the one platform that calls it would leave the two-halves rule unchecked on every
/// runner but one; the `allow` is scoped so that on macOS, where there IS a caller, an unused one
/// is still the lint error CI can see.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn dead_program_warning(
    installed: bool,
    state: &nxs_service::ProgramState,
    link: &Path,
) -> Option<String> {
    if !installed {
        return None;
    }
    let target = state.dangling()?;
    let fault = nxs_service::ServiceFault::ProgramMissing {
        link: link.to_path_buf(),
        target: target.to_path_buf(),
    };
    Some(format!("warning: {}", fault.detail()))
}

const PERSONA_UMBRELLA: &str = "nxs";
const PERSONA_FLOW: &str = "nxf";
const PERSONA_MEMORY: &str = "nxm";
const PERSONA_CHAT: &str = "nxc";

/// Declare the persona to the foundation, then run the route.
///
/// The foundation phrases its "no workspace" advice as that persona's own `init`, and it cannot
/// read the persona off the process argv: the `nxs flow|memory|chat` routes synthesize argv[0] for
/// the sub-tool's parser only, leaving the real argv saying `nxs`. Declaring it here keeps the
/// routed form byte-identical to the direct one, which `tests/multicall.rs` pins.
fn route(persona: &str, run: impl FnOnce() -> ExitCode) -> ExitCode {
    nxs_foundation::workspace::set_invoked_persona(persona);
    run()
}

/// The invocation name: the basename of `argv[0]`, with any `.exe` suffix stripped. `None` for an
/// empty/non-UTF-8 argv[0] (then the umbrella is used). This — not `current_exe()` — is what carries
/// the multicall persona through a symlink.
fn program_name(args: &[OsString]) -> Option<String> {
    let raw = args.first()?.to_str()?;
    let base = Path::new(raw).file_name()?.to_str()?;
    Some(base.strip_suffix(".exe").unwrap_or(base).to_string())
}

/// Build the umbrella argv a service alias (`nexus-flow`, `nexus-flow-dev`, …) stands for:
/// `nxs sync daemon <rest>`.
///
/// Separate from [`synth_argv`] because this one inserts a whole verb path rather than only
/// rewriting `argv[0]` — the alias names the SERVICE, and `nxs sync daemon` is what running the
/// service is spelled as.
fn synth_service_argv(rest: &[OsString]) -> Vec<OsString> {
    let mut argv = Vec::with_capacity(rest.len() + 4);
    argv.push(OsString::from(PERSONA_UMBRELLA));
    argv.push(OsString::from("sync"));
    argv.push(OsString::from("daemon"));
    argv.extend(rest.iter().cloned());
    argv
}

/// Build a routed sub-tool's argv: the synthesized program name at index 0 + the remaining tokens.
/// The sub-tool's clap reports THAT program name, so `nxs flow …` is byte-identical to `nxf …`.
fn synth_argv(prog: &str, rest: &[OsString]) -> Vec<OsString> {
    let mut argv = Vec::with_capacity(rest.len() + 1);
    argv.push(OsString::from(prog));
    argv.extend(rest.iter().cloned());
    argv
}

/// The `nxs` umbrella surface (init/prime/migrate/doctor/sync) over an explicit argv (argv[0] =
/// `nxs`). Split out from [`run`] so the multicall dispatcher is the single process entry.
fn umbrella_run(args: Vec<OsString>) -> ExitCode {
    let cli = Cli::parse_from(args);
    match dispatch(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => ExitCode::from(error::emit(&e, cli.json) as u8),
    }
}

fn cwd() -> Result<PathBuf> {
    std::env::current_dir().map_err(|e| NxfError::io(format!("reading current dir: {e}")))
}

/// Resolve the active workspace (an explicit `--db`/`NXS_DB`, else walk up for `.nxs/`).
fn resolve(db: Option<&str>) -> Result<Workspace> {
    Workspace::resolve(db, &cwd()?)
}

fn dispatch(cli: &Cli) -> Result<()> {
    let db = cli.db.as_deref();
    match &cli.command {
        Command::Init {
            module,
            preselect,
            plugin,
            from_beads,
            service,
            no_service,
        } => {
            // `init` is genuinely cwd-scoped (it CREATES a workspace in the current directory), so
            // an explicit `--db`/`NXS_DB` cannot be honored. Every other verb resolves a db; `init`
            // would have to ignore it — so reject it loudly rather than accept-and-ignore (the
            // no-silent-surprises CLI contract). The other verbs still honor `--db` via `db` below.
            if cli.db.is_some() {
                return Err(NxfError::validation(
                    "`init` does not accept --db/NXS_DB — it sets up a workspace in the current \
                     directory. Run `init` without --db (and unset NXS_DB), or `cd` to the target \
                     directory first.",
                ));
            }
            // The two flags are mutually exclusive at the clap level, so the tri-state below is
            // total: an explicit yes, an explicit no, or nothing said (only a terminal is asked).
            let service = match (service, no_service) {
                (true, _) => Some(true),
                (_, true) => Some(false),
                _ => None,
            };
            init::run(
                cli.json,
                module,
                preselect,
                plugin.as_deref(),
                *from_beads,
                service,
            )
        }
        Command::Prime { persona } => prime_cmd(cli.json, db, persona.as_deref()),
        // No `resolve(db)` here: guides are static content compiled into each binary, so `guide`
        // opens no store. `db` still travels in — it selects WHICH workspace's active modules are
        // fanned over — but it never reaches a child (`guide.rs`, "Two deliberate differences").
        Command::Guide { topic } => guide_cmd(cli.json, db, topic.as_deref()),
        Command::Migrate => migrate_cmd(cli.json, db),
        Command::Doctor | Command::Status => doctor_cmd(cli.json, db),
        // `setup claude` is cwd-scoped (it writes `.claude/` + agent files at the project root next
        // to `.nxs/`, resolving the workspace by walk-up), so an explicit `--db`/`NXS_DB` cannot be
        // honored. Like `init`, REJECT it loudly rather than accept-and-ignore (the no-silent-
        // surprises CLI contract): a stray `NXS_DB` pointing at another workspace must never silently
        // wire host integration into the cwd one instead.
        Command::Setup { action } => {
            if cli.db.is_some() {
                return Err(NxfError::validation(
                    "`setup claude` does not accept --db/NXS_DB — it wires host integration for the \
                     workspace in the current directory (found by walking up for `.nxs/`). Run it \
                     without --db (and unset NXS_DB), or `cd` to the target workspace first.",
                ));
            }
            match action {
                SetupAction::Claude => setup::claude(cli.json),
            }
        }
        // `sync endpoint` is machine-wide configuration (`~/.nexusflow/config.toml`), not
        // per-workspace state — it must work with no workspace at all, before one exists or
        // from any directory — so `resolve(db)` moves into the arms that actually need a
        // workspace, rather than running unconditionally for every sync subcommand. `daemon` is
        // the same shape: it serves the WHOLE registry, not the cwd's workspace, so it never
        // resolves one either.
        Command::Sync { action } => match action {
            SyncAction::Bind {
                create,
                join,
                endpoint,
                rebind,
                no_daemon,
                snapshot,
            } => {
                let ws = resolve(db)?;
                sync::bind(
                    cli.json,
                    &ws,
                    sync::BindOptions {
                        create: *create,
                        join: join.as_deref(),
                        endpoint: endpoint.as_deref(),
                        rebind: *rebind,
                        no_daemon: *no_daemon,
                        snapshot: snapshot.as_deref().map(std::path::Path::new),
                    },
                )
            }
            // No `resolve(db)` when a path is given: the entry worth pruning is often one whose
            // directory no longer exists, and resolving it would fail with the very `no_workspace`
            // error the prune is meant to clear.
            SyncAction::Unregister { path } => sync::unregister_verb(cli.json, path.as_deref()),
            SyncAction::Run { remote } => {
                let ws = resolve(db)?;
                sync::run(cli.json, &ws, remote.as_deref())
            }
            SyncAction::Snapshot { file } => {
                let ws = resolve(db)?;
                sync::snapshot_verb(cli.json, &ws, std::path::Path::new(file))
            }
            SyncAction::Endpoint { url } => sync::endpoint_verb(cli.json, url.as_deref()),
            // Machine-wide like `endpoint`: this machine is the service home's, not the cwd's
            // workspace's, so no workspace is resolved for it.
            SyncAction::Machine { name } => sync::machine_verb(cli.json, name.as_deref()),
            SyncAction::Machines { remote } => {
                let ws = resolve(db)?;
                sync::machines_verb(cli.json, &ws, remote.as_deref())
            }
            SyncAction::Key => sync::trust::key_verb(cli.json, &resolve(db)?),
            SyncAction::Trust { action } => {
                let ws = resolve(db)?;
                match action {
                    TrustAction::List => sync::trust::list_verb(cli.json, &ws),
                    TrustAction::Add { key_id, name } => sync::trust::add_verb(
                        cli.json,
                        &ws,
                        key_id,
                        name.as_deref(),
                        &crate::prime::resolve_now()?,
                    ),
                    TrustAction::Remove { key_id } => {
                        sync::trust::remove_verb(cli.json, &ws, key_id)
                    }
                }
            }
            SyncAction::Verify { op_id } => {
                sync::trust::verify_verb(cli.json, &resolve(db)?, op_id)
            }
            SyncAction::Daemon { interval, action } => match action {
                None => sync::daemon::serve(cli.json, *interval),
                Some(DaemonAction::Status {
                    allow_redirected_home,
                }) => sync::daemon::status(cli.json, *allow_redirected_home),
                Some(DaemonAction::Install {
                    allow_redirected_home,
                }) => sync::launchd::install(cli.json, *allow_redirected_home),
                Some(DaemonAction::Uninstall {
                    allow_redirected_home,
                }) => sync::launchd::uninstall(cli.json, *allow_redirected_home),
            },
        },
        // `self-update` is owned end-to-end by the cli crate — the updater contract, the embedded
        // signing key, and the suite-wide atomic swap all live there, and the umbrella already links
        // it. Delegating keeps ONE updater + ONE swap behind both `nxs self-update` (canonical) and
        // the hidden `nxf self-update` (nexus-flow-gel). It returns the SHARED error envelope, so it
        // flows through the umbrella's normal error rendering with no type mapping. `init`-style
        // `--db` does not apply (this updates the install, not a workspace), so it is ignored here.
        Command::SelfUpdate { channel, check } => {
            nexus_flow_cli::self_update(cli.json, *check, channel.as_deref())
        }
        // The MCP seam (#76u). `--db`/`NXS_DB` still applies as the lower-level store override; the
        // per-instance default is the launch `--workspace`/`NXS_WORKSPACE` (MCP hosts have no cwd).
        #[cfg(feature = "mcp")]
        Command::Mcp { action } => match action {
            McpAction::Serve { workspace, actor } => {
                crate::mcp::serve(db, workspace.as_deref(), actor.as_deref())
            }
            // `install` writes host config, not the workspace store, so `--db`/`NXS_DB` does not
            // apply; the pinned launch workspace is the entry's own `--workspace` (optional). With
            // `--setup`/`--plugin` it ALSO materializes that workspace (#65y).
            McpAction::Install {
                workspace,
                host,
                setup,
                plugin,
                runner,
            } => crate::mcp::install(
                cli.json,
                workspace.as_deref(),
                host,
                *setup,
                plugin.as_deref(),
                runner,
            ),
            // Read-only registry view; `--db`/`NXS_DB` does not apply (the registry is a global,
            // per-user file, not a workspace store).
            McpAction::Workspaces => crate::mcp::workspaces(cli.json),
        },
    }
}

fn prime_cmd(json: bool, db: Option<&str>, persona: Option<&str>) -> Result<()> {
    let ws = resolve(db)?;
    if let Some(handle) = persona {
        return persona_prime_cmd(json, &ws, handle);
    }
    let fan = prime::fan_out(&ws, json)?;
    if json {
        // {"modules":[{"module":..,"prime":<module's prime json>}],"now":..} — keys sorted by
        // serde_json (deterministic); the per-module `prime` is that module's own --json record.
        let mut modules = Vec::with_capacity(fan.modules.len());
        for mp in &fan.modules {
            let value: serde_json::Value = serde_json::from_str(mp.raw.trim()).map_err(|e| {
                NxfError::io(format!("parsing `{} prime --json` output: {e}", mp.module))
            })?;
            modules.push(serde_json::json!({ "module": mp.module, "prime": value }));
        }
        let out = serde_json::json!({ "now": fan.now, "modules": modules });
        println!("{out}");
    } else if fan.modules.is_empty() {
        println!("# nxs\n\n_No active modules — run `nxf init` or `nxm init` to set one up._");
    } else {
        // Concatenate each module's prime markdown, framing-normalized, in fan-out order.
        let sections: Vec<String> = fan
            .modules
            .iter()
            .map(|m| m.raw.trim().to_string())
            .collect();
        let composed = sections.join("\n\n");
        // The live tripwire on the host's cut-off (nxf 6j6v.xbnh): measured on the COMPOSED block,
        // because that is what the host weighs, and appended last so the warning is the final thing
        // a reader sees. Silent in the ordinary case — see `prime::ceiling_warning`.
        match prime::ceiling_warning(composed.len()) {
            Some(warning) => println!("{composed}\n\n{warning}"),
            None => println!("{composed}"),
        }
    }
    Ok(())
}

/// `nxs prime --persona <handle>`: **what a declared persona is handed at its session start**
/// (nxf 6j6v.k8zq).
///
/// Two things make it different from the plain fan-out above, and both come from the declaration:
///
/// - **The persona's `prime:` filter decides which modules contribute.** A pure reviewer that
///   excludes the board gets no `nxf prime` section at all — not an empty one — while a PM's block
///   still carries it. The filter is read here, from the persona's own file, and passed down; it is
///   never a global or an environment setting.
/// - **chat's section is that persona's own**, composed rather than fanned out to: identity,
///   address book and answering rules, exactly as `nxc prime --persona <handle>` renders them.
///
/// It goes through [`prime::persona_block`] — the SAME function `nxc`'s spawn path composes a
/// system prompt from — so what this prints is what that persona reads, not a second assembly of
/// the same pieces.
///
/// `--json` wraps the composed text rather than the modules' records: this verb answers "what does
/// this persona read", and the answer is one document. The per-module JSON is what the plain
/// `nxs prime --json` is for.
fn persona_prime_cmd(json: bool, ws: &Workspace, handle: &str) -> Result<()> {
    let text = prime::persona_block(ws, handle)?;
    if json {
        println!(
            "{}",
            serde_json::json!({ "persona": handle, "prime": text })
        );
    } else {
        println!("{text}");
    }
    Ok(())
}

/// `nxs guide [topic]`: the suite-wide listing, or one block's guide.
///
/// # The `--json` contract (v1, 6j6v.9e3r)
///
/// The per-tool verbs' contract is untouched — `nxf guide --json` still emits exactly
/// `[{"summary":…,"topic":…}]`. `nxs guide --json` WRAPS those records rather than reshaping them,
/// mirroring how `nxs prime --json` wraps each module's own prime record:
///
/// ```text
/// nxs guide --json          {"modules":[{"binary":"nxf","module":"flow","topics":[{"summary":…,"topic":…}]}]}
/// nxs guide <topic> --json  {"binary":"nxf","content":"<raw markdown>","module":"flow","topic":"commands"}
/// ```
///
/// A block with no guides yet appears with `"topics":[]` — it is part of the suite whether or not
/// its content is written, and an agent enumerating the suite must see it.
fn guide_cmd(json: bool, db: Option<&str>, topic: Option<&str>) -> Result<()> {
    let modules = guide::resolve_modules(db, &cwd()?)?;
    let mut fan = guide::fan_out(&modules)?;
    // The umbrella's own guides go FIRST (6j6v.0fvt): first setup and the map of the blocks are
    // what a reader needs before they know the blocks exist, so they head the listing rather than
    // trailing three products they have not chosen between yet.
    fan.modules.insert(0, guide::umbrella_guide());
    // …and the develop group LAST (6j6v.jepw): architecture and the op-log's internals are what a
    // reader reaches for after they know what the blocks are, not before.
    fan.modules.push(guide::develop_guide());
    match topic {
        None => list_guides(json, &fan),
        Some(t) => show_guide(json, &fan, t),
    }
}

/// No topic: every fanned-over block with its own topics, in fan-out order.
fn list_guides(json: bool, fan: &guide::GuideFanOut) -> Result<()> {
    if json {
        let modules: Vec<serde_json::Value> = fan
            .modules
            .iter()
            .map(|m| {
                let topics: Vec<serde_json::Value> = m
                    .topics
                    .iter()
                    .map(|t| serde_json::json!({ "topic": t.topic, "summary": t.summary }))
                    .collect();
                serde_json::json!({ "module": m.module, "binary": m.binary, "topics": topics })
            })
            .collect();
        println!("{}", serde_json::json!({ "modules": modules }));
        return Ok(());
    }
    // Defensive, and NOT the "no active modules" case `prime` has: `resolve_modules` falls back to
    // the full roster, so an empty fan-out means this binary links no product at all — which the
    // shipped `nxs` never is (main.rs links all three). Say what would actually be true rather than
    // pointing at `nxs init`, which could not fix it.
    if fan.modules.is_empty() {
        println!("_This build links no products, so it carries no guides._");
        return Ok(());
    }
    // One block per paragraph, each headed by the binary a reader types. The column width is
    // computed across ALL blocks so the summaries line up down the whole listing, not per block.
    let width = fan
        .modules
        .iter()
        .flat_map(|m| m.topics.iter().map(|t| t.topic.len()))
        .max()
        .unwrap_or(0);
    let mut first = true;
    for m in &fan.modules {
        if !first {
            println!();
        }
        first = false;
        println!("{} — run `{} guide <topic>`:\n", m.module, m.binary);
        if m.topics.is_empty() {
            println!("  (no guides yet)");
            continue;
        }
        for t in &m.topics {
            println!("  {:<width$}  {}", t.topic, t.summary);
        }
    }
    Ok(())
}

/// A topic: resolve which block serves it (never guessing between several), fetch its raw markdown
/// from that block, and render it with the SAME renderer the block itself uses — so the guide text
/// is byte-identical to `<binary> guide <topic>`, down to the code-block indentation.
fn show_guide(json: bool, fan: &guide::GuideFanOut, topic: &str) -> Result<()> {
    let owner = fan.resolve(topic)?;
    // The umbrella's own topics are compiled into THIS binary, so they are read straight from the
    // catalog. Going through `fetch_topic` would spawn `nxs guide <topic> --json` from inside
    // `nxs guide` — an unbounded recursion, not a slow path.
    let content = if let Some(catalog) = guide::catalog_for(&owner.module) {
        catalog.lookup(topic)?.to_string()
    } else {
        guide::fetch_topic(&owner.binary, topic)?
    };
    if json {
        let out = serde_json::json!({
            "module": owner.module,
            "binary": owner.binary,
            "topic": topic,
            "content": content,
        });
        println!("{out}");
    } else {
        print!("{}", nxs_guide::render_markdown(&content));
    }
    Ok(())
}

fn migrate_cmd(json: bool, db: Option<&str>) -> Result<()> {
    let ws = resolve(db)?;
    let report = migrate::migrate(&ws)?;
    if json {
        println!("{}", to_json(&report)?);
        return Ok(());
    }
    if report.changed {
        println!(
            "migrated workspace db: schema v{} → v{}",
            report.from, report.to
        );
    } else {
        println!("workspace db already current (schema v{})", report.to);
    }
    // v0.6.0 umbrella upgrade (aye.32) — report only when it actually did something.
    if !report.modules_registered.is_empty() {
        println!(
            "  registered active module(s): {}",
            report.modules_registered.join(", ")
        );
    }
    if report.hook_rewritten {
        // The target is one entry per active module since nxf n2m6 + a2a1, so the line names what
        // was actually wired — via the assembler's own renderer, not a sixth hand-rolled list.
        //
        // **The lead-in stopped naming `nxf prime` in nxf 6j6v.sp5v**, when the rewrite grew to
        // cover the intermediate single `nxs prime` hook as well: a line that named the one shape
        // it used to find would now be wrong for the shape it actually found most of the time.
        println!(
            "  outdated SessionStart wiring rewritten — {}",
            nxs_init::assembler::describe_hooks(true, &report.hook_commands)
        );
    }
    Ok(())
}

fn doctor_cmd(json: bool, db: Option<&str>) -> Result<()> {
    let ws = resolve(db)?;
    let d = doctor::diagnose(&ws)?;
    if json {
        println!("{}", to_json(&d)?);
        return Ok(());
    }
    use doctor::SchemaStatus::*;
    let modules = if d.active_modules.is_empty() {
        "(none)".to_string()
    } else {
        d.active_modules.join(", ")
    };
    let schema = match d.schema_status {
        Current => format!("v{} (current)", d.schema_version),
        Degraded => format!(
            "v{} (degraded — this nxs speaks v{})",
            d.schema_version, d.schema_supported
        ),
        Incompatible => format!(
            "v{} (INCOMPATIBLE — written by a newer nxs; upgrade nxs (this one speaks v{}))",
            d.schema_version, d.schema_supported
        ),
    };
    println!("nxs workspace at {}", d.workspace);
    println!("  modules:    {modules}");
    println!("  replica:    prefix {} · site {}", d.prefix, d.site_id);
    println!("  schema:     {schema}");
    println!(
        "  sync:       {}",
        if d.sync_bound { "bound" } else { "not bound" }
    );
    println!("  ops:        {}", d.op_count);
    // Silent on a clean log, like the beads line below: only a log written before 6j6v.fc5p can
    // carry one, and a line that always prints "0" teaches a reader to skip it.
    if d.duplicate_coordinates > 0 {
        println!(
            "  coordinates: ⚠ {} (lamport, site) coordinate{} held by more than one op — written \
             before nxs could refuse it, by two processes sharing a stale clock. On such a pair \
             a concurrent edit is decided by the order the ops arrived, not by the CRDT order; \
             nothing is lost, and an append-only log cannot be repaired, so this is for knowing, \
             not fixing",
            d.duplicate_coordinates,
            if d.duplicate_coordinates == 1 {
                ""
            } else {
                "s"
            }
        );
    }
    println!(
        "  integrity:  {}",
        if d.integrity_ok { "ok" } else { "FAILED" }
    );
    // Surface a half-finished beads → nxs migration (nexus-flow-6ef): a clean workspace prints
    // nothing here, so existing output is unchanged.
    if d.beads_residue {
        println!(
            "  beads:      ⚠ config still wired (a half-finished migration) — remove the \
             `BEADS INTEGRATION` block + `bd prime` hook, or re-run `nxs init --from-beads`"
        );
    }
    Ok(())
}

/// Serialize a report to its `--json` line (declaration field order; serde preserves it).
fn to_json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| NxfError::io(format!("serializing output: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<OsString> {
        parts.iter().map(OsString::from).collect()
    }

    #[test]
    fn program_name_is_the_basename_of_argv0() {
        // The multicall persona is the invocation NAME, not the path it was reached through — a
        // path-laden argv[0] (an absolute install path, or `./nxf`) still routes by its basename.
        assert_eq!(program_name(&argv(&["nxf"])).as_deref(), Some("nxf"));
        assert_eq!(
            program_name(&argv(&["/usr/local/bin/nxm"])).as_deref(),
            Some("nxm")
        );
        assert_eq!(program_name(&argv(&["./nxs"])).as_deref(), Some("nxs"));
    }

    #[test]
    fn program_name_strips_a_windows_exe_suffix() {
        // The persona match is against bare `nxf`/`nxm`, so a Windows `nxf.exe` must strip to `nxf`
        // (the route table never carries `.exe`). This is the edge the unix integration test cannot
        // reach — argv[0] there never ends in `.exe`.
        assert_eq!(program_name(&argv(&["nxf.exe"])).as_deref(), Some("nxf"));
        assert_eq!(
            program_name(&argv(&["/opt/suite/nxm.exe"])).as_deref(),
            Some("nxm")
        );
    }

    #[test]
    fn program_name_is_none_for_an_empty_argv() {
        // No argv[0] at all → no persona → the caller falls through to the `nxs` umbrella.
        assert_eq!(program_name(&[]), None);
    }

    #[cfg(unix)]
    #[test]
    fn program_name_is_none_for_a_non_utf8_argv0() {
        // A non-UTF-8 argv[0] cannot name a persona (the routes are UTF-8 literals); `to_str()`
        // fails, so we fall through to the umbrella rather than panicking on the bad bytes.
        use std::os::unix::ffi::OsStringExt;
        let bad = OsString::from_vec(vec![0x6e, 0xff, 0x66]); // "n\xfff" — invalid UTF-8
        assert_eq!(program_name(&[bad]), None);
    }

    #[test]
    fn synth_argv_prepends_the_synthesized_program_name() {
        // `nxs flow guide --json` → the flow parser must see argv `["nxf","guide","--json"]`, so its
        // clap reports program-name `nxf` (byte-identical to the standalone tool).
        let out = synth_argv("nxf", &argv(&["guide", "--json"]));
        assert_eq!(out, argv(&["nxf", "guide", "--json"]));
    }

    #[test]
    fn synth_argv_with_no_rest_is_just_the_program_name() {
        // `nxs memory` with no further tokens → `["nxm"]`, i.e. bare `nxm` (its own no-args behavior).
        assert_eq!(synth_argv("nxm", &[]), argv(&["nxm"]));
    }

    #[test]
    fn self_update_verb_parses_with_its_flags() {
        // nexus-flow-gel: `nxs self-update` is a first-class umbrella verb carrying the same
        // `--channel`/`--check`/`--json` surface as the (now hidden) `nxf self-update`.
        let cli = Cli::try_parse_from([
            "nxs",
            "self-update",
            "--channel",
            "beta",
            "--check",
            "--json",
        ])
        .expect("`nxs self-update` parses with its flags");
        assert!(cli.json);
        match cli.command {
            Command::SelfUpdate { channel, check } => {
                assert_eq!(channel.as_deref(), Some("beta"));
                assert!(check);
            }
            other => panic!("expected SelfUpdate, got {other:?}"),
        }
    }

    #[test]
    fn self_update_rejects_an_unknown_channel() {
        // The promotion ring is constrained to the three known channels (parity with `nxf`).
        assert!(Cli::try_parse_from(["nxs", "self-update", "--channel", "nightly"]).is_err());
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp_install_parses_workspace_and_repeatable_host() {
        // #c3i: `nxs mcp install` carries an optional `--workspace` (pin the entry's launch root)
        // and a repeatable `--host` (target specific hosts). Bare `install` leaves both empty.
        let cli = Cli::try_parse_from([
            "nxs",
            "mcp",
            "install",
            "--workspace",
            "/proj",
            "--host",
            "cursor",
            "--host",
            "claude-desktop",
        ])
        .expect("`nxs mcp install` parses with its flags");
        match cli.command {
            Command::Mcp {
                action:
                    McpAction::Install {
                        workspace,
                        host,
                        setup,
                        plugin,
                        runner,
                    },
            } => {
                assert_eq!(workspace.as_deref(), Some("/proj"));
                assert_eq!(
                    host,
                    vec!["cursor".to_string(), "claude-desktop".to_string()]
                );
                assert!(!setup, "setup is off unless requested");
                assert_eq!(plugin, None);
                assert_eq!(runner, "native", "runner defaults to native");
            }
            other => panic!("expected Mcp/Install, got {other:?}"),
        }
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp_install_defaults_to_no_workspace_and_no_host() {
        let cli =
            Cli::try_parse_from(["nxs", "mcp", "install"]).expect("bare `nxs mcp install` parses");
        match cli.command {
            Command::Mcp {
                action:
                    McpAction::Install {
                        workspace,
                        host,
                        setup,
                        plugin,
                        runner,
                    },
            } => {
                assert_eq!(workspace, None);
                assert!(host.is_empty());
                assert!(!setup);
                assert_eq!(plugin, None);
                assert_eq!(runner, "native");
            }
            other => panic!("expected Mcp/Install, got {other:?}"),
        }
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp_install_parses_setup_and_plugin() {
        // #65y: `--setup` opts into workspace materialization and `--plugin` chooses which plugin to
        // seat (the coding case picks issue-tracker).
        let cli = Cli::try_parse_from([
            "nxs",
            "mcp",
            "install",
            "--setup",
            "--plugin",
            "issue-tracker",
        ])
        .expect("`nxs mcp install --setup --plugin` parses");
        match cli.command {
            Command::Mcp {
                action:
                    McpAction::Install {
                        setup,
                        plugin,
                        workspace,
                        host,
                        runner,
                    },
            } => {
                assert!(setup);
                assert_eq!(plugin.as_deref(), Some("issue-tracker"));
                assert_eq!(workspace, None);
                assert!(host.is_empty());
                assert_eq!(runner, "native");
            }
            other => panic!("expected Mcp/Install, got {other:?}"),
        }
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp_install_parses_runner_npx() {
        // #kz8: `--runner npx` selects the portable npx shim entry over the default absolute-path one.
        let cli = Cli::try_parse_from(["nxs", "mcp", "install", "--runner", "npx"])
            .expect("`nxs mcp install --runner npx` parses");
        match cli.command {
            Command::Mcp {
                action: McpAction::Install { runner, .. },
            } => assert_eq!(runner, "npx"),
            other => panic!("expected Mcp/Install, got {other:?}"),
        }
    }

    #[test]
    fn a_dead_program_link_is_reported_only_when_a_job_is_actually_installed() {
        use nxs_service::ProgramState;
        let link = Path::new("/home/u/.nexusflow/bin/nexus-flow");
        let dead = ProgramState::Dangling(PathBuf::from("/repo/target/debug/nxs"));

        let line = dead_program_warning(true, &dead, link)
            .expect("installed + dead target is the whole hazard");
        assert!(line.contains("/home/u/.nexusflow/bin/nexus-flow"), "{line}");
        assert!(line.contains("/repo/target/debug/nxs"), "{line}");
        assert!(line.contains("nxs sync daemon install"), "{line}");
        assert!(
            line.contains("windows will not fire") && line.contains("neither pushes nor pulls"),
            "both halves of the service's work, not only the clock: {line}"
        );
        // **And it is the FAULT's own sentence, not a second copy of it** (review of PR #393, Code
        // Quality #2). The four `contains` above would all still pass against a hand-written
        // paraphrase; this is what makes the two say the same thing by construction, so a future
        // edit to `ServiceFault::detail()` cannot leave this entrance describing the same machine
        // differently.
        assert_eq!(
            line,
            format!(
                "warning: {}",
                nxs_service::ServiceFault::ProgramMissing {
                    link: link.to_path_buf(),
                    target: PathBuf::from("/repo/target/debug/nxs"),
                }
                .detail()
            )
        );

        assert_eq!(
            dead_program_warning(false, &dead, link),
            None,
            "a dangling link with no job installed is leftover litter, not a broken machine"
        );
    }

    #[test]
    fn a_healthy_or_absent_program_link_says_nothing_at_all() {
        use nxs_service::ProgramState;
        let link = Path::new("/home/u/.nexusflow/bin/nexus-flow");
        assert_eq!(
            dead_program_warning(
                true,
                &ProgramState::Present(PathBuf::from("/home/u/.local/bin/nxs")),
                link
            ),
            None
        );
        assert_eq!(
            dead_program_warning(true, &ProgramState::Absent, link),
            None,
            "no alias at all is a machine that never installed one, and every invocation staying \
             silent about that is the point"
        );
    }
}
