//! nxm — the nexus-memory agent CLI.
//!
//! A thin, deterministic interface over [`MemoryStore`]. Every command resolves a `.nxs/`
//! workspace (or creates one via `init`) and reports failures through the shared structured error
//! envelope. `--json` everywhere is the agent contract; records carry a stable, declared field
//! order (serde preserves declaration order), so the machine output is byte-stable for goldens.

use crate::error::{self, NxfError, Result};
use crate::facade::{self, Classification, MemoryRecord};
use crate::import;
use crate::migration;
use crate::model::Scope;
use crate::onboarding;
use crate::project_doc;
use crate::store::{MemoryQuery, MemoryStore};
use crate::workspace::{self, MemoryWorkspaceExt, Workspace};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::process::ExitCode;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// nexus-memory agent CLI.
#[derive(Parser, Debug)]
#[command(name = "nxm", version, about = "nexus-memory agent CLI", long_about = None)]
struct Cli {
    /// Emit machine-readable JSON instead of human output.
    #[arg(long, global = true)]
    json: bool,

    /// Use this sqlite db file directly instead of discovering `.nxs/`.
    #[arg(long, global = true, env = "NXM_DB")]
    db: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Initialize memory in the current directory: ensure a `.nxs` workspace, register the memory
    /// module, then delegate the shared agent file + one SessionStart hook per active module to
    /// the `nxs` assembler (re-assembling every active module).
    Init {
        /// Driven/quiet mode: set up memory and delegate the agent files + hook, but render no
        /// banner. The seam the `nxs` umbrella uses to drive `nxm init` without output bleeding
        /// through; also the non-interactive agent entry.
        #[arg(long)]
        quiet: bool,
        /// Consent to the beads → nxs migration: import beads tickets + memories
        /// and roll back beads' config. Routed through the `nxs` umbrella, which owns the migration.
        #[arg(long = "from-beads")]
        from_beads: bool,
    },
    /// Emit nxm's declared contribution to the shared agent file (the AGENTS.md section,
    /// the prime command, the SessionStart hook) as data. `--json` is the machine contract the
    /// `nxs` umbrella assembles from.
    AgentManifest,
    /// Session bootstrap: the memory rule + the key commands + the memory index as a context
    /// block, the index bounded by the byte budget a host's SessionStart hook actually delivers
    /// (`nxm index` serves the whole of it). The bd-prime equivalent. Hidden from `--help`
    /// (nexus-flow-fyr): the umbrella
    /// `nxs prime` is the single user-facing entry and fans out to this verb as a subprocess —
    /// `nxm prime` stays a working internal seam, just no longer advertised.
    #[command(hide = true)]
    Prime,
    /// Remember a fact: create it, or update it in place under a stable `--key`. With no `--key`
    /// the key is a content hash (byte-identical bodies dedup; a reworded body gets a new key).
    /// Runs entirely offline and deterministically — nothing here asks a model.
    Remember {
        /// The fact text to store.
        text: String,
        /// A stable, human-chosen key to upsert in place (e.g. `auth-jwt`). Omit for an auto-key.
        #[arg(long)]
        key: Option<String>,
        /// REQUIRED. The one line every session start is handed for this memory — at most 200
        /// characters, no line breaks. The body is read on demand with `nxm recall <key>`; this
        /// line is what decides whether anyone does. Say what the memory SAYS, not what it is
        /// about. For `--category rules` it SPEAKS the rule — "Never run X while Y is running",
        /// not "Rules about X" — because nobody looks a prohibition up before breaking it.
        #[arg(long)]
        introduction: String,
        #[command(flatten)]
        class: ClassArgs,
    },
    /// File a memory: set its category, reach, references and/or introduction, leaving its text
    /// untouched.
    Classify {
        /// The memory key.
        key: String,
        /// Rewrite the one line every session start is handed for this memory — at most 200
        /// characters, no line breaks, and for `--category rules` it SPEAKS the rule rather than
        /// announcing it. This is also how a memory written before introductions existed gets one.
        #[arg(long)]
        introduction: Option<String>,
        #[command(flatten)]
        class: ClassArgs,
    },
    /// Store an explicit reading order: the first key becomes position 1, the second 2, and so on.
    /// Deliberate and rare — everyday writes never touch the order, and a memory that was never
    /// reordered keeps sorting by insertion order, after every placed one.
    Reorder {
        /// The memory keys, in the order they should read.
        #[arg(required = true)]
        keys: Vec<String>,
    },
    /// Recall the full text of one memory by key.
    Recall {
        /// The memory key.
        key: String,
    },
    /// The whole memory index: one line per memory — its key and the introduction its author wrote
    /// — for every memory the session start replays, in the same reading order and unbounded in
    /// count. `nxm prime` renders as much of this index as its byte budget allows and names this
    /// verb for the rest. Read the lines first, then open the one that matters with
    /// `nxm recall <key>`.
    Index,
    /// List active memories, or search them by a case-insensitive substring over key and body.
    Memories {
        /// Optional search term (substring over key + body).
        search: Option<String>,
        /// Only memories filed under this category.
        #[arg(long)]
        category: Option<String>,
        /// Only memories with this reach.
        #[arg(long, value_enum)]
        scope: Option<ScopeArg>,
        /// List in the stored reading order instead of by key.
        #[arg(long)]
        ordered: bool,
    },
    /// Print the generated project-memory document — the `NEXUS_MEMORY.md` projection of this
    /// workspace's memories, which every write regenerates. Nothing here writes; `--check` compares
    /// the file on disk with the store and exits non-zero when they have drifted apart.
    Doc {
        /// Compare `NEXUS_MEMORY.md` with the store instead of printing it, and fail if it was
        /// hand-edited or never regenerated.
        #[arg(long)]
        check: bool,
    },
    /// The judging migration: file this workspace's memories and move its hand-written context
    /// document into them, section by section. Explicitly invoked, run once, and the one command
    /// here that may ask a model.
    Migrate {
        #[command(subcommand)]
        command: MigrateCommand,
    },
    /// Forget a memory by key (a reversible tombstone — a later `remember` revives it).
    Forget {
        /// The memory key.
        key: String,
    },
    /// Import an existing Claude-host memory store into nexus-memory. Each fact is taken
    /// in under its frontmatter `name:` as a stable key, so a re-run upserts in place (idempotent)
    /// and the source is never modified. With no `--from`, the Claude memory dir for this project
    /// is auto-detected.
    Import {
        /// The Claude-host memory directory to import from (a `memory/` dir with a `MEMORY.md`
        /// index + per-fact markdown files). Omit to auto-detect this project's Claude memory.
        #[arg(long)]
        from: Option<String>,
    },
    /// Print an embedded, offline guide. Omit the topic to list the available ones.
    ///
    /// One of the three building blocks' guide verbs (6j6v.9e3r): `nxm guide` serves MEMORY's
    /// topics, `nxf guide` flow's, `nxc guide` chat's, and `nxs guide` fans out over the active
    /// modules. Needs no workspace — the content is compiled into the binary.
    Guide {
        /// Guide topic; omit to list topics.
        topic: Option<String>,
    },
}

