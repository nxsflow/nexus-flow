//! `nxs mcp install` (E9 #c3i): register the running `nxs` binary as an MCP server in installed
//! hosts' config files — the "nxs is not on the host's PATH" half of the distribution problem.
//!
//! The writer is IDEMPOTENT and NON-DESTRUCTIVE: it upserts a single `mcpServers.nxs` entry naming
//! the launch command + `mcp serve`, preserving every other key in the file. A re-run whose entry
//! already matches touches nothing. Two launch modes ([`Runner`], `--runner`): `native` (default)
//! names the ABSOLUTE path of the running binary — the "nxs is installed but not on the host's PATH"
//! case; `npx` writes the portable `npx @nexus-flow/mcp` runner shim (#kz8) that fetches + verifies +
//! execs `nxs` on demand — the "not installed at all" case, and machine-independent.
//!
//! `--workspace` is OPTIONAL in the written entry (per #76u.7/#76u.8): omit it and the server serves
//! its auto-initialized per-user App-Data-Home default; pass a path to pin a specific project
//! workspace. The absolute `nxs` path is always mandatory.

use crate::error::{NxfError, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The server key written under a host's `mcpServers` map. Kept byte-identical to the server's
/// advertised Implementation name (`nxs mcp serve`'s `get_info`), so a host shows one stable name.
const SERVER_NAME: &str = "nxs";

/// The flow plugin the optional install-time setup (#65y) seats when `--plugin` is omitted.
/// Deliberately `personal-todo` (the knowledge-worker board), NOT flow's own `issue-tracker`
/// [`DEFAULT_PLUGIN`](nexus_flow_facade::workspace::DEFAULT_PLUGIN): the one-command onboarding flow
/// leads with the personal board; the coding case opts in explicitly with `--plugin issue-tracker`.
///
/// This divergence is observable on the *shared* App-Data-Home (the no-`--workspace` tier): the
/// server's own first-start auto-init seeds `issue-tracker` (`mcp/mod.rs`), so whichever path
/// materializes the board FIRST wins, and `setup` never re-seats after (never clobbers). That is
/// intentional, not a race to fix — the receipt reads the seated plugin back from disk, so it never
/// misreports which board exists. Corollary (inherited from the #c3i flag-less default entry): a
/// flag-less entry re-resolves the App-Data-Home from the environment at *serve* time, so if the
/// host launches `mcp serve` under a different env than install ran in (e.g. install under `sudo`,
/// serve as the user), the two can resolve different homes; pin an explicit `--workspace` to remove
/// that dependency.
const SETUP_DEFAULT_PLUGIN: &str = "personal-todo";

/// A known MCP host the server can be registered into. Every supported host shares the near-
/// universal `mcpServers` config schema (`{ "<name>": { "command", "args" } }`) — Claude Desktop,
/// Cursor, Windsurf, Amazon Quick — so one writer serves them all; only the config-file location
/// differs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Host {
    ClaudeDesktop,
    Cursor,
    Windsurf,
    /// Amazon Quick (the renamed Amazon Q Developer, 5kky). Its CLI reads the same `mcpServers`
    /// schema from a global config under `$HOME/.aws/amazonq/mcp.json`.
    AmazonQuick,
}

/// Every known host, in a stable display/scan order.
const HOSTS: &[Host] = &[
    Host::ClaudeDesktop,
    Host::Cursor,
    Host::Windsurf,
    Host::AmazonQuick,
];

impl Host {
    /// The stable `--host` id and `--json` key.
    fn id(self) -> &'static str {
        match self {
            Host::ClaudeDesktop => "claude-desktop",
            Host::Cursor => "cursor",
            Host::Windsurf => "windsurf",
            Host::AmazonQuick => "amazon-quick",
        }
    }

    /// The human label for text output.
    fn label(self) -> &'static str {
        match self {
            Host::ClaudeDesktop => "Claude Desktop",
            Host::Cursor => "Cursor",
            Host::Windsurf => "Windsurf",
            Host::AmazonQuick => "Amazon Quick",
        }
    }

    /// Parse a `--host` id back to a host (case-insensitive), `None` for an unknown id.
    fn from_id(id: &str) -> Option<Host> {
        HOSTS
            .iter()
            .copied()
            .find(|h| h.id().eq_ignore_ascii_case(id))
    }

    /// This host's config-file path under the given base directories. Claude Desktop lives under the
    /// platform config dir (`.../Claude/claude_desktop_config.json`), which on macOS coincides with
    /// Application Support; Cursor, Windsurf, and Amazon Quick keep their config under a fixed dotdir
    /// in `$HOME` (Amazon Quick's is the Amazon Q Developer CLI's global `~/.aws/amazonq/mcp.json`).
    fn config_path(self, base: &BaseDirs) -> PathBuf {
        match self {
            Host::ClaudeDesktop => base
                .config
                .join("Claude")
                .join("claude_desktop_config.json"),
            Host::Cursor => base.home.join(".cursor").join("mcp.json"),
            Host::Windsurf => base
                .home
                .join(".codeium")
                .join("windsurf")
                .join("mcp_config.json"),
            Host::AmazonQuick => base.home.join(".aws").join("amazonq").join("mcp.json"),
        }
    }
}

/// The base directories host config paths resolve against — snapshotted (not the live `directories`
/// resolver) so the whole path/detect/apply core is unit-testable against a tempdir.
struct BaseDirs {
    /// The user home (`$HOME`), Cursor/Windsurf's config root.
    home: PathBuf,
    /// The platform config dir (`dirs::config_dir()`), Claude Desktop's config root.
    config: PathBuf,
}

/// The successful disposition of one host's config file (the fallible bits surface as `Err` and are
/// mapped to [`HostResult::Failed`] by the auto-detect scan).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    /// The entry was added or updated (the file was written).
    Written,
    /// The entry already matched — nothing was written (the idempotent path).
    Unchanged,
    /// The host was not detected and not explicitly requested — left untouched.
    Skipped,
}

/// What became of one host — the applied [`Status`], or a per-host failure captured during an
/// auto-detect scan so one unreadable/corrupt/locked config never strands the OTHER hosts.
#[derive(Clone, PartialEq, Eq, Debug)]
enum HostResult {
    Written,
    Unchanged,
    Skipped,
    /// Auto-detect only: this host errored (corrupt/non-object/unreadable config) but the scan went
    /// on. Carries the human message for rendering. An explicit `--host` fails loudly instead.
    Failed(String),
}

