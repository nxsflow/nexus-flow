//! The on-disk workspace: a `.nxs/` directory holding the store, the replica identity, and the
//! active-module config (spec §7). Commands discover it by walking up from the current directory
//! (git-style); [`init`] creates it. This is foundation-owned (the directory belongs to the
//! *platform*, not flow — the `.nexusflow` name was semantically wrong); a product opens ITS store
//! over the resolved workspace via its own `open_store` (flow's lives in `nexus-flow-facade`).
//!
//! Migration (spec §7): the legacy `.nexusflow/` directory is migrated to `.nxs/` idempotently on
//! open — [`discover`] renames it the first time it is resolved as the workspace (an ancestor
//! merely walked past is left untouched), a data-preserving atomic dir rename that is a no-op once
//! `.nxs/` exists, so opening a legacy workspace any number of times is harmless and never loses
//! data. ~3 existing workspaces are affected; each migrates on its next `nxf` invocation.

use crate::error::{NxfError, Result};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

/// The platform workspace directory (spec §7).
const WORKSPACE_DIR: &str = ".nxs";

/// The pre-§7 directory name, migrated to [`WORKSPACE_DIR`] on open. Kept only for the one-time
/// idempotent migration; nothing new is ever written under it.
const LEGACY_WORKSPACE_DIR: &str = ".nexusflow";

/// Stable per-replica identity, persisted at init. `site_id` feeds the CRDT op site;
/// `prefix` namespaces minted ids offline; `replica_uuid` is the durable, globally-unique
/// identity anchor (bab/§6) that tells two replicas with a *colliding* prefix apart — it
/// is what the prefix registry keys on, and unlike `prefix` it never changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replica {
    pub site_id: i64,
    pub prefix: String,
    /// `#[serde(default)]` so a pre-bab `replica.toml` (site+prefix only) still parses; an
    /// empty value is lazily minted and persisted on load (see [`load`]).
    #[serde(default)]
    pub replica_uuid: String,
}

impl Replica {
    /// [`display_id`] against this replica's own `prefix` — the CLI's id-display surface.
    pub fn display_id<'a>(&self, id: &'a str) -> &'a str {
        display_id(&self.prefix, id)
    }

    /// [`resolve_id`] against this replica's own `prefix` — the CLI's id-input surface.
    pub fn resolve_id<'a>(&self, input: &'a str) -> Cow<'a, str> {
        resolve_id(&self.prefix, input)
    }
}

/// Display form of an item id relative to `prefix` (nexus-flow-ykv). An id is internally atomic
/// `<prefix>.<suffix>`; the 4-char prefix is machine-specific and identical across every LOCAL
/// item, so it is pure noise to a reader. A LOCAL id — one whose prefix is `prefix`, at a `.`
/// boundary — is therefore shown by its bare suffix alone; a FOREIGN id (any other prefix, or an
/// id carrying no `<prefix>.` boundary) keeps its full form, because its suffix is ambiguous
/// against the local namespace. Returns a borrow of `id` — no allocation. Inverse of [`resolve_id`].
///
/// The machine contract is untouched: `--json` / the engine / an embedding app always emit the
/// full `<prefix>.<suffix>` (this helper drives only human CLI text + agent↔user communication).
pub fn display_id<'a>(prefix: &str, id: &'a str) -> &'a str {
    id.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('.'))
        .unwrap_or(id)
}

/// Resolve a user-supplied id against `prefix` (nexus-flow-ykv): a BARE id (no `.`) gets
/// `"<prefix>."` prepended, so a bare suffix always names the LOCAL item; an already-qualified
/// `<prefix>.<suffix>` (local OR foreign) is returned verbatim. The namespace is thus cleanly
/// partitioned — a bare id can only ever resolve locally, a foreign ticket always carries its
/// prefix. Inverse of [`display_id`].
pub fn resolve_id<'a>(prefix: &str, input: &'a str) -> Cow<'a, str> {
    if input.contains('.') {
        Cow::Borrowed(input)
    } else {
        Cow::Owned(format!("{prefix}.{input}"))
    }
}

/// The workspace config (spec §7): generic platform/active-modules config owned by the foundation,
/// plus a seam for product-owned config sections. `config.toml` carries it. Domain-agnostic — no
/// product's vocabulary or presentation lives here (aye.2.2): a product reads its own `[<module>]`
/// section over [`products`](Self::products) (flow's plugin selection + default live in
/// `nexus-flow-facade::workspace::flow_plugin`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Platform axis (spec §7): the product modules active in this workspace. Dormant in P1
    /// (flow-only); the `nxs` umbrella (P3) reads it to fan init/prime out over the active modules.
    #[serde(default)]
    pub active_modules: Vec<String>,
    /// Product-owned config sections, opaque to the foundation (the §7 seam). The foundation never
    /// interprets these — a product deserializes its own `[<module>]` table over the workspace.
    /// Flattened so a section appears as a top-level `[<module>]` table in `config.toml`. A legacy
    /// top-level `plugin = "..."` (pre-rework) is captured here too, where flow's reader finds it.
    ///
    /// RESERVED: the bare key `plugin` denotes the legacy scalar above and is read by
    /// `flow_plugin`'s fallback — a module must NOT name itself `plugin`, or its `[plugin]` table
    /// would shadow that fallback. Every other top-level key is a `[<module>]` section.
    #[serde(flatten)]
    pub products: toml::value::Table,
}

/// A resolved workspace: the workspace directory plus its parsed identity/config.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub dir: PathBuf,
    pub replica: Replica,
    pub config: WorkspaceConfig,
}

impl Workspace {
    pub fn db_path(&self) -> PathBuf {
        self.dir.join("db.sqlite")
    }

    /// The db path as UTF-8, or an `io` error — the shared check every store-open path needs.
    pub fn db_path_str(&self) -> Result<String> {
        self.db_path()
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| NxfError::io("workspace db path is not valid UTF-8"))
    }

    /// Resolve the active workspace: an explicit `--db`/`NXF_DB` override points at a
    /// db file (its parent dir is the workspace), otherwise walk up from `start`.
    ///
    /// The `--db` override loads its directory **in place** and does NOT run the legacy
    /// `.nexusflow/`→`.nxs/` migration: the caller named an explicit db path, and renaming the
    /// directory out from under it would invalidate that path. Only the discovery path migrates
    /// (a `--db` pointed at a legacy workspace is loaded as-is). The asymmetry is intentional.
    pub fn resolve(db_override: Option<&str>, start: &Path) -> Result<Workspace> {
        match db_override {
            Some(db) => {
                let dir = Path::new(db)
                    .parent()
                    .ok_or_else(|| NxfError::io(format!("--db path has no parent: {db}")))?
                    .to_path_buf();
                load(&dir)
            }
            None => discover(start),
        }
    }
}