/// The three steps of the judging migration (6j6v.9yaj), split because the middle one is a
/// judgement and belongs to a human: `plan` reads and proposes, somebody (or a model) decides, and
/// `apply` writes. One document travels between them, so the thing reviewed IS the thing applied.
#[derive(Subcommand, Debug)]
enum MigrateCommand {
    /// Propose a migration: every memory still filed as `unsorted`, plus every section of this
    /// workspace's hand-written context documents, as one plan document. Writes nothing. `--json`
    /// emits the document `apply` takes back.
    Plan {
        /// Ask a coding assistant to decide the plan instead of leaving the decisions at their
        /// status quo. Bare `--with-judge` uses `claude`. This is the sanctioned model call: one
        /// process, started because a human asked for it, outside every write path.
        #[arg(
            long = "with-judge",
            value_name = "ASSISTANT",
            num_args = 0..=1,
            default_missing_value = "claude"
        )]
        with_judge: Option<String>,
        /// How long to wait for the judge's answer: `<n>` plus `s`/`m`/`h` (`90s`, `30m`, `2h`), or
        /// `0` to wait as long as it takes. Generous by default — a judge reading a large plan
        /// legitimately thinks for minutes, and a limit that interrupts it costs a whole judgement.
        #[arg(
            long = "judge-timeout",
            value_name = "DURATION",
            default_value = migration::DEFAULT_JUDGE_TIMEOUT_SPELLING,
            requires = "with_judge"
        )]
        judge_timeout: String,
        /// Plan again even though this stream already carries a completed migration. Without it a
        /// second device reports the earlier run and stops, rather than producing a second,
        /// independent filing of the same memories for last-writer-wins to blend.
        #[arg(long)]
        again: bool,
    },
    /// Execute a decided plan: file the memories, write the sections as memories, keep the plan's
    /// sequence as the reading order, move the migrated sections out of their documents, and leave
    /// the mark. Refuses a plan whose entries nobody judged.
    Apply {
        /// The plan document, or `-` to read it from stdin.
        #[arg(long, value_name = "FILE")]
        plan: String,
        /// Leave the source documents untouched. Off by default: the migration is a MOVE, and a
        /// copy leaves two sources for one truth.
        #[arg(long)]
        keep_sources: bool,
        /// Report what would happen and write nothing at all.
        #[arg(long)]
        dry_run: bool,
        /// Apply even though this stream already carries a completed migration. Without it, a
        /// second run — on another device, or from a plan file saved before the first one — is
        /// refused: two independent filings of the same memories converge by last-writer-wins into
        /// a blend neither of them decided.
        #[arg(long)]
        again: bool,
    },
    /// Report what is still unfiled here and whether a migration already ran on this stream.
    Status,
}

/// How far a memory reaches, as the CLI spells it (6j6v.e0z6) — the [`Scope`] enum with clap's
/// `--help` rendering attached. Kept separate so the vocabulary stays in `model` and the CLI owns
/// only its presentation.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum ScopeArg {
    /// Reaches only the board items the memory names (`--refs`).
    Item,
    /// Reaches this workspace — the default, and what every memory did before reaches existed.
    Project,
    /// Reaches beyond this workspace.
    Global,
}

impl From<ScopeArg> for Scope {
    fn from(s: ScopeArg) -> Scope {
        match s {
            ScopeArg::Item => Scope::Item,
            ScopeArg::Project => Scope::Project,
            ScopeArg::Global => Scope::Global,
        }
    }
}

/// The classification flags shared by `remember` and `classify` (6j6v.e0z6). Declared once and
/// flattened into both, so the two verbs cannot drift apart. An omitted flag leaves that register
/// untouched — it is not a request to reset it to the default.
#[derive(Args, Debug, Clone, Default)]
struct ClassArgs {
    /// File the memory under this section — a lower-case slug (e.g. `introduction`,
    /// `architecture`, `rules`). Unclassified memories stay `unsorted`.
    #[arg(long)]
    category: Option<String>,
    /// How far the memory reaches. Defaults to `project` — what every memory already did.
    #[arg(long, value_enum)]
    scope: Option<ScopeArg>,
    /// The board items this memory is about, comma-separated (e.g. `ab12.0007,ab12.0011`).
    /// Replaces the whole set; pass an empty value to clear it.
    #[arg(long)]
    refs: Option<String>,
}

impl ClassArgs {
    /// The [`Classification`] these shared flags ask for, with the verb's OWN `--introduction`
    /// folded in (6j6v.xbnh). Declared per verb rather than here because the two verbs differ on
    /// exactly one point — it is REQUIRED on `remember` and optional on `classify` — and that is a
    /// difference clap can only express at the variant that owns it.
    fn with_introduction(&self, introduction: Option<String>) -> Classification {
        Classification {
            introduction,
            ..Classification::from(self)
        }
    }
}

impl From<&ClassArgs> for Classification {
    fn from(a: &ClassArgs) -> Classification {
        Classification {
            category: a.category.clone(),
            scope: a.scope.map(Scope::from),
            // `--refs ""` is an explicit "no references", NOT an absent flag: `Some(vec![])` clears
            // the register, `None` leaves it alone.
            refs: a
                .refs
                .as_deref()
                .map(|r| r.split(',').map(str::trim).map(str::to_string).collect()),
            // The verb's own flag supplies this ([`ClassArgs::with_introduction`]).
            introduction: None,
        }
    }
}

/// The fully-built clap command tree for `nxm`.
pub fn command() -> clap::Command {
    <Cli as clap::CommandFactory>::command()
}

/// Entry point for the `nxm` binary. [`run_from`] is the multicall seam the `nxs` umbrella routes
/// `nxm`/`nxs memory` through (nexus-flow-5jz.7).
pub fn run() -> ExitCode {
    run_from(std::env::args_os())
}