impl HostResult {
    fn as_str(&self) -> &'static str {
        match self {
            HostResult::Written => "written",
            HostResult::Unchanged => "unchanged",
            HostResult::Skipped => "skipped",
            HostResult::Failed(_) => "error",
        }
    }
}

/// One host's outcome, for rendering.
#[derive(Debug)]
struct HostOutcome {
    host: Host,
    path: PathBuf,
    result: HostResult,
}

/// The npm runner shim (#kz8) the `npx` runner writes: it fetches the signed `nxs` prebuilt for the
/// platform, verifies it (sha256 + minisign), caches it, and execs `nxs mcp serve`. Lives in
/// `npm/mcp` in this repo, published as `@nexus-flow/mcp`.
const NPX_PACKAGE: &str = "@nexus-flow/mcp";

/// How a written host entry launches the server. `Native` names the ABSOLUTE path of the already-
/// installed `nxs` (this binary) — no runtime dependency, but the entry is machine-specific and
/// breaks if the binary moves. `Npx` writes the portable `npx @nexus-flow/mcp` runner shim (#kz8):
/// machine-independent and the ONLY entry that works before `nxs` is installed at all, at the cost of
/// a Node dependency and a one-time first-run download. Native is the default.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Runner {
    #[default]
    Native,
    Npx,
}

impl Runner {
    /// Parse the `--runner` value, naming the bad id on an unknown one.
    fn parse(s: &str) -> Result<Runner> {
        match s {
            "native" => Ok(Runner::Native),
            "npx" => Ok(Runner::Npx),
            other => Err(NxfError::validation(format!(
                "unknown --runner `{other}` — known runners: native, npx"
            ))),
        }
    }

    /// Build this runner's `{ "command", "args" }` host entry. `nxs_abs` is the absolute path of the
    /// running binary (used only by `Native`; `Npx` ignores it — the shim resolves `nxs` itself).
    /// A pinned `workspace` is passed through to `nxs mcp serve --workspace <ws>` either way.
    fn entry(self, nxs_abs: &str, workspace: Option<&str>) -> Value {
        match self {
            Runner::Native => server_entry(nxs_abs, workspace),
            Runner::Npx => npx_entry(workspace),
        }
    }

    /// The launch command shown in the report (`{command}` in the summary / `--json`).
    fn display(self, nxs_abs: &str) -> String {
        match self {
            Runner::Native => nxs_abs.to_string(),
            Runner::Npx => format!("npx -y {NPX_PACKAGE}"),
        }
    }
}

/// The native server entry `{ "command": <abs nxs>, "args": ["mcp","serve", ...] }`. `--workspace`
/// is appended only when a path is pinned; omitted, the server uses its App-Data-Home default.
/// `command` is the already-UTF-8-validated absolute path of the running binary (see [`command_str`]).
fn server_entry(command: &str, workspace: Option<&str>) -> Value {
    let mut args = vec![json!("mcp"), json!("serve")];
    if let Some(ws) = workspace {
        args.push(json!("--workspace"));
        args.push(json!(ws));
    }
    json!({ "command": command, "args": args })
}

/// The npx runner entry `{ "command": "npx", "args": ["-y","@nexus-flow/mcp", "--", ...] }`. Args
/// after `--` are the shim's passthrough to `nxs mcp serve` (the shim prepends `mcp serve`), so a
/// pinned workspace becomes `-- --workspace <ws>`; with no workspace the trailing separator is
/// dropped for a clean entry.
fn npx_entry(workspace: Option<&str>) -> Value {
    let mut args = vec![json!("-y"), json!(NPX_PACKAGE)];
    if let Some(ws) = workspace {
        args.push(json!("--"));
        args.push(json!("--workspace"));
        args.push(json!(ws));
    }
    json!({ "command": "npx", "args": args })
}

/// Merge `entry` into `config`'s `mcpServers.<SERVER_NAME>`, preserving every other top-level key and
/// every other server. A GENUINELY ABSENT `mcpServers` is created; a PRESENT-but-non-object root or
/// `mcpServers` is a loud [`NxfError::validation`] — we refuse to overwrite readable-but-unexpected
/// user data (matching the corrupt-file policy in [`apply_host`]) rather than silently discarding it.
/// Returns whether anything changed — `false` means the entry already matched (idempotent no-op).
///
/// Note: on a change the file is re-serialized, which alphabetizes the top-level keys (serde_json has
/// no `preserve_order`). That is semantically non-destructive and stays idempotent, but it reorders a
/// hand-edited file's keys once.
fn upsert_entry(config: &mut Value, entry: &Value) -> Result<bool> {
    let root = config.as_object_mut().ok_or_else(|| {
        NxfError::validation("host config root is not a JSON object — refusing to overwrite it")
    })?;
    // `entry(..).or_insert_with` creates an object ONLY when `mcpServers` is absent; a present value
    // is left as-is, so a present-but-non-object `mcpServers` falls through to the loud error below.
    let servers = root.entry("mcpServers").or_insert_with(|| json!({}));
    let servers = servers.as_object_mut().ok_or_else(|| {
        NxfError::validation(
            "host config `mcpServers` is not a JSON object — refusing to overwrite it",
        )
    })?;
    if servers.get(SERVER_NAME) == Some(entry) {
        return Ok(false);
    }
    servers.insert(SERVER_NAME.to_string(), entry.clone());
    Ok(true)
}

/// Whether a host looks installed: its config file already exists, or its parent directory does
/// (the app has run at least once and created its support dir).
fn is_present(path: &Path) -> bool {
    path.exists() || path.parent().is_some_and(Path::is_dir)
}

/// Apply the entry to one host's config file. `force` writes even when the host is absent (an
/// explicit `--host`), creating parent directories; otherwise an absent host is left `Skipped`.
fn apply_host(path: &Path, entry: &Value, force: bool) -> Result<Status> {
    if !force && !is_present(path) {
        return Ok(Status::Skipped);
    }
    // Parse the existing config (absent/empty ⇒ a fresh `{}`); a present-but-corrupt file is a loud
    // error rather than a silent clobber of the user's other servers. A parse failure is a malformed-
    // input problem, so it is a `validation` error (consistent with `upsert_entry`'s non-object case).
    let mut config = match std::fs::read_to_string(path) {
        Ok(raw) if !raw.trim().is_empty() => serde_json::from_str(&raw).map_err(|e| {
            NxfError::validation(format!(
                "existing host config {} is not valid JSON — refusing to overwrite it: {e}",
                path.display()
            ))
        })?,
        Ok(_) => json!({}),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => {
            return Err(NxfError::io(format!(
                "reading host config {}: {e}",
                path.display()
            )))
        }
    };
    if !upsert_entry(&mut config, entry)? {
        return Ok(Status::Unchanged);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            NxfError::io(format!(
                "creating host config dir {}: {e}",
                parent.display()
            ))
        })?;
    }
    let mut rendered = serde_json::to_string_pretty(&config)
        .map_err(|e| NxfError::io(format!("serializing host config: {e}")))?;
    rendered.push('\n');
    atomic_write(path, rendered.as_bytes())?;
    Ok(Status::Written)
}

