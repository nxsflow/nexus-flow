//! `nxf self-update` (nexus-flow-85y.18): a narrow self-updater against our own `/updater`
//! contract (spec §7.2). Not the `self_update` crate — that is wired to GitHub-release
//! backends and would not carry our manifest contract (TB-3).

use crate::error::NxfError;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The production updater origin. Overridable via `NXF_BASE_URL` (point it at
/// `https://staging.nxsflow.com/nxs` to rehearse against staging — same env name install.sh uses).
const DEFAULT_BASE_URL: &str = "https://nxsflow.com/nxs";

/// The canonical, suite-wide upgrade verb every user-facing path advertises (nexus-flow-gel): the
/// umbrella `nxs self-update`. `nxf self-update` still works as a hidden, deprecated alias, but no
/// hint, status line, install script, or doc steers the user there anymore — they all point here.
const CANONICAL_SELF_UPDATE: &str = "nxs self-update";

/// Resolve the updater origin: `NXF_BASE_URL` if set and non-empty, else production.
fn updater_base_url() -> String {
    std::env::var("NXF_BASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

/// The per-installation config directory: `NXF_CONFIG_DIR` (test/override), else
/// `XDG_CONFIG_HOME/nxf`, else `~/.config/nxf`. This is where the channel + machine_id live.
fn user_config_dir() -> crate::error::Result<PathBuf> {
    if let Some(d) = std::env::var_os("NXF_CONFIG_DIR") {
        if !d.is_empty() {
            return Ok(PathBuf::from(d));
        }
    }
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return Ok(PathBuf::from(x).join("nxf"));
        }
    }
    if let Some(h) = std::env::var_os("HOME") {
        if !h.is_empty() {
            return Ok(PathBuf::from(h).join(".config").join("nxf"));
        }
    }
    Err(NxfError::io(
        "cannot locate a config directory (set HOME or NXF_CONFIG_DIR)",
    ))
}

/// Map a self-update failure onto the shared CLI error envelope. A trust failure (bad sha,
/// bad signature, missing binary, off-origin download) is `verification`; network and
/// filesystem failures are `io`.
fn to_nxf(e: UpdateError) -> NxfError {
    match e {
        UpdateError::Sha256Mismatch { .. }
        | UpdateError::Signature(_)
        | UpdateError::MissingBinary(_)
        | UpdateError::UntrustedOrigin { .. } => NxfError::verification(e.to_string()),
        UpdateError::Io(_) | UpdateError::Http(_) => NxfError::io(e.to_string()),
    }
}

/// The refusal shown when self-update runs on a build with no embedded signing key (eprg). Its
/// audience is whoever runs `nxs self-update` on a **locally-built** binary (a `cargo build` /
/// `cargo install` never bakes in `NXF_MINISIGN_PUBKEY`), so the wording names that cause and the
/// concrete fix — installing an official signed release — instead of the repo-internal spec the
/// old text pointed at. The maintainer key runbook (release-management.md §12.4) lives in
/// `signature.rs`'s module doc. Kept as a function so the wording is unit-tested without simulating
/// an available update. Fail-closed behaviour is unchanged — this is wording only.
fn no_key_selfupdate_message(brand: &str) -> String {
    format!(
        "this {brand} binary is unsigned — a locally-built or development build with no embedded \
         minisign public key — so it cannot verify the update's signature and refuses to \
         self-update. Install an official signed release instead: \
         curl -fsSL https://nxsflow.com/nxs/install.sh | sh"
    )
}

/// `<brand> self-update [--channel ..] [--check] [--json]` (spec §7.2). Queries `/updater`, and —
/// unless `--check` — downloads, verifies (sha256 + minisign, fail-closed), and atomically swaps
/// the suite for a strictly-newer build. Output goes to stdout (this is the command's result, not
/// the incidental update hint); `--json` is deterministic and brand-independent.
///
/// `brand` is the invocation name (`nxs` for the canonical `nxs self-update`, `nxf` for the hidden
/// deprecated alias) — it only flavors the **human** lines (`updated nxs …`). The mechanics are
/// suite-neutral regardless of `brand`: there is no "primary" binary — [`install_from`] always
/// swaps the single real `nxs` binary and relinks the `nxf`/`nxm`/`nxc` personas, so the whole suite
/// (including the binary currently running) moves together (nexus-flow-gel). The canonical
/// upgrade verb advertised to the user is always `nxs self-update` ([`CANONICAL_SELF_UPDATE`]).
pub fn run(
    brand: &str,
    json: bool,
    check: bool,
    channel: Option<&str>,
) -> crate::error::Result<()> {
    let cfg_dir = user_config_dir()?;
    let (channel, machine_id) = ensure_install_identity(&cfg_dir, channel).map_err(to_nxf)?;
    let current = env!("CARGO_PKG_VERSION");

    let base_url = updater_base_url();
    let agent = updater_agent();
    let response = fetch_update(
        &agent,
        &base_url,
        &channel,
        std::env::consts::OS,
        std::env::consts::ARCH,
        current,
        &machine_id,
    )
    .map_err(to_nxf)?;
    let status = decide_status(current, response);

    // `--check` never mutates anything: report the version decision and stop.
    if check {
        if json {
            println!("{}", status_json(current, &channel, &status));
        } else {
            println!("{}", status_human(brand, current, &channel, &status));
        }
        return Ok(());
    }

    // Already on the channel head: the `nxs` binary needs nothing — but the persona links may be
    // missing (a partial install, or a pre-multicall layout being healed). Self-heal LOCALLY by
    // (re)creating any absent/stale `nxf`/`nxm` symlink to `nxs`; no download, `nxs` is never
    // touched (no-downgrade). Without this, a layout missing a persona link could never gain it via
    // self-update — the updater would just keep saying "up to date" (nexus-flow-224, multicall).
    if matches!(status, UpdateStatus::UpToDate) {
        let dir = install_dir_of_running_exe()?;
        let repaired = ensure_persona_links(&dir).map_err(to_nxf)?;
        // Checked on THIS branch too (nxf 6j6v.dcpk (b)): a machine whose service has drifted onto
        // another build stays drifted for every run that finds itself already current, which on the
        // measured machine is most of them.
        let service = service_alias_divergence(&dir);
        if json {
            if repaired.is_empty() {
                println!(
                    "{}",
                    with_service_alias(status_json(current, &channel, &status), service.as_deref())
                );
            } else {
                println!(
                    "{}",
                    with_service_alias(
                        repaired_json(current, &channel, &repaired),
                        service.as_deref()
                    )
                );
            }
        } else if repaired.is_empty() {
            println!("{}", status_human(brand, current, &channel, &status));
        } else {
            println!(
                "{brand} {current} is up to date (channel {channel}); re-linked persona {}: {}",
                if repaired.len() == 1 { "link" } else { "links" },
                repaired.join(", ")
            );
        }
        if let Some(note) = &service {
            eprintln!("note: {note}");
        }
        // The already-on-the-head case still deserves the one-time hint: an install that arrived at
        // this version some other way (a fresh `install.sh`) would otherwise never hear it here.
        // Never under `--json` — that output is a byte contract.
        if !json {
            maybe_emit_migration_hint(&cfg_dir, current);
        }
        return Ok(());
    }

    // An update is available and the user asked to apply it. Verify against the baked-in key;
    // a build with no embedded key fails closed (cannot prove authenticity ⇒ refuse), exactly
    // like `verify` (signature.rs). The atomic swap is the only filesystem mutation and runs
    // last, after the in-memory bytes have passed both gates.
    let UpdateStatus::Available(resp) = &status else {
        unreachable!("UpToDate handled above")
    };
    let pubkey = crate::signature::embedded_public_key()
        .ok_or_else(|| NxfError::verification(no_key_selfupdate_message(brand)))?;
    // The install dir is the parent of the running binary. Under multicall (5jz.7) the running
    // exe may be the `nxf`/`nxm` SYMLINK or — when `current_exe()` resolves it (platform-dependent)
    // — the real `nxs`; either way the parent IS the install dir, and `install_from` swaps the
    // `<dir>/nxs` real binary there and re-points the persona links. The same-dir temp+rename swap
    // fails loudly if the directory is not writable rather than dangling anything.
    let dir = install_dir_of_running_exe()?;
    install_from(&agent, resp, pubkey, &dir, &base_url).map_err(to_nxf)?;

    // The binary moved; the service's alias did not (nxf 6j6v.dcpk (b)). Asked AFTER the swap, so
    // what it compares is the binary that is now on disk.
    let service = service_alias_divergence(&dir);
    if json {
        println!(
            "{}",
            with_service_alias(
                updated_json(current, &resp.version, &channel),
                service.as_deref()
            )
        );
    } else {
        println!(
            "updated {brand} {current} -> {} (channel {channel})",
            resp.version
        );
        // The threshold this update may have just crossed — one extra line beside the result line
        // that was printed anyway, and never again.
        maybe_emit_migration_hint(&cfg_dir, &resp.version);
    }
    if let Some(note) = &service {
        eprintln!("note: {note}");
    }
    Ok(())
}

/// **What the background service will run after this update** (nxf 6j6v.dcpk (b)).
///
/// `self-update` swaps `<dir>/nxs` and re-points the `nxf`/`nxm`/`nxc` persona links. It does NOT
/// touch `~/.nexusflow/bin/nexus-flow`, and it never did — which is why, measured on the owner's
/// machine on 2026-08-29, three updates in one day (0.69 → 0.70 → 0.71) left the machine's only
/// clock running a `target/debug` build from four days earlier, and nobody was told.
///
/// Silence was the defect, not the divergence. Whether a development build may be the system's
/// clock is a product question and has a ticket of its own (nxf 6j6v.7gz6); moving the service
/// under the user's feet would be answering it here, from inside a command they ran for a different
/// reason. So this NAMES the divergence, with both paths in full, and says which command settles
/// it — the "oder die Abweichung benannt" half of the item's own two options.
///
/// Pure over its three inputs so every arm is provable without an install, a service or a `$HOME`.
fn service_alias_note(
    state: &nxs_service::ProgramState,
    link: &Path,
    installed: &Path,
) -> Option<String> {
    match state {
        // No alias: this machine has never installed the background service, so there is nothing
        // for an update to have left behind.
        nxs_service::ProgramState::Absent => None,
        nxs_service::ProgramState::Dangling(target) => Some(format!(
            "the nexus-flow background service cannot start: {} points at {}, which is not there. \
             The update itself succeeded — this is the service beside it. Run \
             `nxs sync daemon install` to point it at {}.",
            link.display(),
            target.display(),
            installed.display()
        )),
        nxs_service::ProgramState::Present(target) if target != installed => Some(format!(
            "the nexus-flow background service still runs {}, not the binary this update wrote \
             ({}). `self-update` deliberately does not re-point {} — which build keeps this \
             machine's time is a choice, not a side effect. Run `nxs sync daemon install` to move \
             the service onto the updated binary, or leave it where it is.",
            target.display(),
            installed.display(),
            link.display()
        )),
        nxs_service::ProgramState::Present(_) => None,
    }
}

/// [`service_alias_note`] against the real `~/.nexusflow` — `None` on any resolution failure, since
/// a note about the service is a courtesy on top of an update that has already happened and must
/// never be able to fail one.
///
/// Both sides are canonicalised before they are compared, and only then: `<dir>/nxs` is reached
/// through whatever path `current_exe()` gave, the alias through `read_link`, and two spellings of
/// one file would otherwise read as a divergence on every single update.
fn service_alias_divergence(install_dir: &Path) -> Option<String> {
    fn resolved(p: &Path) -> PathBuf {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    }
    let home = nxs_service::ServiceHome::resolve().ok()?;
    let state = match home.program_state() {
        nxs_service::ProgramState::Present(t) => nxs_service::ProgramState::Present(resolved(&t)),
        other => other,
    };
    service_alias_note(
        &state,
        &home.program(),
        &resolved(&install_dir.join(REAL_BINARY)),
    )
}

/// Report a service-alias divergence on both channels the caller might be reading: the `--json`
/// receipt (an added `service_alias` key, absent when there is nothing to say) and stderr for a
/// human. A message only a terminal reader meets is the breadcrumb class this repo keeps closing.
fn with_service_alias(mut body: serde_json::Value, note: Option<&str>) -> serde_json::Value {
    if let (Some(note), Some(map)) = (note, body.as_object_mut()) {
        map.insert(
            "service_alias".into(),
            serde_json::Value::String(note.into()),
        );
    }
    body
}

/// The install directory: the parent of the running executable. Under multicall the running exe
/// may be a persona symlink (`nxf`/`nxm`) or the resolved real `nxs`, but the parent dir — where
/// `nxs` and the persona links live — is the same either way.
fn install_dir_of_running_exe() -> crate::error::Result<PathBuf> {
    let exe = std::env::current_exe()
        .map_err(|e| NxfError::io(format!("locating the running nxf executable: {e}")))?;
    exe.parent().map(Path::to_path_buf).ok_or_else(|| {
        NxfError::io(format!(
            "nxf path has no parent directory: {}",
            exe.display()
        ))
    })
}

// ===========================================================================================
// Update hint (nexus-flow-85y.21): a dezent, once-a-day "an update is available" line.
// NO background auto-update — an agent CLI must be deterministic, and a binary that swaps
// itself between two invocations is not (TB-4). The hint is the rustup/gh pattern: an explicit
// `self-update` command + a quiet nudge. It goes ONLY to stderr (never stdout/--json), at most
// once a day (cache file), and is silenced for any non-interactive use.
// ===========================================================================================

/// The throttle/state file for the update hint (`~/.cache/nxf/update-check.toml`). `last_check`
/// (a `YYYY-MM-DD` date) bounds the network probe to once a day; `latest_version` is the last
/// version the updater offered, so same-day runs render the hint with no network at all.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
struct UpdateCache {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_check: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest_version: Option<String>,
}

/// Is this an environment where a hint must stay silent? `--json` (machine output), an explicit
/// `NXF_NO_UPDATE_CHECK`, CI, or a non-interactive stderr (piped/captured — every agent flow and
/// test harness) all suppress it. The non-TTY rule is what keeps the hint — and its network
/// probe — entirely out of programmatic use, no env var required.
fn hint_disabled(json: bool, no_update_check: bool, is_ci: bool, stderr_is_tty: bool) -> bool {
    json || no_update_check || is_ci || !stderr_is_tty
}

/// Interpret an `NXF_NO_UPDATE_CHECK` value. Present and not an explicit off (`0`/`false`/`""`)
/// ⇒ disabled. Mirrors the usual "set to anything truthy to turn it on" convention, inverted.
fn env_flag_truthy(val: Option<&str>) -> bool {
    matches!(val.map(|v| v.trim()), Some(v) if !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
}

/// Detect CI from a name→value lookup. `CI` truthy, or any well-known CI marker present.
fn is_ci_env(get: impl Fn(&str) -> Option<String>) -> bool {
    if env_flag_truthy(get("CI").as_deref()) {
        return true;
    }
    [
        "GITHUB_ACTIONS",
        "GITLAB_CI",
        "BUILDKITE",
        "CIRCLECI",
        "JENKINS_URL",
        "TF_BUILD",
    ]
    .iter()
    .any(|k| get(k).is_some_and(|v| !v.trim().is_empty()))
}

/// Should we make the (throttled) network probe? Only when the cache wasn't already refreshed
/// today — so the updater is hit at most once per calendar day per install.
fn needs_network_refresh(last_check: Option<&str>, today: &str) -> bool {
    last_check != Some(today)
}

/// The hint line for stderr, or `None` when the known latest is not strictly newer than the
/// running version (nothing to nudge about).
fn hint_line(current: &str, latest: Option<&str>) -> Option<String> {
    match latest {
        Some(v) if is_strictly_newer(current, v) => Some(format!(
            "note: nxf {v} is available (you have {current}). Run `{CANONICAL_SELF_UPDATE}` to \
             upgrade, or set NXF_NO_UPDATE_CHECK=1 to silence this."
        )),
        _ => None,
    }
}

/// The hint cache directory: `NXF_CACHE_DIR` (override), else `XDG_CACHE_HOME/nxf`, else
/// `~/.cache/nxf`. Separate from the config dir so a `--no-cache`-style wipe never loses the
/// persisted channel.
fn cache_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("NXF_CACHE_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    if let Some(x) = std::env::var_os("XDG_CACHE_HOME").filter(|x| !x.is_empty()) {
        return Some(PathBuf::from(x).join("nxf"));
    }
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".cache").join("nxf"))
}