/// Run the `nxm` surface over an explicit argv INCLUDING the program name at index 0 (multicall
/// seam). clap takes the program name from argv0 — a synthesized `nxm` (from `nxs memory …`)
/// renders identical `nxm` help/errors — and parses the rest. Otherwise identical to `run()`.
pub fn run_from<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    match dispatch(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => ExitCode::from(error::emit(&e, cli.json) as u8),
    }
}

fn dispatch(cli: &Cli) -> Result<()> {
    let db = cli.db.as_deref();
    match &cli.command {
        Command::Init { quiet, from_beads } => init(cli.json, *quiet, *from_beads),
        Command::AgentManifest => agent_manifest(cli.json),
        Command::Prime => prime(cli.json, db),
        Command::Index => index(cli.json, db),
        Command::Remember {
            text,
            key,
            introduction,
            class,
        } => remember(
            cli.json,
            db,
            text,
            key.as_deref(),
            &class.with_introduction(Some(introduction.clone())),
        ),
        Command::Classify {
            key,
            introduction,
            class,
        } => classify(
            cli.json,
            db,
            key,
            &class.with_introduction(introduction.clone()),
        ),
        Command::Reorder { keys } => reorder(cli.json, db, keys),
        Command::Recall { key } => recall(cli.json, db, key),
        Command::Memories {
            search,
            category,
            scope,
            ordered,
        } => memories(
            cli.json,
            db,
            &MemoryQuery {
                search: search.clone(),
                category: category.clone(),
                scope: scope.map(Scope::from),
                ordered: *ordered,
            },
        ),
        Command::Doc { check } => doc(cli.json, db, *check),
        Command::Migrate { command } => match command {
            MigrateCommand::Plan {
                with_judge,
                judge_timeout,
                again,
            } => migrate_plan(cli.json, db, with_judge.as_deref(), judge_timeout, *again),
            MigrateCommand::Apply {
                plan,
                keep_sources,
                dry_run,
                again,
            } => migrate_apply(
                cli.json,
                db,
                plan,
                &migration::ApplyOptions {
                    keep_sources: *keep_sources,
                    dry_run: *dry_run,
                    again: *again,
                },
            ),
            MigrateCommand::Status => migrate_status(cli.json, db),
        },
        Command::Forget { key } => forget(cli.json, db, key),
        Command::Import { from } => import_memories(cli.json, db, from.as_deref()),
        Command::Guide { topic } => crate::guide::guide(cli.json, topic.as_deref()),
    }
}

// ---- shared helpers --------------------------------------------------------

/// The actor recorded on each op: `NXM_ACTOR`, else `USER`, else `nxm` — with a set-but-empty
/// variable counting as unset (invariant 1, 6j6v.xsf3; see `nxs_foundation::model::resolve_author`).
fn actor() -> String {
    nxs_foundation::model::resolve_author(
        std::env::var("NXM_ACTOR").ok(),
        std::env::var("USER").ok(),
        "nxm",
    )
}

fn cwd() -> Result<PathBuf> {
    std::env::current_dir().map_err(|e| NxfError::io(format!("reading current dir: {e}")))
}

/// The user's home directory (`$HOME`) — the root under which Claude Code keeps its per-project
/// memory dirs. `None` if unset, in which case auto-detection simply finds nothing.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The `NXM_CLAUDE_MEMORY_DIR` escape hatch: point import/detection at an explicit Claude memory
/// dir (also the deterministic test seam). Empty is treated as unset.
fn claude_memory_override() -> Option<String> {
    std::env::var("NXM_CLAUDE_MEMORY_DIR")
        .ok()
        .filter(|s| !s.is_empty())
}

/// The Claude-host memory source to import from: an explicit `--from`, else auto-detection of this
/// project's Claude memory dir (the env override, else the derived `~/.claude/projects/<slug>`
/// path). `None` when there is nothing to import.
fn resolve_import_source(from: Option<&str>, root: &std::path::Path) -> Option<PathBuf> {
    match from {
        Some(p) => Some(PathBuf::from(p)),
        None => import::detect(
            root,
            home_dir().as_deref(),
            claude_memory_override().as_deref(),
        ),
    }
}

/// The display timestamp stamped on emitted ops (the `updated` value): a pinned `NXM_NOW` (the
/// determinism knob for goldens), else the system clock as RFC3339.
fn resolve_now() -> Result<String> {
    match std::env::var("NXM_NOW").ok().filter(|s| !s.is_empty()) {
        Some(s) => Ok(s),
        None => OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|e| NxfError::io(format!("formatting current time: {e}"))),
    }
}

/// Resolve the workspace and open memory's store over it. The write clock is NOT set here — the
/// facade write paths ([`facade::remember`]/[`facade::forget`]) stamp the caller's explicit `now`,
/// so the seam stays deterministic (the `now`-explicit contract, mirror of flow's facade). The one
/// non-facade write path ([`import_memories`]) sets it itself.
fn open(db: Option<&str>) -> Result<MemoryStore> {
    Ok(open_ws(db)?.1)
}

/// [`open`], keeping the resolved workspace too — for every caller that needs the workspace ROOT
/// and not just the store. That is every WRITE path (each one regenerates the project-memory
/// document, [`project_doc`]) plus [`doc`] itself, which compares the file at that root.
fn open_ws(db: Option<&str>) -> Result<(Workspace, MemoryStore)> {
    let ws = Workspace::resolve(db, &cwd()?)?;
    let store = ws.open_memory_store()?;
    Ok((ws, store))
}

/// Regenerate `<root>/NEXUS_MEMORY.md` after a successful write (6j6v.8q88). Called by every write
/// verb, which is what makes the document a build product rather than something a person has to
/// remember to refresh.
///
/// Silent on success, on purpose: the record the verb prints is its contract (byte-stable under
/// `--json`), and the regenerated file announces itself perfectly well in `git status`. A FAILURE
/// prints a `note:` to stderr and still exits 0 — the memory itself is already committed, so
/// failing the command here would report a write that actually happened as a write that did not.
/// The document is a projection; `nxm doc --check` is where its staleness becomes loud.
fn project_after_write(ws: &Workspace, store: &MemoryStore) {
    let root = ws.dir.parent().unwrap_or(&ws.dir);
    if let Err(e) = project_doc::regenerate(store, root) {
        eprintln!(
            "note: the memory was saved, but {} could not be regenerated ({}); \
             run `nxm doc --check` to see the drift",
            project_doc::FILE_NAME,
            e.msg
        );
    }
}

/// Print one canonical record as the `--json` line (declared field order IS the contract; `to_string`
/// preserves it, the goldens pin the bytes).
fn print_record(rec: &MemoryRecord) {
    println!("{}", serde_json::to_string(rec).expect("record serializes"));
}