/// Walk up from `start` looking for the nearest workspace directory with a replica file. A legacy
/// `.nexusflow/` is migrated to `.nxs/` in place **only at the level it is resolved as the
/// workspace** (spec §7) — never at an ancestor merely walked PAST, so an unrelated parent
/// `.nexusflow/` (e.g. another tool's `$HOME/.nexusflow`) is left untouched whenever a closer
/// workspace exists.
pub fn discover(start: &Path) -> Result<Workspace> {
    let mut cur = Some(start);
    while let Some(dir) = cur {
        // A `.nxs/` here is the workspace — it takes precedence over a sibling legacy dir.
        if dir.join(WORKSPACE_DIR).join("replica.toml").is_file() {
            return load(&dir.join(WORKSPACE_DIR));
        }
        // Otherwise a legacy `.nexusflow/` here is the workspace we resolve at THIS level: migrate
        // it in place and use it. We reach an ancestor's legacy dir only when no closer workspace
        // exists, in which case it genuinely IS the workspace nxf would open (so migrating it is
        // correct, not a stray side-effect on an unrelated dir).
        if dir
            .join(LEGACY_WORKSPACE_DIR)
            .join("replica.toml")
            .is_file()
        {
            migrate_legacy_dir(dir)?;
            return load(&dir.join(WORKSPACE_DIR));
        }
        cur = dir.parent();
    }
    Err(NxfError::no_workspace(format!(
        "no {WORKSPACE_DIR} workspace found from {}; run `{}`",
        start.display(),
        init_hint()
    )))
}

/// The persona the multicall dispatcher routed to, when it told us. `nxs flow|memory|chat …`
/// synthesizes the sub-tool's argv[0] so its parser reports the persona's name; the PROCESS argv
/// still reads `nxs`, so the routed and direct forms would otherwise disagree — and they are
/// contractually byte-identical.
static INVOKED_PERSONA: AtomicU8 = AtomicU8::new(PERSONA_UNSET);
const PERSONA_UNSET: u8 = 0;

/// Record which persona this process is acting as. Called by the multicall dispatcher for every
/// route it takes; an embedder that never calls it falls back to `argv[0]`.
pub fn set_invoked_persona(program: &str) {
    INVOKED_PERSONA.store(persona_code(program), Ordering::Relaxed);
}

/// The init command to suggest when no workspace was found: the invoked persona's OWN init.
///
/// Each persona's init registers ITS module — `nxf init` activates flow, `nxm init` memory, `nxc
/// init` chat. Suggesting `nxf init` to an `nxm` user therefore produced a workspace their module
/// was never registered in: the command they retried appeared to work (the store is shared), but
/// `nxs prime` fanned out nothing for memory, so nothing was ever replayed at session start.
fn init_hint() -> &'static str {
    match INVOKED_PERSONA.load(Ordering::Relaxed) {
        PERSONA_UNSET => {
            // No dispatcher told us: fall back to `argv[0]`, for the same reason the multicall
            // dispatcher reads it there — `current_exe()` resolves the symlink back to `nxs` and
            // loses the persona.
            let prog = std::env::args_os()
                .next()
                .and_then(|a| a.to_str().map(str::to_owned));
            let base = prog
                .as_deref()
                .and_then(|p| Path::new(p).file_name()?.to_str())
                .map(|b| b.strip_suffix(".exe").unwrap_or(b))
                .unwrap_or("nxs");
            init_hint_for(base)
        }
        code => init_hint_for(persona_name(code)),
    }
}

/// The persona names, indexed by their stored code. Index 0 is [`PERSONA_UNSET`] and never read
/// back as a name; keeping it in the table makes the two directions one list.
const PERSONA_NAMES: [&str; 4] = ["nxs", "nxf", "nxm", "nxc"];

fn persona_code(program: &str) -> u8 {
    match PERSONA_NAMES.iter().position(|&p| p == program) {
        // `nxs` sits at index 0, which doubles as UNSET — map it to the umbrella explicitly so
        // setting it is not silently the same as never setting it.
        Some(0) | None => PERSONA_NAMES.len() as u8,
        Some(i) => i as u8,
    }
}

fn persona_name(code: u8) -> &'static str {
    PERSONA_NAMES.get(code as usize).copied().unwrap_or("nxs")
}

/// [`init_hint`]'s pure core, split out so it is testable without touching the process argv.
fn init_hint_for(program: &str) -> &'static str {
    match program {
        "nxf" => "nxf init",
        "nxm" => "nxm init",
        "nxc" => "nxc init",
        // The umbrella and anything unrecognised: `nxs init` sets up whichever tools the user
        // picks, so it is the one answer that is never wrong.
        _ => "nxs init",
    }
}

/// Detect an ORPHANED legacy `.nexusflow/` directly under `root`: one with a `db.sqlite` but NO
/// `replica.toml` — e.g. a crash mid-init left it half-written (nexus-flow-aye.2.3). Migration
/// ([`migrate_legacy_dir`]) only renames a legacy dir that HAS a replica, so an orphan is silently
/// left behind: [`discover`] fails loudly (`no_workspace`) rather than trusting stale data, and a
/// fresh [`setup`] would create a new `.nxs/` one directory over, stranding the old db. The
/// foundation only DETECTS the orphan (a pure read, no stderr — it is a library); the CLI surfaces
/// the warning. Returns the orphan's path so the warning can name it.
///
/// Scope is intentionally narrow (warn-don't-delete): only `root` itself is probed (init creates
/// the workspace there, so a stranded sibling is the only concern — not an ancestor's legacy dir),
/// and a present-but-EMPTY legacy dir is ignored (a db-less directory strands no data). Both are by
/// design, not an oversight.
pub fn orphaned_legacy_dir(root: &Path) -> Option<PathBuf> {
    let legacy = root.join(LEGACY_WORKSPACE_DIR);
    let has_db = legacy.join("db.sqlite").is_file();
    let has_replica = legacy.join("replica.toml").is_file();
    (has_db && !has_replica).then_some(legacy)
}

/// Idempotent legacy-read (spec §7): if `parent` holds a `.nexusflow/` workspace and no `.nxs/`
/// yet, rename it to `.nxs/`. An atomic same-filesystem dir rename preserves every file (db,
/// replica, config, .gitignore) — no copy, no data loss. A no-op once `.nxs/` exists, so opening a
/// migrated (or already-`.nxs`) workspace repeatedly is harmless; a concurrent racer that loses the
/// rename simply finds the `.nxs/` the winner produced.
fn migrate_legacy_dir(parent: &Path) -> Result<()> {
    let legacy = parent.join(LEGACY_WORKSPACE_DIR);
    let target = parent.join(WORKSPACE_DIR);
    if legacy.join("replica.toml").is_file() && !target.exists() {
        std::fs::rename(&legacy, &target).map_err(|e| {
            NxfError::io(format!(
                "migrating {LEGACY_WORKSPACE_DIR} → {WORKSPACE_DIR} at {}: {e}",
                parent.display()
            ))
        })?;
    }
    Ok(())
}

