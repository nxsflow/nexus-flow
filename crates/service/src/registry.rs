//! The workspace registry: `~/.nexusflow/workspaces.toml`, the list the service sweeps.
//!
//! It arrived as an MCP convenience (E9 0jq8) — a host reads it to offer workspaces by NAME rather
//! than by path — and the sync daemon then made it load-bearing: the service attends exactly the
//! workspaces listed here, so an unlisted workspace is one the service does not know exists.
//!
//! It stays plain, hand-editable TOML with a deterministic order, and an absent file is an EMPTY
//! registry rather than an error — a machine where nothing has ever been registered is a state, not
//! a fault. A present-but-malformed file is loud: guessing at the contents of a file the user is
//! invited to edit would be the worse failure.
//!
//! The core here takes an explicit path so it is provable against a `TempDir`;
//! [`crate::ServiceHome`] supplies the real one.

use std::path::Path;

use nxs_foundation::error::{NxfError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::atomic::write_atomic;

/// One registered workspace: a human-facing `name` (defaults to the path's final component) and the
/// absolute `path`. This is the canonical record shape — serialized identically for the
/// `list_workspaces` MCP tool and `nxs mcp workspaces --json`, so the two seams stay byte-identical.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    /// Human label a host shows / selects by.
    pub name: String,
    /// Absolute workspace root (the directory holding `.nxs/`).
    pub path: String,
}

/// The on-disk TOML shape: an array-of-tables under the `workspace` key
/// (`[[workspace]] name=.. path=..`).
#[derive(Default, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    workspace: Vec<WorkspaceEntry>,
}

/// Deterministic order for BOTH seams and a hand-diff: by `name`, then by `path` to break ties (two
/// checkouts of the same repo share a final component).
fn sort_entries(entries: &mut [WorkspaceEntry]) {
    entries.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
}

fn to_array(entries: &[WorkspaceEntry]) -> Value {
    Value::Array(
        entries
            .iter()
            .map(|e| serde_json::json!({ "name": e.name, "path": e.path }))
            .collect(),
    )
}

/// The human label derived from a workspace path: its final component, or the whole path when it
/// has none (e.g. `/`).
pub fn name_for(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string())
}

/// The key a workspace is registered under: its path with any trailing separators removed.
///
/// The registry is keyed by exact string, and `/p` and `/p/` are the same directory — without this,
/// an app registering the trailing-slash form and then deregistering the bare one would leave the
/// entry behind, which is precisely the accumulation this seam exists to stop. A path that is
/// nothing but separators (`/`) keeps its single one rather than normalizing to the empty string.
pub fn key_for(path: &str) -> String {
    let trimmed = path.trim_end_matches(std::path::is_separator);
    if trimmed.is_empty() {
        path.chars().take(1).collect()
    } else {
        trimmed.to_string()
    }
}

/// Load the registry at `path` as a DETERMINISTIC list — sorted by `(name, path)`. A genuinely
/// absent (or empty) file is an empty registry, not an error; a present-but-malformed file is a
/// loud [`NxfError::validation`] naming it (never a silent empty list that would hide a typo).
pub fn load_from(path: &Path) -> Result<Vec<WorkspaceEntry>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(NxfError::io(format!(
                "reading the workspace registry {}: {e}",
                path.display()
            )))
        }
    };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let file: RegistryFile = toml::from_str(&raw).map_err(|e| {
        NxfError::validation(format!(
            "the workspace registry {} is not valid TOML — refusing to guess its contents: {e}",
            path.display()
        ))
    })?;
    let mut entries = file.workspace;
    sort_entries(&mut entries);
    Ok(entries)
}

/// The canonical `list_workspaces` / `nxs mcp workspaces --json` value: the deterministic registry
/// as a JSON ARRAY of `{name, path}`.
pub fn list_value_from(path: &Path) -> Result<Value> {
    Ok(to_array(&load_from(path)?))
}