/// Today's date as `YYYY-MM-DD` (UTC) — the throttle granularity for the once-a-day probe.
fn today_utc() -> String {
    let d = time::OffsetDateTime::now_utc().date();
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

/// Emit the once-a-day "an update is available" hint to **stderr**, if appropriate. Called on
/// the success path of every non-`self-update` command. Entirely silent — and free of any
/// filesystem/network work — for `--json`, `NXF_NO_UPDATE_CHECK`, CI, or non-interactive stderr
/// (the cheap gate runs first). Otherwise it consults a once-a-day cache, doing at most one
/// best-effort `/updater` probe per day; every failure is swallowed (a nudge must never break
/// or slow a real command). NEVER touches stdout, so `--json` consumers are unaffected even
/// when it does run.
pub fn maybe_emit_update_hint(json: bool) {
    use std::io::IsTerminal;
    let no_check = env_flag_truthy(std::env::var("NXF_NO_UPDATE_CHECK").ok().as_deref());
    let is_ci = is_ci_env(|k| std::env::var(k).ok());
    // Cheap gate first: for any non-interactive use this returns before any fs/network work.
    if hint_disabled(json, no_check, is_ci, std::io::stderr().is_terminal()) {
        return;
    }
    let Some(dir) = cache_dir() else { return };
    let (channel, machine_id) = read_install_identity_for_hint();
    if let Some(line) = compute_update_hint(
        &dir,
        &updater_base_url(),
        env!("CARGO_PKG_VERSION"),
        &channel,
        &machine_id,
        &today_utc(),
    ) {
        eprintln!("{line}");
    }
}

/// The hint core, fully injectable (cache dir, updater origin, versions, day) so the
/// probe → cache → render path is testable without env vars or a real TTY. Performs at most one
/// `/updater` probe per `today`, persists the result, and returns the stderr line to print (or
/// `None`). Every probe/persist failure is swallowed — a nudge must never break a real command.
fn compute_update_hint(
    cache_dir: &Path,
    base_url: &str,
    current: &str,
    channel: &str,
    machine_id: &str,
    today: &str,
) -> Option<String> {
    let cache_path = cache_dir.join("update-check.toml");
    let mut cache: UpdateCache = std::fs::read_to_string(&cache_path)
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default();

    if needs_network_refresh(cache.last_check.as_deref(), today) {
        // On 204 the latest IS the running version (nothing to nudge); on 200 it is the offer.
        // Record the date regardless so a transient failure can't re-probe on every command today.
        if let Ok(resp) = fetch_update(
            &updater_agent(),
            base_url,
            channel,
            std::env::consts::OS,
            std::env::consts::ARCH,
            current,
            machine_id,
        ) {
            cache.latest_version = Some(match resp {
                Some(r) => r.version,
                None => current.to_string(),
            });
        }
        cache.last_check = Some(today.to_string());
        let _ = save_update_cache(cache_dir, &cache_path, &cache);
    }

    hint_line(current, cache.latest_version.as_deref())
}

/// Read `(channel, machine_id)` for the hint probe WITHOUT minting or persisting anything — the
/// hint must not have side effects on the user config. Missing values fall back to the default
/// channel and an empty machine_id (the updater still answers; only rollout stickiness is lost).
fn read_install_identity_for_hint() -> (String, String) {
    let cfg = user_config_dir()
        .ok()
        .map(|d| load_user_config(&d))
        .unwrap_or_default();
    let channel = cfg
        .channel
        .filter(|c| is_valid_channel(c))
        .unwrap_or_else(|| DEFAULT_CHANNEL.to_string());
    (channel, cfg.machine_id.unwrap_or_default())
}

/// Persist the hint cache (best-effort; the caller ignores failures).
fn save_update_cache(dir: &Path, path: &Path, cache: &UpdateCache) -> Result<(), UpdateError> {
    std::fs::create_dir_all(dir)
        .map_err(|e| UpdateError::Io(format!("creating cache dir: {e}")))?;
    let toml =
        toml::to_string(cache).map_err(|e| UpdateError::Io(format!("serializing cache: {e}")))?;
    std::fs::write(path, toml).map_err(|e| UpdateError::Io(format!("writing cache: {e}")))
}

/// Why a self-update could not be applied. Every variant is a refusal that leaves the
/// installed binary untouched — verification happens entirely on the downloaded bytes in a
/// scratch dir, and the atomic swap is the last step, so a failed update is always a no-op.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// The downloaded bytes do not match the integrity digest from `/updater` (mandatory gate).
    #[error("sha256 mismatch: expected {expected}, got {actual} (refusing to install)")]
    Sha256Mismatch { expected: String, actual: String },
    /// minisign verification against the baked-in pubkey failed (authenticity gate, fail-closed).
    #[error("signature verification failed: {0} (refusing to install)")]
    Signature(String),
    /// The verified tarball does not contain an expected suite member to install.
    #[error("the release tarball does not contain a `{0}` binary")]
    MissingBinary(String),
    /// The `/updater` response pointed the download off the origin we queried (origin-pin).
    #[error("refusing to download from an untrusted origin: {url} (expected origin {expected})")]
    UntrustedOrigin { url: String, expected: String },
    /// Filesystem failure while unpacking or swapping the binary.
    #[error("{0}")]
    Io(String),
    /// Network / HTTP failure talking to `/updater` or fetching the tarball.
    #[error("{0}")]
    Http(String),
}

/// The per-installation user config (`~/.config/nxf/config.toml`). Distinct from the
/// per-workspace `.nxs/config.toml`: the update channel and the rollout `machine_id`
/// are properties of THIS install of `nxf`, not of any project it operates on (TB-14).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct UserConfig {
    /// The promotion ring to self-update from. `None` ⇒ the default ("stable").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// A stable, per-install random id the updater hashes to a rollout bucket so a client
    /// never flaps between "update available / not" (spec §6.1). Minted once, then durable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<String>,
    /// Whether this install has already been told that the judging migration exists (nxf
    /// 6j6v.9yaj). Persisted, because the hint's whole contract is **exactly once**: a line that
    /// reappears on every update is a line people learn to skip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_hint_shown: Option<bool>,
}