// ---- init ------------------------------------------------------------------

/// Initialize memory in the current directory (spec §5.2): ensure a `.nxs/` workspace (join an
/// existing one, else create a fresh memory workspace) via `foundation::setup`, register `memory`
/// as an active module, materialize the `memories` view, then **delegate** the shared agent file +
/// one SessionStart hook per active module to the ONE assembler in `nxs` (P3-S5). Joining a flow
/// workspace re-assembles BOTH modules' blocks under that one hook — no second hook, no self-
/// assembly. The assembler reaches each active module by shelling out to its `agent-manifest`.
fn init(json: bool, quiet: bool, from_beads: bool) -> Result<()> {
    // Single front door (5jz.2): an interactive `nxm init` is routed through the shared `nxs init`
    // frame with memory PRE-SELECTED, so it lands on the same suite chooser (memory ticked, the user
    // can add flow). The driven `--quiet` seam the umbrella calls, and `--json`/piped, stay
    // memory-native and byte-stable — "Nicht-interaktiv unverändert flag-gesteuert". memory takes no
    // `--plugin`. Non-recursive: `nxm init` (TTY) → `nxs init --preselect memory` → `nxm init --quiet`.
    //
    // `--from-beads` (the beads → nxs migration consent, nexus-flow-6ef.1) also routes through the
    // umbrella even non-interactively — it owns the migration (full roster + import/rollback).
    if !quiet && (from_beads || (!json && nxs_init::frontdoor::is_interactive())) {
        return nxs_init::frontdoor::reexec_umbrella_init("memory", json, None, from_beads);
    }
    let root = cwd()?;
    // Delegate the foundation to setup() — idempotent: join an existing workspace, else create one.
    let ws = match workspace::discover(&root) {
        Ok(existing) => existing,
        Err(_) => workspace::setup(&root, &workspace::memory_config())?,
    };
    // Register memory as an active module. setup loads an existing config UNCHANGED, so when memory
    // joins a flow workspace this is what records it in `active_modules` (idempotent).
    workspace::activate_module(&ws.dir, workspace::MEMORY_MODULE)?;
    // Open memory's store to add the `memories` view (setup created only the substrate db).
    ws.open_memory_store()?;
    // Re-resolve so the config reflects the just-activated memory module (and any module that was
    // already active, e.g. flow), then delegate to the assembler — it re-assembles every active
    // module's block, and wires one SessionStart hook per active module.
    let ws = workspace::discover(&root)?;
    let report = assemble_agent_files(&root, &ws)?;
    let modules = ws.config.active_modules.clone();
    // Cross-sell the not-yet-active suite modules (aye.31): `Some` when an addable module (e.g.
    // flow) is still inactive; `None` once flow + memory are both active. Sourced from `nxs-init`.
    let ad = nxs_init::advertise::advertisement(&modules);

    // On-ramp (aye.24): if this project already has a Claude-host memory store, surface — don't
    // perform — an import suggestion, and delegate the work to `nxm import`. Only when it carries
    // actual facts, so a project without one stays noise-free.
    let import_source = import::detect(
        &root,
        home_dir().as_deref(),
        claude_memory_override().as_deref(),
    );
    let import_count = import_source
        .as_deref()
        .map(import::scan)
        .map_or(0, |f| f.len());
    let import_suggestion = import_source.filter(|_| import_count > 0);

    // **The workspace comes to the background service at its birth on THIS path too** (nxf
    // 6j6v.y12q). `nxs init` has done it since 6j6v.npf9, and this is the path that never reaches
    // it: under `--json` or `--quiet` the re-exec above does not happen, which is exactly how an
    // agent enters. Since 6j6v.8see the service's list is where a workspace's deadlines come from.
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

    if json {
        let mut out = serde_json::json!({
            "ok": true,
            "workspace": ws.dir.display().to_string(),
            "site_id": ws.replica.site_id,
            "prefix": ws.replica.prefix,
            "modules": modules,
            "onboarding": {
                "agents": report.agents.as_str(),
                "claude": report.claude.as_str(),
            },
            // **Superseded 2026-08-28 (nxf n2m6 + a2a1):** `command` was ONE string while the
            // assembler wired one hook; it is now one entry per active module, so the field is a
            // LIST under the name that says so. A consumer reading the old key gets nothing rather
            // than the first of three silently mistaken for the whole wiring.
            "hook": {
                "hook_added": report.hook_added,
                "commands": report.hook_commands,
            },
            // nxf 6j6v.y12q. The same key `nxs init --json` carries, meaning the same thing:
            // whether this workspace is on the list the background service attends. No `answer`
            // beside it — this verb never asks the install question, so it reports nothing about
            // one.
            "service": { "registered": service.registered },
        });
        // Additive `advertisement` field (aye.31): present only when an addable module is inactive.
        if let Some(ad) = &ad {
            out["advertisement"] = serde_json::Value::String(ad.agent_copy.clone());
        }
        // Additive `import_suggestion` field (aye.24): present only when a Claude-host source with
        // importable facts exists.
        if let Some(src) = &import_suggestion {
            out["import_suggestion"] = serde_json::json!({
                "source": src.display().to_string(),
                "count": import_count,
            });
        }
        println!("{out}");
    } else if !quiet {
        println!("initialized memory at {}", ws.dir.display());
        println!("  prefix:  {}", ws.replica.prefix);
        println!("  modules: {}", modules.join(", "));
        println!("  agent files: {}", describe_onboarding(&report));
        println!(
            "  hook:    {}",
            // Rendered by the assembler, not spelled out here: the hook command grew a
            // `|| cat NEXUS_MEMORY.md` fallback in 6j6v.8q88, and became a LIST — one entry per
            // active module — in nxf n2m6 + a2a1. Hand-written copies across the module CLIs would
            // be that many places to go stale the next time it moves.
            nxs_init::assembler::describe_hooks(report.hook_added, &report.hook_commands)
        );
        println!("  service: {}", service.summary_line());
        // The manufakt-Forge upsell CTA (TTY-gated → ember on a terminal, plain otherwise).
        if let Some(ad) = &ad {
            let theme = nxs_ui::Theme::detect(false);
            println!("\n{}", theme.accent(&ad.human_cta));
        }
        // On-ramp suggestion (aye.24): propose migrating an existing Claude-host memory store.
        if import_suggestion.is_some() {
            println!(
                "\nFound {import_count} existing Claude memories — run `nxm import` to bring them \
                 into nexus-memory."
            );
        }
    }
    // `--quiet` without `--json`: silent success — the umbrella renders.
    Ok(())
}

