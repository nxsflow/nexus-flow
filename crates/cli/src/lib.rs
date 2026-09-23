//! nxf — the nexus-flow agent CLI.
//!
//! A thin, deterministic interface over `nexus-flow-core`. Commands are added
//! incrementally (see the E2 epic). Every command resolves a `.nxs/`
//! workspace (or creates one via `init`) and reports failures through the shared
//! structured error envelope.
//!
//! This crate is built as both a library and the `nxf` binary. The binary
//! (`src/main.rs`) is a thin shim over [`run`]; the library form exists so repo
//! tooling (`cargo xtask docs`) can reach the clap [`command`] tree — the single
//! source of truth for the generated command reference (man-pages, Markdown, JSON).

pub mod beads_import;
mod commands;
mod error;
// `pub` so the E9 MCP server can attach the SAME retrieval-rule join its `flow_show`/`flow_next`
// tools are pinned byte-identical to (6j6v.srpg) — one definition, two seams.
pub mod memories;
mod onboarding;
mod output;
mod plugin;
mod selfupdate;
mod signature;
mod sync;
mod validate;
mod workspace;

/// The active flow plugin's example `--type` at a workspace root (nexus-flow-92zt), for the `nxs`
/// umbrella's post-init "your first move" banner: the umbrella substitutes it into flow's `{type}`
/// first-move template so the copy-pasted command is valid for the chosen plugin.
pub use commands::flow_example_type;

use clap::{CommandFactory, Parser, Subcommand};
use error::Result;
use std::process::ExitCode;

/// nexus-flow agent CLI.
#[derive(Parser, Debug)]
#[command(name = "nxf", version, about = "nexus-flow agent CLI", long_about = None)]
struct Cli {
    /// Emit machine-readable JSON instead of human output. `priority` and `type` carry canonical
    /// keys (stable ordinals / declared type keys), not display labels; the labels are available
    /// via `nxf schema` and, on `show`/`list`/`next`, the additive `priority_label`/`type_label`
    /// fields. `create`/`update` accept either the key or the label.
    #[arg(long, global = true)]
    json: bool,

    /// Use this sqlite db file directly instead of discovering `.nxs/`.
    #[arg(long, global = true, env = "NXF_DB")]
    db: Option<String>,

    #[command(subcommand)]
    command: Command,
}