/// Idempotently upsert `entry` into the registry at `path`, keyed by `path` (a workspace IS its
/// directory): a new path is appended; an existing path's `name` is refreshed; an already-identical
/// entry writes nothing. Returns whether anything changed.
pub fn upsert_into(path: &Path, entry: &WorkspaceEntry) -> Result<bool> {
    // Held across the read AND the write — see [`lock_registry`] for the entry that goes missing
    // without it, and why a failure to take it is not a failure to register.
    let _lock = lock_registry(path);
    let mut entries = load_from(path)?;
    match entries.iter_mut().find(|e| e.path == entry.path) {
        // Same path, same name → already registered; write nothing (idempotent).
        Some(existing) if existing.name == entry.name => return Ok(false),
        // Same path, new name → refresh the label (the path is the key).
        Some(existing) => existing.name = entry.name.clone(),
        // New path → append.
        None => entries.push(entry.clone()),
    }
    write_registry(path, entries)?;
    Ok(true)
}

/// Remove the entry for `workspace_path` from the registry at `path`. Returns whether anything was
/// removed — removing an entry that is not there is a successful no-op, the same shape as
/// [`upsert_into`]'s idempotent arm, so a caller that deregisters twice (or deregisters a workspace
/// it never registered) does not have to special-case it.
///
/// The counterpart nothing had (6j6v.5zst). Without it, the only way to prune the registry was to
/// hand-edit the TOML — and the file was measured at 98 stale entries on one developer machine
/// because every write path added and none removed.
pub fn remove_from(path: &Path, workspace_path: &str) -> Result<bool> {
    let _lock = lock_registry(path);
    let key = key_for(workspace_path);
    let mut entries = load_from(path)?;
    let before = entries.len();
    entries.retain(|e| key_for(&e.path) != key);
    if entries.len() == before {
        return Ok(false);
    }
    write_registry(path, entries)?;
    Ok(true)
}

/// The exclusive lock a registry write is held under, released when it is dropped.
///
/// `cfg(unix)` only, and [`lock_registry`]'s doc says what the other platforms get instead.
#[cfg(unix)]
#[derive(Debug)]
pub(crate) struct RegistryLock {
    /// The descriptor the kernel's `flock` hangs off — keeping it alive IS the mechanism, exactly
    /// as in [`crate::ServiceLock`], and so is the `Drop` below that hands the lock back rather
    /// than leaving it to the close ([`crate::lock`]'s module doc measures why, nxf 6j6v.1zs2).
    file: std::fs::File,
}

#[cfg(unix)]
impl Drop for RegistryLock {
    /// Hand the lock back on the open file description, before the descriptor closes. Best-effort
    /// for the same reason [`crate::ServiceLock`]'s is: a descriptor this struct owns can only fail
    /// this call by being gone already, which the close underneath it covers.
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        // SAFETY: `flock` takes a raw fd this struct owns and touches no memory.
        unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// How many times to ask for the registry lock before giving up and writing without it.
///
/// **Giving up is deliberate.** A blocking `flock` has no timeout, and this call sits inside
/// `nxf init` — a verb a person waits on. Every holder keeps it for one small read-modify-write, so
/// half a second is orders of magnitude more than any honest contention needs; anything longer is
/// something pathological, and hanging a user's `init` on it would be a worse failure than the
/// race this exists to narrow. Proceeding unlocked is exactly what every caller did before this,
/// so the fallback can only be as bad as the previous behaviour, never worse.
#[cfg(unix)]
const LOCK_ATTEMPTS: u32 = 50;

/// Take the registry's exclusive lock for the duration of one read-modify-write (nxf 6j6v.y12q,
/// review of PR #421, Integrity #2).
///
/// **What it fixes.** [`upsert_into`] and [`remove_from`] read the whole registry, change it, and
/// write it back. Each WRITE is atomic (temp + rename, so the file is never half-parsed), but the
/// read-modify-write around it was not: two registrants could read the same list, each compute its
/// own successor, and the second rename silently drop the first's entry — with both callers told
/// they succeeded. Since y12q every `nxf init`/`nxm init` is a registry writer, including the
/// `--json` path a fleet of agents runs in parallel, so the window is entered far more often than
/// when `nxs sync bind` was the only way in.
///
/// `None` means no lock was taken and the caller proceeds anyway — an unlockable lock file must
/// never turn a registration into a failure. `cfg(not(unix))` has no portable advisory lock and so
/// always answers `None`, which is the pre-existing behaviour on that platform, unchanged.
#[cfg(unix)]
pub(crate) fn lock_registry(path: &Path) -> Option<RegistryLock> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;