/// memory's self-registered in-process init entry (5jz.3). The umbrella calls this DIRECTLY (no
/// longer a `<bin> init --quiet` subprocess), so memory sets itself up inside the one umbrella
/// process. memory has no interactive sub-config of its own today (its setup is a no-op config), so
/// it always runs silently here: `json = false, quiet = true` keeps it from EVER printing its own
/// record — the umbrella owns all visible output, so nothing bleeds through regardless of the
/// umbrella's own `--json`. memory takes no `--plugin` (accepts_plugin: false).
pub fn memory_init_entry(_req: &nxs_init::InitRequest) -> Result<()> {
    init(false, true, false)
}

// Self-registration (5jz.1): memory declares its roster identity + chooser copy + in-process init
// entry; the umbrella collects this via `inventory`, so memory appears without any `nxs` change.
inventory::submit! {
    nxs_init::ModuleInit {
        key: "memory",
        binary: "nxm",
        now_env: "NXM_NOW",
        blurb: "durable agent memory — facts the agent recalls across sessions",
        recommended: false,
        // **LAST in the fan-out, and that is a safety property** (6j6v.xbnh). The host truncates a
        // large SessionStart output — measured: 25.893 bytes pass through, 32.000 are filed away
        // behind a 2 KB preview — so the ORDER decides what survives a truncation that happens
        // anyway. Memory is the one block a session can fetch afterwards (`nxm recall <key>`,
        // `nxm memories <text>`); the board and the command vocabulary are written down nowhere
        // else. So memory moved from 20 to 30, behind chat, to make the worst case benign instead
        // of catastrophic.
        order: 30,
        accepts_plugin: false,
        init_fn: memory_init_entry,
        // 5jz.6: memory's own welcome pitch, shown in the nxs frame when memory is preselected
        // (`nxm init`). No first command — memory's value is recall the agent does on its own, not a
        // command the user runs first.
        welcome: Some(
            "memory gives the agent durable facts it recalls across sessions — so you stop \
             re-explaining your project.",
        ),
        // huy: memory's "learn more" decision-help, shown in the picker's details panel.
        details: Some(
            "Durable, structured facts the agent stores and recalls across sessions — decisions, \
             conventions, and context that would otherwise be re-explained each time. Loaded \
             automatically at session start; offline-first, so recall needs no network.",
        ),
        first_command: None,
    }
}

/// Assemble the shared agent file + one SessionStart hook per active module from the workspace's
/// active modules' manifests (P3-S5; a single `nxs prime` hook until nxf n2m6 + a2a1). nxm delegates to the ONE assembler in `nxs`, which shells out to
/// each active module's `agent-manifest` (TB-5) — so a flow+memory workspace gets both blocks.
fn assemble_agent_files(
    root: &std::path::Path,
    ws: &Workspace,
) -> Result<nxs_init::assembler::AssembleReport> {
    // A lone module assembles only the agent-file blocks it OWNS; an active sibling it does not link
    // (e.g. flow, when `nxm init` joins a flow workspace) has already assembled its own block via its
    // own init — so resolve the modules this binary knows and skip the rest, rather than erroring.
    let roster = nxs_init::roster();
    let modules = nxs_init::resolve_known(&roster, &ws.config);
    let manifests = nxs_init::assembler::collect_manifests(&modules)?;
    nxs_init::assembler::assemble(root, &manifests)
}

/// One-line human summary of what the assembler wrote to AGENTS.md and, since nxf 6j6v.q6e3,
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

// ---- contract verbs: agent-manifest + prime --------------------------------

/// `nxm agent-manifest`: emit nxm's declared contribution to the shared agent file (spec §6.2) as
/// data. Static (no workspace, no store), so it runs anywhere. `--json` is the contract the `nxs`
/// umbrella consumes; the typed struct's declaration order IS the field-order contract.
fn agent_manifest(json: bool) -> Result<()> {
    let manifest = onboarding::manifest();
    if json {
        let value = serde_json::to_string(&manifest)
            .map_err(|e| NxfError::io(format!("serializing manifest: {e}")))?;
        println!("{value}");
    } else {
        println!("nexus-memory agent manifest");
        println!("  prime command:  {}", manifest.prime_command);
        println!(
            "  hook:           {} → {}",
            manifest.hook.event, manifest.hook.command
        );
        println!("\nRun with --json for the machine contract the `nxs` umbrella assembles from.");
    }
    Ok(())
}

/// `nxm prime`: the session bootstrap (spec §5.2) — the bd-prime equivalent. It (1) states the
/// memory rule (durable knowledge ONLY via `nxm remember`, never a MEMORY.md), (2) lists the key
/// commands, and (3) replays every active memory deterministically. Wired as memory's OWN
/// SessionStart hook (nxf n2m6 + a2a1; it used to be reached through the single `nxs prime` hook's
/// fan-out), so it loads automatically each session; hosts render the human form (Markdown) as
/// context.
///
/// The record is computed by [`facade::prime`] and renders ITSELF (nxf 6j6v.wph0) — this command
/// only picks the view. The assembly used to live here as hard-wired `println!` prose, which is
/// precisely why an embedding host could not obtain the block from the library and rebuilt it in
/// TypeScript instead.
fn prime(json: bool, db: Option<&str>) -> Result<()> {
    let report = facade::prime(&open(db)?)?;
    if json {
        println!("{}", report.to_value());
    } else {
        println!("{}", report.render_markdown());
    }
    Ok(())
}

/// `nxm index`: the whole memory index (nxf 6j6v.5jm3) — every memory the session start replays,
/// one written line each, with no byte budget over it.
///
/// It exists because `nxm prime`'s index HAS one: the host delivers nothing at all above 10.240 B
/// per hook output, so a workspace that remembers enough would otherwise lose the module rather
/// than lose lines. The block carries what fits and names this verb; this verb carries the rest.
/// The intended sequence is index first, then one body — `nxm recall <key>` on the entry the lines
/// made worth opening.
fn index(json: bool, db: Option<&str>) -> Result<()> {
    let report = facade::index(&open(db)?)?;
    if json {
        println!("{}", report.to_value());
    } else {
        println!("{}", report.render_markdown());
    }
    Ok(())
}

// ---- memory verbs ----------------------------------------------------------

