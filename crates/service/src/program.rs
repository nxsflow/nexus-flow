//! **What binary the background service actually runs** (nxf 6j6v.dcpk).
//!
//! The launchd job does not name a binary. It names an ALIAS —
//! [`ServiceHome::program`], `~/.nexusflow/bin/nexus-flow` — which
//! [`crate::launchd::link_program`] re-points at `current_exe()` on every install. That indirection
//! is deliberate and good: it puts a NAME in the user's background items and it survives a move of
//! the installation directory.
//!
//! What it also does is hide two facts that were, until this module, unaskable:
//!
//! 1. **Which binary is that?** Two installations do not collide — the last one silently wins — and
//!    nothing recorded which. Measured on the owner's machine on 2026-08-29: the alias had pointed
//!    at a `target/debug/nxs` since the 25th, three `nxs self-update` runs on one day had not
//!    touched it, and the only clock on the machine was a development build nobody had chosen.
//! 2. **Is it still there?** [`link_program`](crate::launchd::link_program)'s own doc names the
//!    hazard verbatim: *"a stale link points at a binary that is gone, and launchd's only report of
//!    that is a service that never starts."* A `cargo clean` or a disk sweep is enough, and the
//!    machine then keeps no time and syncs nothing, with no message anywhere.
//!
//! [`ProgramState`] is the answer to both, as one reading, so that every place that needs it —
//! the daemon's own heartbeat, `nxs sync daemon status`, `nxs self-update`, and the line every
//! `nxs` invocation prints when the alias is dead — asks the same question of the same file rather
//! than each spelling its own.

use std::path::{Path, PathBuf};

use crate::home::ServiceHome;

/// What the program alias points at, and whether that file is there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramState {
    /// Nothing at that path: the service has never been installed here, or the alias was removed.
    Absent,
    /// The alias resolves to a file that exists — the RESOLVED target, never the alias itself,
    /// because "which binary does the service run" is a question about the target.
    Present(PathBuf),
    /// The alias is there and what it names is not. The one state launchd cannot report: it will
    /// keep trying to `exec` this path and the only symptom is a service that never starts.
    Dangling(PathBuf),
}

impl ProgramState {
    /// The resolved target, whether or not it exists — `None` only when there is no alias at all.
    pub fn target(&self) -> Option<&Path> {
        match self {
            ProgramState::Absent => None,
            ProgramState::Present(p) | ProgramState::Dangling(p) => Some(p),
        }
    }

    /// The dead target, when that is what this is.
    pub fn dangling(&self) -> Option<&Path> {
        match self {
            ProgramState::Dangling(p) => Some(p),
            _ => None,
        }
    }
}

/// Read the state of the alias at `link`.
///
/// `symlink_metadata`, never `exists`: `exists` FOLLOWS the link and answers `false` for a dangling
/// one, which is the single state this whole module is about — it would report the hazard as "never
/// installed". Same reason [`link_program`](crate::launchd::link_program) uses it to decide whether
/// to replace.
///
/// A link is not required to be a symlink: an installation that put a real binary there is
/// `Present` with the path itself as its own target. A RELATIVE symlink target is resolved against
/// the link's own directory — this repo always writes an absolute one, but a hand-made link is a
/// path a person can take, and resolving it against the process's working directory would answer a
/// different question on every call.
pub fn state_of(link: &Path) -> ProgramState {
    let Ok(meta) = std::fs::symlink_metadata(link) else {
        return ProgramState::Absent;
    };
    let target = if meta.file_type().is_symlink() {
        let Ok(raw) = std::fs::read_link(link) else {
            return ProgramState::Absent;
        };
        match (raw.is_absolute(), link.parent()) {
            (false, Some(dir)) => dir.join(raw),
            _ => raw,
        }
    } else {
        link.to_path_buf()
    };
    if target.exists() {
        ProgramState::Present(target)
    } else {
        ProgramState::Dangling(target)
    }
}

