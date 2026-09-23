//! `nxs sync` (spec §6.3) — stream binding + one client-driven push/pull pass over the ONE shared
//! op-log. Relocated from `nxf sync` (aye.2.1): with a single shared log, sync is a **platform**
//! operation, not a flow-module one, so the verb lives on the umbrella.
//!
//! Binding is the explicit create-vs-join step (§4.2), persisted in `.nxs/sync.toml` — pure
//! filesystem state, no product knowledge. `run` drives the client anti-entropy engine
//! ([`nxs_sync::engine`]) over the shared db.
//!
//! INTERIM coupling: `run` opens flow's store (`nexus_flow_core::store::Store`) so the engine's
//! prefix-collision remap + the on-pull fold of `task` ops keep working exactly as `nxf sync` did.
//! The op-log itself syncs correctly for EVERY domain (foreign-domain ops ride the wire and land in
//! the log). A pull that lands a foreign product's ops (e.g. `fact`) does NOT fold them here — flow
//! registers only the task reducer — but that is no longer a gap: each product store now refolds
//! its own views on open when the shared log advanced past its folded-through watermark
//! (aye.36, `Store::refold_if_behind`). So `nxs sync` reusing flow's store as the op-log driver is
//! a documented driver choice, not a correctness compromise.
//!
//! The verb bodies live here; the pieces each have their own file:
//! - [`slug`]: git remote -> canonical slug -> `stream_id`, plus the id charset gate.
//! - [`endpoint`]: the global default and the precedence that resolves one endpoint.
//! - [`daemon`]: the trigger channel, the pure scheduler, the loop.
//! - [`launchd`]: the macOS agent lifecycle.

pub mod daemon;
pub mod endpoint;
pub mod launchd;
pub mod slug;
pub mod trust;