/// Load a workspace from an existing workspace directory.
fn load(dir: &Path) -> Result<Workspace> {
    // Name the file by its FULL path: `--db` resolves the workspace next to the db file, so a
    // bare `replica.toml` left the reader with a filename they never typed and cannot locate.
    let replica_path = dir.join("replica.toml");
    let replica_raw = std::fs::read_to_string(&replica_path)
        .map_err(|e| NxfError::io(format!("reading {}: {e}", replica_path.display())))?;
    let mut replica: Replica = toml::from_str(&replica_raw)
        .map_err(|e| NxfError::io(format!("parsing {}: {e}", replica_path.display())))?;
    // Lazy migration (bab): a pre-bab workspace has no replica_uuid. Mint one and persist it ONCE
    // so the identity is durable and stable across later loads — re-minting on each load would
    // defeat the registry's whole purpose.
    if replica.replica_uuid.is_empty() {
        replica.replica_uuid = fresh_replica_uuid();
        let toml = toml::to_string(&replica)
            .map_err(|e| NxfError::io(format!("serializing migrated replica: {e}")))?;
        std::fs::write(dir.join("replica.toml"), toml)
            .map_err(|e| NxfError::io(format!("persisting migrated replica_uuid: {e}")))?;
    }
    let config = match std::fs::read_to_string(dir.join("config.toml")) {
        Ok(raw) => {
            toml::from_str(&raw).map_err(|e| NxfError::io(format!("parsing config.toml: {e}")))?
        }
        Err(_) => WorkspaceConfig::default(),
    };
    Ok(Workspace {
        dir: dir.to_path_buf(),
        replica,
        config,
    })
}

/// Create a fresh workspace rooted at `start` with `config` (the caller's deliberate, validated
/// platform + product config). Fails loudly if `start` already lives inside a workspace, then
/// delegates the actual `.nxs/` creation to the idempotent [`setup`]. Domain-agnostic — the product
/// builds `config` (e.g. flow seats its plugin via `flow_config`); this just enforces no nesting.
pub fn init(start: &Path, config: &WorkspaceConfig) -> Result<Workspace> {
    if let Ok(existing) = discover(start) {
        return Err(NxfError::new(
            crate::error::ErrorKind::Conflict,
            format!("already inside a workspace at {}", existing.dir.display()),
        ));
    }
    setup(start, config)
}

/// Idempotent foundation library seam (spec §6.1): ensure a `.nxs/` workspace exists directly under
/// `root` and return it. **Every** init path calls this (`nxf init` today; `nxm`/`nxs init` later),
/// so a flow-only user needs no `nxs` binary — `nexus-flow` links the foundation and calls setup
/// directly.
///
/// On a fresh root it creates the directory, its self-ignore, a fresh replica identity,
/// `config.toml` (from `config`), and materializes the substrate `db.sqlite` (the op-log +
/// schema-version stamp). A legacy `.nexusflow/` *with a replica* at `root` is migrated to `.nxs/`
/// in place first (consistent with [`discover`]). An existing workspace is loaded UNCHANGED —
/// setup never clobbers identity or config, so re-running it (or a second product's init) is safe;
/// the `config` argument seeds a FRESH workspace only.
///
/// Domain-agnostic (D5): setup writes the platform config it is given and the substrate db; a
/// product's own materialized views are created when it opens ITS store over the returned
/// workspace. Refusing a *nested* workspace is the product `init`'s policy (see [`init`]), not
/// setup's — setup is pure "ensure".
pub fn setup(root: &Path, config: &WorkspaceConfig) -> Result<Workspace> {
    let dir = root.join(WORKSPACE_DIR);
    // Idempotent: an existing `.nxs/` is loaded unchanged…
    if dir.join("replica.toml").is_file() {
        return ensure_substrate_db(load(&dir)?);
    }
    // …and a legacy `.nexusflow/` with a replica at this root is migrated in place, then loaded.
    if root
        .join(LEGACY_WORKSPACE_DIR)
        .join("replica.toml")
        .is_file()
    {
        migrate_legacy_dir(root)?;
        return ensure_substrate_db(load(&dir)?);
    }

    // Fresh workspace. `create_dir_all` is a no-op on a pre-existing (e.g. crash-partial) `.nxs/`,
    // so setup can complete a half-written directory rather than fail on it.
    std::fs::create_dir_all(&dir)
        .map_err(|e| NxfError::io(format!("creating {WORKSPACE_DIR}: {e}")))?;

    // Self-ignore (ha4): write the workspace `.gitignore` = `*` (the cargo `target/` pattern) so
    // `db.sqlite` (binary churn) and especially `replica.toml` (the replica IDENTITY — two clones
    // sharing it would remint the same prefix and revive the cross-replica collision class
    // E4/bab eliminated) are never committed. We self-ignore rather than touch the project's root
    // `.gitignore`, keeping the workspace self-contained.
    std::fs::write(dir.join(".gitignore"), "*\n")
        .map_err(|e| NxfError::io(format!("writing {WORKSPACE_DIR}/.gitignore: {e}")))?;

    let replica = fresh_replica();
    let replica_toml =
        toml::to_string(&replica).map_err(|e| NxfError::io(format!("serializing replica: {e}")))?;
    std::fs::write(dir.join("replica.toml"), replica_toml)
        .map_err(|e| NxfError::io(format!("writing replica.toml: {e}")))?;

    let config_toml =
        toml::to_string(config).map_err(|e| NxfError::io(format!("serializing config: {e}")))?;
    std::fs::write(dir.join("config.toml"), config_toml)
        .map_err(|e| NxfError::io(format!("writing config.toml: {e}")))?;

    ensure_substrate_db(Workspace {
        dir,
        replica,
        config: config.clone(),
    })
}

/// Materialize the substrate `db.sqlite` for `ws` (idempotent): open the foundation store — which
/// creates the `ops` log and stamps the schema version — then drop it. A product's own views are
/// added when it opens ITS store over the same file, so this is the foundation half of db creation.
fn ensure_substrate_db(ws: Workspace) -> Result<Workspace> {
    let path = ws.db_path_str()?;
    crate::store::Store::open(&path, ws.replica.site_id)
        .map_err(|e| NxfError::io(format!("materializing substrate db at {path}: {e}")))?;
    Ok(ws)
}

/// Adopt a reassigned prefix (bab/T-remap): after the registry reassigns this replica a new prefix
/// and the local store is remapped, persist the new `prefix` so every future mint uses it.
/// `site_id` and `replica_uuid` are unchanged — only the display prefix moves.
pub fn adopt_prefix(ws: &Workspace, new_prefix: &str) -> Result<()> {
    let mut replica = ws.replica.clone();
    replica.prefix = new_prefix.to_string();
    let toml =
        toml::to_string(&replica).map_err(|e| NxfError::io(format!("serializing replica: {e}")))?;
    std::fs::write(ws.dir.join("replica.toml"), toml)
        .map_err(|e| NxfError::io(format!("persisting adopted prefix: {e}")))
}

/// Idempotently register `module` in the workspace's `active_modules` (spec §6.3). `dir` is the
/// workspace directory (where `config.toml` lives). Re-reads the config, appends `module` if
/// absent, and rewrites it; product-owned sections (the §7 seam, e.g. flow's `[flow]`) are
/// preserved verbatim. Returns whether it was newly added.
///
/// This is the seam a SECOND product's `init` uses to join an EXISTING workspace: `setup` loads an
/// existing config UNCHANGED (it never clobbers), so `nxm init` over a flow workspace registers
/// `memory` here. The P3 `nxs` umbrella reads `active_modules` to fan init/prime out over modules.
pub fn activate_module(dir: &Path, module: &str) -> Result<bool> {
    let path = dir.join("config.toml");
    let mut config = read_config_for_update(&path)?;
    if config.active_modules.iter().any(|m| m == module) {
        return Ok(false);
    }
    config.active_modules.push(module.to_string());
    let toml =
        toml::to_string(&config).map_err(|e| NxfError::io(format!("serializing config: {e}")))?;
    // Atomic temp+rename (consistent with the agent-file/settings writers): a crash mid-write can
    // never truncate a pre-existing config.toml — it has either the old bytes or the new ones.
    atomic_write(&path, toml.as_bytes())
        .map_err(|e| NxfError::io(format!("writing config.toml: {e}")))?;
    Ok(true)
}