impl ServiceHome {
    /// What [`ServiceHome::program`] points at right now.
    pub fn program_state(&self) -> ProgramState {
        state_of(&self.program())
    }
}

/// The binary THIS process is running, resolved through every symlink on the way.
///
/// The resolution is the whole point. Under launchd the service is `exec`ed through the
/// `nexus-flow` alias, and on macOS `current_exe()` hands that alias path straight back — so a
/// service recording `current_exe()` verbatim would record the alias it already knew and answer
/// nothing. `canonicalize` is what turns it into the build that is actually running.
///
/// Falls back to the unresolved path rather than to `None` when canonicalisation fails (a deleted
/// binary a running process still holds open is exactly such a case, and it is the one worth
/// reporting): a path that is merely unresolved is far more use than no answer at all.
pub fn running_program() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(std::fs::canonicalize(&exe).unwrap_or(exe))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn home(tmp: &TempDir) -> ServiceHome {
        ServiceHome::at(tmp.path().join(".nexusflow"))
    }

    #[test]
    fn the_running_program_is_resolved_through_the_link_it_was_started_as() {
        // The test binary is not started through a link, so the weaker property this can hold
        // anywhere is that the answer exists and is itself fully resolved (canonical) — which is
        // exactly what the launchd case needs and what a bare `current_exe()` would not give.
        let running = running_program().expect("a test binary can name itself");
        assert!(running.is_absolute(), "{}", running.display());
        assert_eq!(
            std::fs::canonicalize(&running).unwrap(),
            running,
            "already resolved, so recording it records a build and not an alias"
        );
    }

    #[test]
    fn no_alias_at_all_is_absent_and_names_no_target() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(home(&tmp).program_state(), ProgramState::Absent);
        assert_eq!(ProgramState::Absent.target(), None);
    }

    #[test]
    #[cfg(unix)]
    fn an_alias_pointing_at_a_real_binary_resolves_to_that_binary_not_to_itself() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        let binary = tmp.path().join("nxs");
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();
        std::fs::create_dir_all(home.bin()).unwrap();
        std::os::unix::fs::symlink(&binary, home.program()).unwrap();
        assert_eq!(home.program_state(), ProgramState::Present(binary));
    }

    #[test]
    #[cfg(unix)]
    fn an_alias_whose_target_is_gone_is_dangling_and_not_absent() {
        // The whole point of the module: `exists()` follows the link and would call this `Absent`,
        // reporting a machine with a dead clock as one that never installed a service.
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        let binary = tmp.path().join("target").join("debug").join("nxs");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();
        std::fs::create_dir_all(home.bin()).unwrap();
        std::os::unix::fs::symlink(&binary, home.program()).unwrap();
        assert_eq!(home.program_state(), ProgramState::Present(binary.clone()));

        std::fs::remove_file(&binary).unwrap();
        assert!(!home.program().exists(), "`exists` follows the link");
        assert_eq!(home.program_state(), ProgramState::Dangling(binary.clone()));
        assert_eq!(
            home.program_state().dangling(),
            Some(binary.as_path()),
            "and the dead target is named, because a message about it has to say what is missing"
        );
    }

    #[test]
    fn a_real_file_in_the_aliass_place_is_its_own_target() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        std::fs::create_dir_all(home.bin()).unwrap();
        std::fs::write(home.program(), b"#!/bin/sh\n").unwrap();
        assert_eq!(home.program_state(), ProgramState::Present(home.program()));
    }

    #[test]
    #[cfg(unix)]
    fn a_relative_link_resolves_against_the_links_own_directory_not_the_working_one() {
        let tmp = TempDir::new().unwrap();
        let home = home(&tmp);
        std::fs::create_dir_all(home.bin()).unwrap();
        std::fs::write(home.bin().join("real"), b"#!/bin/sh\n").unwrap();
        std::os::unix::fs::symlink("real", home.program()).unwrap();
        assert_eq!(
            home.program_state(),
            ProgramState::Present(home.bin().join("real")),
            "resolved against the alias's directory, so the answer does not move with the cwd"
        );
    }
}