use crate::error::{ErrorKind, NxfError, Result};
use nexus_flow_core::store::Store;
use nxs_foundation::workspace::{self, Workspace};
use nxs_sync::engine::{self, Announced, HttpTransport, Transport, Watermarks};
use nxs_sync::presence::Presence;
use nxs_sync::protocol::{MachineHello, StreamId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const SYNC_META_FILE: &str = "sync.toml";

/// One pull page size — the client follows `next` until the stream is exhausted, so this only
/// bounds each round-trip, never the total synced.
const SYNC_PAGE_LIMIT: usize = 500;

/// Persisted sync state for a workspace: the bound stream plus the two anti-entropy watermarks
/// (§4.3). The watermark fields default, so a bind-only meta (no run yet) still parses.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SyncMeta {
    pub stream_id: String,
    /// Local insertion watermark = highest pushed `ops.rowid`.
    #[serde(default)]
    pub pushed_through: i64,
    /// Server cursor watermark = highest pulled sequence.
    #[serde(default)]
    pub pulled_through: i64,
    /// Where this workspace syncs to — the per-workspace override in the precedence of
    /// [`endpoint::resolve`]. `None` falls back to the global default. Additive and
    /// defaulted, so a pre-kgn5 bind-only meta still parses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

fn meta_path(ws: &Workspace) -> PathBuf {
    ws.dir.join(SYNC_META_FILE)
}

/// Whether this workspace is bound to a sync stream (the mere presence of `.nxs/sync.toml`).
pub fn is_bound(ws: &Workspace) -> bool {
    meta_path(ws).exists()
}

/// Load the bound stream meta, or `None` if the workspace was never bound.
pub fn load(ws: &Workspace) -> Result<Option<SyncMeta>> {
    let path = meta_path(ws);
    match std::fs::read_to_string(&path) {
        Ok(raw) => {
            // A TOML error quotes the offending line — and the endpoint line can carry a relay's
            // key, on its way into the service log and the heartbeat (review of PR #487).
            let meta: SyncMeta = toml::from_str(&raw).map_err(|e| {
                NxfError::io(nxs_sync::redact::redact_userinfo(&format!(
                    "parsing {SYNC_META_FILE}: {e}"
                )))
            })?;
            Ok(Some(meta))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(NxfError::io(format!("reading {SYNC_META_FILE}: {e}"))),
    }
}

/// Persist sync meta (binding + watermarks). Routed through [`crate::workspaces::write_atomic`]
/// (review finding Integrity #1, PR #263): `sync.toml` is written on every bind and after every
/// sync pass, and a plain `fs::write` follows symlinks — a symlink planted at this path (e.g. by
/// a cloned repo) would turn an ordinary `nxs sync bind`/`run` into a write/truncate of whatever
/// it points at. Same hardening the registry, the endpoint config and the heartbeat already use.
pub fn save(ws: &Workspace, meta: &SyncMeta) -> Result<()> {
    let raw =
        toml::to_string(meta).map_err(|e| NxfError::io(format!("serializing sync meta: {e}")))?;
    // Owner-only: the endpoint it stores can carry a relay's key (review of PR #487).
    crate::workspaces::write_atomic_private(&meta_path(ws), raw.as_bytes())
}

/// Reject binding when the workspace is already bound — re-binding would orphan the existing
/// stream's history. Names the current stream so the operator sees what they would clobber.
fn ensure_unbound(ws: &Workspace) -> Result<()> {
    if let Some(meta) = load(ws)? {
        return Err(NxfError::new(
            ErrorKind::Conflict,
            format!(
                "workspace is already bound to stream '{}'; create/join is a one-time step",
                meta.stream_id
            ),
        ));
    }
    Ok(())
}

/// Reject a `stream_id` that would not survive a URL path (nexus-flow p5sa). Applied at
/// every BIND path — never on load, which would break existing working bindings.
fn ensure_valid_stream_id(id: &str) -> Result<()> {
    if slug::is_valid_stream_id(id) {
        return Ok(());
    }
    let safe: String = id
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Err(NxfError::validation(format!(
        "stream id {id:?} must match [a-z0-9_.-]+ — other characters are mangled by URL \
         path normalization (a '/' turns one segment into several and the relay returns \
         404). Try {safe:?}",
    )))
}

/// First replica: mint a fresh, durable `stream_id` and persist it.
fn bind_create(ws: &Workspace) -> Result<SyncMeta> {
    ensure_unbound(ws)?;
    let stream_id = format!(
        "stream-{}",
        ulid::Ulid::new().to_string().to_ascii_lowercase()
    );
    ensure_valid_stream_id(&stream_id)?;
    let meta = SyncMeta {
        stream_id,
        ..Default::default()
    };
    save(ws, &meta)?;
    Ok(meta)
}

/// Every other device: bind to the existing shared `stream_id` verbatim — no new id.
fn bind_join(ws: &Workspace, stream_id: &str) -> Result<SyncMeta> {
    ensure_unbound(ws)?;
    if stream_id.trim().is_empty() {
        return Err(NxfError::validation("stream id to join must not be empty"));
    }
    ensure_valid_stream_id(stream_id)?;
    let meta = SyncMeta {
        stream_id: stream_id.to_string(),
        ..Default::default()
    };
    save(ws, &meta)?;
    Ok(meta)
}

/// The default bind path (spec §4.1): derive the stream from this workspace's git remote,
/// so every clone lands on the same stream with no id to copy around.
fn bind_derived(ws: &Workspace) -> Result<SyncMeta> {
    ensure_unbound(ws)?;
    // The repo root, not `.nxs/` — `ws.dir` IS the `.nxs` directory.
    let root = ws.dir.parent().unwrap_or(&ws.dir);
    let stream_id = slug::origin_remote(root)
        .and_then(|url| slug::stream_id_from_remote(&url))
        .ok_or_else(|| {
            NxfError::validation(
                "cannot derive a stream id: no usable `origin` remote here. Use \
                 `--create` (first replica) or `--join <stream_id>` (every other device)",
            )
        })?;
    ensure_valid_stream_id(&stream_id)?;
    let meta = SyncMeta {
        stream_id,
        ..Default::default()
    };
    save(ws, &meta)?;
    Ok(meta)
}

/// The id this invocation WOULD bind, without touching anything — used only to classify a
/// request against an already-bound workspace as idempotent (same id), a switch (different
/// id), or, for `--create`, always-a-switch (a fresh id is unknowable ahead of minting it).
fn intended_stream_id(ws: &Workspace, create: bool, join: Option<&str>) -> Result<String> {
    match (create, join) {
        (false, Some(id)) => Ok(id.to_string()),
        (false, None) => {
            let root = ws.dir.parent().unwrap_or(&ws.dir);
            slug::origin_remote(root)
                .and_then(|url| slug::stream_id_from_remote(&url))
                .ok_or_else(|| {
                    NxfError::validation(
                        "cannot derive a stream id: no usable `origin` remote here. Use \
                         `--create` or `--join <stream_id>`",
                    )
                })
        }
        // `--create` mints a fresh id by definition, so it can never be an idempotent
        // re-bind; it is always a switch and needs --rebind on a bound workspace.
        (true, _) => Ok(String::new()),
    }
}

/// Render the "switch to" id for the conflict message. `--create`'s destination is not known
/// ahead of `bind_create` actually minting it (`intended_stream_id` reports it as `""`), so
/// describe that case in words instead of printing an empty pair of quotes.
fn describe_intended(intended: &str) -> String {
    if intended.is_empty() {
        "a freshly minted id".to_string()
    } else {
        format!("'{intended}'")
    }
}

/// The flags `nxs sync bind` accepts. A struct rather than a parameter list because later
/// slices add more flags (`--no-daemon`), and each addition would otherwise churn every call
/// site and test.
pub struct BindOptions<'a> {
    pub create: bool,
    pub join: Option<&'a str>,
    pub endpoint: Option<&'a str>,
    /// Switch an already-bound workspace to a different stream (§4.4): re-pushes the local log
    /// to the new stream and resets both watermarks. Ignored (has no effect) on a first bind —
    /// there is nothing to switch from.
    pub rebind: bool,
    /// Skip the best-effort launchd autostart a successful bind would otherwise attempt (task 13,
    /// `x1c2` DoD). For CI/tests/environments where a background daemon is unwanted, or where
    /// `nxs sync run` will be driven manually instead.
    pub no_daemon: bool,
    /// Start this fresh workspace from a snapshot file (6j6v.mxt2) — see [`bind_from_snapshot`].
    pub snapshot: Option<&'a std::path::Path>,
}

/// Starts the sync daemon as the best-effort side effect of a successful bind (task 13, `x1c2`
/// DoD: binding a workspace starts continuous sync). A trait — not a bare `fn` — so a test's spy
/// can both RECORD whether it was invoked AND be told to FAIL, which is what proves all three
/// required properties: a successful bind calls it, `--no-daemon` suppresses the call entirely,
/// and a failing installer still yields a successful bind. Private: nothing outside this module
/// needs the seam, only `finish_bind`'s single production call site and this file's own tests.
trait Autostart {
    fn start(&self) -> Result<()>;
}

/// Production autostart: delegates to [`launchd::install_quiet`] — the real launchd install on
/// macOS, a loud "unsupported platform" `Err` elsewhere. Either way `finish_bind_with` treats the
/// result exactly like the registry upsert below: a warning, never a reason to fail the bind.
struct RealAutostart;

impl Autostart for RealAutostart {
    fn start(&self) -> Result<()> {
        launchd::install_quiet()
    }
}

/// Registers a bound workspace into this service instance's `workspaces.toml` as the best-effort
/// side effect of a successful bind — same shape and reasoning as [`Autostart`]: a trait, not a
/// bare call to [`crate::workspaces::register_workspace`], so a test can substitute a spy that
/// never touches the real registry file. This closes the SAME class of leak the autostart seam
/// closed, one level deeper: `crate::workspaces::register_workspace` resolves the REAL registry —
/// `~/.nexusflow/workspaces.toml`, or `~/.nexusflow-<name>/` for a development build that
/// `NXS_SERVICE_INSTANCE` names since nxf 6j6v.gd9p — and that variable is not an override a TEST
/// can use: it selects which real service's registry is written, not whether a real one is. So, exactly as before, an in-process
/// unit test cannot isolate this the way the subprocess CLI tests do by faking `$HOME` — every
/// unit test that reached this call was writing into the developer's real registry (measured at 98
/// stale entries on one machine, task 13 follow-up).
trait Registrar {
    fn register(&self, path: &str) -> Result<bool>;
}

/// Production registrar: the upsert into whichever instance's registry this process resolves.
struct RealRegistrar;

impl Registrar for RealRegistrar {
    fn register(&self, path: &str) -> Result<bool> {
        let changed = crate::workspaces::register_workspace(path)?;
        // **The moment an overlap is created is the moment to say so** (nxf 6j6v.gd9p). Since a
        // machine can run several service instances, this bind may have just made a workspace the
        // second one's too — and two services attending one workspace both read its deadline book.
        // Here rather than in `register_workspace`, for this seam's own reason: the spy in tests
        // must not reach the developer's real registry, and only the production registrar does.
        launchd::warn_if_attended_by_more_than_one();
        // **And which service it landed in, when the environment named another** (nxf 6j6v.cvpy,
        // review of PR #441). A bind DECIDES which service attends this workspace, so the one
        // moment it is worth saying that `NXS_SERVICE_INSTANCE` had no vote is this one — before
        // this, `install`/`uninstall` said it and the path that writes the registry did not. Same
        // seam, same reason as the line above it.
        if let Some(note) = nxs_service::ignored_instance_env() {
            eprintln!("note: {note}");
        }
        // **What a relay will call this machine** (6j6v.f0b5, review of PR #485, Integrity #8). Binding
        // hands the workspace to the service, and the service's first pass announces this machine
        // under its name — by default the host name — to everyone who can read the stream. Said
        // here, before that happens, rather than only in the guide. Here, not in `finish_bind_with`,
        // for the reason above: the spy registrar must never mint in the developer's real home.
        match nxs_service::ServiceHome::resolve().and_then(|home| home.machine()) {
            Ok(machine) => eprintln!(
                "note: this machine is listed as \"{}\" to anyone who can read this stream at the \
                 relay — `nxs sync machine <name>` renames it",
                machine.name
            ),
            Err(e) => eprintln!("note: this machine has no usable identity yet: {}", e.msg),
        }
        Ok(changed)
    }
}

/// Shared tail for every successful bind outcome — including the idempotent "unchanged" no-op,
/// which returns before the mint/derive/join match in [`bind_with`] but still needs the same
/// best-effort follow-up (endpoint persistence, registry upsert, daemon autostart) and output
/// shape. The autostart AND registrar actions are injected as `autostart`/`registrar` — the seam
/// that makes "bind starts the daemon" / `--no-daemon` / "a failed autostart is still a
/// successful bind" / "bind registers the workspace" all provable without ever installing a real
/// launchd agent or writing into the real workspace registry (task 13, `x1c2`, plus its
/// registry-leak follow-up). [`bind_with`] is the only caller; [`bind`] (production) always
/// passes [`RealAutostart`]/[`RealRegistrar`] through it, while this file's tests call either
/// `finish_bind_with` directly or `bind_with` with spies in their place.
// 8 params: each is a distinct, independently-documented piece of the bind outcome (the two
// trait objects are the whole point — the seam this function exists for) rather than incidental
// clutter a struct would meaningfully collapse; `BindOptions` already carries the CLI-facing
// flags separately, one layer up in `bind_with`.
#[allow(clippy::too_many_arguments)]
fn finish_bind_with(
    json: bool,
    ws: &Workspace,
    meta: SyncMeta,
    mode: &str,
    endpoint: Option<&str>,
    no_daemon: bool,
    autostart: &dyn Autostart,
    registrar: &dyn Registrar,
) -> Result<()> {
    finish_bind_reporting(
        json, ws, meta, mode, endpoint, no_daemon, autostart, registrar, None,
    )
}

/// [`finish_bind_with`], carrying what a bind from a snapshot started from (6j6v.mxt2) into the
/// output — `--json` gains a `snapshot` object only then, so every other bind prints what it
/// always printed.
#[allow(clippy::too_many_arguments)]
fn finish_bind_reporting(
    json: bool,
    ws: &Workspace,
    mut meta: SyncMeta,
    mode: &str,
    endpoint: Option<&str>,
    no_daemon: bool,
    autostart: &dyn Autostart,
    registrar: &dyn Registrar,
    started: Option<&StartedFrom>,
) -> Result<()> {
    // By the time this runs, the binding itself already durably succeeded (the `save` inside
    // the mint/derive/join match, or nothing at all for "unchanged") — once that returns Ok the
    // workspace IS bound, and a retry would hit the conflict/idempotent checks above with no way
    // back except hand-editing `.nxs/sync.toml`. Everything here is best-effort follow-up:
    // reporting a failed bind for a workspace that is in fact durably bound would be worse than
    // a warning, so neither the endpoint re-save nor the registry upsert is propagated with `?`.
    if let Some(url) = endpoint {
        meta.endpoint = Some(url.to_string());
        // Same reasoning as the registry upsert below: the stream binding already succeeded, so
        // failing to persist the endpoint override is a discoverability regression (`sync run`
        // falls back to the global default), not a reason to report the bind itself as failed.
        if let Err(e) = save(ws, &meta) {
            eprintln!(
                "note: bound, but could not persist --endpoint ({e}); `nxs sync run` will fall \
                 back to the global default endpoint until this is fixed"
            );
        }
    }
    // The daemon iterates the registry, so a bound workspace that is not registered would
    // simply never be synced. Idempotent, keyed by path.
    if let Some(root) = ws.dir.parent().and_then(|p| p.to_str()) {
        if let Err(e) = registrar.register(root) {
            eprintln!(
                "note: bound, but could not register the workspace ({e}); \
                 the sync daemon will not see it until you re-run bind"
            );
        }
    }
    // The DoD: binding a workspace starts continuous sync. Best-effort and non-fatal — a
    // workspace that is bound but whose agent could not be installed is still a correctly bound
    // workspace, and `nxs sync daemon install` retries it.
    if !no_daemon {
        if let Err(e) = autostart.start() {
            eprintln!(
                "note: could not start the sync daemon ({e}); run `nxs sync daemon install` to \
                 retry"
            );
        }
    }
    if json {
        // Masked: an idempotent re-run prints the STORED endpoint, key and all, into whatever
        // captures the output (review of PR #487, Integrity #2).
        let mut out = serde_json::json!({
            "ok": true, "mode": mode, "stream_id": meta.stream_id,
            "endpoint": meta.endpoint.as_deref().map(nxs_sync::redact::redact_userinfo),
        });
        if let Some(started) = started {
            out["snapshot"] = started.to_json();
        }
        println!("{out}");
    } else {
        println!("bound stream {} ({mode})", meta.stream_id);
        if let Some(started) = started {
            println!("{}", started.sentence());
        }
    }
    Ok(())
}

/// What a bind from a snapshot started from (6j6v.mxt2) — the report half of
/// [`nxs_sync::snapshot::Imported`].
struct StartedFrom {
    imported: nxs_sync::snapshot::Imported,
    /// Set when the relay reassigned this replica's id prefix before the import.
    reassigned_prefix: Option<String>,
}

impl StartedFrom {
    fn views(&self) -> (&'static str, Option<String>) {
        match &self.imported.loaded {
            nxs_foundation::image::Loaded::Views => ("taken", None),
            nxs_foundation::image::Loaded::Refolded(why) => ("refolded", Some(why.to_string())),
        }
    }

    fn anchor(&self) -> &'static str {
        match self.imported.anchor {
            nxs_sync::snapshot::Anchor::Nothing => "nothing_to_confirm",
            nxs_sync::snapshot::Anchor::Confirmed => "confirmed",
            nxs_sync::snapshot::Anchor::Mismatch => "not_confirmed",
        }
    }

    fn to_json(&self) -> serde_json::Value {
        let (views, refold_reason) = self.views();
        let h = &self.imported.header;
        serde_json::json!({
            "ops": h.ops,
            "taken_by": h.engine,
            "snapshot_through": h.pulled_through,
            "resumes_from": self.imported.marks.pulled_through,
            "views": views,
            "refold_reason": refold_reason,
            "position": self.anchor(),
            "reassigned_prefix": self.reassigned_prefix,
        })
    }

    fn sentence(&self) -> String {
        let h = &self.imported.header;
        let (_, refold_reason) = self.views();
        let folded = match refold_reason {
            None => "its folded board taken as it was".to_string(),
            Some(why) => format!("its board folded again from its log ({why})"),
        };
        let resume = match self.imported.anchor {
            nxs_sync::snapshot::Anchor::Mismatch => format!(
                "the relay does not hold what the snapshot says sits at position {} (another \
                 relay, or a log rebuilt since), so the first pass pulls from the start and skips \
                 every op it already has",
                h.pulled_through
            ),
            _ => format!(
                "the first pass pulls only what came after relay position {}",
                h.pulled_through
            ),
        };
        format!(
            "started from a snapshot taken by nxs {}: {} ops, {folded}; {resume}",
            h.engine, h.ops
        )
    }
}