// `Create` carries the full create surface as flat flags (its many long-text fields each gained
// a `--<field>-file` companion in 95d.2), which clippy flags as a large variant. This enum is
// parsed exactly once per process and never stored in bulk, so the size gap is irrelevant —
// keeping the flags flat (rather than boxing into a sub-struct) keeps the parser the readable
// single source of truth the generated docs render from.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand, Debug)]
enum Command {
    /// Initialize a `.nxs` workspace in the current directory: set up flow, then delegate the
    /// shared agent file + one SessionStart hook per active module to the `nxs` assembler. To set
    /// up the whole suite (and pick tools interactively), use `nxs init` instead.
    Init {
        /// Plugin to activate (vocabulary + ranking): `issue-tracker` | `personal-todo`.
        /// Required when non-interactive (`--json`/piped/`--quiet`); on a TTY, omitting it prompts.
        #[arg(long)]
        plugin: Option<String>,
        /// Driven/quiet mode: set up the workspace and delegate the agent files + the
        /// SessionStart hooks, but render no banner or prompt. The seam the `nxs` umbrella uses to
        /// drive `nxf init` without its output bleeding through; implies a non-interactive
        /// plugin choice (`--plugin`).
        #[arg(long)]
        quiet: bool,
        /// Consent to the beads → nxs migration: in a project that still uses
        /// beads, import its tickets + memories and roll back beads' config. Routed through the
        /// `nxs` umbrella, which owns the migration.
        #[arg(long = "from-beads")]
        from_beads: bool,
    },
    /// Session bootstrap: workflow rules, a ready/blocked snapshot, command reference. Hidden
    /// from `--help` (nexus-flow-fyr): the umbrella `nxs prime` is the single user-facing entry,
    /// and it fans out to this verb as a subprocess — `nxf prime` stays a working internal seam,
    /// just no longer advertised.
    #[command(hide = true)]
    Prime,
    /// Emit nxf's declared contribution to the shared agent file (the AGENTS.md section,
    /// the prime command, the SessionStart hook) as data. The `nxs` umbrella assembles the shared
    /// files from each active module's manifest; `--json` is the machine contract.
    AgentManifest,
    /// Create a new item — a complete item in one call. Title, description, and priority are
    /// required; design, the definition of done, a parent, and dependencies are optional. Run
    /// `nxf schema` for the active plugin's field model (labels, constraints, required fields).
    Create {
        /// Item type, from the active plugin's own vocabulary (under `issue-tracker`: `epic`,
        /// `feature`, `bug`, `chore`, `decision`). The set is plugin-declared, so run `nxf schema`
        /// for the one this workspace actually accepts.
        #[arg(long = "type", required_unless_present = "json_stdin")]
        ty: Option<String>,
        /// Item title (required).
        #[arg(long, required_unless_present = "json_stdin")]
        title: Option<String>,
        /// Description — why this item exists and its goal (required). Pass `-` to read it from
        /// STDIN (escaping-free, multi-line; e.g. `cat desc.md | nxf create … --description -`),
        /// or supply it via `--description-file` instead.
        #[arg(long, required_unless_present_any = ["description_file", "json_stdin"])]
        description: Option<String>,
        /// Read the description from this file (UTF-8, verbatim); `-` reads STDIN. Mutually
        /// exclusive with `--description`.
        #[arg(long = "description-file")]
        description_file: Option<String>,
        /// Priority — the plugin's named variant (e.g. P0..P4) OR its canonical ordinal key
        /// (0..4, the form `--json` emits), required; run `nxs prime`/`nxf schema` to see the set.
        /// Validated against it; ranks `next` by the variant order.
        #[arg(long, required_unless_present = "json_stdin")]
        priority: Option<String>,
        /// Design — the path to the goal; can be filled in later (optional). `-` reads it from STDIN.
        #[arg(long)]
        design: Option<String>,
        /// Read the design from this file (UTF-8, verbatim); `-` reads STDIN.
        #[arg(long = "design-file")]
        design_file: Option<String>,
        /// Definition of done — what "done" means for this item (optional). `-` reads it from STDIN.
        #[arg(long = "dod")]
        dod: Option<String>,
        /// Read the definition of done from this file (UTF-8, verbatim); `-` reads STDIN.
        #[arg(long = "dod-file")]
        dod_file: Option<String>,
        /// Due date (ISO-8601 / RFC3339).
        #[arg(long)]
        due: Option<String>,
        /// Defer-until date (ISO-8601 / RFC3339).
        #[arg(long)]
        defer: Option<String>,
        /// Parent item id (sets belongs-to); must exist.
        #[arg(long)]
        parent: Option<String>,
        /// This item depends on <id> (repeatable): <id> must exist and blocks this item until it
        /// closes. Same direction as `dep add <this> <id>`.
        #[arg(long = "depends-on")]
        depends_on: Vec<String>,
        /// Set a plugin CUSTOM field: `name=value` (repeatable). Canonical fields use their own
        /// flags above; `--set` carries the active plugin's declared custom fields (run `nxf schema`
        /// for the set). Validated against the field's type; a value of `-` reads STDIN.
        #[arg(long = "set")]
        set: Vec<String>,
        /// Set a custom field from a file: `name=path` (UTF-8, verbatim); `path` of `-` reads STDIN.
        /// Repeatable. A field may be set by `--set` or `--set-file`, not both.
        #[arg(long = "set-file")]
        set_file: Vec<String>,
        /// Print ONLY the new item's id — one line, no JSON, no output framing — so it can be
        /// captured straight into the next command (`id=$(nxf create … -q)`) without parsing JSON.
        /// Takes precedence over `--json` (the id wins; `--json` is ignored, not an error).
        #[arg(short = 'q', long = "id-only")]
        id_only: bool,
        /// `-` reads the whole item as one JSON object from STDIN (every field in one piped
        /// payload — no escaping, no flag combinatorics; pair with `--json` for JSON output too,
        /// the canonical form `nxf create --json -`). The only accepted value is `-`, and it is
        /// mutually exclusive with the field flags.
        #[arg(value_name = "-")]
        json_stdin: Option<String>,
    },
    /// Update item fields via `--set field=value` (repeatable). The long-form
    /// `description`/`design`/`completion_criterion` are ordinary `--set` fields. Run `nxf schema`
    /// for the settable fields and their plugin names. Convention: correct the field model once
    /// shortly after creation; once it is stable, record what you learn as append-only notes
    /// (`nxf note add`) rather than further field edits.
    Update {
        /// Item id.
        id: String,
        /// `-` reads the fields to set as one JSON object from STDIN (`{"description":"…",
        /// "design":"…"}`) — every field in one piped payload, no escaping. The only accepted
        /// value is `-` (canonical form `nxf update <id> --json -`), mutually exclusive with
        /// `--set`/`--set-file`.
        #[arg(value_name = "-")]
        json_stdin: Option<String>,
        /// A `field=value` assignment; repeat for multiple fields. An unknown field is rejected
        /// with the settable set listed. Aliases (same words `create` uses): `parent`→belongs_to,
        /// `defer`→defer_until. A value of `-` reads that field from STDIN (escaping-free,
        /// multi-line); at most one field per call may read STDIN.
        #[arg(long = "set")]
        set: Vec<String>,
        /// A `field=path` assignment that reads the field's value from a file (UTF-8, verbatim);
        /// `path` of `-` reads STDIN. Repeatable; lets several long-text fields each come from
        /// their own file in one call. A field may be set by `--set` or `--set-file`, not both.
        #[arg(long = "set-file")]
        set_file: Vec<String>,
    },
    /// Claim an item: mark it in progress (and optionally assign it).
    Claim {
        /// Item id.
        id: String,
        /// Assignee to record.
        #[arg(long)]
        assignee: Option<String>,
    },
    /// Archive one or more items — "closed and put away". Cascades DOWN: each root and its whole
    /// `belongs_to` subtree are archived, but only when the root is closed and every descendant is
    /// closed (already-archived counts as closed). Atomic per root, independent between roots; the
    /// result reports each root's outcome plus the full list of ids actually archived (incl.
    /// cascaded). Archiving is reversible — see `unarchive`.
    Archive {
        /// Item ids to archive (each a cascade root). At least one is required.
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Unarchive one or more items, making them visible again. Cascades UP only: each item and its
    /// ancestor chain resurface, but its children/siblings stay archived. Partial per root; the
    /// result reports each root's outcome plus the full list of ids actually unarchived.
    Unarchive {
        /// Item ids to unarchive (each surfaces its ancestor chain). At least one is required.
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Close an item, recording why. Closing is final (no hard delete), so a reason is required.
    Close {
        /// Item id.
        id: String,
        /// Closing comment — required: why this item is being closed. `-` reads it from STDIN.
        #[arg(long)]
        reason: Option<String>,
        /// Read the closing comment from this file (UTF-8, verbatim); `-` reads STDIN. Mutually
        /// exclusive with `--reason`.
        #[arg(long = "reason-file")]
        reason_file: Option<String>,
    },
    /// List items, optionally filtered by status and/or type. Id-ordered by default (so the
    /// `--json` record stays plugin-independent); override with `--sort rank`.
    List {
        /// Filter by status.
        #[arg(long)]
        status: Option<String>,
        /// Filter by type, using the active plugin's vocabulary (see `nxf schema`).
        #[arg(long = "type")]
        ty: Option<String>,
        /// Filter to items carrying this label (user vocabulary, not the display type).
        #[arg(long)]
        label: Option<String>,
        /// Sort order: `rank` | `id` (default `id`). An unknown key is a loud error.
        #[arg(long)]
        sort: Option<String>,
    },
    /// List blocked items (open, with an open blocker or in a cycle). Ranked by default; override with `--sort id`.
    Blocked {
        /// Sort order: `rank` | `id` (default `rank`). An unknown key is a loud error.
        #[arg(long)]
        sort: Option<String>,
    },
    /// List deferred items (open, unblocked, with a future defer date) — the lane that is neither
    /// ready nor blocked. Ordered by defer-date ascending (soonest first); override with `--sort`.
    Deferred {
        /// Reference time (ISO-8601 / RFC3339); defaults to now. Sets the defer boundary.
        #[arg(long)]
        now: Option<String>,
        /// Sort order: `defer` | `id` | `rank` (default `defer`). An unknown key is a loud error.
        #[arg(long)]
        sort: Option<String>,
    },
    /// List closed items (excluding archived). Ordered by close-date descending (most recent
    /// first); override with `--sort`.
    Closed {
        /// Sort order: `closed` | `id` | `rank` (default `closed`). An unknown key is a loud error.
        #[arg(long)]
        sort: Option<String>,
    },
    /// List archived items (any status). Ordered by archive-date descending (most recent first);
    /// override with `--sort`.
    Archived {
        /// Sort order: `archived` | `id` | `rank` (default `archived`). An unknown key is a loud error.
        #[arg(long)]
        sort: Option<String>,
    },
    /// Recap the most recently closed work — "what got finished lately" — newest close first,
    /// archived closes INCLUDED. A recency recall view, distinct from `closed` (the lane, which
    /// drops archived). `--limit` caps the count (default 10); `--since` keeps only later closes.
    Recap {
        /// Max items to show (default 10).
        #[arg(long)]
        limit: Option<usize>,
        /// Only closes on/after this ISO-8601 date (`YYYY-MM-DD` or RFC3339); filters `closed_at >= since`.
        #[arg(long)]
        since: Option<String>,
    },
    /// List actionable work — ready plus already-claimed, unblocked, not deferred — ranked by the
    /// active plugin's `next` policy and tiered to finish before starting: work you can close now,
    /// then each started epic with its open children, then the backlog. `--sort id` gives the flat,
    /// untiered order; `--limit` shows only the head of it, and says so.
    Next {
        /// Reference time (ISO-8601 / RFC3339); defaults to now.
        #[arg(long)]
        now: Option<String>,
        /// Filter to ready items carrying this label (user vocabulary, not the display type).
        #[arg(long)]
        label: Option<String>,
        /// Sort order: `rank` | `id` (default `rank`). An unknown key is a loud error.
        #[arg(long)]
        sort: Option<String>,
        /// Max items to show (default: no limit). Applied last — after `--sort` and `--label` —
        /// and never silently: a truncated list is headed `showing <n> of <total>`, and under
        /// `--json` the flag wraps the records as `{"items": [...], "total": <n>}` so a consumer
        /// reads the untruncated total instead of inferring it from the array's length.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Add or remove a dependency edge. Direction: `dep add A B` makes A depend on B, so B
    /// blocks A — A cannot be ready until B closes.
    Dep {
        #[command(subcommand)]
        action: DepAction,
    },
    /// Add, remove, or list reference edges (free-text short-id citations).
    Mention {
        #[command(subcommand)]
        action: MentionAction,
    },
    /// Add, remove, or list contributes-to edges: `from` contributes to `to` (n:m). Like a
    /// mention it never blocks — it records a structural "feeds into" relation, not a dependency.
    Contributes {
        #[command(subcommand)]
        action: ContributesAction,
    },
    /// Link a chat thread to a board item, or list what a thread is about. This is how the
    /// conversation that did the work and the ticket it was about stop being two artefacts: a later
    /// reader of the item sees that there are conversations, without having to go looking.
    ///
    /// n:m in both directions, and never blocking — a link carries no dependency semantics at all.
    /// It may be made at any point in the thread's life, changed, and taken back again.
    Thread {
        #[command(subcommand)]
        action: ThreadAction,
    },
    /// Add, remove, or list user labels (free-text tags) on an item. A label is user vocabulary,
    /// distinct from the plugin's display type; filter `list`/`next` with `--label`.
    Label {
        #[command(subcommand)]
        action: LabelAction,
    },
    /// Add or list notes (the change/worklog stream) on an item.
    Note {
        #[command(subcommand)]
        action: NoteAction,
    },
    /// Search items by substring over title, description, design, DoD, and notes. Results are
    /// grouped by lane priority (next+in-progress → blocked → deferred → closed), each group in its
    /// natural order; archived items are excluded by default.
    Search {
        /// Query string.
        query: String,
        /// Filter by status.
        #[arg(long)]
        status: Option<String>,
        /// Filter by type, using the active plugin's vocabulary (see `nxf schema`).
        #[arg(long = "type")]
        ty: Option<String>,
        /// Reference time (ISO-8601 / RFC3339); defaults to now. Sets the defer boundary used to
        /// group deferred matches.
        #[arg(long)]
        now: Option<String>,
        /// Append the archived group (lowest priority) instead of excluding it.
        #[arg(long = "include-archived", conflicts_with = "archived_only")]
        include_archived: bool,
        /// Search ONLY the archive; the live lanes are skipped.
        #[arg(long = "archived-only")]
        archived_only: bool,
        /// Flatten the lane grouping into one order by this key (`rank` | `id` | …). Unknown = loud error.
        #[arg(long)]
        sort: Option<String>,
    },
    /// Show one item with its dependencies and notes.
    Show {
        /// Item id.
        id: String,
    },
    /// Describe the active plugin's field model — vocabulary, priorities, and per field whether
    /// it is required on create / settable on update and how. `--json` is the machine contract;
    /// run it to learn what `create`/`update` accept in this workspace (the static `--help`
    /// cannot carry the active plugin's vocabulary).
    Schema,
    /// Wire host-specific agent integration (idempotent, merge-only). `nxf init` stays
    /// tool-agnostic; this opts a project into a host's deterministic delivery.
    Setup {
        #[command(subcommand)]
        action: SetupAction,
    },
    /// Verify a file against a detached minisign signature using the key compiled into nxf.
    /// Internal plumbing for install.sh / `self-update`; the release pipeline (85y.13/.18)
    /// is the real caller. Hidden from `--help`.
    #[command(hide = true)]
    VerifySignature {
        /// Path to the file whose signature to check.
        #[arg(long)]
        file: String,
        /// Path to the detached `.minisig` signature.
        #[arg(long)]
        signature: String,
    },
    /// Update this `nxf` in place from the release channel (sha256 + minisign verified,
    /// no downgrade). `--check` reports without installing. Hidden from `--help`
    /// (nexus-flow-gel): `nxs self-update` is the canonical, suite-wide entry; this stays a
    /// working, deprecated alias that points the user at `nxs self-update`.
    #[command(hide = true)]
    SelfUpdate {
        /// Promotion ring to update from; persisted as this install's default.
        #[arg(long, value_parser = ["stable", "beta", "alpha"])]
        channel: Option<String>,
        /// Report whether an update is available without installing it.
        #[arg(long)]
        check: bool,
    },
    /// Print an embedded, offline guide. Omit the topic to list the available ones.
    Guide {
        /// Guide topic; omit to list topics.
        topic: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum SetupAction {
    /// Wire Claude Code: (re)assemble the shared agent file (AGENTS.md) and one
    /// SessionStart hook per active module (`nxf prime`, `nxm prime`, `nxc prime`) plus the
    /// `nxs`/module permission allowlist, merged into `.claude/settings.json` (idempotent, never
    /// overwriting). The canonical verb is `nxs setup claude`; this delegates to it (host setup is
    /// an umbrella responsibility).
    Claude,
}

#[derive(Subcommand, Debug)]
enum DepAction {
    /// Make `from` depend on `to`: `to` blocks `from`, so `from` stays blocked until `to`
    /// closes. Rejected if it would create a cycle.
    Add { from: String, to: String },
    /// Remove the dependency where `from` depends on `to`.
    Remove { from: String, to: String },
}

#[derive(Subcommand, Debug)]
enum MentionAction {
    /// Record that `from` cites the short-id `to` in its free text (no blocking).
    Add { from: String, to: String },
    /// Remove the `from -> to` reference.
    Remove { from: String, to: String },
    /// List the short-ids `id` mentions.
    List { id: String },
}

#[derive(Subcommand, Debug)]
enum ThreadAction {
    /// Link `thread` to `item`. Re-running with different attributes UPDATES the link rather than
    /// adding a second one — that is how a link firms up once a passing mention turns out to be the
    /// subject.
    Link {
        /// The chat thread's own id. Not resolved or shape-checked: the thread lives in the chat
        /// store's id space, which flow holds the address of but does not own.
        thread: String,
        /// The board item.
        item: String,
        /// What the link consists in: `worked_on` (this thread is working on the item) or `cited`
        /// (it refers to the item without working on it). Default `worked_on` — the deterministic
        /// case, the one a work order's ticket set produces with no judgement involved.
        #[arg(long, default_value = "worked_on")]
        relation: String,
        /// How much the thread is about the item: `bearing` (the item is its subject) or `passing`
        /// (it came up, no more). Default `bearing`. Only `bearing` links are advertised by `next`;
        /// a `passing` one shows on the item itself, so a wandering conversation cannot flood the
        /// work list with items it merely touched.
        #[arg(long, default_value = "bearing")]
        weight: String,
    },
    /// Unlink `thread` from `item`, whatever attributes the link currently carries.
    Unlink { thread: String, item: String },
    /// List the board items `thread` is linked to.
    List { thread: String },
}

#[derive(Subcommand, Debug)]
enum ContributesAction {
    /// Record that `from` contributes to `to` (both must exist; never blocks).
    Add { from: String, to: String },
    /// Remove the `from -> to` contributes-to edge.
    Remove { from: String, to: String },
    /// List the items `id` contributes to.
    List { id: String },
}

#[derive(Subcommand, Debug)]
enum LabelAction {
    /// Attach a label to an item (idempotent). The label is trimmed; a blank label is rejected.
    Add { id: String, label: String },
    /// Detach a label from an item (observed-remove).
    Remove { id: String, label: String },
    /// List an item's labels (sorted).
    List { id: String },
}

#[derive(Subcommand, Debug)]
enum NoteAction {
    /// Append a note to an item. Pass `-` as the text to read the note body from STDIN
    /// (escaping-free, multi-line).
    Add { id: String, text: String },
    /// List an item's notes.
    List { id: String },
}

/// The fully-built clap command tree for `nxf` — the single source of truth that
/// `cargo xtask docs` renders into man-pages, Markdown, and the command-tree JSON.
/// Keeping generation pinned to this (rather than a hand-written copy) is what makes
/// the docs drift gate meaningful: the docs cannot diverge from the parser.
pub fn command() -> clap::Command {
    Cli::command()
}

/// Entry point for the `nxf` binary. `src/main.rs` is a thin shim over this so the
/// parser definition can also live in the library (see [`command`]). Parses the real process
/// args; [`run_from`] is the multicall seam the `nxs` umbrella routes `nxf`/`nxs flow` through.
pub fn run() -> ExitCode {
    run_from(std::env::args_os())
}

/// Run the `nxf` surface over an explicit argv (multicall seam, nexus-flow-5jz.7). `args` is a full
/// argv INCLUDING the program name at index 0 — clap takes the program name from there (so a
/// synthesized `nxf` argv0 from `nxs flow …` renders identical `nxf` help/errors), and parses the
/// rest. `run()` passes the real `std::env::args_os()`; the umbrella passes `nxf` + the post-`flow`
/// tokens. Behaviour is otherwise identical to the former `run()`.
pub fn run_from<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    // Give human output room to breathe: a blank line before the first line and after the
    // last (nexus-flow-vwx). `--json` is the machine contract and stays byte-exact, so it is
    // NEVER framed. A driven `init --quiet` (S5) is also never framed — the umbrella drives it
    // and it must emit nothing on stdout, not even structural blanks. The leading blank goes to
    // stdout up front; the trailing blank follows the payload on its own stream — stdout on
    // success here, stderr in `NxfError::emit` on error.
    // `-q` create (id-only) is unframed too: it emits exactly the new id on one line for capture,
    // so the leading/trailing structural blanks must not bleed in (nexus-flow-82h).
    let framed = !cli.json
        && !matches!(cli.command, Command::Init { quiet: true, .. })
        && !matches!(cli.command, Command::Create { id_only: true, .. });
    if framed {
        println!();
    }
    match dispatch(&cli) {
        Ok(()) => {
            if framed {
                println!();
            }
            // A dezent, once-a-day "update available" nudge on stderr (nexus-flow-85y.21).
            // Skipped after `self-update` itself (it already reported the update state) and
            // silent for any non-interactive use — see `maybe_emit_update_hint`.
            if !matches!(cli.command, Command::SelfUpdate { .. }) {
                selfupdate::maybe_emit_update_hint(cli.json);
            }
            ExitCode::SUCCESS
        }
        Err(e) => ExitCode::from(error::emit(&e, cli.json) as u8),
    }
}

/// The self-update flow branded for the `nxs` umbrella — the canonical `nxs self-update`
/// (nexus-flow-gel). The umbrella crate (`nxs`) links this crate, which owns the updater, the
/// embedded signing key, and the suite-wide atomic swap; it calls this so `nxs self-update` and
/// the hidden, deprecated `nxf self-update` share ONE code path and ONE swap. Suite-neutral: it
/// updates the whole suite (the single real `nxs` binary plus the `nxf`/`nxm` persona links),
/// including the binary currently running, regardless of which name invoked it. Returns the shared
/// error envelope on failure so the umbrella renders it through its own `error::emit`; success and
/// `--json` output are byte-identical to the `nxf` path bar the brand on human lines.
pub fn self_update(json: bool, check: bool, channel: Option<&str>) -> Result<()> {
    selfupdate::run("nxs", json, check, channel)
}

fn dispatch(cli: &Cli) -> Result<()> {
    let db = cli.db.as_deref();
    match &cli.command {
        Command::Init {
            plugin,
            quiet,
            from_beads,
        } => commands::init(cli.json, *quiet, plugin.as_deref(), *from_beads),
        Command::Prime => commands::prime(cli.json, db),
        Command::AgentManifest => commands::agent_manifest(cli.json),
        Command::Create {
            ty,
            title,
            description,
            description_file,
            priority,
            design,
            design_file,
            dod,
            dod_file,
            due,
            defer,
            parent,
            depends_on,
            set,
            set_file,
            id_only,
            json_stdin,
        } => commands::create(
            cli.json,
            db,
            ty.as_deref(),
            title.as_deref(),
            commands::CreateArgs {
                description: description.as_deref(),
                description_file: description_file.as_deref(),
                priority: priority.as_deref(),
                design: design.as_deref(),
                design_file: design_file.as_deref(),
                dod: dod.as_deref(),
                dod_file: dod_file.as_deref(),
                due: due.as_deref(),
                defer: defer.as_deref(),
                parent: parent.as_deref(),
                depends_on: depends_on.as_slice(),
                set: set.as_slice(),
                set_file: set_file.as_slice(),
            },
            json_stdin.as_deref(),
            *id_only,
        ),
        Command::Update {
            id,
            set,
            set_file,
            json_stdin,
        } => commands::update(cli.json, db, id, set, set_file, json_stdin.as_deref()),
        Command::Claim { id, assignee } => commands::claim(cli.json, db, id, assignee.as_deref()),
        Command::Archive { ids } => commands::archive_items(cli.json, db, ids),
        Command::Unarchive { ids } => commands::unarchive_items(cli.json, db, ids),
        Command::Close {
            id,
            reason,
            reason_file,
        } => commands::close(cli.json, db, id, reason.as_deref(), reason_file.as_deref()),
        Command::List {
            status,
            ty,
            label,
            sort,
        } => commands::list(
            cli.json,
            db,
            status.as_deref(),
            ty.as_deref(),
            label.as_deref(),
            sort.as_deref(),
        ),
        Command::Blocked { sort } => commands::blocked(cli.json, db, sort.as_deref()),
        Command::Deferred { now, sort } => {
            commands::deferred(cli.json, db, now.as_deref(), sort.as_deref())
        }
        Command::Closed { sort } => commands::closed(cli.json, db, sort.as_deref()),
        Command::Archived { sort } => commands::archived(cli.json, db, sort.as_deref()),
        Command::Recap { limit, since } => commands::recap(cli.json, db, *limit, since.as_deref()),
        Command::Next {
            now,
            label,
            sort,
            limit,
        } => commands::next(
            cli.json,
            db,
            now.as_deref(),
            label.as_deref(),
            sort.as_deref(),
            *limit,
        ),
        Command::Dep { action } => match action {
            DepAction::Add { from, to } => commands::dep_add(cli.json, db, from, to),
            DepAction::Remove { from, to } => commands::dep_remove(cli.json, db, from, to),
        },
        Command::Mention { action } => match action {
            MentionAction::Add { from, to } => commands::mention_add(cli.json, db, from, to),
            MentionAction::Remove { from, to } => commands::mention_remove(cli.json, db, from, to),
            MentionAction::List { id } => commands::mention_list(cli.json, db, id),
        },
        Command::Thread { action } => match action {
            ThreadAction::Link {
                thread,
                item,
                relation,
                weight,
            } => commands::thread_link(cli.json, db, thread, item, relation, weight),
            ThreadAction::Unlink { thread, item } => {
                commands::thread_unlink(cli.json, db, thread, item)
            }
            ThreadAction::List { thread } => commands::thread_list(cli.json, db, thread),
        },
        Command::Contributes { action } => match action {
            ContributesAction::Add { from, to } => {
                commands::contributes_add(cli.json, db, from, to)
            }
            ContributesAction::Remove { from, to } => {
                commands::contributes_remove(cli.json, db, from, to)
            }
            ContributesAction::List { id } => commands::contributes_list(cli.json, db, id),
        },
        Command::Label { action } => match action {
            LabelAction::Add { id, label } => commands::label_add(cli.json, db, id, label),
            LabelAction::Remove { id, label } => commands::label_remove(cli.json, db, id, label),
            LabelAction::List { id } => commands::label_list(cli.json, db, id),
        },
        Command::Note { action } => match action {
            NoteAction::Add { id, text } => commands::note_add(cli.json, db, id, text),
            NoteAction::List { id } => commands::note_list(cli.json, db, id),
        },
        Command::Search {
            query,
            status,
            ty,
            now,
            include_archived,
            archived_only,
            sort,
        } => commands::search(
            cli.json,
            db,
            query,
            commands::SearchArgs {
                status: status.as_deref(),
                item_type: ty.as_deref(),
                now: now.as_deref(),
                include_archived: *include_archived,
                archived_only: *archived_only,
                sort: sort.as_deref(),
            },
        ),
        Command::Show { id } => commands::show(cli.json, db, id),
        Command::Schema => commands::schema(cli.json, db),
        Command::Setup { action } => match action {
            SetupAction::Claude => commands::setup_claude(cli.json),
        },
        Command::VerifySignature { file, signature } => {
            crate::signature::verify_command(cli.json, file, signature)
        }
        Command::SelfUpdate { channel, check } => {
            // The hidden, deprecated alias (nexus-flow-gel): still fully functional and suite-wide,
            // but nudge the user to the canonical `nxs self-update`. Stderr only and never under
            // `--json`, so the machine contract on stdout stays byte-identical to the old surface.
            if !cli.json {
                eprintln!("note: `nxf self-update` is deprecated — use `nxs self-update` instead.");
            }
            crate::selfupdate::run("nxf", cli.json, *check, channel.as_deref())
        }
        Command::Guide { topic } => commands::guide(cli.json, topic.as_deref()),
    }
}