// ===========================================================================================
// The migration hint (nxf 6j6v.9yaj) — one line, once.
//
// `self-update` swaps a BINARY. The workspaces are many, they live elsewhere, and this command
// also runs unattended, so something that touches N databases and asks questions does not belong
// here — that carrier is `nxs prime`, which runs per workspace and is the only place that knows
// whether THIS one still has unfiled memories. What self-update can do, and should, is say the
// capability now exists: it prints a result line anyway, and a second one costs nothing.
// ===========================================================================================

/// The first version that carries `nxm migrate` — the threshold whose crossing the hint announces.
const MIGRATION_MIN_VERSION: &str = "0.48.0";

/// The hint itself. Names the command and the one thing it does, and nothing more: what is worth
/// doing in a given workspace is `prime`'s to say.
const MIGRATION_HINT: &str = "new: `nxm migrate plan --with-judge` files a workspace's memories \
                              and moves its CLAUDE.md into them (once per workspace).";

/// Whether the hint is due: the version now installed carries the migration, and this install has
/// never been told. Pure over its inputs, so both halves of "exactly once" are unit-testable
/// without a filesystem or a network.
fn migration_hint_due(installed: &str, already_shown: bool) -> bool {
    !already_shown
        && (installed == MIGRATION_MIN_VERSION
            || is_strictly_newer(MIGRATION_MIN_VERSION, installed))
}

/// Print the migration hint if it is due, and record that it was. Best-effort on the write: a
/// config that cannot be saved costs a repeated hint, never a failed update — so the caller's
/// result is never turned into an error by a note.
fn maybe_emit_migration_hint(dir: &Path, installed: &str) {
    let mut cfg = load_user_config(dir);
    if !migration_hint_due(installed, cfg.migration_hint_shown.unwrap_or(false)) {
        return;
    }
    println!("{MIGRATION_HINT}");
    cfg.migration_hint_shown = Some(true);
    let _ = save_user_config(dir, &cfg);
}

/// The three promotion rings. The CLI restricts `--channel` to these; a persisted value
/// outside the set is ignored (falls back to the default) rather than trusted.
const CHANNELS: [&str; 3] = ["stable", "beta", "alpha"];
const DEFAULT_CHANNEL: &str = "stable";

fn is_valid_channel(s: &str) -> bool {
    CHANNELS.contains(&s)
}

fn user_config_path(dir: &Path) -> std::path::PathBuf {
    dir.join("config.toml")
}

/// Load the user config from `dir`, treating a missing or unparseable file as the default
/// (best-effort: a corrupt config must never wedge `nxf`, only lose the persisted channel).
fn load_user_config(dir: &Path) -> UserConfig {
    std::fs::read_to_string(user_config_path(dir))
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Persist the user config to `dir`, creating the directory if needed.
fn save_user_config(dir: &Path, cfg: &UserConfig) -> Result<(), UpdateError> {
    std::fs::create_dir_all(dir)
        .map_err(|e| UpdateError::Io(format!("creating config dir {}: {e}", dir.display())))?;
    let toml = toml::to_string(cfg)
        .map_err(|e| UpdateError::Io(format!("serializing user config: {e}")))?;
    std::fs::write(user_config_path(dir), toml)
        .map_err(|e| UpdateError::Io(format!("writing user config: {e}")))
}

/// Resolve `(channel, machine_id)` for this install, persisting any change. An explicit
/// `--channel` is recorded (so it sticks for the update hint and future runs); otherwise the
/// persisted channel is used, falling back to "stable". The `machine_id` is minted once and
/// then stable forever — re-minting it each run would defeat the updater's sticky rollout.
fn ensure_install_identity(
    dir: &Path,
    cli_channel: Option<&str>,
) -> Result<(String, String), UpdateError> {
    let mut cfg = load_user_config(dir);
    let mut dirty = false;

    // Channel: an explicit --channel wins and is recorded; otherwise the persisted value if
    // it is one we recognize, else the default. A persisted-but-invalid channel is ignored.
    let channel = match cli_channel {
        Some(c) => {
            if cfg.channel.as_deref() != Some(c) {
                cfg.channel = Some(c.to_string());
                dirty = true;
            }
            c.to_string()
        }
        None => cfg
            .channel
            .as_deref()
            .filter(|c| is_valid_channel(c))
            .unwrap_or(DEFAULT_CHANNEL)
            .to_string(),
    };

    // machine_id: mint once, then durable. A full lowercased ULID — the same entropy source
    // the workspace replica identity uses (workspace.rs), independent per install.
    let machine_id = match &cfg.machine_id {
        Some(id) if !id.is_empty() => id.clone(),
        _ => {
            let id = ulid::Ulid::new().to_string().to_ascii_lowercase();
            cfg.machine_id = Some(id.clone());
            dirty = true;
            id
        }
    };

    if dirty {
        save_user_config(dir, &cfg)?;
    }
    Ok((channel, machine_id))
}

/// The deterministic `--json` body for a check / pre-install decision. Stable keys are the
/// agent contract: `status` is one of `up-to-date` | `update-available`.
fn status_json(current: &str, channel: &str, status: &UpdateStatus) -> serde_json::Value {
    match status {
        UpdateStatus::UpToDate => serde_json::json!({
            "status": "up-to-date",
            "current_version": current,
            "channel": channel,
        }),
        UpdateStatus::Available(r) => serde_json::json!({
            "status": "update-available",
            "current_version": current,
            "latest_version": r.version,
            "channel": channel,
            "pub_date": r.pub_date,
            "notes": r.notes,
            "url": r.url,
        }),
    }
}

/// The human (stderr/stdout) line for a check / pre-install decision. `brand` flavors the subject
/// (the binary the user invoked); the suggested action is always the canonical `nxs self-update`
/// ([`CANONICAL_SELF_UPDATE`]) — even via the deprecated `nxf self-update` alias (nexus-flow-gel).
fn status_human(brand: &str, current: &str, channel: &str, status: &UpdateStatus) -> String {
    match status {
        UpdateStatus::UpToDate => {
            format!("{brand} {current} is up to date (channel {channel})")
        }
        UpdateStatus::Available(r) => format!(
            "{brand} {} is available (you have {current}, channel {channel}). Run `{CANONICAL_SELF_UPDATE}` to upgrade.",
            r.version
        ),
    }
}

/// The deterministic `--json` body after a successful install.
fn updated_json(previous: &str, new: &str, channel: &str) -> serde_json::Value {
    serde_json::json!({
        "status": "updated",
        "previous_version": previous,
        "new_version": new,
        "channel": channel,
    })
}

/// The deterministic `--json` body when `nxf` was already current but missing suite binaries were
/// healed in place (nexus-flow-224). `repaired` lists the delivered names (e.g. `["nxs"]`).
fn repaired_json(current: &str, channel: &str, repaired: &[String]) -> serde_json::Value {
    serde_json::json!({
        "status": "repaired",
        "current_version": current,
        "channel": channel,
        "repaired": repaired,
    })
}

/// Hard cap on a downloaded tarball (the shipped binaries are a few MB; this only bounds a
/// hostile or runaway response, it never limits a real release).
const MAX_TARBALL_BYTES: u64 = 100 * 1024 * 1024;

/// Hard cap on the `/updater` JSON body. The real response is well under a kilobyte; `ureq`'s
/// `into_json` applies no size limit (only `into_string` does), so a hostile updater could
/// stream an unbounded body into the parser. We read the body bounded, then parse from the slice.
const MAX_UPDATER_JSON_BYTES: u64 = 256 * 1024;

/// A `ureq` agent with finite timeouts so a hung updater can never wedge `nxf`.
fn updater_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .build()
}

/// Ask `/updater` whether an update is available. `Ok(None)` == 204 (up to date / no build /
/// rollout-excluded); `Ok(Some(resp))` == 200 with the offer. A non-2xx/transport failure is
/// an `Http` error — never silently read as "up to date".
fn fetch_update(
    agent: &ureq::Agent,
    base_url: &str,
    channel: &str,
    target: &str,
    arch: &str,
    current_version: &str,
    machine_id: &str,
) -> Result<Option<UpdaterResponse>, UpdateError> {
    let url = format!("{}/updater", base_url.trim_end_matches('/'));
    let resp = agent
        .get(&url)
        .query("channel", channel)
        .query("target", target)
        .query("arch", arch)
        .query("current_version", current_version)
        .query("machine_id", machine_id)
        .call();
    match resp {
        Ok(r) if r.status() == 204 => Ok(None),
        Ok(r) => parse_updater_body(r).map(Some),
        Err(ureq::Error::Status(code, _)) => {
            Err(UpdateError::Http(format!("/updater returned HTTP {code}")))
        }
        Err(ureq::Error::Transport(t)) => {
            Err(UpdateError::Http(format!("/updater request failed: {t}")))
        }
    }
}

/// Read a `/updater` 200 body bounded by [`MAX_UPDATER_JSON_BYTES`] and parse it. Bounding the
/// read (rather than `into_json`, which has no cap) stops a hostile updater from OOM-ing the
/// client with an unbounded body — symmetric with the [`download`] cap.
fn parse_updater_body(resp: ureq::Response) -> Result<UpdaterResponse, UpdateError> {
    use std::io::Read;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(MAX_UPDATER_JSON_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| UpdateError::Http(format!("reading /updater response: {e}")))?;
    if buf.len() as u64 > MAX_UPDATER_JSON_BYTES {
        return Err(UpdateError::Http(format!(
            "/updater response exceeds {MAX_UPDATER_JSON_BYTES} bytes (refusing)"
        )));
    }
    serde_json::from_slice(&buf)
        .map_err(|e| UpdateError::Http(format!("malformed /updater response: {e}")))
}

