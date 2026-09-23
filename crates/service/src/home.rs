//! Where a nexus-flow service keeps its machine-wide state: `~/.nexusflow`, or `~/.nexusflow-<qualifier>`
//! for a named [`Instance`].
//!
//! Four modules used to resolve that directory independently — the workspace registry, the sync
//! endpoint config, the daemon lock/heartbeat and the launchd agent's log directory — each with its
//! own copy of the string and its own `directories::BaseDirs::new()` call, and each with no way for
//! a test to point it somewhere else. [`ServiceHome`] is that one directory, named once, with the
//! file names it holds beside it.
//!
//! **"Named once" is now true, and it was not.** The claim stood in this doc while
//! `crates/nxs/src/sync/endpoint.rs` held a fifth copy of the string for `config.toml` — the whole
//! reason [`ServiceHome::config`] exists (nxf 6j6v.gd9p).
//!
//! **The directory NAME is the instance's** ([`Instance::home_dir`]). That is what makes two
//! services on one machine structurally unable to fight: the single-instance lock, the heartbeat
//! and the registry are files IN here, so separate homes are separate locks by construction rather
//! than by agreement.
//!
//! **It is a value, not a global.** [`ServiceHome::at`] is what lets every behaviour in this crate
//! be proven against a `TempDir` instead of the developer's real home — the specific defect the
//! registry's own history records (98 stale entries written by unit tests that had no way to reach
//! anywhere else).

use std::path::{Path, PathBuf};

use nxs_foundation::error::{NxfError, Result};

use crate::instance::Instance;

/// The workspace registry within it.
const REGISTRY_FILE: &str = "workspaces.toml";
/// The global sync configuration — the file `crates/nxs`'s endpoint module used to resolve with its
/// own copy of the directory name (nxf 6j6v.gd9p).
const CONFIG_FILE: &str = "config.toml";
/// The single-instance lock the running service holds for its whole lifetime.
const LOCK_FILE: &str = "sync-daemon.lock";
/// The heartbeat the running service rewrites after every pass, and `status` reads.
const HEARTBEAT_FILE: &str = "sync-daemon.json";
/// Where an unattended service's stdout/stderr land.
const LOG_DIR: &str = "logs";
/// Where the alias lives — see [`ServiceHome::program`].
const BIN_DIR: &str = "bin";

/// The directory a nexus-flow background service keeps its state in, and the files within it.
///
/// The file names are kept as they were (`sync-daemon.lock`/`sync-daemon.json`) even though the
/// service is growing past sync: renaming them would strand the lock and heartbeat of a service
/// already installed on a machine, which is a live upgrade hazard for a cosmetic gain. The same
/// argument is why the production instance's DIRECTORY is still `.nexusflow` and not `.nexus-flow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceHome {
    root: PathBuf,
    instance: Instance,
}

impl ServiceHome {
    /// The real one for THIS process's instance — `~/.nexusflow`, or `~/.nexusflow-dev` when the
    /// process was started as the `nexus-flow-dev` alias, or is a BUILD told so by
    /// `NXS_SERVICE_INSTANCE`. An installed binary is always the production one, whatever a working
    /// copy exports (nxf 6j6v.cvpy). See [`Instance::resolve_from`] for the whole precedence.
    pub fn resolve() -> Result<ServiceHome> {
        ServiceHome::for_instance(Instance::ambient()?)
    }

    /// The real home of a NAMED instance — the form every verb that has already decided which
    /// instance it is talking about uses, so that decision is made once and passed rather than
    /// re-derived.
    pub fn for_instance(instance: Instance) -> Result<ServiceHome> {
        let dirs = directories::BaseDirs::new().ok_or_else(|| {
            NxfError::io("could not resolve a home directory for the nexus-flow service")
        })?;
        let root = dirs.home_dir().join(instance.home_dir());
        Ok(ServiceHome { root, instance })
    }

    /// A service home at an explicit directory. Production calls [`resolve`](Self::resolve); tests
    /// call this with a `TempDir` so nothing they do can reach the developer's real `~/.nexusflow`.
    ///
    /// The instance is the production one — which is what a test that does not care about instances
    /// wants, and what keeps every existing caller of this constructor meaning what it did.
    pub fn at(root: impl Into<PathBuf>) -> ServiceHome {
        ServiceHome {
            root: root.into(),
            instance: Instance::production(),
        }
    }

    /// [`at`](Self::at) for a named instance — a directory AND the identity to report for it.
    pub fn at_instance(root: impl Into<PathBuf>, instance: Instance) -> ServiceHome {
        ServiceHome {
            root: root.into(),
            instance,
        }
    }

    /// Which service this home belongs to.
    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    /// The directory itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<home>/workspaces.toml` — the registry the service sweeps.
    pub fn registry(&self) -> PathBuf {
        self.root.join(REGISTRY_FILE)
    }

    /// `<home>/config.toml` — the machine-wide sync configuration (the global default endpoint).
    pub fn config(&self) -> PathBuf {
        self.root.join(CONFIG_FILE)
    }

    /// `<home>/sync-daemon.lock` — the single-instance lock.
    pub fn lock(&self) -> PathBuf {
        self.root.join(LOCK_FILE)
    }

    /// `<home>/sync-daemon.json` — the heartbeat.
    pub fn heartbeat(&self) -> PathBuf {
        self.root.join(HEARTBEAT_FILE)
    }

    /// `<home>/logs` — where an unattended service's output lands.
    pub fn logs(&self) -> PathBuf {
        self.root.join(LOG_DIR)
    }