/// `nxs sync bind --snapshot <file>` (6j6v.mxt2): a NEW machine starts from a snapshot instead of
/// folding the whole history.
///
/// **The import comes BEFORE the binding exists.** `nxs init` already registered the workspace
/// with the background service, which skips it only while it is unbound: the moment
/// `.nxs/sync.toml` appears, a pass may start. So the log is loaded first, and the binding is then
/// written ONCE with everything a pass reads — the stream, both watermarks and the endpoint. A pass
/// that starts after that resumes from the snapshot's position against the right relay; none can
/// start before, because until then the workspace is not bound.
///
/// It joins the stream the snapshot names (a `--join` that names another is refused) and needs a
/// relay to confirm the snapshot's position against (`--endpoint`, or the global default). A
/// refused import writes nothing, so a failed attempt leaves the workspace as it found it.
fn bind_from_snapshot(
    json: bool,
    ws: &Workspace,
    opts: &BindOptions<'_>,
    file: &std::path::Path,
    autostart: &dyn Autostart,
    registrar: &dyn Registrar,
) -> Result<()> {
    if opts.create || opts.rebind {
        return Err(NxfError::validation(
            "--snapshot starts a fresh replica on the stream the snapshot was taken from; it \
             cannot be combined with --create or --rebind",
        ));
    }
    if let Some(meta) = load(ws)? {
        return Err(NxfError::new(
            ErrorKind::Conflict,
            format!(
                "workspace is already bound to stream '{}'; --snapshot starts a fresh replica",
                meta.stream_id
            ),
        ));
    }
    let size = std::fs::metadata(file)
        .map_err(|e| NxfError::io(format!("reading the snapshot {}: {e}", file.display())))?
        .len();
    if size > nxs_sync::snapshot::MAX_UNPACKED_BYTES {
        return Err(NxfError::validation(format!(
            "{} is {size} bytes — larger than any snapshot this nxs reads",
            file.display()
        )));
    }
    let bytes = std::fs::read(file)
        .map_err(|e| NxfError::io(format!("reading the snapshot {}: {e}", file.display())))?;
    let header = nxs_sync::snapshot::peek(&bytes).map_err(snapshot_error)?;
    if let Some(join) = opts.join {
        if join != header.stream_id {
            return Err(NxfError::validation(format!(
                "--join names stream '{join}' but the snapshot belongs to '{}'",
                header.stream_id
            )));
        }
    }
    ensure_valid_stream_id(&header.stream_id)?;
    let path = ws.db_path_str()?;
    let mut store = Store::open(&path, ws.replica.site_id)
        .map_err(|e| NxfError::io(format!("opening store at {path}: {e}")))?;
    let held = store.op_count();
    if held > 0 {
        return Err(NxfError::new(
            ErrorKind::Conflict,
            format!(
                "this workspace already holds {held} ops — a snapshot starts a fresh replica; bind \
                 it without --snapshot and it syncs the ordinary way"
            ),
        ));
    }
    let global = match opts.endpoint {
        Some(_) => None,
        None => endpoint::load_global_from(&endpoint::config_path()?)?,
    };
    let relay = opts
        .endpoint
        .map(str::to_string)
        .or(global)
        .ok_or_else(|| {
            NxfError::validation(
            "--snapshot needs a relay to confirm the snapshot against: pass --endpoint <url> or \
             set a default with `nxs sync endpoint <url>`",
        )
        })?;

    let stream = StreamId(header.stream_id.clone());
    let transport = HttpTransport::new(&relay);
    // Step 0 of any first sync, and it has to come FIRST here too (review of PR #487, Integrity
    // #5): a reassigned prefix remaps every id carrying the old one, which is sound only while all
    // of them are this replica's own — true of the empty store now, not once a foreign log landed.
    let reassigned_prefix = engine::register_prefix(
        &mut store,
        &stream,
        &ws.replica.prefix,
        &ws.replica.replica_uuid,
        &transport,
    )
    .map_err(|e| NxfError::io(format!("prefix registration failed: {e}")))?;
    if let Some(new_prefix) = &reassigned_prefix {
        workspace::adopt_prefix(ws, new_prefix)?;
    }
    let imported = nxs_sync::snapshot::import(&mut store, &bytes, &stream, &transport)
        .map_err(snapshot_error)?;
    drop(store);
    let meta = SyncMeta {
        stream_id: header.stream_id,
        pushed_through: imported.marks.pushed_through,
        pulled_through: imported.marks.pulled_through,
        endpoint: opts.endpoint.map(str::to_string),
    };
    save(ws, &meta)?;
    finish_bind_reporting(
        json,
        ws,
        meta,
        "snapshot",
        opts.endpoint,
        opts.no_daemon,
        autostart,
        registrar,
        Some(&StartedFrom {
            imported,
            reassigned_prefix,
        }),
    )
}

/// A snapshot refusal as the CLI's error, with the kind a caller branches on.
fn snapshot_error(e: nxs_sync::snapshot::SnapshotError) -> NxfError {
    use nxs_foundation::image::ImageError;
    use nxs_sync::snapshot::SnapshotError;
    let kind = match &e {
        SnapshotError::Unpushed { .. }
        | SnapshotError::Unbacked { .. }
        | SnapshotError::Image(ImageError::NotEmpty { .. }) => ErrorKind::Conflict,
        SnapshotError::Storage(_)
        | SnapshotError::Transport(_)
        | SnapshotError::Image(ImageError::Storage(_)) => ErrorKind::Io,
        _ => ErrorKind::Validation,
    };
    NxfError::new(kind, e.to_string())
}

/// `nxs sync snapshot <file>` (6j6v.mxt2): write this workspace's snapshot — the whole log, the
/// folded board and the relay position they reach — for a new machine to start from.
pub fn snapshot_verb(json: bool, ws: &Workspace, file: &std::path::Path) -> Result<()> {
    let meta = load(ws)?.ok_or_else(|| {
        NxfError::validation("workspace is not bound to a stream; run `nxs sync bind` first")
    })?;
    let path = ws.db_path_str()?;
    let store = Store::open(&path, ws.replica.site_id)
        .map_err(|e| NxfError::io(format!("opening store at {path}: {e}")))?;
    let marks = Watermarks {
        pushed_through: meta.pushed_through,
        pulled_through: meta.pulled_through,
    };
    let remote = resolve_endpoint(None, &meta)?;
    let bytes = nxs_sync::snapshot::export(
        &store,
        ws.replica.site_id,
        &StreamId(meta.stream_id.clone()),
        &marks,
        &HttpTransport::new(&remote),
    )
    .map_err(snapshot_error)?;
    // Owner-only: it carries the whole log (review of PR #487, Code Quality #7).
    crate::workspaces::write_atomic_private(file, &bytes)?;
    let ops = nxs_sync::snapshot::peek(&bytes)
        .map_err(snapshot_error)?
        .ops;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "file": file.display().to_string(),
                "bytes": bytes.len(),
                "ops": ops,
                "stream_id": meta.stream_id,
                "snapshot_through": meta.pulled_through,
                "taken_by": nxs_foundation::image::ENGINE_VERSION,
            })
        );
    } else {
        println!(
            "wrote {} ({} bytes): {ops} ops of stream {} through relay position {} — a new machine \
             starts from it with `nxs sync bind --snapshot {}`",
            file.display(),
            bytes.len(),
            meta.stream_id,
            meta.pulled_through,
            file.display()
        );
    }
    Ok(())
}

/// Loud stderr note for the default (derived) bind path — review Integrity #2 (PR #263). The
/// project owner's decision was explicitly NOT to change the derivation (it is the whole point of
/// the zero-flag default: every clone converges with nothing to copy around); the mitigation is
/// making the resulting security posture impossible to miss instead. The relay has no
/// authentication yet (`crates/server/src/app.rs`), and a derived `stream_id` is a pure function
/// of the repo's remote URL — the previous random `--join` ULID acted as a de facto bearer
/// secret, but anyone who can see the remote URL can now compute the same stream id and
/// read/write it on any reachable relay. Never fatal: this is a statement of the current posture,
/// not a reason to fail an otherwise-successful bind. `bind_with` calls this only on the
/// no-`--create`-no-`--join` path — see `derived_path` there for why `--create` (a random,
/// undiscoverable ULID) and `--join` (an id the caller already holds some other way) are exempt.
fn warn_if_derived_bind(derived_path: bool) {
    if derived_path {
        eprintln!(
            "warning: this stream id was derived from the repo's git remote — the sync relay \
             has no authentication yet, so anyone who knows this repository's remote URL can \
             compute the same stream id and read/write it on any reachable relay. Treat the \
             endpoint as private until authentication ships (use `nxs sync bind --create` for an \
             id that cannot be derived from anything public)."
        );
    }
}

/// `nxs sync unregister [<path>]`: take a workspace off the list the background service attends —
/// the counterpart to the registration every successful `bind` performs (6j6v.5zst).
///
/// The registry only ever grew: `bind` and `nxs mcp install --workspace` both add, nothing removed,
/// and the file was measured at 98 entries for workspaces that no longer existed on one developer
/// machine. The only remedy was hand-editing the TOML, which is also why this takes an OPTIONAL
/// path: the entries most worth pruning name directories that are gone, and resolving one of those
/// as a workspace would fail with `no_workspace` instead of removing it.
///
/// Removing an entry that is not registered is a successful no-op (`{"ok": true, "removed": false}`)
/// rather than a `not_found`: "this workspace is not attended" is the state the caller asked for,
/// and a script that unregisters on shutdown should not have to care whether it ever registered.
pub fn unregister_verb(json: bool, path: Option<&str>) -> Result<()> {
    let target = match path {
        Some(p) => p.to_string(),
        None => {
            let ws = Workspace::resolve(
                None,
                &std::env::current_dir()
                    .map_err(|e| NxfError::io(format!("reading the current directory: {e}")))?,
            )?;
            ws.dir.parent().unwrap_or(&ws.dir).display().to_string()
        }
    };
    // Asked BEFORE the removal, so it describes the registry this verb is about to change rather
    // than a re-derivation after the fact — and it is the counterpart of `bind`'s: somebody who
    // believes they are detaching a workspace from a development service must be told that the
    // production one is what they changed (nxf 6j6v.cvpy).
    let ignored = nxs_service::ignored_instance_env();
    let removed = crate::workspaces::unregister_workspace(&target)?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "removed": removed,
                "path": target,
                // `null` unless the environment named a service this binary could not be — the
                // same key `sync daemon install`/`uninstall` carry, so a script reads one name.
                // `bind` says the same thing on stderr only, beside the overlap warning it has
                // always said there: its `--json` is built behind the `Registrar` seam, and
                // widening a trait for a note is a worse trade than matching how that verb
                // already reports this class.
                "instance_env_ignored": ignored,
            })
        );
    } else if removed {
        println!("the background service no longer attends {target}");
    } else {
        println!("{target} was not registered with the background service — nothing to do");
    }
    if let Some(note) = &ignored {
        eprintln!("note: {note}");
    }
    Ok(())
}

