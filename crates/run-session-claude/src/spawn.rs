//! The spawn itself, and the files a session is given.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nexus_run_contracts::StartSpec;

use crate::ledger::{ChildRecord, SessionLedger, SpawnRecord};
use crate::procs;

/// Which spec a spawn was made from, for a person comparing two attempts: the text is in `spec.json`.
fn spec_hash(spec: &StartSpec) -> String {
    let mut hasher = DefaultHasher::new();
    serde_json::to_string(spec)
        .unwrap_or_default()
        .hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// `spawn.json`, BEFORE the spawn. Whoever creates it is the one who may spawn.
pub(crate) fn claim(
    ledger: &SessionLedger,
    spec: &StartSpec,
    now_ms: i64,
) -> std::io::Result<bool> {
    ledger.claim_spawn(&SpawnRecord {
        spec_hash: spec_hash(spec),
        at_ms: now_ms,
    })
}

/// A program on the session's own `PATH`: the service's may be launchd's, which knows neither `node` nor `claude`.
fn on_path(spec: &StartSpec, program: &str) -> Option<PathBuf> {
    let path = spec.env.iter().find(|var| var.name == "PATH")?;
    std::env::split_paths(&path.value)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// No execute bit to read: a file is all a platform without one can say. Never reached by a start, which refuses
/// there first (`SPAWNS_HERE`); it exists so the adapter compiles everywhere.
#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(unix)]
fn own_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn own_process_group(_command: &mut Command) {}

/// What the sidecar reads: the start spec as it is, and the `claude` this adapter found for it, because the shipped
/// bundle has no `node_modules` for the SDK to find its own.
fn spec_for_the_sidecar(spec: &StartSpec) -> std::io::Result<serde_json::Value> {
    let mut value = serde_json::to_value(spec).map_err(std::io::Error::other)?;
    if let Some(claude) = on_path(spec, "claude") {
        value["claude_path"] = serde_json::Value::String(claude.to_string_lossy().into_owned());
    }
    Ok(value)
}

/// In a process group of its own, so a service restart neither kills it nor orphans it unseen. No stdin and no pipe: the
/// session owes this process nothing, and outlives it. With nothing but the spec's environment.
pub(crate) fn spawn(
    ledger: &SessionLedger,
    spec: &StartSpec,
    bundle: &Path,
) -> std::io::Result<()> {
    let spawned = try_spawn(ledger, spec, bundle);
    if spawned.is_err() {
        // It never ran, so the next attempt may claim the key. A spawn failure is a slot with no `started` (§13).
        ledger.release_spawn()?;
    }
    spawned
}

fn try_spawn(ledger: &SessionLedger, spec: &StartSpec, bundle: &Path) -> std::io::Result<()> {
    ledger.write_spec(&spec_for_the_sidecar(spec)?)?;
    let node = on_path(spec, "node").ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no `node` on the session's PATH",
        )
    })?;
    let log = ledger.sidecar_log()?;
    let mut command = Command::new(node);
    command
        .arg(bundle)
        .arg("--facts")
        .arg("--spec")
        .arg(ledger.spec_path())
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(spec.env.iter().map(|var| (&var.name, &var.value)))
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    own_process_group(&mut command);
    let child = command.spawn()?;
    let pid = child.id();
    if let Some(info) = procs::info(pid) {
        ledger.write_child_unless_there(&ChildRecord {
            pid,
            start_time: procs::start_text(info.start_ms),
            handlers: false,
        })?;
    }
    // Dropped unwaited for: it is reaped by `procs::reap`, by identity, whichever process of the service gets there.
    drop(child);
    Ok(())
}

/// One nudge: an ask file the sidecar drains at the turn's yield, named so they sort in the order they were written.
pub(crate) fn write_ask(ledger: &SessionLedger, text: &str, now_ms: i64) -> std::io::Result<()> {
    let path = ledger
        .dir()
        .join(format!("ask.{now_ms:016}.{}.json", std::process::id()));
    let temp = path.with_extension("tmp");
    let body = serde_json::json!({ "text": text });
    std::fs::write(
        &temp,
        serde_json::to_vec(&body).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&temp, &path)
}