/// Write `contents` to `path` atomically: write a sibling temp file in the SAME directory, then
/// rename it over the target. A same-dir rename is atomic, so a crash or concurrent reader only ever
/// sees the whole old file or the whole new one — never a truncated mix that would strand the user's
/// OTHER MCP servers. The temp is opened O_EXCL via [`super::write_new_exclusive`], so a symlink
/// squatting on its predictable path is refused rather than followed (0jq8 Integrity #1). Mirrors the
/// foundation config/settings writers' temp+rename idiom.
fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("host");
    let tmp = dir.join(format!(".{name}.nxs.tmp.{}", std::process::id()));
    super::write_new_exclusive(&tmp, contents)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp); // best-effort: never strand the temp sibling
            NxfError::io(format!("writing host config {}: {e}", path.display()))
        })
}

/// Resolve `--host` ids to hosts, erroring (naming the bad id) on any unknown one. An empty input
/// stays empty (the auto-detect-all signal).
fn resolve_selected(hosts: &[String]) -> Result<Vec<Host>> {
    hosts
        .iter()
        .map(|id| {
            Host::from_id(id).ok_or_else(|| {
                let known: Vec<&str> = HOSTS.iter().map(|h| h.id()).collect();
                NxfError::validation(format!(
                    "unknown --host `{id}` — known hosts: {}",
                    known.join(", ")
                ))
            })
        })
        .collect()
}

/// Plan + apply the already-built `entry` across the selected hosts (the testable core). `selected`
/// empty ⇒ auto-detect every known host (write the present ones, skip the absent); non-empty ⇒
/// exactly those, force-written. The entry is built by the caller (see [`Runner::entry`]) so this
/// stays a pure fan-out over hosts, agnostic to native-vs-npx.
fn install_into(base: &BaseDirs, entry: &Value, selected: &[Host]) -> Result<Vec<HostOutcome>> {
    // Explicit `--host` targets exactly those, force-written; no `--host` scans every known host and
    // writes only the detected ones.
    let (hosts, force): (Vec<Host>, bool) = if selected.is_empty() {
        (HOSTS.to_vec(), false)
    } else {
        (selected.to_vec(), true)
    };
    let mut out = Vec::with_capacity(hosts.len());
    for host in hosts {
        let path = host.config_path(base);
        let result = match apply_host(&path, entry, force) {
            Ok(Status::Written) => HostResult::Written,
            Ok(Status::Unchanged) => HostResult::Unchanged,
            Ok(Status::Skipped) => HostResult::Skipped,
            // Auto-detect (no explicit `--host`): a single corrupt/unreadable/locked host config must
            // not strand the healthy ones — record it and carry on. An explicit `--host` still fails
            // loudly, because the user named exactly that target and expects it written.
            Err(e) if !force => HostResult::Failed(e.msg),
            Err(e) => return Err(e),
        };
        out.push(HostOutcome { host, path, result });
    }
    Ok(out)
}

/// Absolutize an explicit `--workspace` lexically (no filesystem access, no symlink resolution),
/// erroring on a non-UTF-8 result; `None` stays `None` and an already-absolute path passes through
/// unchanged. This makes the setup target, the pinned host entry, and the reported path ONE absolute
/// path: a RELATIVE `--workspace` would otherwise be resolved against the install-process cwd for
/// setup but re-resolved against the HOST's (useless) cwd when it later launches `mcp serve`, so
/// setup and the server would open two different boards — the #65y trap. Absolutizing at the single
/// entry point also hardens the plain (no-setup) host entry, where a relative path never made sense.
fn absolutize_workspace(workspace: Option<&str>) -> Result<Option<String>> {
    let Some(ws) = workspace else {
        return Ok(None);
    };
    let abs = std::path::absolute(ws)
        .map_err(|e| NxfError::io(format!("resolving --workspace `{ws}`: {e}")))?;
    abs.to_str().map(|s| Some(s.to_string())).ok_or_else(|| {
        NxfError::io(format!(
            "the resolved --workspace path is not valid UTF-8: {}",
            abs.display()
        ))
    })
}

/// What the optional install-time workspace setup (#65y) materialized: the seated flow plugin and
/// the root the `.nxs/` was created under — the same root the entry's `--workspace` names (or, when
/// no `--workspace` was pinned, the App-Data-Home the server resolves by default).
struct SetupOutcome {
    plugin: String,
    /// The absolute workspace root the board lives under, as validated UTF-8 for the report. A
    /// non-UTF-8 path (possible only on the App-Data-Home tier under a non-UTF-8 `$HOME`) is a loud
    /// error in [`run_setup_in`] — never a `to_string_lossy` U+FFFD mangling that would put a wrong-
    /// but-silent path in the deterministic `--json` receipt (matching [`command_str`]).
    workspace: String,
}

/// Validate the requested setup plugin against the bundled set, defaulting to [`SETUP_DEFAULT_PLUGIN`]
/// when `--plugin` is omitted. An unknown id is a loud [`NxfError::validation`] naming it and the
/// known set — so a typo fails BEFORE any board is materialized, never a silently-unloadable plugin
/// the server only chokes on at connect time.
fn resolve_plugin(plugin: Option<&str>) -> Result<&str> {
    let name = plugin.unwrap_or(SETUP_DEFAULT_PLUGIN);
    let names = nexus_flow_facade::plugin::plugin_names();
    if names.contains(&name) {
        Ok(name)
    } else {
        Err(NxfError::validation(format!(
            "unknown --plugin `{name}` — known plugins: {}",
            names.join(", ")
        )))
    }
}

