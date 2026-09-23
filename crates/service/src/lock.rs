//! The single-instance lock: one nexus-flow service process per machine.
//!
//! `cfg(unix)`: a kernel advisory lock (`flock`, `LOCK_EX | LOCK_NB`) on a plain file — exclusion
//! comes from the KERNEL, not from the file's mere existence, so there is no window where two
//! processes can each convince themselves the other is dead and both "reclaim" the same lock. (An
//! earlier version used `O_EXCL` + a stale-pid probe, and that scheme admits exactly this race: two
//! processes can both read the same dead pid, both `remove_file` + recreate, and both walk away
//! believing they hold the only lock.) `flock` needs no reclaim step at all: the lock hangs off the
//! OPEN FILE DESCRIPTION, and the kernel drops it when the LAST descriptor referring to that
//! description closes — which a normal exit, a crash, and even `SIGKILL`/OOM all reach, because
//! dying closes every descriptor a process had. So a "stale" lock cannot exist; a leftover lock
//! *file* from a dead process is just an ordinary UNLOCKED file, acquirable exactly like a fresh
//! one. The pid written into it is diagnostics only (`cat` it by hand); it plays no
//! role in acquiring or releasing.
//!
//! **The holder's own `close` is not necessarily the last close, which is why [`Drop`] is explicit**
//! (nxf 6j6v.1zs2). `fork` — and `posix_spawn`, which is how `std::process::Command` starts every
//! child — copies the whole descriptor table, so a child that has been forked and has not yet
//! reached `execve` holds a second reference to this very description. `O_CLOEXEC` closes it there,
//! but not before: for those microseconds the holder can close its own descriptor and release
//! NOTHING. MEASURED on macOS on 2026-09-07, one thread spawning `/usr/bin/true` beside a loop of
//! acquire/close/acquire: 165 of 100 000 re-acquires refused with `EWOULDBLOCK`, against 0 of
//! 100 000 with nothing spawning beside it — which is what reddened `macOS tests (hosted)` on a tree
//! nobody had changed, and what a service that exits and is restarted straight away can be refused
//! by: a transient error takes an early `?` out of `serve`, launchd's `KeepAlive` starts the next
//! process immediately, and it asks for a lock the previous one only *closed*. So the [`Drop`]
//! below performs `flock(fd, LOCK_UN)`, which releases the lock on the description itself: letting
//! go depends on the holder and on nothing else in the process. The kernel's release-on-last-close
//! stays the backstop underneath it, for every path that runs no destructor.
//!
//! **Which paths those are, precisely** (review of PR #446). `Drop` covers the deaths that run
//! destructors: a normal return and every early `?` out of `serve`. It does NOT cover a signalled
//! death — and `launchctl bootout`, which `sync daemon install` runs before it bootstraps, is one:
//! launchd delivers `SIGTERM` (escalating to `SIGKILL`), this service installs no handler for
//! either, so the old process dies without unlocking anything. That reinstall sequence therefore
//! still rests on release-on-last-close, and with it on the microseconds any child of the dying
//! process needs to reach `execve`. Narrow, bounded by one `execve`, and not widened by anything
//! here — but it is the residual, not an example of what this `Drop` fixes.
//!
//! `cfg(not(unix))`: no portable kernel advisory-lock primitive is available, so this keeps the
//! PREVIOUS `O_EXCL` + stale-pid-probe scheme as a fallback. It is the same design the race above
//! was found in, but [`probe_pid`](crate::heartbeat) answers `Unknown` on non-unix and the lock
//! reads that as "still held", which means that SPECIFIC race — which requires two processes to each
//! independently PROVE the recorded owner is dead — cannot happen here: nothing on this platform can
//! ever prove that.

use std::path::Path;
#[cfg(not(unix))]
use std::path::PathBuf;

use nxs_foundation::error::{ErrorKind, NxfError, Result};

/// The message a second service start gets. One string, so both platforms' refusals read the same.
fn already_running(path: &Path) -> NxfError {
    NxfError::new(
        ErrorKind::Conflict,
        format!(
            "a nexus-flow service is already running (lock held at {}); stop it before starting \
             another, or wait for it to exit",
            path.display()
        ),
    )
}

#[cfg(unix)]
#[derive(Debug)]
pub struct ServiceLock {
    /// The descriptor the kernel's `flock` hangs off: keeping it alive for the lock's whole
    /// lifetime IS the mechanism, and [`ServiceLock`]'s own `Drop` reads it once more to hand the
    /// lock back — see this module's doc for why closing it is not enough on its own.
    file: std::fs::File,
}