/// `nxs sync bind`: the explicit create-vs-join step (§4.2), plus the default derived path. At
/// most one of `create`/`join` may be given.
///
/// Re-running `bind` is no longer strictly one-time (§4.4): with an unchanged intended id it is
/// a successful no-op (so `bind` is safe to call from scripts and from `nxs init`); a different
/// id is still refused unless `opts.rebind` is set, in which case the workspace is switched to
/// the new stream and both watermarks reset (`pushed_through` is a local rowid and
/// `pulled_through` a cursor on the OLD stream — neither means anything against a new one).
///
/// Thin wrapper around [`bind_with`], supplying the real autostart + registrar — see that
/// function for the actual body and the reason the seam exists.
pub fn bind(json: bool, ws: &Workspace, opts: BindOptions<'_>) -> Result<()> {
    bind_with(json, ws, opts, &RealAutostart, &RealRegistrar)
}

/// The actual worker behind [`bind`], with the autostart + registrar actions injected — the seam
/// that lets EVERY in-process test exercise `bind`'s full create/derive/join/rebind/idempotent
/// logic without ever installing a real launchd agent or writing into the real
/// `~/.nexusflow/workspaces.toml` (task 13, `x1c2`, plus its registry-leak follow-up: an
/// in-process unit test cannot isolate `$HOME` the way a subprocess CLI test can, so every test
/// that used to reach the real registrar/autostart through a plain `bind()` call was a leak).
/// `bind` is the only production caller (always passing the real implementations); this file's
/// tests call `bind_with` directly with spies in their place.
fn bind_with(
    json: bool,
    ws: &Workspace,
    opts: BindOptions<'_>,
    autostart: &dyn Autostart,
    registrar: &dyn Registrar,
) -> Result<()> {
    if opts.create && opts.join.is_some() {
        return Err(NxfError::validation(
            "use either --create or --join, not both",
        ));
    }
    if let Some(file) = opts.snapshot {
        return bind_from_snapshot(json, ws, &opts, file, autostart, registrar);
    }

    // Review Integrity #2 (PR #263), decided by the project owner: the DEFAULT (no-flag) path
    // derives `stream_id` from the repo remote, which the relay has no auth to gate
    // (`crates/server/src/app.rs`) — so this id is only as private as the remote URL. The
    // derivation itself stays (it is the whole point of the default path); this only decides
    // whether `warn_if_derived_bind` fires below. `--create` mints an unguessable random ULID and
    // `--join` takes an id the caller already has some other way, so neither is this specific
    // exposure.
    let derived_path = !opts.create && opts.join.is_none();

    let mut carried_endpoint = None;
    // Whether an existing binding was actually torn down below — as opposed to `opts.rebind`
    // being set on a workspace that was never bound, where there is nothing to switch from and
    // this is just an ordinary first bind. Drives the "rebind" vs. "derived"/"create"/"join"
    // mode label below.
    let mut switched = false;
    if let Some(existing) = load(ws)? {
        let intended = intended_stream_id(ws, opts.create, opts.join)?;
        // Id-equality is checked BEFORE the `--rebind` flag (review Code Quality #1, PR #263):
        // a bind that changes nothing is a no-op regardless of whether `--rebind` was passed —
        // `--rebind` only has teeth when it is actually switching to a DIFFERENT id. Checking the
        // flag first used to make `--rebind` on an unchanged id fall through to the switch path
        // below, deleting `sync.toml` and resetting both watermarks for a bind that changed
        // nothing. `--create` can never land here: `intended_stream_id` reports its destination
        // as `""` (unknowable ahead of minting), which never equals a real persisted id, so the
        // "always a switch" behaviour `create_with_rebind_on_an_already_bound_workspace_…` pins
        // is untouched.
        if intended == existing.stream_id {
            // Idempotent: same stream, nothing to switch. Still honour a new endpoint and keep
            // the registry entry fresh, so `bind` is safe to re-run from scripts.
            warn_if_derived_bind(derived_path);
            return finish_bind_with(
                json,
                ws,
                existing,
                "unchanged",
                opts.endpoint,
                opts.no_daemon,
                autostart,
                registrar,
            );
        }
        if !opts.rebind {
            return Err(NxfError::new(
                ErrorKind::Conflict,
                format!(
                    "workspace is bound to stream '{}', and this would switch it to {}. \
                     Re-run with --rebind to switch (the local log is re-pushed to the new \
                     stream and both watermarks reset).",
                    existing.stream_id,
                    describe_intended(&intended),
                ),
            ));
        }
        // Confirmed switch: validate the destination BEFORE mutating anything, so a refused
        // rebind (e.g. a URL-hostile id) leaves the existing binding untouched. `--create`
        // validates its own freshly minted id inside `bind_create`; nothing to pre-check here.
        if !opts.create {
            ensure_valid_stream_id(&intended)?;
        }
        // The endpoint lives on the meta we are about to remove — capture it so it survives the
        // switch unless the caller passes a new one (see the `finish_bind_with` call below).
        carried_endpoint = existing.endpoint;
        switched = true;
        // `bind_*` below is gated by `ensure_unbound`, so the old binding must go first.
        std::fs::remove_file(meta_path(ws))
            .map_err(|e| NxfError::io(format!("removing {SYNC_META_FILE}: {e}")))?;
    }

    let (meta, mode) = match (opts.create, opts.join) {
        // No flags is not an error: it is the DEFAULT path (derive from the remote).
        (false, None) => (bind_derived(ws)?, "derived"),
        (true, None) => (bind_create(ws)?, "create"),
        (false, Some(id)) => (bind_join(ws, id)?, "join"),
        (true, Some(_)) => unreachable!("rejected at the top of this function"),
    };
    let mode = if switched { "rebind" } else { mode };

    warn_if_derived_bind(derived_path);
    finish_bind_with(
        json,
        ws,
        meta,
        mode,
        opts.endpoint.or(carried_endpoint.as_deref()),
        opts.no_daemon,
        autostart,
        registrar,
    )
}

/// What one [`run_pass`] moved — the ONE outcome shape `nxs sync run` and the daemon sweep both
/// consume (n4dn), so a daemon log line and the CLI's `--json` output describe the same pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassOutcome {
    pub pushed: usize,
    pub pulled: usize,
    /// Set when the relay reassigned this replica's id prefix mid-pass (bab/§8) — `None` on an
    /// ordinary pass. Carried on the outcome rather than printed inside `run_pass` itself, so
    /// `run_pass` stays free of presentation and both callers (the CLI verb, the daemon) can
    /// report it their own way.
    pub reassigned_prefix: Option<String>,
    /// Mirrors [`nxs_sync::engine::SyncOutcome::budget_exhausted`] (6j6v.25f6): the pass ran out
    /// of its per-pass wall-clock budget. A clean stop — the watermark sits at the resume point —
    /// and the next pass continues from there.
    pub budget_exhausted: bool,
    /// Mirrors the engine's own page counters, so a caller can tell a genuine backlog from a relay
    /// that answers every request with nothing. See [`stopped_early`].
    pub pull_pages: usize,
    pub pull_empty_pages: usize,
    /// Mirrors [`nxs_sync::engine::SyncOutcome::pull_ceiling_hit`] (Integrity #3, PR #263): the
    /// pull loop hit its hard per-pass page ceiling rather than genuinely exhausting the stream.
    /// Not an error — the watermark is already at the right resume point — but worth surfacing so
    /// an operator watching either seam knows another pass is needed to finish a huge backlog.
    pub pull_ceiling_hit: bool,
    /// What became of this machine's announcement to the relay (6j6v.f0b5) — `None` when the pass
    /// announced nothing, which is every pass `nxs sync run` makes: only the service announces.
    pub presence: Option<PresenceNote>,
    /// Mirrors [`nxs_sync::engine::SyncOutcome::signatures_stripped`] (6j6v.pzkb). See
    /// [`downgraded`].
    pub signatures_stripped: usize,
    /// Mirrors [`nxs_sync::engine::SyncOutcome::unsigned_from_signers`] (6j6v.pzkb).
    pub unsigned_from_signers: usize,
}

/// What became of one announcement (6j6v.f0b5). Carried on the [`PassOutcome`] rather than failing
/// it: the sync is what the pass is for, and a relay that cannot record presence has still synced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenceNote {
    /// The relay recorded this machine as seen.
    Recorded,
    /// The relay has no presence route — it predates 6j6v.f0b5, or the endpoint is not a relay.
    Unsupported,
    /// Announcing failed; the sync before it did not. Never feeds the service's backoff.
    Failed(String),
}

/// One bidirectional push/pull pass (§4.3/§5.4). The ONE implementation — `nxs sync run` and the
/// daemon sweep (`daemon::sweep`, behind the `daemon::Pass` trait) both call this, so the
/// anti-entropy logic exists exactly once. `remote` is the already-resolved endpoint (see
/// [`endpoint::resolve`]); `run_pass` itself does no endpoint resolution — that precedence is the
/// caller's job, so it stays in ONE place regardless of how many callers there are.
///
/// Watermarks are persisted BEFORE any transport error is propagated: the engine advances them
/// per durable batch, so a mid-pass failure still records real progress and the next pass does
/// not re-push work that already landed. This ordering is pinned by
/// `crates/cli/tests/sync_run.rs::a_mid_pass_failure_still_persists_the_push_watermark` — do not
/// reorder it under a future refactor of this function.
pub fn run_pass(ws: &Workspace, remote: &str) -> Result<PassOutcome> {
    run_pass_announcing(ws, remote, None)
}