/// `nxm remember "<text>" --introduction "<line>" [--key <key>] [--category …] [--scope …]
/// [--refs …]`: upsert the fact, optionally filing it in the same step. The resolved key is the
/// explicit `--key` verbatim, else the content-hash auto-key (spec §2.2).
fn remember(
    json: bool,
    db: Option<&str>,
    text: &str,
    key: Option<&str>,
    class: &Classification,
) -> Result<()> {
    let (ws, mut store) = open_ws(db)?;
    let rec = facade::remember(&mut store, &resolve_now()?, &actor(), key, text, class)?;
    project_after_write(&ws, &store);
    if json {
        print_record(&rec);
    } else {
        println!("remembered {}", rec.key);
    }
    Ok(())
}

/// `nxm classify <key> [--category …] [--scope …] [--refs …] [--introduction …]`: file an existing
/// memory without touching its text. At least one flag is required — silently doing nothing would
/// be worse than saying so.
fn classify(json: bool, db: Option<&str>, key: &str, class: &Classification) -> Result<()> {
    let (ws, mut store) = open_ws(db)?;
    let rec = facade::classify(&mut store, &resolve_now()?, &actor(), key, class)?;
    project_after_write(&ws, &store);
    if json {
        print_record(&rec);
    } else {
        println!("classified {}", rec.key);
        println!("  category: {}", rec.category);
        println!("  scope:    {}", rec.scope);
        println!("  refs:     {}", render_refs(&rec.refs));
        println!(
            "  intro:    {}",
            rec.introduction.as_deref().unwrap_or("none")
        );
    }
    Ok(())
}

/// `nxm reorder <key>…`: store the given sequence as the reading order (positions 1, 2, …).
fn reorder(json: bool, db: Option<&str>, keys: &[String]) -> Result<()> {
    let (ws, mut store) = open_ws(db)?;
    let records = facade::reorder(&mut store, &resolve_now()?, &actor(), keys)?;
    project_after_write(&ws, &store);
    if json {
        println!(
            "{}",
            serde_json::to_string(&records).expect("records serialize")
        );
    } else {
        println!("reordered {} memories", records.len());
        for rec in &records {
            println!("  {}  {}", rec.ordinal.unwrap_or_default(), rec.key);
        }
    }
    Ok(())
}

/// The human rendering of a memory's references — the ids, or an explicit "none" so an empty set
/// reads as an answer rather than a truncated line.
fn render_refs(refs: &[String]) -> String {
    if refs.is_empty() {
        "none".to_string()
    } else {
        refs.join(", ")
    }
}

/// `nxm recall <key>`: the full text of one active memory, or `not_found`.
fn recall(json: bool, db: Option<&str>, key: &str) -> Result<()> {
    let store = open(db)?;
    let rec = facade::recall(&store, key)?;
    if json {
        print_record(&rec);
    } else {
        println!("{}", rec.body.as_deref().unwrap_or(""));
    }
    Ok(())
}

/// `nxm memories [<search>] [--category …] [--scope …] [--ordered]`: list/search active memories,
/// deterministic — key-sorted unless `--ordered` asks for the stored reading order.
fn memories(json: bool, db: Option<&str>, query: &MemoryQuery) -> Result<()> {
    let store = open(db)?;
    let records = facade::memories(&store, query)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&records).expect("records serialize")
        );
    } else if records.is_empty() {
        println!("no memories");
    } else {
        for m in &records {
            println!("{}  {}", m.key, m.body.as_deref().unwrap_or(""));
        }
    }
    Ok(())
}

/// `nxm doc [--check]`: the project-memory document (6j6v.8q88).
///
/// Bare, it PRINTS the projection — byte for byte what `NEXUS_MEMORY.md` holds, so a caller can
/// diff, pipe or inspect it without trusting the file. It deliberately does not write: the document
/// is regenerated by writes and by a sync pass that pulled something, never by somebody
/// remembering to run a command.
///
/// `--check` is the guard rule (a) asks for. A file that no longer matches the store was either
/// edited by hand — source and projection forked — or never regenerated here, which is what a sync
/// daemon that was not running in this checkout looks like. Both are reported as a non-zero exit
/// naming the file, because a guard nobody's tooling can see go red is not a guard.
fn doc(json: bool, db: Option<&str>, check: bool) -> Result<()> {
    let (ws, store) = open_ws(db)?;
    let root = ws.dir.parent().unwrap_or(&ws.dir);
    let path = root.join(project_doc::FILE_NAME).display().to_string();

    if !check {
        let rendered = project_doc::render(&store)?;
        if json {
            println!(
                "{}",
                serde_json::json!({ "path": path, "content": rendered })
            );
        } else {
            // `print!`, not `println!`: the rendering already ends in the newline the file ends
            // with, so stdout is byte-identical to the document itself.
            print!("{rendered}");
        }
        return Ok(());
    }

    let drift = project_doc::check(&store, root)?;
    match drift {
        project_doc::Drift::Drifted => Err(NxfError::validation(format!(
            "{path} no longer matches the memory store — it was edited by hand, or a write that \
             should have regenerated it never ran here. The file is a PROJECTION: change it with \
             `nxm remember` / `nxm forget` (the next write rewrites it), and run `nxm doc` to see \
             what it should hold."
        ))),
        project_doc::Drift::Missing => Err(NxfError::validation(format!(
            "{path} is missing, but this workspace has memories to project into it. The next `nxm` \
             write regenerates it; `nxm doc` prints what it should hold."
        ))),
        project_doc::Drift::InSync | project_doc::Drift::NotApplicable => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "path": path, "status": drift.as_str() })
                );
            } else {
                println!("{path} is in sync with the memory store");
            }
            Ok(())
        }
    }
}

// ---- the judging migration (6j6v.9yaj) -------------------------------------