#[cfg(unix)]
impl ServiceLock {
    /// Open (creating if absent) the lock file at `path` and take an exclusive, non-blocking kernel
    /// lock on it. A second `acquire` while the first is still held gets `EWOULDBLOCK` from `flock`,
    /// reported as a loud [`ErrorKind::Conflict`] naming the lock path.
    pub fn acquire(path: &Path) -> Result<ServiceLock> {
        use std::io::{Seek, Write};
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;

        create_parent(path)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            // Explicit rather than default: an existing lock file must be OPENED, not truncated,
            // here — its content (a previous run's pid) still needs to be replaced under the flock
            // below, once acquired, not raced away before we even know we hold the lock.
            .truncate(false)
            // Every OTHER file the service writes under `~/.nexusflow/` goes through
            // `atomic::write_atomic`'s temp+`O_EXCL`+rename, which never follows a pre-placed
            // symlink. The lock cannot use that idiom (it is held open for the service's whole
            // lifetime, not written once and renamed into place), so it needs its own symlink
            // defense: `O_NOFOLLOW` refuses to open the path at all when it is a symlink, rather
            // than following it and then `set_len(0)`-truncating whatever it points at.
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|e| {
                NxfError::io(format!("opening the service lock {}: {e}", path.display()))
            })?;

        // SAFETY: `flock` takes a raw fd this function owns and touches no memory.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(already_running(path));
        }

        // The lock is HELD from here on, so it belongs to the guard rather than to this function:
        // the three fallible steps below each hand it back explicitly on the way out, through
        // `ServiceLock`'s own `Drop`, instead of leaving it to a bare `File`'s close.
        let mut lock = ServiceLock { file };

        // Diagnostics only — exclusion already comes from the flock above. Truncate first so a
        // shorter pid never leaves trailing bytes from whatever the file held before.
        lock.file.set_len(0).map_err(|e| {
            NxfError::io(format!(
                "truncating the service lock {}: {e}",
                path.display()
            ))
        })?;
        lock.file.seek(std::io::SeekFrom::Start(0)).map_err(|e| {
            NxfError::io(format!("seeking the service lock {}: {e}", path.display()))
        })?;
        write!(lock.file, "{}", std::process::id()).map_err(|e| {
            NxfError::io(format!("writing the service lock {}: {e}", path.display()))
        })?;

        Ok(lock)
    }
}

#[cfg(unix)]
impl Drop for ServiceLock {
    /// Hand the lock back on the open file description, before the descriptor closes.
    ///
    /// Best-effort by construction: the only way `flock` can fail on a descriptor this struct owns
    /// is one that is already gone, and the close below it is the backstop for exactly that. There
    /// is no error to report and nobody left to report it to.
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        // SAFETY: `flock` takes a raw fd this struct owns and touches no memory.
        unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// The non-unix fallback — see this module's own doc for why this platform keeps the older,
/// imperfect `O_EXCL` + stale-pid-probe scheme rather than the `flock`-based one.
#[cfg(not(unix))]
#[derive(Debug)]
pub struct ServiceLock {
    path: PathBuf,
}

#[cfg(not(unix))]
impl ServiceLock {
    /// Acquire the lock at `path`. The common case is a single `O_EXCL` create; on `AlreadyExists`
    /// it reads the pid the file names and probes it (which answers `Unknown` on this platform — see
    /// the module doc). A DEAD owner's file is removed and the create retried exactly once; a LIVE
    /// owner (in practice: any owner, on this platform) is a loud [`ErrorKind::Conflict`].
    pub fn acquire(path: &Path) -> Result<ServiceLock> {
        create_parent(path)?;
        let pid = std::process::id().to_string();
        match crate::atomic::write_new_exclusive(path, pid.as_bytes()) {
            Ok(()) => Ok(ServiceLock {
                path: path.to_path_buf(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if Self::owner_is_alive(path) {
                    return Err(already_running(path));
                }
                let _ = std::fs::remove_file(path);
                crate::atomic::write_new_exclusive(path, pid.as_bytes()).map_err(|_| {
                    NxfError::new(
                        ErrorKind::Conflict,
                        format!(
                            "a nexus-flow service is already running (lock held at {}) — lost the \
                             race to reclaim a stale lock to another process that just started",
                            path.display()
                        ),
                    )
                })?;
                Ok(ServiceLock {
                    path: path.to_path_buf(),
                })
            }
            Err(e) => Err(NxfError::io(format!(
                "creating the service lock {}: {e}",
                path.display()
            ))),
        }
    }

    fn owner_is_alive(path: &Path) -> bool {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(_) => return true,
        };
        match raw.trim().parse::<u32>() {
            Ok(pid) => crate::heartbeat::lock_owner_is_alive(crate::heartbeat::probe_pid(pid)),
            Err(_) => true,
        }
    }
}

#[cfg(not(unix))]
impl Drop for ServiceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path); // best-effort: nothing to do if already gone
    }
}