/// [`run_pass`], and — when `hello` is given — this machine's announcement to the relay after the
/// sync succeeded (6j6v.f0b5). The background service's pass (`daemon::ServicePass`) is the one
/// caller that passes a hello: "online" promises that something on the machine attends the stream,
/// and only the service keeps that promise, so a manual `nxs sync run` never makes a machine look
/// online.
///
/// After, not before, and only on success: a sighting then means "synced just now", and a pass
/// that failed announces nothing — which is what lets a machine whose relay is unreachable drop out
/// of everyone's list on its own. The announcement's own fate is reported, never raised
/// ([`PresenceNote`]).
pub fn run_pass_announcing(
    ws: &Workspace,
    remote: &str,
    hello: Option<&MachineHello>,
) -> Result<PassOutcome> {
    let mut meta = load(ws)?.ok_or_else(|| {
        NxfError::validation("workspace is not bound to a stream; run `nxs sync bind` first")
    })?;

    // Drive the engine over flow's store (interim, aye.36): it folds `task` ops on pull and owns
    // the prefix-collision remap. The op-log push/pull itself carries every domain regardless.
    let path = ws.db_path_str()?;
    let mut store = Store::open(&path, ws.replica.site_id)
        .map_err(|e| NxfError::io(format!("opening store at {path}: {e}")))?;
    let stream = StreamId(meta.stream_id.clone());
    let mut marks = Watermarks {
        pushed_through: meta.pushed_through,
        pulled_through: meta.pulled_through,
    };
    let transport = HttpTransport::new(remote);

    // Step 0 (bab/§8): register this replica's prefix. A relay reassignment remaps the local store
    // in place BEFORE any merge and we adopt the new prefix for future mints.
    let reassigned = engine::register_prefix(
        &mut store,
        &stream,
        &ws.replica.prefix,
        &ws.replica.replica_uuid,
        &transport,
    )
    .map_err(|e| NxfError::io(format!("prefix registration failed: {e}")))?;
    if let Some(new_prefix) = &reassigned {
        workspace::adopt_prefix(ws, new_prefix)?;
    }

    let result = engine::sync(
        &mut store,
        ws.replica.site_id,
        &stream,
        &mut marks,
        &transport,
        SYNC_PAGE_LIMIT,
        &WallClockBudget::starting_now(PASS_WALL_CLOCK_BUDGET),
    );

    // Persist whatever durably moved BEFORE propagating any error (the engine advances the
    // watermarks per durable batch), so a mid-pass transport failure still records real progress.
    meta.pushed_through = marks.pushed_through;
    meta.pulled_through = marks.pulled_through;
    save(ws, &meta)?;

    let outcome = result.map_err(|e| NxfError::io(format!("sync failed: {e}")))?;

    let presence = hello.map(|hello| match transport.announce(&stream, hello) {
        Ok(Announced::Recorded) => PresenceNote::Recorded,
        Ok(Announced::Unsupported) => PresenceNote::Unsupported,
        Err(e) => PresenceNote::Failed(e.to_string()),
    });

    Ok(PassOutcome {
        pushed: outcome.pushed,
        pulled: outcome.pulled,
        reassigned_prefix: reassigned,
        pull_ceiling_hit: outcome.pull_ceiling_hit,
        budget_exhausted: outcome.budget_exhausted,
        pull_pages: outcome.pull_pages,
        pull_empty_pages: outcome.pull_empty_pages,
        presence,
        signatures_stripped: outcome.signatures_stripped,
        unsigned_from_signers: outcome.unsigned_from_signers,
    })
}

/// The endpoint a verb standing in a bound workspace talks to: the one-shot `remote`, else the
/// workspace's own, else the global default — [`endpoint::resolve`]'s precedence. The ONE place a
/// verb resolves it (`run` and `machines` both), and the global config is only READ when nothing
/// closer names an endpoint, so a workspace bound to its own relay never touches the machine-wide
/// file at all.
fn resolve_endpoint(remote: Option<&str>, meta: &SyncMeta) -> Result<String> {
    let global = match remote.or(meta.endpoint.as_deref()) {
        Some(_) => None,
        None => endpoint::load_global_from(&endpoint::config_path()?)?,
    };
    endpoint::resolve(remote, meta.endpoint.as_deref(), global.as_deref())
}

/// Which machines a workspace's relay knows, judged (6j6v.f0b5) — what `nxs sync machines` shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachinesReading {
    pub stream_id: String,
    /// The relay that was asked, after the one precedence ([`endpoint::resolve`]) — with any
    /// credential in its URL masked (6j6v.q3kk), because this is a line for a transcript, not the
    /// configuration. `nxs sync endpoint` is where the configured value itself is read.
    pub endpoint: String,
    pub presence: Presence,
}

/// Ask the workspace's relay which machines sync its stream. The read itself is
/// [`nxs_sync::engine::machines`] — the same call an embedding host makes with a relay URL and a
/// stream id — so the CLI and the seam cannot show two different things; this only resolves the
/// workspace's stream and endpoint first (`remote` is the one-shot override, as for `run`).
pub fn read_machines(ws: &Workspace, remote: Option<&str>) -> Result<MachinesReading> {
    let meta = load(ws)?.ok_or_else(|| {
        NxfError::validation("workspace is not bound to a stream; run `nxs sync bind` first")
    })?;
    let endpoint = resolve_endpoint(remote, &meta)?;
    let shown = nxs_sync::redact::redact_userinfo(&endpoint);
    let presence = engine::machines(
        &HttpTransport::new(&endpoint),
        &StreamId(meta.stream_id.clone()),
    )
    .map_err(|e| {
        NxfError::io(format!(
            "asking {shown} which machines sync this workspace: {e}"
        ))
    })?;
    Ok(MachinesReading {
        stream_id: meta.stream_id,
        endpoint: shown,
        presence,
    })
}

/// How long ONE workspace's pass may spend pulling before it stops and lets the sweep move on
/// (6j6v.25f6).
///
/// **Why a page ceiling was not enough.** The pull loop was capped only by
/// `PULL_PAGE_CEILING` = 20,000 pages. A relay that answers every request slowly but nudges the
/// cursor by any amount at all can therefore hold one pass for 20,000 round-trips — against the
/// `HttpTransport`'s own 30-second read timeout, that is over 160 hours, arithmetically. And the
/// service is a single-threaded loop over every registered workspace, so one wedged workspace does
/// not starve only itself: it starves ALL of them, including their deadlines.
///
/// **Sixty seconds**, and the number is a trade rather than a guess. It must be comfortably larger
/// than one transport round-trip (30s read timeout) so an ordinary slow page is not mistaken for a
/// wedge; it must be small enough that a sweep over a handful of workspaces still fits inside the
/// 300-second safety-net interval. Nothing is lost by stopping: the watermark advances per page, so
/// the next pass resumes exactly where this one stopped — a real backlog drains over several passes
/// instead of one long one.
///
/// **What it does NOT bound** is a single request. The budget is checked between pages, so the true
/// worst case is this plus one transport timeout. Bounding a request is the transport's job and it
/// already does it.
const PASS_WALL_CLOCK_BUDGET: Duration = Duration::from_secs(60);

/// The production [`engine::PassBudget`]: a wall clock, started when the pass starts.
///
/// It lives HERE and not in the engine on purpose. The design keeps the sync engine clock-free —
/// all time belongs to the daemon, where it is injected and provable without sleeping — so the
/// engine asks "am I out of budget?" and this answers out of a clock it owns. `Instant`, not
/// `SystemTime`: a pass must not be cut short (or extended) by an NTP correction landing mid-sweep.
struct WallClockBudget {
    started: Instant,
    limit: Duration,
}

impl WallClockBudget {
    fn starting_now(limit: Duration) -> WallClockBudget {
        WallClockBudget {
            started: Instant::now(),
            limit,
        }
    }
}

impl engine::PassBudget for WallClockBudget {
    fn exhausted(&self) -> bool {
        self.started.elapsed() >= self.limit
    }
}

/// What to tell an operator about a pass that stopped before the stream was exhausted — or `None`
/// when it finished normally.
///
/// **The sentence is the point of this function** (6j6v.25f6's second half). A pass that stops
/// early looks identical from the outside whether the stream is genuinely enormous or the relay is
/// answering with almost nothing and nudging the cursor a little each time. The first drains; the
/// second never does. An operator could not tell them apart at all, and "more history remains, the
/// next pass will continue" is actively misleading for the second.
///
/// Pure, and separate from every place that prints it, so the CLI verb and the service log say the
/// same thing and one test covers both.
pub fn stopped_early(outcome: &PassOutcome) -> Option<String> {
    if !outcome.pull_ceiling_hit && !outcome.budget_exhausted {
        return None;
    }
    let limit = if outcome.budget_exhausted {
        "its per-pass time budget"
    } else {
        "its per-pass page limit"
    };
    // A MAJORITY of pages carrying nothing while the cursor kept moving is the trickle signature.
    // Not "any empty page": a legitimate stream can end on one, and a relay may serve a sparse
    // stretch without being wedged.
    if outcome.pull_empty_pages * 2 > outcome.pull_pages {
        return Some(format!(
            "hit {limit} after {} pages, {} of which delivered nothing while the cursor kept \
             moving — this looks like a relay trickling, not a backlog draining; the next pass \
             will try again from where this one stopped",
            outcome.pull_pages, outcome.pull_empty_pages
        ));
    }
    Some(format!(
        "hit {limit} after {} pages; more history remains, and the next pass continues from where \
         this one stopped",
        outcome.pull_pages
    ))
}

/// What to tell an operator about signatures that went missing on the way (6j6v.pzkb) — or `None`
/// when none did.
///
/// An op without its signature never carries an agent action, so a relay that drops signatures
/// cannot make anything act; but it silently turns every instruction another machine signs into
/// one this machine ignores, and "silently" is the part this sentence exists to remove. Pure, and
/// shared by `nxs sync run` and the service log, like [`stopped_early`].
pub fn downgraded(outcome: &PassOutcome) -> Option<String> {
    let (stripped, unsigned) = (outcome.signatures_stripped, outcome.unsigned_from_signers);
    if stripped == 0 && unsigned == 0 {
        return None;
    }
    let mut seen = Vec::new();
    if stripped > 0 {
        seen.push(format!(
            "{stripped} op(s) this replica holds signed came back from the relay without their \
             signature"
        ));
    }
    if unsigned > 0 {
        seen.push(format!(
            "{unsigned} new op(s) arrived unsigned from replicas that otherwise sign"
        ));
    }
    Some(format!(
        "{} — the relay (or something in front of it) drops signatures. Those ops are kept and \
         shown, but no agent action follows them. A relay older than nxs 0.58 drops them by \
         design: upgrade it.",
        seen.join("; ")
    ))
}

/// `nxs sync run`: one client-driven push/pull pass against the relay (§4.3). Requires a prior
/// bind; the advanced watermarks are persisted so re-running only moves what is new. `remote` is
/// the one-shot `--remote` override (never persisted) — absent, the bound workspace endpoint or
/// the global default is used, via the single precedence in [`endpoint::resolve`].
///
/// Thin by design: resolve the endpoint, delegate the pass itself to [`run_pass`] (the ONE
/// implementation the daemon also calls), then render the outcome.
pub fn run(json: bool, ws: &Workspace, remote: Option<&str>) -> Result<()> {
    let meta = load(ws)?.ok_or_else(|| {
        NxfError::validation("workspace is not bound to a stream; run `nxs sync bind` first")
    })?;

    let remote = resolve_endpoint(remote, &meta)?;

    let outcome = run_pass(ws, &remote)?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "stream_id": meta.stream_id,
                "pushed": outcome.pushed,
                "pulled": outcome.pulled,
                "reassigned_prefix": outcome.reassigned_prefix,
                "pull_ceiling_hit": outcome.pull_ceiling_hit,
                "budget_exhausted": outcome.budget_exhausted,
                "pull_pages": outcome.pull_pages,
                "pull_empty_pages": outcome.pull_empty_pages,
                "stopped_early": stopped_early(&outcome),
                "signatures_stripped": outcome.signatures_stripped,
                "unsigned_from_signers": outcome.unsigned_from_signers,
                "downgraded": downgraded(&outcome),
            })
        );
    } else {
        if let Some(new_prefix) = &outcome.reassigned_prefix {
            println!("prefix reassigned to {new_prefix} (local ids remapped)");
        }
        // ONE sentence for both stop reasons, from the shared builder, so the verb and the service
        // log cannot describe the same pass differently (6j6v.25f6).
        if let Some(note) = stopped_early(&outcome) {
            println!("note: this pass {note}");
        }
        if let Some(warning) = downgraded(&outcome) {
            eprintln!("warning: {warning}");
        }
        println!(
            "synced stream {}: pushed {}, pulled {}",
            meta.stream_id, outcome.pushed, outcome.pulled
        );
    }
    Ok(())
}