    /// `<home>/bin` — the directory holding the alias.
    pub fn bin(&self) -> PathBuf {
        self.root.join(BIN_DIR)
    }

    /// Every OTHER instance that has a home beside this one.
    ///
    /// **Two instances cannot fight over a lock, but they CAN both attend one workspace** — the
    /// registries are separate files, so nothing stops a person binding the same checkout twice.
    /// What follows if they do is not a stalemate but a duplicate: both services tick that
    /// workspace's deadline book, and a window that comes due can be started twice. That is decided
    /// and written down rather than prevented ([`crate::attend::Attendance`] and
    /// `nxs sync daemon status` both report it), because a registry is a hand-editable file and no
    /// instance is in a position to police another's.
    ///
    /// A `$HOME` that cannot be read yields nothing: this is a diagnostic beside an answer that has
    /// already been given, never a reason to fail one.
    pub fn siblings(&self) -> Vec<ServiceHome> {
        let Some(parent) = self.root.parent() else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(parent) else {
            return Vec::new();
        };
        let mut found: Vec<ServiceHome> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                let name = e.file_name();
                let instance = Instance::from_home_dir(name.to_str()?).ok()?;
                (instance != self.instance).then(|| ServiceHome::at_instance(e.path(), instance))
            })
            .collect();
        found.sort_by(|a, b| a.instance.cmp(&b.instance));
        found
    }

    /// `<home>/bin/nexus-flow` — a link to the installed binary, and the path the background agent
    /// `exec`s. Its NAME is [`Instance::name`], which is what a named instance is made of.
    ///
    /// It exists for that name. macOS takes a process's displayed name from the last component of
    /// the path handed to `exec`, so this is what makes the user's background items read
    /// `nexus-flow` instead of `nxs` — or, before 6j6v.8see, `sh`. The repo already ships one binary
    /// under four names (`nxf`/`nxm`/`nxc` are `argv[0]` links to `nxs`), and `nxs` routes on
    /// `argv[0]`, so this is a fifth spelling of an existing mechanism rather than a new one — and
    /// since 6j6v.gd9p it is what a RUNNING service reads its own instance out of.
    pub fn program(&self) -> PathBuf {
        self.bin().join(self.instance.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_the_service_owns_is_a_child_of_the_one_home() {
        let home = ServiceHome::at("/tmp/service-home");
        for path in [
            home.registry(),
            home.config(),
            home.lock(),
            home.heartbeat(),
            home.logs(),
        ] {
            assert_eq!(
                path.parent(),
                Some(Path::new("/tmp/service-home")),
                "{} escaped the service home",
                path.display()
            );
        }
    }

    #[test]
    fn the_production_home_is_the_dot_nexusflow_directory_under_the_users_home() {
        let home = ServiceHome::for_instance(Instance::production())
            .expect("a home directory resolves on a test runner");
        assert_eq!(
            home.root().file_name().and_then(|n| n.to_str()),
            Some(".nexusflow")
        );
        assert_eq!(home.program().file_name().unwrap(), "nexus-flow");
    }

    #[test]
    fn a_named_instance_resolves_to_its_own_directory_beside_the_production_one() {
        let dev = Instance::named("nexus-flow-dev").unwrap();
        let home = ServiceHome::for_instance(dev.clone()).expect("resolves");
        let prod = ServiceHome::for_instance(Instance::production()).expect("resolves");
        assert_eq!(
            home.root().file_name().and_then(|n| n.to_str()),
            Some(".nexusflow-dev")
        );
        assert_eq!(home.root().parent(), prod.root().parent(), "same $HOME");
        assert_eq!(home.instance(), &dev);
    }

    #[test]
    fn a_home_finds_its_sisters_beside_it_and_no_stranger_among_them() {
        let tmp = tempfile::TempDir::new().unwrap();
        for dir in [
            ".nexusflow",
            ".nexusflow-dev",
            ".nexusflow-foundations-dev",
            ".config",
            ".nexusflow.bak",
        ] {
            std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }
        // A FILE that happens to be named like one of ours is not a home either.
        std::fs::write(tmp.path().join(".nexusflow-notadir"), "x").unwrap();

        let prod = ServiceHome::at_instance(tmp.path().join(".nexusflow"), Instance::production());
        let names: Vec<String> = prod
            .siblings()
            .iter()
            .map(|h| h.instance().name())
            .collect();
        assert_eq!(
            names,
            vec![
                "nexus-flow-dev".to_string(),
                "nexus-flow-foundations-dev".to_string()
            ],
            "a sister is a nexus-flow home other than this one, and nothing else is"
        );
    }

    #[test]
    fn two_instances_can_never_reach_for_the_same_lock_registry_or_heartbeat() {
        // Not an assumption of the design — the design itself. Everything that could be contended
        // is a file inside a directory whose name is the instance's.
        let tmp = Path::new("/tmp/homes");
        let a = ServiceHome::at_instance(
            tmp.join(Instance::production().home_dir()),
            Instance::production(),
        );
        let b = ServiceHome::at_instance(
            tmp.join(Instance::named("nexus-flow-dev").unwrap().home_dir()),
            Instance::named("nexus-flow-dev").unwrap(),
        );
        for (x, y) in [
            (a.lock(), b.lock()),
            (a.registry(), b.registry()),
            (a.heartbeat(), b.heartbeat()),
            (a.config(), b.config()),
            (a.program(), b.program()),
            (a.logs(), b.logs()),
        ] {
            assert_ne!(x, y, "two instances would contend for {}", x.display());
        }
    }
}