/// `nxm migrate plan [--with-judge [<assistant>]] [--judge-timeout <duration>] [--again]`: propose
/// the migration. Reads only.
///
/// The mark guard is the "once per stream, not once per machine" rule made visible: the sync daemon
/// carries everything written here, so a run on a second device would produce a SECOND independent
/// filing of the same memories, and last-writer-wins would blend the two into something neither
/// judged. So a marked stream reports the earlier run and stops — asking, in the only way a
/// non-interactive command can — and `--again` is the explicit answer.
fn migrate_plan(
    json: bool,
    db: Option<&str>,
    with_judge: Option<&str>,
    judge_timeout: &str,
    again: bool,
) -> Result<()> {
    // Resolve the judge FIRST: a name this build does not know is a typo, and reading the whole
    // workspace before saying so only delays the answer.
    let judge = with_judge
        .map(|name| {
            migration::Judge::parse(name).ok_or_else(|| {
                NxfError::validation(format!(
                    "unknown judge '{name}': this build knows {}",
                    migration::Judge::ALL
                        .iter()
                        .map(|j| j.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })
        })
        .transpose()?;
    // …and the deadline with it, for the same reason: a malformed duration must not be discovered
    // after a plan has been built and a judge has already been asked.
    let options = migration::JudgeOptions {
        timeout: migration::parse_judge_timeout(judge_timeout)?,
    };

    let (ws, store) = open_ws(db)?;
    let root = ws.dir.parent().unwrap_or(&ws.dir);

    // A courtesy stop, not the guard: planning writes nothing, so the rule that actually decides
    // lives on `apply` at the seam (`ApplyOptions::again`), where the CLI, an embedding app and a
    // re-applied old plan file all meet it. This one just saves the trouble of building — and
    // judging — a plan somebody was not going to be allowed to apply.
    if !again {
        if let Some(mark) = migration::read_mark(&store)? {
            return Err(NxfError::validation(migration::already_migrated(&mark)));
        }
    }

    let plan = facade::migrate_plan(&store, root)?;
    let plan = match judge {
        None => plan,
        Some(judge) => {
            // The deadline is announced BEFORE the wait, not only when it expires: the whole point
            // of a generous limit is that the user knows what they are waiting for while a judge
            // thinks for minutes. On stderr, so `--json` still pipes cleanly.
            eprintln!(
                "note: asking the judge `{judge}` — {}",
                match options.timeout {
                    Some(limit) => format!(
                        "waiting up to {} (`--judge-timeout` changes it; 0 waits as long as it \
                         takes)",
                        migration::spell_timeout(limit)
                    ),
                    None => "waiting as long as it takes (`--judge-timeout <n>[smh]` bounds it)"
                        .to_string(),
                }
            );
            migration::ask_judge(judge, &plan, &options)?
        }
    };

    if json {
        // Pretty-printed, unlike every other `--json` record here, and deliberately: this document
        // exists to be saved, read, edited and diffed by whoever decides it. One long line would
        // make the review step — the whole reason the migration is two verbs — impractical.
        println!(
            "{}",
            serde_json::to_string_pretty(&plan)
                .map_err(|e| NxfError::io(format!("serializing the migration plan: {e}")))?
        );
    } else {
        render_plan(&plan, judge.is_some());
    }
    Ok(())
}

/// The human rendering of a plan: what it would touch, and the one command that turns it into a
/// document somebody can decide.
fn render_plan(plan: &migration::MigrationPlan, judged: bool) {
    let memories = plan
        .entries
        .iter()
        .filter(|e| e.kind == migration::EntryKind::Memory)
        .count();
    let sections = plan.entries.len() - memories;
    println!(
        "migration plan: {memories} unfiled {}, {sections} document {}",
        if memories == 1 { "memory" } else { "memories" },
        if sections == 1 { "section" } else { "sections" }
    );
    for entry in &plan.entries {
        let source = entry
            .source
            .as_ref()
            .map(|s| format!("  ({} · {})", s.path, s.heading))
            .unwrap_or_default();
        println!(
            "  {:<10} {:<24} {} / {}{source}",
            format!("{:?}", entry.action).to_lowercase(),
            entry.key,
            entry.category,
            entry.scope
        );
    }
    for skipped in &plan.skipped_sources {
        println!("  skipped    {} ({})", skipped.path, skipped.reason);
    }
    if judged {
        println!("\nRe-run with --json to save the plan, review it, then `nxm migrate apply --plan <file>`.");
    } else {
        println!(
            "\nNothing is judged yet. Run `nxm migrate plan --with-judge --json > migration.json` \
             to have a coding assistant propose the filing, review it, then \
             `nxm migrate apply --plan migration.json`."
        );
    }
}

/// `nxm migrate apply --plan <file|-> [--keep-sources] [--dry-run]`: execute a decided plan.
fn migrate_apply(
    json: bool,
    db: Option<&str>,
    plan_path: &str,
    options: &migration::ApplyOptions,
) -> Result<()> {
    let raw = if plan_path == "-" {
        use std::io::Read as _;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| NxfError::io(format!("reading the plan from stdin: {e}")))?;
        buf
    } else {
        std::fs::read_to_string(plan_path)
            .map_err(|e| NxfError::io(format!("reading {plan_path}: {e}")))?
    };
    let plan: migration::MigrationPlan = serde_json::from_str(&raw)
        .map_err(|e| NxfError::validation(format!("{plan_path} is not a migration plan: {e}")))?;

    let (ws, mut store) = open_ws(db)?;
    let root = ws.dir.parent().unwrap_or(&ws.dir).to_path_buf();
    let report =
        facade::migrate_apply(&mut store, &resolve_now()?, &actor(), &root, &plan, options)?;
    if !report.dry_run {
        project_after_write(&ws, &store);
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(&report).expect("the report serializes")
        );
    } else {
        let verb = if report.dry_run {
            "would file"
        } else {
            "filed"
        };
        println!(
            "{verb} {} memories, remembered {} sections, skipped {}",
            report.classified.len(),
            report.remembered.len(),
            report.skipped.len()
        );
        if !report.pruned.is_empty() {
            println!(
                "{} {}",
                if report.dry_run {
                    "would move the migrated sections out of"
                } else {
                    "moved the migrated sections out of"
                },
                report.pruned.join(", ")
            );
        }
    }
    Ok(())
}

/// `nxm migrate status`: what is still unfiled here, and whether a run already happened.
fn migrate_status(json: bool, db: Option<&str>) -> Result<()> {
    let status = facade::migrate_status(&open(db)?)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&status).expect("the status serializes")
        );
    } else if status.is_open() {
        println!(
            "{} unfiled {} — run `nxm migrate plan --with-judge`",
            status.unsorted,
            if status.unsorted == 1 {
                "memory"
            } else {
                "memories"
            }
        );
    } else {
        println!("nothing left to file");
    }
    if !json {
        match &status.mark {
            Some(mark) if !mark.at.is_empty() => {
                println!("migrated by {} on {}", mark.actor, mark.at)
            }
            Some(_) => println!("migrated on another device"),
            None => {}
        }
    }
    Ok(())
}