/// Download `url` into memory, bounded by [`MAX_TARBALL_BYTES`].
fn download(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, UpdateError> {
    use std::io::Read;
    let resp = agent.get(url).call().map_err(|e| match e {
        ureq::Error::Status(code, _) => {
            UpdateError::Http(format!("downloading {url}: HTTP {code}"))
        }
        ureq::Error::Transport(t) => UpdateError::Http(format!("downloading {url}: {t}")),
    })?;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(MAX_TARBALL_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| UpdateError::Http(format!("reading download body: {e}")))?;
    if buf.len() as u64 > MAX_TARBALL_BYTES {
        return Err(UpdateError::Http(format!(
            "download exceeds {MAX_TARBALL_BYTES} bytes (refusing)"
        )));
    }
    Ok(buf)
}

/// True iff `url` shares its `scheme://authority` with `base`. The `/updater` response carries a
/// server-controlled `url`; pinning the download to the origin we queried mirrors install.sh's
/// `same_origin` guard. The signature gate already makes a foreign artifact un-installable, but
/// pinning the origin also closes the SSRF / forced-download / `machine_id`-exfil vectors and
/// keeps the two install surfaces consistent (defense in depth).
fn same_origin(url: &str, base: &str) -> bool {
    fn origin(s: &str) -> Option<&str> {
        let scheme_end = s.find("://")?;
        let after = scheme_end + 3;
        let authority_len = s[after..].find('/').unwrap_or(s.len() - after);
        Some(&s[..after + authority_len])
    }
    match (origin(url), origin(base)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Download the offered tarball, verify it (sha256 + minisign under `pubkey`), unpack the
/// `nxf` member, and atomically swap it over `target_exe`. The download is pinned to the origin
/// of `base_url` (the host we queried) before any bytes are fetched; verification then runs on
/// the in-memory bytes before any filesystem mutation, so a failed/forged update leaves
/// `target_exe` intact.
fn install_from(
    agent: &ureq::Agent,
    resp: &UpdaterResponse,
    pubkey: &str,
    install_dir: &Path,
    base_url: &str,
) -> Result<(), UpdateError> {
    if !same_origin(&resp.url, base_url) {
        return Err(UpdateError::UntrustedOrigin {
            url: resp.url.clone(),
            expected: base_url.to_string(),
        });
    }
    let bytes = download(agent, &resp.url)?;
    verify_artifact_with_key(&bytes, &resp.sha256, &resp.signature, pubkey)?;
    // Multicall (5jz.7): the tarball ships ONE real binary, `nxs`. Extract it FIRST (so a
    // missing/oversized member aborts before any write), atomically swap `<dir>/nxs`, then ensure
    // the `nxf`/`nxm` persona symlinks point at it — the links are local (no download) and idempotent,
    // and re-pointing them also migrates a pre-multicall install (real `nxf`/`nxm` files) to the
    // single-binary layout on its first update.
    let nxs = extract_member(&bytes, REAL_BINARY)?;
    // BOTH members come out of the archive before the first write, for the reason stated above:
    // an oversized or unreadable sidecar member is an archive problem, and it must abort while
    // aborting is still free. Doing it after the swap would report a FAILED update that has in fact
    // completed — and since the new `nxs` is then already on disk, the next `self-update` answers
    // "up to date" and the sidecar is never installed at all (6j6v.81v5 review, Integrity #5).
    let sidecar = extract_optional_member(&bytes, SIDECAR_FILE)?;
    atomic_replace(&install_dir.join(REAL_BINARY), &nxs)?;
    ensure_persona_links(install_dir)?;
    match sidecar {
        // A release from 0.52.0/0.53.0, reachable through `--version`: those binaries have no
        // embedded sidecar and can only find one beside themselves, so it still gets placed.
        //
        // Past the swap, the update HAS happened. A failure to place the sidecar here is worth
        // saying out loud, but it is not worth reporting the whole update as failed — that is the
        // state with no way forward. The binary that landed reports the missing sidecar itself,
        // naming both ways out, the moment someone tries to start an agent.
        Some(sidecar) => {
            if let Err(e) =
                atomic_replace_with_mode(&install_dir.join(SIDECAR_FILE), &sidecar, 0o644)
            {
                eprintln!(
                    "warning: the update completed, but the agent sidecar could not be written to \
                     {}: {e}\n         `nxc send --to <persona>` will refuse until it is there — \
                     re-run `nxs self-update` once the cause is fixed.",
                    install_dir.join(SIDECAR_FILE).display()
                );
            }
        }
        // The normal path since 6j6v.smsz: the sidecar is compiled into the `nxs` that just landed,
        // so there is nothing to place — and a copy left beside the binary by <=0.53.0 now belongs
        // to a version that is no longer installed. Removing it reclaims 1.4 MB and closes the
        // binary/sidecar drift 6j6v.5vct described; it cannot change which sidecar runs, because
        // the embedded one is preferred either way (`nexus_chat::worker::installed_sidecar`).
        // Best-effort by design: the update succeeded, and a leftover file is not worth failing it.
        None => {
            let stale = install_dir.join(SIDECAR_FILE);
            if stale.exists() {
                match std::fs::remove_file(&stale) {
                    Ok(()) => println!(
                        "removed stale {} (this nxs carries its own)",
                        stale.display()
                    ),
                    Err(e) => eprintln!(
                        "warning: the update completed, but the superseded agent sidecar at {} \
                         could not be removed: {e}\n         It is inert — the sidecar is built \
                         into nxs now — so this is housekeeping, not a failure.",
                        stale.display()
                    ),
                }
            }
        }
    }
    Ok(())
}

/// Extract `name` if the archive carries it, `Ok(None)` if it simply does not.
///
/// The bundled `nxc` agent sidecar (nexus-flow-6j6v.81v5) is optional in exactly one direction:
/// every release cut before it shipped has no such member, and `NXF_VERSION`-pinning to one of
/// those is legal, so its ABSENCE must not fail an update. Every other failure — an oversized
/// member, an unreadable archive — still propagates; this is the narrow tolerance, not a catch-all.
fn extract_optional_member(tar_gz: &[u8], name: &str) -> Result<Option<Vec<u8>>, UpdateError> {
    match extract_member(tar_gz, name) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(UpdateError::MissingBinary(_)) => Ok(None),
        Err(other) => Err(other),
    }
}

/// Lowercase hex sha256 of `bytes` — the integrity half of the gate (spec §5.3).
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        // write! into the buffer — no throwaway String per byte. Infallible for a String sink.
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Verify a downloaded tarball under an explicit pubkey: sha256 (mandatory integrity) AND
/// minisign (mandatory authenticity, fail-closed — never a fallback to sha256-only, l83).
/// Pure: the key is a parameter so the trust decision is testable with an ephemeral keypair.
fn verify_artifact_with_key(
    bytes: &[u8],
    expected_sha256: &str,
    signature_minisig: &str,
    pubkey: &str,
) -> Result<(), UpdateError> {
    // Integrity first: a cheap byte check that aborts before we trust anything further.
    let actual = sha256_hex(bytes);
    if !actual.eq_ignore_ascii_case(expected_sha256.trim()) {
        return Err(UpdateError::Sha256Mismatch {
            expected: expected_sha256.trim().to_lowercase(),
            actual,
        });
    }
    // Authenticity: minisign against the trusted key. ALWAYS — there is no sha256-only path.
    crate::signature::verify_with_key(pubkey, bytes, signature_minisig)
        .map_err(|e| UpdateError::Signature(e.to_string()))
}

/// Upper bound on the UNCOMPRESSED `nxf` member (the shipped binaries are a few MB; this only
/// bounds a hostile archive). The download is capped at [`MAX_TARBALL_BYTES`], but gzip can
/// expand ~1000:1, so a 100 MB tarball could inflate to ~100 GB into a `Vec` without a cap on the
/// decompressed stream — this is the decompression-bomb guard (requires a pipeline/key compromise
/// to even reach, but it is cheap defense in depth).
const MAX_UNCOMPRESSED_NXS_BYTES: u64 = 512 * 1024 * 1024;

/// The single real CLI binary the tarball ships (nexus-flow-5jz.7 multicall): `nxs` contains the
/// whole suite and serves the `nxf`/`nxm`/`nxc` surfaces by `argv[0]`. `self-update` swaps THIS file.
const REAL_BINARY: &str = "nxs";

/// The persona names that are SYMLINKS to [`REAL_BINARY`] in an install dir (multicall): typing
/// `nxf`/`nxm`/`nxc` runs `nxs`, which routes by `argv[0]`. They carry no bytes of their own — an
/// install/update creates them as links, never as copies (the relay `nxf-relay` stays a separate
/// real binary, opt-in, so it is intentionally NOT here). MUST stay in sync with the dev-build list
/// in `crates/nxs/build.rs` and the `install.sh` `link_persona` calls — a persona missing from any
/// one of the three breaks that install path (6j6v.kew6: `nxc` was missing here + in `install.sh`,
/// so `nxs init` for chat could not spawn `nxc agent-manifest`).
const PERSONA_LINKS: &[&str] = &["nxf", "nxm", "nxc"];

/// The `nxc` agent sidecar — one self-contained ESM bundle, no `node_modules`.
///
/// **No current release carries it as a tarball member** (nexus-flow-6j6v.smsz): it is compiled
/// into `nxs` itself, and this name survives here for the two things an updater still does with it
/// — placing it when `--version` pins 0.52.0/0.53.0 (whose binaries can only find one beside
/// themselves), and REMOVING a copy those versions left behind.
///
/// Named here rather than imported from `nexus_chat::worker` because this crate does not depend on
/// chat — several places have to agree on this string, and
/// `every_place_that_ships_the_sidecar_spells_it_the_same` in `crates/chat/tests/worker.rs` is the
/// gate that keeps them agreeing.
const SIDECAR_FILE: &str = "nxc-agent-sidecar.mjs";

/// Ensure every [`PERSONA_LINKS`] entry in `dir` is a symlink to [`REAL_BINARY`], (re)creating any
/// that is missing, broken, or a stale REAL binary left by a pre-multicall install. Returns the
/// names it had to (re)create — empty when the layout was already correct, so a healthy install
/// self-heals to a no-op. Local only: a persona is a link, never a download.
fn ensure_persona_links(dir: &Path) -> Result<Vec<String>, UpdateError> {
    let mut fixed = Vec::new();
    for &persona in PERSONA_LINKS {
        let link = dir.join(persona);
        if persona_link_ok(&link) {
            continue;
        }
        replace_with_symlink(&link, REAL_BINARY)?;
        fixed.push(persona.to_string());
    }
    Ok(fixed)
}

/// Is `link` already a symlink whose target is exactly [`REAL_BINARY`] (the relative `nxs`)?
fn persona_link_ok(link: &Path) -> bool {
    std::fs::symlink_metadata(link)
        .ok()
        .filter(|m| m.file_type().is_symlink())
        .and_then(|_| std::fs::read_link(link).ok())
        .is_some_and(|t| t.as_os_str() == REAL_BINARY)
}

/// Point `link` at `target` as a relative symlink, replacing whatever is there first (a stale real
/// `nxf`/`nxm` binary from a pre-multicall install, or a broken/old link). Unix only swaps in place;
/// on a non-unix host symlinks need privilege, so this is best-effort and surfaces the error.
fn replace_with_symlink(link: &Path, target: &str) -> Result<(), UpdateError> {
    if link.exists() || link.symlink_metadata().is_ok() {
        std::fs::remove_file(link).map_err(|e| {
            UpdateError::Io(format!("removing {} before relinking: {e}", link.display()))
        })?;
    }
    symlink_file(target, link)
        .map_err(|e| UpdateError::Io(format!("linking {} -> {target}: {e}", link.display())))
}

#[cfg(unix)]
fn symlink_file(target: &str, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_file(target: &str, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

/// Extract one named member's bytes from a gzip-compressed tar, bounding the decompressed size.
fn extract_member(tar_gz: &[u8], name: &str) -> Result<Vec<u8>, UpdateError> {
    extract_member_capped(tar_gz, name, MAX_UNCOMPRESSED_NXS_BYTES)
}

/// Extract the `name` member, refusing one whose decompressed size exceeds `max_bytes`. Matches the
/// bare member name exactly so a `../`/absolute-path entry in a tampered archive can never escape
/// (defense in depth behind the verified sha256 + signature; the archive ships bare `nxf`/`nxm`/
/// `nxs`/`nxf-relay`).
fn extract_member_capped(
    tar_gz: &[u8],
    name: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, UpdateError> {
    use std::io::Read;
    let decoder = flate2::read::GzDecoder::new(tar_gz);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive
        .entries()
        .map_err(|e| UpdateError::Io(format!("reading tarball: {e}")))?
    {
        let entry = entry.map_err(|e| UpdateError::Io(format!("reading tar entry: {e}")))?;
        let path = entry
            .path()
            .map_err(|e| UpdateError::Io(format!("reading tar entry path: {e}")))?;
        // Match the bare member name exactly — never honor a `../` or absolute path.
        if path.as_os_str() == name {
            let mut buf = Vec::new();
            // Read at most max_bytes+1 so an oversized member is detected, not slurped whole.
            entry
                .take(max_bytes + 1)
                .read_to_end(&mut buf)
                .map_err(|e| UpdateError::Io(format!("reading {name} from tarball: {e}")))?;
            if buf.len() as u64 > max_bytes {
                return Err(UpdateError::Io(format!(
                    "decompressed {name} exceeds {max_bytes} bytes (refusing — possible decompression bomb)"
                )));
            }
            return Ok(buf);
        }
    }
    Err(UpdateError::MissingBinary(name.to_string()))
}

/// Atomically replace the file at `target` with `new_bytes`: write a temp sibling in the same
/// directory, set it executable, then rename over the target. A crashed `nxf` mid-update is
/// never a half-written binary, and the new bytes only become visible once fully written.
fn atomic_replace(target: &Path, new_bytes: &[u8]) -> Result<(), UpdateError> {
    atomic_replace_with_mode(target, new_bytes, 0o755)
}

/// [`atomic_replace`] with the unix mode stated: 755 for a binary, 644 for a shipped data file such
/// as the sidecar bundle (which `node` reads and nothing execs). `mode` is ignored off unix, where
/// the concept does not apply.
fn atomic_replace_with_mode(target: &Path, new_bytes: &[u8], mode: u32) -> Result<(), UpdateError> {
    // Windows has no unix permission bits; naming the binding here keeps the parameter honest
    // rather than dead on that target (nxf 6j6v.rf4b is the epic that will build it).
    #[cfg(not(unix))]
    let _ = mode;
    let dir = target.parent().ok_or_else(|| {
        UpdateError::Io(format!("target path has no parent: {}", target.display()))
    })?;
    let file_name = target.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        UpdateError::Io(format!(
            "target path has no file name: {}",
            target.display()
        ))
    })?;
    // Temp sibling in the SAME directory so the final rename stays on one filesystem (a
    // cross-device rename is not atomic). pid-scoped so two concurrent updates don't collide.
    let tmp = dir.join(format!(".{file_name}.tmp.{}", std::process::id()));

    let result = (|| {
        std::fs::write(&tmp, new_bytes)
            .map_err(|e| UpdateError::Io(format!("writing temp binary {}: {e}", tmp.display())))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))
                .map_err(|e| UpdateError::Io(format!("chmod temp binary: {e}")))?;
        }
        std::fs::rename(&tmp, target).map_err(|e| {
            UpdateError::Io(format!(
                "installing new binary over {}: {e}",
                target.display()
            ))
        })
    })();

    // On any failure leave the running binary untouched and never strand a temp sibling.
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// The `/updater` 200 body (spec §6.1). `signature` is the FULL `.minisig` text (the
/// publish/promote scripts embed it verbatim) so we verify against the baked-in pubkey
/// without a second fetch; `sha256` is the lowercase hex integrity digest.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct UpdaterResponse {
    pub version: String,
    pub pub_date: String,
    pub notes: String,
    pub url: String,
    pub sha256: String,
    pub signature: String,
}