/// Set one key inside a product-owned section of the workspace config (the §7 seam), creating the
/// section if it is not there yet. `dir` is the workspace directory (where `config.toml` lives).
///
/// The sibling of [`activate_module`], and it shares that function's two properties for the same
/// reasons: everything it does not touch — `active_modules`, every OTHER product section — is
/// preserved verbatim, and the rewrite is atomic. Returns whether the file changed, so a caller
/// that writes the same value on every run does not rewrite it every time.
///
/// A `section` that is already present as something OTHER than a table is a loud error naming it,
/// never an overwrite: the foundation reserves the bare key `plugin` for a pre-rework workspace's
/// legacy scalar, and turning that into a table would take flow's plugin choice with it.
pub fn set_product_key(dir: &Path, section: &str, key: &str, value: toml::Value) -> Result<bool> {
    let path = dir.join("config.toml");
    let mut config = read_config_for_update(&path)?;
    let existing = config.products.get(section);
    if let Some(present) = existing {
        if !present.is_table() {
            return Err(NxfError::validation(format!(
                "config.toml already carries `{section}` as a {}, not a section — refusing to \
                 overwrite it",
                present.type_str()
            )));
        }
        if present.get(key) == Some(&value) {
            return Ok(false);
        }
    }
    config
        .products
        .entry(section.to_string())
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
        .as_table_mut()
        // Checked above: a non-table `section` returned already.
        .expect("the section is a table")
        .insert(key.to_string(), value);
    let toml =
        toml::to_string(&config).map_err(|e| NxfError::io(format!("serializing config: {e}")))?;
    atomic_write(&path, toml.as_bytes())
        .map_err(|e| NxfError::io(format!("writing config.toml: {e}")))?;
    Ok(true)
}

/// **Neither writer is safe against a CONCURRENT writer** (review of PR #419, Integrity #1), and
/// that is a known, unclosed gap rather than an oversight. Both read this file, change one thing in
/// memory and write the whole of it back; the write itself is atomic (temp + rename), so the loser
/// of a race loses its own change rather than corrupting the file — a lost update, never a torn
/// one. Two `nxs` processes writing one workspace's config at the same moment is what it takes, and
/// nothing in the suite serialises them. Closing it needs the same compare-and-swap on both, and it
/// is tracked as `nxf 6j6v.wak2` rather than half-done on one of them here.
///
/// Read `config.toml` for a read-modify-write, or an empty config when there is genuinely none yet.
///
/// **Only `NotFound` is an empty config** (review of PR #419, Integrity #2). Both writers used to
/// swallow every read error into `WorkspaceConfig::default()` — and the next thing each of them
/// does is WRITE that back. So a config that could not be read for any other reason (a permission
/// bit, a failing disk) was replaced by an empty one, taking `active_modules` and every product
/// section with it, and the call returned `Ok`. A first write into a workspace that has no
/// `config.toml` yet is the one case that legitimately starts from nothing, and it is exactly the
/// one `NotFound` names.
fn read_config_for_update(path: &Path) -> Result<WorkspaceConfig> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(WorkspaceConfig::default()),
        Err(e) => {
            return Err(NxfError::io(format!(
                "reading {} for update: {e}",
                path.display()
            )))
        }
    };
    toml::from_str(&raw).map_err(|e| NxfError::io(format!("parsing config.toml: {e}")))
}

/// Write `contents` to `path` atomically: write a sibling temp file, then rename it over the
/// target. A same-directory rename is atomic, so a reader (or a crash) only ever sees the whole
/// old file or the whole new one — never a truncated mix. Mirrors the agent-file/settings writers.
fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(".config.toml.nxs.tmp.{}", std::process::id()));
    let result = std::fs::write(&tmp, contents).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp); // best-effort: never strand the temp sibling
    }
    result
}

/// Test/docs determinism switch (nexus-flow-4oa.2). When `NXF_DETERMINISTIC_IDS` is set, a fresh
/// workspace adopts a FIXED replica identity and `create` mints sequential ids, so the golden
/// documentation examples produce byte-stable output across runs and machines. Harness-only —
/// production never sets it; defined here once so both `init` (identity) and the product's id
/// minting read the same flag.
pub fn deterministic_ids_enabled() -> bool {
    std::env::var_os("NXF_DETERMINISTIC_IDS").is_some()
}

/// The fixed replica identity used under [`deterministic_ids_enabled`]. The prefix is the
/// `ab12.…` shape used throughout the docs/examples; `site_id` and `replica_uuid` are stable
/// constants (the uuid is never surfaced in any command output, only the prefix and site are).
fn deterministic_replica() -> Replica {
    Replica {
        site_id: 1,
        prefix: "ab12".to_string(),
        replica_uuid: "00000000000000000000000abc".to_string(),
    }
}

/// Mint a fresh replica identity. Random (ULID entropy) in production; a fixed, reproducible
/// identity under the golden-docs determinism switch (see [`deterministic_ids_enabled`]).
fn fresh_replica() -> Replica {
    if deterministic_ids_enabled() {
        return deterministic_replica();
    }
    let ulid = ulid::Ulid::new();
    let bits: u128 = ulid.into();
    let site_id = (bits as u64 & i64::MAX as u64) as i64;
    let s = ulid.to_string().to_ascii_lowercase();
    let prefix = s[s.len() - 4..].to_string();
    Replica {
        site_id,
        prefix,
        replica_uuid: fresh_replica_uuid(),
    }
}