    let lock_path = path.with_extension("toml.lock");
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        // The same symlink defense, and the same reason, as `ServiceLock::acquire`: this path is
        // held open rather than written through `write_atomic`'s temp+rename, so it needs
        // `O_NOFOLLOW` of its own rather than inheriting that idiom's protection.
        .custom_flags(libc::O_NOFOLLOW)
        .open(&lock_path)
        .ok()?;

    for _ in 0..LOCK_ATTEMPTS {
        // SAFETY: `flock` takes a raw fd this function owns and touches no caller memory.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Some(RegistryLock { file });
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

#[cfg(not(unix))]
pub(crate) fn lock_registry(_path: &Path) -> Option<()> {
    None
}

/// Render + atomically write the whole registry, re-sorted so the file stays stable across writes.
fn write_registry(path: &Path, mut entries: Vec<WorkspaceEntry>) -> Result<()> {
    sort_entries(&mut entries);
    let rendered = toml::to_string(&RegistryFile { workspace: entries })
        .map_err(|e| NxfError::io(format!("serializing the workspace registry: {e}")))?;
    write_atomic(path, rendered.as_bytes())
}

/// Absolutize `path` against `cwd` without resolving symlinks, then normalize its key form.
///
/// `canonicalize` is deliberately NOT used: registration is lazy (a workspace may not exist yet, or
/// may live on another machine), and resolving symlinks would also rewrite the very string the user
/// sees in the file and hands back through a per-call `workspace` override.
pub fn absolutize(path: &Path, cwd: &Path) -> String {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    key_for(&joined.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn reg(dir: &TempDir) -> PathBuf {
        dir.path().join(".nexusflow").join("workspaces.toml")
    }

    /// One registry entry, for the concurrency tests below.
    fn entry(name: &str, path: &str) -> WorkspaceEntry {
        WorkspaceEntry {
            name: name.to_string(),
            path: path.to_string(),
        }
    }

    #[test]
    fn load_from_missing_file_is_an_empty_registry() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(load_from(&reg(&tmp)).unwrap(), Vec::<WorkspaceEntry>::new());
    }

    #[test]
    fn load_from_returns_entries_sorted_by_name_then_path() {
        let tmp = TempDir::new().unwrap();
        let path = reg(&tmp);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "[[workspace]]\nname = \"beta\"\npath = \"/b\"\n\
             [[workspace]]\nname = \"alpha\"\npath = \"/a2\"\n\
             [[workspace]]\nname = \"alpha\"\npath = \"/a1\"\n",
        )
        .unwrap();
        let got = load_from(&path).unwrap();
        assert_eq!(
            got.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
            vec!["/a1", "/a2", "/b"]
        );
    }

    #[test]
    fn load_from_malformed_toml_is_a_loud_validation_error_naming_the_file() {
        let tmp = TempDir::new().unwrap();
        let path = reg(&tmp);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "this is not = = valid toml [[[").unwrap();
        let err = load_from(&path).unwrap_err();
        assert_eq!(err.kind.as_str(), "validation");
        assert!(
            err.msg.contains("workspaces.toml"),
            "the error names the file: {}",
            err.msg
        );
    }