/// Materialize the launch workspace for the optional install-time setup (#65y) at the target the
/// server will resolve — keeping init-target == server-resolution (the #65y design trap). The target
/// is chosen the SAME way the server's own launch resolution chooses it:
///
///   - `workspace = Some(path)` → init THAT path (the entry pins it via `--workspace`, so the server
///     resolves the same path).
///   - `workspace = None`       → init the App-Data-Home (the entry omits `--workspace`, so the
///     server defaults to that same per-user dir).
///
/// It NEVER inits the cwd, so it can never leave setup and the server pointing at two different
/// boards. The App-Data-Home resolver is injected (mirroring the server's `launch_workspace_in`) so
/// the tiering is unit-testable against a tempdir, and is consulted ONLY on the `None` tier. The
/// foundation `setup` is idempotent and never clobbers, so a re-run loads the existing board
/// unchanged (it never re-seats the plugin).
fn run_setup_in(
    workspace: Option<&str>,
    plugin: &str,
    app_data_home: impl FnOnce() -> Result<PathBuf>,
) -> Result<SetupOutcome> {
    let root = match workspace {
        Some(ws) => PathBuf::from(ws),
        None => app_data_home()?,
    };
    // Validate the root is UTF-8 BEFORE materializing, so a non-UTF-8 App-Data-Home fails loud (never
    // a U+FFFD report) and no board is created on a path the receipt couldn't faithfully name. The
    // `Some(ws)` tier is already UTF-8 (absolutized by `absolutize_workspace`); this guards the
    // `None`/App-Data-Home tier.
    let workspace = root
        .to_str()
        .ok_or_else(|| {
            NxfError::io(format!(
                "the resolved workspace path is not valid UTF-8: {}",
                root.display()
            ))
        })?
        .to_string();
    use nexus_flow_facade::workspace as fws;
    let ws = nxs_foundation::workspace::setup(&root, &fws::flow_config(plugin)).map_err(|e| {
        NxfError::io(format!(
            "setting up the workspace at {}: {}",
            root.display(),
            e.msg
        ))
    })?;
    Ok(SetupOutcome {
        // Report the ACTUALLY-seated plugin read back from the returned workspace, NOT the requested
        // one: on the idempotent path `setup` loads a pre-existing board unchanged (it never
        // re-seats), so a board already on issue-tracker stays issue-tracker even if we asked for
        // personal-todo. Reporting the request would make the receipt lie about the board.
        plugin: fws::flow_plugin(&ws.config),
        workspace,
    })
}

/// The running `nxs` binary's absolute path — the `command` every host entry points at. Under
/// multicall (#5jz.7) `current_exe()` resolves the persona symlink back to the real `nxs`, which is
/// exactly what `mcp serve` needs to run on.
fn running_exe() -> Result<PathBuf> {
    std::env::current_exe()
        .map_err(|e| NxfError::io(format!("locating the running nxs executable: {e}")))
}

/// The platform base directories: `$HOME` (Cursor/Windsurf) + the platform config dir (Claude
/// Desktop). The same cross-platform `directories` resolver the MCP `serve` default already uses.
fn platform_base_dirs() -> Result<BaseDirs> {
    let dirs = directories::BaseDirs::new().ok_or_else(|| {
        NxfError::io("could not resolve home/config directories for MCP host detection")
    })?;
    Ok(BaseDirs {
        home: dirs.home_dir().to_path_buf(),
        config: dirs.config_dir().to_path_buf(),
    })
}

/// The running binary's path as UTF-8 for the host `command` field. A non-UTF-8 path is a loud error
/// rather than a `to_string_lossy` mangling (U+FFFD) that would write a `command` no host can launch.
fn command_str(exe: &Path) -> Result<&str> {
    exe.to_str().ok_or_else(|| {
        NxfError::io(format!(
            "the running nxs path is not valid UTF-8, cannot write a host entry: {}",
            exe.display()
        ))
    })
}

/// `nxs mcp install`: resolve the running binary + platform base dirs, optionally set up the
/// workspace, apply the host entries, and render the report.
///
/// `setup` (or any `plugin`, which implies it, #65y) additionally materializes the launch workspace
/// at the target the server will resolve. The entry's `--workspace` and the setup target are BOTH
/// derived from the same `workspace` arg under the same `None → App-Data-Home` rule the server uses,
/// so they cannot diverge — the guard against the "setup and server point at two different boards"
/// trap. Setup runs FIRST: if it fails, no host entry is written pointing at a board that never got
/// materialized.
pub fn install(
    json: bool,
    workspace: Option<&str>,
    hosts: &[String],
    setup: bool,
    plugin: Option<&str>,
    runner: &str,
) -> Result<()> {
    let runner = Runner::parse(runner)?;
    let selected = resolve_selected(hosts)?;
    let exe = running_exe()?;
    let nxs_abs = command_str(&exe)?;
    let base = platform_base_dirs()?;

    // Absolutize `--workspace` ONCE, up front, so the setup target, the pinned host entry, and the
    // reported path are all the same absolute path — never a relative string the host re-resolves
    // against its own cwd (#65y). Everything downstream uses this absolute form.
    let workspace = absolutize_workspace(workspace)?;
    let workspace = workspace.as_deref();

    // A bare `--plugin` implies `--setup` (its only purpose is to choose the setup plugin), so either
    // flag triggers the workspace materialization.
    let setup_outcome = if setup || plugin.is_some() {
        let requested = resolve_plugin(plugin)?;
        // The SAME App-Data-Home resolver the server's launch resolution uses (`super::app_data_home`),
        // so init-target == server-resolution on the default tier — never a reimplementation that
        // could drift to a different directory.
        let outcome = run_setup_in(workspace, requested, super::app_data_home)?;
        // If the user EXPLICITLY named a `--plugin` but the board already existed with a different
        // one, `setup` never re-seats (never clobbers) — so their request was a silent no-op. Say so
        // on STDERR (stdout stays the pure report/`--json` channel) so a human isn't left wondering
        // why `--plugin` had no effect. Only for an explicit request: a defaulted plugin meeting an
        // existing board is the expected first-writer-wins case, not a surprise. The receipt's
        // `setup.plugin` already reports the real seated plugin either way.
        if let Some(explicit) = plugin {
            if outcome.plugin != explicit {
                eprintln!(
                    "nxs mcp install: workspace already set up with plugin `{}`; ignoring \
                     `--plugin {explicit}` (setup never re-seats an existing board)",
                    outcome.plugin
                );
            }
        }
        Some(outcome)
    } else {
        None
    };

    // Auto-register a PINNED workspace into the multi-workspace registry (0jq8) so a host can later
    // select it by name via `list_workspaces` — the "registry maintained" upkeep hook. Only an
    // explicit `--workspace` is registered (the App-Data-Home default is the unnamed fallback, not a
    // named board). Done BEFORE the host writes so a registry failure is loud and strands nothing —
    // never a silent skip.
    if let Some(ws) = workspace {
        super::registry::register_workspace(ws)?;
    }

    let entry = runner.entry(nxs_abs, workspace);
    let outcomes = install_into(&base, &entry, &selected)?;
    // Report the runner's launch command (the abs nxs path, or the `npx …` invocation), not the raw
    // binary path — the JSON `command` must match what was actually written into the host entry.
    render(
        json,
        &runner.display(nxs_abs),
        workspace,
        setup_outcome.as_ref(),
        &outcomes,
    );
    Ok(())
}