/// `nxm forget <key>`: tombstone an active memory (reversible). A missing/forgotten key is a
/// friendly `not_found` — the store-level forget is pure, the existence check is the facade's.
fn forget(json: bool, db: Option<&str>, key: &str) -> Result<()> {
    let (ws, mut store) = open_ws(db)?;
    // The facade returns the tombstone record (the value the engine/MCP seams render); the CLI
    // receipt just echoes the key the caller asked to forget. Rendering from the input `key` keeps
    // `--json` byte-stable WITHOUT resting on the store echoing the queried key back through the
    // record (CQ review #1) — the byte-equality is now self-evident.
    facade::forget(&mut store, &resolve_now()?, &actor(), key)?;
    project_after_write(&ws, &store);
    if json {
        println!("{}", serde_json::json!({ "ok": true, "key": key }));
    } else {
        println!("forgot {key}");
    }
    Ok(())
}

// ---- import ----------------------------------------------------------------

/// `nxm import [--from <dir>]`: migrate an existing Claude-host memory store into nexus-memory
/// (aye.24). Non-destructive (read-only on the source) and idempotent (each fact upserts under its
/// frontmatter `name:`, so a re-run never duplicates). With no source, it is a quiet no-op success
/// — running it is itself the opt-in, so it does the work without an interactive prompt; `--json`
/// returns a deterministic, countable result.
fn import_memories(json: bool, db: Option<&str>, from: Option<&str>) -> Result<()> {
    let source = resolve_import_source(from, &cwd()?);
    let facts = source.as_deref().map(import::scan).unwrap_or_default();

    // The bulk import is the one write path that does NOT route through the facade (it upserts many
    // facts via `remember`'s core), so it stamps the write clock itself — `open` no longer does.
    let (ws, mut store) = open_ws(db)?;
    store.set_wall_clock(&resolve_now()?);
    let keys = upsert_facts(&mut store, &actor(), &facts);
    project_after_write(&ws, &store);
    let source_str = source.as_ref().map(|p| p.display().to_string());

    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "imported": facts.len(),
                "source": source_str,
                "keys": keys,
            })
        );
    } else {
        match &source_str {
            None => println!("no Claude memory source found — nothing to import"),
            Some(src) => println!("imported {} memories from {src}", facts.len()),
        }
    }
    Ok(())
}

/// Upsert imported facts — the ONE import write path, shared by the Claude-host import and the
/// beads import (6ef.4), so there is no parallel write logic between the two SOURCES.
///
/// **It writes through the store, not through [`facade::remember`], and therefore imports a memory
/// with NO introduction** (nxf 6j6v.xbnh, review of PR #381, Integrity #1). That is a deliberate
/// exception to a rule the rest of the codebase treats as absolute, so it is named here rather
/// than left to be discovered:
///
/// * An imported fact is somebody else's text — a Claude-host memory file, a beads export. There
///   is no introduction to carry over, and **deriving one is precisely the mechanism 6j6v.xbnh
///   removed**: the old index took a body's first line and produced empty entries for exactly the
///   memories an agent had written. An import inventing a line would put that back, at scale, in
///   the one place nobody is watching.
/// * So the import lands in the same state as every memory written before the register existed:
///   `introduction` is NULL, the session start renders the named placeholder and COUNTS it
///   ([`facade::PrimeReport::introduction_gap`]), and `nxm classify <key> --introduction …` closes
///   it. Visible and countable, never a blank line.
/// * The imports also land as [`model::CATEGORY_UNSORTED`], so the existing migration workflow
///   already treats them as material somebody has yet to judge.
///
/// What this is NOT is a way around the mandatory field for ordinary writing: every `remember`
/// reachable from the CLI, the embedding engine and MCP goes through [`facade::remember`], which
/// refuses a write with no introduction. Pinned by `an_imported_memory_reads_as_a_named_gap`.
fn upsert_facts(
    store: &mut MemoryStore,
    author: &str,
    facts: &[import::ImportedFact],
) -> Vec<String> {
    for fact in facts {
        store.remember(&fact.key, &fact.body, author);
    }
    facts.iter().map(|f| f.key.clone()).collect()
}

/// Import beads memories into nxm (nexus-flow-6ef.4), reusing the [`upsert_facts`] core — beads is
/// a SECOND source for the same path, not a second write path. Like every import, the memories
/// arrive without an introduction; see [`upsert_facts`] for why that is deliberate. Opens the memory store at `root`,
/// stamps `now` (the beads export carries no per-memory timestamp), and upserts each memory under
/// its beads key verbatim (so a re-import upserts in place, idempotent). Returns the imported keys.
/// The seam the `nxs` orchestrator calls, keeping it free of memory's workspace/store internals.
pub fn import_beads_memories_at(
    root: &std::path::Path,
    now: &str,
    memories: &[nxs_init::beads::BeadsMemory],
) -> Result<Vec<String>> {
    let ws = Workspace::resolve(None, root)?;
    let mut store = ws.open_memory_store()?;
    store.set_wall_clock(now);
    let keys = upsert_facts(&mut store, &actor(), &import::beads_facts(memories));
    project_after_write(&ws, &store);
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The `render_memory_blocks` unit tests moved with the function into `facade.rs` (nxf
    // 6j6v.wph0) — the CLI no longer formats anything itself.

    #[test]
    fn import_beads_memories_at_preserves_keys_and_is_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        workspace::setup(tmp.path(), &workspace::memory_config()).unwrap();
        let mems = vec![
            nxs_init::beads::BeadsMemory {
                key: "changelog-scope".into(),
                value: "fragments feed release-notes".into(),
            },
            nxs_init::beads::BeadsMemory {
                key: "dolt-phantoms".into(),
                value: "phantom DBs hide in three places".into(),
            },
        ];
        let keys = import_beads_memories_at(tmp.path(), "2026-06-26T00:00:00Z", &mems).unwrap();
        assert_eq!(keys.len(), 2, "both memories imported");

        // Persisted under their beads keys verbatim.
        let store = Workspace::resolve(None, tmp.path())
            .unwrap()
            .open_memory_store()
            .unwrap();
        assert_eq!(
            store
                .recall("changelog-scope")
                .unwrap()
                .unwrap()
                .body
                .as_deref(),
            Some("fragments feed release-notes")
        );
        assert_eq!(
            store.recall("changelog-scope").unwrap().unwrap().updated,
            "2026-06-26T00:00:00Z",
            "stamped with the migration clock (beads export carries no timestamp)"
        );

        // Re-import upserts in place — no duplicate.
        import_beads_memories_at(tmp.path(), "2026-06-26T00:00:00Z", &mems).unwrap();
        let store = Workspace::resolve(None, tmp.path())
            .unwrap()
            .open_memory_store()
            .unwrap();
        assert_eq!(
            store.memories(&MemoryQuery::default()).unwrap().len(),
            2,
            "re-import is idempotent"
        );
    }
}