    #[test]
    fn list_value_is_a_deterministic_json_array_of_name_path() {
        let tmp = TempDir::new().unwrap();
        let path = reg(&tmp);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "[[workspace]]\nname = \"work\"\npath = \"/w\"\n\
             [[workspace]]\nname = \"home\"\npath = \"/h\"\n",
        )
        .unwrap();
        assert_eq!(
            list_value_from(&path).unwrap(),
            json!([
                { "name": "home", "path": "/h" },
                { "name": "work", "path": "/w" },
            ])
        );
    }

    #[test]
    fn list_value_of_an_empty_registry_is_an_empty_array() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(list_value_from(&reg(&tmp)).unwrap(), json!([]));
    }

    #[test]
    fn upsert_appends_a_new_workspace_and_reports_changed() {
        let tmp = TempDir::new().unwrap();
        let path = reg(&tmp);
        let entry = WorkspaceEntry {
            name: "proj".into(),
            path: "/proj/x".into(),
        };
        assert!(
            upsert_into(&path, &entry).unwrap(),
            "a new path is a change"
        );
        assert_eq!(load_from(&path).unwrap(), vec![entry]);
    }

    #[test]
    fn upsert_of_an_identical_entry_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        let path = reg(&tmp);
        let entry = WorkspaceEntry {
            name: "proj".into(),
            path: "/proj/x".into(),
        };
        assert!(upsert_into(&path, &entry).unwrap());
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !upsert_into(&path, &entry).unwrap(),
            "re-registering the same entry is idempotent"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "no rewrite on a no-op"
        );
    }

    #[test]
    fn upsert_refreshes_the_name_for_an_existing_path() {
        let tmp = TempDir::new().unwrap();
        let path = reg(&tmp);
        upsert_into(
            &path,
            &WorkspaceEntry {
                name: "old".into(),
                path: "/p".into(),
            },
        )
        .unwrap();
        assert!(upsert_into(
            &path,
            &WorkspaceEntry {
                name: "new".into(),
                path: "/p".into()
            }
        )
        .unwrap());
        assert_eq!(
            load_from(&path).unwrap(),
            vec![WorkspaceEntry {
                name: "new".into(),
                path: "/p".into()
            }],
            "one entry, name refreshed — the path is the key"
        );
    }

    #[test]
    fn upsert_preserves_other_registered_workspaces() {
        let tmp = TempDir::new().unwrap();
        let path = reg(&tmp);
        for (name, p) in [("a", "/a"), ("b", "/b")] {
            upsert_into(
                &path,
                &WorkspaceEntry {
                    name: name.into(),
                    path: p.into(),
                },
            )
            .unwrap();
        }
        assert_eq!(load_from(&path).unwrap().len(), 2);
    }

    // ---- remove_from: the half that was missing (6j6v.5zst) --------------------------------------

    #[test]
    fn remove_takes_the_named_workspace_out_and_leaves_its_siblings() {
        let tmp = TempDir::new().unwrap();
        let path = reg(&tmp);
        for (name, p) in [("a", "/a"), ("b", "/b"), ("c", "/c")] {
            upsert_into(
                &path,
                &WorkspaceEntry {
                    name: name.into(),
                    path: p.into(),
                },
            )
            .unwrap();
        }
        assert!(remove_from(&path, "/b").unwrap(), "an entry was removed");
        assert_eq!(
            load_from(&path)
                .unwrap()
                .iter()
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>(),
            vec!["/a", "/c"]
        );
    }

    #[test]
    fn removing_a_workspace_that_was_never_registered_is_a_successful_no_op() {
        let tmp = TempDir::new().unwrap();
        let path = reg(&tmp);
        upsert_into(
            &path,
            &WorkspaceEntry {
                name: "a".into(),
                path: "/a".into(),
            },
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(
            !remove_from(&path, "/never").unwrap(),
            "nothing was removed, and that is not an error"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "an absent entry must not rewrite the file"
        );
    }

    #[test]
    fn removing_from_a_registry_that_does_not_exist_yet_is_a_successful_no_op() {
        let tmp = TempDir::new().unwrap();
        let path = reg(&tmp);
        assert!(!remove_from(&path, "/a").unwrap());
        assert!(
            !path.exists(),
            "a deregister must not CREATE the registry it found nothing in"
        );
    }

    #[test]
    fn a_trailing_separator_names_the_same_workspace_as_the_bare_path() {
        let tmp = TempDir::new().unwrap();
        let path = reg(&tmp);
        upsert_into(
            &path,
            &WorkspaceEntry {
                name: "p".into(),
                path: "/proj/p".into(),
            },
        )
        .unwrap();
        assert!(
            remove_from(&path, "/proj/p/").unwrap(),
            "`/proj/p/` and `/proj/p` are one directory, so one entry"
        );
        assert!(load_from(&path).unwrap().is_empty());
    }

    // ---- the key/name derivations ----------------------------------------------------------------

    #[test]
    fn name_for_is_the_final_path_component() {
        assert_eq!(name_for("/Users/me/dev/nexus-flow"), "nexus-flow");
        assert_eq!(name_for("/proj/x/"), "x");
    }

    #[test]
    fn key_for_strips_trailing_separators_but_never_empties_the_root() {
        assert_eq!(key_for("/proj/x/"), "/proj/x");
        assert_eq!(key_for("/proj/x"), "/proj/x");
        assert_eq!(
            key_for("/"),
            "/",
            "the root is a directory, not an empty key"
        );
    }

    #[test]
    fn absolutize_joins_a_relative_path_onto_the_working_directory() {
        assert_eq!(
            absolutize(Path::new("proj"), Path::new("/home/me")),
            "/home/me/proj"
        );
        assert_eq!(
            absolutize(Path::new("/abs/proj/"), Path::new("/home/me")),
            "/abs/proj",
            "an absolute path is kept, only its trailing separator normalized"
        );
    }

    // ---- concurrent registrants (nxf 6j6v.y12q, review of PR #421, Integrity #2) --------------

    #[cfg(unix)]
    #[test]
    fn a_second_registrant_waits_for_the_first_to_finish_its_read_modify_write() {
        // The race, made deterministic rather than left to luck. `upsert_into` reads the whole
        // registry, appends, and writes it back; two of them interleaved lose one entry, and both
        // callers are told they succeeded. Sixteen threads racing would only PROBABLY catch that.
        // Holding the lock and watching a registrant fail to finish cannot pass by accident.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("workspaces.toml");
        upsert_into(&path, &entry("first", "/proj/first")).unwrap();

        let held = lock_registry(&path).expect("a unix build locks the registry");
        let racer = {
            let path = path.clone();
            std::thread::spawn(move || upsert_into(&path, &entry("second", "/proj/second")))
        };
        std::thread::sleep(std::time::Duration::from_millis(250));
        let blocked = !racer.is_finished();
        drop(held);
        racer.join().unwrap().unwrap();

        assert!(
            blocked,
            "a registrant must not read-modify-write while another holds the registry"
        );
        assert_eq!(load_from(&path).unwrap().len(), 2, "and then it lands");
    }

    /// **A descriptor that outlives the holder must not outlive the LOCK** — the same property
    /// [`crate::lock`]'s module doc measures, at the registry's own lock (nxf 6j6v.1zs2).
    ///
    /// `dup` stands in for what every `fork` in the process briefly creates: a second reference to
    /// the same OPEN FILE DESCRIPTION. A lock left to the close ends only when the LAST of them
    /// goes — and here the cost of that is quiet rather than loud, because [`lock_registry`] does
    /// not fail when it cannot lock. It spends its fifty attempts, answers `None`, and the caller
    /// writes UNLOCKED: the mutual exclusion this lock exists for, gone without a word.
    #[cfg(unix)]
    #[test]
    fn a_descriptor_that_outlives_the_holder_does_not_leave_the_registry_locked() {
        use std::os::unix::io::AsRawFd;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("workspaces.toml");

        let held = lock_registry(&path).expect("a unix build locks the registry");
        // SAFETY: `dup` takes a raw fd the held lock owns and touches no caller memory.
        let survivor = unsafe { libc::dup(held.file.as_raw_fd()) };
        assert!(survivor >= 0, "dup the holder's own descriptor");
        drop(held);

        let next = lock_registry(&path);
        // SAFETY: `survivor` is a live fd this test owns and nothing else refers to.
        unsafe { libc::close(survivor) };
        assert!(
            next.is_some(),
            "the holder's own drop hands the registry lock back; a surviving copy of its \
             descriptor must not push the next registrant into writing unlocked"
        );
    }

    #[test]
    fn every_one_of_many_concurrent_registrations_survives() {
        // The behaviour the lock exists for, at the seam a caller sees: sixteen workspaces
        // registering at once — the shape an agent fleet produces since y12q made every `init` a
        // registry writer — and none of them silently clobbered by another's write.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("workspaces.toml");
        let threads: Vec<_> = (0..16)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    upsert_into(&path, &entry(&format!("w{i}"), &format!("/proj/w{i}"))).unwrap()
                })
            })
            .collect();
        for t in threads {
            t.join().expect("no registrant panicked");
        }
        assert_eq!(
            load_from(&path).unwrap().len(),
            16,
            "every concurrent registration must survive; a lost one is a workspace with no clock"
        );
    }
}