/// Render the install report — the `--json` envelope or the aligned human summary. `setup` is
/// `Some` when the optional workspace materialization ran (#65y), reported as its own block/line.
fn render(
    json: bool,
    command: &str,
    workspace: Option<&str>,
    setup: Option<&SetupOutcome>,
    outcomes: &[HostOutcome],
) {
    if json {
        let hosts: Vec<Value> = outcomes
            .iter()
            .map(|o| {
                let mut rec = json!({
                    "host": o.host.id(),
                    "label": o.host.label(),
                    "path": o.path.to_string_lossy(),
                    "status": o.result.as_str(),
                });
                if let HostResult::Failed(msg) = &o.result {
                    rec["error"] = json!(msg);
                }
                rec
            })
            .collect();
        let out = json!({
            "server": SERVER_NAME,
            "command": command,
            "workspace": workspace,
            // `null` when no `--setup` ran; else the seated plugin + the (validated UTF-8) root the
            // board lives under.
            "setup": setup.map(|s| json!({
                "plugin": s.plugin,
                "workspace": s.workspace,
            })),
            "hosts": hosts,
        });
        println!("{out}");
        return;
    }

    println!(
        "Registered `{SERVER_NAME}` MCP server → {command}{}",
        match workspace {
            Some(ws) => format!(" (workspace {ws})"),
            None => String::new(),
        }
    );
    if let Some(s) = setup {
        println!("Set up workspace → {} (plugin {})", s.workspace, s.plugin);
    }
    let width = outcomes
        .iter()
        .map(|o| o.host.label().len())
        .max()
        .unwrap_or(0);
    for o in outcomes {
        let mark = match o.result {
            HostResult::Written => "✓",
            HostResult::Unchanged => "·",
            HostResult::Skipped => "–",
            HostResult::Failed(_) => "✗",
        };
        let note = match &o.result {
            HostResult::Skipped => "not detected".to_string(),
            HostResult::Failed(msg) => msg.clone(),
            _ => o.path.display().to_string(),
        };
        println!(
            "  {mark} {:width$}  {:<9}  {note}",
            o.host.label(),
            o.result.as_str()
        );
    }
    if outcomes.iter().all(|o| o.result == HostResult::Skipped) {
        println!(
            "\nNo MCP hosts detected. Install one (e.g. Claude Desktop), or target it explicitly \
             with `--host <id>`."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn base_at(root: &Path) -> BaseDirs {
        BaseDirs {
            home: root.to_path_buf(),
            config: root.join("config"),
        }
    }

    // ---- server_entry --------------------------------------------------------------------------

    #[test]
    fn server_entry_without_workspace_is_command_plus_mcp_serve() {
        let entry = server_entry("/opt/suite/nxs", None);
        assert_eq!(
            entry,
            json!({ "command": "/opt/suite/nxs", "args": ["mcp", "serve"] })
        );
    }

    #[test]
    fn server_entry_with_workspace_pins_it_in_args() {
        let entry = server_entry("/opt/suite/nxs", Some("/proj"));
        assert_eq!(
            entry,
            json!({ "command": "/opt/suite/nxs", "args": ["mcp", "serve", "--workspace", "/proj"] })
        );
    }

    // ---- Runner (native | npx) -----------------------------------------------------------------

    #[test]
    fn runner_native_entry_is_the_absolute_nxs_plus_mcp_serve() {
        // Native ignores the runner indirection: same entry as `server_entry` directly.
        assert_eq!(
            Runner::Native.entry("/opt/suite/nxs", Some("/proj")),
            json!({ "command": "/opt/suite/nxs", "args": ["mcp", "serve", "--workspace", "/proj"] })
        );
    }

    #[test]
    fn runner_npx_entry_references_the_shim_and_ignores_the_local_binary() {
        // The `npx @nexus-flow/mcp` shim resolves/verifies/execs nxs itself, so the abs path is
        // irrelevant; host args ride after `--` (the shim prepends `mcp serve`).
        assert_eq!(
            Runner::Npx.entry("/opt/suite/nxs", Some("/proj")),
            json!({ "command": "npx", "args": ["-y", "@nexus-flow/mcp", "--", "--workspace", "/proj"] })
        );
    }

    #[test]
    fn runner_npx_entry_without_workspace_drops_the_trailing_separator() {
        assert_eq!(
            Runner::Npx.entry("/opt/suite/nxs", None),
            json!({ "command": "npx", "args": ["-y", "@nexus-flow/mcp"] })
        );
    }

    #[test]
    fn runner_parse_accepts_native_and_npx_and_rejects_others() {
        assert_eq!(Runner::parse("native").unwrap(), Runner::Native);
        assert_eq!(Runner::parse("npx").unwrap(), Runner::Npx);
        let err = Runner::parse("wget").unwrap_err();
        assert!(
            err.msg.contains("unknown --runner"),
            "names the bad runner: {}",
            err.msg
        );
    }

    #[test]
    fn runner_display_summarizes_the_launch_command() {
        assert_eq!(Runner::Native.display("/opt/suite/nxs"), "/opt/suite/nxs");
        assert_eq!(
            Runner::Npx.display("/opt/suite/nxs"),
            "npx -y @nexus-flow/mcp"
        );
    }

    // ---- upsert_entry --------------------------------------------------------------------------

    #[test]
    fn upsert_adds_the_entry_when_absent_and_reports_changed() {
        let entry = server_entry("/opt/nxs", None);
        let mut config = json!({});
        assert!(
            upsert_entry(&mut config, &entry).unwrap(),
            "adding is a change"
        );
        assert_eq!(config["mcpServers"]["nxs"], entry);
    }

    #[test]
    fn upsert_is_a_noop_when_the_entry_already_matches() {
        let entry = server_entry("/opt/nxs", None);
        let mut config = json!({ "mcpServers": { "nxs": entry.clone() } });
        assert!(
            !upsert_entry(&mut config, &entry).unwrap(),
            "re-upserting the same entry is not a change (idempotent)"
        );
    }

    #[test]
    fn upsert_replaces_a_stale_entry() {
        let mut config = json!({
            "mcpServers": { "nxs": { "command": "/old/nxs", "args": ["mcp", "serve"] } }
        });
        let entry = server_entry("/new/nxs", None);
        assert!(
            upsert_entry(&mut config, &entry).unwrap(),
            "a changed command is a change"
        );
        assert_eq!(config["mcpServers"]["nxs"], entry);
    }

    #[test]
    fn upsert_preserves_other_servers_and_top_level_keys() {
        let mut config = json!({
            "theme": "dark",
            "mcpServers": { "other": { "command": "/bin/other", "args": [] } }
        });
        let entry = server_entry("/opt/nxs", None);
        upsert_entry(&mut config, &entry).unwrap();
        assert_eq!(config["theme"], json!("dark"));
        assert_eq!(
            config["mcpServers"]["other"],
            json!({ "command": "/bin/other", "args": [] })
        );
        assert_eq!(config["mcpServers"]["nxs"], entry);
    }

    #[test]
    fn upsert_creates_a_genuinely_absent_mcpservers() {
        // The one coercion we keep: an object root with NO `mcpServers` key gets the map created,
        // and sibling keys survive.
        let mut config = json!({ "theme": "dark" });
        let entry = server_entry("/opt/nxs", None);
        assert!(upsert_entry(&mut config, &entry).unwrap());
        assert_eq!(config["theme"], json!("dark"));
        assert_eq!(config["mcpServers"]["nxs"], entry);
    }

    #[test]
    fn upsert_errors_on_a_non_object_root() {
        // A present-but-non-object root is refused loudly, NOT silently discarded (Integrity #2).
        let mut config = json!([1, 2, 3]);
        let entry = server_entry("/opt/nxs", None);
        let err = upsert_entry(&mut config, &entry).unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
    }

    #[test]
    fn upsert_errors_on_a_non_object_mcpservers_leaving_it_intact() {
        // A present-but-non-object `mcpServers` is refused loudly and left as-is (not coerced to {}).
        let mut config = json!({ "mcpServers": 5 });
        let entry = server_entry("/opt/nxs", None);
        let err = upsert_entry(&mut config, &entry).unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
        assert_eq!(
            config["mcpServers"],
            json!(5),
            "the offending value is untouched"
        );
    }

    // ---- config_path ---------------------------------------------------------------------------

    #[test]
    fn config_path_resolves_each_host_under_its_root() {
        let base = base_at(Path::new("/u"));
        assert_eq!(
            Host::ClaudeDesktop.config_path(&base),
            Path::new("/u/config/Claude/claude_desktop_config.json")
        );
        assert_eq!(
            Host::Cursor.config_path(&base),
            Path::new("/u/.cursor/mcp.json")
        );
        assert_eq!(
            Host::Windsurf.config_path(&base),
            Path::new("/u/.codeium/windsurf/mcp_config.json")
        );
        // Amazon Quick (formerly Amazon Q) reads the same `mcpServers` schema from its global CLI
        // config under `$HOME/.aws/amazonq/mcp.json` (5kky).
        assert_eq!(
            Host::AmazonQuick.config_path(&base),
            Path::new("/u/.aws/amazonq/mcp.json")
        );
    }

    // ---- is_present ----------------------------------------------------------------------------

    #[test]
    fn is_present_is_true_when_the_parent_dir_exists() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".cursor");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(
            is_present(&dir.join("mcp.json")),
            "parent dir present ⇒ installed"
        );
    }

    #[test]
    fn is_present_is_false_when_neither_file_nor_parent_exists() {
        let tmp = TempDir::new().unwrap();
        assert!(!is_present(&tmp.path().join("nope").join("mcp.json")));
    }

    // ---- resolve_selected ----------------------------------------------------------------------

    #[test]
    fn resolve_selected_empty_stays_empty() {
        assert_eq!(resolve_selected(&[]).unwrap(), Vec::<Host>::new());
    }

    #[test]
    fn resolve_selected_maps_known_ids() {
        let sel = resolve_selected(&["cursor".to_string(), "claude-desktop".to_string()]).unwrap();
        assert_eq!(sel, vec![Host::Cursor, Host::ClaudeDesktop]);
    }

    #[test]
    fn resolve_selected_maps_amazon_quick() {
        // The new host id round-trips through `--host` (5kky).
        let sel = resolve_selected(&["amazon-quick".to_string()]).unwrap();
        assert_eq!(sel, vec![Host::AmazonQuick]);
    }

    #[test]
    fn resolve_selected_rejects_an_unknown_id_naming_it() {
        let err = resolve_selected(&["zed".to_string()]).unwrap_err();
        assert!(
            err.msg.contains("zed"),
            "error names the bad id: {}",
            err.msg
        );
    }

    // ---- apply_host ----------------------------------------------------------------------------

    #[test]
    fn apply_host_writes_then_is_unchanged_on_rerun() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".cursor").join("mcp.json");
        let entry = server_entry("/opt/nxs", None);

        assert_eq!(apply_host(&path, &entry, true).unwrap(), Status::Written);
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["mcpServers"]["nxs"], entry);

        assert_eq!(
            apply_host(&path, &entry, true).unwrap(),
            Status::Unchanged,
            "second identical apply writes nothing new"
        );
    }

    #[test]
    fn apply_host_treats_an_empty_file_as_fresh() {
        // A present-but-empty/whitespace file is NOT corrupt — it becomes a fresh `{}` and gets the
        // entry written (the `Ok(_) => json!({})` branch).
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("mcp.json");
        std::fs::write(&path, "   \n").unwrap();
        let entry = server_entry("/opt/nxs", None);
        assert_eq!(apply_host(&path, &entry, true).unwrap(), Status::Written);
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["mcpServers"]["nxs"], entry);
    }

    #[test]
    fn apply_host_errors_on_malformed_json_leaving_bytes_untouched() {
        // The load-bearing guard: a corrupt config is a loud error, and — critically — the user's
        // file is left byte-for-byte intact (never truncated or clobbered). This is the test whose
        // absence would let a refactor to `unwrap_or_default()` silently wipe a real config.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("mcp.json");
        let original = b"{ this is not valid json";
        std::fs::write(&path, original).unwrap();

        let err = apply_host(&path, &server_entry("/opt/nxs", None), true).unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            original,
            "a corrupt config is never overwritten"
        );
    }

    #[test]
    fn apply_host_errors_on_a_non_object_root_leaving_bytes_untouched() {
        // Valid JSON but not an object (an array) — the more dangerous silent-discard case. Loud
        // error, file untouched.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("mcp.json");
        let original = br#"["valid json array, but not an object"]"#;
        std::fs::write(&path, original).unwrap();

        let err = apply_host(&path, &server_entry("/opt/nxs", None), true).unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
        assert_eq!(std::fs::read(&path).unwrap(), &original[..]);
    }

    #[test]
    fn apply_host_leaves_no_temp_sibling_behind() {
        // The atomic temp+rename must not strand its sibling temp file in the config dir.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("mcp.json");
        apply_host(&path, &server_entry("/opt/nxs", None), true).unwrap();
        let leftovers: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("nxs.tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp sibling cleaned up: {leftovers:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn apply_host_refuses_a_pre_placed_temp_symlink_and_leaves_the_target_untouched() {
        // TOCTOU/symlink hardening (0jq8 Integrity #1 / Test #2), the install-side instance: a symlink
        // squatting on `atomic_write`'s predictable temp path must NOT be followed (O_EXCL via
        // `write_new_exclusive`) — its target stays byte-for-byte intact and no host config lands.
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("mcp.json");
        let outside = tmp.path().join("precious.txt");
        std::fs::write(&outside, b"PRECIOUS").unwrap();
        let temp_path = tmp
            .path()
            .join(format!(".mcp.json.nxs.tmp.{}", std::process::id()));
        symlink(&outside, &temp_path).unwrap();

        let err = apply_host(&cfg, &server_entry("/opt/nxs", None), true).unwrap_err();
        assert_eq!(
            err.kind.as_str(),
            "io",
            "a squatted temp path is a loud io error"
        );
        assert_eq!(
            std::fs::read(&outside).unwrap(),
            b"PRECIOUS",
            "the symlink target is never written through"
        );
    }

    // ---- install_into --------------------------------------------------------------------------

    #[test]
    fn install_into_autodetect_writes_present_and_skips_absent() {
        let tmp = TempDir::new().unwrap();
        let base = base_at(tmp.path());
        // Cursor "installed": create its dotdir. Claude + Windsurf absent.
        std::fs::create_dir_all(tmp.path().join(".cursor")).unwrap();

        let out = install_into(&base, &server_entry("/opt/nxs", None), &[]).unwrap();
        let result = |h: Host| out.iter().find(|o| o.host == h).unwrap().result.clone();
        assert_eq!(result(Host::Cursor), HostResult::Written);
        assert_eq!(result(Host::ClaudeDesktop), HostResult::Skipped);
        assert_eq!(result(Host::Windsurf), HostResult::Skipped);
        // Amazon Quick absent too → skipped, and it is part of the scanned set (5kky).
        assert_eq!(result(Host::AmazonQuick), HostResult::Skipped);
    }

    #[test]
    fn install_into_autodetect_writes_amazon_quick_when_present() {
        let tmp = TempDir::new().unwrap();
        let base = base_at(tmp.path());
        // "Amazon Quick installed": create its `.aws/amazonq` config dir.
        std::fs::create_dir_all(tmp.path().join(".aws").join("amazonq")).unwrap();

        let out = install_into(&base, &server_entry("/opt/nxs", None), &[]).unwrap();
        let quick = out.iter().find(|o| o.host == Host::AmazonQuick).unwrap();
        assert_eq!(quick.result, HostResult::Written);
        // The entry landed at the Amazon Quick global config path.
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&quick.path).unwrap()).unwrap();
        assert_eq!(written["mcpServers"]["nxs"], server_entry("/opt/nxs", None));
    }

    #[test]
    fn install_into_rerun_is_all_unchanged() {
        let tmp = TempDir::new().unwrap();
        let base = base_at(tmp.path());
        std::fs::create_dir_all(tmp.path().join(".cursor")).unwrap();

        install_into(&base, &server_entry("/opt/nxs", None), &[]).unwrap();
        let out = install_into(&base, &server_entry("/opt/nxs", None), &[]).unwrap();
        let cursor = out.iter().find(|o| o.host == Host::Cursor).unwrap();
        assert_eq!(cursor.result, HostResult::Unchanged, "re-run is idempotent");
    }

    #[test]
    fn install_into_explicit_host_force_writes_when_absent() {
        let tmp = TempDir::new().unwrap();
        let base = base_at(tmp.path());
        // Nothing pre-created; an explicit --host must still write (create dirs).
        let out = install_into(&base, &server_entry("/opt/nxs", None), &[Host::Windsurf]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].host, Host::Windsurf);
        assert_eq!(out[0].result, HostResult::Written);
        assert!(Host::Windsurf.config_path(&base).exists());
    }

    #[test]
    fn install_into_pins_the_workspace_in_the_written_entry() {
        let tmp = TempDir::new().unwrap();
        let base = base_at(tmp.path());
        install_into(
            &base,
            &server_entry("/opt/nxs", Some("/proj")),
            &[Host::Cursor],
        )
        .unwrap();
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(Host::Cursor.config_path(&base)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            written["mcpServers"]["nxs"]["args"],
            json!(["mcp", "serve", "--workspace", "/proj"])
        );
    }

    #[test]
    fn install_into_autodetect_continues_past_a_corrupt_host() {
        // One corrupt host config must NOT strand the healthy hosts' registration (CQ #2). This
        // matters more now that a bad file errors loudly (Integrity #2) instead of silently coercing.
        let tmp = TempDir::new().unwrap();
        let base = base_at(tmp.path());
        // Cursor "installed" but corrupt; Windsurf "installed" and healthy.
        let cursor_cfg = tmp.path().join(".cursor").join("mcp.json");
        std::fs::create_dir_all(cursor_cfg.parent().unwrap()).unwrap();
        std::fs::write(&cursor_cfg, "{ corrupt").unwrap();
        std::fs::create_dir_all(tmp.path().join(".codeium").join("windsurf")).unwrap();

        let out = install_into(&base, &server_entry("/opt/nxs", None), &[]).unwrap();
        let result = |h: Host| out.iter().find(|o| o.host == h).unwrap().result.clone();
        assert!(
            matches!(result(Host::Cursor), HostResult::Failed(_)),
            "the corrupt host is recorded as an error, not fatal"
        );
        assert_eq!(
            result(Host::Windsurf),
            HostResult::Written,
            "the healthy host is still registered despite the corrupt sibling"
        );
        assert_eq!(
            std::fs::read_to_string(&cursor_cfg).unwrap(),
            "{ corrupt",
            "the corrupt file is left untouched"
        );
    }

    #[test]
    fn install_into_explicit_host_propagates_a_corrupt_config_error() {
        // An explicitly named `--host` fails loudly on a corrupt config (the user asked for exactly
        // that target) rather than swallowing it into a per-host status.
        let tmp = TempDir::new().unwrap();
        let base = base_at(tmp.path());
        let cursor_cfg = tmp.path().join(".cursor").join("mcp.json");
        std::fs::create_dir_all(cursor_cfg.parent().unwrap()).unwrap();
        std::fs::write(&cursor_cfg, "{ corrupt").unwrap();

        let err =
            install_into(&base, &server_entry("/opt/nxs", None), &[Host::Cursor]).unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
    }

    // ---- command_str ---------------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn command_str_rejects_a_non_utf8_path() {
        // A non-UTF-8 binary path is a loud error, not a U+FFFD-mangled `command` no host can launch.
        use std::os::unix::ffi::OsStrExt;
        let bad = std::ffi::OsStr::from_bytes(&[0x2f, 0xff, 0x6e, 0x78, 0x73]); // "/\xffnxs"
        assert!(command_str(Path::new(bad)).is_err());
    }

    // ---- resolve_plugin (#65y) -----------------------------------------------------------------

    #[test]
    fn resolve_plugin_defaults_to_personal_todo() {
        // The install-time setup default is personal-todo (the knowledge-worker board), NOT flow's
        // own issue-tracker default — the coding case opts in with `--plugin issue-tracker`.
        assert_eq!(resolve_plugin(None).unwrap(), "personal-todo");
    }

    #[test]
    fn resolve_plugin_accepts_the_known_plugins() {
        assert_eq!(
            resolve_plugin(Some("issue-tracker")).unwrap(),
            "issue-tracker"
        );
        assert_eq!(
            resolve_plugin(Some("personal-todo")).unwrap(),
            "personal-todo"
        );
    }

    #[test]
    fn resolve_plugin_rejects_an_unknown_plugin_naming_it() {
        // A typo'd plugin fails LOUDLY here — before any board is materialized — rather than seating
        // an unloadable plugin the server only chokes on at connect time.
        let err = resolve_plugin(Some("bogus")).unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
        assert!(
            err.msg.contains("bogus"),
            "error names the bad plugin: {}",
            err.msg
        );
    }

    // ---- run_setup_in (#65y) -------------------------------------------------------------------

    #[test]
    fn run_setup_in_materializes_the_app_data_home_when_no_workspace() {
        // No explicit --workspace → the target is the injected App-Data-Home; a `.nxs/` is created
        // there seated with the chosen plugin, and the outcome names that root.
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();
        let outcome = run_setup_in(None, "personal-todo", || Ok(home.clone())).unwrap();
        assert_eq!(outcome.workspace, home.to_str().unwrap());
        assert_eq!(outcome.plugin, "personal-todo");
        assert!(
            home.join(".nxs").join("config.toml").is_file(),
            "board materialized under the App-Data-Home"
        );
        let ws = nexus_flow_facade::workspace::discover(&home).unwrap();
        assert_eq!(
            nexus_flow_facade::workspace::flow_plugin(&ws.config),
            "personal-todo"
        );
    }

    #[test]
    fn run_setup_in_targets_the_explicit_workspace_without_consulting_app_data_home() {
        // An explicit --workspace pins the target; the App-Data-Home resolver must NOT run (proven by
        // panicking in it) — init-target == server-resolution for the pinned path (the #65y trap).
        let tmp = TempDir::new().unwrap();
        let ws_dir = tmp.path().join("proj");
        std::fs::create_dir_all(&ws_dir).unwrap();
        let outcome = run_setup_in(Some(ws_dir.to_str().unwrap()), "issue-tracker", || {
            panic!("App-Data-Home resolver must not be consulted when --workspace is given")
        })
        .unwrap();
        assert_eq!(outcome.workspace, ws_dir.to_str().unwrap());
        let ws = nexus_flow_facade::workspace::discover(&ws_dir).unwrap();
        assert_eq!(
            nexus_flow_facade::workspace::flow_plugin(&ws.config),
            "issue-tracker"
        );
    }

    #[test]
    fn run_setup_in_is_idempotent_and_never_clobbers_an_existing_board() {
        // A second setup — even with a DIFFERENT plugin — loads the existing board unchanged (the
        // foundation `setup` contract), so a re-run of `install --setup` can never re-seat the plugin.
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();
        run_setup_in(None, "personal-todo", || Ok(home.clone())).unwrap();
        let second = run_setup_in(None, "issue-tracker", || Ok(home.clone())).unwrap();
        let ws = nexus_flow_facade::workspace::discover(&home).unwrap();
        assert_eq!(
            nexus_flow_facade::workspace::flow_plugin(&ws.config),
            "personal-todo",
            "the first plugin survives — setup never clobbers"
        );
        // The outcome must report the ACTUALLY-seated plugin (read back from disk), not the one the
        // second call requested — otherwise the install report would claim `issue-tracker` for a
        // board that is really personal-todo.
        assert_eq!(
            second.plugin, "personal-todo",
            "the outcome reports the seated plugin, not the requested one"
        );
    }

    // ---- absolutize_workspace (#65y) -----------------------------------------------------------

    #[test]
    fn absolutize_workspace_leaves_none_and_absolute_paths_alone() {
        assert_eq!(absolutize_workspace(None).unwrap(), None);
        assert_eq!(
            absolutize_workspace(Some("/proj/x")).unwrap().as_deref(),
            Some("/proj/x"),
            "an already-absolute path passes through unchanged"
        );
    }

    #[test]
    fn absolutize_workspace_resolves_a_relative_path_against_cwd() {
        // A relative --workspace is made absolute against the install-process cwd, so the pinned
        // entry + setup target are one absolute path the host can't re-resolve elsewhere (#65y trap).
        let abs = absolutize_workspace(Some("rel/sub")).unwrap().unwrap();
        let abs = Path::new(&abs);
        assert!(abs.is_absolute(), "made absolute: {}", abs.display());
        assert!(
            abs.ends_with("rel/sub"),
            "keeps the relative tail: {}",
            abs.display()
        );
        assert!(
            abs.starts_with(std::env::current_dir().unwrap()),
            "rooted at the install cwd: {}",
            abs.display()
        );
    }
}