/// The outcome of asking `/updater` whether to update. A 204 (or a 200 the client refuses
/// as a downgrade) is [`UpdateStatus::UpToDate`]; a 200 with a strictly-newer version is
/// [`UpdateStatus::Available`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    UpToDate,
    Available(UpdaterResponse),
}

/// Decide the status from the current version and the updater's answer (`None` == 204).
/// The Lambda already gates strictly-newer, but the client re-checks: a 200 that is not
/// strictly newer than `current` is treated as up-to-date and never installed — the
/// no-downgrade guarantee must not depend on the server alone (defense in depth, §7.2).
pub fn decide_status(current: &str, response: Option<UpdaterResponse>) -> UpdateStatus {
    match response {
        Some(resp) if is_strictly_newer(current, &resp.version) => UpdateStatus::Available(resp),
        _ => UpdateStatus::UpToDate,
    }
}

/// Parsed plain SemVer `X.Y.Z` (the only shape the project ships — spec §4.3).
type Semver = (u64, u64, u64);

/// Parse strict plain SemVer `X.Y.Z`. A `v`-prefix, suffix, or partial ⇒ `None`. Mirrors
/// the updater Lambda's `parseSemver` so client and server agree on what "a version" is.
fn parse_semver(s: &str) -> Option<Semver> {
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None; // a fourth component ⇒ not plain X.Y.Z
    }
    Some((major, minor, patch))
}

