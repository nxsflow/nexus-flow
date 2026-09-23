//! The one atomic file writer this house uses for shared-home state.
//!
//! Every file the service owns lives in a directory several processes write concurrently and any
//! user process can create a path in. So each write goes to a fresh sibling temp opened `O_EXCL`
//! and is then renamed over the target: a same-directory rename is atomic, so a concurrent reader
//! sees the whole old file or the whole new one and never a torn one, and a symlink pre-placed on
//! the predictable temp path is refused rather than followed.
//!
//! It arrived with the workspace registry (0jq8 Integrity #1) and had grown four consumers by the
//! time it moved here: the registry, `~/.nexusflow/config.toml`, the heartbeat, and `.nxs/sync.toml`
//! — plus the launchd plist, which was fixed to use it after a review found it on plain
//! `fs::write`. It is one policy, in one place, exactly so those cannot drift apart.

use std::path::Path;

use nxs_foundation::error::{NxfError, Result};

/// Write `contents` to `path` atomically: create the parent, write a sibling temp with
/// [`write_new_exclusive`], rename over the target. The temp is removed on any failure so a
/// half-written sibling is never stranded next to a good file.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    write_atomic_as(path, contents, false)
}

/// [`write_atomic`] for a file that can hold a CREDENTIAL — a relay URL carries its key as userinfo,
/// and it is stored in `.nxs/sync.toml` and `~/.nexusflow/config.toml` — created readable by its
/// owner alone (0600) instead of at the process umask (review of PR #487, Integrity #2). The mode
/// is set on the temp at creation, so there is no moment the file exists wider; the rename carries
/// it over whatever mode the old file had. On a platform without Unix modes it is [`write_atomic`].
pub fn write_atomic_private(path: &Path, contents: &[u8]) -> Result<()> {
    write_atomic_as(path, contents, true)
}

fn write_atomic_as(path: &Path, contents: &[u8], private: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| NxfError::io(format!("creating {}: {e}", parent.display())))?;
    }
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("nxs-service.tmp");
    let tmp = dir.join(format!(".{name}.nxs.tmp.{}", std::process::id()));
    create_exclusive(&tmp, contents, private)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp); // best-effort: never strand the temp sibling
            NxfError::io(format!("writing {}: {e}", path.display()))
        })
}

/// [`write_atomic`] for a writer that may run CONCURRENTLY WITH ITSELF inside one process — an
/// embedding host renaming its machine from two threads (nxf 6j6v.f0b5, review of PR #485,
/// Integrity #5). `write_atomic`'s temp is named by the pid alone, so two such calls share it: the
/// second `O_EXCL` fails, and the loser's cleanup deletes the winner's temp. Here a process-wide
/// counter gives every call a temp of its own; everything else — exclusive create, rename over the
/// target, cleanup on failure — is the same policy.
///
/// Not the default for the others on purpose: their writers are single per process (the registry
/// holds a lock across read and write), and their squatted-temp tests pin the predictable name.
pub fn write_atomic_unique(path: &Path, contents: &[u8]) -> Result<()> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| NxfError::io(format!("creating {}: {e}", parent.display())))?;
    }
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("nxs-service.tmp");
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.nxs.tmp.{}.{seq}", std::process::id()));
    write_new_exclusive(&tmp, contents)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp); // best-effort: never strand the temp sibling
            NxfError::io(format!("writing {}: {e}", path.display()))
        })
}

/// Write `contents` to a BRAND-NEW file at `path`, failing if anything already exists there
/// (`O_EXCL` via `create_new`).
///
/// This is the half that closes the TOCTOU redirect: the temp path
/// (`.{name}.nxs.tmp.{pid}`) is predictable, and a plain `fs::write` would happily follow a symlink
/// somebody had put there, writing outside the intended directory. Exposed on its own because two
/// callers need the primitive rather than the temp+rename pair around it.
pub fn write_new_exclusive(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    create_exclusive(path, contents, false)
}

fn create_exclusive(path: &Path, contents: &[u8], private: bool) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    options.open(path)?.write_all(contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A file that may hold a relay's key is its owner's alone, from the moment it exists — and
    /// replacing a wider file narrows it.
    #[cfg(unix)]
    #[test]
    fn a_private_write_is_readable_by_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sync.toml");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_atomic_private(&path, b"endpoint = \"https://u:p@relay\"").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "endpoint = \"https://u:p@relay\""
        );
    }

    #[test]
    fn write_new_exclusive_writes_a_fresh_file() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("fresh");
        write_new_exclusive(&p, b"hello").expect("a fresh path is written");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "hello");
    }

    #[test]
    fn write_new_exclusive_refuses_an_existing_target_and_leaves_it_untouched() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("taken");
        std::fs::write(&p, b"original").unwrap();
        let err = write_new_exclusive(&p, b"replacement").expect_err("an existing path is refused");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "original");
    }

    #[test]
    fn write_atomic_creates_the_parent_directory_it_needs() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("not").join("yet").join("there.toml");
        write_atomic(&p, b"x = 1").expect("the parent chain is created");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "x = 1");
    }

    #[test]
    fn write_atomic_replaces_an_existing_file_whole() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("state.json");
        write_atomic(&p, b"first").unwrap();
        write_atomic(&p, b"second").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "second");
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_refuses_a_pre_placed_temp_symlink_and_leaves_its_target_untouched() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("victim");
        std::fs::write(&target, b"do not touch").unwrap();
        let p = dir.path().join("state.json");
        let tmp = dir
            .path()
            .join(format!(".state.json.nxs.tmp.{}", std::process::id()));
        std::os::unix::fs::symlink(&target, &tmp).unwrap();

        let err = write_atomic(&p, b"payload").expect_err("a squatted temp path is refused");
        assert!(
            err.msg.contains("state.json"),
            "the refusal names the file it was writing: {}",
            err.msg
        );
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "do not touch",
            "the symlink's target must never be written through"
        );
        assert!(!p.exists(), "nothing was installed at the target path");
    }

    #[test]
    fn write_atomic_leaves_no_temp_sibling_behind() {
        let dir = TempDir::new().unwrap();
        write_atomic(&dir.path().join("state.json"), b"x").unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "stranded temps: {leftovers:?}");
    }
}