/// `nxs sync machine [<name>]`: show or rename this machine (6j6v.f0b5). Workspace-free, like
/// `endpoint`: the machine belongs to the service home ([`nxs_service::ServiceHome::machine`]),
/// which is also where an embedding host reads and renames it.
pub fn machine_verb(json: bool, name: Option<&str>) -> Result<()> {
    let home = nxs_service::ServiceHome::resolve()?;
    // Which service home's machine this is, when the environment named another (nxf 6j6v.cvpy):
    // with the installed binary and a working copy's `.envrc`, a rename would otherwise silently
    // rename the PRODUCTION machine. The same sentence `bind` and `unregister` say.
    if let Some(note) = nxs_service::ignored_instance_env() {
        eprintln!("note: {note}");
    }
    let machine = match name {
        Some(name) => home.rename_machine(name)?,
        None => home.machine()?,
    };
    let instance = home.instance().name();
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "machine_id": machine.id,
                "name": machine.name,
                "instance": instance,
                "renamed": name.is_some(),
            })
        );
    } else if name.is_some() {
        println!(
            "this machine is now called {} — its service announces the new name on its next pass",
            machine.name
        );
    } else {
        println!(
            "{} (machine {}, service {instance})",
            machine.name, machine.id
        );
    }
    Ok(())
}

/// `nxs sync machines [--remote <url>]`: which machines sync this workspace, and which are online
/// (6j6v.f0b5). A thin renderer over [`read_machines`] — the read an embedding host makes too —
/// plus one local fact the relay cannot know: which of the listed machines is THIS one.
pub fn machines_verb(json: bool, ws: &Workspace, remote: Option<&str>) -> Result<()> {
    let reading = read_machines(ws, remote)?;
    // Peeked, never minted: listing must leave no trace on a machine that never announced itself.
    // And only to mark "(this machine)" — a local machine file that cannot be read costs the
    // marker, with a note, never the list the relay just gave (review of PR #485, Code Quality #7).
    let this = match nxs_service::ServiceHome::resolve().and_then(|home| home.peek_machine()) {
        Ok(machine) => machine.map(|m| m.id),
        Err(e) => {
            eprintln!(
                "note: cannot tell which of these is this machine: {}",
                e.msg
            );
            None
        }
    };
    let is_this = |id: &str| this.as_deref() == Some(id);
    let (presence, listed, truncated) = match &reading.presence {
        Presence::Reported {
            machines,
            truncated,
        } => ("reported", machines.as_slice(), *truncated),
        Presence::Unsupported => ("unsupported", &[][..], false),
    };
    if json {
        let machines: Vec<serde_json::Value> = listed
            .iter()
            .map(|m| {
                serde_json::json!({
                    "machine_id": m.machine_id,
                    "name": m.name,
                    "online": m.online,
                    "last_seen": m.last_seen,
                    "last_seen_at": unix_to_rfc3339(m.last_seen),
                    "age_secs": m.age_secs,
                    "interval_secs": m.interval_secs,
                    "online_within_secs": m.online_within_secs,
                    "this_machine": is_this(&m.machine_id),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "stream_id": reading.stream_id,
                "endpoint": reading.endpoint,
                "presence": presence,
                "machines": machines,
                "truncated": truncated,
            })
        );
        return Ok(());
    }
    if matches!(reading.presence, Presence::Unsupported) {
        println!(
            "the relay at {} does not record which machines sync a stream (it predates machine \
             presence, a gateway in front of it does not route the presence path, or the URL is \
             not a relay) — syncing is unaffected",
            reading.endpoint
        );
        return Ok(());
    }
    if listed.is_empty() && !truncated {
        println!(
            "stream {} on {}: no machine has announced itself yet — a machine appears once its \
             background service has synced this workspace (`nxs sync daemon install`)",
            reading.stream_id, reading.endpoint
        );
        return Ok(());
    }
    let online = listed.iter().filter(|m| m.online).count();
    println!(
        "stream {} on {} — {} machine{}, {online} online",
        reading.stream_id,
        reading.endpoint,
        listed.len(),
        if listed.len() == 1 { "" } else { "s" }
    );
    let width = listed
        .iter()
        .map(|m| m.name.chars().count())
        .max()
        .unwrap_or(0);
    for m in listed {
        println!(
            "  {:<7}  {:<width$}  last seen {} ago{}",
            if m.online { "online" } else { "offline" },
            m.name,
            ago(m.age_secs),
            if is_this(&m.machine_id) {
                "  (this machine)"
            } else {
                ""
            },
        );
    }
    if truncated {
        println!(
            "note: this is not every machine the relay holds for the stream — it knows more than one \
             answer carries, or sent entries no honest machine announces. The relay authenticates \
             nobody, so a stream can be flooded; trust the names you recognise"
        );
    }
    Ok(())
}

/// A relay age as a person reads it: `42s`, `7m`, `2h 5m`, `3d 4h`.
fn ago(secs: u64) -> String {
    let (m, h, d) = (secs / 60, secs / 3_600, secs / 86_400);
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3_599 => format!("{m}m"),
        3_600..=86_399 if m % 60 == 0 => format!("{h}h"),
        3_600..=86_399 => format!("{h}h {}m", m % 60),
        _ if h % 24 == 0 => format!("{d}d"),
        _ => format!("{d}d {}h", h % 24),
    }
}