/// A fresh, globally-unique replica identity anchor — a full ULID (lowercased). Distinct from
/// `prefix` (4 chars, reassignable) and `site_id` (the CRDT op site).
fn fresh_replica_uuid() -> String {
    ulid::Ulid::new().to_string().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A config carrying one product section — the foundation stays agnostic about what the
    /// section means, so tests assert only that it round-trips through the seam verbatim.
    fn cfg_with_section(module: &str, key: &str, val: &str) -> WorkspaceConfig {
        let mut section = toml::value::Table::new();
        section.insert(key.into(), toml::Value::String(val.into()));
        let mut products = toml::value::Table::new();
        products.insert(module.into(), toml::Value::Table(section));
        WorkspaceConfig {
            active_modules: vec![module.into()],
            products,
        }
    }

    /// The "no workspace" error must name the init of the persona the user actually invoked.
    /// Naming `nxf init` for every persona sent an `nxm`/`nxc` user to flow's init, which creates
    /// the workspace but leaves their module UNREGISTERED — `nxs prime` then replays nothing for
    /// it, which is the whole point of the product they were using.
    #[test]
    fn init_hint_names_the_invoked_personas_own_init() {
        assert_eq!(init_hint_for("nxf"), "nxf init");
        assert_eq!(init_hint_for("nxm"), "nxm init");
        assert_eq!(init_hint_for("nxc"), "nxc init");
        // The umbrella, and anything unrecognised (a test harness, a renamed binary), get the
        // umbrella's own init — it sets up whichever tools the user picks, so it is never wrong.
        assert_eq!(init_hint_for("nxs"), "nxs init");
        assert_eq!(init_hint_for("some-embedding-app"), "nxs init");
    }

    /// `INVOKED_PERSONA` is process-global, so a test that writes it must serialize against every
    /// other test in this binary that reads it and put it back afterwards — the same reason
    /// `crates/chat/tests/worker.rs` guards the env with `ENV_LOCK`. Restoring happens in `Drop`
    /// rather than after the call, so a failing assertion cannot leave the global stuck for every
    /// later test (PR #326 review, Code Quality #1). A poisoned mutex is recovered, not
    /// propagated, so one failure does not cascade.
    static PERSONA_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_persona<T>(program: &str, f: impl FnOnce() -> T) -> T {
        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                INVOKED_PERSONA.store(PERSONA_UNSET, Ordering::Relaxed);
            }
        }
        let _lock = PERSONA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Declared AFTER the lock, so it drops BEFORE it: the global is restored while this test
        // still holds exclusive access, never in the window where the next one has taken over.
        let _restore = Restore;
        set_invoked_persona(program);
        f()
    }

    /// `nxs chat …` must be byte-identical to `nxc …` (the multicall contract). The routed form
    /// synthesizes argv[0] for the sub-tool's parser, so the process argv still reads `nxs` —
    /// reading it here would make the routed error name a different init than the direct one.
    #[test]
    fn an_explicitly_set_persona_beats_the_process_argv() {
        with_persona("nxc", || assert_eq!(init_hint(), "nxc init"));
        with_persona("nxf", || assert_eq!(init_hint(), "nxf init"));
    }

    /// `nxs` sits at index 0 of [`PERSONA_NAMES`], which is also the `PERSONA_UNSET` sentinel, so
    /// `persona_code` maps it away deliberately. Without that, explicitly declaring the umbrella
    /// would be indistinguishable from never declaring anything — it would still answer
    /// `nxs init` via the argv[0] fallback, but only by luck, and would silently start following
    /// argv the moment the binary were invoked under another name.
    #[test]
    fn declaring_the_umbrella_is_distinct_from_declaring_nothing() {
        assert_ne!(persona_code("nxs"), PERSONA_UNSET);
        with_persona("nxs", || assert_eq!(init_hint(), "nxs init"));
        // An unrecognised program name lands on the umbrella too, and is likewise not UNSET.
        assert_ne!(persona_code("some-embedding-app"), PERSONA_UNSET);
        with_persona("some-embedding-app", || assert_eq!(init_hint(), "nxs init"));
    }

    #[test]
    fn no_workspace_error_carries_the_init_hint() {
        let tmp = TempDir::new().unwrap();
        let err = discover(tmp.path()).expect_err("empty dir has no workspace");
        let msg = err.to_string();
        assert!(msg.contains("no .nxs workspace found"), "{msg}");
        assert!(msg.contains(" init`"), "names an init command: {msg}");
    }

    /// A failure to read the replica must name the file by its FULL path. `reading replica.toml:
    /// No such file or directory` gave a first-time user a filename they cannot locate and never
    /// typed — the `--db` path in particular resolves the workspace next to the db file.
    #[test]
    fn replica_read_error_names_the_full_path() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("nowhere");
        let err = load(&missing).expect_err("no replica there");
        let msg = err.to_string();
        assert!(
            msg.contains(&missing.join("replica.toml").display().to_string()),
            "error names the full path it tried: {msg}"
        );
    }

    #[test]
    fn discover_finds_workspace_from_subdir() {
        let tmp = TempDir::new().unwrap();
        init(
            tmp.path(),
            &cfg_with_section("flow", "plugin", "issue-tracker"),
        )
        .unwrap();

        let sub = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        let ws = discover(&sub).expect("found from subdir");
        assert_eq!(ws.dir, tmp.path().join(WORKSPACE_DIR));
        assert_eq!(ws.config.active_modules, vec!["flow".to_string()]);
        assert_eq!(
            ws.config.products["flow"]["plugin"].as_str(),
            Some("issue-tracker")
        );
    }

    #[test]
    fn init_writes_config_and_reloads_it() {
        let tmp = TempDir::new().unwrap();
        let ws = init(
            tmp.path(),
            &cfg_with_section("flow", "plugin", "personal-todo"),
        )
        .unwrap();
        assert_eq!(
            ws.config.products["flow"]["plugin"].as_str(),
            Some("personal-todo")
        );
        let reloaded = discover(tmp.path()).unwrap();
        assert_eq!(
            reloaded.config.products["flow"]["plugin"].as_str(),
            Some("personal-todo")
        );
    }

    #[test]
    fn init_self_ignores_the_workspace_dir() {
        let tmp = TempDir::new().unwrap();
        let ws = init(tmp.path(), &WorkspaceConfig::default()).unwrap();
        let gitignore = ws.dir.join(".gitignore");
        assert!(gitignore.is_file(), "init writes the workspace .gitignore");
        assert_eq!(std::fs::read_to_string(gitignore).unwrap(), "*\n");
    }

    #[test]
    fn discover_without_workspace_is_no_workspace_error() {
        let tmp = TempDir::new().unwrap();
        let err = discover(tmp.path()).unwrap_err();
        assert_eq!(err.kind, crate::error::ErrorKind::NoWorkspace);
    }

    #[test]
    fn init_inside_existing_workspace_is_rejected() {
        let tmp = TempDir::new().unwrap();
        init(tmp.path(), &WorkspaceConfig::default()).unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let err = init(&sub, &WorkspaceConfig::default()).unwrap_err();
        assert_eq!(err.kind, crate::error::ErrorKind::Conflict);
    }

    #[test]
    fn deterministic_replica_is_fixed_and_well_formed() {
        let a = deterministic_replica();
        let b = deterministic_replica();
        assert_eq!(a.site_id, b.site_id);
        assert_eq!(a.prefix, b.prefix);
        assert_eq!(a.replica_uuid, b.replica_uuid);
        // Matches the documented example shape (`ab12.…`): a 4-char lowercase-alnum prefix.
        assert_eq!(a.prefix, "ab12");
        assert!(a.prefix.len() == 4 && a.prefix.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(a.site_id, 1);
        assert!(!a.replica_uuid.is_empty());
    }

    #[test]
    fn fresh_replicas_differ() {
        let a = fresh_replica();
        let b = fresh_replica();
        assert!(a.site_id >= 0 && b.site_id >= 0);
        assert!(a.prefix.len() == 4 && b.prefix.len() == 4);
        assert!(!a.replica_uuid.is_empty() && !b.replica_uuid.is_empty());
        assert_ne!(a.replica_uuid, b.replica_uuid);
    }

    #[test]
    fn init_persists_a_replica_uuid() {
        let tmp = TempDir::new().unwrap();
        let ws = init(tmp.path(), &WorkspaceConfig::default()).unwrap();
        assert!(!ws.replica.replica_uuid.is_empty());
        let again = discover(tmp.path()).unwrap();
        assert_eq!(again.replica.replica_uuid, ws.replica.replica_uuid);
    }

    #[test]
    fn a_replica_uuid_is_opaque_and_persists_whatever_shape_it_has() {
        // Forward-compat invariant 2 (6j6v.xsf3): the uuid is a random ULID *today*, but nothing
        // may assume that shape. The E4 auth slice (6j6v.6aza) anchors a per-replica Ed25519
        // keypair at this identity; it has to be able to do so ADDITIVELY — writing a key-derived
        // identity into an EXISTING workspace, or carrying one in from a device that already has
        // a key — without re-minting the replica and orphaning everything it has already signed
        // or registered. So the loader must persist and return it verbatim, never normalize,
        // validate or re-mint it.
        let key_shaped = "ed25519:MCowBQYDK2VwAyEA6r7Vv0mQ3wLcC1xN9pKfZ8hQ2sT4uY6bW8aE0gJnHkI=";
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(WORKSPACE_DIR);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(
            dir.join("replica.toml"),
            format!("site_id = 7\nprefix = \"abcd\"\nreplica_uuid = \"{key_shaped}\"\n"),
        )
        .unwrap();
        std::fs::write(dir.join("config.toml"), "").unwrap();

        let ws = discover(tmp.path()).expect("opens a workspace whose identity is key-shaped");
        assert_eq!(ws.replica.replica_uuid, key_shaped, "returned verbatim");
        // And it is not re-minted on the next open — the lazy pre-bab migration only fires on an
        // EMPTY uuid, so an identity the replica already has is never replaced underneath it.
        let again = discover(tmp.path()).unwrap();
        assert_eq!(
            again.replica.replica_uuid, key_shaped,
            "stable across opens"
        );
        assert!(
            std::fs::read_to_string(dir.join("replica.toml"))
                .unwrap()
                .contains(key_shaped),
            "and stayed that way on disk"
        );
    }

    #[test]
    fn legacy_nexusflow_is_migrated_to_nxs_on_discover() {
        // §7: a pre-rename `.nexusflow/` workspace is migrated to `.nxs/` in place on the first
        // open, preserving every file; migration is idempotent (open again → no-op, no data loss).
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join(LEGACY_WORKSPACE_DIR);
        std::fs::create_dir(&legacy).unwrap();
        std::fs::write(
            legacy.join("replica.toml"),
            "site_id = 7\nprefix = \"abcd\"\nreplica_uuid = \"uuid-legacy\"\n",
        )
        .unwrap();
        std::fs::write(legacy.join("config.toml"), "plugin = \"personal-todo\"\n").unwrap();
        std::fs::write(legacy.join("db.sqlite"), b"sentinel-db-bytes").unwrap();
        std::fs::write(legacy.join(".gitignore"), "*\n").unwrap();

        let ws = discover(tmp.path()).expect("discovers + migrates the legacy workspace");
        assert_eq!(
            ws.dir,
            tmp.path().join(WORKSPACE_DIR),
            "resolved under the new name"
        );
        assert!(
            !tmp.path().join(LEGACY_WORKSPACE_DIR).exists(),
            "legacy dir is gone (renamed, not copied)"
        );
        assert!(
            tmp.path().join(WORKSPACE_DIR).is_dir(),
            ".nxs now holds the workspace"
        );
        // Identity + config survived the move.
        assert_eq!(ws.replica.site_id, 7);
        assert_eq!(ws.replica.prefix, "abcd");
        assert_eq!(ws.replica.replica_uuid, "uuid-legacy");
        // A legacy top-level `plugin = "..."` is captured by the product seam (flow's reader
        // finds it there); the foundation itself stays plugin-agnostic.
        assert_eq!(ws.config.products["plugin"].as_str(), Some("personal-todo"));
        // The db file moved byte-for-byte (no data loss).
        assert_eq!(
            std::fs::read(tmp.path().join(WORKSPACE_DIR).join("db.sqlite")).unwrap(),
            b"sentinel-db-bytes"
        );

        // Idempotent: discovering again is a clean no-op (already `.nxs`).
        let again = discover(tmp.path()).unwrap();
        assert_eq!(again.dir, tmp.path().join(WORKSPACE_DIR));
        assert_eq!(again.replica.replica_uuid, "uuid-legacy");
    }

    // ---- aye.11: config generalization (active modules + product seam) -----

    #[test]
    fn workspace_config_carries_active_modules_and_a_product_seam() {
        // §7: config.toml = generic platform/active-modules config PLUS a seam for product-owned
        // sections, so the foundation stays domain-agnostic. It must round-trip through TOML with a
        // product table present (the serde-flatten-into-TOML ordering is the real risk this guards).
        let cfg = cfg_with_section("flow", "plugin", "personal-todo");
        let serialized = toml::to_string(&cfg).expect("config serializes to valid TOML");
        let back: WorkspaceConfig = toml::from_str(&serialized).expect("config round-trips");
        assert_eq!(back.active_modules, vec!["flow".to_string()]);
        assert_eq!(
            back.products["flow"]["plugin"].as_str(),
            Some("personal-todo"),
            "the product section is preserved verbatim through the seam"
        );
    }

    #[test]
    fn workspace_config_reads_a_legacy_plugin_only_config() {
        // Existing workspaces hold just `plugin = "..."`; they must still parse (the foundation
        // captures it in the product seam, active_modules defaults empty) — shipped workspaces
        // read unchanged. flow's reader resolves the plugin from here.
        let cfg: WorkspaceConfig = toml::from_str("plugin = \"personal-todo\"\n").unwrap();
        assert_eq!(cfg.products["plugin"].as_str(), Some("personal-todo"));
        assert!(cfg.active_modules.is_empty());
    }

    // ---- aye.2.3: orphaned legacy dir detection ----------------------------

    #[test]
    fn orphaned_legacy_dir_is_some_for_a_partial_legacy_dir() {
        // A crash mid-init can leave `.nexusflow/` with a db but no replica.toml — migration skips
        // it (no replica), so it is orphaned. The foundation only DETECTS it; the CLI warns.
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join(LEGACY_WORKSPACE_DIR);
        std::fs::create_dir(&legacy).unwrap();
        std::fs::write(legacy.join("db.sqlite"), b"stale").unwrap();
        assert_eq!(orphaned_legacy_dir(tmp.path()), Some(legacy));
    }

    #[test]
    fn orphaned_legacy_dir_is_none_when_a_replica_is_present() {
        // A legacy dir WITH a replica is migratable (discover/setup handle it) — not orphaned.
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join(LEGACY_WORKSPACE_DIR);
        std::fs::create_dir(&legacy).unwrap();
        std::fs::write(legacy.join("db.sqlite"), b"stale").unwrap();
        std::fs::write(
            legacy.join("replica.toml"),
            "site_id = 1\nprefix = \"ab12\"\n",
        )
        .unwrap();
        assert_eq!(orphaned_legacy_dir(tmp.path()), None);
    }

    #[test]
    fn orphaned_legacy_dir_is_none_without_a_db_or_a_legacy_dir() {
        let tmp = TempDir::new().unwrap();
        // No legacy dir at all.
        assert_eq!(orphaned_legacy_dir(tmp.path()), None);
        // A legacy dir with neither db nor replica (nothing stranded) is not flagged.
        std::fs::create_dir(tmp.path().join(LEGACY_WORKSPACE_DIR)).unwrap();
        assert_eq!(orphaned_legacy_dir(tmp.path()), None);
    }

    // ---- aye.11: foundation::setup() ---------------------------------------

    #[test]
    fn setup_creates_a_fresh_workspace_with_a_substrate_db() {
        let tmp = TempDir::new().unwrap();
        let ws = setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        assert_eq!(ws.dir, tmp.path().join(WORKSPACE_DIR));
        assert!(ws.dir.join("replica.toml").is_file(), "replica.toml");
        assert!(ws.dir.join("config.toml").is_file(), "config.toml");
        assert!(ws.dir.join(".gitignore").is_file(), "self-ignore");
        assert!(
            ws.dir.join("db.sqlite").is_file(),
            "setup materializes the substrate db.sqlite"
        );
        // The db carries the substrate schema (ops) + the version stamp (spec §4.3).
        let conn = rusqlite::Connection::open(ws.dir.join("db.sqlite")).unwrap();
        assert_eq!(
            crate::schema::schema_version(&conn).unwrap(),
            crate::schema::SCHEMA_VERSION,
            "db is stamped at the current schema version"
        );
        let has_ops: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='ops'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_ops, 1, "the op-log substrate exists");
    }

    #[test]
    fn setup_is_idempotent_and_never_clobbers_an_existing_workspace() {
        let tmp = TempDir::new().unwrap();
        let cfg = cfg_with_section("flow", "plugin", "personal-todo");
        let first = setup(tmp.path(), &cfg).unwrap();
        let uuid = first.replica.replica_uuid.clone();
        // A second setup with a DIFFERENT config must load the existing workspace unchanged —
        // setup is "ensure", never "recreate"; identity and config are preserved.
        let second = setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        assert_eq!(second.replica.replica_uuid, uuid, "identity preserved");
        assert_eq!(
            second.config.products["flow"]["plugin"].as_str(),
            Some("personal-todo"),
            "config is not clobbered on a re-run"
        );
    }

    #[test]
    fn setup_migrates_a_legacy_dir_at_root() {
        // setup ensures `.nxs/` at root; a legacy `.nexusflow/` WITH a replica there is migrated in
        // place (consistent with discover), so a flow-only init through setup never strands data.
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join(LEGACY_WORKSPACE_DIR);
        std::fs::create_dir(&legacy).unwrap();
        std::fs::write(
            legacy.join("replica.toml"),
            "site_id = 9\nprefix = \"wxyz\"\nreplica_uuid = \"uuid-legacy\"\n",
        )
        .unwrap();
        std::fs::write(legacy.join("config.toml"), "plugin = \"personal-todo\"\n").unwrap();

        let ws = setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        assert_eq!(ws.dir, tmp.path().join(WORKSPACE_DIR));
        assert!(
            !tmp.path().join(LEGACY_WORKSPACE_DIR).exists(),
            "legacy gone"
        );
        assert_eq!(ws.replica.replica_uuid, "uuid-legacy", "identity preserved");
        assert_eq!(
            ws.config.products["plugin"].as_str(),
            Some("personal-todo"),
            "config preserved (legacy plugin captured in the product seam)"
        );
    }

    #[test]
    fn activate_module_appends_idempotently_and_preserves_product_sections() {
        // aye.22: a second product's init registers itself in active_modules of an EXISTING
        // workspace (setup loads the config unchanged). Must append, be idempotent, and leave
        // flow's `[flow]` product section verbatim — the §7 seam is not clobbered.
        let tmp = TempDir::new().unwrap();
        let ws = init(
            tmp.path(),
            &cfg_with_section("flow", "plugin", "issue-tracker"),
        )
        .unwrap();

        assert!(activate_module(&ws.dir, "memory").unwrap(), "newly added");
        let reloaded = discover(tmp.path()).unwrap();
        assert_eq!(
            reloaded.config.active_modules,
            vec!["flow".to_string(), "memory".to_string()],
            "memory appended after flow"
        );
        assert_eq!(
            reloaded.config.products["flow"]["plugin"].as_str(),
            Some("issue-tracker"),
            "flow's product section is preserved"
        );
        // Idempotent: a second activate is a no-op.
        assert!(
            !activate_module(&ws.dir, "memory").unwrap(),
            "already active → no-op"
        );
        assert_eq!(
            discover(tmp.path()).unwrap().config.active_modules,
            vec!["flow".to_string(), "memory".to_string()],
            "no duplicate appended"
        );
    }

    #[test]
    fn activate_module_on_a_sectionless_config_seeds_active_modules() {
        // A fresh workspace with a default (empty) config still registers the module.
        let tmp = TempDir::new().unwrap();
        let ws = setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        assert!(activate_module(&ws.dir, "memory").unwrap());
        assert_eq!(
            discover(tmp.path()).unwrap().config.active_modules,
            vec!["memory".to_string()]
        );
    }

    // ---- the product-key writer (nxf 6j6v.npf9) ---------------------------

    #[test]
    fn set_product_key_creates_the_section_and_leaves_everything_else_verbatim() {
        let tmp = TempDir::new().unwrap();
        let ws = init(
            tmp.path(),
            &cfg_with_section("flow", "plugin", "issue-tracker"),
        )
        .unwrap();

        assert!(
            set_product_key(&ws.dir, "nxs", "service", "declined".into()).unwrap(),
            "a key that was not there is a change"
        );

        let reloaded = discover(tmp.path()).unwrap();
        assert_eq!(
            reloaded.config.products["nxs"]["service"].as_str(),
            Some("declined")
        );
        assert_eq!(
            reloaded.config.products["flow"]["plugin"].as_str(),
            Some("issue-tracker"),
            "another product's section is untouched"
        );
        assert_eq!(
            reloaded.config.active_modules,
            vec!["flow".to_string()],
            "the platform axis is untouched"
        );
    }

    #[test]
    fn set_product_key_writing_the_same_value_again_changes_nothing() {
        let tmp = TempDir::new().unwrap();
        let ws = setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        assert!(set_product_key(&ws.dir, "nxs", "service", "declined".into()).unwrap());
        let after_first = std::fs::read_to_string(ws.dir.join("config.toml")).unwrap();

        assert!(
            !set_product_key(&ws.dir, "nxs", "service", "declined".into()).unwrap(),
            "an unchanged value is reported as no change"
        );
        assert_eq!(
            std::fs::read_to_string(ws.dir.join("config.toml")).unwrap(),
            after_first,
            "and the file is not rewritten"
        );
    }

    #[test]
    fn set_product_key_replaces_a_value_that_is_already_there() {
        let tmp = TempDir::new().unwrap();
        let ws = setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        set_product_key(&ws.dir, "nxs", "service", "declined".into()).unwrap();

        assert!(set_product_key(&ws.dir, "nxs", "service", "installed".into()).unwrap());
        assert_eq!(
            discover(tmp.path()).unwrap().config.products["nxs"]["service"].as_str(),
            Some("installed")
        );
    }

    #[test]
    fn set_product_key_refuses_to_turn_the_legacy_plugin_scalar_into_a_table() {
        // The one reserved key in the product seam: a pre-rework workspace carries
        // `plugin = "personal-todo"` as a top-level SCALAR, and flow's reader falls back to it.
        // Writing a section over it would take that choice with it, so it is a loud error.
        let tmp = TempDir::new().unwrap();
        let ws = setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        std::fs::write(
            ws.dir.join("config.toml"),
            "active_modules = [\"flow\"]\nplugin = \"personal-todo\"\n",
        )
        .unwrap();

        let err = set_product_key(&ws.dir, "plugin", "service", "declined".into()).unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
        assert!(err.msg.contains("plugin"), "{}", err.msg);
        assert_eq!(
            discover(tmp.path()).unwrap().config.products["plugin"].as_str(),
            Some("personal-todo"),
            "and the scalar it refused to clobber is still there"
        );
    }

    // ---- what a config that cannot be READ must not become (review of PR #419, Integrity #2/#3)

    #[test]
    fn a_malformed_config_is_a_loud_error_from_both_writers_never_a_silent_reset() {
        // Both writers read-modify-write `config.toml`. A parse failure must propagate: starting
        // from a default and writing it back would replace a hand-mangled config — the one a user
        // is invited to edit — with an empty one, and report success.
        let tmp = TempDir::new().unwrap();
        let ws = setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        let mangled = "active_modules = [\"flow\"\n[nxs\n";
        std::fs::write(ws.dir.join("config.toml"), mangled).unwrap();

        for err in [
            set_product_key(&ws.dir, "nxs", "service", "declined".into()).unwrap_err(),
            activate_module(&ws.dir, "memory").unwrap_err(),
        ] {
            assert!(err.msg.contains("config.toml"), "{}", err.msg);
        }
        assert_eq!(
            std::fs::read_to_string(ws.dir.join("config.toml")).unwrap(),
            mangled,
            "and the bytes the user has to go and fix are still there"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_config_that_cannot_be_read_is_an_error_not_an_empty_one_to_write_back() {
        // THE DEFECT, at the one shape that reaches it: an unreadable — not absent — config used
        // to fall back to `WorkspaceConfig::default()`, and the very next line WROTE that back.
        // A permission or I/O error on this file would have taken `active_modules` and every
        // product section with it, silently and with an `Ok`.
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let ws = setup(
            tmp.path(),
            &cfg_with_section("flow", "plugin", "issue-tracker"),
        )
        .unwrap();
        let path = ws.dir.join("config.toml");
        let before = std::fs::read_to_string(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_to_string(&path).is_ok() {
            // Running as root (some containerised CI images do): the mode bit is not a barrier
            // there, so there is no unreadable file to prove anything about. Say so and stop
            // rather than assert something this environment cannot produce.
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            eprintln!("skipped: this user can read a 0o000 file, so the case cannot be staged");
            return;
        }

        let readable_again = |err: crate::error::NxfError| {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            err
        };
        let err =
            readable_again(set_product_key(&ws.dir, "nxs", "service", "x".into()).unwrap_err());
        assert!(err.msg.contains("config.toml"), "{}", err.msg);

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let err = readable_again(activate_module(&ws.dir, "memory").unwrap_err());
        assert!(err.msg.contains("config.toml"), "{}", err.msg);

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "nothing was written over a config nobody could read"
        );
    }

    #[test]
    fn an_absent_config_still_starts_from_an_empty_one() {
        // The fallback that stays: a workspace whose `config.toml` was never written is the
        // ordinary first-write case, and `NotFound` is the only error that means it.
        let tmp = TempDir::new().unwrap();
        let ws = setup(tmp.path(), &WorkspaceConfig::default()).unwrap();
        std::fs::remove_file(ws.dir.join("config.toml")).unwrap();
        assert!(activate_module(&ws.dir, "memory").unwrap());
        assert!(set_product_key(&ws.dir, "nxs", "service", "declined".into()).unwrap());
        let reloaded = discover(tmp.path()).unwrap();
        assert_eq!(reloaded.config.active_modules, vec!["memory".to_string()]);
        assert_eq!(
            reloaded.config.products["nxs"]["service"].as_str(),
            Some("declined")
        );
    }

    // ---- nexus-flow-ykv: prefix-relative id ergonomics --------------------

    #[test]
    fn display_id_strips_the_local_prefix_to_a_bare_suffix() {
        // A LOCAL id (our own prefix, at a `.` boundary) shows only its suffix — the prefix is
        // identical across every local item, so it is pure noise to the reader.
        assert_eq!(display_id("ab12", "ab12.0001"), "0001");
        assert_eq!(display_id("ab12", "ab12.00Z9"), "00Z9");
    }

    #[test]
    fn display_id_keeps_a_foreign_id_in_full() {
        // A FOREIGN id (any other replica's prefix) keeps its full `<prefix>.<suffix>` form: its
        // suffix is ambiguous against the local namespace, so the prefix is load-bearing.
        assert_eq!(display_id("ab12", "zz99.0001"), "zz99.0001");
    }

    #[test]
    fn display_id_requires_a_dot_boundary_not_a_mere_string_prefix() {
        // `ab120.0001` merely *starts with* the chars of `ab12` — but the prefix is `ab120`, not
        // `ab12`, so it is foreign and must stay full (the `.` boundary is what makes it local).
        assert_eq!(display_id("ab12", "ab120.0001"), "ab120.0001");
        // A degenerate id with no suffix at all is treated as foreign/opaque (returned verbatim).
        assert_eq!(display_id("ab12", "ab12"), "ab12");
    }

    #[test]
    fn resolve_id_prepends_the_local_prefix_to_a_bare_id() {
        // A BARE id (no `.`) resolves against the local prefix — `nxf show 0001` means `ab12.0001`.
        assert_eq!(resolve_id("ab12", "0001"), "ab12.0001");
    }

    #[test]
    fn resolve_id_leaves_a_qualified_id_unchanged() {
        // An already-qualified id (it has a `.`) is taken verbatim — local full or foreign full.
        assert_eq!(resolve_id("ab12", "ab12.0001"), "ab12.0001");
        assert_eq!(resolve_id("ab12", "zz99.0001"), "zz99.0001");
    }

    #[test]
    fn display_and_resolve_are_mutually_inverse() {
        // The two helpers are spiegelbildlich: resolving the display form of a LOCAL id round-trips
        // to the full id, and a FOREIGN id is a fixed point of both (display → full → full).
        let p = "ab12";
        let local = "ab12.0001";
        assert_eq!(resolve_id(p, display_id(p, local)), local);
        let foreign = "zz99.0007";
        assert_eq!(display_id(p, foreign), foreign);
        assert_eq!(resolve_id(p, foreign), foreign);
    }

    #[test]
    fn replica_methods_use_the_replicas_own_prefix() {
        // The `Replica` methods are the ergonomic surface the CLI calls; they apply the helpers
        // with the replica's own `prefix`. Under the deterministic identity that prefix is `ab12`.
        let r = deterministic_replica();
        assert_eq!(r.display_id("ab12.0042"), "0042");
        assert_eq!(r.display_id("zz99.0042"), "zz99.0042");
        assert_eq!(r.resolve_id("0042"), "ab12.0042");
        assert_eq!(r.resolve_id("zz99.0042"), "zz99.0042");
    }

    #[test]
    fn pre_bab_workspace_is_migrated_with_a_stable_replica_uuid() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(WORKSPACE_DIR);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("replica.toml"), "site_id = 7\nprefix = \"abcd\"\n").unwrap();

        let first = load(&dir).unwrap();
        assert!(
            !first.replica.replica_uuid.is_empty(),
            "migration mints a uuid"
        );
        let second = load(&dir).unwrap();
        assert_eq!(
            second.replica.replica_uuid, first.replica.replica_uuid,
            "migrated uuid is durable, not re-minted each load"
        );
        assert!(std::fs::read_to_string(dir.join("replica.toml"))
            .unwrap()
            .contains(&first.replica.replica_uuid));
    }
}