fn create_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            NxfError::io(format!(
                "creating the service lock dir {}: {e}",
                parent.display()
            ))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn a_second_service_refuses_to_start_while_the_first_holds_the_lock() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("service.lock");
        let _held = ServiceLock::acquire(&path).expect("the first acquire succeeds");
        let err = ServiceLock::acquire(&path).expect_err("the second is refused");
        assert_eq!(err.kind.as_str(), "conflict");
        assert!(
            err.msg.contains("already running"),
            "the refusal says what is wrong: {}",
            err.msg
        );
    }

    /// The plain property, and the test that was OBSERVED FLAKING — PASS/FAIL/FAIL/PASS across four
    /// runs of one identical tree on the hosted three-core macOS runner (nxf 6j6v.1zs2). Kept
    /// exactly as it was, because nothing about it was ever wrong: it asserts something true that
    /// the code did not always deliver, and the explicit `Drop` is what makes it reliable rather
    /// than lucky. The sibling below reproduces its failure on demand instead of once in a while.
    #[test]
    fn the_lock_is_released_on_drop() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("service.lock");
        drop(ServiceLock::acquire(&path).expect("first acquire"));
        ServiceLock::acquire(&path).expect("the lock is free again once the holder is dropped");
    }

    /// **A descriptor that outlives the holder must not outlive the LOCK** (nxf 6j6v.1zs2).
    ///
    /// The module doc's measurement, made deterministic. `dup` and `fork` both add a reference to
    /// the same OPEN FILE DESCRIPTION, and a lock left to the close ends only when the LAST of
    /// them goes — so on a machine where anything spawns a child beside this test, the holder's
    /// own `close` releases nothing for as long as the child takes to reach `execve`. Spawning a
    /// real child here would reproduce that 165 times in 100 000, which is precisely the
    /// once-in-a-while red this exists to keep from coming back; `dup` reproduces it every time.
    #[cfg(unix)]
    #[test]
    fn the_lock_is_released_by_its_holder_even_when_a_second_descriptor_survives_it() {
        use std::os::unix::io::AsRawFd;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("service.lock");

        let held = ServiceLock::acquire(&path).expect("first acquire");
        // SAFETY: `dup` takes a raw fd the held lock owns and touches no caller memory.
        let survivor = unsafe { libc::dup(held.file.as_raw_fd()) };
        assert!(survivor >= 0, "dup the holder's own descriptor");
        drop(held);

        // Taken before the copy is closed, so the copy is still outstanding at the moment that
        // matters — and closed before the verdict, so a red leaves no descriptor behind.
        let reacquired = ServiceLock::acquire(&path);
        // SAFETY: `survivor` is a live fd this test owns and nothing else refers to.
        unsafe { libc::close(survivor) };
        reacquired.expect("the holder's own drop releases the lock, whoever else still has the fd");
    }

    #[test]
    fn a_leftover_lock_file_from_a_dead_process_is_simply_acquirable() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("service.lock");
        // A pid that cannot name a live process — what a crashed predecessor leaves behind.
        std::fs::write(&path, "999999999").unwrap();
        ServiceLock::acquire(&path).expect("a leftover, unlocked lock file is not a stale lock");
    }

    #[test]
    fn the_lock_records_the_holders_pid_for_a_human_reading_the_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("service.lock");
        let _held = ServiceLock::acquire(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            std::process::id().to_string()
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_lock_refuses_a_pre_placed_symlink_and_leaves_its_target_untouched() {
        let tmp = TempDir::new().unwrap();
        let victim = tmp.path().join("precious");
        std::fs::write(&victim, b"PRECIOUS").unwrap();
        let path = tmp.path().join("service.lock");
        std::os::unix::fs::symlink(&victim, &path).unwrap();

        let err =
            ServiceLock::acquire(&path).expect_err("O_NOFOLLOW refuses a symlinked lock path");
        assert_eq!(err.kind.as_str(), "io");
        assert_eq!(
            std::fs::read(&victim).unwrap(),
            b"PRECIOUS",
            "the symlink's target is never truncated through the lock"
        );
    }
}