/// Is `candidate` strictly newer than `current` by SemVer? The no-downgrade rule: a
/// self-update installs only a strictly-greater version, so a ring-change onto an older
/// stable keeps the installed binary until the ring catches up (reference behavior, §7.2).
/// Unparseable input fails closed (`false` — never "newer").
pub fn is_strictly_newer(current: &str, candidate: &str) -> bool {
    match (parse_semver(current), parse_semver(candidate)) {
        (Some(cur), Some(cand)) => cand > cur,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use nxs_service::ProgramState;

    /// The measured case, as a unit: three updates land in `~/.local/bin` while the service's alias
    /// still names a build tree, and nothing says so.
    #[test]
    fn an_update_that_leaves_the_service_on_another_build_names_both_paths() {
        let note = service_alias_note(
            &ProgramState::Present(PathBuf::from(
                "/Users/u/Development/nexus-flow/target/debug/nxs",
            )),
            Path::new("/Users/u/.nexusflow/bin/nexus-flow"),
            Path::new("/Users/u/.local/bin/nxs"),
        )
        .expect("a divergence is exactly what this reports");
        assert!(note.contains("/Users/u/.local/bin/nxs"), "{note}");
        assert!(
            note.contains("/Users/u/Development/nexus-flow/target/debug/nxs"),
            "{note}"
        );
        assert!(note.contains("nxs sync daemon install"), "{note}");
    }

    #[test]
    fn an_update_that_moved_the_binary_the_service_already_runs_says_nothing() {
        assert_eq!(
            service_alias_note(
                &ProgramState::Present(PathBuf::from("/Users/u/.local/bin/nxs")),
                Path::new("/Users/u/.nexusflow/bin/nexus-flow"),
                Path::new("/Users/u/.local/bin/nxs"),
            ),
            None,
            "the ordinary, healthy update must stay quiet"
        );
    }

    #[test]
    fn a_machine_with_no_service_installed_is_never_nagged_about_one() {
        assert_eq!(
            service_alias_note(
                &ProgramState::Absent,
                Path::new("/Users/u/.nexusflow/bin/nexus-flow"),
                Path::new("/Users/u/.local/bin/nxs"),
            ),
            None
        );
    }

    #[test]
    fn a_dead_alias_is_reported_by_the_update_too_and_points_at_the_binary_it_just_wrote() {
        let note = service_alias_note(
            &ProgramState::Dangling(PathBuf::from("/repo/target/debug/nxs")),
            Path::new("/Users/u/.nexusflow/bin/nexus-flow"),
            Path::new("/Users/u/.local/bin/nxs"),
        )
        .expect("a dead alias is always worth saying");
        assert!(note.contains("/repo/target/debug/nxs"), "{note}");
        assert!(note.contains("/Users/u/.local/bin/nxs"), "{note}");
    }

    #[test]
    fn the_note_rides_the_json_receipt_and_is_absent_when_there_is_nothing_to_say() {
        let plain = with_service_alias(updated_json("0.71.0", "0.72.0", "beta"), None);
        assert!(
            plain.get("service_alias").is_none(),
            "an added key on every update would change a byte contract for nothing: {plain}"
        );
        let noted = with_service_alias(updated_json("0.71.0", "0.72.0", "beta"), Some("drifted"));
        assert_eq!(noted["service_alias"], "drifted");
        assert_eq!(
            noted["new_version"], "0.72.0",
            "the receipt is otherwise untouched"
        );
    }

    use super::*;

    #[test]
    fn strictly_newer_compares_each_component() {
        assert!(is_strictly_newer("0.1.0", "0.2.0"));
        assert!(is_strictly_newer("0.1.0", "0.1.1"));
        assert!(is_strictly_newer("0.9.9", "1.0.0"));
    }

    #[test]
    fn no_key_selfupdate_message_is_actionable_not_internal() {
        // eprg: the self-update no-key refusal must name the cause and the fix for its real
        // audience (a locally-built binary) and never point at a repo-internal document.
        let msg = no_key_selfupdate_message("nxs");
        assert!(
            !msg.contains("release-management") && !msg.contains(".md") && !msg.contains("§12"),
            "no repo-internal reference in the user-facing message: {msg}"
        );
        assert!(msg.contains("unsigned"), "names the likely cause: {msg}");
        assert!(
            msg.contains("install.sh"),
            "points to the official signed release: {msg}"
        );
        assert!(
            msg.contains("nxs"),
            "keeps the brand it was invoked as: {msg}"
        );
    }

    #[test]
    fn the_migration_hint_is_due_only_from_the_version_that_carries_it() {
        // nxf 6j6v.9yaj: "self-update gives the hint exactly once" — the version half. An install
        // still below the threshold has nothing to be told about yet.
        assert!(migration_hint_due(MIGRATION_MIN_VERSION, false), "at it");
        assert!(migration_hint_due("0.48.1", false), "past it");
        assert!(migration_hint_due("1.0.0", false), "well past it");
        assert!(!migration_hint_due("0.47.0", false), "before it");
        assert!(
            !migration_hint_due("0.9.0", false),
            "0.9 < 0.48 componentwise"
        );
    }

    #[test]
    fn the_migration_hint_is_shown_once_and_then_never_again() {
        // The "exactly once" half, over the real persisted flag: a line that reappears on every
        // update is a line people learn to skip, which is the same as not printing it.
        let dir = tempfile::TempDir::new().unwrap();
        assert!(migration_hint_due(MIGRATION_MIN_VERSION, false));

        maybe_emit_migration_hint(dir.path(), MIGRATION_MIN_VERSION);
        assert_eq!(
            load_user_config(dir.path()).migration_hint_shown,
            Some(true),
            "the install records that it was told"
        );
        assert!(
            !migration_hint_due(MIGRATION_MIN_VERSION, true),
            "and it is not due a second time"
        );

        // …and recording it does not disturb what else the file carries.
        let mut cfg = load_user_config(dir.path());
        cfg.channel = Some("beta".into());
        save_user_config(dir.path(), &cfg).unwrap();
        maybe_emit_migration_hint(dir.path(), "1.0.0");
        let after = load_user_config(dir.path());
        assert_eq!(after.channel.as_deref(), Some("beta"));
        assert_eq!(after.migration_hint_shown, Some(true));
    }

    #[test]
    fn equal_or_older_is_not_newer() {
        assert!(!is_strictly_newer("0.2.0", "0.2.0")); // equal ⇒ no update
        assert!(!is_strictly_newer("0.2.0", "0.1.9")); // older ⇒ no downgrade
        assert!(!is_strictly_newer("1.0.0", "0.9.9"));
    }

    #[test]
    fn unparseable_versions_fail_closed() {
        assert!(!is_strictly_newer("0.2.0", "not-a-version"));
        assert!(!is_strictly_newer("garbage", "0.3.0"));
        assert!(!is_strictly_newer("v0.1.0", "v0.2.0")); // v-prefix is not plain SemVer
    }

    fn sample_response(version: &str) -> UpdaterResponse {
        UpdaterResponse {
            version: version.to_string(),
            pub_date: "2026-06-12T00:00:00Z".to_string(),
            notes: "notes".to_string(),
            url: "https://nxf.example/download/stable/9.9.9/nxf_9.9.9_linux-x86_64.tar.gz"
                .to_string(),
            sha256: "abc".to_string(),
            signature: "untrusted comment: x\nsig\ntrusted comment: y\nglobalsig".to_string(),
        }
    }

    #[test]
    fn no_content_is_up_to_date() {
        assert_eq!(decide_status("0.1.0", None), UpdateStatus::UpToDate);
    }

    #[test]
    fn strictly_newer_offer_is_available() {
        let resp = sample_response("0.2.0");
        assert_eq!(
            decide_status("0.1.0", Some(resp.clone())),
            UpdateStatus::Available(resp)
        );
    }

    #[test]
    fn non_newer_offer_is_refused_as_up_to_date() {
        // A 200 that is not strictly newer (equal or older) must never install — the
        // client enforces no-downgrade even if the server offered it.
        assert_eq!(
            decide_status("0.2.0", Some(sample_response("0.2.0"))),
            UpdateStatus::UpToDate
        );
        assert_eq!(
            decide_status("0.2.0", Some(sample_response("0.1.0"))),
            UpdateStatus::UpToDate
        );
    }

    // ---- verify / extract / swap -------------------------------------------

    use minisign::{sign, KeyPair};
    use std::io::Cursor;

    /// sha256 + minisig + pubkey for `tar_gz`, signed by a fresh ephemeral key.
    fn sign_tar_gz(tar_gz: &[u8]) -> (String, String, String) {
        let sha = sha256_hex(tar_gz);
        let KeyPair { pk, sk } = KeyPair::generate_unencrypted_keypair().unwrap();
        let sig = sign(
            None,
            &sk,
            Cursor::new(tar_gz),
            Some("trusted"),
            Some("untrusted"),
        )
        .unwrap();
        (sha, sig.into_string(), pk.to_base64())
    }

    /// Build a gzip-tar containing one member `nxf` with `nxf_bytes`, and return
    /// `(tar_gz, sha256_hex, minisig, pubkey_base64)` signed by a fresh ephemeral key.
    fn make_signed_tarball(nxs_bytes: &[u8]) -> (Vec<u8>, String, String, String) {
        // Multicall (5jz.7): the tarball ships ONE real binary, `nxs` (personas are local symlinks).
        let tar_gz = tar_gz_with(&[("nxs", nxs_bytes)]);
        let (sha, sig, pk) = sign_tar_gz(&tar_gz);
        (tar_gz, sha, sig, pk)
    }

    #[test]
    fn verify_accepts_matching_sha_and_signature() {
        let (tar_gz, sha, sig, pk) = make_signed_tarball(b"#!/fake nxf");
        assert!(verify_artifact_with_key(&tar_gz, &sha, &sig, &pk).is_ok());
    }

    #[test]
    fn verify_rejects_a_sha_mismatch_before_anything_else() {
        let (tar_gz, _sha, sig, pk) = make_signed_tarball(b"payload");
        let err =
            verify_artifact_with_key(&tar_gz, "00".repeat(32).as_str(), &sig, &pk).unwrap_err();
        assert!(matches!(err, UpdateError::Sha256Mismatch { .. }));
    }

    #[test]
    fn verify_rejects_a_tampered_artifact_even_with_a_correct_sha() {
        // sha matches the tampered bytes (an attacker who controls the bytes controls the
        // hash), but the signature does not verify under the trusted key ⇒ refuse. This is
        // exactly the case sha256 alone cannot catch — the authenticity gate must be unconditional.
        let (tar_gz, _sha, sig, pk) = make_signed_tarball(b"original");
        let mut tampered = tar_gz.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0xff;
        let tampered_sha = sha256_hex(&tampered);
        let err = verify_artifact_with_key(&tampered, &tampered_sha, &sig, &pk).unwrap_err();
        assert!(matches!(err, UpdateError::Signature(_)));
    }

    /// Build a gzip-tar with several named members (no signing — for extraction tests).
    fn tar_gz_with(members: &[(&str, &[u8])]) -> Vec<u8> {
        use flate2::{write::GzEncoder, Compression};
        let mut tar_gz = Vec::new();
        {
            let enc = GzEncoder::new(&mut tar_gz, Compression::default());
            let mut builder = tar::Builder::new(enc);
            for (name, bytes) in members {
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder.append_data(&mut header, name, *bytes).unwrap();
            }
            builder.into_inner().unwrap().finish().unwrap();
        }
        tar_gz
    }

    #[test]
    fn extract_pulls_the_nxs_member() {
        let (tar_gz, _sha, _sig, _pk) = make_signed_tarball(b"the new nxs bytes");
        assert_eq!(
            extract_member(&tar_gz, REAL_BINARY).unwrap(),
            b"the new nxs bytes"
        );
    }

    #[test]
    fn extract_pulls_each_real_binary_from_the_tarball() {
        // Multicall (5jz.7): the release tarball ships the real binaries `nxs` + `nxf-relay` (the
        // personas are local symlinks, not tarball members); self-update extracts each by exact name.
        let tar_gz = tar_gz_with(&[("nxs", b"umbrella-cli"), ("nxf-relay", b"relay")]);
        assert_eq!(extract_member(&tar_gz, "nxs").unwrap(), b"umbrella-cli");
        assert_eq!(extract_member(&tar_gz, "nxf-relay").unwrap(), b"relay");
    }

    #[test]
    fn extract_missing_member_names_the_member() {
        // A tarball that lacks a required suite binary fails loud, naming which one is absent —
        // exactly the v0.6.0 gap (nxs/nxm missing) surfaced instead of silently skipped.
        let tar_gz = tar_gz_with(&[("nxf", b"flow-cli")]);
        let err = extract_member(&tar_gz, "nxs").unwrap_err();
        assert!(
            matches!(&err, UpdateError::MissingBinary(m) if m == "nxs"),
            "{err}"
        );
    }

    #[test]
    fn extract_refuses_a_member_over_the_decompression_cap() {
        // A member that decompresses past the cap is refused (decompression-bomb guard), rather
        // than read whole into memory. `make_signed_tarball` writes a 10-byte member here.
        let (tar_gz, _sha, _sig, _pk) = make_signed_tarball(b"0123456789");
        let err = extract_member_capped(&tar_gz, REAL_BINARY, 4).unwrap_err();
        assert!(matches!(err, UpdateError::Io(_)));
        // The same member is fine under a generous cap.
        assert_eq!(
            extract_member_capped(&tar_gz, REAL_BINARY, 1024).unwrap(),
            b"0123456789"
        );
    }

    #[test]
    fn same_origin_pins_scheme_and_authority() {
        assert!(same_origin(
            "https://nxsflow.com/nxs/download/stable/0.1.0/nxf.tar.gz",
            "https://nxsflow.com/nxs"
        ));
        // Discriminator: origin-pinning is scheme+host+port, NOT string-prefix. A same-origin URL
        // whose path does NOT start with the base's `/nxs` path is still same-origin — a naive
        // `url.starts_with(base)` guard would wrongly reject this, origin-stripping accepts it. This
        // is the case the bare-host fixtures can't distinguish, and it matters now that the base
        // carries a `/nxs` path (brand-domain cutover) and nxsflow.com is a shared host.
        assert!(same_origin(
            "https://nxsflow.com/elsewhere/x.tar.gz",
            "https://nxsflow.com/nxs"
        ));
        assert!(same_origin(
            "http://127.0.0.1:8731/download/x",
            "http://127.0.0.1:8731"
        ));
        // Different host, scheme downgrade, port mismatch, or a host that merely SUFFIXES ours ⇒
        // not the same origin.
        assert!(!same_origin(
            "https://evil.example/x",
            "https://nxsflow.com/nxs"
        ));
        assert!(!same_origin(
            "https://nxsflow.com.evil.example/nxs/x",
            "https://nxsflow.com/nxs"
        ));
        assert!(!same_origin(
            "http://nxsflow.com/nxs/x",
            "https://nxsflow.com/nxs"
        ));
        assert!(!same_origin(
            "http://127.0.0.1:9999/x",
            "http://127.0.0.1:8731"
        ));
        assert!(!same_origin("not-a-url", "https://nxsflow.com/nxs"));
    }

    #[test]
    fn atomic_replace_swaps_the_target_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("nxf");
        std::fs::write(&target, b"old binary").unwrap();
        atomic_replace(&target, b"new binary").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new binary");
        // No stray temp siblings left behind in the directory.
        let leftover: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() != "nxf")
            .collect();
        assert!(leftover.is_empty(), "temp sibling not cleaned up");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_replace_sets_the_executable_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("nxf");
        std::fs::write(&target, b"old").unwrap();
        atomic_replace(&target, b"new").unwrap();
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "installed binary must be executable");
    }

    // ---- install identity (user config) ------------------------------------

    #[test]
    fn install_identity_defaults_to_stable_and_mints_a_machine_id() {
        let dir = tempfile::tempdir().unwrap();
        let (channel, machine_id) = ensure_install_identity(dir.path(), None).unwrap();
        assert_eq!(channel, "stable");
        assert!(!machine_id.is_empty());
        // It was persisted: a config.toml now exists carrying the minted id.
        let on_disk = load_user_config(dir.path());
        assert_eq!(on_disk.machine_id.as_deref(), Some(machine_id.as_str()));
    }

    #[test]
    fn machine_id_is_durable_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let (_, first) = ensure_install_identity(dir.path(), None).unwrap();
        let (_, second) = ensure_install_identity(dir.path(), None).unwrap();
        assert_eq!(
            first, second,
            "machine_id must be minted once, not re-rolled"
        );
    }

    #[test]
    fn explicit_channel_is_persisted_and_then_sticky() {
        let dir = tempfile::tempdir().unwrap();
        let (channel, _) = ensure_install_identity(dir.path(), Some("beta")).unwrap();
        assert_eq!(channel, "beta");
        // A later run with no --channel uses the persisted value.
        let (later, _) = ensure_install_identity(dir.path(), None).unwrap();
        assert_eq!(later, "beta");
    }

    #[test]
    fn a_corrupt_persisted_channel_falls_back_to_stable() {
        let dir = tempfile::tempdir().unwrap();
        save_user_config(
            dir.path(),
            &UserConfig {
                channel: Some("nonsense".to_string()),
                machine_id: Some("mid".to_string()),
                migration_hint_shown: None,
            },
        )
        .unwrap();
        let (channel, machine_id) = ensure_install_identity(dir.path(), None).unwrap();
        assert_eq!(channel, "stable");
        assert_eq!(machine_id, "mid"); // a valid machine_id is still honored
    }

    // ---- HTTP glue (fetch + install) against a real local server ------------

    /// Spin a minimal updater on a loopback port. When `offer` is `Some((tar_gz, sha, sig))`
    /// it answers `/updater` with 200 + a contract body whose `url` points back at its own
    /// `/download/nxf.tar.gz`; when `None` it answers 204. Returns the base URL.
    fn spawn_updater(offer: Option<(Vec<u8>, String, String)>) -> String {
        use axum::extract::State;
        use axum::http::{header, StatusCode};
        use axum::response::IntoResponse;
        use axum::routing::get;

        #[derive(Clone)]
        struct Srv {
            base: String,
            tar_gz: Option<Vec<u8>>,
            sha256: String,
            signature: String,
        }

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{addr}");

        let (tar_gz, sha256, signature) = match offer {
            Some((t, s, g)) => (Some(t), s, g),
            None => (None, String::new(), String::new()),
        };
        let state = Srv {
            base: base.clone(),
            tar_gz,
            sha256,
            signature,
        };

        async fn updater(State(s): State<Srv>) -> axum::response::Response {
            match &s.tar_gz {
                None => StatusCode::NO_CONTENT.into_response(),
                Some(_) => axum::Json(serde_json::json!({
                    "version": "9.9.9",
                    "pub_date": "2026-06-12T00:00:00Z",
                    "notes": "test build",
                    "url": format!("{}/download/nxf.tar.gz", s.base),
                    "sha256": s.sha256,
                    "signature": s.signature,
                }))
                .into_response(),
            }
        }
        async fn download(State(s): State<Srv>) -> axum::response::Response {
            match &s.tar_gz {
                Some(bytes) => {
                    ([(header::CONTENT_TYPE, "application/gzip")], bytes.clone()).into_response()
                }
                None => StatusCode::NOT_FOUND.into_response(),
            }
        }

        let router = axum::Router::new()
            .route("/updater", get(updater))
            .route("/download/nxf.tar.gz", get(download))
            .with_state(state);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                axum::serve(listener, router).await.unwrap();
            });
        });
        base
    }

    #[test]
    fn fetch_returns_none_on_204() {
        let base = spawn_updater(None);
        let got = fetch_update(
            &updater_agent(),
            &base,
            "stable",
            "linux",
            "x86_64",
            "0.1.0",
            "mid",
        )
        .unwrap();
        assert_eq!(got, None);
    }

    #[test]
    fn fetch_parses_the_200_offer() {
        let (tar_gz, sha, sig, _pk) = make_signed_tarball(b"x");
        let base = spawn_updater(Some((tar_gz, sha.clone(), sig)));
        let got = fetch_update(
            &updater_agent(),
            &base,
            "stable",
            "linux",
            "x86_64",
            "0.1.0",
            "mid",
        )
        .unwrap()
        .expect("an offer");
        assert_eq!(got.version, "9.9.9");
        assert_eq!(got.sha256, sha);
    }

    #[test]
    fn fetch_refuses_an_oversized_updater_body() {
        use axum::routing::get;
        // A hostile updater streams a body far over the cap. fetch_update must refuse rather than
        // slurp it into memory (OOM guard), symmetric with the bounded download path.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        async fn flood() -> String {
            "x".repeat((MAX_UPDATER_JSON_BYTES as usize) + 4096)
        }
        let router = axum::Router::new().route("/updater", get(flood));
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let l = tokio::net::TcpListener::from_std(listener).unwrap();
                axum::serve(l, router).await.unwrap();
            });
        });
        let base = format!("http://{addr}");
        let err = fetch_update(
            &updater_agent(),
            &base,
            "stable",
            "linux",
            "x86_64",
            "0.1.0",
            "mid",
        )
        .unwrap_err();
        assert!(matches!(err, UpdateError::Http(_)));
    }

    #[test]
    fn install_from_swaps_the_one_binary_and_links_the_personas_end_to_end() {
        // Multicall (5jz.7): the tarball ships ONE real `nxs`; install_from swaps `<dir>/nxs` and
        // (re)points the `nxf`/`nxm` persona symlinks at it — migrating a pre-multicall layout
        // (real `nxf`/`nxm` files) to the single-binary model on this update.
        let (tar_gz, sha, sig, pk) = make_signed_tarball(b"the freshly downloaded nxs");
        let base = spawn_updater(Some((tar_gz, sha, sig)));
        let resp = fetch_update(
            &updater_agent(),
            &base,
            "stable",
            "linux",
            "x86_64",
            "0.1.0",
            "mid",
        )
        .unwrap()
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        // A pre-multicall layout: nxs + REAL nxf/nxm files (no symlinks yet).
        std::fs::write(dir.path().join("nxs"), b"the old nxs").unwrap();
        std::fs::write(dir.path().join("nxf"), b"the old real nxf").unwrap();
        std::fs::write(dir.path().join("nxm"), b"the old real nxm").unwrap();

        install_from(&updater_agent(), &resp, &pk, dir.path(), &base).unwrap();

        // The one real binary was swapped…
        assert_eq!(
            std::fs::read(dir.path().join("nxs")).unwrap(),
            b"the freshly downloaded nxs"
        );
        // …and the personas are now symlinks → nxs (the stale real files were replaced).
        for persona in PERSONA_LINKS {
            let link = dir.path().join(persona);
            assert!(
                std::fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                "{persona} is a symlink"
            );
            assert_eq!(std::fs::read_link(&link).unwrap().as_os_str(), "nxs");
            // Resolving the link reads the freshly-swapped nxs bytes.
            assert_eq!(std::fs::read(&link).unwrap(), b"the freshly downloaded nxs");
        }
    }

    #[test]
    fn install_from_refuses_an_off_origin_download_url() {
        // The /updater response points the download off the origin we queried (SSRF / forced
        // download). install_from must refuse BEFORE fetching, and leave the binary untouched —
        // even though the signature gate would also reject a foreign artifact.
        let (tar_gz, sha, sig, pk) = make_signed_tarball(b"x");
        let base = spawn_updater(Some((tar_gz, sha, sig)));
        let mut resp = fetch_update(
            &updater_agent(),
            &base,
            "stable",
            "linux",
            "x86_64",
            "0.1.0",
            "mid",
        )
        .unwrap()
        .unwrap();
        resp.url = "http://evil.example/download/nxf.tar.gz".to_string();

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("nxs");
        std::fs::write(&target, b"the old nxs").unwrap();
        let err = install_from(&updater_agent(), &resp, &pk, dir.path(), &base).unwrap_err();
        assert!(matches!(err, UpdateError::UntrustedOrigin { .. }));
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"the old nxs",
            "binary untouched"
        );
    }

    #[test]
    fn install_from_refuses_a_forged_tarball_and_keeps_the_old_binary() {
        // The server hands out a tarball whose bytes don't match the advertised sha256
        // (an origin/CDN compromise). install_from must abort and leave the binary untouched.
        let (tar_gz, _sha, sig, pk) = make_signed_tarball(b"good");
        let bad_sha = "00".repeat(32);
        let base = spawn_updater(Some((tar_gz, bad_sha, sig)));
        let resp = fetch_update(
            &updater_agent(),
            &base,
            "stable",
            "linux",
            "x86_64",
            "0.1.0",
            "mid",
        )
        .unwrap()
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("nxs");
        std::fs::write(&target, b"the old nxs").unwrap();
        let err = install_from(&updater_agent(), &resp, &pk, dir.path(), &base).unwrap_err();
        assert!(matches!(err, UpdateError::Sha256Mismatch { .. }));
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"the old nxs",
            "binary untouched"
        );
    }

    // ---- the sidecar beside the binary (nexus-flow-6j6v.81v5, 6j6v.smsz) ---

    #[test]
    fn install_from_places_a_sidecar_a_pinned_older_release_still_carries() {
        // No CURRENT release carries a sidecar member — it is compiled into `nxs` (6j6v.smsz) — but
        // `--version` can pin 0.52.0/0.53.0, whose binaries have no embedded copy and can only find
        // one beside themselves. For those, an update that swaps the binary and leaves no sidecar
        // behind is not an update that fixes anything, so the placing half stays exactly as it was.
        let tar_gz = tar_gz_with(&[
            ("nxs", b"the freshly downloaded nxs"),
            (SIDECAR_FILE, b"// the bundled sidecar"),
        ]);
        let (sha, sig, pk) = sign_tar_gz(&tar_gz);
        let base = spawn_updater(Some((tar_gz, sha, sig)));
        let resp = fetch_update(
            &updater_agent(),
            &base,
            "stable",
            "linux",
            "x86_64",
            "0.1.0",
            "mid",
        )
        .unwrap()
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("nxs"), b"the old nxs").unwrap();
        install_from(&updater_agent(), &resp, &pk, dir.path(), &base).unwrap();

        let sidecar = dir.path().join(SIDECAR_FILE);
        assert_eq!(std::fs::read(&sidecar).unwrap(), b"// the bundled sidecar");
        // Beside the binary is the CONTRACT, not a convenience: `nexus_chat::worker::sidecar_beside`
        // looks exactly there, relative to the running program, so that no environment variable is
        // needed. And 644, not 755 — `node` reads it, nothing execs it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&sidecar).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o644, "the sidecar is data, not a program");
        }
    }

    #[test]
    fn install_from_still_succeeds_against_a_release_that_ships_no_sidecar() {
        // The NORMAL path since 6j6v.smsz (and every release cut before 6j6v.81v5). The binary swap
        // has ALREADY happened by the time the member turns out to be absent, so turning that into
        // an error would report a failed update that in fact completed.
        let (tar_gz, sha, sig, pk) = make_signed_tarball(b"an older nxs");
        let base = spawn_updater(Some((tar_gz, sha, sig)));
        let resp = fetch_update(
            &updater_agent(),
            &base,
            "stable",
            "linux",
            "x86_64",
            "0.1.0",
            "mid",
        )
        .unwrap()
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("nxs"), b"the old nxs").unwrap();
        install_from(&updater_agent(), &resp, &pk, dir.path(), &base).unwrap();

        assert_eq!(
            std::fs::read(dir.path().join("nxs")).unwrap(),
            b"an older nxs"
        );
        assert!(!dir.path().join(SIDECAR_FILE).exists());
    }

    #[test]
    fn install_from_clears_a_sidecar_the_new_release_no_longer_carries() {
        // nxf 6j6v.5vct, closed by construction rather than by policing. A machine that installed
        // 0.52.0/0.53.0 has a 1.4 MB bundle sitting beside `nxs`, belonging to a version that is
        // about to stop being installed. Nothing used to remove it, and the pair could drift.
        //
        // It is INERT after this update — `installed_sidecar` prefers the embedded copy — so the
        // removal reclaims the space and ends the question rather than changing which sidecar runs.
        // Best-effort: the update itself must not fail over housekeeping.
        let (tar_gz, sha, sig, pk) = make_signed_tarball(b"an nxs that carries its own sidecar");
        let base = spawn_updater(Some((tar_gz, sha, sig)));
        let resp = fetch_update(
            &updater_agent(),
            &base,
            "stable",
            "linux",
            "x86_64",
            "0.1.0",
            "mid",
        )
        .unwrap()
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("nxs"), b"the old nxs").unwrap();
        let leftover = dir.path().join(SIDECAR_FILE);
        std::fs::write(&leftover, b"// the 0.53.0 bundle").unwrap();

        install_from(&updater_agent(), &resp, &pk, dir.path(), &base).unwrap();

        assert_eq!(
            std::fs::read(dir.path().join("nxs")).unwrap(),
            b"an nxs that carries its own sidecar"
        );
        assert!(
            !leftover.exists(),
            "the superseded bundle is gone, not left to age beside the binary"
        );
    }

    #[test]
    fn install_from_survives_a_stale_sidecar_it_cannot_remove() {
        // PR #302 review, Test Quality #3. The removal is HOUSEKEEPING and runs after the binary
        // swap has already happened, so a failure there must not turn a completed update into a
        // reported failure — the state with no way forward. A directory at the sidecar's path makes
        // `remove_file` fail on every platform without needing a permission trick that a root
        // container would defeat.
        let (tar_gz, sha, sig, pk) = make_signed_tarball(b"an nxs that carries its own sidecar");
        let base = spawn_updater(Some((tar_gz, sha, sig)));
        let resp = fetch_update(
            &updater_agent(),
            &base,
            "stable",
            "linux",
            "x86_64",
            "0.1.0",
            "mid",
        )
        .unwrap()
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("nxs"), b"the old nxs").unwrap();
        let blocked = dir.path().join(SIDECAR_FILE);
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("occupant"), b"x").unwrap();

        install_from(&updater_agent(), &resp, &pk, dir.path(), &base)
            .expect("a leftover that cannot be removed is not a failed update");

        assert_eq!(
            std::fs::read(dir.path().join("nxs")).unwrap(),
            b"an nxs that carries its own sidecar",
            "the update itself completed"
        );
        assert!(
            blocked.is_dir(),
            "and the obstacle was left alone, not forced"
        );
    }

    // ---- persona-link self-heal (nexus-flow-224, multicall) ----------------

    #[test]
    fn ensure_persona_links_creates_missing_symlinks_to_nxs() {
        // A dir with only the real `nxs`: ensure creates EVERY persona symlink → nxs, reporting
        // them all. Pinning the exact set (not just "iterates PERSONA_LINKS") is the 6j6v.kew6
        // regression guard: `nxc` was missing from `PERSONA_LINKS`, so a fresh install/self-update
        // never linked it and `nxs init` for chat failed. Adding a fourth persona later must update
        // this assertion — that's the point.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("nxs"), b"nxs").unwrap();

        let fixed = ensure_persona_links(dir.path()).unwrap();
        assert_eq!(
            fixed,
            vec!["nxf".to_string(), "nxm".to_string(), "nxc".to_string()]
        );
        for persona in PERSONA_LINKS {
            let link = dir.path().join(persona);
            assert!(persona_link_ok(&link), "{persona} is a symlink → nxs");
            assert_eq!(std::fs::read_link(&link).unwrap().as_os_str(), "nxs");
        }
    }

    #[test]
    fn ensure_persona_links_is_a_noop_when_already_correct() {
        // A healthy multicall layout self-heals to nothing (no churn, no report).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("nxs"), b"nxs").unwrap();
        ensure_persona_links(dir.path()).unwrap(); // first call creates them
        let again = ensure_persona_links(dir.path()).unwrap();
        assert!(again.is_empty(), "second call is a no-op: {again:?}");
    }

    #[test]
    fn ensure_persona_links_replaces_a_stale_real_persona_file() {
        // A pre-multicall layout (nxf is a REAL file): ensure replaces it with a symlink → nxs.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("nxs"), b"nxs").unwrap();
        std::fs::write(dir.path().join("nxf"), b"stale real nxf").unwrap();
        std::fs::write(dir.path().join("nxm"), b"stale real nxm").unwrap();

        let fixed = ensure_persona_links(dir.path()).unwrap();
        assert_eq!(
            fixed,
            vec!["nxf".to_string(), "nxm".to_string(), "nxc".to_string()]
        );
        assert!(persona_link_ok(&dir.path().join("nxf")));
        assert!(persona_link_ok(&dir.path().join("nxm")));
        assert!(persona_link_ok(&dir.path().join("nxc")));
    }

    #[test]
    fn ensure_persona_links_fixes_a_link_pointing_at_the_wrong_target() {
        // A persona symlink that points somewhere other than `nxs` is re-pointed.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("nxs"), b"nxs").unwrap();
        std::os::unix::fs::symlink("somewhere-else", dir.path().join("nxf")).unwrap();
        assert!(!persona_link_ok(&dir.path().join("nxf")));

        let fixed = ensure_persona_links(dir.path()).unwrap();
        assert!(fixed.contains(&"nxf".to_string()));
        assert_eq!(
            std::fs::read_link(dir.path().join("nxf"))
                .unwrap()
                .as_os_str(),
            "nxs"
        );
    }

    // ---- deterministic output shaping --------------------------------------

    #[test]
    fn up_to_date_json_is_the_stable_contract() {
        let got = status_json("0.1.0", "stable", &UpdateStatus::UpToDate);
        assert_eq!(
            got,
            serde_json::json!({
                "status": "up-to-date",
                "current_version": "0.1.0",
                "channel": "stable",
            })
        );
    }

    #[test]
    fn available_json_is_the_stable_contract() {
        let resp = sample_response("0.2.0");
        let got = status_json("0.1.0", "beta", &UpdateStatus::Available(resp.clone()));
        assert_eq!(
            got,
            serde_json::json!({
                "status": "update-available",
                "current_version": "0.1.0",
                "latest_version": "0.2.0",
                "channel": "beta",
                "pub_date": resp.pub_date,
                "notes": resp.notes,
                "url": resp.url,
            })
        );
    }

    #[test]
    fn updated_json_is_the_stable_contract() {
        assert_eq!(
            updated_json("0.1.0", "0.2.0", "stable"),
            serde_json::json!({
                "status": "updated",
                "previous_version": "0.1.0",
                "new_version": "0.2.0",
                "channel": "stable",
            })
        );
    }

    #[test]
    fn repaired_json_is_the_stable_contract() {
        assert_eq!(
            repaired_json("0.6.1", "beta", &["nxs".to_string()]),
            serde_json::json!({
                "status": "repaired",
                "current_version": "0.6.1",
                "channel": "beta",
                "repaired": ["nxs"],
            })
        );
    }

    #[test]
    fn human_lines_distinguish_up_to_date_from_available() {
        // The subject carries the invoking brand…
        let up = status_human("nxs", "0.1.0", "stable", &UpdateStatus::UpToDate);
        assert!(up.contains("up to date") && up.starts_with("nxs "));
        let avail = status_human(
            "nxs",
            "0.1.0",
            "stable",
            &UpdateStatus::Available(sample_response("0.2.0")),
        );
        assert!(avail.contains("0.2.0"));
        // …but the suggested action is always the canonical umbrella verb, even via `nxf`.
        assert!(avail.contains("nxs self-update"));
        let via_nxf = status_human(
            "nxf",
            "0.1.0",
            "stable",
            &UpdateStatus::Available(sample_response("0.2.0")),
        );
        assert!(
            via_nxf.starts_with("nxf ") && via_nxf.contains("nxs self-update"),
            "the deprecated nxf alias still steers to `nxs self-update`: {via_nxf}"
        );
    }

    // ---- update hint gating / throttle / content ---------------------------

    #[test]
    fn hint_is_disabled_for_any_non_interactive_use() {
        // --json (machine output)
        assert!(hint_disabled(true, false, false, true));
        // NXF_NO_UPDATE_CHECK
        assert!(hint_disabled(false, true, false, true));
        // CI
        assert!(hint_disabled(false, false, true, true));
        // stderr not a terminal (piped/captured — every agent flow & test harness)
        assert!(hint_disabled(false, false, false, false));
        // only an interactive human with none of the above sees it
        assert!(!hint_disabled(false, false, false, true));
    }

    #[test]
    fn env_flag_truthiness_follows_the_usual_convention() {
        assert!(env_flag_truthy(Some("1")));
        assert!(env_flag_truthy(Some("true")));
        assert!(env_flag_truthy(Some("yes")));
        assert!(!env_flag_truthy(Some("0")));
        assert!(!env_flag_truthy(Some("false")));
        assert!(!env_flag_truthy(Some("")));
        assert!(!env_flag_truthy(None));
    }

    #[test]
    fn ci_is_detected_from_ci_or_a_known_marker() {
        let with = |k: &str, v: &str| {
            let (k, v) = (k.to_string(), v.to_string());
            move |q: &str| if q == k { Some(v.clone()) } else { None }
        };
        assert!(is_ci_env(with("CI", "true")));
        assert!(is_ci_env(with("GITHUB_ACTIONS", "true")));
        assert!(!is_ci_env(with("CI", "false")));
        assert!(!is_ci_env(|_| None));
    }

    #[test]
    fn network_probe_is_throttled_to_once_a_day() {
        assert!(needs_network_refresh(None, "2026-06-12"));
        assert!(needs_network_refresh(Some("2026-06-11"), "2026-06-12"));
        assert!(!needs_network_refresh(Some("2026-06-12"), "2026-06-12"));
    }

    #[test]
    fn compute_hint_probes_caches_and_renders_then_serves_from_cache() {
        let (tar_gz, sha, sig, _pk) = make_signed_tarball(b"x");
        let base = spawn_updater(Some((tar_gz, sha, sig))); // offers 9.9.9
        let cache = tempfile::tempdir().unwrap();

        // First call: probes, writes the cache (today + latest), returns the nudge line.
        let line = compute_update_hint(cache.path(), &base, "0.1.0", "stable", "mid", "2026-06-12")
            .expect("a hint when a newer version is offered");
        assert!(line.contains("9.9.9"));
        let toml = std::fs::read_to_string(cache.path().join("update-check.toml")).unwrap();
        assert!(toml.contains("2026-06-12") && toml.contains("9.9.9"));

        // Same day, unreachable updater: served from cache, no probe, still the nudge.
        let again = compute_update_hint(
            cache.path(),
            "http://127.0.0.1:1", // would fail if it probed
            "0.1.0",
            "stable",
            "mid",
            "2026-06-12",
        );
        assert_eq!(again.as_deref(), Some(line.as_str()));
    }

    #[test]
    fn compute_hint_is_silent_when_already_current() {
        let base = spawn_updater(None); // 204 ⇒ up to date
        let cache = tempfile::tempdir().unwrap();
        let line = compute_update_hint(cache.path(), &base, "0.1.0", "stable", "mid", "2026-06-12");
        assert_eq!(line, None, "no nudge when the updater says we're current");
        // The probe was still throttled: the date is recorded.
        let toml = std::fs::read_to_string(cache.path().join("update-check.toml")).unwrap();
        assert!(toml.contains("2026-06-12"));
    }

    #[test]
    fn hint_line_only_when_strictly_newer() {
        let line = hint_line("0.1.0", Some("0.2.0")).unwrap();
        assert!(line.contains("0.2.0") && line.contains("self-update"));
        assert_eq!(hint_line("0.2.0", Some("0.2.0")), None); // equal
        assert_eq!(hint_line("0.2.0", Some("0.1.0")), None); // older
        assert_eq!(hint_line("0.2.0", None), None); // nothing known
    }

    #[test]
    fn response_parses_from_the_updater_contract_json() {
        let body = serde_json::json!({
            "version": "0.3.0",
            "pub_date": "2026-06-12T00:00:00Z",
            "notes": "fixes",
            "url": "https://nxf.example/download/stable/0.3.0/nxf_0.3.0_linux-x86_64.tar.gz",
            "sha256": "deadbeef",
            "signature": "untrusted comment: sig\nAAAA\ntrusted comment: t\nBBBB"
        });
        let parsed: UpdaterResponse = serde_json::from_value(body).unwrap();
        assert_eq!(parsed.version, "0.3.0");
        assert_eq!(parsed.sha256, "deadbeef");
        assert!(parsed.signature.contains("untrusted comment"));
    }
}