/// Unix seconds (the relay's clock) as RFC 3339, or `null` for a time outside what RFC 3339 can
/// spell — a relay's answer is input, and printing must not fail on it.
fn unix_to_rfc3339(secs: i64) -> Option<String> {
    time::OffsetDateTime::from_unix_timestamp(secs)
        .ok()?
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

/// `nxs sync endpoint [<url>]`: show or set the global default. Deliberately workspace-free —
/// it is machine-wide configuration, usable before any workspace exists.
pub fn endpoint_verb(json: bool, url: Option<&str>) -> Result<()> {
    let path = endpoint::config_path()?;
    match url {
        Some(url) => {
            endpoint::save_global_to(&path, url)?;
            // Shown masked, set and read alike: a key in the URL belongs in the config file (written
            // owner-only), not in a terminal scrollback or an agent's transcript.
            let shown = nxs_sync::redact::redact_userinfo(url);
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "ok": true, "default_endpoint": shown })
                );
            } else {
                println!("default sync endpoint set to {shown}");
            }
        }
        None => {
            let current = endpoint::load_global_from(&path)?
                .map(|url| nxs_sync::redact::redact_userinfo(&url));
            if json {
                println!("{}", serde_json::json!({ "default_endpoint": current }));
            } else {
                match &current {
                    Some(url) => println!("{url}"),
                    None => println!("no default sync endpoint set"),
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod presence_tests {
    use super::*;

    #[test]
    fn an_age_reads_the_way_a_person_says_it() {
        for (secs, said) in [
            (0, "0s"),
            (59, "59s"),
            (60, "1m"),
            (65, "1m"),
            (3_599, "59m"),
            (3_600, "1h"),
            (7_500, "2h 5m"),
            (86_400, "1d"),
            (90_000, "1d 1h"),
        ] {
            assert_eq!(ago(secs), said, "{secs}s");
        }
    }

    #[test]
    fn a_time_the_relay_sends_is_printed_as_rfc3339_and_an_impossible_one_as_nothing() {
        assert_eq!(
            unix_to_rfc3339(1_790_000_000).as_deref(),
            Some("2026-09-21T14:13:20Z")
        );
        assert_eq!(unix_to_rfc3339(i64::MAX), None);
    }

    #[test]
    fn the_service_and_the_relay_hold_a_machine_name_to_the_same_rule() {
        // Two crates, one rule: `nxs-service` refuses a rename, the relay refuses an announcement.
        // If they drifted, a name the owner was allowed to set would be refused by every relay — and
        // the machine would silently vanish from every list. `nxs` is the one crate that links both.
        assert_eq!(
            nxs_service::machine::MAX_NAME_CHARS,
            nxs_sync::presence::MAX_MACHINE_NAME_CHARS
        );
        let limit = nxs_service::machine::MAX_NAME_CHARS;
        for name in [
            "Mac mini".to_string(),
            "Carsten’s MacBook (dev)".to_string(),
            "".to_string(),
            "   ".to_string(),
            "tab\there".to_string(),
            "\u{200B}".to_string(),
            "Mac\u{200B}mini".to_string(),
            "Mac mini\u{202E}".to_string(),
            "\u{2066}Mac\u{2069}".to_string(),
            "\u{FEFF}Mac".to_string(),
            "Mac\u{00AD}mini".to_string(),
            "Mac\u{2028}mini".to_string(),
            "Mac\u{00A0}mini".to_string(),
            "é".repeat(limit),
            "é".repeat(limit + 1),
        ] {
            assert_eq!(
                nxs_service::machine::check_name(&name).is_ok(),
                nxs_sync::presence::check_machine_name(&name).is_ok(),
                "{name:?}"
            );
        }
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use nxs_sync::engine::PassBudget;

    fn stopped(f: impl FnOnce(&mut PassOutcome)) -> Option<String> {
        let mut o = PassOutcome::default();
        f(&mut o);
        stopped_early(&o)
    }

    #[test]
    fn a_pass_that_finished_normally_says_nothing() {
        assert_eq!(stopped(|o| o.pulled = 42), None);
    }

    #[test]
    fn a_draining_backlog_and_a_trickling_relay_do_not_read_the_same() {
        let backlog = stopped(|o| {
            o.pull_ceiling_hit = true;
            o.pull_pages = 20_000;
            o.pull_empty_pages = 3;
        })
        .expect("a stop is reported");
        let trickle = stopped(|o| {
            o.budget_exhausted = true;
            o.pull_pages = 120;
            o.pull_empty_pages = 119;
        })
        .expect("a stop is reported");

        assert!(
            backlog.contains("more history remains"),
            "a real backlog drains, and says so: {backlog}"
        );
        assert!(
            !backlog.contains("trickling"),
            "and must not be called a wedge: {backlog}"
        );
        assert!(
            trickle.contains("trickling"),
            "a relay delivering nothing never drains, and that is the thing an operator could \
             not see before: {trickle}"
        );
        assert!(
            trickle.contains("119"),
            "with the numbers it is a judgement on: {trickle}"
        );
    }

    #[test]
    fn the_sentence_names_which_of_the_two_limits_stopped_the_pass() {
        let by_pages = stopped(|o| {
            o.pull_ceiling_hit = true;
            o.pull_pages = 20_000;
        })
        .unwrap();
        let by_time = stopped(|o| {
            o.budget_exhausted = true;
            o.pull_pages = 7;
        })
        .unwrap();
        assert!(by_pages.contains("page limit"), "{by_pages}");
        assert!(by_time.contains("time budget"), "{by_time}");
    }

    #[test]
    fn exactly_half_the_pages_being_empty_is_not_yet_a_trickle() {
        // A strict majority, deliberately: a sparse stretch of a legitimate stream is not a wedge,
        // and calling one a wedge is how an operator learns to ignore the line.
        let half = stopped(|o| {
            o.budget_exhausted = true;
            o.pull_pages = 10;
            o.pull_empty_pages = 5;
        })
        .unwrap();
        assert!(!half.contains("trickling"), "{half}");
    }

    #[test]
    fn the_wall_clock_budget_is_spent_at_its_limit_and_not_before() {
        // No time passes here: a zero limit is already over, and an hour is not. The BOUND is what
        // the engine's own tests prove against a counter; this is only that the production budget
        // reads its clock the way round it says it does.
        assert!(WallClockBudget::starting_now(Duration::ZERO).exhausted());
        assert!(!WallClockBudget::starting_now(Duration::from_secs(3600)).exhausted());
    }

    #[test]
    fn the_per_pass_budget_is_larger_than_one_transport_round_trip() {
        // The trade the constant's doc names, pinned: smaller than the transport's 30s read timeout
        // would abort on one ordinary slow page, and larger than the 300s safety-net interval would
        // let a single workspace own a whole sweep.
        assert!(PASS_WALL_CLOCK_BUDGET > Duration::from_secs(30));
        assert!(PASS_WALL_CLOCK_BUDGET < Duration::from_secs(300));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nxs_foundation::workspace::WorkspaceConfig;
    use tempfile::TempDir;

    fn workspace() -> (TempDir, Workspace) {
        let tmp = TempDir::new().unwrap();
        let ws = workspace::setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        (tmp, ws)
    }

    /// A workspace inside a git repo with `origin` pointing at `remote`.
    fn workspace_with_remote(remote: &str) -> (TempDir, Workspace) {
        let tmp = TempDir::new().unwrap();
        for args in [vec!["init", "-q"], vec!["remote", "add", "origin", remote]] {
            let ok = std::process::Command::new("git")
                .args(&args)
                .current_dir(tmp.path())
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        }
        let ws = workspace::setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        (tmp, ws)
    }

    #[test]
    fn two_clones_of_one_repo_bind_to_the_same_stream_without_join() {
        let (_a, wsa) = workspace_with_remote("git@github.com:nxsflow/manufakt-io.git");
        let (_b, wsb) = workspace_with_remote("https://github.com/nxsflow/manufakt-io");
        let a = bind_derived(&wsa).unwrap();
        let b = bind_derived(&wsb).unwrap();
        assert_eq!(
            a.stream_id, b.stream_id,
            "the DoD: converge with no manual --join"
        );
    }

    #[test]
    fn binding_without_a_git_origin_names_create_and_join() {
        let (_tmp, ws) = workspace(); // no git repo at all
        let err = bind_derived(&ws).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.msg.contains("--create"), "{}", err.msg);
        assert!(err.msg.contains("--join"), "{}", err.msg);
    }

    #[test]
    fn joining_a_url_hostile_id_is_refused_at_bind_time_not_as_a_runtime_404() {
        let (_tmp, ws) = workspace();
        let err = bind_join(&ws, "nxsflow/manufakt-io").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        // Each invalid byte (here, the single '/') is mapped to one '_' by `ensure_valid_stream_id`.
        assert!(
            err.msg.contains("nxsflow_manufakt-io"),
            "suggests a safe id: {}",
            err.msg
        );
        assert!(!is_bound(&ws), "a refused bind must not persist anything");
    }

    #[test]
    fn a_minted_id_satisfies_the_same_gate_binding_enforces() {
        let (_tmp, ws) = workspace();
        let meta = bind_create(&ws).unwrap();
        assert!(slug::is_valid_stream_id(&meta.stream_id));
    }

    #[test]
    fn create_mints_and_persists_a_stream_id() {
        let (_tmp, ws) = workspace();
        assert!(load(&ws).unwrap().is_none(), "unbound to start");
        let meta = bind_create(&ws).unwrap();
        assert!(meta.stream_id.starts_with("stream-"));
        assert!(is_bound(&ws));
        assert_eq!(load(&ws).unwrap().unwrap(), meta);
    }

    #[test]
    fn join_persists_the_given_id_and_mints_no_new_one() {
        let (_tmp, ws) = workspace();
        let meta = bind_join(&ws, "stream-shared-xyz").unwrap();
        assert_eq!(meta.stream_id, "stream-shared-xyz");
        assert_eq!(load(&ws).unwrap().unwrap().stream_id, "stream-shared-xyz");
    }

    #[test]
    fn two_creates_yield_distinct_stream_ids() {
        let (_a, wsa) = workspace();
        let (_b, wsb) = workspace();
        assert_ne!(
            bind_create(&wsa).unwrap().stream_id,
            bind_create(&wsb).unwrap().stream_id,
        );
    }

    #[test]
    fn create_on_an_already_bound_workspace_is_rejected() {
        let (_tmp, ws) = workspace();
        bind_create(&ws).unwrap();
        assert_eq!(bind_create(&ws).unwrap_err().kind, ErrorKind::Conflict);
    }

    #[test]
    fn join_on_an_already_bound_workspace_is_rejected() {
        let (_tmp, ws) = workspace();
        bind_join(&ws, "stream-one").unwrap();
        assert_eq!(
            bind_join(&ws, "stream-two").unwrap_err().kind,
            ErrorKind::Conflict
        );
        assert_eq!(load(&ws).unwrap().unwrap().stream_id, "stream-one");
    }

    #[test]
    fn empty_join_id_is_rejected() {
        let (_tmp, ws) = workspace();
        assert_eq!(
            bind_join(&ws, "  ").unwrap_err().kind,
            ErrorKind::Validation
        );
    }

    #[test]
    fn a_bind_only_meta_written_by_an_older_binary_still_parses() {
        // Forward/backward compat: `endpoint` is additive, so a sync.toml with only a
        // stream_id (what every pre-kgn5 binary wrote) must still load.
        let (_tmp, ws) = workspace();
        std::fs::write(meta_path(&ws), "stream_id = \"stream-old\"\n").unwrap();
        let meta = load(&ws).unwrap().unwrap();
        assert_eq!(meta.stream_id, "stream-old");
        assert_eq!(meta.endpoint, None);
    }

    #[test]
    fn the_endpoint_round_trips_through_the_meta() {
        let (_tmp, ws) = workspace();
        let meta = SyncMeta {
            stream_id: "stream-abc".into(),
            endpoint: Some("https://relay.example".into()),
            ..Default::default()
        };
        save(&ws, &meta).unwrap();
        assert_eq!(load(&ws).unwrap().unwrap(), meta);
    }

    #[cfg(unix)]
    #[test]
    fn save_refuses_a_pre_placed_temp_symlink_and_leaves_the_target_untouched() {
        // Review finding Integrity #1 (PR #263): `save` writes `.nxs/sync.toml` on every bind and
        // after every sync pass — a symlink pre-placed in a cloned repo at the predictable temp
        // path must not be followed through to whatever it points at. Mirrors
        // `endpoint.rs`'s `save_global_to_refuses_a_pre_placed_temp_symlink_…` and
        // `workspaces.rs`'s `upsert_refuses_a_pre_placed_temp_symlink_…`, now that `save` reuses
        // the same `write_atomic` hardening.
        use std::os::unix::fs::symlink;
        let (tmp, ws) = workspace();

        let outside = tmp.path().join("precious.txt");
        std::fs::write(&outside, b"PRECIOUS").unwrap();

        // The exact predictable temp path `write_atomic` computes: `.{file_name}.nxs.tmp.{pid}`.
        let temp_path = ws
            .dir
            .join(format!(".{SYNC_META_FILE}.nxs.tmp.{}", std::process::id()));
        symlink(&outside, &temp_path).unwrap();

        let meta = SyncMeta {
            stream_id: "stream-one".into(),
            ..Default::default()
        };
        let err = save(&ws, &meta).unwrap_err();
        assert_eq!(
            err.kind,
            ErrorKind::Io,
            "a squatted temp path is a loud io error"
        );
        assert_eq!(
            std::fs::read(&outside).unwrap(),
            b"PRECIOUS",
            "the symlink target is never written through"
        );
        assert!(
            !meta_path(&ws).exists(),
            "no sync.toml was produced through the symlink"
        );
    }

    #[test]
    fn a_malformed_persisted_id_still_loads_because_the_gate_is_bind_only() {
        // Explicitly NOT validated on load — that would break existing working bindings.
        let (_tmp, ws) = workspace();
        std::fs::write(meta_path(&ws), "stream_id = \"nxsflow/manufakt-io\"\n").unwrap();
        assert_eq!(load(&ws).unwrap().unwrap().stream_id, "nxsflow/manufakt-io");
    }

    /// Thin wrapper mirroring the `--rebind` CLI shape (`join`/`endpoint` only — `create` and
    /// `--rebind` are never combined in these tests). Kept test-only: the CLI builds
    /// `BindOptions` directly since it already has `rebind` as a field, so nothing outside
    /// tests needs this shorthand. Goes through `bind_with` + spies, not the public `bind` —
    /// every in-process test does (see `SpyAutostart`/`SpyRegistrar` further down this module):
    /// `bind` always uses the REAL autostart/registrar, and unlike a subprocess CLI test, an
    /// in-process test cannot isolate `$HOME`, so a plain `bind()` call here would write into the
    /// developer's real `~/.nexusflow/workspaces.toml` (measured at 98 stale entries on one
    /// machine, task 13 follow-up) and attempt a real launchd install on macOS (CRITICAL).
    fn rebind(
        json: bool,
        ws: &Workspace,
        join: Option<&str>,
        endpoint: Option<&str>,
    ) -> Result<()> {
        bind_with(
            json,
            ws,
            BindOptions {
                create: false,
                join,
                endpoint,
                rebind: true,
                // Belt and suspenders: `--no-daemon` ALSO suppresses the (spy) autostart call,
                // on top of the spy never touching anything real either way.
                no_daemon: true,
                snapshot: None,
            },
            &SpyAutostart::new(),
            &SpyRegistrar::new(),
        )
    }

    #[test]
    fn rebinding_to_the_same_id_is_a_successful_no_op() {
        let (_a, ws) = workspace_with_remote("git@github.com:nxsflow/manufakt-io.git");
        bind_with(
            false,
            &ws,
            BindOptions {
                create: false,
                join: None,
                endpoint: None,
                rebind: false,
                no_daemon: true, // CRITICAL: never touch the real launchd session from a test
                snapshot: None,
            },
            &SpyAutostart::new(),
            &SpyRegistrar::new(),
        )
        .unwrap();
        let first = load(&ws).unwrap().unwrap();
        bind_with(
            false,
            &ws,
            BindOptions {
                create: false,
                join: None,
                endpoint: Some("https://relay.example"),
                rebind: false,
                no_daemon: true,
                snapshot: None,
            },
            &SpyAutostart::new(),
            &SpyRegistrar::new(),
        )
        .unwrap();
        let second = load(&ws).unwrap().unwrap();
        assert_eq!(second.stream_id, first.stream_id);
        assert_eq!(
            second.endpoint.as_deref(),
            Some("https://relay.example"),
            "an idempotent re-bind may still update the endpoint"
        );
    }

    #[test]
    fn switching_to_a_different_id_without_rebind_is_refused_and_names_both() {
        let (_tmp, ws) = workspace();
        bind_join(&ws, "stream-one").unwrap();
        let err = bind_with(
            false,
            &ws,
            BindOptions {
                create: false,
                join: Some("stream-two"),
                endpoint: None,
                rebind: false,
                no_daemon: true,
                snapshot: None,
            },
            &SpyAutostart::new(),
            &SpyRegistrar::new(),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Conflict);
        assert!(
            err.msg.contains("stream-one") && err.msg.contains("stream-two"),
            "{}",
            err.msg
        );
        assert!(err.msg.contains("--rebind"), "{}", err.msg);
        assert_eq!(
            load(&ws).unwrap().unwrap().stream_id,
            "stream-one",
            "unchanged"
        );
    }

    #[test]
    fn rebind_switches_the_stream_and_resets_both_watermarks() {
        // Both are meaningless against a new stream: pushed_through is a LOCAL rowid and
        // pulled_through a cursor on the OLD stream. Keeping either would skip history.
        let (_tmp, ws) = workspace();
        save(
            &ws,
            &SyncMeta {
                stream_id: "stream-one".into(),
                pushed_through: 42,
                pulled_through: 17,
                endpoint: Some("https://relay.example".into()),
            },
        )
        .unwrap();
        rebind(false, &ws, Some("stream-two"), None).unwrap();
        let meta = load(&ws).unwrap().unwrap();
        assert_eq!(meta.stream_id, "stream-two");
        assert_eq!((meta.pushed_through, meta.pulled_through), (0, 0));
        assert_eq!(
            meta.endpoint.as_deref(),
            Some("https://relay.example"),
            "the endpoint survives a stream switch unless explicitly replaced"
        );
    }

    #[test]
    fn rebind_refuses_a_url_hostile_id_too() {
        let (_tmp, ws) = workspace();
        bind_join(&ws, "stream-one").unwrap();
        let err = rebind(false, &ws, Some("nxsflow/manufakt-io"), None).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert_eq!(load(&ws).unwrap().unwrap().stream_id, "stream-one");
    }

    // Review finding (post-task-8): `intended_stream_id`'s `(true, _) => Ok(String::new())`
    // branch makes `--create` on an already-bound workspace ALWAYS classify as a switch (a
    // fresh id can never equal a real existing one) — through `bind()`, not the lower-level
    // `bind_create` the older `create_on_an_already_bound_workspace_is_rejected` test above
    // exercises. That branch had no coverage: the CLI test that used to bind twice via
    // `--create` was rewritten to use `--join` when the idempotent/differing-id rule landed, so
    // nothing exercised it any more. These two pin both sub-cases through `bind()` itself.

    #[test]
    fn create_on_an_already_bound_workspace_without_rebind_is_a_conflict_naming_the_existing_id() {
        let (_tmp, ws) = workspace();
        bind_join(&ws, "stream-one").unwrap();
        let err = bind_with(
            false,
            &ws,
            BindOptions {
                create: true,
                join: None,
                endpoint: None,
                rebind: false,
                no_daemon: true,
                snapshot: None,
            },
            &SpyAutostart::new(),
            &SpyRegistrar::new(),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Conflict);
        assert!(err.msg.contains("stream-one"), "{}", err.msg);
        assert!(err.msg.contains("--rebind"), "{}", err.msg);
        assert_eq!(
            load(&ws).unwrap().unwrap().stream_id,
            "stream-one",
            "unchanged"
        );
    }

    // ---- Autostart + registrar seams (task 13, `x1c2`, and its registry-leak follow-up): bind
    // starts the sync daemon, `--no-daemon` suppresses it, a failing installer never turns a
    // successful bind into a failure, and bind registers the workspace into the registry the
    // daemon sweeps — all without ever touching a real launchd session or the developer's real
    // `~/.nexusflow/workspaces.toml`. The autostart properties were required beyond the task-13
    // brief's own test list; the registrar spy was added after review measured 98 stale entries
    // in the real registry, all written by in-process unit tests that could not isolate `$HOME`
    // the way a subprocess CLI test can. These call `finish_bind_with` directly with spies in
    // both seams, so none of them ever calls `launchd::install_quiet` or
    // `crate::workspaces::register_workspace` — i.e. never touches anything real (CRITICAL,
    // task-13 brief).

    struct SpyAutostart {
        called: std::cell::Cell<bool>,
        fail: bool,
    }

    impl SpyAutostart {
        fn new() -> Self {
            SpyAutostart {
                called: std::cell::Cell::new(false),
                fail: false,
            }
        }
        fn failing() -> Self {
            SpyAutostart {
                called: std::cell::Cell::new(false),
                fail: true,
            }
        }
    }

    impl Autostart for SpyAutostart {
        fn start(&self) -> Result<()> {
            self.called.set(true);
            if self.fail {
                Err(NxfError::io("launchd install failed"))
            } else {
                Ok(())
            }
        }
    }

    /// Records whether the registrar was invoked instead of writing into the real
    /// `~/.nexusflow/workspaces.toml` — the registry-side counterpart to `SpyAutostart`.
    struct SpyRegistrar {
        called: std::cell::Cell<bool>,
        fail: bool,
    }

    impl SpyRegistrar {
        fn new() -> Self {
            SpyRegistrar {
                called: std::cell::Cell::new(false),
                fail: false,
            }
        }
        fn failing() -> Self {
            SpyRegistrar {
                called: std::cell::Cell::new(false),
                fail: true,
            }
        }
    }

    impl Registrar for SpyRegistrar {
        fn register(&self, _path: &str) -> Result<bool> {
            self.called.set(true);
            if self.fail {
                Err(NxfError::io("registry upsert failed"))
            } else {
                Ok(true)
            }
        }
    }

    #[test]
    fn a_successful_bind_calls_the_autostart_installer() {
        let (_tmp, ws) = workspace();
        let meta = bind_join(&ws, "stream-one").unwrap();
        let spy = SpyAutostart::new();
        finish_bind_with(
            false,
            &ws,
            meta,
            "join",
            None,
            false,
            &spy,
            &SpyRegistrar::new(),
        )
        .unwrap();
        assert!(spy.called.get(), "a successful bind starts the daemon");
    }

    #[test]
    fn no_daemon_binds_without_calling_the_autostart_installer() {
        let (_tmp, ws) = workspace();
        let meta = bind_join(&ws, "stream-one").unwrap();
        let spy = SpyAutostart::new();
        finish_bind_with(
            false,
            &ws,
            meta,
            "join",
            None,
            true,
            &spy,
            &SpyRegistrar::new(),
        )
        .unwrap();
        assert!(!spy.called.get(), "--no-daemon suppresses the autostart");
    }

    #[test]
    fn a_failing_autostart_installer_still_yields_a_successful_bind() {
        let (_tmp, ws) = workspace();
        let meta = bind_join(&ws, "stream-one").unwrap();
        let spy = SpyAutostart::failing();
        let result = finish_bind_with(
            false,
            &ws,
            meta,
            "join",
            None,
            false,
            &spy,
            &SpyRegistrar::new(),
        );
        assert!(
            result.is_ok(),
            "the same best-effort rule the registry upsert already follows"
        );
        assert!(spy.called.get(), "the installer was still attempted");
    }

    #[test]
    fn a_successful_bind_calls_the_registrar() {
        // The registrar-side counterpart to `a_successful_bind_calls_the_autostart_installer`:
        // the seam must not silently stop registering — that would make the daemon blind to
        // newly bound workspaces, which is the whole reason the upsert exists.
        let (_tmp, ws) = workspace();
        let meta = bind_join(&ws, "stream-one").unwrap();
        let registrar = SpyRegistrar::new();
        finish_bind_with(
            false,
            &ws,
            meta,
            "join",
            None,
            true, // no_daemon: irrelevant to the registrar, which is never flag-gated
            &SpyAutostart::new(),
            &registrar,
        )
        .unwrap();
        assert!(registrar.called.get(), "a successful bind registers it");
    }

    #[test]
    fn a_failing_registrar_still_yields_a_successful_bind() {
        // Same best-effort discipline as the autostart installer: a registry-upsert failure
        // warns (stderr) but must never turn an otherwise-successful bind into a failure.
        let (_tmp, ws) = workspace();
        let meta = bind_join(&ws, "stream-one").unwrap();
        let registrar = SpyRegistrar::failing();
        let result = finish_bind_with(
            false,
            &ws,
            meta,
            "join",
            None,
            true,
            &SpyAutostart::new(),
            &registrar,
        );
        assert!(
            result.is_ok(),
            "the same best-effort rule the daemon autostart already follows"
        );
        assert!(registrar.called.get(), "the registrar was still attempted");
    }

    #[test]
    fn create_with_rebind_on_an_already_bound_workspace_mints_a_fresh_id_and_resets_watermarks() {
        let (_tmp, ws) = workspace();
        save(
            &ws,
            &SyncMeta {
                stream_id: "stream-one".into(),
                pushed_through: 42,
                pulled_through: 17,
                endpoint: Some("https://relay.example".into()),
            },
        )
        .unwrap();
        bind_with(
            false,
            &ws,
            BindOptions {
                create: true,
                join: None,
                endpoint: None,
                rebind: true,
                no_daemon: true,
                snapshot: None,
            },
            &SpyAutostart::new(),
            &SpyRegistrar::new(),
        )
        .unwrap();
        let meta = load(&ws).unwrap().unwrap();
        assert_ne!(meta.stream_id, "stream-one", "a fresh id was minted");
        assert!(slug::is_valid_stream_id(&meta.stream_id));
        assert_eq!((meta.pushed_through, meta.pulled_through), (0, 0));
        assert_eq!(
            meta.endpoint.as_deref(),
            Some("https://relay.example"),
            "the endpoint survives a stream switch unless explicitly replaced"
        );
    }
}
